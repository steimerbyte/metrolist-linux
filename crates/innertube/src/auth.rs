//! Cookie-backed account authentication for WEB_REMIX.
//!
//! The transport already attaches `Cookie` + `Authorization` +
//! `X-Goog-AuthUser` + `context.user.onBehalfOfUser` when a request opts into
//! `set_login` (`context::prepare`, measured against upstream
//! `SearchAuthenticationTest`). This module owns the *state* — which cookies,
//! which channel — plus the endpoints that only answer when signed in:
//! `account/account_menu` and the `FEmusic_liked_*` library browsers.
//!
//! Nothing here logs or formats a cookie value.

use std::path::PathBuf;

use ml_core::{Album, ArtistItem, LibraryPage, MlError, MlResult, Playlist, Song, YtItem};
use serde_json::{json, Value};

use crate::context::{ClientKind, ClientVersions};
use crate::cookies::{self, CookieJar};
use crate::parse::renderers::{carousel_items, grid_items, shelf_items};
use crate::parse::{continuation as parse_continuation, shelf_continuation};
use crate::InnertubeClient;

/// Versions for one request.
///
/// A `ytcfg` scraped without cookies carries the visitor id of no session, and
/// the API answers an authenticated request that claims it as anonymous
/// (measured 2026-11-02: same cookies, same `SAPISIDHASH`, the only change
/// `X-Goog-Visitor-Id`/`client.visitorData` → signed-out menu instead of
/// `activeAccountHeaderRenderer`). Authenticated requests therefore send only an
/// explicitly configured/discovered visitor, never the anonymous scrape.
pub(crate) fn request_versions(versions: &ClientVersions, set_login: bool) -> ClientVersions {
    let mut versions = versions.clone();
    if set_login {
        // `prepare` prefers `config.visitor_data` (the discovered, session-bound
        // one); the scraped field must never leak into a credentialled request.
        versions.visitor_data = None;
    }
    versions
}

/// Signed-in identity from `account/account_menu`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub name: String,
    /// Masked by [`AccountInfo::masked`] before display/logging.
    pub email: Option<String>,
    pub channel_handle: Option<String>,
    pub thumbnail_url: Option<String>,
    /// `dataSyncId` before its `||` separator, when the session carries one.
    pub channel_id: Option<String>,
}

impl AccountInfo {
    /// Same shape with every identifying string masked.
    pub fn masked(&self) -> Self {
        Self {
            name: cookies::mask(&self.name),
            email: self.email.as_deref().map(cookies::mask_email),
            channel_handle: self.channel_handle.as_deref().map(cookies::mask),
            thumbnail_url: self.thumbnail_url.clone(),
            channel_id: self.channel_id.as_deref().map(cookies::mask),
        }
    }
}

/// Credential state with no secret in it: counts, flags and the channel index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthSnapshot {
    pub authenticated: bool,
    pub cookie_count: usize,
    pub has_sapisid: bool,
    pub has_data_sync_id: bool,
    pub auth_user: Option<String>,
}

/// Library browse ids, copied from upstream `SyncUtils`/`AccountViewModel`.
pub const LIBRARY_PLAYLISTS: &str = "FEmusic_liked_playlists";
pub const LIBRARY_ALBUMS: &str = "FEmusic_liked_albums";
pub const LIBRARY_ARTISTS: &str = "FEmusic_library_corpus_artists";
pub const LIBRARY_SONGS: &str = "FEmusic_liked_videos";
pub const LIBRARY_UPLOADS: &str = "FEmusic_library_privately_owned_tracks";
pub const LIBRARY_RELEASES: &str = "FEmusic_library_privately_owned_releases";
pub const LIBRARY_LANDING: &str = "FEmusic_library_landing";

/// One pass over the signed-in library, ready for `ml_store` upserts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibrarySnapshot {
    pub playlists: Vec<Playlist>,
    pub albums: Vec<Album>,
    pub artists: Vec<ArtistItem>,
    pub liked_songs: Vec<Song>,
}

impl InnertubeClient {
    // ------------------------------------------------------------------ state

    /// Install a ready-made `Cookie:` header (raw, Netscape or JSON blob).
    /// Expiry and domain filtering are [`CookieJar`]'s job.
    pub fn set_cookie_header(&mut self, header: impl Into<String>) {
        let header = header.into();
        self.replace_cookie_jar(CookieJar::parse(&header));
    }

