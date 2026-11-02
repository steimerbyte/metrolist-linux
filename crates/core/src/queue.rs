//! Queue state machine.
//!
//! Invariants:
//! - `order` is always a permutation of `0..songs.len()`. It is the playback
//!   order; `songs` keeps insertion order so shuffle can be undone.
//! - `position` indexes `order` while non-empty and is `0` when empty.
//! - Toggling shuffle on keeps the current track and places it first in a
//!   deterministic permutation of the remaining tracks. Toggling off restores
//!   insertion order at the current track's original index.
//!
//! `next`/`previous` honour repeat: `One` stays on the current track, `All`
//! wraps, `Off` stops at the ends (returning `None` without moving).

use serde::{Deserialize, Serialize};

use crate::models::Song;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    /// Off -> All -> One -> Off, matching the Android toggle order.
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }
}

/// Where the queue came from; preserved for "save queue" and radio continuation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueSource {
    #[default]
    List,
    Album,
    Playlist,
    Radio,
    Artist,
    Local,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Queue {
    songs: Vec<Song>,
    order: Vec<usize>,
    position: usize,
    shuffle: bool,
    repeat: RepeatMode,
    source: QueueSource,
    shuffle_seed: u64,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new(Vec::new(), QueueSource::List)
    }
}

impl Queue {
    pub fn new(songs: Vec<Song>, source: QueueSource) -> Self {
        let order = (0..songs.len()).collect();
        Self {
            songs,
            order,
            position: 0,
            shuffle: false,
            repeat: RepeatMode::Off,
            source,
            shuffle_seed: 0x2545_f491_4f6c_dd1d,
        }
    }

    pub fn len(&self) -> usize {
        self.songs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// Current position in playback order (0 when empty).
    pub fn position(&self) -> usize {
        self.position
    }

    pub fn source(&self) -> QueueSource {
        self.source
    }

    pub fn set_source(&mut self, source: QueueSource) {
        self.source = source;
    }

    pub fn is_shuffled(&self) -> bool {
        self.shuffle
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = self.repeat.cycle();
        self.repeat
    }

    pub fn current(&self) -> Option<&Song> {
        self.song_at(self.position)
    }

    /// Song at a playback-order position.
    pub fn song_at(&self, position: usize) -> Option<&Song> {
        self.order
            .get(position)
            .and_then(|original| self.songs.get(*original))
    }

    /// All songs in insertion order (stable identities, e.g. for persistence).
    pub fn songs(&self) -> &[Song] {
        &self.songs
    }

    /// Songs in playback order.
    pub fn iter(&self) -> impl Iterator<Item = &Song> {
        self.order
            .iter()
            .filter_map(|original| self.songs.get(*original))
    }

    /// Jump to a playback-order position.
    pub fn jump_to(&mut self, position: usize) -> Option<&Song> {
        if position < self.order.len() {
            self.position = position;
            self.current()
        } else {
            None
        }
    }

    pub fn append(&mut self, song: Song) {
        let original = self.songs.len();
        self.songs.push(song);
        self.order.push(original);
    }

    /// Insert after the current track; starts playback when the queue was empty.
    pub fn insert_next(&mut self, song: Song) {
        let original = self.songs.len();
        self.songs.push(song);
        if self.order.is_empty() {
            self.order.push(original);
            self.position = 0;
        } else {
            self.order.insert(self.position + 1, original);
        }
    }

    /// Remove a playback-order position. If it is the current track, the
    /// successor becomes current (or the previous track if it was last).
    pub fn remove(&mut self, position: usize) -> Option<Song> {
        if position >= self.order.len() {
            return None;
        }
        let original = self.order.remove(position);
        let song = self.songs.remove(original);
        for index in &mut self.order {
            if *index > original {
                *index -= 1;
            }
        }
        if self.order.is_empty() {
            self.position = 0;
        } else if position < self.position {
            self.position -= 1;
        } else if self.position >= self.order.len() {
            self.position = self.order.len() - 1;
        }
        Some(song)
    }

    /// Move a track within playback order; returns false for out-of-range indices.
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        let len = self.order.len();
        if from >= len || to >= len {
            return false;
        }
        if from == to {
            return true;
        }
        let item = self.order.remove(from);
        self.order.insert(to, item);
        if self.position == from {
            self.position = to;
        } else if from < self.position && to >= self.position {
            self.position -= 1;
        } else if from > self.position && to <= self.position {
            self.position += 1;
        }
        true
    }

