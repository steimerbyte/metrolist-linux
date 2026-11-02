use vstd::prelude::*;

verus! {

pub enum PlaybackStatus { Idle, Loading, Buffering, Playing, Paused, Stopped, Ended, Error }
pub enum RepeatMode { Off, All, One }
pub enum TransportStatus { Playing, Paused, Stopped }
pub enum TransportLoop { None, Track, Playlist }

pub open spec fn status_map(s: PlaybackStatus) -> TransportStatus {
    match s {
        PlaybackStatus::Playing => TransportStatus::Playing,
        PlaybackStatus::Buffering => TransportStatus::Playing,
        PlaybackStatus::Paused => TransportStatus::Paused,
        _ => TransportStatus::Stopped,
    }
}

pub fn transport_status(status: PlaybackStatus) -> (r: TransportStatus)
    ensures r == status_map(status)
{
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

pub open spec fn loop_map(r: RepeatMode) -> TransportLoop {
    match r {
        RepeatMode::Off => TransportLoop::None,
        RepeatMode::All => TransportLoop::Playlist,
        RepeatMode::One => TransportLoop::Track,
    }
}

pub open spec fn loop_unmap(l: TransportLoop) -> RepeatMode {
    match l {
        TransportLoop::None => RepeatMode::Off,
        TransportLoop::Playlist => RepeatMode::All,
        TransportLoop::Track => RepeatMode::One,
    }
}

pub fn transport_loop(repeat: RepeatMode) -> (r: TransportLoop)
    ensures r == loop_map(repeat)
{
    match repeat {
        RepeatMode::Off => TransportLoop::None,
        RepeatMode::All => TransportLoop::Playlist,
        RepeatMode::One => TransportLoop::Track,
    }
}

pub fn repeat_from_loop(loop_status: TransportLoop) -> (r: RepeatMode)
    ensures r == loop_unmap(loop_status)
{
    match loop_status {
        TransportLoop::None => RepeatMode::Off,
        TransportLoop::Playlist => RepeatMode::All,
        TransportLoop::Track => RepeatMode::One,
    }
}

pub proof fn lemma_loop_roundtrip(r: RepeatMode)
    ensures loop_unmap(loop_map(r)) == r,
{}

pub proof fn lemma_loop_roundtrip_inv(l: TransportLoop)
    ensures loop_map(loop_unmap(l)) == l,
{}

/// `i64::try_from(ms).unwrap_or(i64::MAX).saturating_mul(1000)`
pub open spec fn ms_to_us_spec(ms: u64) -> int {
    let clamped = if (ms as int) > (i64::MAX as int) { i64::MAX as int } else { ms as int };
    if clamped > (i64::MAX as int) / 1000 { i64::MAX as int } else { clamped * 1000 }
}

pub fn ms_to_us(ms: u64) -> (r: i64)
    ensures r as int == ms_to_us_spec(ms), r >= 0
{
    let clamped: i64 = if ms > i64::MAX as u64 { i64::MAX } else { ms as i64 };
    if clamped > i64::MAX / 1000 {
        i64::MAX
    } else {
        clamped * 1000
    }
}

/// Rust `/` on negative dividends truncates toward zero.
pub open spec fn trunc_ms(us: int) -> int {
    if us >= 0 { us / 1000 } else { -((-us) / 1000) }
}

pub fn us_to_ms(us: i64) -> (r: i64)
    ensures r as int == trunc_ms(us as int)
{
    us / 1000
}

/// `(us.max(0) / 1000) as u64`: negative offsets clamp to 0 (not truncation).
pub open spec fn position_ms(us: int) -> int {
    if us >= 0 { us / 1000 } else { 0 }
}

pub fn position_ms_from_us(us: i64) -> (r: u64)
    ensures r as int == position_ms(us as int), r as int >= 0
{
    (us.max(0) / 1000) as u64
}

pub struct Snapshot {
    pub current: Option<u32>,
    pub queue_index: Option<usize>,
    pub queue_len: usize,
    pub repeat: RepeatMode,
}

pub open spec fn go_next(s: Snapshot) -> bool {
    match s.current {
        None => false,
        Some(_) => match (s.queue_index, s.repeat) {
            (None, _) => false,
            (Some(_), RepeatMode::All) => true,
            (Some(_), RepeatMode::One) => true,
            (Some(index), RepeatMode::Off) => match index.checked_add(1) {
            Some(next) => next < s.queue_len,
            None => false,
        },
        },
    }
}

pub fn can_go_next(s: Snapshot) -> (r: bool)
    ensures r == go_next(s)
{
    if s.current.is_none() {
        return false;
    }
    match (s.queue_index, s.repeat) {
        (None, _) => false,
        (Some(_), RepeatMode::All | RepeatMode::One) => true,
        (Some(index), RepeatMode::Off) => match index.checked_add(1) {
            Some(next) => next < s.queue_len,
            None => false,
        },
    }
}

pub open spec fn go_prev(s: Snapshot) -> bool {
    match s.current {
        None => false,
        Some(_) => match (s.queue_index, s.repeat) {
            (None, _) => false,
            (Some(_), RepeatMode::All) => true,
            (Some(_), RepeatMode::One) => true,
            (Some(index), RepeatMode::Off) => index > 0,
        },
    }
}

pub fn can_go_previous(s: Snapshot) -> (r: bool)
    ensures r == go_prev(s)
{
    if s.current.is_none() {
        return false;
    }
    match (s.queue_index, s.repeat) {
        (None, _) => false,
        (Some(_), RepeatMode::All | RepeatMode::One) => true,
        (Some(index), RepeatMode::Off) => index > 0,
    }
}

/// `checked_add(1)` is defensive only: a wrapped index is never a real
/// successor, and for `index < usize::MAX` the result is plain `index + 1`.
pub proof fn lemma_next_checked_add(s: Snapshot, index: usize)
    ensures
        (index as int) + 1 < (s.queue_len as int)
            <==> (index != usize::MAX && index + 1 < s.queue_len),
{
}

pub proof fn lemma_next_wraps(s: Snapshot)
    requires s.current.is_some(), s.queue_index.is_some(), s.repeat != RepeatMode::Off,
    ensures go_next(s), go_prev(s),
{}

pub proof fn lemma_exact_us(ms: u64)
    requires (ms as int) <= (i64::MAX as int) / 1000,
    ensures ms_to_us_spec(ms) == (ms as int) * 1000,
{}

} // verus!
