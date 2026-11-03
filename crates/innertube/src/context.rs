//! Client descriptors, request envelopes and `ytcfg` version discovery.
//!
//! Measured contract (`arch/api-notes.md` §1): a POST to
//! `https://<host>/youtubei/v1/<endpoint>?prettyPrint=false&key=<api_key>` with
//! `context.client` + `context.user` + `context.request`. Browse/search/next use
//! WEB_REMIX on `music.youtube.com`; `player` uses IOS (VISIONOS fallback) on
//! `www.youtube.com`. A stale `clientVersion` answers HTTP 404, which the
//! transport turns into one `ytcfg` refresh + retry.

use std::time::{Duration, Instant};

use ml_core::{MlError, MlResult, ProxyConfig};
use regex::Regex;
use serde_json::{json, Value};

use crate::ClientConfig;

/// Public WEB_REMIX API key observed in `ytcfg` for months at a time.
pub(crate) const API_KEY_FALLBACK: &str = "AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30";
/// Last WEB_REMIX version measured by `arch/api-probe` (2026-11-02).
pub(crate) const WEB_REMIX_VERSION_FALLBACK: &str = "1.20261027.10.00";
/// IOS client version measured by the player matrix probe.
pub(crate) const IOS_VERSION: &str = "21.02.3";
/// VISIONOS client version measured by the POT workstream (`arch/pot-probe`):
/// this descriptor is the proven POT-less full-track path.
pub(crate) const VISIONOS_VERSION: &str = "1.02";

pub(crate) const UA_WEB: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const UA_IOS: &str =
    "com.google.ios.youtube/21.02.3 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)";
const UA_VISIONOS: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

/// Versions scraped from the music home page.
#[derive(Debug, Clone)]
pub(crate) struct ClientVersions {
    pub api_key: String,
    pub web_remix_version: String,
    pub visitor_data: Option<String>,
    /// When the scrape last succeeded; `None` = never.
    pub fetched: Option<Instant>,
}

impl Default for ClientVersions {
    fn default() -> Self {
        Self {
            api_key: API_KEY_FALLBACK.to_string(),
            web_remix_version: WEB_REMIX_VERSION_FALLBACK.to_string(),
            visitor_data: None,
            fetched: None,
        }
    }
}

impl ClientVersions {
    pub fn is_stale(&self) -> bool {
        self.fetched
            .map(|at| at.elapsed() > Duration::from_secs(6 * 60 * 60))
            .unwrap_or(true)
    }
}

/// Which InnerTube client descriptor a request uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientKind {
    WebRemix,
    Ios,
    VisionOs,
}

impl ClientKind {
    pub fn host(self) -> &'static str {
        match self {
            Self::WebRemix => "music.youtube.com",
            Self::Ios | Self::VisionOs => "www.youtube.com",
        }
    }

    /// `X-YouTube-Client-Name` (measured values: WEB_REMIX 67, IOS 5, VISIONOS 101).
    pub fn client_id(self) -> &'static str {
        match self {
            Self::WebRemix => "67",
            Self::Ios => "5",
            Self::VisionOs => "101",
        }
    }

    pub fn user_agent(self) -> &'static str {
        match self {
            Self::WebRemix => UA_WEB,
            Self::Ios => UA_IOS,
            Self::VisionOs => UA_VISIONOS,
        }
    }

    /// `X-YouTube-Client-Version` for this descriptor.
    pub fn version(self, versions: &ClientVersions) -> String {
        match self {
            Self::WebRemix => versions.web_remix_version.clone(),
            Self::Ios => IOS_VERSION.to_string(),
            Self::VisionOs => VISIONOS_VERSION.to_string(),
        }
    }

    /// `context.client`, excluding `hl`/`gl`/`visitorData` added by the builder.
    pub fn descriptor(self, versions: &ClientVersions) -> Value {
        match self {
            Self::WebRemix => json!({
                "clientName": "WEB_REMIX",
                "clientVersion": versions.web_remix_version,
            }),
            Self::Ios => json!({
                "clientName": "IOS",
                "clientVersion": IOS_VERSION,
                "deviceMake": "Apple",
                "deviceModel": "iPhone16,2",
                "osName": "iPhone",
                "osVersion": "18.3.2.22D82",
            }),
            Self::VisionOs => json!({
                "clientName": "VISIONOS",
                "clientVersion": VISIONOS_VERSION,
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
            }),
        }
    }
}