    /// Install a parsed jar, filtered for `music.youtube.com`.
    pub fn set_cookie_jar(&mut self, jar: &CookieJar) {
        self.replace_cookie_jar(jar.clone());
    }

    /// Snapshot of the live jar (rotations included).
    pub fn cookie_jar(&self) -> CookieJar {
        self.cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Where `Set-Cookie` rotations are persisted (0600) after each response.
    /// `None` disables persistence; the jar still rotates in memory.
    pub fn set_session_path(&mut self, path: Option<PathBuf>) {
        *self
            .session_path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = path;
    }

    /// Replace the jar and keep the `ClientConfig` seed in sync.
    pub(crate) fn replace_cookie_jar(&mut self, jar: CookieJar) {
        let header = (!jar.is_empty()).then(|| jar.header());
        *self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = jar;
        self.config.cookie = header;
    }

    /// Current `Cookie:` header, or `None` when the jar is empty.
    pub(crate) fn cookie_header(&self) -> Option<String> {
        let jar = self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (!jar.is_empty()).then(|| jar.header())
    }

    /// Merge every `Set-Cookie` of a response into the live jar and persist the
    /// session file. A persist failure never fails the API call.
    pub(crate) fn absorb_response_cookies(
        &self,
        headers: &reqwest::header::HeaderMap,
        host: &str,
        path: &str,
    ) {
        let set_cookies: Vec<String> = headers
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_string))
            .collect();
        if set_cookies.is_empty() {
            return;
        }
        let session_path = self
            .session_path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let mut jar = self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        jar.merge(&set_cookies, host, path, cookies::now_unix());
        if let Some(path) = session_path {
            let _ = jar.save(&path);
        }
    }

    /// Set the channel identity sent as `onBehalfOfUser`/`X-Goog-AuthUser`.
    /// `data_sync_id` must be the **delegated** id (`DATASYNC_ID` first part, only
    /// for a secondary channel; `None` for a primary one); `auth_user` is the
    /// channel index. Passing a primary user session id makes the API answer 401.
    pub fn set_login_identity(&mut self, data_sync_id: Option<String>, auth_user: Option<String>) {
        self.config.data_sync_id = data_sync_id;
        self.config.auth_user = auth_user;
    }

    /// Sign out: drops cookies, `onBehalfOfUser` and `X-Goog-AuthUser`, and
    /// removes the session file when one was configured.
    pub fn clear_auth(&mut self) {
        self.config.cookie = None;
        self.config.data_sync_id = None;
        self.config.auth_user = None;
        self.replace_cookie_jar(CookieJar::default());
        let session_path = self
            .session_path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(path) = session_path {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Upstream `LoginScreen` gate: a session is signed in with `SAPISID`.
    pub fn is_authenticated(&self) -> bool {
        self.cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .sapisid()
            .is_some()
    }

    /// Masked credential state; safe to log.
    pub fn auth_snapshot(&self) -> AuthSnapshot {
        let jar = self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        AuthSnapshot {
            authenticated: jar.sapisid().is_some(),
            cookie_count: jar.len(),
            has_sapisid: jar.sapisid().is_some(),
            has_data_sync_id: self.config.data_sync_id.is_some(),
            auth_user: self.config.auth_user.clone(),
        }
    }

    fn require_auth(&self) -> MlResult<()> {
        if self.is_authenticated() {
            Ok(())
        } else {
            Err(MlError::AuthRequired)
        }
    }

    /// Read the signed-in values from the music.youtube.com HTML with the
    /// configured cookies: `DATASYNC_ID` (first `||` part becomes
    /// `onBehalfOfUser`, as upstream `LoginScreen` does), `VISITOR_DATA` and
    /// `SESSION_INDEX` (the channel index). Installs them on the client.
    ///
    /// One GET, read-only. Nothing is logged.
    pub async fn discover_session(&mut self) -> MlResult<SessionIdentity> {
        self.require_auth()?;
        let cookie = self.config.cookie.clone().unwrap_or_default();
        let response = self
            .http
            .get("https://music.youtube.com/")
            .header("Cookie", format!("SOCS=CAI; {cookie}"))
            .header("User-Agent", crate::context::UA_WEB)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|error| MlError::Network(error.to_string()))?;
        self.absorb_response_cookies(response.headers(), "music.youtube.com", "/");
        let status = response.status();
        if !status.is_success() {
            return Err(MlError::Http {
                status: status.as_u16(),
                message: "ytcfg session scrape failed".to_string(),
            });
        }
        let html = response
            .text()
            .await
            .map_err(|error| MlError::Network(error.to_string()))?;
        let identity = session_identity(&html);
        self.config.data_sync_id = identity.data_sync_id.clone();
        if identity.visitor_data.is_some() {
            self.config.visitor_data = identity.visitor_data.clone();
        }
        self.config.auth_user = identity.auth_user.clone();
        Ok(identity)
    }

    // --------------------------------------------------------------- account

    /// `POST account/account_menu` — the only endpoint upstream uses for the
    /// signed-in identity (`AccountMenuResponse.activeAccountHeaderRenderer`).
    pub async fn account_info(&self) -> MlResult<AccountInfo> {
        self.require_auth()?;
        let response = self
            .post(
                ClientKind::WebRemix,
                "account/account_menu",
                json!({}),
                true,
                None,
            )
            .await?;
        account_from_menu(&response, self.config.data_sync_id.as_deref())
    }

    // --------------------------------------------------------------- library

    /// One library tab; `tab_index` mirrors upstream `library(tabIndex)`.
    pub async fn library_tab(&self, browse_id: &str, tab_index: usize) -> MlResult<LibraryPage> {
        self.require_auth()?;
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "browseId": browse_id }),
                true,
                None,
            )
            .await?;
        Ok(library_page(&response, tab_index))
    }

    /// Tab 0 of a library browser.
    pub async fn library(&self, browse_id: &str) -> MlResult<LibraryPage> {
        self.library_tab(browse_id, 0).await
    }

    /// Next page of a library browser (grid or shelf continuation).
    pub async fn library_continuation(&self, token: &str) -> MlResult<LibraryPage> {
        self.require_auth()?;
        let response = self
            .post(
                ClientKind::WebRemix,
                "browse",
                json!({ "continuation": token }),
                true,
                None,
            )
            .await?;
        Ok(library_continuation_page(&response))
    }

    pub async fn library_playlists(&self) -> MlResult<LibraryPage> {
        self.library(LIBRARY_PLAYLISTS).await
    }

    pub async fn library_albums(&self) -> MlResult<LibraryPage> {
        self.library(LIBRARY_ALBUMS).await
    }

    pub async fn library_artists(&self) -> MlResult<LibraryPage> {
        self.library(LIBRARY_ARTISTS).await
    }

    /// Liked songs (`FEmusic_liked_videos`).
    pub async fn library_songs(&self) -> MlResult<LibraryPage> {
        self.library(LIBRARY_SONGS).await
    }

    /// Uploaded songs, second tab of the private-tracks browser.
    pub async fn library_uploads(&self) -> MlResult<LibraryPage> {
        self.library_tab(LIBRARY_UPLOADS, 1).await
    }

    /// `FEmusic_library_landing`: the mixed shelf upstream shows before a tab.
    pub async fn library_landing(&self) -> MlResult<LibraryPage> {
        self.library(LIBRARY_LANDING).await
    }

    /// First page of every library browser. `ml-innertube` must not depend on
    /// `ml-store`, so the caller performs the `upsert_*` writes (store already
    /// exposes `upsert_playlist`/`upsert_album`/`upsert_artist`/`upsert_songs`).
    pub async fn library_snapshot(&self) -> MlResult<LibrarySnapshot> {
        let playlists = self.library_playlists().await?;
        let albums = self.library_albums().await?;
        let artists = self.library_artists().await?;
        let liked = self.library_songs().await?;
        Ok(LibrarySnapshot {
            playlists: playlists.playlists,
            albums: albums.albums,
            artists: artists.artists,
            liked_songs: liked.songs,
        })
    }
}

