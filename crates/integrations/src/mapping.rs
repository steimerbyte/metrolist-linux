//! Pure mappings between `ml-core` player state and desktop protocol values.
//!
//! Everything here is total and panic-free: no I/O, no locks, no floats and no
//! dependence on D-Bus types. This is the verifiable core of the crate — the
//! proof lead can restate each function in Verus without pulling in
//! `mpris-server`, `ksni` or `notify-rust`.
//!
//! The protocol enums are mirrored locally ([`TransportStatus`],
//! [`TransportLoop`]) so this module stays dependency-free; `mpris.rs` does the
//! one-line total conversion to the `mpris-server` types.

use ml_core::{PlaybackStatus, PlayerSnapshot, RepeatMode, Song};

/// MPRIS `Playback_Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportStatus {
    Playing,
    Paused,
    Stopped,
}

/// MPRIS `Loop_Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportLoop {
    None,
    Track,
    Playlist,
}

/// Prefix of every `mpris:trackid` we publish.
pub const TRACK_PATH_PREFIX: &str = "/com/metrolist/track/";

/// Core transport state -> MPRIS status.
///
/// `Loading`/`Idle`/`Error` report `Stopped`: no audio is flowing yet, and
/// MPRIS has no separate value for them.
pub fn transport_status(status: PlaybackStatus) -> TransportStatus {
    match status {
        PlaybackStatus::Playing | PlaybackStatus::Buffering => TransportStatus::Playing,
        PlaybackStatus::Paused => TransportStatus::Paused,
        PlaybackStatus::Idle
        | PlaybackStatus::Loading
        | PlaybackStatus::Stopped
        | PlaybackStatus::Ended
        | PlaybackStatus::Error => TransportStatus::Stopped,
    }
}

/// Core repeat mode -> MPRIS loop status (`All` is `Playlist`, `One` is `Track`).
pub fn transport_loop(repeat: RepeatMode) -> TransportLoop {
    match repeat {
        RepeatMode::Off => TransportLoop::None,
        RepeatMode::All => TransportLoop::Playlist,
        RepeatMode::One => TransportLoop::Track,
    }
}

/// MPRIS loop status -> core repeat mode; inverse of [`transport_loop`].
pub fn repeat_from_loop(loop_status: TransportLoop) -> RepeatMode {
    match loop_status {
        TransportLoop::None => RepeatMode::Off,
        TransportLoop::Playlist => RepeatMode::All,
        TransportLoop::Track => RepeatMode::One,
    }
}

/// Milliseconds -> microseconds for `mpris:length` / `Position`, saturating.
///
/// Exact for `ms <= i64::MAX / 1000`; clamped to `i64::MAX` beyond that so a
/// bogus duration can never wrap negative.
pub fn ms_to_us(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX).saturating_mul(1000)
}

/// Microseconds -> milliseconds, truncating toward zero; keeps the sign so
/// `Seek` offsets stay directional.
pub fn us_to_ms(us: i64) -> i64 {
    us / 1000
}

/// Absolute `SetPosition` microseconds -> non-negative milliseconds.
pub fn position_ms_from_us(us: i64) -> u64 {
    (us.max(0) / 1000) as u64
}