/// An assembled request: URL, headers and JSON body. Kept separate from the
/// HTTP call so authentication headers are testable without a server.
#[derive(Debug, Clone)]
pub(crate) struct PreparedRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl PreparedRequest {
    #[cfg(test)]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Build the InnerTube envelope plus client headers.
///
/// `set_login` is deliberately per-request (upstream `SearchAuthenticationTest`):
/// cookie/Authorization/onBehalfOfUser only travel on explicitly authenticated
/// calls, never on automatic lookups or continuations.
pub(crate) fn prepare(
    config: &ClientConfig,
    versions: &ClientVersions,
    kind: ClientKind,
    endpoint: &str,
    body: Value,
    set_login: bool,
    integrity: Option<&crate::pot::PoTokens>,
) -> PreparedRequest {
    let auth = set_login.then_some(config.cookie.as_deref()).flatten();
    let visitor = integrity
        .map(|t| t.visitor_data.as_str())
        .or(config.visitor_data.as_deref())
        .or(versions.visitor_data.as_deref());

    let mut client = kind.descriptor(versions);
    if let Some(object) = client.as_object_mut() {
        object.insert("hl".into(), Value::String(config.language.clone()));
        object.insert("gl".into(), Value::String(config.country.clone()));
        if let Some(visitor) = visitor {
            object.insert("visitorData".into(), Value::String(visitor.to_string()));
        }
    }

    let mut user = serde_json::Map::new();
    user.insert("lockedSafetyMode".into(), Value::Bool(false));
    if auth.is_some() {
        if let Some(sync) = config.data_sync_id.as_deref() {
            user.insert("onBehalfOfUser".into(), Value::String(sync.to_string()));
        }
    }

    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "context".into(),
        json!({
            "client": client,
            "user": Value::Object(user),
            "request": {"useSsl": true},
        }),
    );
    if let Value::Object(params) = body {
        envelope.extend(params);
    }
    if let Some(tokens) = integrity {
        if let Some(player) = tokens.player.as_deref() {
            envelope.insert(
                "serviceIntegrityDimensions".into(),
                json!({ "poToken": player }),
            );
        }
    }

    let url = format!(
        "https://{}/youtubei/v1/{}?prettyPrint=false&key={}",
        kind.host(),
        endpoint,
        versions.api_key
    );

    let origin = format!("https://{}", kind.host());
    let mut headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("User-Agent".into(), kind.user_agent().into()),
        ("X-Goog-Api-Format-Version".into(), "2".into()),
        ("X-YouTube-Client-Name".into(), kind.client_id().into()),
        ("X-YouTube-Client-Version".into(), kind.version(versions)),
        ("Origin".into(), origin.clone()),
        // DE consent wall; without it music.youtube.com 302s to consent.youtube.com.
        (
            "Cookie".into(),
            match auth {
                Some(cookie) => format!("SOCS=CAI; {cookie}"),
                None => "SOCS=CAI".to_string(),
            },
        ),
    ];
    if let Some(visitor) = visitor {
        headers.push(("X-Goog-Visitor-Id".into(), visitor.to_string()));
    }
    if let Some(cookie) = auth {
        if let Some(sapisid) = cookie_value(cookie, "SAPISID") {
            headers.push((
                "Authorization".into(),
                sapisidhash(&sapisid, &origin, unix_now()),
            ));
        }
        if let Some(user) = config.auth_user.as_deref() {
            headers.push(("X-Goog-AuthUser".into(), user.to_string()));
        }
    }

    PreparedRequest {
        url,
        headers,
        body: Value::Object(envelope),
    }
}

