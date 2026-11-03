//! Cookie loading for the authenticated WEB_REMIX session.
//!
//! Three input shapes are accepted: Netscape `cookies.txt`, a raw `Cookie:`
//! header, and the JSON array/object exports of Chrome/Firefox extensions.
//! Parsing is pure and total: malformed rows are skipped, never panicked on.
//! Cookie *values* are secrets — nothing here puts one into an error, `Debug`
//! rendering is not derived for [`Cookie`], and only [`mask`]ed identities or
//! cookie names leave this module.
//!
//! A jar is only ever sent to `https://<host>`, so [`CookieJar::header`] applies
//! the `secure` flag as satisfied and ignores the request scheme.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use ml_core::{MlError, MlResult};
use serde_json::Value;

/// Origin the authenticated transport talks to.
pub const MUSIC_HOST: &str = "music.youtube.com";
/// Host whose cookies also apply to WEB_REMIX (account menu, consent).
pub const GOOGLE_HOST: &str = "www.google.com";

/// One parsed cookie. `domain`/`path` are empty for a bare `Cookie:` header,
/// which matches every host and path.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Set by a `Set-Cookie` without a `Domain` attribute: exact host only.
    /// Netscape/JSON rows cannot express this and stay `false`.
    pub host_only: bool,
    /// Unix seconds; `None` = session cookie (Netscape `0`, JSON absent).
    pub expires_at: Option<i64>,
}

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("host_only", &self.host_only)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

// ------------------------------------------------------------------ parsing

/// Netscape `cookies.txt`. Tolerates CRLF, trailing spaces, `#HttpOnly_`
/// domains and `#` comments; session cookies (expiry `0`/empty) stay `None`.
/// Rows that do not carry domain/path/name are dropped.
pub fn parse_netscape(input: &str) -> Vec<Cookie> {
    input.lines().filter_map(parse_netscape_row).collect()
}

fn parse_netscape_row(line: &str) -> Option<Cookie> {
    let line = line.trim_end_matches(['\r', ' ', '\t']);
    let (http_only, line) = match line.strip_prefix("#HttpOnly_") {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    if line.is_empty() || line.starts_with('#') || !line.contains('\t') {
        return None;
    }
    let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
    if fields.len() < 6 {
        return None;
    }
    let name = fields[5];
    if fields[0].is_empty() || name.is_empty() {
        return None;
    }
    Some(Cookie {
        domain: fields[0].to_string(),
        path: if fields[2].is_empty() {
            "/".to_string()
        } else {
            fields[2].to_string()
        },
        secure: fields[3].eq_ignore_ascii_case("true"),
        http_only,
        host_only: false,
        expires_at: if fields[4].is_empty() {
            None
        } else {
            // A non-numeric expiry is a malformed row, not a session cookie.
            let expires = fields[4].parse::<i64>().ok()?;
            (expires != 0).then_some(expires)
        },
        name: name.to_string(),
        value: fields.get(6).copied().unwrap_or_default().to_string(),
    })
}

/// `a=b; c=d`, the shape of a `Cookie:` header / WebView `getCookie()` result.
pub fn parse_header(input: &str) -> Vec<Cookie> {
    input
        .split(';')
        .filter_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            let name = name.trim();
            (!name.is_empty()).then(|| Cookie {
                name: name.to_string(),
                value: value.trim().to_string(),
                path: "/".to_string(),
                ..Cookie::default()
            })
        })
        .collect()
}

/// Chrome/Firefox cookie export: an array of `{name, value, domain, path,
/// secure, httpOnly, expirationDate}`, a `{cookies: […]}` wrapper, or a flat
/// `{name: value}` map.
pub fn parse_json(input: &str) -> Option<Vec<Cookie>> {
    let value: Value = serde_json::from_str(input).ok()?;
    Some(json_cookies(&value))
}