/// Signed-in ytcfg values, as read from the music.youtube.com HTML.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionIdentity {
    /// Delegated session id (`DATASYNC_ID` before `||`, only when a second part
    /// exists), sent as `onBehalfOfUser`. `None` on a primary channel.
    pub data_sync_id: Option<String>,
    pub visitor_data: Option<String>,
    /// `SESSION_INDEX`, sent as `X-Goog-AuthUser`.
    pub auth_user: Option<String>,
}

/// Pure parser for the three ytcfg values; total, empty when absent.
pub fn session_identity(html: &str) -> SessionIdentity {
    // `DATASYNC_ID` is `delegated||user` on a secondary channel and `user||` on
    // the primary one (yt-dlp `_parse_data_sync_id`). Only the delegated half is
    // ever sent as `onBehalfOfUser`; putting a primary *user* session id there
    // is answered 401.
    let data_sync_id = quoted(html, "\"DATASYNC_ID\":\"")
        .or_else(|| quoted(html, "\"datasyncId\":\""))
        .and_then(|raw| raw.split_once("||"))
        .filter(|(delegated, user)| !delegated.is_empty() && !user.is_empty())
        .map(|(delegated, _)| delegated.to_string());
    SessionIdentity {
        data_sync_id,
        visitor_data: quoted(html, "\"VISITOR_DATA\":\"").map(str::to_string),
        auth_user: integer(html, "\"SESSION_INDEX\":").map(|index| index.to_string()),
    }
}