/// `mpris:trackid` for a song: [`TRACK_PATH_PREFIX`] plus lowercase hex of the id.
///
/// Hex, not the raw YouTube id: ids contain `-`, which is not valid in a D-Bus
/// object path element. Hex is injective, so distinct songs never share a path.
/// An empty id maps to `.../0`, which no encoded id can produce (encoded ids
/// are always even-length).
pub fn track_path(song_id: &str) -> String {
    const HEX: [u8; 16] = *b"0123456789abcdef";
    let mut out = String::with_capacity(TRACK_PATH_PREFIX.len() + song_id.len() * 2 + 1);
    out.push_str(TRACK_PATH_PREFIX);
    if song_id.is_empty() {
        out.push('0');
    } else {
        for byte in song_id.as_bytes() {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

/// The subset of `mpris:trackid` metadata we publish, as plain owned values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackMetadata {
    pub track_id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub length_us: i64,
    pub art_url: Option<String>,
}

/// Song -> MPRIS metadata fields. `art_url` wins over the song thumbnail;
/// empty strings are dropped rather than published.
pub fn track_metadata(song: &Song, art_url: Option<&str>) -> TrackMetadata {
    let thumb = song.thumbnail_url.as_deref().filter(|url| !url.is_empty());
    let preferred = art_url.filter(|url| !url.is_empty());
    TrackMetadata {
        track_id: track_path(&song.id),
        title: song.title.clone(),
        artists: song
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect(),
        album: song
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .filter(|title| !title.is_empty()),
        length_us: ms_to_us(u64::from(song.duration.unwrap_or(0)).saturating_mul(1000)),
        art_url: preferred.or(thumb).map(str::to_owned),
    }
}

/// A property whose published value changed between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    PlaybackStatus,
    LoopStatus,
    Shuffle,
    Volume,
    Metadata,
    CanGoNext,
    CanGoPrevious,
}

/// MPRIS `Volume`: muted reports 0.0, otherwise the player volume clamped to
/// `0.0..=1.0` (`f64::NAN` becomes 0.0).
pub fn volume_of(snapshot: &PlayerSnapshot) -> f64 {
    if snapshot.muted {
        return 0.0;
    }
    let volume = f64::from(snapshot.volume);
    if volume.is_nan() {
        0.0
    } else {
        volume.clamp(0.0, 1.0)
    }
}

/// `CanGoNext`: there is a track and the queue continues (repeat `All`/`One`
/// wraps; shuffle keeps every queued track reachable).
///
/// The `Off` arm uses `checked_add`, not `index + 1`: at `usize::MAX` the plain
/// addition wraps to 0 in release builds and would report `true`. A wrapped
/// index is never a real successor, so it reports `false`.
pub fn can_go_next(snapshot: &PlayerSnapshot) -> bool {
    if snapshot.current.is_none() {
        return false;
    }
    match (snapshot.queue_index, snapshot.repeat) {
        (None, _) => false,
        (Some(_), RepeatMode::All | RepeatMode::One) => true,
        (Some(index), RepeatMode::Off) => index
            .checked_add(1)
            .is_some_and(|next| next < snapshot.queue_len),
    }
}

/// `CanGoPrevious`: there is a track and something precedes it, or repeat wraps.
pub fn can_go_previous(snapshot: &PlayerSnapshot) -> bool {
    if snapshot.current.is_none() {
        return false;
    }
    match (snapshot.queue_index, snapshot.repeat) {
        (None, _) => false,
        (Some(_), RepeatMode::All | RepeatMode::One) => true,
        (Some(index), RepeatMode::Off) => index > 0,
    }
}

/// A snapshot plus the artwork override the control reported alongside it.
///
/// The art URL is not part of [`PlayerSnapshot`], but it is part of the
/// published metadata, so it is tracked next to the snapshot for diffing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Published {
    pub snapshot: PlayerSnapshot,
    pub art_url: Option<String>,
}

impl Published {
    pub fn new(snapshot: PlayerSnapshot, art_url: Option<String>) -> Self {
        Self { snapshot, art_url }
    }
}

/// Properties that must be emitted as `PropertiesChanged` moving from `prev`
/// to `next` (each tracked value appears at most once, in a fixed order:
/// status, loop, shuffle, volume, metadata, CanGoNext, CanGoPrevious).
///
/// `prev == None` is the first publish: everything readable is announced.
pub fn snapshot_changes(prev: Option<&Published>, next: &Published) -> Vec<Change> {
    let previous = prev.map(|published| &published.snapshot);
    let current = &next.snapshot;
    let mut changes = Vec::new();

    if previous.map(|snapshot| transport_status(snapshot.status))
        != Some(transport_status(current.status))
    {
        changes.push(Change::PlaybackStatus);
    }
    if previous.map(|snapshot| transport_loop(snapshot.repeat))
        != Some(transport_loop(current.repeat))
    {
        changes.push(Change::LoopStatus);
    }
    if previous.map(|snapshot| snapshot.shuffle) != Some(current.shuffle) {
        changes.push(Change::Shuffle);
    }
    if previous.map(volume_of) != Some(volume_of(current)) {
        changes.push(Change::Volume);
    }

    let previous_identity = track_identity(
        prev.and_then(|published| published.snapshot.current.as_ref()),
        prev.and_then(|published| published.art_url.as_deref()),
    );
    let current_identity = track_identity(current.current.as_ref(), next.art_url.as_deref());
    if previous_identity != current_identity {
        changes.push(Change::Metadata);
    }

    let can_next = can_go_next(current);
    if previous.map(can_go_next) != Some(can_next) {
        changes.push(Change::CanGoNext);
    }
    let can_previous = can_go_previous(current);
    if previous.map(can_go_previous) != Some(can_previous) {
        changes.push(Change::CanGoPrevious);
    }
    changes
}