fn json_cookies(value: &Value) -> Vec<Cookie> {
    match value {
        Value::Array(items) => items.iter().filter_map(json_cookie).collect(),
        Value::Object(map) => {
            if let Some(items) = map.get("cookies").and_then(Value::as_array) {
                return items.iter().filter_map(json_cookie).collect();
            }
            map.iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|value| Cookie {
                        name: name.clone(),
                        value: value.to_string(),
                        path: "/".to_string(),
                        ..Cookie::default()
                    })
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn json_cookie(item: &Value) -> Option<Cookie> {
    let object = item.as_object()?;
    let name = object.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let expires = ["expirationDate", "expires", "expiry"]
        .iter()
        .find_map(|key| object.get(*key))
        .and_then(json_seconds);
    Some(Cookie {
        name: name.to_string(),
        value: object
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        domain: object
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        path: match object.get("path").and_then(Value::as_str) {
            Some(path) if !path.is_empty() => path.to_string(),
            _ => "/".to_string(),
        },
        secure: object
            .get("secure")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        http_only: object
            .get("httpOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        host_only: false,
        expires_at: expires.filter(|seconds| *seconds > 0),
    })
}

fn json_seconds(value: &Value) -> Option<i64> {
    let seconds = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    seconds.is_finite().then_some(seconds as i64)
}

/// Detect the format: JSON if it starts with `[`/`{`, Netscape if any row has
/// tab-separated fields, otherwise a `Cookie:` header string.
pub fn parse_auto(input: &str) -> Vec<Cookie> {
    let trimmed = input.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        if let Some(cookies) = parse_json(input).filter(|cookies| !cookies.is_empty()) {
            return cookies;
        }
    }
    let netscape = parse_netscape(input);
    if !netscape.is_empty() {
        return netscape;
    }
    parse_header(input)
}

// ----------------------------------------------------------------- matching

/// RFC 6265 domain-match; an empty cookie domain matches every host.
pub fn domain_matches(host: &str, domain: &str) -> bool {
    let domain = domain.trim_start_matches('.').trim_end_matches('.');
    if domain.is_empty() {
        return true;
    }
    if host.eq_ignore_ascii_case(domain) {
        return true;
    }
    let (host, domain) = (host.as_bytes(), domain.as_bytes());
    host.len() > domain.len()
        && ascii_ends_with(host, domain)
        && host[host.len() - domain.len() - 1] == b'.'
}

fn ascii_ends_with(host: &[u8], suffix: &[u8]) -> bool {
    host.len() >= suffix.len() && host[host.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// RFC 6265 path-match: a `/`-terminated prefix, or an exact sibling segment.
pub fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    let cookie_path = if cookie_path.is_empty() {
        "/"
    } else {
        cookie_path
    };
    if !request_path.starts_with(cookie_path) {
        return false;
    }
    cookie_path.ends_with('/')
        || request_path.len() == cookie_path.len()
        || request_path.as_bytes()[cookie_path.len()] == b'/'
}

/// Session cookies (`expires_at == None`) never expire; the rest do.
pub fn is_expired(expires_at: Option<i64>, now: i64) -> bool {
    matches!(expires_at, Some(at) if at <= now)
}

/// Live cookies for `host`/`path`, one per name: the most specific domain, then
/// the most specific path, then the last row wins. Output is sorted by name so
/// the header is byte-identical between runs.
pub fn select<'a>(cookies: &'a [Cookie], host: &str, path: &str, now: i64) -> Vec<&'a Cookie> {
    let mut best: BTreeMap<&str, (usize, usize, &Cookie)> = BTreeMap::new();
    for cookie in cookies {
        if cookie.name.is_empty()
            || is_expired(cookie.expires_at, now)
            || !matches_host(cookie, host)
            || !path_matches(path, &cookie.path)
        {
            continue;
        }
        let specificity = (cookie.domain.len(), cookie.path.len());
        match best.get(cookie.name.as_str()) {
            Some((domain, path, _)) if (*domain, *path) > specificity => {}
            _ => {
                best.insert(cookie.name.as_str(), (specificity.0, specificity.1, cookie));
            }
        }
    }
    best.into_values().map(|(_, _, cookie)| cookie).collect()
}

/// `name=value; …` for `host`, excluding `SOCS`: the transport always sends its
/// own consent cookie, and a duplicated name would be ambiguous to the server.
pub fn header(cookies: &[Cookie], host: &str, path: &str, now: i64) -> String {
    select(cookies, host, path, now)
        .into_iter()
        .filter(|cookie| cookie.name != "SOCS")
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Host filter for sending: a host-only cookie must equal the host exactly.
fn matches_host(cookie: &Cookie, host: &str) -> bool {
    if cookie.host_only {
        cookie
            .domain
            .trim_start_matches('.')
            .eq_ignore_ascii_case(host)
    } else {
        domain_matches(host, &cookie.domain)
    }
}

// --------------------------------------------------------------- set-cookie

/// One parsed `Set-Cookie` header. `remove` marks a deletion (empty value or
/// `Max-Age <= 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCookie {
    pub cookie: Cookie,
    pub remove: bool,
}

/// Parse one `Set-Cookie` header for a request to `request_host`/
/// `request_path`. Pure and total; `None` for a malformed header or a `Domain`
/// that does not domain-match the request host (RFC 6265 §5.3).
///
/// `now` anchors `Max-Age`; an `Expires` in the past keeps the cookie with that
/// deadline so [`select`] drops it, but `Max-Age <= 0`/an empty value delete it.
pub fn parse_set_cookie(
    header: &str,
    request_host: &str,
    request_path: &str,
    now: i64,
) -> Option<SetCookie> {
    let mut segments = header.split(';');
    let (name, value) = segments.next()?.trim().split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let mut domain = String::new();
    let mut path = String::new();
    let mut secure = false;
    let mut http_only = false;
    let mut max_age: Option<i64> = None;
    let mut expires: Option<i64> = None;
    for segment in segments {
        let segment = segment.trim();
        let (key, value) = match segment.split_once('=') {
            Some((key, value)) => (key.trim(), value.trim()),
            None => (segment, ""),
        };
        match key.to_ascii_lowercase().as_str() {
            "domain" => domain = value.to_string(),
            "path" => path = value.to_string(),
            "secure" => secure = true,
            "httponly" => http_only = true,
            "max-age" => max_age = value.parse::<i64>().ok(),
            "expires" => expires = parse_http_date(value),
            // SameSite / Priority / Partitioned do not affect sending here.
            _ => {}
        }
    }

    let host_only = domain.is_empty();
    if !host_only && !domain_matches(request_host, &domain) {
        return None;
    }
    let expires_at = match max_age {
        Some(seconds) => {
            if seconds <= 0 {
                None
            } else {
                Some(now.saturating_add(seconds))
            }
        }
        None => expires,
    };
    Some(SetCookie {
        remove: value.trim().is_empty() || max_age.is_some_and(|seconds| seconds <= 0),
        cookie: Cookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain: if host_only {
                request_host.to_string()
            } else {
                domain
            },
            path: if path.starts_with('/') {
                path
            } else {
                default_path(request_path)
            },
            secure,
            http_only,
            host_only,
            expires_at,
        },
    })
}