/// Extract `name=value` from a raw cookie header or a Netscape `cookies.txt` blob.
pub(crate) fn cookie_value(cookie: &str, name: &str) -> Option<String> {
    for part in cookie.split([';', '\n']) {
        let part = part.trim();
        if part.is_empty() || part.starts_with('#') {
            continue;
        }
        // Netscape rows: domain, flag, path, secure, expiry, name, value
        if part.contains('\t') {
            let fields: Vec<&str> = part.split('\t').collect();
            if fields.len() >= 7 && fields[5] == name {
                return Some(fields[6].to_string());
            }
            continue;
        }
        if let Some((key, value)) = part.split_once('=') {
            if key.trim() == name {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// `SAPISIDHASH <ts>_<sha1(ts sapisid origin)>`, the only auth upstream sends.
/// The field order is load-bearing: `ts origin sapisid` is answered with a
/// signed-out 200 (`logged_in:0`) because the server recomputes the other order.
fn sapisidhash(sapisid: &str, origin: &str, timestamp: u64) -> String {
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

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build the shared reqwest client with rustls, gzip, timeout and optional proxy.
pub(crate) fn build_http_client(config: &ClientConfig) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent(UA_WEB)
        .gzip(true)
        .timeout(Duration::from_secs(config.timeout_secs.max(1)));
    if let Some(proxy) = config.proxy.as_ref().filter(|p| p.enabled) {
        if let Some(p) = proxy_of(proxy) {
            builder = builder.proxy(p);
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

fn proxy_of(config: &ProxyConfig) -> Option<reqwest::Proxy> {
    let url = config.url.trim();
    if url.is_empty() {
        return None;
    }
    let url = if url.contains("://") {
        url.to_string()
    } else {
        format!("http://{url}")
    };
    let proxy = reqwest::Proxy::all(&url).ok()?;
    if config.username.is_empty() {
        Some(proxy)
    } else {
        Some(proxy.basic_auth(&config.username, &config.password))
    }
}

/// Scrape `INNERTUBE_API_KEY`, `INNERTUBE_CLIENT_VERSION` and `VISITOR_DATA`.
pub(crate) async fn scrape_versions(http: &reqwest::Client) -> MlResult<ClientVersions> {
    let response = http
        .get("https://music.youtube.com/")
        .header("Cookie", "SOCS=CAI")
        .header("User-Agent", UA_WEB)
        .send()
        .await
        .map_err(|e| MlError::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(MlError::Http {
            status: response.status().as_u16(),
            message: "ytcfg scrape failed".to_string(),
        });
    }
    let body = response
        .text()
        .await
        .map_err(|e| MlError::Network(e.to_string()))?;
    let mut versions = ClientVersions::default();
    if let Some(key) = capture(&body, r#""INNERTUBE_API_KEY":"(.*?)""#) {
        versions.api_key = key;
    }
    if let Some(version) = capture(&body, r#""INNERTUBE_CLIENT_VERSION":"(.*?)""#) {
        versions.web_remix_version = version;
    }
    versions.visitor_data = parse_visitor_data(&body).map(str::to_string);
    versions.fetched = Some(Instant::now());
    Ok(versions)
}

/// `"VISITOR_DATA":"…"` from music.youtube.com HTML, unescaped exactly once.
/// Returns a non-empty slice of the input, or `None`; total and panic-free.
pub fn parse_visitor_data(html: &str) -> Option<&str> {
    let marker = "\"VISITOR_DATA\":\"";
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    let value = &rest[..end];
    (!value.is_empty()).then_some(value)
}

/// Unpadded base64url (`[A-Za-z0-9_-]`), length `ceil(4n/3)`. Pure and total;
/// used by token helpers without a base64 dependency.
pub fn base64url_nopad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().saturating_mul(4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(triple >> 6) as usize & 0x3f] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[triple as usize & 0x3f] as char);
        }
    }
    out
}

fn capture(haystack: &str, pattern: &str) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    let captured = re.captures(haystack)?.get(1)?.as_str().trim().to_string();
    (!captured.is_empty()).then_some(captured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::{Digest, Sha1};

    fn config() -> ClientConfig {
        ClientConfig {
            cookie: Some("SAPISID=session; HSID=ignored".into()),
            data_sync_id: Some("channel-sync-id".into()),
            auth_user: Some("1".into()),
            ..ClientConfig::default()
        }
    }

    #[test]
    fn cookie_value_reads_raw_and_netscape() {
        assert_eq!(
            cookie_value("SAPISID=abc; x=y", "SAPISID").as_deref(),
            Some("abc")
        );
        let netscape =
            "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tnetscape\n";
        assert_eq!(
            cookie_value(netscape, "SAPISID").as_deref(),
            Some("netscape")
        );
        assert_eq!(cookie_value("HSID=x", "SAPISID"), None);
    }

    #[test]
    fn logged_in_request_carries_identity_and_anonymous_does_not() {
        let versions = ClientVersions::default();
        let authed = prepare(
            &config(),
            &versions,
            ClientKind::WebRemix,
            "search",
            json!({"query":"q"}),
            true,
            None,
        );
        assert!(authed.header("Authorization").is_some());
        assert!(authed.header("Cookie").unwrap().contains("SAPISID=session"));
        assert_eq!(authed.header("X-Goog-AuthUser"), Some("1"));
        assert_eq!(
            authed
                .body
                .pointer("/context/user/onBehalfOfUser")
                .and_then(Value::as_str),
            Some("channel-sync-id")
        );

        let anon = prepare(
            &config(),
            &versions,
            ClientKind::WebRemix,
            "search",
            json!({"query":"q"}),
            false,
            None,
        );
        assert_eq!(anon.header("Authorization"), None);
        assert_eq!(anon.header("Cookie"), Some("SOCS=CAI"));
        assert_eq!(anon.header("X-Goog-AuthUser"), None);
        assert_eq!(anon.body.pointer("/context/user/onBehalfOfUser"), None);
    }

    #[test]
    fn automatic_lookups_omit_account_identity() {
        // Upstream `SearchAuthenticationTest`: query, filtered query,
        // continuation and suggestions never carry login headers.
        let versions = ClientVersions::default();
        for (endpoint, body) in [
            ("search", json!({"query": "artist lookup"})),
            (
                "search",
                json!({"query": "14 m reproducciones", "params": "artist-filter"}),
            ),
            ("search", json!({"continuation": "next-page"})),
            (
                "music/get_search_suggestions",
                json!({"input": "unfinished query"}),
            ),
        ] {
            let request = prepare(
                &config(),
                &versions,
                ClientKind::WebRemix,
                endpoint,
                body,
                false,
                None,
            );
            assert_eq!(request.header("Authorization"), None, "{endpoint}");
            assert_eq!(request.header("X-Goog-AuthUser"), None, "{endpoint}");
            assert_eq!(request.header("Cookie"), Some("SOCS=CAI"), "{endpoint}");
            assert_eq!(
                request.body.pointer("/context/user/onBehalfOfUser"),
                None,
                "{endpoint}"
            );
        }
    }

    #[test]
    fn client_version_header_matches_the_client_descriptor() {
        let versions = ClientVersions::default();
        let ios = prepare(
            &ClientConfig::default(),
            &versions,
            ClientKind::Ios,
            "player",
            json!({"videoId": "v"}),
            false,
            None,
        );
        assert_eq!(ios.header("X-YouTube-Client-Version"), Some(IOS_VERSION));
        let web = prepare(
            &ClientConfig::default(),
            &versions,
            ClientKind::WebRemix,
            "browse",
            json!({"browseId": "FEmusic_home"}),
            false,
            None,
        );
        assert_eq!(
            web.header("X-YouTube-Client-Version"),
            Some(versions.web_remix_version.as_str())
        );
        let vision = prepare(
            &ClientConfig::default(),
            &versions,
            ClientKind::VisionOs,
            "player",
            json!({"videoId": "v"}),
            false,
            None,
        );
        assert_eq!(
            vision.header("X-YouTube-Client-Version"),
            Some(VISIONOS_VERSION)
        );
        assert!(vision.header("User-Agent").unwrap().contains("Macintosh"));
        assert_eq!(
            vision
                .body
                .pointer("/context/client/deviceModel")
                .and_then(Value::as_str),
            Some("RealityDevice17,1")
        );
    }

    #[test]
    fn integrity_token_adds_dimensions_and_visitor_header() {
        let tokens = crate::pot::PoTokens {
            player: Some("PLAYER_POT".into()),
            gvs: Some("GVS_POT".into()),
            visitor_data: "VISITOR".into(),
            expires_at: None,
        };
        let request = prepare(
            &ClientConfig::default(),
            &ClientVersions::default(),
            ClientKind::Ios,
            "player",
            json!({"videoId":"v"}),
            false,
            Some(&tokens),
        );
        assert_eq!(request.header("X-Goog-Visitor-Id"), Some("VISITOR"));
        assert_eq!(
            request
                .body
                .pointer("/serviceIntegrityDimensions/poToken")
                .and_then(Value::as_str),
            Some("PLAYER_POT")
        );
        assert_eq!(
            request
                .body
                .pointer("/context/client/visitorData")
                .and_then(Value::as_str),
            Some("VISITOR")
        );
        assert!(request
            .url
            .starts_with("https://www.youtube.com/youtubei/v1/player?"));
    }

    #[test]
    fn sapisidhash_matches_the_documented_vector() {
        let mut hasher = Sha1::new();
        hasher.update(b"1700000000 SAPISID https://music.youtube.com");
        let expected = hasher.finalize();
        let expected_hex = expected
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(
            sapisidhash("SAPISID", "https://music.youtube.com", 1700000000),
            format!("SAPISIDHASH 1700000000_{expected_hex}")
        );
    }

    #[test]
    fn visitor_data_parser_returns_a_nonempty_substring() {
        let html = r#"<script>var ytcfg={"VISITOR_DATA":"CgtaR3VhY2FrZQ%3D%3D","X":1};</script>"#;
        assert_eq!(parse_visitor_data(html), Some("CgtaR3VhY2FrZQ%3D%3D"));
        assert_eq!(parse_visitor_data(r#""VISITOR_DATA":"""#), None);
        assert_eq!(parse_visitor_data("no visitor here"), None);
        assert_eq!(parse_visitor_data(r#""VISITOR_DATA":"abc"#), None);
    }

    #[test]
    fn base64url_nopad_matches_known_vectors_and_lengths() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64url_nopad(input.as_bytes()), expected, "{input}");
            assert_eq!(
                base64url_nopad(input.as_bytes()).len(),
                (input.len() * 4).div_ceil(3),
                "{input}"
            );
        }
        assert_eq!(base64url_nopad(&[0xff, 0xff, 0xff]), "____");
        assert!(base64url_nopad(&[0, 1, 2, 250, 251, 252])
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
