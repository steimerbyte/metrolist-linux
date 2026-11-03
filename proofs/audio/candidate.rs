

use vstd::prelude::*;

verus! {

/// `PlaybackController::set_sleep_timer`: `(minutes as u64 * 60).min(24*60*60)`.
/// `Instant::now() + Duration` is the unverifiable part; the seconds value is this.

pub open spec fn sleep_seconds_spec(minutes: u32) -> int {
    let secs = (minutes as int) * 60;
    if secs < 86_400 { secs } else { 86_400 }
}

pub fn sleep_seconds(minutes: u32) -> (r: u64)
    ensures r as int == sleep_seconds_spec(minutes),
{
    let seconds = (minutes as u64 * 60).min(24 * 60 * 60);
    seconds
}

pub open spec fn next_id_spec(id: u64) -> int {
    ((id as int) + 1) % 0x1_0000_0000_0000_0000
}

pub fn next_request_id(id: u64) -> (r: u64)
    ensures r as int == next_id_spec(id),
{
    id.wrapping_add(1)
}

/// Consecutive request ids never collide, so one outstanding reply cannot be
/// mistaken for another's.

pub open spec fn window_contains_spec(now_minutes: u32, start_minutes: u32, end_minutes: u32) -> bool {
    let now = (now_minutes as int) % 1440;
    let start = (start_minutes as int) % 1440;
    let end = (end_minutes as int) % 1440;
    if start == end {
        true
    } else if start < end {
        start <= now && now < end
    } else {
        start <= now || now < end
    }
}

pub fn sleep_window_contains(now_minutes: u32, start_minutes: u32, end_minutes: u32) -> (r: bool)
    ensures r == window_contains_spec(now_minutes, start_minutes, end_minutes),
{
    const DAY: u32 = 24 * 60;
    let now = now_minutes % DAY;
    let start = start_minutes % DAY;
    let end = end_minutes % DAY;
    if start == end {
        true
    } else if start < end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

/// `start == end` means the whole day, for every `now`.

pub open spec fn clear_stream_mute_spec(allow_unmute: bool, intended_muted: bool) -> bool {
    allow_unmute && !intended_muted
}

pub fn should_clear_stream_mute(allow_unmute: bool, intended_muted: bool) -> (r: bool)
    ensures r == clear_stream_mute_spec(allow_unmute, intended_muted),
{
    allow_unmute && !intended_muted
}

pub proof fn lemma_sleep_clamped_to_a_day(minutes: u32)
    ensures (sleep_seconds_spec(minutes) as int) <= 86_400,
{
}

pub proof fn lemma_sleep_exact_within_a_day(minutes: u32)
    requires (minutes as int) <= 1440,
    ensures sleep_seconds_spec(minutes) == (minutes as int) * 60,
{
}

pub proof fn lemma_sleep_monotone(a: u32, b: u32)
    requires a <= b,
    ensures sleep_seconds_spec(a) <= sleep_seconds_spec(b),
{
}

/// `MpvEngine::request`: `self.next_id = self.next_id.wrapping_add(1)`, and the
/// new id is the one sent. Modelled as `(id + 1) mod 2^64`.

pub proof fn lemma_ids_never_repeat(id: u64)
    ensures next_id_spec(id) != id as int,
{
}

pub proof fn lemma_id_increments_below_max(id: u64)
    requires (id as int) < 0xFFFF_FFFF_FFFF_FFFF,
    ensures next_id_spec(id) == (id as int) + 1,
{
}

/// `crates/audio/src/controller.rs::sleep_window_contains`: the daily
/// sleep-timer window, in minutes since midnight. `start == end` is the whole
/// day; `start > end` wraps overnight (`22:00..06:00`). The exec body is the
/// real function verbatim.

pub proof fn lemma_full_day_always_contains(now_minutes: u32, start_minutes: u32)
    ensures window_contains_spec(now_minutes, start_minutes, start_minutes),
{
}

/// The controller's own unit-test cases: 22:30 and 05:00 are inside
/// 22:00..06:00 but 12:00 is not; 09:00 is inside 09:00..17:00 while 17:00 is
/// excluded; 23:00..23:00 is the whole day.

pub proof fn lemma_ui_windows()
{
    assert(window_contains_spec(1350u32, 1320u32, 360u32)); // 22:30
    assert(window_contains_spec(300u32, 1320u32, 360u32)); // 05:00
    assert(!window_contains_spec(720u32, 1320u32, 360u32)); // 12:00
    assert(window_contains_spec(540u32, 540u32, 1020u32)); // 09:00
    assert(!window_contains_spec(1020u32, 540u32, 1020u32)); // 17:00
    assert(window_contains_spec(0u32, 1380u32, 1380u32)); // 23:00..23:00
}

/// Inside an ordered (non-wrapping) window, a later minute is still inside.

pub proof fn lemma_ordered_step(a: u32, b: u32, start_minutes: u32, end_minutes: u32)
    requires
        (start_minutes as int) % 1440 < (end_minutes as int) % 1440,
        window_contains_spec(a, start_minutes, end_minutes),
        (a as int) % 1440 <= (b as int) % 1440,
        (b as int) % 1440 < (end_minutes as int) % 1440,
    ensures window_contains_spec(b, start_minutes, end_minutes),
{
}

// ===== should_clear_stream_mute =====
// engine.rs stream-level mute policy: test/probe engines never clear the
// server's mute, and a stream the user muted is never unmuted.

pub proof fn lemma_stream_mute_table()
    ensures
        clear_stream_mute_spec(true, false),
        !clear_stream_mute_spec(true, true),
        !clear_stream_mute_spec(false, false),
        !clear_stream_mute_spec(false, true),
{
}

pub proof fn lemma_muted_stays_muted(allow_unmute: bool, intended_muted: bool)
    ensures intended_muted || !allow_unmute ==> !clear_stream_mute_spec(allow_unmute, intended_muted),
{
}

} // verus!