/// RFC 6265 §5.1.4 default-path: the request path up to (excluding) its last
/// `/`, or `/`.
fn default_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".to_string();
    }
    match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => request_path[..index].to_string(),
    }
}

/// `Sun, 06 Nov 1994 08:49:37 GMT` → Unix seconds. Pure and total.
pub fn parse_http_date(value: &str) -> Option<i64> {
    let value = value.trim();
    let rest = match value.split_once(',') {
        Some((_, rest)) => rest.trim(),
        None => value,
    };
    let mut parts = rest.split_whitespace();
    let day: i64 = parts.next()?.parse().ok()?;
    let month = month_number(parts.next()?)?;
    let year: i64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if !(1..=31).contains(&day) || !(1601..=9999).contains(&year) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn month_number(name: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let name = name.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|month| name.starts_with(month))
        .map(|index| index as i64 + 1)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

// ------------------------------------------------------------------ trimming

/// `A***z`; never reveals the length or the middle of a value.
pub fn mask(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest: Vec<char> = chars.collect();
    match rest.len() {
        0 => "*".to_string(),
        1 => format!("{first}*"),
        _ => format!("{first}***{}", rest[rest.len() - 1]),
    }
}

/// `r***l@example.com`; the local part is the identifying half.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) if !domain.is_empty() => format!("{}@{domain}", mask(local)),
        _ => mask(email),
    }
}

// ----------------------------------------------------------------------- jar