/// Everything that makes two published `Metadata` maps differ.
fn track_identity(song: Option<&Song>, art_url: Option<&str>) -> Option<TrackMetadata> {
    song.map(|song| track_metadata(song, art_url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_core::Artist;

    fn song(id: &str, duration: Option<u32>) -> Song {
        Song {
            id: id.into(),
            title: "Title".into(),
            artists: vec![Artist::new("A", None), Artist::new("B", None)],
            duration,
            thumbnail_url: Some("https://img/thumb".into()),
            album: Some(ml_core::AlbumRef {
                id: "al".into(),
                title: "Album".into(),
            }),
            ..Song::default()
        }
    }

    #[test]
    fn every_core_status_maps_to_a_protocol_status() {
        use PlaybackStatus::*;
        let cases = [
            (Playing, TransportStatus::Playing),
            (Buffering, TransportStatus::Playing),
            (Paused, TransportStatus::Paused),
            (Idle, TransportStatus::Stopped),
            (Loading, TransportStatus::Stopped),
            (Stopped, TransportStatus::Stopped),
            (Ended, TransportStatus::Stopped),
            (Error, TransportStatus::Stopped),
        ];
        for (core, protocol) in cases {
            assert_eq!(transport_status(core), protocol, "{core:?}");
        }
    }

    #[test]
    fn loop_status_round_trips() {
        for repeat in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            assert_eq!(repeat_from_loop(transport_loop(repeat)), repeat);
        }
        assert_eq!(transport_loop(RepeatMode::All), TransportLoop::Playlist);
        assert_eq!(transport_loop(RepeatMode::One), TransportLoop::Track);
        assert_eq!(transport_loop(RepeatMode::Off), TransportLoop::None);
    }

    #[test]
    fn microsecond_conversion_is_exact_or_saturated() {
        assert_eq!(ms_to_us(0), 0);
        assert_eq!(ms_to_us(1), 1_000);
        assert_eq!(ms_to_us(213_000), 213_000_000);
        assert_eq!(ms_to_us(i64::MAX as u64 / 1000), (i64::MAX / 1000) * 1000);
        assert_eq!(ms_to_us(u64::MAX), i64::MAX);
        assert!(ms_to_us(u64::MAX) > 0);
        // monotone on a sample
        let mut last = ms_to_us(0);
        for ms in [1u64, 999, 1000, 1_000_001, u64::MAX] {
            let now = ms_to_us(ms);
            assert!(now >= last);
            last = now;
        }
    }

    #[test]
    fn microseconds_to_milliseconds_truncates_toward_zero() {
        assert_eq!(us_to_ms(0), 0);
        assert_eq!(us_to_ms(999), 0);
        assert_eq!(us_to_ms(1_999), 1);
        assert_eq!(us_to_ms(-1_999), -1);
        assert_eq!(us_to_ms(i64::MIN), i64::MIN / 1000);
        assert_eq!(position_ms_from_us(-5), 0);
        assert_eq!(position_ms_from_us(2_499), 2);
        assert_eq!(position_ms_from_us(i64::MAX), (i64::MAX as u64) / 1000);
    }

    #[test]
    fn track_paths_are_valid_and_injective() {
        let ids = ["", "dQw4w9WgXcQ", "abc-_", "\u{0}", "\u{0}\u{0}", "é"];
        let mut seen = Vec::new();
        for id in ids {
            let path = track_path(id);
            assert!(path.starts_with(TRACK_PATH_PREFIX));
            let component = &path[TRACK_PATH_PREFIX.len()..];
            assert!(!component.is_empty(), "{path}");
            assert!(
                component
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{path}"
            );
            assert!(!seen.contains(&path), "collision on {id:?}");
            seen.push(path);
        }
        assert_eq!(
            track_path("dQw4w9WgXcQ"),
            "/com/metrolist/track/6451773477395767586351"
        );
        assert_eq!(track_path(""), "/com/metrolist/track/0");
    }

    #[test]
    fn metadata_prefers_explicit_art_and_drops_blanks() {
        let s = song("id", Some(213));
        let meta = track_metadata(&s, Some("file:///tmp/cover.png"));
        assert_eq!(meta.track_id, track_path("id"));
        assert_eq!(meta.title, "Title");
        assert_eq!(meta.artists, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(meta.album.as_deref(), Some("Album"));
        assert_eq!(meta.length_us, 213_000_000);
        assert_eq!(meta.art_url.as_deref(), Some("file:///tmp/cover.png"));

        let meta = track_metadata(&s, None);
        assert_eq!(meta.art_url.as_deref(), Some("https://img/thumb"));
        let meta = track_metadata(&s, Some(""));
        assert_eq!(meta.art_url.as_deref(), Some("https://img/thumb"));

        let blank = Song::default();
        let meta = track_metadata(&blank, None);
        assert_eq!(meta.length_us, 0);
        assert_eq!(meta.album, None);
        assert_eq!(meta.art_url, None);
        assert_eq!(meta.track_id, track_path(""));
    }

    #[test]
    fn volume_is_muted_and_clamped() {
        let mut snapshot = PlayerSnapshot::default();
        assert_eq!(volume_of(&snapshot), 1.0);
        snapshot.volume = 0.42;
        assert_eq!(volume_of(&snapshot), f64::from(0.42_f32));
        snapshot.volume = 2.5;
        assert_eq!(volume_of(&snapshot), 1.0);
        snapshot.volume = -1.0;
        assert_eq!(volume_of(&snapshot), 0.0);
        snapshot.volume = f32::NAN;
        assert_eq!(volume_of(&snapshot), 0.0);
        snapshot.volume = 0.7;
        snapshot.muted = true;
        assert_eq!(volume_of(&snapshot), 0.0);
    }

    #[test]
    fn queue_navigation_flags_follow_index_and_repeat() {
        let mut snapshot = PlayerSnapshot {
            current: Some(song("a", Some(1))),
            queue_len: 3,
            queue_index: Some(1),
            ..PlayerSnapshot::default()
        };
        assert!(can_go_next(&snapshot));
        assert!(can_go_previous(&snapshot));

        snapshot.queue_index = Some(0);
        assert!(can_go_next(&snapshot));
        assert!(!can_go_previous(&snapshot));

        snapshot.queue_index = Some(2);
        assert!(!can_go_next(&snapshot));
        assert!(can_go_previous(&snapshot));

        snapshot.queue_index = Some(2);
        snapshot.repeat = RepeatMode::All;
        assert!(can_go_next(&snapshot));
        snapshot.repeat = RepeatMode::One;
        assert!(can_go_next(&snapshot));

        // No current track, no queue index, or an empty queue: no navigation.
        snapshot.current = None;
        assert!(!can_go_next(&snapshot));
        assert!(!can_go_previous(&snapshot));
        snapshot.current = Some(song("a", Some(1)));
        snapshot.queue_index = None;
        assert!(!can_go_next(&snapshot));
        snapshot.repeat = RepeatMode::Off;
        snapshot.queue_index = Some(0);
        snapshot.queue_len = 0;
        assert!(!can_go_next(&snapshot));
    }

    /// `index + 1` would wrap to 0 at `usize::MAX` in a release build, which
    /// would report a successor that does not exist.
    #[test]
    fn can_go_next_does_not_wrap_at_the_index_limit() {
        let mut snapshot = PlayerSnapshot {
            current: Some(song("a", Some(1))),
            queue_len: 3,
            queue_index: Some(usize::MAX),
            ..PlayerSnapshot::default()
        };
        assert!(
            !can_go_next(&snapshot),
            "no index has a successor past usize::MAX"
        );
        // Repeat still wraps independently of the index arithmetic.
        snapshot.repeat = RepeatMode::All;
        assert!(can_go_next(&snapshot));
        snapshot.repeat = RepeatMode::One;
        assert!(can_go_next(&snapshot));
        // Even a full queue cannot contain an index above usize::MAX.
        snapshot.repeat = RepeatMode::Off;
        snapshot.queue_len = usize::MAX;
        assert!(!can_go_next(&snapshot));
        // The largest reachable successor is usize::MAX - 1 < usize::MAX.
        snapshot.queue_index = Some(usize::MAX - 2);
        assert!(can_go_next(&snapshot));
        // At the very edge index + 1 == queue_len, so there is no successor.
        snapshot.queue_index = Some(usize::MAX - 1);
        assert!(!can_go_next(&snapshot));
    }

    #[test]
    fn first_snapshot_publishes_every_readable_property() {
        let snapshot = PlayerSnapshot {
            status: PlaybackStatus::Playing,
            ..PlayerSnapshot::default()
        };
        let published = Published::new(snapshot, None);
        assert_eq!(
            snapshot_changes(None, &published),
            vec![
                Change::PlaybackStatus,
                Change::LoopStatus,
                Change::Shuffle,
                Change::Volume,
                Change::CanGoNext,
                Change::CanGoPrevious,
            ],
            "no current track, so Metadata stays empty and is not announced"
        );

        let with_track = Published::new(
            PlayerSnapshot {
                current: Some(song("a", Some(1))),
                ..PlayerSnapshot::default()
            },
            None,
        );
        assert!(
            snapshot_changes(None, &with_track).contains(&Change::Metadata),
            "a track on first publish announces metadata"
        );
    }

    #[test]
    fn unchanged_snapshot_publishes_nothing() {
        let mut snapshot = PlayerSnapshot {
            current: Some(song("a", Some(100))),
            queue_len: 2,
            queue_index: Some(0),
            status: PlaybackStatus::Playing,
            ..PlayerSnapshot::default()
        };
        snapshot.volume = 0.5;
        snapshot.repeat = RepeatMode::All;
        let published = Published::new(snapshot.clone(), None);
        assert!(snapshot_changes(Some(&published), &published).is_empty());

        // Position and buffering are not published as PropertiesChanged.
        let mut moved = snapshot;
        moved.position_ms = 30_000;
        moved.buffering = true;
        assert!(snapshot_changes(Some(&published), &Published::new(moved, None)).is_empty());
    }

    #[test]
    fn changes_are_reported_once_in_fixed_order() {
        let first = PlayerSnapshot {
            current: Some(song("a", Some(100))),
            queue_len: 2,
            queue_index: Some(0),
            status: PlaybackStatus::Playing,
            volume: 0.5,
            ..PlayerSnapshot::default()
        };
        let mut second = first.clone();
        second.status = PlaybackStatus::Paused;
        second.repeat = RepeatMode::One;
        second.shuffle = true;
        second.volume = 0.2;
        second.current = Some(song("b", Some(100)));
        second.queue_index = Some(1);

        let changes = snapshot_changes(
            Some(&Published::new(first.clone(), None)),
            &Published::new(second, None),
        );
        // `CanGoNext` stays true throughout (repeat One wraps), so it is absent.
        assert_eq!(
            changes,
            vec![
                Change::PlaybackStatus,
                Change::LoopStatus,
                Change::Shuffle,
                Change::Volume,
                Change::Metadata,
                Change::CanGoPrevious,
            ]
        );
    }

    #[test]
    fn metadata_change_ignores_unpublished_fields() {
        let base = Published::new(
            PlayerSnapshot {
                current: Some(song("a", Some(100))),
                ..PlayerSnapshot::default()
            },
            None,
        );
        let mut same_published = base.clone();
        if let Some(track) = same_published.snapshot.current.as_mut() {
            track.liked = Some(1234);
            track.explicit = true;
            track.set_video_id = Some("vid".into());
        }
        assert!(snapshot_changes(Some(&base), &same_published).is_empty());

        let mut retitled = base.clone();
        if let Some(track) = retitled.snapshot.current.as_mut() {
            track.title = "Other".into();
        }
        assert!(snapshot_changes(Some(&base), &retitled).contains(&Change::Metadata));

        // Changing the artwork override also republishes metadata.
        let mut art = base.clone();
        art.art_url = Some("file:///art.png".into());
        assert!(snapshot_changes(Some(&base), &art).contains(&Change::Metadata));

        // Dropping the track republishes an empty metadata map.
        let mut cleared = base.clone();
        cleared.snapshot.current = None;
        assert!(snapshot_changes(Some(&base), &cleared).contains(&Change::Metadata));
    }
}
