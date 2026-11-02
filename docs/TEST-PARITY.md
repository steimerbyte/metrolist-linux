> Copied from a local `arch/test-parity.md`; `arch/` paths below refer to that
> read-only analysis/probe directory, which is not part of this repository.

# Upstream test parity → Rust test requirements

Source: Metrolist (https://github.com/MetrolistGroup/Metrolist), commit
`ed5d480f8f2e75ff3e7fbf76731ed704ca4c6dc6`. Read-only copies of every test file are in
`arch/upstream-tests/` (see its `README.md`).

**Counts (verified, not estimated):**

```sh
cd metrolist
find app/src/test/kotlin innertube/src/test/kotlin -name '*.kt' | wc -l   # 53
grep -rho '@Test' app/src/test/kotlin innertube/src/test/kotlin | wc -l   # 235
for f in $(find app/src/test/kotlin innertube/src/test/kotlin -name '*.kt'); do \
  printf '%3d %s\n' "$(grep -c '@Test' "$f")" "$f"; done                  # per-file table below
```

No `@Ignore`/`@Disabled` anywhere: 235/235 tests are live. All are local JVM tests; DB
tests are Robolectric + Room in-memory (`@Config(sdk = [33])`), network tests use Ktor
`MockEngine`. Nothing needs a device, so every non-Android behaviour below is portable
as-is.

## Summary by domain

| Domain | Files | Tests | Rust verdict |
|---|---:|---:|---|
| ytmusic API parsing (`innertube`) | 11 | 36 | mostly `PORT` |
| store / SQLite (`db`) | 9 | 23 | `ADAPT` (Room → rusqlite/sqlx), pure parts `PORT` |
| playback core | 9 | 47 | mixed: `PORT` for cache/gain/URL/matching, `ADAPT`/`N/A` for media3 surfaces |
| utils | 6 | 42 | `PORT` (PlaylistSync 33 tests are the core sync contract) |
| ui helpers (`ui/player`, `ui/utils`) | 2 | 11 | `PORT` (both are pure string/cause-chain logic) |
| discord | 4 | 24 | `PORT` (token store `ADAPT`) |
| betterlyrics TTML | 1 | 14 | `PORT` |
| lyrics (LRC timing + romanization) | 2 | 11 | `PORT` |
| listen together | 4 | 9 | `PORT` |
| viewmodels | 2 | 9 | `PORT` |
| api (OpenRouter) | 1 | 6 | `PORT` |
| Android extensions | 2 | 3 | `ADAPT` / `N/A` |
| **total** | **53** | **235** | |

(Sum check: 36+23+47+42+11+24+14+11+9+9+6+3 = 235.)

Legend: **PORT** = pure logic, translate directly. **ADAPT** = same behaviour, different
platform mechanism (Room→SQL, media3→own player, Keystore→OS secret storage).
**N/A** = Android-only surface (Compose, MediaItem, ContentResolver, APK updater,
Android Auto) — do not port the mechanism, port the rule if one is stated.

Module paths below are proposals against a workspace of `ytmusic` plus
`metrolist-{store,player,lyrics,sync,discord,ai,util}`. What matters is one Rust module
per upstream behaviour; confirm names once the workspace exists.

Placement convention: unit logic + its `#[cfg(test)] mod tests` inside the owning module;
multi-case parsers get `tests/fixtures/*.json` next to the module and one fixture per
upstream inline literal (the JSON strings in the Kotlin tests are the fixture set — copy
them verbatim, they are what real responses look like).

---

# 1. ytmusic API parsing — 11 files, 36 tests

Crate `ytmusic`. All JSON literals in these tests become fixtures.

### innertube/src/test/kotlin/com/metrolist/innertube/pages/PageHelperTest.kt — `ytmusic::pages::helper` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `duration is not parsed as an artist` | `3:42` alone never becomes an artist | `extract_artists([Run("3:42", None)]) == []` |
| `search metadata is not parsed as an artist` | type/duration/play-count runs yield no artists, duration still parses | `artists == []`, `extract_duration == 184` |
| `linked artist is found after search type label` | linked run after a type label is an artist | `[Artist{name:"Lupus Nocte", id:"UC123"}]` |
| `unlinked metadata is never promoted to artists` | `Artist • Album • 3:42` all unlinked → none | `[]` |
| `commas between linked artists are not parsed as artists` | separator `", "` skipped, both linked names kept | `[UC123, UC456]` in order |
| `linked artist does not authorize unlinked neighbors` | a link does not bless adjacent unlinked runs | only `Primary Artist/UC123` |
| `localized counts and labels cannot become artists but linked names remain whole` | es/zh/ja labels (`canción`, `14 m reproducciones`, `3,3 M reproducciones`, `歌曲`, `1億 回再生`) are never artists; `&` inside one linked name is kept whole; blank linked run dropped; album-typed link rejected | `[21 Savage/UC21, "10 Years and Song & Dance"/UC10]`, no `UCblank`, no `MPREb_album` |
| `user channel link is trusted but album typed link is not` | `MUSIC_PAGE_TYPE_USER_CHANNEL` accepts any id; `MUSIC_PAGE_TYPE_ALBUM` rejected | only `Song/MPLA_channel` |
| `typed artist endpoint does not require a channel id` | `MUSIC_PAGE_TYPE_ARTIST` accepts `MPLA…` (no `UC` prefix requirement) | `[Artist("Artist","MPLA123")]` |

### innertube/src/test/kotlin/com/metrolist/innertube/pages/ParserRegressionTest.kt — `ytmusic::pages::{related,summary,search,podcast,home,artist}` — **PORT**

`json { ignoreUnknownKeys = true; explicitNulls = false }` — Rust needs
`#[serde(default)]` everywhere and no `deny_unknown_fields`.

| Test | Pins | Must assert |
|---|---|---|
| `related album retains menu-only artist id and other metadata` | when subtitle has no artist, the menu's `ARTIST` item supplies `browseId` with an **empty name** | `artists == [Artist{"", "UCartist"}]`, `year == 2024`, thumbnail, playlistId |
| `related playlist retains unlinked owner in last subtitle section` | playlist owner = last subtitle section even unlinked | `author == Artist("Owner & Friends", None)`, `song_count_text == "12 songs"` |
| `summary playlist retains unlinked owner` | card-shelf playlist takes subtitle as owner | `PlaylistItem.author` |
| `summary podcast retains unlinked host` | `PODCAST_SHOW_DETAIL_PAGE` card → host = subtitle | `PodcastItem.author` |
| `search playlist retains unlinked owner with or without type label` | row playlist owner found with and without `"Playlist • "` prefix | both variants → owner |
| `podcast episode row retains unlinked channel byline` | row episode byline = first subtitle section; `Yesterday • 3:12` | `author`, `duration == 192`, `publish_date_text == "Yesterday"` |
| `home and artist episode tiles retain plain and podcast-linked bylines` | two-row episode tile in a carousel: byline kept both plain and podcast-linked, via `HomePage::Section` and `ArtistPage::from_section_list` | both → `EpisodeItem.author` |

### innertube/src/test/kotlin/com/metrolist/innertube/pages/PlaylistPageTest.kt — `ytmusic::pages::playlist` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `legacy detail subtitle has an owner in its second section` | legacy detail subtitle `Playlist • Owner & Friends • 14 songs` → owner is the **second** section (unlinked, keeps `&`); a single localized run → `None`; a second section that is only whitespace → `None` | `Artist("Owner & Friends", None)`; `None`; `None` |

### innertube/src/test/kotlin/com/metrolist/innertube/pages/AlbumPageTest.kt — `ytmusic::pages::album` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `uploaded compilation track keeps its tagged performer instead of album artist` | uploaded compilation (`FEmusic_library_privately_owned_release_detail_upload`) keeps the row's tagged performer | `artists == [Tagged Performer/None]`, `album.id == browseId`, thumbnail retained |
| `art track performer takes precedence over label in known album context` | `MUSIC_VIDEO_TYPE_ATV` row performer wins only for an uploaded (`OLAK5uy_`) playlist; empty subtitle or regular playlist → album artist; linked performer keeps its channel id | three-way branch; `[Artist("10 Years and Song", None)]` / label / `[Linked Performer/UCperformer]` |

### innertube/src/test/kotlin/com/metrolist/innertube/pages/PodcastPageTest.kt — `ytmusic::pages::podcast` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `episode credits require a trusted artist or podcast link` | plain byline is not a credit; a podcast-show link (in `second_subtitle` or `secondary_subtitle`) validates it; linked `UC` artist accepted | `[]` for ambiguous/`"New episodes"`, `[Known Show]` for link, `[Song & Dance/UCartist]` for linked artist |
| `saved episodes retain the channel byline without assigning podcast or album ids to artists` | byline kept unlinked; podcast link keeps name only (no id); album link never becomes artist; linked UC wins; `None` input | 6 assertions incl. `extract_podcast_byline(None) == []` |
| `menu podcast link validates a plain show byline without using menu label as artist` | a `PODCAST` menu item validates the byline; **menu label is never the artist name**; without menu → no credit; localized metadata still rejected | `[Known Show]`, `[]` twice |

### innertube/src/test/kotlin/com/metrolist/innertube/pages/SearchPageTest.kt — `ytmusic::pages::{search,summary,artist,library}` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `card song inherits artist when response only contains type and duration` | card song with no artist falls back to caller-supplied artists | `[Lupus Nocte/UC123]`, duration 184 |
| `localized metadata only songs stay playable across search and artist continuations` | localized metadata-only rows stay songs with **no** artists in search, suggestion, artist-items, shelf, carousel and two-row forms; id still parses | 8 assertions, all `artists == []`, `id == "videoId"` |
| `linked artists and card context survive localized metadata` | linked artist survives in every surface; fallback applies only to `SearchPage::to_yt_item`; card summary does **not** take fallback | repeated `[Song & Dance/UCsong]`; card with unlinked subtitle → `[]` |
| `uploaded library artist columns retain tagged names without relaxing generic metadata parsing` | `isUploaded` library rows keep unlinked tagged names after a `", "`; without the flag they are dropped; plain tagged-only row kept when uploaded; metadata still never artists | 4 assertions |
| `library podcast bylines and playlist owners retain trusted credits` | library podcast `author` from subtitle for plain and podcast-linked bylines; a playlist-typed navigation turns the row into `PlaylistItem` with same owner | both byline forms, both item kinds |

### innertube/src/test/kotlin/com/metrolist/innertube/models/MusicResponsiveHeaderRendererTest.kt — `ytmusic::models` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `buttons may be omitted` | `buttons` is optional and defaults to empty | `{"title":{"runs":[]},"subtitle":{"runs":[]}}` decodes, `buttons.is_empty()` |

### innertube/src/test/kotlin/com/metrolist/innertube/PlaybackRegistrationTest.kt — `ytmusic::playback_stats` — **PORT**

Needs an injectable HTTP transport (see §13) and a mocked player endpoint returning
`playbackTracking.videostatsPlaybackUrl`.

| Test | Pins | Must assert |
|---|---|---|
| `eachListenFetchesFreshAuthenticatedTrackingWithoutAStreamCache` | one `/player` per listen, CPN passed on the stats GET, never reused from a stream cache; Authorization on **every** request | path order `[player, stats/playback, player, stats/playback]`; `cpn=first-play`, `cpn=second-play` |
| `privacyIsCheckedBeforeAndAfterFetchingTracking` | history disabled → no request at all; no auth cookie → no request; disabled mid-flight → abort after `/player` | `requests.is_empty()` then exactly 1 |
| `accountChangeDuringTrackingFetchPreventsRegistration` | cookie changed during fetch cancels the registration (coroutine cancelled) | only `/player` seen, task cancelled |
| `rejectsUntrustedTrackingUrls` | only `https://s.youtube.com:443/api/stats/playback` accepted; reject other host, `http`, non-443 port, embedded userinfo, other path | each case → `IllegalArgumentException` (`InvalidTrackingUrl`), exactly 1 request |

### innertube/src/test/kotlin/com/metrolist/innertube/SearchAuthenticationTest.kt — `ytmusic::auth_context` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `explicitSearchAuthenticationDoesNotLeakIntoLaterLookups` | `setLogin=true` sends `Authorization`, `Cookie: SAPISID=…`, `X-Goog-AuthUser`, body `context.user.onBehalfOfUser = dataSyncId`; `setLogin=false` and unspecified send none of them; per-request flag, never sticky | 5 requests: first 3 authed, last 2 fully anonymous |
| `automaticSearchesOmitAccountIdentityByDefault` | even with cookies configured, artist lookups / flags / continuations / suggestions are anonymous | 4 requests, all headers and `onBehalfOfUser` absent |

### innertube/src/test/kotlin/com/metrolist/innertube/PodcastEnrichmentTest.kt — `ytmusic::podcast_enrichment` — **ADAPT**

Upstream reaches into `YouTube`/`InnerTube` singletons by reflection; in Rust make the
enrichment a free function taking `&dyn InnerTubeApi` + episode list.

| Test | Pins | Must assert |
|---|---|---|
| `new episodes enrich missing artists and preserve linked or unresolved episodes` | `newEpisodes` fetches `/browse`, then `/next` only for episodes lacking artists; resolved owner becomes the artist; linked stays; failures (500) and unresolved (`contents:{}`) yield `[]`; each id looked up **once** | ids `[missing, linked, unknown, failed]` in order, `[Artist("Resolved Host","UChost")]`, linked kept, `[]` twice, lookups exactly one per unresolved id |

### innertube/src/test/kotlin/com/metrolist/innertube/UploadProgressInputStreamTest.kt — `ytmusic::upload` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `reports streamed bytes without exceeding completion` | progress callback is monotonic and ends at exactly 1.0 while bytes pass through unchanged | `read_to_end == input`, non-decreasing, last `== 1.0` |

Port as `ProgressReader<R: Read>` over `Read::read` (+ a `Seek`-free path); test with a
`Cursor`.

---

# 2. store / SQLite — 9 files, 23 tests

Target crate `metrolist-store`, sqlx or rusqlite. Robolectric+Room becomes a real SQLite
file or `:memory:`; `runBlocking` becomes a plain `#[test]`. The Room *queries* are the
specification — copy the SQL from `DatabaseDao.kt`, do not re-derive it.

### app/src/test/kotlin/com/metrolist/music/db/AddSongsToPlaylistTest.kt — `store::db::playlist` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `bulk add is safe when called from main thread` | add is callable from any thread (no main-thread assertion in Rust; keep "works under the runtime" as the point) | song ids `["song"]` after add |
| `append inserts after sparse playlist positions` | append position = `max(position)+1`, never `count` (positions 0 and 5 → new = 6) | ordered ids `[first, last, new]` |
| `bulk add participates in an outer suspending transaction` | bulk add runs inside the caller's transaction and rolls back with it | after rollback `[]` |

### app/src/test/kotlin/com/metrolist/music/db/HistoryDatabaseDeduplicationTest.kt — `store::db::history` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `history hydrates only the latest play per song in each displayed period` | one row per (song, period bucket: today / yesterday / this week / last week / month), newest event id per bucket, overall `ORDER BY event.id DESC` | exact id list `[9,8,6,5,4,2]` |
| `repeated plays do not create repeated song relation graphs` | 2000 plays of one song collapse to 1 row (latest) | `len == 1`, id `2000` |
| `latest event lookup stays bounded to the newest play` | `latestEvent` = `ORDER BY rowId DESC LIMIT 1` | id `2` |

Period bucketing is SQL (`CASE WHEN timestamp >= :x`); in Rust keep the same precedence
order: tomorrow → today → yesterday → thisMonday → lastMonday → `strftime('%Y-%m')`, with
the first bucket labelled `this_week` (yes, the `tomorrowStart` branch is spelled
`this_week` upstream — preserve or rename deliberately, but keep "one bucket per period").

### app/src/test/kotlin/com/metrolist/music/db/ArtistPageCacheTest.kt — `store::db::artist` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `reading a song leaves the cached artist page on disk` | bulk song reads use a projection that **excludes** `cachedPageJson` | nested artist has `cached_page_json == None` |
| `reading a song keeps every other artist column` | projection keeps id, name, thumbnailUrl, channelId, lastUpdateTime, bookmarkedAt, isLocal=false, isPodcastChannel=true | 8 fields |
| `reading an album leaves the cached artist page on disk` | same for `albumWithSongs` | `None` |
| `the artist screen still reads the cached page` | the artist screen query does return it | `== CACHED_PAGE` |
| `storing a thumbnail found while playing keeps the cached page` | `updateArtistThumbnail` updates only `thumbnailUrl` | thumbnail replaced, page intact |

This is the "don't hydrate kilobytes per song" invariant: any Rust query returning artists
in bulk must select an explicit column list. Encode as two DAO functions (bulk vs detail)
and one test that fails if a `SELECT *` reappears.

### app/src/test/kotlin/com/metrolist/music/db/PlaylistDuplicatesBatchedTest.kt — `store::db::playlist::duplicates` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `duplicate lookup stays correct across variable limit batches` | batch size 500 with `batch+1 <= 999` (SQLite variable limit), chunking on distinct ids, empty input → empty | full playlist → 1200 ids == input set; partial → 600 evens; `[]` for empty input |

### app/src/test/kotlin/com/metrolist/music/db/MusicDatabaseMigrationTest.kt — `store::db::migration` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `21 to 24 shortcut adds every version 24 column` | fresh v21 → v24 adds all columns | `libraryAddToken`/`libraryRemoveToken` exist with **NULL** default; `romanizeLyrics` default `'true'`; `isDownloaded` default `'0'`; `isUploaded` `'false'` on song **and** album; `playlist.thumbnailUrl` present |
| `22 to 24 shortcut tolerates partially added columns` | idempotent when `playlist.thumbnailUrl` already exists | same assertions, no error |

### app/src/test/kotlin/com/metrolist/music/db/SongMetadataEditingTest.kt — `store::db::song` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `album refresh preserves edited song metadata` | remote album refresh updates duration but never overwrites the locally edited title or artist links | title `Edited title`, duration `200`, artists `[edited-artist]` |
| `album refresh without trusted credits preserves existing artist associations` | album with `artists = null` (untrusted) keeps album/song artist links | artist name, album maps, song ordered artists unchanged |
| `partial album credits do not delete shared artist entities` | updating one album's credits removes only that album's mappings; shared artist stays for other album and other songs | `Featured` still exists; album `[primary]`, other album `[featured]`, song `[featured]` |
| `download refresh preserves edited title and artists` | `overwriteTitle=false`/`overwriteArtists=false` still updates duration | title/artists preserved, duration 200 |
| `replacing song artists does not rename other songs` | `replaceSongArtists` is per song | edited song → new artist; other song untouched; original artist row unrenamed |

### app/src/test/kotlin/com/metrolist/music/db/entities/SongEntityTest.kt — `store::entities::song` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `localOnlyLikeUsesTheSameStateTransition` | like with `syncToYouTube=false` still sets liked+likedDate+inLibrary; unlike clears liked & likedDate but keeps inLibrary | 6 assertions |
| `libraryMembershipSyncDoesNotChangeLikeState` | removing from library clears only `inLibrary`; restoring sets a new `inLibrary` while keeping liked/likedDate | liked & likedDate preserved both ways |

### app/src/test/kotlin/com/metrolist/music/db/entities/LocalIdTest.kt — `store::entities::local_id` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `generatesPrefixedLetterIds` | 8 random ASCII letters after a caller-supplied prefix | regex `^LP[A-Za-z]{8}$` |

Rust: no `rand` if avoidable — a `rand`-free implementation over `getrandom`/`OsRng` is
fine; test must not assume a fixed alphabet beyond ASCII letters.

### app/src/test/kotlin/com/metrolist/music/db/AndroidAutoDatabasePaginationTest.kt — `store::db::pagination` — **ADAPT** (Android Auto surface itself **N/A**)

| Test | Pins | Must assert |
|---|---|---|
| `song relation query stays below Room recursive fetch threshold` | with 1001 songs, every paginated DAO returns 500-sized pages that tile exactly: `offsets 0/500/1000` give 1001 unique ids, no dup, no gap; works for songs, liked songs, artist songs, album songs, playlist songs, artists (1002), albums (1001), playlists (1002) | sizes and `set_size == len` per loader |

Port only the pagination property (`offset/limit` tiles a superset without repeats) plus
the "relations are hydrated per page, not per table" cost guard. The Android Auto browse
tree is not part of the port.

---

# 3. utils — 6 files, 42 tests

### app/src/test/kotlin/com/metrolist/music/utils/PlaylistSyncTest.kt — `metrolist-util::playlist_sync` — **PORT** (highest value file in the repo: 33 tests, all pure)

Remote/local playlist reconciliation. Split into the same free functions the tests call so
each rule is testable in isolation.

| Test | Pins | Must assert |
|---|---|---|
| `incomplete liked songs skip remote removals` | a short fetch (296 of 321 advertised) is not authoritative; missing advertised count is | `has_complete(fetched=296, advertised=Some(321)) == false`; `advertised=None → true` |
| `only items previously seen remotely are removed` | removal requires "was in previous remote set AND is now absent"; no previous snapshot → remove nothing | `{removedOnYouTube}`; `empty` when previous is `None` |
| `local-only songs are preserved` | local items absent remotely are kept and reported by index | `[2]` |
| `duplicate occurrences are compared separately` | local `[a,a]` vs remote `[a]` → second occurrence is extra | `[1]` |
| `remote ordering does not create local-only songs` | order is irrelevant, membership/count is not | `[]` for `[a,b]` vs `[b,a]` |
| `empty fetch with advertised songs is not genuine` | empty remote page claiming `37 songs` is an error, not a deletion | `false` |
| `empty fetch with zero advertised songs is genuine` | `0 songs` is a real empty playlist | `true` |
| `empty fetch without advertised count is not genuine` | unknown count → not genuine | `false` |
| `non-empty fetch is never genuine empty` | items present → false regardless of count text | `false` |
| `backfill fills null setVideoIds positionally` | only rows with `None` are backfilled, positionally, when lengths match | `[localMap(a,0,"set-a")]`; `b` untouched |
| `backfill returns empty when already in sync` | no rows to write | `[]` |
| `backfill returns empty on size mismatch` | no positional guessing | `[]` |
| `backfill skips null remote setVideoIds` | a `None` remote id aborts the batch | `[]` |
| `added songs map newest remote occurrence onto pending row appended at end` | newest remote occurrence → pending row | `x@1 := set-x-new` |
| `added songs map newest remote occurrence onto pending row prepended at start` | same, prepended | `x@0 := set-x-new` |
| `added songs leave rows that already carry a setVideoId untouched` | confirmed rows untouched | `[]` |
| `added songs return empty when song is missing remotely` | no remote occurrence → nothing | `[]` |
| `added songs map duplicate occurrences from newest backwards` | two adds of `x` → newest remote first | `[x@1:=set-x-2, x@0:=set-x-1]` (order!) |
| `added songs never reuse an already known setVideoId` | remote id already used by another song → skip | `[]` |
| `added songs ignore remote setVideoIds already stored locally` | already-stored ids skipped | `x@1 := set-x-new` |
| `added songs skip ambiguous duplicates when only one add succeeded` | 2 pending + 1 remote → ambiguous | `[]` |
| `added songs skip when remote has more new occurrences than confirmed adds` | counts mismatch → skip | `[]` |
| `confirmed remote deletion is dropped while unconfirmed local addition is preserved` | drop only rows whose `setVideoId` is gone; keep pending | `[b@1]` |
| `duplicate pending copy is preserved while synced copy is consumed` | matched synced copy consumed, pending kept | `[x@1]` |
| `unresolved duplicate stays pending and the next sync keeps one pending copy` | pending duplicate not claimed by songId | `[x@1]` |
| `pending duplicate is preserved when a confirmed copy still exists remotely` | | `[x@0]` |
| `pending and confirmed duplicates are both handled when remote has none` | keep exactly one | `[x@0]` |
| `confirmed occurrence claimed by setVideoId, not by songId` | requires actual setVideoId match | `[]` |
| `readded occurrence does not confirm the deleted copy in reverse order` | a new `set-new` does not revive `set-old` | `[]` |
| `extra confirmed copy beyond remote count is dropped` | 2 confirmed, 1 remote → drop the extra | `[]` |
| `confirmed row is dropped when the remote occurrence has no setVideoId` | nothing to confirm against | `[]` |
| `mixed confirmed and pending duplicates keep only the unmatched pending copy` | | `[x@2]` |
| `pending row takes the leftover occurrence when remote has no setVideoId` | | `[]` |

Rust shape: `IdsRemovedRemotely`, `LocalIndexesAbsentFromRemote`, `IsGenuineEmpty`,
`SetVideoIdBackfills`, `SetVideoIdUpdatesForAddedSongs`, `PreservedLocalSongs` — all
`fn(&[...]) -> Vec<...>`, no I/O, no DB. These 33 tests are the spec for the entire
mutation-sync path; port them 1:1 before touching sync code.

### app/src/test/kotlin/com/metrolist/music/utils/ArtistNameAliasesTest.kt — `metrolist-util::artist_aliases` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `artist id alias takes precedence over name alias` | key `artist-id` beats `name:…` | `"ID alias"` |
| `name alias applies when artist id is missing` | `None` id → `name:` lookup | `"Renamed artist"` |
| `original name is retained when no alias exists` | identity when absent | `"Original name"` |
| `chained renames resolve to the latest name` | id alias result is re-resolved through `name:` (one extra hop) | `"Second rename"` |

### app/src/test/kotlin/com/metrolist/music/utils/InnerTubeXPlayerTest.kt — `ytmusic::player_fallback` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `failed stream clients accumulate per video and expire` | failed client names are tracked per videoId and expire after 300 s | `{VISIONOS, WEB_REMIX}` at t=2 s; `[]` for other video; `[]` at t=302 s |
| `failed watch page falls back to anonymous embedded config` | `/player` failure → embedded config fetched **without** login cookies | returns `PlayerConfig("embedded",…)`, `use_login_cookies == false` |

Implement with a `YtConfigParser` trait; the failure window is an injected clock, never
`SystemTime::now()` inside the logic.

### app/src/test/kotlin/com/metrolist/music/utils/UpdaterTest.kt — `N/A`

| Test | Pins | Why N/A |
|---|---|---|
| `parsesKmpReleaseArtifact` | GitHub release JSON → version/assets, rejecting the `Metrolist-with-Google-Cast.apk` asset | Linux distributes AppImage/deb/rpm from the packaging probe, not an APK; no updater in the port. If auto-update is added, write a new test against that feed. |

### app/src/test/kotlin/com/metrolist/music/utils/ComposeToImageTest.kt — `N/A` (mechanism) / **ADAPT** if lyric sharing ships

| Test | Pins | Note |
|---|---|---|
| `createLyricsImage_clipsOutputToRoundedCorners` | exported 2160² image is transparent at all four corners and opaque at centre | Compose `ImageBitmap` → `image`/`vello`; only port if the port offers "share lyrics as image". Keep the same 4-corner + centre assertion. |

### app/src/test/kotlin/com/metrolist/music/utils/NetworkConnectivityObserverTest.kt — `metrolist-util::net` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `networkStatusIsSharedAcrossCollectors` | one shared status stream; two concurrent collectors see the same first value, without hanging | with a 1 s timeout, both `.first()` equal |

Rust: `tokio::sync::watch::Receiver<bool>` behind one `NetworkMonitor`; test with two
subscribers and a timeout so the historic "flow restart hang" cannot regress.

---

# 4. playback core — 9 files, 47 tests

### app/src/test/kotlin/com/metrolist/music/playback/StreamUrlCacheTest.kt — `metrolist-player::stream_cache` — **PORT** (pure + threads)

| Test | Pins | Must assert |
|---|---|---|
| `fresh entry is returned with request headers` | hit returns url + per-entry headers + client name; TTL not yet elapsed | equal to input at +9.999 s of a 10 s TTL |
| `entry is evicted at expiry boundary` | expiry is **inclusive**: `expires_at <= now` evicts; eviction bumps generation; a later read with a rolled-back clock does not resurrect | `None`, `generation == before+1`, still `None` after `now` reset |
| `entry can be invalidated explicitly` | explicit invalidation drops entry | `None` |
| `least recently used entry is evicted at capacity` | LRU by access order (`maxEntries=2`), reading an entry refreshes it | after 3 puts: `second` gone, `first` and `third` present |
| `concurrent access remains consistent` | 1000 ops over 8 threads: put/get/invalidate never deadlocks or corrupts | final put/get succeeds |
| `invalidation rejects a stale in-flight write` | `expected_generation` guard rejects a write that raced an invalidation | `put(...) == false`, `None` |
| `invalidation does not reject another media item write` | generation is per media id | `put(...) == true` |
| `cache preserves bounded range policy` | `requireBoundedRange` + `rangeChunkSizeBytes` + client name + headers survive | `true`, `1024`, `test-client` |

Rust: generation counters + LRU are the subtle part. Use `HashMap<String, Entry>` plus a
`VecDeque`/counter LRU (or an indexed map) and a `Mutex`; keep the exact `<=` boundary and
per-id generation. Fake clock is a `Clock` trait, not `Instant::now()` inline.

### app/src/test/kotlin/com/metrolist/music/playback/NormalizationGainTest.kt — `metrolist-player::replaygain` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `perceptual loudness takes priority` | `perceptual_loudness_db` wins over `loudness_db` | `-506` for `(-20.0, -8.94, -14)` |
| `legacy loudness uses YouTube reference level` | legacy db is offset by the AGGRESSIVE reference `-7 LUFS` | `-506` for `(-1.94, None, -14)` |
| `gain is clamped to processor limits` | clamp to `[-1500, 300]` millibels | `300`, `-1500` |
| `missing loudness disables normalization` | both `None` → no gain | `None` |

Formula to pin: `gain_mb = round(-(measured - target) * 100)` (upstream truncates via
`toInt`) then clamp. Note the truncation: `(-(x)*100).toInt()` truncates toward zero; a
Rust `as i32` matches, `round()` does not. Add one case with a fractional value if you
want that visible.

### app/src/test/kotlin/com/metrolist/music/playback/DownloadContentLengthTest.kt — `metrolist-player::download::length` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `partial response uses total from content range` | 206 + `bytes 0-0/4096` → total | `4096` |
| `full response can use content length` | 200 uses `Content-Length` | `4096` |
| `partial response does not mistake chunk length for total` | 206 without range → unknown, not 1 byte | `None` |
| `unsatisfied range can still report total` | 416 + `bytes */4096` → total | `4096` |
| `failed and malformed responses leave length unknown` | 403, `invalid`, `items …`, `bytes 1-1/4096`, 416 with `0-0`, 200 with `0` | all `None` |

Regexes must be anchored (`^bytes\s+0-0/(\d+)$`, `^bytes\s+\*/(\d+)$`, case-insensitive)
and require `> 0`; `1-1` must fail.

### app/src/test/kotlin/com/metrolist/music/playback/DownloadUtilTest.kt — `metrolist-player::download` — **PORT** (state mapping `ADAPT`)

| Test | Pins | Must assert |
|---|---|---|
| `completed downloads are not prepared again` | only `COMPLETED` skips preparation; failed and absent prepare | `false/true/true` |
| `downloadArtworkUrls keeps distinct song and album covers` | dedupe, drop empty/null, song cover first | `[song, album]`, `[song]`, `[]` |

Rust: replace media3 `Download.STATE_*` with own enum; the rule is "completed ⇒ don't
re-prepare".

### app/src/test/kotlin/com/metrolist/music/playback/QueueStatusFilterTest.kt — `metrolist-player::queue::filter` — **ADAPT** (logic **PORT**)

| Test | Pins | Must assert |
|---|---|---|
| `filtering keeps the selected song as the start item` | removing explicit items keeps the selected item as start; index follows it; position kept | `[b,d]`, index→`d`, position `500` |
| `filtered start song falls through to the next kept song` | if the selected item is filtered out, the next kept item becomes the start and position resets to 0 | start `d`, position `0` |

Also pin the unstated third case (documented gap upstream): selected item filtered and
nothing kept after it → last kept item, position 0.

### app/src/test/kotlin/com/metrolist/music/playback/VoiceSearchMatcherTest.kt — `metrolist-player::voice_match` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `jaro-winkler and tokenizer edge cases` | JW identity/empty rules, JW≈0.93 for `martha`/`marhta`, 0 for `abc`/`xyz`, no boost when Jaro < 0.7 (`MARGHERITA`/`MARMMELLATA` < 0.7); tokenizer strips `'` and `!` | 8 assertions incl. `{don,t,stop,me,now}` |
| `exact match scores 1 and is order-independent` | token-set equality **or** full-title equality → 1.0, regardless of word order | `1.0` for `Bohemian Rhapsody`; `Pressure Under` vs `Under Pressure` → `1.0` |
| `artist tokens are stripped before scoring` | tokens matching an artist (exact if <4 chars, JW ≥ 0.85 if ≥4) removed; equal → 0.95; all tokens were artist → 0.5 | `0.95`, `0.95`, `0.5` |
| `fuzzy matching and generic-query guard` | generic query (1 token <4 chars or query <5 chars) blocks fuzzy but not exact; typo and punctuation clear `STRONG_MATCH_THRESHOLD` (0.60) | `0.0`, `1.0`, typo ≥0.60, punctuation ≥0.60 |
| `edge cases - no match, empty inputs, best not first` | below-threshold match → `None`; empty candidates / blank query → `[]`; best need not be first | `None`, `[]`, `[]`, correct winner |

Constants to carry: `STRONG_MATCH_THRESHOLD = 0.60`, `FUZZY_THRESHOLD = 0.85`,
`is_too_generic = (1 token && len < 4) || query.len() < 5`, fuzzy = harmonic mean of
title/query coverage.

### app/src/test/kotlin/com/metrolist/music/playback/MediaLibrarySessionCallbackTest.kt — `metrolist-player::browse` — **ADAPT** (Android Auto surface **N/A**)

| Test | Pins | Must assert |
|---|---|---|
| `browse tree parents accept subscriptions` | browsable: `root`, `artist`, `artist/*`, `playlist/*`; not `song/*`, not unknown | 6 assertions |
| `children are limited to the requested page` | page/size slicing incl. short last page and out-of-range → empty | `[one,two]`, `[three,four]`, `[five]`, `[]` |
| `database pages are capped before loading relations` | `MAX_ANDROID_AUTO_PAGE_SIZE = 500` caps both limit and offset stride regardless of requested size | `(0,500)`, `(500,500)`, `(1000,500)` |
| `playlist pages account for the leading shuffle action` | one leading synthetic item shifts offset by 1 and shrinks the first page by 1 | `(0,99)`, `(99,100)`, `(499,500)` |
| `playlist containers account for both built in playlists` | two leading items; pages before them yield `(0,0)` | `(0,0)`, `(0,0)`, `(0,1)` |

Port only `AndroidAutoPageRequest { offset, limit }` arithmetic + `is_browsable_media_id`
as pure functions; no Android Auto code in the port. Add the untested case: bare
`playlist` (no id) — upstream's `isBrowsableMediaId` only tests `playlist/id`, so pin the
intended result explicitly.

### app/src/test/kotlin/com/metrolist/music/playback/YouTubeHistoryReporterTest.kt — `metrolist-player::history_report` — **ADAPT** (media3 `AnalyticsListener` → own event loop; timing rules **PORT**)

This is the second-most valuable file. Upstream drives a fake `Player` proxy with a paused
Robolectric looper. In Rust, drive a pure "listen accounting" state machine with an
explicit clock:

```rust
struct ListenTimer { played_ms: i64, threshold_ms: i64, reported: bool }
// inputs: on_playing(), on_paused(), on_buffering(), on_seek(from,to), on_speed(f32),
//         on_track_change(), on_release(), set_enabled(bool), set_threshold(ms)
```

| Test | Pins | Must assert |
|---|---|---|
| `sends at the threshold without waiting for song end and does not resend` | report fires exactly at 30 s (29.999 s → none, +1 ms → one), never again for the same playback | `[]`, `[song]`, still `[song]` after 90 s and release |
| `close cancels timers but deferred final stats still apply the threshold` | closing before the end still reports if ≥ threshold | 10 s → none; 30 s → `[song]` |
| `pauses and buffering do not count as listening` | paused and buffering time never accrues; resuming accrues again | 60 s + 60 s of non-playing → none; then 20 s playing → `[song]` |
| `seeking ahead and changing speed do not manufacture listened time` | a 180 s jump and 2× speed only credit wall-clock playing time | 24.999 s → none, 25 s total → `[song]` |
| `skips stay below threshold and repeats get their own entry` | a skipped item under threshold reports nothing; a repeat of the same video is a new entry | `[second, second]`, 2 distinct report ids, 1 natural end |
| `live failure retries once at the end with the same report` | failed live report → single retry at end reusing the same report | `[song, song]`, ids `[1,1]` |
| `ending during a request does not send a duplicate` | an in-flight report completed during release is not resent | `[song, second]` |
| `failed in flight request retries once after the session ends` | failure after release → exactly one retry | `[song, song]`, ids `[1,1]` |
| `final retry rechecks the threshold against the full listen time` | retry only if total listen ≥ threshold (threshold changed after start) | 90 s → `[song]`; 40 s → `[song, song]` |
| `privacy changes and cancellation block an in flight final retry` | history disabled or task cancelled → no retry | `[song]` in both cases |
| `paused history blocks live reports` | disabled at start → nothing at all | `[]` |
| `re-enabling history and changing threshold reschedules the live report` | re-enable + lower threshold re-arms against already-accumulated time | `[song]` after 5 more seconds |
| `crossfade players track their own listens and release cancels their timers` | per-player accounting; release stops one without affecting the other | both report once |

### app/src/test/kotlin/com/metrolist/music/playback/audio/AudioProcessorBufferReuseTest.kt — `N/A` (mechanism)

| Test | Pins | Note |
|---|---|---|
| `volume normalization reuses its direct output buffer` | media3 `AudioProcessor` returns the same direct `ByteBuffer` instance | media3-only. Do not port the identity assertion; if the Rust DSP chain is `cpal`/`rodio`, write a fresh "no per-buffer allocation" benchmark/assertion on the real pipeline instead. |
| `silence detection reuses its direct output buffer` | same | same |
| `custom equalizer reuses its direct output buffer` | same | same |

---

# 5. lyrics — 2 files, 11 tests

### app/src/test/kotlin/com/metrolist/music/lyrics/LyricsBackgroundTimingTest.kt — `metrolist-lyrics::lrc::timing` — **PORT**

Robolectric is used but there is no Android dependency; port verbatim.

| Test | Pins | Must assert |
|---|---|---|
| `last word of a line with background vocals does not end before it starts` | deriving last-word end from the *next line* must skip background lines (`[bg: …]` starts 10.20, before the last word at 10.50) | `end > start`, last word text `line` |
| `last word runs to the next main line rather than to the background line` | end = next **main** line start | `12.0` |
| `background line itself is still parsed with its own timings` | bg entry kept separate with its own text | 1 bg entry, text `ooh` |
| `the bracketed bg spelling is skipped too` | `[MM:SS.mm]{bg}…` form behaves like `[bg: …]` | `end > start`, `12.0` |
| `a background line whose timings sit on a continuation line is skipped whole` | a standalone `<word:start:end>` line belongs to the preceding bg entry and must not terminate the main line | main end `12.0`; bg first word start `10.2` |
| `a line followed by a normal line is unaffected` | no regression in the plain path | `12.0` |

### app/src/test/kotlin/com/metrolist/music/lyrics/MacedonianRomanizationTest.kt — `metrolist-lyrics::romanize` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `a line carrying the grave accented e is still recognised as Macedonian` | `ѐ` is a Macedonian letter for detection | `is_macedonian("Сѐ што имам и куќа и нива")` |
| `a line carrying the grave accented i is still recognised as Macedonian` | `ѝ` likewise | `is_macedonian("Ги виде ѝ рече")` |
| `lyrics containing a grave accent are romanised rather than dropped` | accents must not make every language check fail | output contains no `Ѐ..ӿ` chars |
| `the grave accented vowels romanise to their plain latin letters` | `ѐ → e`, `ѝ → i` | `"Se shto imam i kukja i niva"` |
| `every line of a song is read with the same alphabet` | detection runs once for the whole lyrics; every line read as Macedonian (`ц → c`, `ж → zh`), never re-detected per line as Russian/Serbian | 4 lines all romanised; contains `cresha`, `zhiva`, `tazhno` |

This is the "one alphabet per song" rule: `romanize(lyrics, line, …, detect_per_line=false)`
must take the language from the whole text once.

---

# 6. betterlyrics — 1 file, 14 tests

### app/src/test/kotlin/com/metrolist/music/betterlyrics/TTMLParserTest.kt — `metrolist-lyrics::ttml` — **PORT**

Apple/YouTube Music TTML → LRC with word timing. Assert on the **LRC string**, matching
upstream; it is the contract other code consumes.

| Test | Pins | Must assert |
|---|---|---|
| `testV1000AgentMapping` | agents are remapped in document order: `v1` stays `v1`, any other agent becomes `v2` (so `v1000 → v2`) | `{agent:v2}(Group vocal)`, `{agent:v1}Main vocal` |
| `testTimeFormats` | `1.5s`, `2000ms`, `00:03.50` all parse | `[00:01.50]`, `[00:02.00]`, `[00:03.50]` |
| `testRoleXBgMapping` | `ttm:role="x-bg"` → `{bg}` prefix | `{bg}Background role` |
| `testWordLevelSync` | word tags are absolute seconds with 1 decimal: `<text:start:end>` | `<Hello:10.0:10.5\|world:10.6:11.0>` |
| `testSingleVocalistWithBackground` | `v1` label kept when a bg exists to disambiguate | `{agent:v1}` and `{bg}bg` |
| `testSingleVocalistNotV1` | explicitly named non-v1 agent is preserved | `{agent:v2}` |
| `testLyricOffset` | `<audio lyricOffset="10.5">` shifts line **and** word times | `[00:11.50]`, `Hello:11.5:12.5` |
| `testTranslationAndRomanSpansAreSkipped` | `x-roman` / `x-translation` spans dropped, no extra line | 1 line, text `Main lyric` |
| `testSplitSyllableSpansAreMerged` | split syllables merge into one word, span start→first, end→last | `hello`, 1 word, `1.0..2.0` |
| `testMultipleBackgroundSpansInOneP` | multiple bg spans in one `<p>` merge into one `{bg}` line with combined word data | `{bg}Bg1 Bg2`, `<Bg1:1.2:1.5\|Bg2:1.6:1.9>` |
| `testV2000AgentMapping` | `v2000` → `v2` while `v1` stays | `{agent:v1}Main vocal`, `{agent:v2}Outro vocal` |
| `testSequentialBackgroundDeDuplication` | consecutive `x-bg` paragraphs emit `{bg}` on the **first only**; nested bg still gets `{bg}`; following non-bg line does not | 6 assertions on lines 0..5, incl. `{agent:v1}Main Line`, `{agent:v2}Line 5` |
| `testTtpTimingOnParagraph` | `ttp:begin`/`ttp:end` on `<p>` (namespaced) are honoured | line exists, `[00:02.50]` |
| `testParagraphInheritsBeginFromFirstSpan` | `<p>` without begin inherits the first span's begin | first line `[00:05.00]` |

Note the ordering of the agent map: build it from the document (first-seen), reserving
`v1` for the `v1` id — do not sort ids numerically (`v1000` must become `v2`, which
numeric sorting would get wrong).

---

# 7. listen together — 4 files, 9 tests

All plain JUnit, no Android: port directly. Crate `metrolist-sync`.

### app/src/test/kotlin/com/metrolist/music/listentogether/MessageCodecTest.kt — `sync::codec` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `playback timing fields survive a protobuf round trip` | encode/decode preserves action, track id, position, server time, revision, `capturedAtServerTime`; envelope type preserved | all 7 fields equal; note the payload is decoded as `SYNC_PLAYBACK` while encoded as `PLAYBACK_ACTION` — keep the type mapping explicit and tested |
| `timestamped ping is encoded and pong is decoded` | ping carries client time + sequence; pong carries receive/send times and satisfies `serverSend >= serverReceive` | all 4 pong fields; equality with the manually built message |

Rust: prost with the same `.proto` (field numbers must match the server), plus a
compression on/off test — upstream constructs `MessageCodec(compressionEnabled = true)`
but only exercises the compressed path; add the uncompressed path.

### app/src/test/kotlin/com/metrolist/music/listentogether/ServerClockTest.kt — `sync::clock` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `maps server wall time onto local monotonic time` | before any sample `now()` is `None`; first accepted sample maps server→local; `position_at` advances while playing and not while paused | `now()==10_065` with elapsed 1_120; `position_at(500, 10_000, true)==565`; `+100 ms → 1_065`/`665`; paused → `500` |
| `rejects invalid and stale samples` | reject `clientTime<=0`, `clientTime > receivedAt`, `serverSend < serverReceive`, sample older than 60 s | all `false`, `now()` stays `None` |
| `reset removes the server clock mapping` | reset clears offset; `position_at` then returns the raw position | `None`, `500` even when playing |

Formula: `net_rtt = max(0, (received_at - client_time) - (server_send - server_receive))`,
`offset = server_send + net_rtt/2 - received_at`, EWMA weight 0.25 (good sample, within
50 ms of best RTT) else 0.05; `record_pong` returns `true` only for the first sample.
Clock is injected (`Fn() -> i64` monotonic ms).

### app/src/test/kotlin/com/metrolist/music/listentogether/PlaybackSyncTest.kt — `sync::drift` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `heartbeatDriftDoesNotInterruptActivePlayback` | hard seek only when playback is ready **and** drift > 2000 ms (2000 exactly does not seek) | `false` for 0/50/750/1999/2000; `false` for 10 000 when not ready; `true` at 2001 |

### app/src/test/kotlin/com/metrolist/music/listentogether/QueueSyncTest.kt — `sync::queue` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `upcomingQueueNeverReplacesCurrentTrack` | canonical queue is current first, then upcoming in order | `[current, suggested, later]` |
| `canonicalQueueRemovesCurrentAndDuplicateUpcomingEntries` | current removed from upcoming, duplicates dropped (first occurrence kept) | `[current, next]` |
| `outgoingQueueExcludesPlaybackHistoryAndCurrentItem` | outgoing queue = items strictly after current index; unknown index → empty | `[suggested, later]`; `[]` for index `-1` |

---

# 8. discord — 4 files, 24 tests

Crate `metrolist-discord`.

### app/src/test/kotlin/com/metrolist/music/discord/DiscordReconnectStrategyTest.kt — `discord::reconnect` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `resume_when4000_andHadSession` | 4000 + session + seq>0 → `Resume{session_id, seq}` | session `session-abc`, seq `42` |
| `reIdentify_when4000_andNoSession` | 4000 without session → `ReIdentify` | |
| `reIdentify_when4000_andSeqZero_evenWithSession` | `seq == 0` means no dispatch received; resume would fail | `ReIdentify` |
| `reIdentify_forTransientCloseCodes` | 4001/4003/4005/4007/4009/unknown → `ReIdentify` | all 6 |
| `refreshAndReIdentify_for4004` | 4004 → `RefreshAndReIdentify` | |
| `surfaceFatal_for4014` | 4014 → `SurfaceFatal` | |

Pure `fn decide(close_code, had_session, seq, session_id) -> ReconnectAction`; no I/O.

### app/src/test/kotlin/com/metrolist/music/discord/DiscordPresenceTest.kt — `discord::presence` — **PORT**

Asserts on serialized JSON (`org.json` → `serde_json`); keep field names exact.

| Test | Pins | Must assert |
|---|---|---|
| `buildPresenceUpdate_serializesAsOp3` | op 3, `d.status` lowercase, `afk=false`, activities array with name and numeric type | `op==3`, `"online"`, `afk==false`, `type==Listening` code |
| `buildPresenceUpdate_emitsTimestampsWhenProvided` | `timestamps.start/end` only when given | `1000`, `2000` |
| `buildPresenceUpdate_emitsAssetsWhenImagesPresent` | `assets.large_image/large_text/small_image/small_text` | 4 values |
| `buildPresenceUpdate_omitsAssetsWhenImagesNull` | no `assets` key at all (not `null`) | `!has("assets")` |
| `buildPresenceUpdate_emitsButtonsAsStringArrayAndUrlsInMetadata` | buttons are **labels** in `activities[].buttons`, URLs in `metadata.button_urls`, same order | 2 labels, 2 urls |
| `buildPresenceUpdate_supportsEmptyActivities` | empty list allowed | `status=="invisible"`, `len==0` |
| `buildPresenceUpdate_emitsUrlWhenProvided` | streaming activity carries `url` | `https://example.com/stream` |

### app/src/test/kotlin/com/metrolist/music/discord/DiscordAuthTest.kt — `discord::oauth` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `refresh_succeeds_withValidResponse` | POST `/api/v10/oauth2/token`, parses access/refresh/expires_in/scope | `604800`, `"rpc"` |
| `refresh_throwsInvalidGrant_onBadRequest` | 400 + `error: invalid_grant` → typed `InvalidGrant` with message, not a generic error | variant + message present |
| `refresh_throwsNetworkFailure_onServerError` | 500 → `NetworkFailure` whose message contains the status | contains `"500"` |
| `generatePkcePair_producesValidChallenge` | URL-safe base64 without `=`, challenge length 43 (SHA-256 of verifier, base64url-no-pad) | non-empty, no `=`, `len==43` |

Rust: `reqwest` + mock server (`wiremock`/`httpmock`) or a `Transport` trait; error enum
must be matched exhaustively in the test.

### app/src/test/kotlin/com/metrolist/music/discord/DiscordTokenStoreTest.kt — `discord::token_store` — **ADAPT**

| Test | Pins | Must assert |
|---|---|---|
| `storeAndRetrieveToken` | round trip | equal |
| `storeFull_retainsAccessTokenAndRefreshToken` | stores access+refresh+expiry in the future | `expires_at > now` |
| `storeAccessToken_updatesAccessWithoutChangingRefresh` | access-only update keeps refresh | `access2` / `refresh1` |
| `clear_removesAllTokens` | clears everything, `expires_at == 0` | `None`, `None`, `0` |
| `retrieveSuspend_returnsSameAsRetrieve` | sync and async reads agree | equal |
| `generatesAndCachesUuids` | device vendor id and client uuid are stable across calls and different from each other | both non-null, equal per kind, distinct across kinds |
| `aesEncryptionRoundTrip` | AES-256 round trip, ciphertext ≠ plaintext | equal after decrypt |

Rust: Android Keystore + `SharedPreferences` → OS credential store (secret-service) or a
key file with restrictive perms + AES-GCM. Inject the key source (`set_test_key`) exactly
as upstream does so tests need no OS keyring. Keep per-field storage semantics from the
tests; storage backend is the only ADAPT part.

---

# 9. api — 1 file, 6 tests

### app/src/test/kotlin/com/metrolist/music/api/OpenRouterServiceTest.kt — `metrolist-ai::translation` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `translation parsing handles fenced and short responses` | ```` ```json … ``` ```` fence stripped; short array padded with `""` to the line count | `["uno", ""]` for 2 lines |
| `translation parsing handles the structured output object shape` | `{"lines": [...]}` accepted | `["uno","dos"]` |
| `request uses structured outputs with a lines schema and strict parameter routing` | `response_format = json_schema` with `{type: object, properties: {lines: {type: array, items: {type: string}}}}`; `provider.require_parameters == true` | schema shape + flag |
| `openrouter-only provider routing is omitted for direct providers` | non-openrouter base URL → no `provider` key, `response_format` still present | both |
| `openai requests omit parameters gpt-5 models reject` | `gpt-5*` → no `max_tokens`, no `temperature` | both absent |
| `streaming romanization uses the romanization prompt` | `stream=true`; `mode=Romanized` selects the prompt starting with `Romanize` | flag + prefix |

Provider detection is by base URL; keep it a pure function of the request parameters.

---

# 10. viewmodels — 2 files, 9 tests

### app/src/test/kotlin/com/metrolist/music/viewmodels/CachePlaylistPartitionTest.kt — `metrolist-util::cache_partition` — **PORT**

`partitionCachedSongs` takes a `resolveContentLength` and an `exists(id, len) -> bool`
probe (upstream uses the download cache). Inject both.

| Test | Pins | Must assert |
|---|---|---|
| `song whose bytes are still present stays cached` | probe true → still cached | `[a]` / `[]` |
| `song whose bytes are gone is reported as stale` | probe false → stale | `[]` / `[a]` |
| `downloaded song is excluded without clearing its download date` | downloaded songs are neither probed nor listed | both empty, `lookups == 0` |
| `cache metadata length is used when format is missing` | format absent → `resolveContentLength` supplies the length | probe got `(a, 2000)` |
| `song with unknown content length is stale and is never looked up` | no length → stale, no probe | `lookups == 0` |
| `each song is checked with its own id and content length` | no cross-wiring | `[(a,10),(b,20)]` |
| `mixed input is split preserving input order in both groups` | input order kept in both output groups | `[keep1,keep2]`, `[drop1,drop2]` |
| `empty input produces empty groups` | empty in, empty out | both empty |

### app/src/test/kotlin/com/metrolist/music/viewmodels/HomeSpeedDialTest.kt — `metrolist-util::speed_dial` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `home recommendations fill sparse speed dial without replacing pinned items` | pinned items first, untouched; fill from keep-listening/quick-picks/home, skipping ids already present | `[pinned, home]` not `[pinned, duplicate]` |

---

# 11. ui helpers — 2 files, 11 tests

Both are pure string logic and port directly; the `ui` package name is misleading.

### app/src/test/kotlin/com/metrolist/music/ui/utils/YouTubeUtilsTest.kt — `metrolist-util::images::resize` — **PORT**

YouTube thumbnail URL rewriting. One function, nine cases; every one is a regression from
a real blurry/broken-art bug.

| Test | Pins | Must assert |
|---|---|---|
| `lh3 googleusercontent is rewritten to the requested size` | `=w120-h120-l90-rj` → `=w544-h544-p-l90-rj` | exact string |
| `yt3 googleusercontent is rewritten too (YouTube migrated host - fixes the blurry player)` | `yt3.googleusercontent.com` must be matched as well, else a 60 px image gets upscaled | exact string |
| `i_ytimg url keeps the thumbnail variant provided by YouTube` | `i.ytimg.com` untouched (`maxresdefault` may 404) | identical string |
| `single dimension preserves googleusercontent aspect ratio` | `w=300` alone → `w300-h150` (scale height by the source ratio 120:60) | `=w300-h150-p-l90-rj` |
| `null dimensions return the url unchanged` | no size request → no rewrite | identical |
| `ggpht avatar uses w-h-p format when both dimensions requested` | `=s88` → `=w544-h544-p-l90-rj` | exact string |
| `googleusercontent resize preserves query parameters` | query string survives, including `?token=a=b` (embedded `=`) | suffix preserved |
| `ggpht resize preserves query parameters` | same for ggpht | suffix preserved |
| `ggpht query without resize suffix remains unchanged` | no `=s…`/`=w…` suffix → leave alone rather than guessing | identical |

Rust: `fn resize(url: &str, w: Option<u32>, h: Option<u32>) -> Cow<str>` (or `String`).
Split the path from the query at the first `?`, work only on the path's `=`-suffix, then
re-join — that is what makes `?token=a=b` work. Host check is a suffix/equality check
against the three allowed hosts, not `contains`.

### app/src/test/kotlin/com/metrolist/music/ui/player/PlaybackErrorTest.kt — `metrolist-player::error` — **PORT**

| Test | Pins | Must assert |
|---|---|---|
| `cause chain keeps each nested playback failure` | the whole chain in outermost→root order, not just the top message | `[IllegalStateException, IOException, EOFException]` |
| `offline warning replaces misleading stream errors` | when offline, show only the connectivity message and suppress the raw stream errors (they are misdiagnoses) | exactly `["Connect to the internet"]` |

Rust has no `cause` chain; model it as `struct PlaybackError { message, source: Option<Box<PlaybackError>> }`
with `fn chain(&self) -> Vec<&PlaybackError>` and keep the ordering. The offline rule is a
pure `fn messages(is_online, offline_message, guidance, raw) -> Vec<String>` — port it as
a function, not inside UI code, so it stays testable.

# 12. extensions — 2 files, 3 tests

### app/src/test/kotlin/com/metrolist/music/extensions/ContentResolverExtTest.kt — `ADAPT`

| Test | Pins | Rust replacement |
|---|---|---|
| `reads display name and size without opening file` | name+size from metadata only; no file opened | `metrolist-util::local_file::probe(path) -> (name, len)` via `std::fs::metadata`; no read of contents |
| `uses descriptor length when provider omits size` | size falls back to descriptor length when the provider reports none | same function: `metadata.len()`, and name fallback = last path segment (`"unknown"` when absent) |

Keep the two fallbacks (metadata size, then stream length) as separate branches, and the
`size <= 0` → probe rule. The `ContentResolver`/`AssetFileDescriptor` mechanism is Android
only.

### app/src/test/kotlin/com/metrolist/music/extensions/MediaItemExtTest.kt — `N/A`

| Test | Pins | Why N/A |
|---|---|---|
| `updated metadata replaces title artists artwork and explicit flag` | media3 `MediaItem` extras round trip, incl. `EXTRAS_KEY_IS_EXPLICIT` set/clear and `artworkUri` clearing | `MediaItem`/`MediaConstants` do not exist in the port. If the port keeps a `MediaMetadata`-style struct, write a fresh round-trip test for *that* struct (`title`, joined artists, artwork url, explicit) — do not emulate extras. |

---

# 13. Must-not-miss invariants (the rules these tests encode)

Artist / metadata parsing (`ytmusic`):
1. **Only linked runs can become artists.** A `Run` without a navigation endpoint is never an artist, no matter what it looks like; a link does not bless its unlinked neighbours; separators (` • `, `", "`, `" & "`, blank) are never artists.
2. **Only artist-ish links count.** `MUSIC_PAGE_TYPE_ARTIST` (any id, no `UC` requirement) and `MUSIC_PAGE_TYPE_USER_CHANNEL` are trusted; `MUSIC_PAGE_TYPE_ALBUM`, `MUSIC_PAGE_TYPE_PLAYLIST` are not. Podcast bylines additionally need `MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE` or an artist link — a plain string is not a credit.
3. **Album/song metadata runs are never artists.** Durations (`3:42` → 222 s, `3:12` → 192 s), play counts and type labels in any locale (`canción`, `14 m reproducciones`, `3,3 M reproducciones`, `歌曲`, `1億 回再生`, `6m\u00a0plays` with NBSP) are filtered by *shape*, not by an English word list.
4. **Menu-only artist ids carry an empty name** (`Artist("", "UCartist")`); the menu's *label* is never used as an artist name (podcast menu case).
5. **Context decides the performer.** Uploaded compilation rows keep their tagged performer; ATV tracks override the label only inside uploaded (`OLAK5uy_`) playlists; otherwise the album artist wins. `isUploaded` library rows may keep unlinked tagged names after a `", "`; generic rows may not.
6. **Optional renderer fields must default.** `buttons`, `subtitle`, `menu`, `overlay`, `continuations` may be absent → empty/`None` (serde `default`, `ignore_unknown`, no `deny_unknown_fields`).

Playback / queue:
7. **Stream URL cache:** expiry is inclusive (`expires_at <= now` evicts) and bumps a **per-media-id generation**; a write carrying a stale `expected_generation` is silently rejected, a write for another id is not; LRU eviction on access order (default 500); headers, client name and the bounded-range policy travel with the entry.
8. **Explicit filter keeps the start item:** filtering explicit tracks preserves the selected item and its position; if the selected item is itself removed, the next kept item becomes the start **and position resets to 0**.
9. **ReplayGain:** perceptual loudness beats legacy `loudness_db`; legacy is offset by the AGGRESSIVE reference (-7 LUFS); `gain_mb = trunc(-(measured - target_lufs) * 100)` clamped to `[-1500, 300]`; no loudness ⇒ normalization off (`None`).
10. **Listen accounting:** 30 s default threshold, credited only while playing and ready (paused/buffering excluded); seeking and >1× speed never manufacture time; one report per playback session at threshold, no resend on release; a failed report gets exactly one retry at the end, re-checked against the total listen time, and is blocked if history was disabled or the task cancelled meanwhile.
11. **Playback registration:** every listen fetches `/player` fresh (no stream-cache reuse), sends a per-listen CPN, re-checks privacy before *and* after the fetch, aborts if the account changed mid-flight, and only accepts `https://s.youtube.com:443/api/stats/playback` (no other host/port/path/scheme/userinfo).
12. **Search auth is per-request:** explicit `setLogin` adds `Authorization`, `Cookie`, `X-Goog-AuthUser` and `context.user.onBehalfOfUser`; automatic searches (suggestions, continuations, artist lookups) send none even when cookies exist. Nothing leaks to the next call.

Store / sync:
13. **Playlist append position = max+1** (sparse positions allowed), bulk add joins the caller's transaction (rollback removes it), and songs must already exist.
14. **History dedup:** one row per (song, period bucket) keeping the newest event, ordered by event id desc; bucket precedence tomorrow→today→yesterday→this Monday→last Monday→`YYYY-MM`; repeated plays must not multiply hydrated relation graphs.
15. **Cached artist page stays on disk** for bulk reads (explicit column list) but is returned by the artist detail query and preserved by thumbnail updates.
16. **Duplicate lookup batches at ≤500 ids** (SQLite's 999-variable limit), chunking on distinct ids; empty input ⇒ empty result.
17. **Server-side playlist sync:** only remove locally what a *previous* remote snapshot contained; a short fetch (fetched < advertised) or an empty fetch with a non-zero/unknown advertised count is not authoritative; local-only and pending duplicate rows survive; duplicate occurrences are compared by **count**, never by set membership; `setVideoId` backfill is positional and only when lengths match and all ids are present; added-song mapping goes newest-remote-backwards, never reuses a known `setVideoId`, and skips on any ambiguity; confirmed rows are matched by `setVideoId`, not by song id.
18. **Liked/library state transitions:** like sets `liked`, `likedDate`, `inLibrary`; unlike clears `liked`+`likedDate` but leaves `inLibrary`; library removal/restore never touches like state; album/download refresh only overwrites duration unless `overwrite*` is requested, and partial credits delete only that album's mappings.
19. **Migration defaults** are part of the schema: NULL `libraryAddToken`/`libraryRemoveToken`, `romanizeLyrics='true'`, `isDownloaded='0'`, `isUploaded='false'` (song + album), `playlist.thumbnailUrl` added; re-running after a partial migration must not fail.

Listen together:
20. **Clock/sync:** accept a sample only when `0 < client_time <= received_at`, `received_at - client_time <= 60 s`, `server_send >= server_receive`; offset = `server_send + net_rtt/2 - received_at` with `net_rtt = max(0, rtt - server_processing)`; EWMA 0.25 for samples within 50 ms of the best RTT else 0.05; `record_pong` returns true only for the first sample; `position_at` advances only while playing; hard seek only when ready **and** drift > 2000 ms (2000 exactly is tolerated); canonical queue = current + deduped upcoming with the current removed.
21. **Codec field numbers** must match the server protobuf; a playback action encoded as `PLAYBACK_ACTION` is decoded as `SYNC_PLAYBACK` — keep that mapping explicit.

Lyrics:
22. **TTML:** agent remap by document order with `v1` reserved (so `v1000`/`v2000` → `v2`); `x-bg` → `{bg}` with `{bg}` emitted only on the first of consecutive background paragraphs; `x-roman`/`x-translation` skipped; word tags `<text:start:end>` in absolute seconds with one decimal; split syllable spans merge into one word (first start → last end); multiple bg spans in a `<p>` merge into one line; `<audio lyricOffset>` shifts lines and words; `ttp:begin` on `<p>` honoured; `<p>` without begin inherits the first span's begin.
23. **LRC background timing:** when the last word's end is derived from the next line, skip background entries (both `[bg: …]` and `{bg}` spellings) *and* a background's standalone `<word:…>` continuation line; end must always be > start and must run to the next **main** line.
24. **Romanization:** `ѐ`/`ѝ` count as Macedonian; they romanize to `e`/`i`; language detection runs once over the whole lyrics and that one alphabet is applied to every line (never re-detect per line, which would read `ц` as `ts` or `ж` as `ž`).

Misc:
25. **Thumbnail URLs:** only `lh3`/`yt3.googleusercontent.com` and `yt3.ggpht.com` are rewritten (`=w120-h120-l90-rj` and `=s88` → `=w{W}-h{H}-p-l90-rj`); `i.ytimg.com` URLs are left untouched (maxres may 404); single dimension preserves aspect (scale the other); both `None` ⇒ unchanged; query strings preserved; ggpht without a resize suffix unchanged.
26. **Discord:** presence op 3, lowercase status, `afk:false`; assets omitted (key absent) when no images; buttons are labels with URLs under `metadata.button_urls`; reconnect: 4000 + session + seq>0 ⇒ resume, seq 0 ⇒ re-identify, 4004 ⇒ refresh+re-identify, 4014 ⇒ fatal, everything else ⇒ re-identify.
27. **Voice search:** thresholds 0.60 (best match) / 0.85 (fuzzy, artist-strip); generic query (single token < 4 chars or query < 5 chars) blocks fuzzy but not exact; artist-stripped-empty ⇒ 0.5; exact-token match ⇒ 0.95; scoring order-independent.
28. **Android Auto paging arithmetic:** page size caps at 500 for both limit and stride, synthetic leading items (shuffle, built-in playlists) shift offset and shrink the first limit.
29. **Home speed dial** fills only missing slots, keeps pinned items, and never duplicates an id.

---

# 14. Making the Android-mocked logic testable in Rust

The upstream tests that reach for Android/Robolectric do so for four reasons. Replace each
with an injected seam; none of them requires a mock framework.

| Upstream mechanism | Tests | Rust seam |
|---|---|---|
| Ktor `MockEngine` (request interception) | `PlaybackRegistrationTest`, `SearchAuthenticationTest`, `PodcastEnrichmentTest`, `DiscordAuthTest` | `trait Transport { fn send(&self, req: Request) -> Result<Response> }` (or `tower::Service`). Production: `reqwest`. Test: a `Vec<(Request, Response)>` matcher that records `request_history`, mirroring `engine.requestHistory`. Assert on method, path, query (`cpn`), headers and JSON body — the upstream assertions are all header/body-level. |
| Clock (`System.currentTimeMillis`, `SystemClock.elapsedRealtime`, `ShadowSystemClock`) | `StreamUrlCacheTest`, `InnerTubeXPlayerTest`, `ServerClockTest`, `YouTubeHistoryReporterTest`, `DiscordTokenStoreTest` (expiry) | `Clock` trait with `now_ms()`; pass `&dyn Clock` or `impl Fn() -> i64`. Never call `Instant::now()` inside the logic under test. For the history reporter, drive a paused virtual clock the way Robolectric's paused looper does. |
| Room + Robolectric application context | all `db/*` tests | real SQLite: `rusqlite::Connection::open_in_memory()` (or `sqlx` pool over `:memory:`), migrations applied by the same `migrate()` the app uses. Tests call the repository functions, not raw SQL, except the migration test which asserts on `PRAGMA table_info` — same query works unchanged. |
| Android storage (`SharedPreferences`, `Keystore`), `ContentResolver`, media3 listeners | `DiscordTokenStoreTest`, `ContentResolverExtTest`, `MediaItemExtTest`, `QueueStatusFilterTest`, `MediaLibrarySessionCallbackTest`, `YouTubeHistoryReporterTest`, `AudioProcessorBufferReuseTest` | narrow traits, one per concern: `SecretStore` (+`set_test_key`), `FileProbe { fn exists(id, len) -> bool }`, `MediaItemSource` for the asset/URL probe, `PlayerEvents` enum feeding the listen-accounting state machine, and `AudioSink` for DSP. The tests then construct the logic with fakes; no Android surface appears in the port's core crates. |
| Reflection into singletons (`YouTube`, `InnerTube`) | `PodcastEnrichmentTest` | none needed: make enrichment a free `async fn(api: &dyn InnerTubeApi, episodes: Vec<Episode>) -> Vec<Episode>` and pass a fake API. This is the only upstream test that mutates private fields; do not reproduce the technique. |

Test placement: one `#[cfg(test)] mod tests` per module for the small files; for TTML,
`SearchPage`, `PlaylistSync`, `YouTubeHistoryReporter`, `StreamUrlCache` and `PlaylistSync`
use `tests/` integration files with `fixtures/` JSON copied from the Kotlin literals. Keep
the upstream test *names* (snake_cased) as Rust `fn` names: they are already the
behavioural index used above and make parity review mechanical.

---

# 15. Gaps, oddities, and things that look off

- No `@Ignore`/`@Disabled`; nothing is skipped.
- **`PlaylistSyncTest.duplicate occurrences are compared separately`** encodes count-based comparison (`["a","a"]` vs `["a"]` → index 1 extra). Any Rust implementation that de-duplicates ids before comparing will pass "local-only" tests and fail this one.
- **`TTMLParserTest.testSequentialBackgroundDeDuplication`** depends on the implementation quirk that consecutive `x-bg` paragraphs emit `{bg}` only on the first. It reads like a bug being pinned, but the consumer regex definitely relies on it; port the behaviour and the test together, and add a comment.
- **`MusicResponsiveHeaderRendererTest`** is the only model-level JSON test; there is no equivalent coverage for other renderers. When porting, add `serde` round-trip fixtures for the models you actually parse rather than treating this as coverage.
- **`MediaLibrarySessionCallbackTest`** never tests a bare `PLAYLIST`/`ROOT` id combination (`isBrowsableMediaId(PLAYLIST)` is untested). Pin the intended behaviour in the Rust test and note the decision.
- **`QueueStatusFilterTest`** misses "selected item removed and nothing kept after it". Add that case.
- **`MessageCodecTest`** only exercises compression on; the codec is constructed with `compressionEnabled = true`, so the uncompressed branch is untested. Add it.
- **`DiscordTokenStoreTest.retrieveSuspend_returnsSameAsRetrieve`** is a tautology upstream (both sides read the same store); in Rust it is worth keeping only as a "no lock/suspend deadlock" test, not as a parity assertion.
- **`AudioProcessorBufferReuseTest`**, **`MediaItemExtTest`**, **`ComposeToImageTest`**, **`UpdaterTest`**, **`NetworkConnectivityObserverTest`** test Android mechanisms. Their *intent* (no per-buffer allocation; metadata round trip; rounded-corner export; update-feed parsing; a single shared connectivity stream) is worth a native test, but the assertions must be rewritten for the platform.
- Robolectric on `LyricsBackgroundTimingTest` and `MacedonianRomanizationTest` is incidental — those are pure and port 1:1.