/// Value between `marker` and the next `"`; `None` when absent or empty.
fn quoted<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    (!rest[..end].is_empty()).then_some(&rest[..end])
}

fn integer(html: &str, marker: &str) -> Option<u32> {
    let start = html.find(marker)? + marker.len();
    let digits: String = html[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

// ---------------------------------------------------------------- account json

/// `actions[0].openPopupAction.popup.multiPageMenuRenderer.header
///  .activeAccountHeaderRenderer`; `data_sync_id` supplies the channel id.
pub fn account_from_menu(response: &Value, data_sync_id: Option<&str>) -> MlResult<AccountInfo> {
    // A 200 without this header is the signed-out compact menu, not a 401: the
    // server accepted the request but did not recognise the session.
    let missing = || MlError::Unavailable("account menu: signed-out response".to_string());
    let header = response
        .pointer(
            "/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer",
        )
        .ok_or_else(missing)?;
    let name = runs_text(header.get("accountName")).ok_or_else(missing)?;
    Ok(AccountInfo {
        name,
        email: runs_text(header.get("email")),
        channel_handle: runs_text(header.get("channelHandle")),
        thumbnail_url: header
            .pointer("/accountPhoto/thumbnails")
            .and_then(Value::as_array)
            .and_then(|thumbs| thumbs.last())
            .and_then(|thumb| thumb.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string),
        channel_id: channel_id(header, data_sync_id),
    })
}

fn runs_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    value
        .pointer("/runs/0/text")
        .or_else(|| value.get("simpleText"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// `UCH…||UC…` → `UCH…`; falls back to a `channelId` in the header.
fn channel_id(header: &Value, data_sync_id: Option<&str>) -> Option<String> {
    data_sync_id
        .and_then(|sync| sync.split("||").next())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| {
            header
                .pointer("/accountByline/browseEndpoint/browseId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

// ---------------------------------------------------------------- library json

/// Parse one library browse response. Tabs come from the single-column or
/// two-column wrapper; the first tab with renderers wins for `tab_index` 0.
pub fn library_page(response: &Value, tab_index: usize) -> LibraryPage {
    let tabs = browse_tabs(response);
    let tab = tabs.get(tab_index).or_else(|| tabs.first());
    let mut page = LibraryPage::default();
    let Some(sections) = tab.and_then(|tab| {
        tab.pointer("/tabRenderer/content/sectionListRenderer/contents")
            .and_then(Value::as_array)
    }) else {
        return page;
    };
    let mut items = Vec::new();
    let mut continuation = None;
    collect_sections(sections, &mut items, &mut continuation);
    fill(&mut page, items);
    page.continuation = continuation;
    page
}

/// Parse a `browse(continuation: …)` response for a library page.
pub fn library_continuation_page(response: &Value) -> LibraryPage {
    let mut page = LibraryPage::default();
    let Some(contents) = response.get("continuationContents") else {
        return page;
    };
    let mut items = Vec::new();
    let mut continuation = None;
    if let Some(grid) = contents.get("gridContinuation") {
        items.extend(grid_items(grid));
        continuation = shelf_continuation(grid.get("contents"))
            .or_else(|| parse_continuation(grid.get("continuations")));
    }
    for key in ["musicShelfContinuation", "musicPlaylistShelfContinuation"] {
        if let Some(shelf) = contents.get(key) {
            items.extend(shelf_items(shelf));
            continuation = continuation.or_else(|| {
                shelf_continuation(shelf.get("contents"))
                    .or_else(|| parse_continuation(shelf.get("continuations")))
            });
        }
    }
    fill(&mut page, items);
    page.continuation = continuation;
    page
}

fn browse_tabs(response: &Value) -> &[Value] {
    let single =
        crate::parse::array_at(response, "/contents/singleColumnBrowseResultsRenderer/tabs");
    if single.is_empty() {
        crate::parse::array_at(response, "/contents/twoColumnBrowseResultsRenderer/tabs")
    } else {
        single
    }
}

const SHELF_RENDERERS: [&str; 4] = [
    "gridRenderer",
    "musicShelfRenderer",
    "musicPlaylistShelfRenderer",
    "musicCarouselShelfRenderer",
];

fn collect_sections(
    sections: &[Value],
    items: &mut Vec<YtItem>,
    continuation: &mut Option<String>,
) {
    for section in sections {
        collect_renderers(std::slice::from_ref(section), items, continuation);
        collect_renderers(
            crate::parse::array_at(section, "/itemSectionRenderer/contents"),
            items,
            continuation,
        );
    }
}

fn collect_renderers(
    containers: &[Value],
    items: &mut Vec<YtItem>,
    continuation: &mut Option<String>,
) {
    for container in containers {
        for key in SHELF_RENDERERS {
            let Some(renderer) = container.get(key) else {
                continue;
            };
            let parsed = match key {
                "gridRenderer" => grid_items(renderer),
                "musicCarouselShelfRenderer" => carousel_items(renderer),
                _ => shelf_items(renderer),
            };
            items.extend(parsed);
            if continuation.is_none() {
                *continuation = shelf_continuation(renderer.get("contents"))
                    .or_else(|| parse_continuation(renderer.get("continuations")));
            }
        }
    }
}

fn fill(page: &mut LibraryPage, items: Vec<YtItem>) {
    for item in items {
        match item {
            YtItem::Song(song) => page.songs.push(song),
            YtItem::Album(album) => page.albums.push(album),
            YtItem::Playlist(playlist) => page.playlists.push(playlist),
            YtItem::Artist(artist) => page.artists.push(artist),
            YtItem::Episode(_) | YtItem::Video(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Value {
        json!({
            "actions": [{
                "openPopupAction": {
                    "popup": {
                        "multiPageMenuRenderer": {
                            "header": {
                                "activeAccountHeaderRenderer": {
                                    "accountName": { "runs": [{ "text": "Rahul Kumar" }] },
                                    "email": { "runs": [{ "text": "rahul@example.com" }] },
                                    "channelHandle": { "runs": [{ "text": "@rahulk" }] },
                                    "accountPhoto": { "thumbnails": [
                                        { "url": "https://yt3.example/s88", "width": 88 },
                                        { "url": "https://yt3.example/s176", "width": 176 }
                                    ]}
                                }
                            }
                        }
                    }
                }
            }]
        })
    }

    #[tokio::test]
    async fn auth_state_transitions_never_report_values() {
        let mut client = crate::InnertubeClient::new(crate::ClientConfig::default());
        assert!(!client.is_authenticated());
        assert_eq!(client.auth_snapshot(), AuthSnapshot::default());
        assert_eq!(client.account_info().await, Err(MlError::AuthRequired));

        client.set_cookie_jar(&CookieJar::parse("SAPISID=abc; SID=def"));
        client.set_login_identity(Some("UCHx||UCx".to_string()), Some("0".to_string()));
        let snapshot = client.auth_snapshot();
        assert!(snapshot.authenticated && snapshot.has_sapisid);
        assert_eq!(snapshot.cookie_count, 2);
        assert!(snapshot.has_data_sync_id);
        assert_eq!(snapshot.auth_user.as_deref(), Some("0"));
        assert!(
            !format!("{snapshot:?}").contains("abc"),
            "no value in Debug"
        );

        client.clear_auth();
        assert!(!client.is_authenticated());
        let cleared = client.auth_snapshot();
        assert_eq!(cleared.cookie_count, 0);
        assert!(!cleared.has_data_sync_id && cleared.auth_user.is_none());

        client.set_cookie_header("   ");
        assert!(!client.is_authenticated());
    }

    #[test]
    fn session_identity_reads_primary_and_secondary_datasync_ids() {
        let primary = r#"{"DATASYNC_ID":"USER-only||","VISITOR_DATA":"vd","SESSION_INDEX":0}"#;
        let identity = session_identity(primary);
        assert_eq!(identity.data_sync_id, None, "primary has no delegated id");
        assert_eq!(identity.visitor_data.as_deref(), Some("vd"));
        assert_eq!(identity.auth_user.as_deref(), Some("0"));

        let secondary = r#"{"datasyncId":"DELEGATED||USER","VISITOR_DATA":"vd"}"#;
        let identity = session_identity(secondary);
        assert_eq!(identity.data_sync_id.as_deref(), Some("DELEGATED"));
        assert_eq!(identity.auth_user, None);

        assert_eq!(session_identity(""), SessionIdentity::default());
        assert_eq!(
            session_identity(r#"{"DATASYNC_ID":"||"}"#).data_sync_id,
            None
        );
    }

    #[test]
    fn authenticated_requests_never_claim_the_anonymous_visitor() {
        let versions = ClientVersions {
            visitor_data: Some("anonymous-scrape".to_string()),
            ..ClientVersions::default()
        };
        assert_eq!(
            request_versions(&versions, true).visitor_data,
            None,
            "a credentialled request must not carry the cookie-less visitor"
        );
        assert!(
            request_versions(&versions, false).visitor_data.is_some(),
            "anonymous requests keep the scraped visitor"
        );
    }

    #[test]
    fn session_rotation_persists_and_reaches_the_next_header() {
        let directory = std::env::temp_dir().join(format!(
            "ml-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("cookies.txt");

        let mut client = crate::InnertubeClient::new(crate::ClientConfig::default());
        client.set_cookie_jar(&CookieJar::parse(
            ".youtube.com\tTRUE\t/\tTRUE\t0\t__Secure-1PSIDTS\told\n",
        ));
        client.set_session_path(Some(path.clone()));

        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static(
                "__Secure-1PSIDTS=new; Domain=.youtube.com; Path=/; Secure; HttpOnly",
            ),
        );
        client.absorb_response_cookies(
            &headers,
            "music.youtube.com",
            "/youtubei/v1/account/account_menu",
        );

        assert_eq!(
            client.cookie_jar().get("__Secure-1PSIDTS"),
            Some("new"),
            "the next request sees the rotation"
        );
        assert_eq!(
            client.cookie_header().as_deref(),
            Some("__Secure-1PSIDTS=new"),
            "post() builds its Cookie header from the live jar"
        );
        let persisted = CookieJar::load(&path).expect("session file");
        assert_eq!(persisted.get("__Secure-1PSIDTS"), Some("new"));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        client.clear_auth();
        assert!(!client.is_authenticated());
        assert!(!path.exists(), "sign-out removes the session file");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn account_menu_parses_identity_and_uses_largest_photo() {
        let info = account_from_menu(&menu(), Some("UCHxyz||UCxyz")).expect("parses");
        assert_eq!(info.name, "Rahul Kumar");
        assert_eq!(info.email.as_deref(), Some("rahul@example.com"));
        assert_eq!(info.channel_handle.as_deref(), Some("@rahulk"));
        assert_eq!(
            info.thumbnail_url.as_deref(),
            Some("https://yt3.example/s176")
        );
        assert_eq!(info.channel_id.as_deref(), Some("UCHxyz"));
        assert_eq!(account_from_menu(&menu(), None).unwrap().channel_id, None);
    }

    #[test]
    fn account_menu_without_a_signed_in_header_is_reported_as_signed_out() {
        for response in [json!({}), json!({"actions": []})] {
            assert_eq!(
                account_from_menu(&response, None),
                Err(MlError::Unavailable(
                    "account menu: signed-out response".to_string()
                ))
            );
        }
    }

    #[test]
    fn masked_account_never_shows_the_email() {
        let masked = account_from_menu(&menu(), Some("UCHxyz||UCxyz"))
            .unwrap()
            .masked();
        assert_eq!(masked.email.as_deref(), Some("r***l@example.com"));
        assert_eq!(masked.name, "R***r");
        assert_eq!(masked.channel_handle.as_deref(), Some("@***k"));
        assert_eq!(masked.channel_id.as_deref(), Some("U***z"));
        assert!(!format!("{masked:?}").contains("rahul"));
    }

    fn playlist_tile() -> Value {
        json!({
            "musicTwoRowItemRenderer": {
                "title": { "runs": [{ "text": "My Playlist" }] },
                "navigationEndpoint": {
                    "browseEndpoint": {
                        "browseId": "VLPL123",
                        "browseEndpointContextSupportedConfigs": {
                            "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_PLAYLIST" }
                        }
                    }
                },
                "subtitle": { "runs": [{ "text": "Playlist" }] }
            }
        })
    }

    fn album_tile() -> Value {
        json!({
            "musicTwoRowItemRenderer": {
                "title": { "runs": [{ "text": "Album" }] },
                "navigationEndpoint": {
                    "browseEndpoint": {
                        "browseId": "MPREb1",
                        "browseEndpointContextSupportedConfigs": {
                            "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" }
                        }
                    }
                },
                "subtitle": { "runs": [{ "text": "Artist" }] }
            }
        })
    }

    fn song_row() -> Value {
        json!({
            "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": "vid123" },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Song Title" }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                        { "text": "Artist", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCartist" } } },
                        { "text": " \u{2022} " },
                        { "text": "Album", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREbalbum" } } }
                    ] } } }
                ]
            }
        })
    }

    fn tab_page(tab: &str, renderer: Value) -> Value {
        json!({
            "contents": { (tab): { "tabs": [
                { "tabRenderer": { "content": { "sectionListRenderer": { "contents": [renderer] } } } }
            ] } }
        })
    }

    #[test]
    fn library_grid_tab_buckets_items_and_keeps_continuation() {
        let grid = json!({
            "gridRenderer": {
                "items": [playlist_tile()],
                "continuations": [{ "nextContinuationData": { "continuation": "GRID-TOKEN" } }]
            }
        });
        let page = library_page(&tab_page("twoColumnBrowseResultsRenderer", grid), 0);
        assert_eq!(page.playlists.len(), 1);
        assert_eq!(page.playlists[0].title, "My Playlist");
        assert_eq!(page.continuation.as_deref(), Some("GRID-TOKEN"));
        assert!(page.albums.is_empty() && page.songs.is_empty() && page.artists.is_empty());
    }

    #[test]
    fn library_shelf_tab_buckets_songs() {
        let shelf = json!({
            "musicShelfRenderer": {
                "contents": [song_row()],
                "continuations": [{ "nextContinuationData": { "continuation": "SHELF-TOKEN" } }]
            }
        });
        let page = library_page(&tab_page("singleColumnBrowseResultsRenderer", shelf), 0);
        assert_eq!(page.songs.len(), 1, "{:?}", page.songs);
        assert_eq!(page.songs[0].id, "vid123");
        assert_eq!(page.continuation.as_deref(), Some("SHELF-TOKEN"));
    }

    #[test]
    fn empty_tab_and_missing_contents_are_empty_pages() {
        assert_eq!(library_page(&json!({}), 0), LibraryPage::default());
        let response = json!({"contents": {"twoColumnBrowseResultsRenderer": {"tabs": []}}});
        assert_eq!(library_page(&response, 0), LibraryPage::default());
    }

    #[test]
    fn continuation_page_parses_grid_and_shelf_shapes() {
        let grid = json!({
            "continuationContents": { "gridContinuation": {
                "items": [album_tile()],
                "continuations": [{ "nextContinuationData": { "continuation": "NEXT" } }]
            } }
        });
        let page = library_continuation_page(&grid);
        assert_eq!(page.albums.len(), 1);
        assert_eq!(page.continuation.as_deref(), Some("NEXT"));

        let shelf = json!({
            "continuationContents": { "musicShelfContinuation": {
                "contents": [song_row()]
            } }
        });
        let page = library_continuation_page(&shelf);
        assert_eq!(page.songs.len(), 1);
        assert_eq!(page.continuation, None);

        assert_eq!(
            library_continuation_page(&json!({})),
            LibraryPage::default()
        );
    }
}