/// Parsed cookies plus the filters the transport needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn parse(input: &str) -> Self {
        Self {
            cookies: parse_auto(input),
        }
    }

    pub fn from_cookies(cookies: Vec<Cookie>) -> Self {
        Self { cookies }
    }

    /// Read a `cookies.txt`/JSON/header file **read-only**. Writes only ever
    /// happen through [`save_file`]/[`CookieJar::save`]; permission warnings are
    /// reported separately by [`permission_warning`] and are not an error.
    pub fn load(path: &Path) -> MlResult<Self> {
        let text =
            fs::read_to_string(path).map_err(|error| MlError::Io(format!("cookies: {error}")))?;
        Ok(Self::parse(&text))
    }

    /// Write a Netscape file with `0600`, atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> MlResult<()> {
        save_file(path, self)
    }

    pub fn cookies(&self) -> &[Cookie] {
        &self.cookies
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// Cookie names present, sorted, values never included.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.cookies.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// A non-empty value for `name`, preferring the most specific domain.
    pub fn get(&self, name: &str) -> Option<&str> {
        let now = now_unix();
        select(&self.cookies, MUSIC_HOST, "/", now)
            .into_iter()
            .chain(select(&self.cookies, GOOGLE_HOST, "/", now))
            .find(|cookie| cookie.name == name && !cookie.value.is_empty())
            .map(|cookie| cookie.value.as_str())
    }

    pub fn sapisid(&self) -> Option<&str> {
        self.get("SAPISID")
    }

    /// `VISITOR_DATA` issued with this session; the ytcfg scrape fallback still
    /// applies when the file has none.
    pub fn visitor_data(&self) -> Option<&str> {
        self.get("VISITOR_INFO1_LIVE")
    }

    /// Upstream `LoginScreen` gate: a session is authenticated with `SAPISID`.
    pub fn is_authenticated(&self) -> bool {
        self.sapisid().is_some()
    }

    /// `name=value; …` for `music.youtube.com`, without `SOCS`.
    pub fn header(&self) -> String {
        header(&self.cookies, MUSIC_HOST, "/", now_unix())
    }

    pub fn header_at(&self, now: i64) -> String {
        header(&self.cookies, MUSIC_HOST, "/", now)
    }

    /// Apply one `Set-Cookie` header: rotation replaces the same
    /// `(name, domain, path)`, an empty value or `Max-Age <= 0` deletes it.
    pub fn apply_set_cookie(
        &mut self,
        set_cookie: &str,
        request_host: &str,
        request_path: &str,
        now: i64,
    ) {
        let Some(parsed) = parse_set_cookie(set_cookie, request_host, request_path, now) else {
            return;
        };
        let existing = self.cookies.iter().position(|cookie| {
            cookie.name == parsed.cookie.name
                && cookie
                    .domain
                    .trim_start_matches('.')
                    .eq_ignore_ascii_case(parsed.cookie.domain.trim_start_matches('.'))
                && cookie.path == parsed.cookie.path
        });
        match (parsed.remove, existing) {
            (true, Some(index)) => {
                self.cookies.remove(index);
            }
            (true, None) => {}
            (false, Some(index)) => self.cookies[index] = parsed.cookie,
            (false, None) => self.cookies.push(parsed.cookie),
        }
    }

    /// Apply every `Set-Cookie` header of one response.
    pub fn merge(&mut self, set_cookies: &[String], host: &str, path: &str, now: i64) {
        for set_cookie in set_cookies {
            self.apply_set_cookie(set_cookie, host, path, now);
        }
    }

    /// Earliest expiry among live cookies, for a "session ends on" hint.
    pub fn soonest_expiry(&self, now: i64) -> Option<i64> {
        self.cookies
            .iter()
            .filter(|cookie| !is_expired(cookie.expires_at, now))
            .filter_map(|cookie| cookie.expires_at)
            .min()
    }

    pub fn to_netscape(&self) -> String {
        let mut out = String::from("# Netscape HTTP Cookie File\n");
        for cookie in &self.cookies {
            out.push_str(&netscape_row(cookie));
            out.push('\n');
        }
        out
    }
}

fn netscape_row(cookie: &Cookie) -> String {
    // A `Cookie:` header carries no domain; this jar is only ever sent to
    // music.youtube.com, so that is the honest host to write back.
    let domain = if cookie.domain.is_empty() {
        ".youtube.com"
    } else {
        cookie.domain.as_str()
    };
    format!(
        "{}{}\t{}\t{}\t{}\t{}\t{}\t{}",
        if cookie.http_only { "#HttpOnly_" } else { "" },
        domain,
        if domain.starts_with('.') {
            "TRUE"
        } else {
            "FALSE"
        },
        if cookie.path.is_empty() {
            "/"
        } else {
            cookie.path.as_str()
        },
        if cookie.secure { "TRUE" } else { "FALSE" },
        cookie.expires_at.unwrap_or(0),
        cookie.name,
        cookie.value,
    )
}

/// `0600` atomic write. Refuses to write through a symlink; fixes the mode of
/// an existing file rather than trusting it.
pub fn save_file(path: &Path, jar: &CookieJar) -> MlResult<()> {
    if fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(MlError::Io(
            "cookies: refusing to write through a symlink".to_string(),
        ));
    }
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("cookies.txt");
    let temp = directory.join(format!(".{name}.{}.tmp", std::process::id()));

    let write = || -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        let mut file = options.open(&temp)?;
        file.write_all(jar.to_netscape().as_bytes())?;
        file.sync_all()?;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&temp, path)
    };
    write().map_err(|error| {
        let _ = fs::remove_file(&temp);
        MlError::Io(format!("cookies: {error}"))
    })
}

