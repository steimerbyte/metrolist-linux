use vstd::prelude::*;

verus! {

// `saturating_sub(300)` on i64: only `expires < i64::MIN + 300` can saturate.
pub open spec fn expiry_threshold(expires: i64) -> int {
    if (expires as int) < ((i64::MIN as int) + 300) {
        i64::MIN as int
    } else {
        expires as int - 300
    }
}

pub open spec fn expired(expires_at: Option<i64>, now: i64) -> bool {
    match expires_at {
        None => false,
        Some(expires) => now as int >= expiry_threshold(expires),
    }
}

/// `StreamInfo::is_expired`: `None` never expires; `Some(e)` expires at
/// `e - 300` inclusive (the 300 s safety margin).
pub fn is_expired(expires_at: Option<i64>, now_epoch_secs: i64) -> (r: bool)
    ensures r == expired(expires_at, now_epoch_secs),
{
    match expires_at {
        None => false,
        Some(expires) => {
            let threshold = if expires < i64::MIN + 300 {
                i64::MIN
            } else {
                expires - 300
            };
            now_epoch_secs >= threshold
        }
    }
}

proof fn lemma_none_never_expires(now: i64)
    ensures !expired(None, now),
{
}

proof fn lemma_margin_is_inclusive(expires: i64)
    requires (expires as int) >= ((i64::MIN as int) + 300),
    ensures expired(Some(expires), (expires - 300) as i64),
{
}

proof fn lemma_expiry_monotone(expires_at: Option<i64>, t1: i64, t2: i64)
    requires t1 <= t2,
    ensures expired(expires_at, t1) ==> expired(expires_at, t2),
{
}

} // verus!
