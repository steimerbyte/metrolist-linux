//! Read-only import of a live Firefox / LibreWolf session.
//!
//! The app needs a `Cookie:` header for the signed-in session; asking the user
//! to re-export `cookies.txt` by hand goes stale immediately. Firefox keeps
//! that session in `<profile>/cookies.sqlite`, so this module finds a profile
//! and turns its `moz_cookies` rows into Netscape `cookies.txt` text — the
//! format [`ml_innertube::cookies`] already parses. It never writes to the
//! browser database (a running Firefox holds its own lock), never reads one
//! outside the requested hosts, and never puts a cookie value into an error.
//!
//! Two reads are attempted: a plain `SQLITE_OPEN_READ_ONLY` open, then — because
//! a WAL-mode database refuses read-only access when `-shm` cannot be created —
//! the same file through `?immutable=1`. `immutable=1` trusts that the file does
//! not change under us, so it may read a snapshot from before the last commit;
//! for a cookie import that is a fine trade for working while Firefox runs.
//!
//! shortcut: rows holding tabs, newlines or NUL are dropped rather than escaped,
//! because a malformed field would shift the rest of the line and corrupt the
//! jar. Upgrade if a real provider starts issuing such values.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ml_core::{MlError, MlResult};
use rusqlite::{Connection, OpenFlags};

use crate::rows::store_error;

/// Hosts whose cookies make up a YouTube Music session: the bare domains, plus
/// any subdomain of each (Firefox stores a domain cookie as `.youtube.com`).
const HOST_FILTER: &str = "(host IN ('youtube.com', 'www.youtube.com', 'music.youtube.com', \
     'google.com', 'www.google.com', 'accounts.google.com') \
     OR host LIKE '%.youtube.com' OR host LIKE '%.google.com')";

/// One `cookies.sqlite` belonging to a browser profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirefoxProfile {
    /// Directory name, e.g. `abc123.default-release`. The suffix is what a user
    /// recognises in Firefox's own profile manager.
    pub name: String,
    pub cookies_db: PathBuf,
    /// File modification time; used only to rank profiles.
    pub modified: Option<SystemTime>,
}

/// One `moz_cookies` row, before it becomes a Netscape line.
///
/// `Debug` is derived: a value is a secret, so callers must not print a row.
#[derive(Clone, PartialEq, Eq)]
pub struct FirefoxCookie {
    pub host: String,
    pub path: String,
    pub name: String,
    pub value: String,
    /// Unix seconds; `0` is a session cookie.
    pub expiry: i64,
    pub secure: bool,
    pub http_only: bool,
}

impl std::fmt::Debug for FirefoxCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirefoxCookie")
            .field("host", &self.host)
            .field("path", &self.path)
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("expiry", &self.expiry)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .finish()
    }
}

/// Netscape header. Every consumer here skips `#` lines.
const NETSCAPE_HEADER: &str = "# Netscape HTTP Cookie File\n";

/// Profile roots to search, in the order they are scanned. `XDG_CONFIG_HOME`
/// is honoured when it names an absolute directory, as the spec says to.
fn profile_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    vec![
        home.join(".mozilla/firefox"),
        config.join("librewolf/librewolf"),
        home.join(".librewolf"),
        // LibreWolf's Flatpak build keeps its own home.
        home.join(".var/app/io.gitlab.librewolf-community/.librewolf"),
    ]
}

/// `cookies.sqlite` under every profile of the given roots, newest modification
/// first. Missing roots and unreadable files are skipped, never an error: a
/// machine without Firefox is the normal case.
pub fn firefox_profiles_in(bases: &[PathBuf]) -> Vec<FirefoxProfile> {
    let mut profiles = Vec::new();
    for base in bases {
        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let cookies_db = entry.path().join("cookies.sqlite");
            let Ok(metadata) = std::fs::metadata(&cookies_db) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            // A database we may not read is not a usable profile.
            if std::fs::File::open(&cookies_db).is_err() {
                continue;
            }
            profiles.push(FirefoxProfile {
                name: entry.file_name().to_string_lossy().into_owned(),
                cookies_db,
                modified: metadata.modified().ok(),
            });
        }
    }
    // Newest first; the name breaks ties so the order is total.
    profiles.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.name.cmp(&right.name))
    });
    profiles
}

/// Every Firefox/LibreWolf profile on this machine, newest first.
pub fn firefox_profiles() -> Vec<FirefoxProfile> {
    firefox_profiles_in(&profile_dirs())
}

/// The most recently used profile, if the machine has one.
pub fn newest_firefox_profile() -> Option<FirefoxProfile> {
    firefox_profiles().into_iter().next()
}

/// Netscape `cookies.txt` for the YouTube/Google cookies in one profile, as of
/// `now_unix` (session cookies and unexpired rows only).
pub fn read_firefox_cookies_at(profile: &FirefoxProfile, now_unix: i64) -> MlResult<String> {
    let rows = read_moz_cookies(&profile.cookies_db, now_unix)?;
    Ok(netscape_from_rows(&rows, now_unix))
}

