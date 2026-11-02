use vstd::prelude::*;

verus! {

// ---------- duration formatting (arithmetic core) ----------

pub open spec fn duration_h(total: u32) -> int { total as int / 3600 }
pub open spec fn duration_m(total: u32) -> int { total as int % 3600 / 60 }
pub open spec fn duration_s(total: u32) -> int { total as int % 60 }

pub fn duration_parts(total: u32) -> (r: (u32, u32, u32))
    ensures
        r.0 as int == duration_h(total),
        r.1 as int == duration_m(total),
        r.2 as int == duration_s(total),
        r.1 < 60,
        r.2 < 60,
        r.0 as int * 3600 + r.1 as int * 60 + r.2 as int == total as int,
        (r.0 == 0) == ((total as int) < 3600),
{
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
    }
    (hours, minutes, seconds)
}

/// `format_position` divides by 1000 and saturates at `u32::MAX` before casting.
pub fn position_seconds(position_ms: u64) -> (r: u32)
    ensures
        r as int == if position_ms as int / 1000 <= u32::MAX as int {
            position_ms as int / 1000
        } else {
            u32::MAX as int
        },
{
    let seconds = position_ms / 1000;
    let clamped = if seconds <= u32::MAX as u64 { seconds } else { u32::MAX as u64 };
    clamped as u32
}

pub fn millis_to_seconds(millis: u64) -> (r: u64)
    ensures r as int == millis as int / 1000,
{
    millis / 1000
}

proof fn lemma_millis_floor(millis: u64)
    ensures millis / 1000 <= millis,
{
}

// ---------- thumbnail size buckets ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThumbBucket { Default, Mq, Hq, Sd, MaxRes }

pub open spec fn bucket_of(size: u32) -> ThumbBucket {
    if size <= 120 {
        ThumbBucket::Default
    } else if size <= 320 {
        ThumbBucket::Mq
    } else if size <= 480 {
        ThumbBucket::Hq
    } else if size <= 640 {
        ThumbBucket::Sd
    } else {
        ThumbBucket::MaxRes
    }
}

pub open spec fn bucket_ord(b: ThumbBucket) -> int {
    match b {
        ThumbBucket::Default => 0,
        ThumbBucket::Mq => 1,
        ThumbBucket::Hq => 2,
        ThumbBucket::Sd => 3,
        ThumbBucket::MaxRes => 4,
    }
}

pub open spec fn bucket_file(b: ThumbBucket) -> &'static str {
    match b {
        ThumbBucket::Default => "default.jpg",
        ThumbBucket::Mq => "mqdefault.jpg",
        ThumbBucket::Hq => "hqdefault.jpg",
        ThumbBucket::Sd => "sddefault.jpg",
        ThumbBucket::MaxRes => "maxresdefault.jpg",
    }
}

pub fn thumb_bucket(size: u32) -> (r: ThumbBucket)
    ensures r == bucket_of(size),
{
    match size {
        0..=120 => ThumbBucket::Default,
        121..=320 => ThumbBucket::Mq,
        321..=480 => ThumbBucket::Hq,
        481..=640 => ThumbBucket::Sd,
        _ => ThumbBucket::MaxRes,
    }
}

/// Server-side buckets only grow with the requested size: a larger request
/// never picks a smaller image.
proof fn lemma_bucket_monotone(s1: u32, s2: u32)
    requires s1 <= s2,
    ensures bucket_ord(bucket_of(s1)) <= bucket_ord(bucket_of(s2)),
{
}

// ---------- video id shape ----------

pub open spec fn valid_id_byte(b: u8) -> bool {
    (b'0' <= b && b <= b'9')
        || (b'a' <= b && b <= b'z')
        || (b'A' <= b && b <= b'Z')
        || b == b'-'
        || b == b'_'
}

pub open spec fn id_shape(s: Seq<u8>) -> bool {
    s.len() == 11 && forall|i: int| 0 <= i < s.len() ==> #[trigger] valid_id_byte(s[i])
}

fn exec_valid_id_byte(b: u8) -> (r: bool)
    ensures r == valid_id_byte(b),
{
    (b'0' <= b && b <= b'9')
        || (b'a' <= b && b <= b'z')
        || (b'A' <= b && b <= b'Z')
        || b == b'-'
        || b == b'_'
}

pub fn is_video_id_bytes(bytes: &[u8]) -> (r: bool)
    ensures r == id_shape(bytes@),
{
    if bytes.len() != 11 {
        return false;
    }
    let mut i = 0;
    while i < bytes.len()
        decreases bytes@.len() - i,
    {
        if !exec_valid_id_byte(bytes[i]) {
            return false;
        }
        i += 1;
    }
    true
}

} // verus!