/// `Some("cookies: mode 0644 is world-readable")` when the file is not `0600`.
pub fn permission_warning(path: &Path) -> Option<String> {
    let mode = fs::metadata(path).ok()?.permissions().mode() & 0o777;
    (mode & 0o077 != 0)
        .then(|| format!("cookies: mode {mode:04o} is readable by other users (expected 0600)"))
}

// --------------------------------------------------------------- SAPISIDHASH

/// `SAPISIDHASH <ts>_<sha1 "ts sapisid origin">` — the only `Authorization`
/// upstream sends. The order is load-bearing (see `context::sapisidhash`).
pub fn sapisidhash(sapisid: &str, origin: &str, timestamp: u64) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(format!("{timestamp} {sapisid} {origin}").as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("SAPISIDHASH {timestamp}_{hex}")
}

/// Headers an authenticated WEB_REMIX request needs, in transport order.
/// `X-Goog-Visitor-Id` is built by the context from the ytcfg scrape and is not
/// repeated here.
pub fn auth_headers(
    jar: &CookieJar,
    origin: &str,
    timestamp: u64,
    auth_user: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(sapisid) = jar.sapisid() {
        headers.push((
            "Authorization".to_string(),
            sapisidhash(sapisid, origin, timestamp),
        ));
    }
    if let Some(user) = auth_user {
        headers.push(("X-Goog-AuthUser".to_string(), user.to_string()));
    }
    headers
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Cookie {
        fn new(name: &str, value: &str, domain: &str, path: &str) -> Self {
            Self {
                name: name.to_string(),
                value: value.to_string(),
                domain: domain.to_string(),
                path: path.to_string(),
                ..Self::default()
            }
        }
    }

    #[test]
    fn netscape_rows_cover_httponly_comments_session_and_crlf() {
        let input = "# Netscape HTTP Cookie File\r\n\
                     #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1815841718\tSAPISID\tsecret\r\n\
                     .google.com\tTRUE\t/\tFALSE\t0\tNID\tsession\r\n\
                     music.youtube.com\tFALSE\t/\tFALSE\tfuture\tBROKEN\trow\r\n\
                     \r\n\
                     .youtube.com\tTRUE\t/\tTRUE\t\r\n";
        let cookies = parse_netscape(input);
        assert_eq!(cookies.len(), 2, "malformed rows are dropped");
        assert!(cookies[0].http_only);
        assert_eq!(cookies[0].name, "SAPISID");
        assert_eq!(cookies[0].expires_at, Some(1815841718));
        assert_eq!(cookies[1].name, "NID");
        assert_eq!(cookies[1].expires_at, None, "expiry 0 is a session cookie");
        assert!(!cookies[1].secure);
        assert_eq!(cookies[1].path, "/");
    }

    #[test]
    fn header_string_keeps_equals_in_values_and_unicode() {
        let cookies = parse_header("a=b; token=x=y; name=héllo; ; novalue");
        let pairs: Vec<(&str, &str)> = cookies
            .iter()
            .map(|c| (c.name.as_str(), c.value.as_str()))
            .collect();
        assert_eq!(pairs, vec![("a", "b"), ("token", "x=y"), ("name", "héllo")]);
    }

    #[test]
    fn json_array_object_and_flat_map() {
        let array = r#"[
            {"name":"SAPISID","value":"abc","domain":".youtube.com","path":"/",
             "secure":true,"httpOnly":true,"expirationDate":1815841718.9},
            {"name":"VISITOR_INFO1_LIVE","value":"vd","domain":".youtube.com"}
        ]"#;
        let cookies = parse_json(array).expect("array parses");
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].expires_at, Some(1815841718));
        assert!(cookies[0].http_only && cookies[0].secure);
        assert_eq!(cookies[1].path, "/");

        let wrapped = r#"{"cookies":[{"name":"A","value":"1"}]}"#;
        assert_eq!(parse_json(wrapped).unwrap()[0].name, "A");

        let flat = r#"{"SAPISID":"abc","SID":"def"}"#;
        let names: Vec<String> = parse_json(flat)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["SAPISID", "SID"]);

        assert_eq!(parse_json("not json"), None);
    }

    #[test]
    fn auto_detection_prefers_the_format_that_yields_rows() {
        assert_eq!(parse_auto("SAPISID=a; SID=b").len(), 2);
        assert_eq!(parse_auto(r#"[{"name":"A","value":"1"}]"#).len(), 1);
        assert_eq!(
            parse_auto(".youtube.com\tTRUE\t/\tTRUE\t0\tA\t1\n").len(),
            1
        );
        assert!(parse_auto("").is_empty());
        assert!(parse_auto("# only comments\n").is_empty());
    }

    #[test]
    fn domain_matching_is_rfc6265() {
        assert!(domain_matches("music.youtube.com", ".youtube.com"));
        assert!(domain_matches("youtube.com", ".youtube.com"));
        assert!(domain_matches("MUSIC.YOUTUBE.COM", ".youtube.com"));
        assert!(domain_matches("music.youtube.com", "music.youtube.com"));
        assert!(domain_matches("music.youtube.com", ""), "header cookies");
        assert!(!domain_matches("notyoutube.com", ".youtube.com"));
        assert!(!domain_matches("music.youtube.com", ".google.com"));
    }

    #[test]
    fn path_matching_respects_the_slash_rule() {
        assert!(path_matches("/", "/"));
        assert!(path_matches("/youtubei/v1/browse", "/"));
        assert!(path_matches("/a/b", "/a"));
        assert!(!path_matches("/ab", "/a"));
        assert!(path_matches("/a", "/a"));
        assert!(!path_matches("/", "/a"));
        assert!(path_matches("/x", ""));
    }

    #[test]
    fn expired_and_session_cookies() {
        assert!(is_expired(Some(10), 10));
        assert!(is_expired(Some(10), 11));
        assert!(!is_expired(Some(11), 10));
        assert!(!is_expired(None, i64::MAX));
    }

    #[test]
    fn selection_dedupes_by_name_and_prefers_specific_domains() {
        let cookies = vec![
            Cookie::new("A", "youtube", ".youtube.com", "/"),
            Cookie::new("A", "music", "music.youtube.com", "/youtubei"),
            Cookie::new("A", "google", ".google.com", "/"),
            Cookie::new("B", "old", ".youtube.com", "/"),
            Cookie {
                expires_at: Some(1),
                ..Cookie::new("C", "expired", ".youtube.com", "/")
            },
            Cookie::new("D", "session", ".youtube.com", "/"),
        ];
        let selected = select(&cookies, "music.youtube.com", "/youtubei/v1/browse", 1_000);
        let pairs: Vec<(&str, &str)> = selected
            .iter()
            .map(|c| (c.name.as_str(), c.value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![("A", "music"), ("B", "old"), ("D", "session")],
            "most specific domain wins, expired dropped, order by name"
        );
        assert!(!selected.iter().any(|c| c.name == "C"));
    }

    #[test]
    fn header_is_sorted_and_omits_consent_cookie() {
        let cookies = vec![
            Cookie::new("SOCS", "CAI", ".youtube.com", "/"),
            Cookie::new("SAPISID", "s", ".youtube.com", "/"),
            Cookie::new("HSID", "h", ".youtube.com", "/"),
            Cookie::new("NID", "n", ".google.com", "/"),
        ];
        assert_eq!(header(&cookies, MUSIC_HOST, "/", 0), "HSID=h; SAPISID=s");
    }

    #[test]
    fn http_date_parses_rfc1123_and_rejects_garbage() {
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_http_date("1 Jan 2030 00:00:00"), Some(1_893_456_000));
        assert_eq!(parse_http_date("garbage"), None);
        assert_eq!(parse_http_date("Sun, 06 Foo 1994 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 25:49:37 GMT"), None);
    }

    #[test]
    fn set_cookie_rotation_replaces_the_same_name_domain_path() {
        let mut jar = CookieJar::parse(".youtube.com\tTRUE\t/\tTRUE\t0\t__Secure-1PSIDTS\told\n");
        jar.apply_set_cookie(
            "__Secure-1PSIDTS=new; Domain=.youtube.com; Path=/; Secure; HttpOnly; SameSite=none",
            "music.youtube.com",
            "/youtubei/v1/browse",
            1_000,
        );
        assert_eq!(jar.len(), 1, "rotation replaces, never appends");
        assert_eq!(jar.get("__Secure-1PSIDTS"), Some("new"));
        assert!(jar.cookies()[0].http_only && jar.cookies()[0].secure);
    }

    #[test]
    fn set_cookie_deletes_on_empty_value_or_non_positive_max_age() {
        let mut jar = CookieJar::parse(".youtube.com\tTRUE\t/\tTRUE\t0\tA\t1\n");
        jar.apply_set_cookie(
            "A=; Domain=.youtube.com; Path=/",
            "music.youtube.com",
            "/",
            1_000,
        );
        assert!(jar.get("A").is_none(), "empty value deletes");

        jar.apply_set_cookie("B=1; Path=/", "music.youtube.com", "/", 1_000);
        jar.apply_set_cookie("B=x; Max-Age=0; Path=/", "music.youtube.com", "/", 1_000);
        assert!(jar.get("B").is_none(), "Max-Age=0 deletes");

        jar.apply_set_cookie("C=1; Path=/; Max-Age=60", "music.youtube.com", "/", 1_000);
        jar.apply_set_cookie("C=x; Path=/; Max-Age=-5", "music.youtube.com", "/", 2_000);
        assert!(jar.get("C").is_none(), "negative Max-Age deletes");
    }

    #[test]
    fn set_cookie_expiry_controls_the_next_header() {
        let base = 1_700_000_000;
        let mut jar = CookieJar::default();
        jar.apply_set_cookie(
            "SHORT=1; Domain=.youtube.com; Path=/; Expires=Sun, 06 Nov 1994 08:49:37 GMT",
            "music.youtube.com",
            "/",
            base,
        );
        jar.apply_set_cookie(
            "LONG=1; Domain=.youtube.com; Path=/; Max-Age=100",
            "music.youtube.com",
            "/",
            base,
        );
        assert_eq!(
            jar.header_at(base),
            "LONG=1",
            "Expires in the past is dropped"
        );
        assert_eq!(jar.header_at(base + 50), "LONG=1");
        assert_eq!(
            jar.header_at(base + 101),
            "",
            "Max-Age=100 from base ends at base+100"
        );
    }

    #[test]
    fn set_cookie_host_only_matches_exactly_and_domain_matches_subdomains() {
        let mut jar = CookieJar::default();
        jar.apply_set_cookie("HOSTONLY=1; Path=/", "music.youtube.com", "/", 1_000);
        jar.apply_set_cookie(
            "DOMAIN=1; Domain=.youtube.com; Path=/",
            "music.youtube.com",
            "/",
            1_000,
        );
        assert!(jar
            .cookies()
            .iter()
            .any(|c| c.name == "HOSTONLY" && c.host_only));
        assert_eq!(
            header(jar.cookies(), "music.youtube.com", "/", 1_000),
            "DOMAIN=1; HOSTONLY=1"
        );
        assert_eq!(
            header(jar.cookies(), "www.youtube.com", "/", 1_000),
            "DOMAIN=1",
            "host-only cookie never leaks to a sibling host"
        );
        // A Domain that does not cover the request host rejects the cookie.
        jar.apply_set_cookie(
            "FOREIGN=1; Domain=.google.com; Path=/",
            "music.youtube.com",
            "/",
            1_000,
        );
        assert!(jar.get("FOREIGN").is_none());
    }

    #[test]
    fn set_cookie_path_defaults_to_the_request_directory() {
        let mut jar = CookieJar::default();
        jar.apply_set_cookie(
            "DEFAULT=1",
            "music.youtube.com",
            "/youtubei/v1/browse",
            1_000,
        );
        assert_eq!(jar.cookies()[0].path, "/youtubei/v1");
        assert!(path_matches("/youtubei/v1/next", &jar.cookies()[0].path));
        assert!(!path_matches("/other", &jar.cookies()[0].path));

        jar.apply_set_cookie("ROOT=1; Path=/", "music.youtube.com", "/a/b/c", 1_000);
        assert_eq!(jar.cookies()[1].path, "/");
        jar.apply_set_cookie("BADPATH=1; Path=nope", "music.youtube.com", "/", 1_000);
        assert_eq!(
            jar.cookies()[2].path,
            "/",
            "non-/ Path falls back to default"
        );
    }

    #[test]
    fn set_cookie_malformed_headers_are_ignored() {
        let mut jar = CookieJar::default();
        for header in ["", "no-equals", "=value", "  ", "; Path=/"] {
            jar.apply_set_cookie(header, "music.youtube.com", "/", 1_000);
        }
        assert!(jar.is_empty());
    }

    #[test]
    fn merge_applies_all_headers_and_the_next_request_carries_the_rotation() {
        let mut jar = CookieJar::parse("__Secure-1PSIDTS=first; SAPISID=s; SOCS=keep");
        let response = vec![
            "__Secure-1PSIDTS=second; Domain=.youtube.com; Path=/; Secure; HttpOnly".to_string(),
            "VISITOR_INFO1_LIVE=vd; Domain=.youtube.com; Path=/".to_string(),
            "__Secure-3PSIDTS=stale; Domain=.youtube.com; Path=/; Max-Age=0".to_string(),
        ];
        jar.merge(
            &response,
            "music.youtube.com",
            "/youtubei/v1/account/account_menu",
            1_000,
        );
        assert_eq!(
            jar.header_at(1_000),
            "SAPISID=s; VISITOR_INFO1_LIVE=vd; __Secure-1PSIDTS=second",
            "rotations replace, deletions drop, SOCS is not sent by the jar"
        );
        assert!(jar.get("__Secure-3PSIDTS").is_none());
    }

    #[test]
    fn jar_requires_sapisid_and_reads_visitor_data() {
        let jar = CookieJar::parse("SAPISID=a; VISITOR_INFO1_LIVE=vd; SID=x");
        assert!(jar.is_authenticated());
        assert_eq!(jar.sapisid(), Some("a"));
        assert_eq!(jar.visitor_data(), Some("vd"));
        assert_eq!(jar.names(), vec!["SAPISID", "SID", "VISITOR_INFO1_LIVE"]);
        assert_eq!(jar.len(), 3);
        let anonymous = CookieJar::parse("SID=x");
        assert!(!anonymous.is_authenticated());
        assert_eq!(anonymous.sapisid(), None);
        assert_eq!(anonymous.visitor_data(), None);
    }

    #[test]
    fn netscape_round_trip_keeps_flags_and_domains() {
        let input = "#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1815841718\tSAPISID\tv\n\
                     .youtube.com\tTRUE\t/\tFALSE\t0\tYSC\tw\n";
        let jar = CookieJar::parse(input);
        let round = CookieJar::parse(&jar.to_netscape());
        assert_eq!(round, jar);
        assert!(jar
            .to_netscape()
            .starts_with("# Netscape HTTP Cookie File\n"));
    }

    #[test]
    fn masks_never_reveal_the_value() {
        assert_eq!(mask(""), "");
        assert_eq!(mask("a"), "*");
        assert_eq!(mask("ab"), "a*");
        assert_eq!(mask("abcdef"), "a***f");
        assert_eq!(mask_email("rahul@example.com"), "r***l@example.com");
        assert_eq!(mask_email("no-at-sign"), "n***n");
        assert!(!mask_email("rahul.kumar@example.com").contains("kumar"));
    }

    #[test]
    fn sapisidhash_matches_the_known_vector() {
        // echo -n "1700000000 SAPISID-value https://music.youtube.com" | sha1sum
        assert_eq!(
            sapisidhash("SAPISID-value", "https://music.youtube.com", 1_700_000_000),
            SAPISIDHASH_VECTOR
        );
    }

    const SAPISIDHASH_VECTOR: &str =
        "SAPISIDHASH 1700000000_d4b678dc943e8f0851e23c86ae4c55f68bb6c7b3";

    #[test]
    fn auth_headers_only_when_a_session_exists() {
        let jar = CookieJar::parse("SAPISID=abc");
        let headers = auth_headers(&jar, "https://music.youtube.com", 1_700_000_000, Some("1"));
        assert_eq!(headers.len(), 2);
        assert!(headers[0].1.starts_with("SAPISIDHASH 1700000000_"));
        assert_eq!(headers[1], ("X-Goog-AuthUser".to_string(), "1".to_string()));

        let anonymous = CookieJar::parse("SID=x");
        assert!(auth_headers(&anonymous, "https://music.youtube.com", 1, None).is_empty());
    }

    #[test]
    fn save_and_load_round_trip_with_0600() {
        let directory = std::env::temp_dir().join(format!(
            "ml-cookies-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("cookies.txt");
        let jar = CookieJar::parse("SAPISID=a; YSC=b");
        jar.save(&path).expect("save");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(permission_warning(&path), None);
        let reloaded = CookieJar::load(&path).expect("load");
        assert_eq!(reloaded.header_at(0), jar.header_at(0));
        assert_eq!(reloaded.len(), 2);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(permission_warning(&path).is_some());

        let link = directory.join("link.txt");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(jar.save(&link).is_err(), "never writes through a symlink");
        let _ = fs::remove_dir_all(&directory);
    }
}
