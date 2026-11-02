use vstd::prelude::*;

verus! {

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode { Off, All, One }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QueueSource { List, Album, Playlist, Radio, Artist, Local }

pub struct Queue {
    pub songs: Vec<u32>,
    pub order: Vec<usize>,
    pub position: usize,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub source: QueueSource,
    pub shuffle_seed: u64,
}

pub open spec fn is_perm(o: Seq<usize>, n: nat) -> bool {
    o.len() == n
        && forall|i: int| 0 <= i < n ==> 0 <= #[trigger] o[i] < n
        && forall|i: int, j: int| 0 <= i < j < n ==> #[trigger] o[i] != #[trigger] o[j]
}

pub open spec fn cycle(m: RepeatMode) -> RepeatMode {
    match m {
        RepeatMode::Off => RepeatMode::All,
        RepeatMode::All => RepeatMode::One,
        RepeatMode::One => RepeatMode::Off,
    }
}

impl RepeatMode {
    pub fn cycle(self) -> (r: Self)
        ensures r == cycle(self),
    {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }

    pub fn as_str(self) -> (r: &'static str)
        ensures
            match self {
                RepeatMode::Off => r == "off",
                RepeatMode::All => r == "all",
                RepeatMode::One => r == "one",
            },
    {
        match self {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        }
    }
}


pub open spec fn remap_val(x: usize, removed: usize) -> usize {
    if x > removed { (x - 1) as usize } else { x }
}

pub open spec fn remove_position(old_pos: int, pos: int, new_len: int) -> int {
    if new_len == 0 {
        0
    } else if pos < old_pos {
        old_pos - 1
    } else if old_pos >= new_len {
        new_len - 1
    } else {
        old_pos
    }
}


pub open spec fn move_position(old_pos: int, from: int, to: int) -> int {
    if old_pos == from {
        to
    } else if from < old_pos && to >= old_pos {
        old_pos - 1
    } else if from > old_pos && to <= old_pos {
        old_pos + 1
    } else {
        old_pos
    }
}

impl Queue {
    pub open spec fn inv(&self) -> bool {
        is_perm(self.order@, self.songs@.len())
            && (if self.songs@.len() == 0 {
                self.position == 0
            } else {
                self.position < self.order@.len()
            })
    }

    pub open spec fn song_at_spec(&self, position: int) -> Option<u32> {
        if position < self.order@.len() {
            Some(self.songs@[self.order@[position] as int])
        } else {
            None
        }
    }

    pub open spec fn current_spec(&self) -> Option<u32> {
        self.song_at_spec(self.position as int)
    }

    pub fn new(songs: Vec<u32>, source: QueueSource) -> (r: Self)
        ensures
            r.inv(),
            r.songs@ == songs@,
            r.position == 0,
            !r.shuffle,
            r.order@.len() == songs@.len(),
            forall|j: int| 0 <= j < songs@.len() ==> #[trigger] r.order@[j] == j as usize,
    {
        let n = songs.len();
        let mut order: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < n
            invariant
                i <= n,
                order@.len() == i,
                forall|j: int| 0 <= j < i ==> #[trigger] order@[j] == j as usize,
            decreases n - i,
        {
            order.push(i);
            i += 1;
        }
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

    pub fn len(&self) -> (r: usize)
        ensures r as int == self.songs@.len(),
    {
        self.songs.len()
    }

    pub fn is_empty(&self) -> (r: bool)
        ensures r == (self.songs@.len() == 0),
    {
        self.songs.is_empty()
    }

    pub fn position(&self) -> (r: usize)
        requires self.inv()
        ensures r == self.position,
    {
        self.position
    }

    pub fn is_shuffled(&self) -> (r: bool)
        ensures r == self.shuffle,
    {
        self.shuffle
    }

    pub fn repeat_mode(&self) -> (r: RepeatMode)
        ensures r == self.repeat,
    {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode)
        requires old(self).inv()
        ensures final(self).inv(), final(self).repeat == repeat,
    {
        self.repeat = repeat;
    }

    pub fn cycle_repeat(&mut self) -> (r: RepeatMode)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).repeat == cycle(old(self).repeat),
            r == final(self).repeat,
    {
        self.repeat = self.repeat.cycle();
        self.repeat
    }

    pub fn song_at(&self, position: usize) -> (r: Option<u32>)
        requires self.inv()
        ensures self.inv(), r == self.song_at_spec(position as int),
    {
        if position < self.order.len() {
            let original = self.order[position];
            if original < self.songs.len() {
                Some(self.songs[original])
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn current(&self) -> (r: Option<u32>)
        requires self.inv()
        ensures self.inv(), r == self.current_spec(),
    {
        self.song_at(self.position)
    }

    pub fn jump_to(&mut self, position: usize) -> (r: Option<u32>)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).order@ == old(self).order@,
            final(self).songs@ == old(self).songs@,
            (position as int) < old(self).order@.len() ==> final(self).position == position && r == final(self).current_spec(),
            (position as int) >= old(self).order@.len() ==> final(self).position == old(self).position && r.is_none(),
    {
        if position < self.order.len() {
            self.position = position;
            self.current()
        } else {
            None
        }
    }

    pub fn append(&mut self, song: u32)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@ == old(self).songs@.push(song),
            final(self).order@ == old(self).order@.push(old(self).songs@.len() as usize),
            final(self).position == old(self).position,
    {
        let original = self.songs.len();
        self.songs.push(song);
        self.order.push(original);
    }

    pub fn insert_next(&mut self, song: u32)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@ == old(self).songs@.push(song),
            old(self).order@.len() == 0 ==> (final(self).order@.len() == 1 && final(self).order@[0] == 0 && final(self).position == 0),
            old(self).order@.len() > 0 ==> (
                final(self).position == old(self).position
                    && final(self).order@ == old(self).order@.insert(old(self).position as int + 1, old(self).songs@.len() as usize)
            ),
    {
        let original = self.songs.len();
        self.songs.push(song);
        if self.order.is_empty() {
            self.order.push(original);
            self.position = 0;
        } else {
            self.order.insert(self.position + 1, original);
        }
    }

    pub fn clear(&mut self)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@.len() == 0,
            final(self).order@.len() == 0,
            final(self).position == 0,
    {
        self.songs.clear();
        self.order.clear();
        self.position = 0;
    }



    /// Move a track within playback order; returns false for out-of-range indices.
    pub fn move_item(&mut self, from: usize, to: usize) -> (r: bool)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@ == old(self).songs@,
            ((from as int) >= old(self).order@.len() || (to as int) >= old(self).order@.len()) ==> (
                r == false
                    && final(self).order@ == old(self).order@
                    && final(self).position == old(self).position
            ),
            (from as int) < old(self).order@.len() && (to as int) < old(self).order@.len() ==> r == true,
            (from as int) < old(self).order@.len() && (to as int) < old(self).order@.len() && from == to ==> (
                final(self).order@ == old(self).order@
                    && final(self).position == old(self).position
            ),
            (from as int) < old(self).order@.len() && (to as int) < old(self).order@.len() && from != to ==> (
                final(self).order@ == old(self).order@.remove(from as int).insert(to as int, old(self).order@[from as int])
                    && final(self).position as int == move_position(old(self).position as int, from as int, to as int)
            ),
    {
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


    /// Deterministic Fisher-Yates over the non-current indices. Keeps the
    /// current track first and leaves a permutation of `0..songs.len()`.
    fn permute(&mut self)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@ == old(self).songs@,
            final(self).shuffle == old(self).shuffle,
            final(self).repeat == old(self).repeat,
            old(self).order@.len() == 0 ==> (final(self).order@.len() == 0 && final(self).position == 0),
            old(self).order@.len() > 0 ==> (
                final(self).position == 0
                    && final(self).order@[0] == old(self).order@[old(self).position as int]
            ),
    {
        if self.order.is_empty() {
            self.order.clear();
            let mut i = 0;
            while i < self.songs.len()
                invariant
                    i <= self.songs@.len(),
                    self.order@.len() == i,
                    forall|j: int| 0 <= j < i ==> #[trigger] self.order@[j] == j as usize,
                decreases self.songs@.len() - i,
            {
                self.order.push(i);
                i += 1;
            }
            self.position = 0;
            return;
        }
        let current = self.order[self.position];
        let ghost n = self.songs@.len() as int;
        let mut rest: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < self.songs.len()
            invariant
                i as int <= self.songs@.len(),
                rest@.len() as int == i as int - (if current < i { 1int } else { 0int }),
                forall|k: int| 0 <= k < rest@.len() ==> 0 <= #[trigger] rest@[k] < i,
                forall|k: int| 0 <= k < rest@.len() ==> #[trigger] rest@[k] != current,
                forall|k: int, l: int| 0 <= k < l < rest@.len() ==> #[trigger] rest@[k] != #[trigger] rest@[l],
            decreases self.songs@.len() - i,
        {
            if i != current {
                rest.push(i);
            }
            i += 1;
        }
        self.shuffle_seed = self.shuffle_seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut state = self.shuffle_seed;
        let mut m = rest.len();
        while m > 1
            invariant
                m <= rest@.len(),
                rest@.len() as int == self.songs@.len() as int - 1,
                forall|k: int| 0 <= k < rest@.len() ==> 0 <= #[trigger] rest@[k] < self.songs@.len(),
                forall|k: int| 0 <= k < rest@.len() ==> #[trigger] rest@[k] != current,
                forall|k: int, l: int| 0 <= k < l < rest@.len() ==> #[trigger] rest@[k] != #[trigger] rest@[l],
            decreases m,
        {
            m -= 1;
            let j = (lcg(&mut state) % (m as u64 + 1)) as usize;
            rest.swap(m, j);
        }
        self.order.clear();
        self.order.push(current);
        let mut k = 0;
        while k < rest.len()
            invariant
                k <= rest@.len(),
                self.order@.len() == k + 1,
                self.order@[0] == current,
                forall|q: int| 1 <= q <= k ==> #[trigger] self.order@[q] == rest@[q - 1],
                rest@.len() as int == self.songs@.len() as int - 1,
                forall|q: int| 0 <= q < rest@.len() ==> 0 <= #[trigger] rest@[q] < self.songs@.len(),
                forall|q: int| 0 <= q < rest@.len() ==> #[trigger] rest@[q] != current,
                forall|q: int, l: int| 0 <= q < l < rest@.len() ==> #[trigger] rest@[q] != #[trigger] rest@[l],
            decreases rest@.len() - k,
        {
            self.order.push(rest[k]);
            k += 1;
        }
        self.position = 0;
        proof {
            assert forall|i: int| 0 <= i < self.order@.len() implies 0 <= #[trigger] self.order@[i] < self.songs@.len() by {
                if i == 0 {
                } else {
                }
            }
            assert forall|i: int, j: int| 0 <= i < j < self.order@.len() implies #[trigger] self.order@[i] != #[trigger] self.order@[j] by {
                if i == 0 {
                } else {
                }
            }
        }
    }


    /// Enable/disable shuffle. Enabling keeps the current track first.
    pub fn set_shuffle(&mut self, enabled: bool)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@ == old(self).songs@,
            final(self).shuffle == enabled,
            enabled == old(self).shuffle ==> (
                final(self).order@ == old(self).order@
                    && final(self).position == old(self).position
            ),
            enabled && !old(self).shuffle && old(self).order@.len() > 0 ==> (
                final(self).position == 0
                    && final(self).order@[0] == old(self).order@[old(self).position as int]
            ),
            !enabled && old(self).shuffle ==> forall|j: int| 0 <= j < final(self).order@.len() ==> #[trigger] final(self).order@[j] == j as usize,
            !enabled && old(self).shuffle && old(self).order@.len() > 0
                ==> final(self).position as int == old(self).order@[old(self).position as int],
            !enabled && old(self).shuffle && old(self).order@.len() == 0 ==> final(self).position == 0,
    {
        if enabled == self.shuffle {
            return;
        }
        self.shuffle = enabled;
        if enabled {
            self.permute();
        } else {
            let current_original = if self.position < self.order.len() {
                Some(self.order[self.position])
            } else {
                None
            };
            self.order.clear();
            let mut i = 0;
            while i < self.songs.len()
                invariant
                    i <= self.songs@.len(),
                    self.order@.len() == i,
                    forall|j: int| 0 <= j < i ==> #[trigger] self.order@[j] == j as usize,
                decreases self.songs@.len() - i,
            {
                self.order.push(i);
                i += 1;
            }
            self.position = match current_original {
                Some(v) => v,
                None => 0,
            };
        }
    }

    /// Generate a new permutation while shuffle is enabled.
    pub fn reshuffle(&mut self)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@ == old(self).songs@,
            !old(self).shuffle ==> (
                final(self).order@ == old(self).order@ && final(self).position == old(self).position
            ),
            old(self).shuffle && old(self).order@.len() == 0 ==> (
                final(self).order@.len() == 0 && final(self).position == 0
            ),
            old(self).shuffle && old(self).order@.len() > 0 ==> (
                final(self).position == 0
                    && final(self).order@[0] == old(self).order@[old(self).position as int]
            ),
    {
        if self.shuffle {
            self.permute();
        }
    }

    /// Replace all songs, keeping shuffle/repeat/source. Current resets to 0.
    pub fn replace_songs(&mut self, songs: Vec<u32>)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).songs@ == songs@,
            final(self).position == 0,
            !final(self).shuffle ==> forall|j: int| 0 <= j < final(self).order@.len() ==> #[trigger] final(self).order@[j] == j as usize,
            final(self).shuffle ==> final(self).order@.len() == songs@.len(),
    {
        self.songs = songs;
        self.order.clear();
        let mut i = 0;
        while i < self.songs.len()
            invariant
                i <= self.songs@.len(),
                self.order@.len() == i,
                forall|j: int| 0 <= j < i ==> #[trigger] self.order@[j] == j as usize,
            decreases self.songs@.len() - i,
        {
            self.order.push(i);
            i += 1;
        }
        self.position = 0;
        if self.shuffle {
            self.permute();
        }
    }

    /// Remove a playback-order position. If it is the current track, the
    /// successor becomes current (or the previous track if it was last).
    pub fn remove(&mut self, position: usize) -> (r: Option<u32>)
        requires old(self).inv()
        ensures
            final(self).inv(),
            (position as int) >= old(self).order@.len() ==> (
                r.is_none()
                    && final(self).order@ == old(self).order@
                    && final(self).songs@ == old(self).songs@
                    && final(self).position == old(self).position
            ),
            (position as int) < old(self).order@.len() ==> (
                r == Some(old(self).songs@[old(self).order@[position as int] as int])
                    && final(self).songs@ == old(self).songs@.remove(old(self).order@[position as int] as int)
                    && final(self).order@.len() == old(self).order@.len() - 1
                    && final(self).position as int
                        == remove_position(old(self).position as int, position as int, (old(self).order@.len() - 1) as int)
                    && forall|i: int| 0 <= i < final(self).order@.len() ==> #[trigger] final(self).order@[i]
                        == remap_val(old(self).order@.remove(position as int)[i], old(self).order@[position as int])
            ),
    {
        if position >= self.order.len() {
            return None;
        }
        let ghost pre_order = self.order@;
        let original = self.order.remove(position);
        let song = self.songs.remove(original);
        let ghost mid_order = self.order@;
        let mut index = 0;
        while index < self.order.len()
            invariant
                index <= self.order@.len(),
                self.order@.len() == mid_order.len(),
                forall|i: int| 0 <= i < index ==> #[trigger] self.order@[i] == remap_val(mid_order[i], original),
                forall|i: int| index <= i < mid_order.len() ==> #[trigger] self.order@[i] == mid_order[i],
            decreases self.order@.len() - index,
        {
            let v = self.order[index];
            if v > original {
                self.order.set(index, v - 1);
            }
            index += 1;
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

    pub fn next(&mut self) -> (r: Option<u32>)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).order@ == old(self).order@,
            final(self).songs@ == old(self).songs@,
            final(self).repeat == old(self).repeat,
            old(self).order@.len() == 0 ==> r.is_none() && final(self).position == 0,
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::One
                ==> r == old(self).current_spec() && final(self).position == old(self).position,
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::All
                ==> final(self).position as int == (old(self).position as int + 1) % (old(self).order@.len() as int)
                    && r == final(self).current_spec(),
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::Off
                && old(self).position as int + 1 >= old(self).order@.len()
                ==> r.is_none() && final(self).position == old(self).position,
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::Off
                && old(self).position as int + 1 < old(self).order@.len()
                ==> final(self).position as int == old(self).position as int + 1 && r == final(self).current_spec(),
    {
        if self.order.is_empty() {
            return None;
        }
        assert(self.position < self.order.len());
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

    pub fn previous(&mut self) -> (r: Option<u32>)
        requires old(self).inv()
        ensures
            final(self).inv(),
            final(self).order@ == old(self).order@,
            final(self).songs@ == old(self).songs@,
            final(self).repeat == old(self).repeat,
            old(self).order@.len() == 0 ==> r.is_none() && final(self).position == 0,
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::One
                ==> r == old(self).current_spec() && final(self).position == old(self).position,
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::All
                ==> final(self).position as int == (if old(self).position == 0 {
                        (old(self).order@.len() - 1) as int
                    } else {
                        old(self).position as int - 1
                    }) && r == final(self).current_spec(),
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::Off && old(self).position == 0
                ==> r.is_none() && final(self).position == 0,
            old(self).order@.len() > 0 && old(self).repeat == RepeatMode::Off && old(self).position > 0
                ==> final(self).position as int == old(self).position as int - 1 && r == final(self).current_spec(),
    {
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
                assert(self.position > 0);
                self.position -= 1;
            }
        }
        self.current()
    }
}


fn lcg(state: &mut u64) -> (r: u64)
{
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

} // verus!