/// [`read_firefox_cookies_at`] against the wall clock.
pub fn read_firefox_cookies(profile: &FirefoxProfile) -> MlResult<String> {
    let now = (ml_core::now_millis() / 1000) as i64;
    read_firefox_cookies_at(profile, now)
}

/// Render rows as Netscape lines: `#HttpOnly_` + host, domain flag, path,
/// secure, expiry seconds, name, value.
///
/// Pure and total. Rows are filtered to [`HOST_FILTER`]'s hosts, expired rows
/// are dropped (session cookies kept), and fields that would break the format
/// are skipped rather than escaped. Output order is `(host, path, name)`, so the
/// same database always produces the same text.
pub fn netscape_from_rows(rows: &[FirefoxCookie], now_unix: i64) -> String {
    let mut kept: Vec<&FirefoxCookie> = rows
        .iter()
        .filter(|row| usable(row, now_unix))
        .filter(|row| {
            !row.name.is_empty()
                && well_formed(&row.host)
                && well_formed(&row.path)
                && well_formed(&row.name)
                && well_formed(&row.value)
        })
        .collect();
    kept.sort_by(|left, right| {
        left.host
            .cmp(&right.host)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut out = String::with_capacity(NETSCAPE_HEADER.len() + kept.len() * 64);
    out.push_str(NETSCAPE_HEADER);
    for row in kept {
        if row.http_only {
            out.push_str("#HttpOnly_");
        }
        out.push_str(&row.host);
        out.push('\t');
        out.push_str(if row.host.starts_with('.') {
            "TRUE"
        } else {
            "FALSE"
        });
        out.push('\t');
        out.push_str(&row.path);
        out.push('\t');
        out.push_str(if row.secure { "TRUE" } else { "FALSE" });
        out.push('\t');
        out.push_str(&row.expiry.to_string());
        out.push('\t');
        out.push_str(&row.name);
        out.push('\t');
        out.push_str(&row.value);
        out.push('\n');
    }
    out
}

/// Whether one row may enter the jar: a YouTube/Google host, and either a
/// session cookie or one that has not expired at `now_unix`.
///
/// This is [`HOST_FILTER`] plus the expiry rule restated in Rust, so the pure
/// renderer is correct for rows that did not come from the SQL.
pub fn usable(row: &FirefoxCookie, now_unix: i64) -> bool {
    wanted_host(&row.host) && (row.expiry == 0 || row.expiry > now_unix)
}

/// Mirror of [`HOST_FILTER`] in Rust, so the pure renderer is safe on its own.
fn wanted_host(host: &str) -> bool {
    let host = host.trim_start_matches('.');
    matches!(
        host,
        "youtube.com"
            | "www.youtube.com"
            | "music.youtube.com"
            | "google.com"
            | "www.google.com"
            | "accounts.google.com"
    ) || host.ends_with(".youtube.com")
        || host.ends_with(".google.com")
}

/// A field is usable when it cannot split or terminate its Netscape line.
fn well_formed(field: &str) -> bool {
    !field.contains(['\t', '\n', '\r', '\0'])
}

fn read_moz_cookies(path: &Path, now_unix: i64) -> MlResult<Vec<FirefoxCookie>> {
    with_read_only(path, |conn| {
        if !has_moz_cookies(conn)? {
            return Err(MlError::NotFound(format!(
                "{} is not a Firefox cookie database",
                path.display()
            )));
        }
        // A partitioned cookie belongs to one top-level site and must not be
        // replayed as a global one; skip the flag when this Firefox is too old
        // (or new) to have the column.
        let partitioned = if has_column(conn, "moz_cookies", "isPartitionedAttributeSet")? {
            "AND isPartitionedAttributeSet = 0"
        } else {
            ""
        };
        let sql = format!(
            "SELECT host, path, name, value, expiry, isSecure, isHttpOnly FROM moz_cookies \
             WHERE {HOST_FILTER} AND (expiry = 0 OR expiry > ?1) {partitioned}"
        );
        let mut stmt = conn.prepare(&sql).map_err(store_error)?;
        let rows = stmt
            .query_map([now_unix], |row| {
                Ok(FirefoxCookie {
                    host: row.get(0)?,
                    path: row.get(1)?,
                    name: row.get(2)?,
                    value: row.get(3)?,
                    expiry: row.get(4)?,
                    secure: row.get::<_, i64>(5)? != 0,
                    http_only: row.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?;
        Ok(rows)
    })
}

/// Open `path` read-only and run `query` on it, retrying through
/// `?immutable=1` when the first attempt fails (a locked WAL database, or a
/// browser that holds an exclusive lock).
fn with_read_only<T>(path: &Path, query: impl Fn(&Connection) -> MlResult<T>) -> MlResult<T> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    if let Ok(conn) = Connection::open_with_flags(path, flags) {
        if let Ok(value) = query(&conn) {
            return Ok(value);
        }
    }
    // `immutable=1` skips locking entirely, which is what makes a WAL database
    // openable while Firefox owns it.
    let conn = Connection::open_with_flags(file_uri(path), flags).map_err(store_error)?;
    query(&conn)
}

fn has_moz_cookies(conn: &Connection) -> MlResult<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'moz_cookies'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(store_error)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> MlResult<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(store_error)?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(store_error)?;
    for name in names {
        if name.map_err(store_error)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `file:<percent-encoded path>?immutable=1`, the URI SQLite wants for a path
/// that may contain spaces, `?` or `#`.
fn file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut uri = String::with_capacity(raw.len() + 24);
    uri.push_str("file:");
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(host: &str, name: &str, value: &str, expiry: i64) -> FirefoxCookie {
        FirefoxCookie {
            host: host.to_string(),
            path: "/".to_string(),
            name: name.to_string(),
            value: value.to_string(),
            expiry,
            secure: true,
            http_only: false,
        }
    }

    #[test]
    fn host_filter_accepts_youtube_google_and_their_subdomains() {
        for host in [
            ".youtube.com",
            "youtube.com",
            "music.youtube.com",
            ".music.youtube.com",
            "www.youtube.com",
            ".google.com",
            "accounts.google.com",
            ".accounts.google.com",
        ] {
            assert!(wanted_host(host), "{host}");
        }
        for host in [
            "example.com",
            "notyoutube.com",
            "youtube.com.evil.test",
            "google.com.evil.test",
            "",
        ] {
            assert!(!wanted_host(host), "{host}");
        }
    }

    #[test]
    fn netscape_lines_carry_the_flags_and_expiry() {
        let mut http_only = cookie(".youtube.com", "SID", "s3cret", 1_900_000_000);
        http_only.http_only = true;
        let mut plain = cookie("music.youtube.com", "PREF", "tz=1", 0);
        plain.secure = false;
        let rows = [plain, http_only];
        let text = netscape_from_rows(&rows, 1_800_000_000);
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with('#') || line.starts_with("#HttpOnly_"))
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1900000000\tSID\ts3cret"
        );
        assert_eq!(
            lines[1], "music.youtube.com\tFALSE\t/\tFALSE\t0\tPREF\ttz=1",
            "the dotted host sorts first"
        );
        assert!(text.starts_with(NETSCAPE_HEADER));
    }

    #[test]
    fn expired_rows_are_dropped_and_session_cookies_kept() {
        let rows = [
            cookie(".youtube.com", "old", "x", 100),
            cookie(".youtube.com", "edge", "x", 1_000),
            cookie(".youtube.com", "session", "x", 0),
            cookie(".youtube.com", "future", "x", 1_001),
        ];
        let text = netscape_from_rows(&rows, 1_000);
        let names: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split('\t').nth(5))
            .collect();
        assert_eq!(
            names,
            vec!["future", "session"],
            "the boundary itself is expired; equal hosts order by name"
        );
    }

    #[test]
    fn foreign_hosts_and_malformed_fields_never_reach_the_output() {
        let rows = [
            cookie("example.com", "tracker", "x", 0),
            cookie(".youtube.com", "good", "x", 0),
            cookie(".youtube.com", "tab\tin", "x", 0),
            cookie(".youtube.com", "new\nline", "x", 0),
            cookie(".youtube.com", "value", "a\tb", 0),
            cookie(".youtube.com", "", "x", 0),
        ];
        let text = netscape_from_rows(&rows, 0);
        let names: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split('\t').nth(5))
            .collect();
        assert_eq!(names, vec!["good"]);
        assert!(!text.contains("tracker"));
        // Every emitted line has exactly the seven tabs-separated fields.
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            assert_eq!(line.split('\t').count(), 7, "{line}");
        }
    }

    #[test]
    fn output_is_deterministic() {
        let a = cookie("music.youtube.com", "b", "1", 0);
        let b = cookie(".youtube.com", "a", "2", 0);
        let c = cookie(".youtube.com", "a", "3", 0);
        let c = FirefoxCookie {
            path: "/x".to_string(),
            ..c
        };
        assert_eq!(
            netscape_from_rows(&[a.clone(), b.clone(), c.clone()], 0),
            netscape_from_rows(&[c, b, a], 0)
        );
    }

    #[test]
    fn cookie_debug_never_shows_the_value() {
        let row = cookie(".youtube.com", "SID", "super-secret", 0);
        let rendered = format!("{row:?}");
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("redacted"));
        assert!(rendered.contains("SID"));
    }

    #[test]
    fn file_uri_escapes_anything_sqlite_would_misread() {
        assert_eq!(
            file_uri(Path::new("/home/a b/cookies.sqlite")),
            "file:/home/a%20b/cookies.sqlite?immutable=1"
        );
        assert_eq!(
            file_uri(Path::new("/tmp/x?y#z%")),
            "file:/tmp/x%3Fy%23z%25?immutable=1"
        );
    }
}
