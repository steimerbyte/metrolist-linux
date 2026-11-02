use vstd::prelude::*;

verus! {

// ---------- crates/together/src/model.rs ----------
// Strings are modelled by an opaque symbol plus presence: `Option<u32>` is
// "empty string" (None) vs "non-empty, symbol s" (Some(s)).

/// `ml_core::Song`, reduced to the fields the two conversions read.
pub struct Song {
    pub id: u32,
    /// Seconds.
    pub duration: Option<u32>,
    pub album: Option<u32>,
    pub thumbnail: Option<u32>,
    /// `Vec<Artist>` joined by `artist_names()`: None = no artists.
    pub artist: Option<u32>,
}

/// `proto::TrackInfo` wire track, reduced to the same fields.
pub struct TrackInfo {
    pub id: u32,
    /// Milliseconds on the wire.
    pub duration: i64,
    pub album: Option<u32>,
    pub thumbnail: Option<u32>,
    pub artist: Option<u32>,
}

pub open spec fn ms_of(seconds: u32) -> int {
    (seconds as int) * 1000
}

/// Seconds -> wire milliseconds, `None`/`Some(0)` -> 0.
pub fn song_to_track_info(song: &Song) -> (r: TrackInfo)
    ensures
        r.id == song.id,
        r.album == song.album,
        r.thumbnail == song.thumbnail,
        r.artist == song.artist,
        match song.duration {
            None => r.duration == 0,
            Some(seconds) => r.duration == ms_of(seconds),
        },
{
    TrackInfo {
        id: song.id,
        duration: match song.duration {
            Some(seconds) => (seconds as i64) * 1000,
            None => 0,
        },
        album: song.album,
        thumbnail: song.thumbnail,
        artist: song.artist,
    }
}

pub open spec fn seconds_of(ms: int) -> Option<u32> {
    if ms > 0 {
        if ms / 1000 < u32::MAX as int {
            Some((ms / 1000) as u32)
        } else {
            Some(u32::MAX)
        }
    } else {
        None
    }
}

/// Wire milliseconds -> whole seconds; non-positive durations become absent.
pub fn track_info_to_song(track: &TrackInfo) -> (r: Song)
    ensures
        r.id == track.id,
        r.album == track.album,
        r.thumbnail == track.thumbnail,
        r.artist == track.artist,
        r.duration == seconds_of(track.duration as int),
        r.duration.is_some() == (track.duration > 0),
{
    Song {
        id: track.id,
        duration: if track.duration > 0 {
            let seconds = track.duration / 1000;
            Some(if seconds < u32::MAX as i64 {
                seconds as u32
            } else {
                u32::MAX
            })
        } else {
            None
        },
        album: track.album,
        thumbnail: track.thumbnail,
        artist: track.artist,
    }
}

// ---------- crates/together/src/clock.rs ----------

pub const MAX_SAMPLE_AGE_MS: i64 = 60_000;
pub const GOOD_SAMPLE_MARGIN_MS: i64 = 50;
const GOOD_SAMPLE_WEIGHT: f64 = 0.25;
const POOR_SAMPLE_WEIGHT: f64 = 0.05;

/// Server wall clock mapped onto the local monotonic clock. The `f64` offset is
/// kept: the real type is `Option<f64>`, and vstd has no float reasoning, so
/// only the guards, the best-RTT tracker and the position arithmetic below are
/// proven. `now_at`'s value is opaque; its *presence* is proven.
pub struct ServerClock {
    pub server_offset_ms: Option<f64>,
    pub best_round_trip_ms: u64,
}

/// The acceptance guard of `record_pong`, verbatim.
pub open spec fn sample_accepted(
    client_time: i64,
    server_receive_time: i64,
    server_send_time: i64,
    received_at: i64,
) -> bool {
    !(client_time <= 0
        || client_time > received_at
        || received_at - client_time > MAX_SAMPLE_AGE_MS)
        && !(server_receive_time <= 0 || server_send_time < server_receive_time)
}

/// `max(0, round_trip - server_processing)`: the network share of the RTT.
pub open spec fn network_round_trip(round_trip: int, server_processing: int) -> int {
    if round_trip > server_processing {
        (round_trip as int) - (server_processing as int)
    } else {
        0
    }
}

/// Min-tracking: the best RTT never grows.
pub open spec fn best_after(best: u64, rtt: u64) -> u64 {
    if rtt < best {
        rtt
    } else {
        best
    }
}

impl ServerClock {
    pub fn new() -> (r: Self)
        ensures
            r.server_offset_ms.is_none(),
            r.best_round_trip_ms == u64::MAX,
    {
        ServerClock { server_offset_ms: None, best_round_trip_ms: u64::MAX }
    }

    pub fn reset(&mut self)
        ensures
            final(self).server_offset_ms.is_none(),
            final(self).best_round_trip_ms == u64::MAX,
    {
        self.server_offset_ms = None;
        self.best_round_trip_ms = u64::MAX;
    }

    pub fn has_offset(&self) -> (r: bool)
        ensures r == self.server_offset_ms.is_some(),
    {
        self.server_offset_ms.is_some()
    }

    pub fn server_offset_ms(&self) -> (r: Option<f64>)
        ensures r == self.server_offset_ms,
    {
        self.server_offset_ms
    }

    /// `(best != u64::MAX).then_some(best)`.
    pub fn best_round_trip_ms(&self) -> (r: Option<u64>)
        ensures
            r.is_some() == (self.best_round_trip_ms != u64::MAX),
            r.is_some() ==> r.unwrap() == self.best_round_trip_ms,
    {
        if self.best_round_trip_ms != u64::MAX {
            Some(self.best_round_trip_ms)
        } else {
            None
        }
    }

    /// The real body, including the `f64` filter update. The guards, the RTT
    /// bound and the first-sample flag are proven; the filter value is not.
    pub fn record_pong(
        &mut self,
        client_time: i64,
        server_receive_time: i64,
        server_send_time: i64,
        received_at: i64,
    ) -> (r: bool)
        ensures
            r == (old(self).server_offset_ms.is_none() && sample_accepted(
                client_time,
                server_receive_time,
                server_send_time,
                received_at,
            )),
            r ==> final(self).server_offset_ms.is_some(),
            sample_accepted(client_time, server_receive_time, server_send_time, received_at)
                ==> final(self).best_round_trip_ms == best_after(
                old(self).best_round_trip_ms,
                network_round_trip(
                    received_at as int - client_time as int,
                    server_send_time as int - server_receive_time as int,
                ) as u64,
            ),
            !sample_accepted(client_time, server_receive_time, server_send_time, received_at)
                ==> final(self).server_offset_ms == old(self).server_offset_ms
                    && final(self).best_round_trip_ms == old(self).best_round_trip_ms,
    {
        if client_time <= 0
            || client_time > received_at
            || received_at - client_time > MAX_SAMPLE_AGE_MS
        {
            return false;
        }
        if server_receive_time <= 0 || server_send_time < server_receive_time {
            return false;
        }

        let round_trip = received_at - client_time;
        let server_processing = server_send_time - server_receive_time;
        let network_round_trip = if round_trip > server_processing {
            round_trip - server_processing
        } else {
            0
        };
        let sample_offset =
            server_send_time as f64 + network_round_trip as f64 / 2.0 - received_at as f64;
        let previous_offset = self.server_offset_ms;

        if (network_round_trip as u64) < self.best_round_trip_ms {
            self.best_round_trip_ms = network_round_trip as u64;
        }
        let weight = if network_round_trip <= self.best_round_trip_ms as i64 + GOOD_SAMPLE_MARGIN_MS
        {
            GOOD_SAMPLE_WEIGHT
        } else {
            POOR_SAMPLE_WEIGHT
        };
        self.server_offset_ms = Some(match previous_offset {
            Some(previous) => previous + weight * (sample_offset - previous),
            None => sample_offset,
        });
        previous_offset.is_none()
    }

    /// Server time at the given local monotonic timestamp.
    pub fn now_at(&self, elapsed_realtime: i64) -> (r: Option<i64>)
        ensures r.is_some() == self.server_offset_ms.is_some(),
    {
        self.server_offset_ms.map(|offset| (elapsed_realtime as f64 + offset) as i64)
    }

    /// Extrapolate a playback position captured at `effective_at_server_time`.
    /// Returns `position` unchanged while paused, without an anchor, or before
    /// the first pong.
    pub fn position_at(
        &self,
        position: i64,
        effective_at_server_time: Option<i64>,
        is_playing: bool,
        elapsed_realtime: i64,
    ) -> (r: i64)
        ensures
            r >= position,
            r > position ==> is_playing
                && self.server_offset_ms.is_some()
                && match effective_at_server_time {
                Some(at) => at > 0,
                None => false,
            },
            (!is_playing
                || self.server_offset_ms.is_none()
                || match effective_at_server_time {
                Some(at) => at <= 0,
                None => true,
            }) ==> r == position,
    {
        let effective_at = match effective_at_server_time {
            Some(at) if at > 0 => at,
            _ => return position,
        };
        if !is_playing {
            return position;
        }
        let server_now = match self.now_at(elapsed_realtime) {
            Some(now) => now,
            None => return position,
        };
        let delta = if server_now > effective_at {
            if effective_at < 0 && server_now > i64::MAX + effective_at {
                i64::MAX
            } else {
                server_now - effective_at
            }
        } else {
            0
        };
        if position > i64::MAX - delta {
            i64::MAX
        } else {
            position + delta
        }
    }
}

/// The saturating tail of `position_at` in exact integers: the proven twin of
/// the transcribed expression, whose only opaque input is `now`.
pub open spec fn advance_delta(now: int, at: int) -> int {
    if now > at {
        if now - at > i64::MAX as int {
            i64::MAX as int
        } else {
            now - at
        }
    } else {
        0
    }
}

pub open spec fn advance_spec(position: int, now: int, at: int) -> int {
    let delta = advance_delta(now, at);
    let sum = position + delta;
    if sum > i64::MAX as int {
        i64::MAX as int
    } else {
        sum
    }
}

/// `position_at`'s last statement, verbatim, as a total function.
pub fn saturating_advance(position: i64, server_now: i64, effective_at: i64) -> (r: i64)
    ensures r == advance_spec(position as int, server_now as int, effective_at as int),
{
    let delta = if server_now > effective_at {
        if effective_at < 0 && server_now > i64::MAX + effective_at {
            i64::MAX
        } else {
            server_now - effective_at
        }
    } else {
        0
    };
    assert(delta as int == advance_delta(server_now as int, effective_at as int));
    assert(delta as int >= 0);
    if position > i64::MAX - delta {
        i64::MAX
    } else {
        position + delta
    }
}

// ---------- crates/together/src/sync.rs ----------

pub const SOFT_SYNC_THRESHOLD_MS: i64 = 50;
pub const HARD_SYNC_THRESHOLD_MS: i64 = 750;
pub const ACTIVE_PLAYBACK_SYNC_TOLERANCE_MS: i64 = 2_000;
pub const MAX_RECONNECT_ATTEMPTS: u32 = 15;
pub const INITIAL_RECONNECT_DELAY_MS: u64 = 1_000;
pub const MAX_RECONNECT_DELAY_MS: u64 = 120_000;

/// What the drift corrector should do next. `Speed` carries the multiplier in
/// percent (`1.0 ± 0.02`); the real type is `f32`, which vstd cannot reason
/// about, and the player only consumes the multiplier.
pub enum DriftDecision {
    Hold,
    Seek(i64),
    Speed(i32),
}

pub open spec fn drift_of(target: int, current: int) -> int {
    target - current
}

pub open spec fn drift_abs(target: int, current: int) -> int {
    if target >= current {
        target - current
    } else {
        current - target
    }
}

pub open spec fn drift_decision_spec(target: int, current: int, ready: bool) -> DriftDecision {
    if !ready {
        DriftDecision::Hold
    } else {
        let drift = drift_of(target, current);
        if drift_abs(target, current) <= SOFT_SYNC_THRESHOLD_MS as int {
            DriftDecision::Hold
        } else if drift_abs(target, current) >= HARD_SYNC_THRESHOLD_MS as int {
            DriftDecision::Seek(target as i64)
        } else if drift > 0 {
            DriftDecision::Speed(102)
        } else {
            DriftDecision::Speed(98)
        }
    }
}

/// One drift-correction iteration. `unsigned_abs()` is modelled as
/// `if drift >= 0 { drift } else { -drift }`, exact for every drift except
/// `i64::MIN`; the contract's ±2^60 ms bound (a track position is < 2^40 ms)
/// puts `i64::MIN` out of reach.
pub fn drift_decision(
    target_position_ms: i64,
    current_position_ms: i64,
    playback_ready: bool,
) -> (r: DriftDecision)
    requires
        -0x1000_0000_0000_0000 <= target_position_ms <= 0x1000_0000_0000_0000,
        -0x1000_0000_0000_0000 <= current_position_ms <= 0x1000_0000_0000_0000,
    ensures
        r == drift_decision_spec(
            target_position_ms as int,
            current_position_ms as int,
            playback_ready,
        ),
{
    if !playback_ready {
        return DriftDecision::Hold;
    }
    let drift = target_position_ms - current_position_ms;
    let absolute: u64 = if drift >= 0 {
        drift as u64
    } else {
        (0i64 - drift) as u64
    };
    if absolute <= SOFT_SYNC_THRESHOLD_MS as u64 {
        DriftDecision::Hold
    } else if absolute >= HARD_SYNC_THRESHOLD_MS as u64 {
        DriftDecision::Seek(target_position_ms)
    } else if drift > 0 {
        DriftDecision::Speed(102)
    } else {
        DriftDecision::Speed(98)
    }
}

/// Port of `shouldSeekDuringActivePlayback`.
pub fn should_seek_during_active_playback(
    position_difference_ms: i64,
    playback_ready: bool,
) -> (r: bool)
    ensures
        r == (playback_ready && position_difference_ms > ACTIVE_PLAYBACK_SYNC_TOLERANCE_MS),
{
    playback_ready && position_difference_ms > ACTIVE_PLAYBACK_SYNC_TOLERANCE_MS
}

/// Extrapolate a paused/playing position between two server timestamps.
pub open spec fn sync_position_at_spec(
    position: int,
    at: Option<i64>,
    playing: bool,
    now: Option<i64>,
) -> int {
    match at {
        Some(a) => if a > 0 && playing {
            match now {
                Some(n) => advance_spec(position, n as int, a as int),
                None => position,
            }
        } else {
            position
        },
        None => position,
    }
}

pub fn position_at(
    position: i64,
    effective_at_server_time: Option<i64>,
    is_playing: bool,
    server_now: Option<i64>,
) -> (r: i64)
    ensures
        r == sync_position_at_spec(
            position as int,
            effective_at_server_time,
            is_playing,
            server_now,
        ),
{
    let effective_at = match effective_at_server_time {
        Some(at) if at > 0 => at,
        _ => return position,
    };
    let server_now = match server_now {
        Some(now) if is_playing => now,
        _ => return position,
    };
    saturating_advance(position, server_now, effective_at)
}

/// Monotonic revision gate.
pub struct RevisionGate {
    pub last: u64,
}

pub open spec fn accept_spec(last: u64, revision: u64) -> (bool, u64) {
    if revision == 0 {
        (true, last)
    } else if revision < last {
        (false, last)
    } else {
        (true, revision)
    }
}

impl RevisionGate {
    pub fn last(&self) -> (r: u64)
        ensures r == self.last,
    {
        self.last
    }

    pub fn reset(&mut self)
        ensures final(self).last == 0,
    {
        self.last = 0;
    }

    /// Force the gate to a known revision (join/reconnect snapshots).
    pub fn set(&mut self, revision: u64)
        ensures final(self).last == revision,
    {
        self.last = revision;
    }

    pub fn accept(&mut self, revision: u64) -> (r: bool)
        ensures
            r == accept_spec(old(self).last, revision).0,
            final(self).last == accept_spec(old(self).last, revision).1,
            final(self).last >= old(self).last,
    {
        if revision == 0 {
            return true;
        }
        if revision < self.last {
            return false;
        }
        self.last = self.last.max(revision);
        true
    }
}

/// Reconnect backoff table: `2 shl min(attempt - 1, 4)` scaled by 1 s.
pub open spec fn backoff_base_ms(attempt: u32) -> int {
    if attempt <= 1 {
        2_000
    } else if attempt == 2 {
        4_000
    } else if attempt == 3 {
        8_000
    } else if attempt == 4 {
        16_000
    } else {
        32_000
    }
}

pub fn backoff_base(attempt: u32) -> (r: u64)
    ensures r == backoff_base_ms(attempt),
{
    let shift = if attempt == 0 {
        0
    } else {
        (attempt - 1).min(4)
    };
    assert(shift <= 4);
    let factor: u64 = match shift {
        0 => 2,
        1 => 4,
        2 => 8,
        3 => 16,
        _ => 32,
    };
    let exponential = INITIAL_RECONNECT_DELAY_MS * factor;
    let capped = exponential.min(MAX_RECONNECT_DELAY_MS);
    assert(capped == exponential);
    capped
}

/// Exponential reconnect delay with up to 20 % jitter. `jitter_percent` is the
/// integer model of `jitter_sample * 100` (the real argument is an `f64` in
/// `[0, 1)`); the delay is the proven integer backbone plus
/// `base * jitter_percent / 500`.
pub fn backoff_delay_ms(attempt: u32, jitter_percent: u32) -> (r: u64)
    requires jitter_percent <= 100,
    ensures
        r >= backoff_base_ms(attempt),
        r == backoff_base_ms(attempt) + (backoff_base_ms(attempt) * jitter_percent as int) / 500,
        r <= backoff_base_ms(attempt) + backoff_base_ms(attempt) / 5,
{
    let capped = backoff_base(attempt);
    assert(capped <= 32_000);
    let jitter = capped * jitter_percent as u64 / 500;
    capped + jitter
}

pub fn should_reconnect(attempts: u32) -> (r: bool)
    ensures r == (attempts < MAX_RECONNECT_ATTEMPTS),
{
    attempts < MAX_RECONNECT_ATTEMPTS
}

pub open spec fn reconnect_spec(attempts: u32) -> bool {
    attempts < MAX_RECONNECT_ATTEMPTS
}

/// `canonicalPlaybackQueue` over id tokens: the current track first, then the
/// upcoming tracks in server order, each id at most once.
pub open spec fn canonical_prefix(current: u32, upcoming: Seq<u32>, i: int) -> Seq<u32>
    decreases i
{
    if i <= 0 {
        seq![current]
    } else {
        let prev = canonical_prefix(current, upcoming, i - 1);
        if prev.contains(upcoming[i - 1]) {
            prev
        } else {
            prev.push(upcoming[i - 1])
        }
    }
}

pub open spec fn canonical_ids(current: u32, upcoming: Seq<u32>) -> Seq<u32> {
    canonical_prefix(current, upcoming, upcoming.len() as int)
}

/// `page.songs.iter().any(|queued| queued.id == song.id)`.
pub fn contains_id(ids: &Vec<u32>, id: u32) -> (r: bool)
    ensures r == ids@.contains(id),
{
    let mut j: usize = 0;
    while j < ids.len()
        invariant
            j <= ids.len(),
            forall|k: int| 0 <= k < j ==> #[trigger] ids@[k] != id,
        decreases ids.len() - j,
    {
        if ids[j] == id {
            return true;
        }
        j = j + 1;
    }
    false
}

pub fn canonical_playback_queue(current: u32, upcoming: &Vec<u32>) -> (r: Vec<u32>)
    ensures
        r@ == canonical_ids(current, upcoming@),
        r@.len() > 0,
        r@[0] == current,
{
    let mut queue = Vec::new();
    queue.push(current);
    let mut i: usize = 0;
    while i < upcoming.len()
        invariant
            i <= upcoming.len(),
            queue@ == canonical_prefix(current, upcoming@, i as int),
            queue@.len() > 0,
            queue@[0] == current,
        decreases upcoming.len() - i,
    {
        let track = upcoming[i];
        if !contains_id(&queue, track) {
            queue.push(track);
        }
        i = i + 1;
    }
    queue
}

// ---------- crates/together/src/codec.rs ----------

/// Kotlin `COMPRESSION_THRESHOLD`.
pub const COMPRESSION_THRESHOLD: usize = 100;

/// Gzip only when strictly smaller. Pure and total.
pub fn should_compress(original_len: usize, compressed_len: usize) -> (r: bool)
    ensures r == (compressed_len < original_len),
{
    compressed_len < original_len
}

pub open spec fn compress_spec(original_len: usize, compressed_len: usize) -> bool {
    compressed_len < original_len
}

/// The envelope's decision: gzip is attempted above the threshold and kept
/// only when it shrinks the payload.
pub open spec fn envelope_compresses(original_len: usize, compressed_len: usize) -> bool {
    original_len > COMPRESSION_THRESHOLD && compress_spec(original_len, compressed_len)
}


/// One second->millisecond->second round trip is the identity for any
/// duration the app can hold, so a room round trip does not drift.
pub proof fn lemma_duration_round_trip(seconds: u32)
    requires seconds > 0,
    ensures seconds_of(ms_of(seconds)) == Some(seconds),
{
}

/// Only a 0-second duration is not preserved: `Some(0)` seconds becomes 0 ms,
/// which decodes as `None`. Every real duration (`> 0`) round-trips.
pub proof fn lemma_zero_second_duration_is_absent()
    ensures seconds_of(ms_of(0)) == None,
{
}

/// The correction never rewinds playback.
pub proof fn lemma_advance_no_rewind(position: i64, now: i64, at: i64)
    ensures advance_spec(position as int, now as int, at as int) >= position as int,
{
}

/// An anchor in the future (or a clock that agrees) leaves the position alone.
pub proof fn lemma_advance_nothing_to_add(position: i64, now: i64, at: i64)
    requires now <= at,
    ensures advance_spec(position as int, now as int, at as int) == position as int,
{
}

/// Drift correction is monotone in the observed server time.
pub proof fn lemma_advance_monotone(position: i64, now1: i64, now2: i64, at: i64)
    requires now1 <= now2,
    ensures advance_spec(position as int, now1 as int, at as int) <= advance_spec(
        position as int,
        now2 as int,
        at as int,
    ),
{
}

/// The i64 top is sticky.
pub proof fn lemma_advance_saturates(now: i64, at: i64)
    ensures advance_spec(i64::MAX as int, now as int, at as int) == i64::MAX as int,
{
}

/// A stale sample (older than 60 s) is rejected.
pub proof fn lemma_stale_sample_rejected(
    client_time: i64,
    server_receive_time: i64,
    server_send_time: i64,
    received_at: i64,
)
    requires received_at - client_time > MAX_SAMPLE_AGE_MS,
    ensures !sample_accepted(
        client_time,
        server_receive_time,
        server_send_time,
        received_at,
    ),
{
}

/// The age bound is inclusive: exactly 60 s is accepted (when the server side
/// is well formed).
pub proof fn lemma_age_bound_inclusive(
    client_time: i64,
    server_receive_time: i64,
    server_send_time: i64,
    received_at: i64,
)
    requires
        client_time > 0,
        received_at - client_time == MAX_SAMPLE_AGE_MS,
        server_receive_time > 0,
        server_send_time >= server_receive_time,
    ensures sample_accepted(client_time, server_receive_time, server_send_time, received_at),
{
}

/// An accepted sample's network RTT is at most the 60 s age bound.
pub proof fn lemma_network_rtt_bounded(
    client_time: i64,
    server_receive_time: i64,
    server_send_time: i64,
    received_at: i64,
)
    requires sample_accepted(client_time, server_receive_time, server_send_time, received_at),
    ensures
        0 <= network_round_trip(
            received_at as int - client_time as int,
            server_send_time as int - server_receive_time as int,
        ) <= MAX_SAMPLE_AGE_MS,
{
}

/// The best RTT is the running minimum.
pub proof fn lemma_best_after_is_min(best: u64, rtt: u64)
    ensures
        best_after(best, rtt) <= best,
        best_after(best, rtt) <= rtt,
{
}

/// A first accepted sample replaces `u64::MAX`, so `best_round_trip_ms()` stops
/// returning `None`.
pub proof fn lemma_first_sample_records_best(rtt: u64)
    requires rtt < u64::MAX,
    ensures best_after(u64::MAX, rtt) == rtt, best_after(u64::MAX, rtt) != u64::MAX,
{
}

/// Revision 0 is the unversioned escape hatch: accepted, state untouched.
pub proof fn lemma_accept_unversioned(last: u64)
    ensures accept_spec(last, 0) == (true, last),
{
    assert((0u64) == 0);
}

/// A stale revision is dropped and cannot move the gate backwards.
pub proof fn lemma_accept_stale_rejected(last: u64, revision: u64)
    requires revision > 0, revision < last,
    ensures accept_spec(last, revision) == (false, last),
{
    assert(!(revision == 0));
    assert(revision < last);
}

/// A fresh revision is accepted and becomes the new high-water mark.
pub proof fn lemma_accept_advances(last: u64, revision: u64)
    requires revision > 0, revision >= last,
    ensures accept_spec(last, revision) == (true, revision),
{
    assert(!(revision == 0));
    assert(!(revision < last));
}

/// Re-presenting the revision the gate already holds is accepted (idempotent).
pub proof fn lemma_accept_idempotent(last: u64)
    requires last > 0,
    ensures accept_spec(last, last) == (true, last),
{
    assert(!(last == 0));
    assert(!(last < last));
}

/// After accepting `revision`, every older revision is rejected until `set` or
/// `reset`: the gate never replays an event.
pub proof fn lemma_no_replay(last: u64, revision: u64, stale: u64)
    requires revision > 0, revision >= last, stale > 0, stale < revision,
    ensures accept_spec(accept_spec(last, revision).1, stale).0 == false,
{
    assert(accept_spec(last, revision).1 == revision);
}

/// `set` reopens exactly the revision it was given.
pub proof fn lemma_set_then_accept(revision: u64)
    ensures accept_spec(revision, revision) == (true, revision),
{
    if revision == 0 {
    } else {
        assert(!(revision < revision));
    }
}

/// The published table: 2 s after the first attempt, doubling to 32 s.
pub proof fn lemma_backoff_table()
    ensures
        backoff_base_ms(0) == 2_000,
        backoff_base_ms(1) == 2_000,
        backoff_base_ms(2) == 4_000,
        backoff_base_ms(3) == 8_000,
        backoff_base_ms(4) == 16_000,
        backoff_base_ms(5) == 32_000,
        backoff_base_ms(6) == 32_000,
        backoff_base_ms(1_000_000) == 32_000,
{
    assert(backoff_base_ms(0) == 2_000) by (compute);
    assert(backoff_base_ms(1) == 2_000) by (compute);
    assert(backoff_base_ms(2) == 4_000) by (compute);
    assert(backoff_base_ms(3) == 8_000) by (compute);
    assert(backoff_base_ms(4) == 16_000) by (compute);
    assert(backoff_base_ms(5) == 32_000) by (compute);
    assert(backoff_base_ms(6) == 32_000) by (compute);
    assert(backoff_base_ms(1_000_000) == 32_000) by (compute);
}

/// The nominal 120 s cap never binds: the exponent cap holds the maximum at
/// 32 s, so `MAX_RECONNECT_DELAY_MS` is dead code.
pub proof fn lemma_backoff_far_below_nominal_cap(attempt: u32)
    ensures backoff_base_ms(attempt) <= 32_000 < MAX_RECONNECT_DELAY_MS as int,
{
}

/// Backoff never shrinks as attempts grow.
pub proof fn lemma_backoff_monotone(a: u32, b: u32)
    requires a <= b,
    ensures backoff_base_ms(a) <= backoff_base_ms(b),
{
}

/// Jitter stays under 20 %, so the delay can never exceed 38.4 s.
pub proof fn lemma_jitter_bounded(attempt: u32, jitter_percent: u32)
    requires jitter_percent <= 100,
    ensures backoff_base_ms(attempt) + (backoff_base_ms(attempt) * jitter_percent as int) / 500
        <= 38_400,
{
    lemma_backoff_far_below_nominal_cap(attempt);
}

/// The last allowed attempt is 14; 15 gives up.
pub proof fn lemma_reconnect_boundary()
    ensures
        reconnect_spec(14),
        !reconnect_spec(15),
        !reconnect_spec(u32::MAX),
{
}

/// Giving up is monotone: once false it stays false.
pub proof fn lemma_reconnect_monotone(a: u32, b: u32)
    requires a <= b,
    ensures reconnect_spec(b) ==> reconnect_spec(a),
{
}

/// Compression is never even attempted at or below 100 bytes.
pub proof fn lemma_small_payload_never_compressed(original_len: usize, compressed_len: usize)
    requires original_len <= COMPRESSION_THRESHOLD,
    ensures !envelope_compresses(original_len, compressed_len),
{
}

/// The wire payload never grows: the compressed form is only used when
/// strictly smaller.
pub proof fn lemma_compression_never_inflates(original_len: usize, compressed_len: usize)
    requires envelope_compresses(original_len, compressed_len),
    ensures compressed_len < original_len,
{
}

/// Equal sizes stay uncompressed (gzip overhead alone is a loss).
pub proof fn lemma_equal_size_not_compressed(len: usize)
    ensures !compress_spec(len, len),
{
}

} // verus!