    pub fn clear(&mut self) {
        self.songs.clear();
        self.order.clear();
        self.position = 0;
    }

    /// Replace all songs, keeping shuffle/repeat/source. Current resets to 0.
    pub fn replace_songs(&mut self, songs: Vec<Song>) {
        self.songs = songs;
        self.order = (0..self.songs.len()).collect();
        self.position = 0;
        if self.shuffle {
            self.permute();
        }
    }

    /// Enable/disable shuffle. Enabling keeps the current track first.
    pub fn set_shuffle(&mut self, enabled: bool) {
        if enabled == self.shuffle {
            return;
        }
        self.shuffle = enabled;
        if enabled {
            self.permute();
        } else {
            let current_original = self.order.get(self.position).copied();
            self.order = (0..self.songs.len()).collect();
            self.position = current_original.unwrap_or(0);
        }
    }

    /// Generate a new permutation while shuffle is enabled.
    pub fn reshuffle(&mut self) {
        if self.shuffle {
            self.permute();
        }
    }

    /// Next track for the player. Returns `None` at the end with repeat off.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&Song> {
        if self.order.is_empty() {
            return None;
        }
        match self.repeat {
            RepeatMode::One => return self.current(),
            RepeatMode::All => {
                self.position = (self.position + 1) % self.order.len();
            }
            RepeatMode::Off => {
                if self.position + 1 >= self.order.len() {
                    return None;
                }
                self.position += 1;
            }
        }
        self.current()
    }

    /// Previous track. With repeat off at the start it returns `None`.
    pub fn previous(&mut self) -> Option<&Song> {
        if self.order.is_empty() {
            return None;
        }
        match self.repeat {
            RepeatMode::One => return self.current(),
            RepeatMode::All => {
                self.position = if self.position == 0 {
                    self.order.len() - 1
                } else {
                    self.position - 1
                };
            }
            RepeatMode::Off => {
                if self.position == 0 {
                    return None;
                }
                self.position -= 1;
            }
        }
        self.current()
    }

    fn permute(&mut self) {
        let Some(current) = self.order.get(self.position).copied() else {
            self.order = (0..self.songs.len()).collect();
            self.position = 0;
            return;
        };
        let mut rest: Vec<usize> = (0..self.songs.len()).filter(|i| *i != current).collect();
        self.shuffle_seed = self.shuffle_seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut state = self.shuffle_seed;
        // Fisher–Yates with a deterministic LCG: shuffling is reproducible
        // across runs, which makes queue tests and bug reports stable.
        for i in (1..rest.len()).rev() {
            let j = (lcg(&mut state) % (i as u64 + 1)) as usize;
            rest.swap(i, j);
        }
        self.order.clear();
        self.order.push(current);
        self.order.extend(rest);
        self.position = 0;
    }
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Song {
        Song {
            id: id.to_string(),
            title: id.to_string(),
            ..Song::default()
        }
    }

    fn queue(ids: &[&str]) -> Queue {
        Queue::new(ids.iter().map(|id| song(id)).collect(), QueueSource::List)
    }

    #[test]
    fn empty_queue_has_no_current_and_cannot_advance() {
        let mut q = queue(&[]);
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert!(q.current().is_none());
        assert!(q.next().is_none());
        assert!(q.previous().is_none());
        assert_eq!(q.position(), 0);
        assert!(q.remove(0).is_none());
        assert!(!q.move_item(0, 0));
    }

    #[test]
    fn repeat_one_with_one_item_stays_put() {
        let mut q = queue(&["a"]);
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.current().unwrap().id, "a");
        assert_eq!(q.next().unwrap().id, "a");
        assert_eq!(q.position(), 0);
        assert_eq!(q.previous().unwrap().id, "a");
        assert_eq!(q.position(), 0);
    }

    #[test]
    fn repeat_off_stops_at_ends_all_wraps() {
        let mut q = queue(&["a", "b"]);
        assert_eq!(q.current().unwrap().id, "a");
        assert_eq!(q.next().unwrap().id, "b");
        assert!(q.next().is_none());
        assert_eq!(q.current().unwrap().id, "b");
        assert!(q.previous().unwrap().id == "a");
        assert!(q.previous().is_none());

        q.set_repeat(RepeatMode::All);
        assert_eq!(q.current().unwrap().id, "a");
        assert_eq!(q.previous().unwrap().id, "b");
        assert_eq!(q.next().unwrap().id, "a");
    }

    #[test]
    fn remove_current_selects_successor() {
        let mut q = queue(&["a", "b", "c"]);
        q.jump_to(1);
        let removed = q.remove(1).unwrap();
        assert_eq!(removed.id, "b");
        assert_eq!(q.len(), 2);
        assert_eq!(q.current().unwrap().id, "c");
        assert_eq!(q.position(), 1);

        let removed = q.remove(1).unwrap();
        assert_eq!(removed.id, "c");
        assert_eq!(q.current().unwrap().id, "a");
        assert_eq!(q.position(), 0);

        q.remove(0);
        assert!(q.is_empty());
        assert_eq!(q.position(), 0);
    }

    #[test]
    fn remove_before_current_adjusts_position() {
        let mut q = queue(&["a", "b", "c"]);
        q.jump_to(2);
        q.remove(0).unwrap();
        assert_eq!(q.current().unwrap().id, "c");
        assert_eq!(q.position(), 1);
    }

    #[test]
    fn shuffle_is_a_permutation_that_keeps_current() {
        let mut q = queue(&["a", "b", "c", "d", "e"]);
        q.jump_to(3);
        assert_eq!(q.current().unwrap().id, "d");
        q.set_shuffle(true);
        assert_eq!(q.current().unwrap().id, "d");
        assert_eq!(q.position(), 0);

        let order: Vec<_> = q.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order.len(), 5);
        assert_eq!(order[0], "d");
        for id in ["a", "b", "c", "e"] {
            assert!(order.contains(&id), "missing {id} from {order:?}");
        }

        q.set_shuffle(false);
        assert_eq!(q.current().unwrap().id, "d");
        assert_eq!(q.position(), 3);
        let natural: Vec<_> = q.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(natural, ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn shuffle_with_repeat_all_visits_every_track_then_wraps() {
        let mut q = queue(&["a", "b", "c", "d", "e"]);
        q.set_shuffle(true);
        q.set_repeat(RepeatMode::All);
        let start = q.current().unwrap().id.clone();
        let mut seen = vec![start];
        for _ in 0..4 {
            seen.push(q.next().unwrap().id.clone());
        }
        seen.sort();
        assert_eq!(seen, ["a", "b", "c", "d", "e"]);
        // Four advances reached the last position; one more wraps to the start.
        assert_eq!(q.position(), 4);
        q.next().unwrap();
        assert_eq!(q.position(), 0);
    }

    #[test]
    fn manual_queue_operations() {
        let mut q = queue(&["a", "b"]);
        q.jump_to(0);
        q.insert_next(song("x"));
        let order: Vec<_> = q.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order, ["a", "x", "b"]);
        assert_eq!(q.current().unwrap().id, "a");

        assert!(q.move_item(2, 0));
        let order: Vec<_> = q.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order, ["b", "a", "x"]);
        assert_eq!(q.current().unwrap().id, "a");
        assert_eq!(q.position(), 1);

        q.append(song("z"));
        assert_eq!(q.len(), 4);
        assert_eq!(q.iter().last().unwrap().id, "z");

        let mut empty = queue(&[]);
        empty.insert_next(song("first"));
        assert_eq!(empty.current().unwrap().id, "first");
        assert_eq!(empty.position(), 0);
    }

    #[test]
    fn repeat_cycle_matches_android_order() {
        assert_eq!(RepeatMode::Off.cycle(), RepeatMode::All);
        assert_eq!(RepeatMode::All.cycle(), RepeatMode::One);
        assert_eq!(RepeatMode::One.cycle(), RepeatMode::Off);
    }
}
