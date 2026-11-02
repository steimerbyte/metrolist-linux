//! Conversions between `ml-core` domain models and the wire `TrackInfo`.
//!
//! `TrackInfo.duration` is milliseconds on the wire (the Android player uses
//! `MediaMetadata.duration * 1000`), while `Song.duration` is seconds.

use ml_core::models::{AlbumRef, Artist, Song};

use crate::proto::TrackInfo;

/// `Song` -> room track. Combines artists with `", "`, the same separator the
/// Android app uses for `TrackInfo.artist`.
pub fn song_to_track_info(song: &Song) -> TrackInfo {
    TrackInfo {
        id: song.id.clone(),
        title: song.title.clone(),
        artist: song.artist_names(),
        album: song
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default(),
        duration: song
            .duration
            .map(|seconds| seconds as i64 * 1_000)
            .unwrap_or(0),
        thumbnail: song.thumbnail_url.clone().unwrap_or_default(),
        suggested_by: String::new(),
    }
}

/// Room track -> `Song`. Empty optional fields become absent; duration is
/// truncated to whole seconds.
pub fn track_info_to_song(track: &TrackInfo) -> Song {
    let artists = if track.artist.is_empty() {
        Vec::new()
    } else {
        vec![Artist {
            id: None,
            name: track.artist.clone(),
        }]
    };
    Song {
        id: track.id.clone(),
        title: track.title.clone(),
        artists,
        duration: (track.duration > 0)
            .then(|| (track.duration / 1_000).min(u32::MAX as i64) as u32),
        thumbnail_url: (!track.thumbnail.is_empty()).then(|| track.thumbnail.clone()),
        album: (!track.album.is_empty()).then(|| AlbumRef {
            id: String::new(),
            title: track.album.clone(),
        }),
        ..Song::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_round_trip_preserves_identity_metadata() {
        let song = Song {
            id: "kJQP7kiw5Fk".to_string(),
            title: "Despacito".to_string(),
            artists: vec![
                Artist::new("Luis Fonsi", Some("a1".into())),
                Artist::new("Daddy Yankee", Some("a2".into())),
            ],
            duration: Some(281),
            thumbnail_url: Some("https://i.ytimg.com/vi/kJQP7kiw5Fk/maxresdefault.jpg".into()),
            album: Some(AlbumRef {
                id: "album".to_string(),
                title: "Despacito".to_string(),
            }),
            ..Song::default()
        };

        let track = song_to_track_info(&song);
        assert_eq!(track.id, song.id);
        assert_eq!(track.title, song.title);
        assert_eq!(track.artist, "Luis Fonsi, Daddy Yankee");
        assert_eq!(track.album, "Despacito");
        assert_eq!(track.duration, 281_000);
        assert_eq!(track.thumbnail, song.thumbnail_url.clone().unwrap());

        let back = track_info_to_song(&track);
        assert_eq!(back.id, song.id);
        assert_eq!(back.title, song.title);
        assert_eq!(back.artist_names(), "Luis Fonsi, Daddy Yankee");
        assert_eq!(back.duration, Some(281));
        assert_eq!(back.thumbnail_url, song.thumbnail_url);
        assert_eq!(back.album.unwrap().title, "Despacito");
    }

    #[test]
    fn empty_optional_fields_stay_absent() {
        let song = Song {
            id: "id".into(),
            title: "title".into(),
            ..Song::default()
        };
        let track = song_to_track_info(&song);
        assert!(track.artist.is_empty());
        assert!(track.album.is_empty());
        assert!(track.thumbnail.is_empty());
        assert_eq!(track.duration, 0);

        let back = track_info_to_song(&track);
        assert!(back.artists.is_empty());
        assert!(back.album.is_none());
        assert!(back.thumbnail_url.is_none());
        assert_eq!(back.duration, None);
    }

    #[test]
    fn fractional_or_negative_durations_are_total() {
        let track = TrackInfo {
            id: "id".into(),
            duration: 2_999,
            ..Default::default()
        };
        assert_eq!(track_info_to_song(&track).duration, Some(2));

        let negative = TrackInfo {
            id: "id".into(),
            duration: -5_000,
            ..Default::default()
        };
        assert_eq!(track_info_to_song(&negative).duration, None);
    }
}
