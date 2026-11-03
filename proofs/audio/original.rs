

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
} // verus!