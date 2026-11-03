use vstd::prelude::*;

verus! {

// ---------- crates/ui/src/pure.rs ----------

// ===== session_file_path =====
//
// `std::path::Path::join` is modelled on the bytes of the Unix path: an empty
// directory yields the name, a trailing `/` is not doubled, otherwise one `/`
// is inserted. See SOURCE.md.

pub open spec fn is_sep(b: u8) -> bool {
    b == 47u8
}

pub open spec fn session_name() -> Seq<u8> {
    // "session-cookies.txt"
    seq![
        115u8, 101u8, 115u8, 115u8, 105u8, 111u8, 110u8, 45u8, 99u8,
        111u8, 111u8, 107u8, 105u8, 101u8, 115u8, 46u8, 116u8, 120u8, 116u8,
    ]
}

pub open spec fn join_spec(dir: Seq<u8>, name: Seq<u8>) -> Seq<u8> {
    if dir.len() == 0 {
        name
    } else if is_sep(dir[dir.len() - 1]) {
        dir + name
    } else {
        dir + seq![47u8] + name
    }
}

pub fn session_file_path(data_dir: &Vec<u8>) -> (r: Vec<u8>)
    ensures
        r@ == join_spec(data_dir@, session_name()),
{
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < data_dir.len()
        invariant
            i <= data_dir@.len(),
            out@.len() == i as int,
            forall|j: int| 0 <= j < (i as int) ==> out@[j] == data_dir@[j],
        decreases data_dir@.len() - i,
    {
        let b = data_dir[i];
        out.push(b);
        i = i + 1;
    }
    assert(out@ =~= data_dir@);
    if i > 0 && data_dir[i - 1] != 47u8 {
        out.push(47u8);
    }
    out.push(115u8);
    out.push(101u8);
    out.push(115u8);
    out.push(115u8);
    out.push(105u8);
    out.push(111u8);
    out.push(110u8);
    out.push(45u8);
    out.push(99u8);
    out.push(111u8);
    out.push(111u8);
    out.push(107u8);
    out.push(105u8);
    out.push(101u8);
    out.push(115u8);
    out.push(46u8);
    out.push(116u8);
    out.push(120u8);
    out.push(116u8);
    out
}

// ===== queue_index_for_id =====

pub fn queue_index_for_id(songs: &Vec<u32>, id: u32) -> (r: Option<usize>)
    ensures
        match r {
            Some(i) => (i as int) < songs@.len() && songs@[i as int] == id
                && (forall|j: int| 0 <= j < (i as int) ==> songs@[j] != id),
            None => forall|j: int| 0 <= j < songs@.len() ==> songs@[j] != id,
        },
{
    let mut i = 0;
    while i < songs.len()
        invariant
            i <= songs@.len(),
            forall|j: int| 0 <= j < (i as int) ==> songs@[j] != id,
        decreases songs@.len() - i,
    {
        if songs[i] == id {
            return Some(i);
        }
        i = i + 1;
    }
    None
}

// ===== playlist_move_target =====

pub fn playlist_move_target(index: usize, len: usize, delta: i8) -> (r: Option<usize>)
    ensures
        match r {
            Some(t) => (t as int) < len
                && (t as int) != (index as int)
                && (delta > 0 ==> (t as int) > (index as int))
                && (delta < 0 ==> (t as int) < (index as int))
                && (t as int) == (index as int) + (delta as int),
            None => true,
        },
        index >= len || delta == 0 ==> r.is_none(),
{
    if index >= len || delta == 0 {
        return None;
    }
    let target: usize = if delta < 0 {
        let step = (0i16 - (delta as i16)) as usize;
        if index < step {
            return None;
        } else {
            index - step
        }
    } else {
        let step = delta as usize;
        if index > usize::MAX - step {
            return None;
        } else {
            index + step
        }
    };
    if target < len {
        Some(target)
    } else {
        None
    }
}

// ===== group_starts =====

pub open spec fn is_group_start(keys: Seq<u8>, i: int) -> bool {
    i == 0 || keys[i - 1] != keys[i]
}

pub open spec fn starts_prefix(keys: Seq<u8>, n: int) -> (r: Seq<usize>)
    decreases n,
{
    if n <= 0 {
        seq![]
    } else {
        starts_prefix(keys, n - 1)
            + (if is_group_start(keys, n - 1) {
            seq![(n - 1) as usize]
        } else {
            seq![]
        })
    }
}

pub fn group_starts(keys: &Vec<u8>) -> (r: Vec<usize>)
    ensures
        r@ == starts_prefix(keys@, keys@.len() as int),
{
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < keys.len()
        invariant
            i <= keys@.len(),
            starts@ == starts_prefix(keys@, i as int),
            forall|j: int| 0 <= j < starts@.len() ==> #[trigger] starts@[j] < i,
            forall|j: int|
                0 <= j && j + 1 < starts@.len() ==> #[trigger] starts@[j] < starts@[j + 1],
        decreases keys@.len() - i,
    {
        if i == 0 || keys[i - 1] != keys[i] {
            starts.push(i);
        }
        i = i + 1;
    }
    starts
}

// ===== expiry_hint =====

pub enum ExpiryHint {
    None,
    Expired,
    UnderDay,
    OneDay,
    Days(u64),
}

pub open spec fn expiry_hint_spec(expires_at: Option<i64>, now_unix: i64) -> ExpiryHint {
    match expires_at {
        None => ExpiryHint::None,
        Some(e) => {
            if e <= now_unix {
                ExpiryHint::Expired
            } else if e - now_unix < 86400 {
                ExpiryHint::UnderDay
            } else if e - now_unix < 172800 {
                ExpiryHint::OneDay
            } else {
                ExpiryHint::Days(((e - now_unix) / 86400) as u64)
            }
        },
    }
}

pub fn expiry_hint(expires_at: Option<i64>, now_unix: i64) -> (r: ExpiryHint)
    requires
        now_unix >= 0,
    ensures
        r == expiry_hint_spec(expires_at, now_unix),
{
    let e = match expires_at {
        None => return ExpiryHint::None,
        Some(v) => v,
    };
    if e <= now_unix {
        return ExpiryHint::Expired;
    }
    let remaining = e - now_unix;
    let days = (remaining / 86400) as u64;
    if days == 0 {
        ExpiryHint::UnderDay
    } else if days == 1 {
        ExpiryHint::OneDay
    } else {
        ExpiryHint::Days(days)
    }
}

// ===== radio_should_extend =====

pub open spec fn should_extend_spec(position: Option<usize>, len: usize) -> bool {
    match position {
        None => false,
        Some(p) => {
            len > 0 && ((p as int) > (usize::MAX as int) - 5 || (p as int) + 5 >= (len as int))
        },
    }
}

pub fn radio_should_extend(position: Option<usize>, len: usize) -> (r: bool)
    ensures
        r == should_extend_spec(position, len),
{
    match position {
        None => false,
        Some(p) => {
            if len > 0 {
                if p > usize::MAX - 5 {
                    true
                } else {
                    p + 5 >= len
                }
            } else {
                false
            }
        },
    }
}



// ===== sleep_seconds =====

pub open spec fn sleep_seconds_spec(minutes: u32) -> int {
    if (minutes as int) * 60 > 86400 {
        86400
    } else {
        (minutes as int) * 60
    }
}

pub fn sleep_seconds(minutes: u32) -> (r: u64)
    ensures
        r as int == sleep_seconds_spec(minutes),
        r <= 86400,
{
    let m = minutes as u64;
    let product = if m > u64::MAX / 60 {
        u64::MAX
    } else {
        m * 60
    };
    if product > 86400 {
        86400
    } else {
        product
    }
}



// ===== seek_relative =====

pub open spec fn seek_raw(position_ms: u64, offset_ms: i64) -> int {
    if offset_ms < 0 {
        if (position_ms as int) < -(offset_ms as int) {
            0
        } else {
            (position_ms as int) + (offset_ms as int)
        }
    } else if (position_ms as int) + (offset_ms as int) > (u64::MAX as int) {
        u64::MAX as int
    } else {
        (position_ms as int) + (offset_ms as int)
    }
}

pub open spec fn seek_relative_spec(position_ms: u64, duration_ms: u64, offset_ms: i64) -> int {
    if duration_ms == 0 {
        seek_raw(position_ms, offset_ms)
    } else if seek_raw(position_ms, offset_ms) > (duration_ms as int) {
        duration_ms as int
    } else {
        seek_raw(position_ms, offset_ms)
    }
}

pub fn seek_relative(position_ms: u64, duration_ms: u64, offset_ms: i64) -> (r: u64)
    ensures
        r as int == seek_relative_spec(position_ms, duration_ms, offset_ms),
        duration_ms > 0 ==> r <= duration_ms,
{
    let target: u64 = if offset_ms < 0 {
        let step = (0i128 - (offset_ms as i128)) as u64;
        if position_ms < step {
            0u64
        } else {
            position_ms - step
        }
    } else {
        let off = offset_ms as u64;
        if position_ms > u64::MAX - off {
            u64::MAX
        } else {
            position_ms + off
        }
    };
    if duration_ms == 0 {
        target
    } else if target > duration_ms {
        duration_ms
    } else {
        target
    }
}



// ===== progress_percent / storage_percent =====

pub open spec fn percent_spec(bytes: u64, total: u64) -> int {
    if total == 0 {
        0
    } else if (bytes as int) * 100 / (total as int) > 100 {
        100
    } else {
        (bytes as int) * 100 / (total as int)
    }
}

pub fn progress_percent(bytes: u64, total: u64) -> (r: u8)
    ensures
        r as int == percent_spec(bytes, total),
        r <= 100,
{
    if total == 0 {
        0
    } else {
        let b = bytes as u128;
        let t = total as u128;
        let raw = b * 100 / t;
        if raw > 100u128 {
            100u8
        } else {
            raw as u8
        }
    }
}



pub fn storage_percent(used: u64, limit: Option<u64>) -> (r: Option<u8>)
    ensures
        limit.is_none() ==> r.is_none(),
        limit.is_some() ==> r.is_some(),
        limit == Some(0u64) ==> r == Some(100u8),
        match r {
            Some(p) => p <= 100,
            None => true,
        },
        match (limit, r) {
            (Some(l), Some(p)) => l > 0 ==> (p as int) == percent_spec(used, l),
            _ => true,
        },
{
    match limit {
        None => None,
        Some(l) => {
            if l == 0 {
                Some(100u8)
            } else {
                Some(progress_percent(used, l))
            }
        },
    }
}

// ===== back_route =====
//
// `Route` is modelled as an opaque `u32` token; the stack is a `Vec<u32>`.

pub fn back_route(stack: &mut Vec<u32>) -> (r: Option<u32>)
    ensures
        final(stack)@ == old(stack)@.take(
            (old(stack)@.len() - (if old(stack)@.len() > 0 {
                1int
            } else {
                0int
            })) as int,
        ),
        old(stack)@.len() == 0 ==> r.is_none(),
        old(stack)@.len() > 0 ==> r == Some(old(stack)@[old(stack)@.len() as int - 1]),
{
    let n = stack.len();
    if n == 0 {
        None
    } else {
        let last = stack[n - 1];
        stack.truncate(n - 1);
        Some(last)
    }
}

pub proof fn lemma_back_route_is_a_pop(stack: Seq<u32>, x: u32)
    ensures
        stack.push(x).len() > 0,
        stack.push(x)[stack.push(x).len() as int - 1] == x,
        stack.push(x).take(stack.push(x).len() - 1) == stack,
{
}

// ===== seed_color_value =====

pub fn seed_color_value(value: u32) -> (r: u32)
    ensures
        r == (value & 0x00FF_FFFFu32),
        r <= 0x00FF_FFFFu32,
        (r & 0xFF00_0000u32) == 0,
{
    assert((value & 0x00FF_FFFFu32) <= 0x00FF_FFFFu32) by (bit_vector);
    assert(((value & 0x00FF_FFFFu32) & 0xFF00_0000u32) == 0u32) by (bit_vector);
    value & 0x00FF_FFFFu32
}


// ===== download_badge / download_icon / download_click =====

pub enum DownloadState {
    Queued,
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

pub enum DownloadBadge {
    None,
    Queued,
    Downloading(u8),
    Completed,
    Failed,
    Cancelled,
}

pub enum DownloadClick {
    Download,
    Cancel,
    Retry,
    None,
}

pub open spec fn badge_spec(state: Option<DownloadState>, percent: u8) -> DownloadBadge {
    match state {
        None => DownloadBadge::None,
        Some(s) => match s {
            DownloadState::Queued => DownloadBadge::Queued,
            DownloadState::Downloading => DownloadBadge::Downloading(percent),
            DownloadState::Completed => DownloadBadge::Completed,
            DownloadState::Failed => DownloadBadge::Failed,
            DownloadState::Cancelled => DownloadBadge::Cancelled,
        },
    }
}

pub fn download_badge(state: Option<DownloadState>, percent: u8) -> (r: DownloadBadge)
    requires
        percent <= 100,
    ensures
        r == badge_spec(state, percent),
        match r {
            DownloadBadge::Downloading(p) => p == percent && p <= 100,
            _ => true,
        },
{
    match state {
        None => DownloadBadge::None,
        Some(DownloadState::Queued) => DownloadBadge::Queued,
        Some(DownloadState::Downloading) => DownloadBadge::Downloading(percent),
        Some(DownloadState::Completed) => DownloadBadge::Completed,
        Some(DownloadState::Failed) => DownloadBadge::Failed,
        Some(DownloadState::Cancelled) => DownloadBadge::Cancelled,
    }
}

pub open spec fn icon_id_spec(badge: DownloadBadge) -> u8 {
    match badge {
        DownloadBadge::None => 1u8,
        DownloadBadge::Queued => 2u8,
        DownloadBadge::Downloading(_) => 3u8,
        DownloadBadge::Completed => 4u8,
        DownloadBadge::Failed => 5u8,
        DownloadBadge::Cancelled => 6u8,
    }
}

pub fn download_icon(badge: DownloadBadge) -> (r: u8)
    ensures
        r == icon_id_spec(badge),
        r >= 1u8,
        r <= 6u8,
{
    match badge {
        DownloadBadge::None => 1u8,
        DownloadBadge::Queued => 2u8,
        DownloadBadge::Downloading(_) => 3u8,
        DownloadBadge::Completed => 4u8,
        DownloadBadge::Failed => 5u8,
        DownloadBadge::Cancelled => 6u8,
    }
}

pub open spec fn click_spec(badge: DownloadBadge) -> DownloadClick {
    match badge {
        DownloadBadge::None => DownloadClick::Download,
        DownloadBadge::Queued => DownloadClick::Cancel,
        DownloadBadge::Downloading(_) => DownloadClick::Cancel,
        DownloadBadge::Failed => DownloadClick::Retry,
        DownloadBadge::Cancelled => DownloadClick::Retry,
        DownloadBadge::Completed => DownloadClick::None,
    }
}

pub fn download_click(badge: DownloadBadge) -> (r: DownloadClick)
    ensures
        r == click_spec(badge),
        (r == DownloadClick::None) == (badge == DownloadBadge::Completed),
{
    match badge {
        DownloadBadge::None => DownloadClick::Download,
        DownloadBadge::Queued => DownloadClick::Cancel,
        DownloadBadge::Downloading(_) => DownloadClick::Cancel,
        DownloadBadge::Failed => DownloadClick::Retry,
        DownloadBadge::Cancelled => DownloadClick::Retry,
        DownloadBadge::Completed => DownloadClick::None,
    }
}



// ===== cookie_import_format =====

pub enum CookieImportFormat {
    Netscape,
    Json,
    Header,
    Unknown,
}

pub open spec fn is_ws(b: u8) -> bool {
    b == 32u8 || b == 9u8 || b == 10u8 || b == 13u8 || b == 11u8 || b == 12u8
}

/// Leading ASCII whitespace, counted from `i`; `trim_start_len` is the `i = 0` case.
pub open spec fn trim_start_len_from(t: Seq<u8>, i: int) -> (r: int)
    decreases (t.len() as int) - i,
{
    if i >= t.len() {
        0
    } else if is_ws(t[i]) {
        1 + trim_start_len_from(t, i + 1)
    } else {
        0
    }
}

pub open spec fn trim_start_len(t: Seq<u8>) -> int {
    trim_start_len_from(t, 0)
}






pub fn trim_start_span(text: &Vec<u8>) -> (r: usize)
    ensures
        r as int == trim_start_len(text@),
{
    let mut i = 0;
    while i < text.len() && (text[i] == 32u8 || text[i] == 9u8 || text[i] == 10u8
        || text[i] == 13u8 || text[i] == 11u8 || text[i] == 12u8)
        invariant
            i <= text@.len(),
            forall|j: int| 0 <= j < i ==> is_ws(text@[j]),
        decreases text@.len() - i,
    {
        i = i + 1;
    }
    proof {
        if i < text.len() {
            lemma_ws_prefix_stops(text@, i as int);
        } else {
            lemma_ws_prefix_split(text@, i as int);
        }
    }
    i
}

pub open spec fn netscape_marker() -> Seq<u8> {
    // "# Netscape"
    seq![35u8, 32u8, 78u8, 101u8, 116u8, 115u8, 99u8, 97u8, 112u8, 101u8]
}

/// `str::starts_with(marker)` on the byte level: all `marker.len()` comparisons.
pub open spec fn matches_at(t: Seq<u8>, s: int, m: Seq<u8>, j: int) -> (r: bool)
    decreases (m.len() as int) - j,
{
    if j >= m.len() {
        true
    } else if s + j >= t.len() {
        false
    } else if t[s + j] != m[j] {
        false
    } else {
        matches_at(t, s, m, j + 1)
    }
}

pub open spec fn line_end(t: Seq<u8>, i: int) -> (r: int)
    decreases (t.len() as int) - i,
{
    if i >= t.len() {
        t.len() as int
    } else if t[i] == 10u8 {
        i
    } else {
        line_end(t, i + 1)
    }
}

pub open spec fn tabs_in(t: Seq<u8>, s: int, e: int) -> (r: int)
    decreases e - s,
{
    if s >= e {
        0
    } else if t[s] == 9u8 {
        1 + tabs_in(t, s + 1, e)
    } else {
        tabs_in(t, s + 1, e)
    }
}

pub open spec fn line_ok(t: Seq<u8>, s: int) -> bool {
    matches_at(t, s, netscape_marker(), 0) || tabs_in(t, s, line_end(t, s)) >= 6
}

/// Any line starting at an index in `[from, s)` (line starts are `from` itself
/// and any index after a `\n`).
pub open spec fn lines_ok(t: Seq<u8>, from: int, s: int) -> (r: bool)
    decreases s - from,
{
    if s <= from {
        false
    } else {
        lines_ok(t, from, s - 1) || ((s - 1 == from || t[s - 2] == 10u8) && line_ok(t, s - 1))
    }
}

pub open spec fn has_byte_in(t: Seq<u8>, from: int, to: int, b: u8) -> (r: bool)
    decreases to - from,
{
    if from >= to {
        false
    } else {
        t[to - 1] == b || has_byte_in(t, from, to - 1, b)
    }
}

pub open spec fn cookie_format_spec(t: Seq<u8>) -> CookieImportFormat {
    if trim_start_len(t) < t.len() && (t[trim_start_len(t)] == 91u8 || t[trim_start_len(t)] == 123u8)
    {
        CookieImportFormat::Json
    } else if lines_ok(t, trim_start_len(t), t.len() as int) {
        CookieImportFormat::Netscape
    } else if has_byte_in(t, trim_start_len(t), t.len() as int, 61u8) {
        CookieImportFormat::Header
    } else {
        CookieImportFormat::Unknown
    }
}



pub fn starts_with_at(text: &Vec<u8>, from: usize, marker: &Vec<u8>) -> (r: bool)
    requires
        from <= text.len(),
    ensures
        r == matches_at(text@, from as int, marker@, 0),
{
    let mut j = 0;
    let mut ok = true;
    while j < marker.len() && ok
        invariant
            j <= marker@.len(),
            from as int <= text@.len(),
            ok ==> matches_at(text@, from as int, marker@, 0) == matches_at(
                text@,
                from as int,
                marker@,
                j as int,
            ),
            !ok ==> !matches_at(text@, from as int, marker@, 0),
        decreases marker@.len() - j,
    {
        if j >= text.len() - from || text[from + j] != marker[j] {
            ok = false;
        } else {
            assert(matches_at(text@, from as int, marker@, j as int) == matches_at(
                text@,
                from as int,
                marker@,
                (j + 1) as int,
            ));
        }
        j = j + 1;
    }
    ok
}

pub fn count_tabs(text: &Vec<u8>, from: usize) -> (r: usize)
    requires
        from <= text.len(),
    ensures
        r as int == tabs_in(text@, from as int, line_end(text@, from as int)),
{
    let mut j = from;
    let mut count = 0;
    while j < text.len() && text[j] != 10u8
        invariant
            from as int <= j <= text@.len(),
            count as int == tabs_in(text@, from as int, j as int),
            count as int <= j as int,
            forall|k: int| from as int <= k < j ==> text@[k] != 10u8,
        decreases text@.len() - j,
    {
        proof {
            lemma_tabs_in_extend(text@, from as int, j as int);
        }
        if text[j] == 9u8 {
            count = count + 1;
        }
        j = j + 1;
    }
    proof {
        lemma_line_end(text@, from as int, j as int);
    }
    count
}

pub fn line_ok_exec(text: &Vec<u8>, from: usize) -> (r: bool)
    requires
        from <= text.len(),
    ensures
        r == line_ok(text@, from as int),
{
    let mut marker: Vec<u8> = Vec::new();
    marker.push(35u8);
    marker.push(32u8);
    marker.push(78u8);
    marker.push(101u8);
    marker.push(116u8);
    marker.push(115u8);
    marker.push(99u8);
    marker.push(97u8);
    marker.push(112u8);
    marker.push(101u8);
    assert(marker@ =~= netscape_marker());
    if starts_with_at(text, from, &marker) {
        true
    } else {
        count_tabs(text, from) >= 6
    }
}

pub fn has_netscape_line(text: &Vec<u8>, from: usize) -> (r: bool)
    requires
        from <= text.len(),
    ensures
        r == lines_ok(text@, from as int, text@.len() as int),
{
    let mut s = from;
    let mut found = false;
    while s < text.len()
        invariant
            from as int <= s as int <= text@.len(),
            found == lines_ok(text@, from as int, s as int),
        decreases text@.len() - s,
    {
        if s == from || text[s - 1] == 10u8 {
            if line_ok_exec(text, s) {
                found = true;
            }
        }
        s = s + 1;
    }
    found
}

pub fn contains_byte(text: &Vec<u8>, from: usize, b: u8) -> (r: bool)
    requires
        from <= text.len(),
    ensures
        r == has_byte_in(text@, from as int, text@.len() as int, b),
{
    let mut i = from;
    let mut found = false;
    while i < text.len()
        invariant
            from as int <= i <= text@.len(),
            found == has_byte_in(text@, from as int, i as int, b),
        decreases text@.len() - i,
    {
        if text[i] == b {
            found = true;
        }
        i = i + 1;
    }
    found
}

// ----- proof lemmas (ghost only; kept together before the last exec fn) -----
pub open spec fn expiry_rank(hint: ExpiryHint) -> int {
    match hint {
        ExpiryHint::None => -1,
        ExpiryHint::Expired => 0,
        ExpiryHint::UnderDay => 1,
        ExpiryHint::OneDay => 2,
        ExpiryHint::Days(_) => 3,
    }
}

pub proof fn lemma_expiry_none(now_unix: i64)
    ensures
        expiry_hint_spec(None, now_unix) == ExpiryHint::None,
{
}

pub proof fn lemma_expiry_expired(expires_at: i64, now_unix: i64)
    requires
        expires_at <= now_unix,
    ensures
        expiry_hint_spec(Some(expires_at), now_unix) == ExpiryHint::Expired,
{
}

pub proof fn lemma_expiry_boundaries(now_unix: i64)
    requires
        now_unix >= 0,
        now_unix <= i64::MAX - 200000,
    ensures
        expiry_hint_spec(Some(now_unix), now_unix) == ExpiryHint::Expired,
        expiry_hint_spec(Some((now_unix + 1) as i64), now_unix) == ExpiryHint::UnderDay,
        expiry_hint_spec(Some((now_unix + 86399) as i64), now_unix) == ExpiryHint::UnderDay,
        expiry_hint_spec(Some((now_unix + 86400) as i64), now_unix) == ExpiryHint::OneDay,
        expiry_hint_spec(Some((now_unix + 172799) as i64), now_unix) == ExpiryHint::OneDay,
        expiry_hint_spec(Some((now_unix + 172800) as i64), now_unix) == ExpiryHint::Days(2u64),
{
}

pub proof fn lemma_expiry_monotone(early: i64, late: i64, now_unix: i64)
    requires
        now_unix >= 0,
        early <= late,
    ensures
        expiry_rank(expiry_hint_spec(Some(early), now_unix)) <= expiry_rank(
            expiry_hint_spec(Some(late), now_unix),
        ),
{
}

pub proof fn lemma_expiry_days_monotone(d1: u64, d2: u64)
    requires
        d1 <= d2,
    ensures
        expiry_rank(ExpiryHint::Days(d1)) <= expiry_rank(ExpiryHint::Days(d2)),
{
}



pub proof fn lemma_extend_needs_queue(position: Option<usize>, len: usize)
    ensures
        should_extend_spec(position, len) ==> len > 0,
{
}

pub proof fn lemma_extend_never_empty(len: usize)
    ensures
        !should_extend_spec(None, len),
        !should_extend_spec(Some(0usize), 0usize),
{
}

pub proof fn lemma_sleep_seconds_minutes(minutes: u32)
    requires
        (minutes as int) <= 1440,
    ensures
        sleep_seconds_spec(minutes) == (minutes as int) * 60,
{
}

pub proof fn lemma_sleep_seconds_monotone(a: u32, b: u32)
    requires
        a <= b,
    ensures
        sleep_seconds_spec(a) <= sleep_seconds_spec(b),
{
}

pub proof fn lemma_seek_relative_no_underflow(position_ms: u64, offset_ms: i64)
    requires
        offset_ms < 0,
    ensures
        seek_raw(position_ms, offset_ms) >= 0,
{
}

pub proof fn lemma_seek_relative_bounded(position_ms: u64, duration_ms: u64, offset_ms: i64)
    requires
        duration_ms > 0,
    ensures
        seek_relative_spec(position_ms, duration_ms, offset_ms) <= (duration_ms as int),
{
}

pub proof fn lemma_percent_zero_bytes(total: u64)
    ensures
        percent_spec(0, total) == 0,
{
}

pub proof fn lemma_percent_full(bytes: u64, total: u64)
    requires
        total > 0,
        bytes >= total,
    ensures
        percent_spec(bytes, total) == 100,
{
    assert((bytes as int) * 100 / (total as int) >= 100) by (nonlinear_arith)
        requires
            total > 0,
            bytes >= total;
}

pub proof fn lemma_seed_color_masked(value: u32)
    ensures
        (value & 0x00FF_FFFFu32) <= 0x00FF_FFFFu32,
        ((value & 0x00FF_FFFFu32) & 0xFF00_0000u32) == 0u32,
{
    assert((value & 0x00FF_FFFFu32) <= 0x00FF_FFFFu32) by (bit_vector);
    assert(((value & 0x00FF_FFFFu32) & 0xFF00_0000u32) == 0u32) by (bit_vector);
}

pub proof fn lemma_icon_table_distinct()
    ensures
        icon_id_spec(DownloadBadge::None) != icon_id_spec(DownloadBadge::Queued),
        icon_id_spec(DownloadBadge::Queued) != icon_id_spec(DownloadBadge::Completed),
        icon_id_spec(DownloadBadge::Completed) != icon_id_spec(DownloadBadge::Failed),
        icon_id_spec(DownloadBadge::Failed) != icon_id_spec(DownloadBadge::Cancelled),
{
}

pub proof fn lemma_trim_split(t: Seq<u8>, i: int)
    requires
        0 <= i < t.len(),
        is_ws(t[i]),
    ensures
        trim_start_len_from(t, i) == 1 + trim_start_len_from(t, i + 1),
{
}

pub proof fn lemma_ws_prefix_split(t: Seq<u8>, i: int)
    requires
        0 <= i <= t.len(),
        forall|j: int| 0 <= j < i ==> is_ws(t[j]),
    ensures
        trim_start_len_from(t, 0) == i + trim_start_len_from(t, i),
    decreases i,
{
    if i == 0 {
    } else {
        lemma_ws_prefix_split(t, i - 1);
        lemma_trim_split(t, i - 1);
    }
}

pub proof fn lemma_ws_prefix_stops(t: Seq<u8>, i: int)
    requires
        0 <= i < t.len(),
        forall|j: int| 0 <= j < i ==> is_ws(t[j]),
        !is_ws(t[i]),
    ensures
        trim_start_len(t) == i,
{
    lemma_ws_prefix_split(t, i);
}

pub proof fn lemma_trim_from_bound(t: Seq<u8>, i: int)
    requires
        0 <= i <= t.len(),
    ensures
        trim_start_len_from(t, i) <= (t.len() as int) - i,
    decreases (t.len() as int) - i,
{
    if i >= t.len() {
    } else if is_ws(t[i]) {
        lemma_trim_from_bound(t, i + 1);
    }
}

pub proof fn lemma_trim_start_len_bound(t: Seq<u8>)
    ensures
        trim_start_len(t) <= t.len(),
{
    lemma_trim_from_bound(t, 0);
}

pub proof fn lemma_tabs_in_extend(t: Seq<u8>, s: int, e: int)
    requires
        s <= e < t.len(),
    ensures
        tabs_in(t, s, e + 1) == tabs_in(t, s, e) + (if t[e] == 9u8 {
            1int
        } else {
            0int
        }),
    decreases e - s,
{
    if s >= e {
    } else {
        lemma_tabs_in_extend(t, s + 1, e);
        assert(tabs_in(t, s, e + 1) == (if t[s] == 9u8 {
            1int
        } else {
            0int
        }) + tabs_in(t, s + 1, e + 1));
        assert(tabs_in(t, s, e) == (if t[s] == 9u8 {
            1int
        } else {
            0int
        }) + tabs_in(t, s + 1, e));
    }
}

pub proof fn lemma_line_end(text: Seq<u8>, i: int, j: int)
    requires
        i <= j <= text.len(),
        forall|k: int| i <= k < j ==> text[k] != 10u8,
        j == text.len() || text[j] == 10u8,
    ensures
        line_end(text, i) == j,
    decreases (j - i) as int,
{
    if i == j {
    } else {
        lemma_line_end(text, i + 1, j);
    }
}

// ===== expand_import_path (v0.1.2 account fix) =====
// A pasted import path: trim, one matching quote pair, `~`/`~/` against HOME.
// Modelled on bytes like session_file_path; `PathBuf::join` is transcribed as
// `path_join_spec` (the exact Unix rules, including an empty rest).

pub open spec fn are_quotes(t: Seq<u8>) -> bool {
    t.len() >= 2 && ((t[0] == 34u8 && t[t.len() - 1] == 34u8) || (t[0] == 39u8
        && t[t.len() - 1] == 39u8))
}

pub open spec fn unquoted(t: Seq<u8>) -> Seq<u8> {
    if are_quotes(t) {
        t.subrange(1, (t.len() - 1) as int)
    } else {
        t
    }
}

pub open spec fn trail_ws_len(t: Seq<u8>) -> int
    decreases t.len(),
{
    if t.len() == 0 {
        0
    } else if is_ws(t[t.len() - 1]) {
        1 + trail_ws_len(t.subrange(0, (t.len() - 1) as int))
    } else {
        0
    }
}

pub open spec fn trim_hi(t: Seq<u8>) -> int {
    if trim_start_len(t) == t.len() {
        t.len() as int
    } else {
        t.len() as int - trail_ws_len(t)
    }
}

pub open spec fn trimmed(t: Seq<u8>) -> Seq<u8> {
    t.subrange(trim_start_len(t), trim_hi(t))
}

pub open spec fn sep_width(home: Seq<u8>) -> int {
    if home.len() == 0 || home[home.len() - 1] == 47u8 {
        0
    } else {
        1
    }
}

pub open spec fn path_join_spec(home: Seq<u8>, rest: Seq<u8>) -> Seq<u8> {
    if rest.len() > 0 && rest[0] == 47u8 {
        rest
    } else if rest.len() == 0 {
        if home.len() == 0 || home[home.len() - 1] == 47u8 {
            home
        } else {
            home.push(47u8)
        }
    } else if home.len() == 0 {
        rest
    } else if home[home.len() - 1] == 47u8 {
        home + rest
    } else {
        home.push(47u8) + rest
    }
}

pub open spec fn opt_home(home: Option<Vec<u8>>) -> Option<Seq<u8>> {
    match home {
        Some(bytes) => Some(bytes@),
        None => None,
    }
}

pub open spec fn expand_import_path_spec(input: Seq<u8>, home: Option<Seq<u8>>) -> Seq<u8> {
    let text = unquoted(trimmed(input));
    if text.len() == 1 && text[0] == 126u8 {
        match home {
            None => text,
            Some(home) => home,
        }
    } else if text.len() >= 2 && text[0] == 126u8 && text[1] == 47u8 {
        match home {
            None => text,
            Some(home) => path_join_spec(home, text.subrange(2, text.len() as int)),
        }
    } else {
        text
    }
}

pub fn trim_span(text: &Vec<u8>) -> (r: (usize, usize))
    ensures
        r.0 as int == trim_start_len(text@),
        r.1 as int == trim_hi(text@),
        r.0 as int <= r.1 as int,
        r.1 as int <= text@.len(),
{
    let mut lead = 0;
    while lead < text.len() && (text[lead] == 32u8 || text[lead] == 9u8
        || text[lead] == 10u8 || text[lead] == 13u8 || text[lead] == 11u8
        || text[lead] == 12u8)
        invariant
            lead <= text@.len(),
            forall|j: int| 0 <= j < lead as int ==> is_ws(#[trigger] text@[j]),
        decreases text@.len() - lead,
    {
        lead = lead + 1;
    }
    if lead == text.len() {
        proof {
            lemma_ws_prefix_split(text@, text.len() as int);
        }
        return (lead, lead);
    }
    proof {
        lemma_ws_prefix_stops(text@, lead as int);
    }
    let mut end = text.len();
    while end > lead && (text[end - 1] == 32u8 || text[end - 1] == 9u8
        || text[end - 1] == 10u8 || text[end - 1] == 13u8 || text[end - 1] == 11u8
        || text[end - 1] == 12u8)
        invariant
            lead <= end <= text@.len(),
            forall|j: int| end as int <= j < text@.len() ==> is_ws(#[trigger] text@[j]),
        decreases end - lead,
    {
        end = end - 1;
    }
    proof {
        if end > 0 {
            assert(!is_ws(text@[end as int - 1]));
        }
        lemma_trail_run(text@, end as int);
    }
    (lead, end)
}

pub fn copy_range(text: &Vec<u8>, lo: usize, hi: usize) -> (r: Vec<u8>)
    requires
        lo <= hi <= text.len(),
    ensures
        r@ == text@.subrange(lo as int, hi as int),
{
    let mut out: Vec<u8> = Vec::new();
    let mut i = lo;
    while i < hi
        invariant
            lo <= i <= hi <= text@.len(),
            out@.len() == i - lo,
            forall|j: int| 0 <= j < out@.len() ==> out@[j] == text@[lo as int + j],
        decreases hi - i,
    {
        out.push(text[i]);
        i = i + 1;
    }
    proof {
        assert(out@ =~= text@.subrange(lo as int, hi as int));
    }
    out
}

pub fn path_join(home: &Vec<u8>, rest: &Vec<u8>) -> (r: Vec<u8>)
    ensures
        r@ == path_join_spec(home@, rest@),
{
    if rest.len() > 0 && rest[0] == 47u8 {
        return copy_range(rest, 0, rest.len());
    }
    if rest.len() == 0 {
        if home.len() == 0 {
            return copy_range(home, 0, home.len());
        }
        if home[home.len() - 1] == 47u8 {
            return copy_range(home, 0, home.len());
        }
        let mut out = copy_range(home, 0, home.len());
        out.push(47u8);
        return out;
    }
    if home.len() == 0 {
        return copy_range(rest, 0, rest.len());
    }
    proof {
        assert(home.len() > 0);
    }
    let mut out = copy_range(home, 0, home.len());
    if home[home.len() - 1] != 47u8 {
        out.push(47u8);
    }
    proof {
        if sep_width(home@) == 1 {
            assert(out@.len() == home@.len() + 1);
            assert(out@[home@.len() as int] == 47u8);
        }
    }
    let mut i = 0;
    while i < rest.len()
        invariant
            i <= rest@.len(),
            out@.len() == home@.len() as int + sep_width(home@) + i as int,
            forall|j: int| 0 <= j < home@.len() ==> out@[j] == home@[j],
            forall|j: int| 0 <= j < i as int ==> out@[home@.len() as int + sep_width(
                home@,
            ) + j] == rest@[j],
            sep_width(home@) == 1 ==> out@[home@.len() as int] == 47u8,
        decreases rest@.len() - i,
    {
        out.push(rest[i]);
        i = i + 1;
    }
    proof {
        if home@[home@.len() - 1] == 47u8 {
            assert(sep_width(home@) == 0);
            assert forall|j: int| 0 <= j < out@.len() implies out@[j] == (home@ + rest@)[j] by {
                if j < home@.len() {
                } else {
                    assert(out@[j] == rest@[j - home@.len()]);
                }
            }
            assert(out@ =~= home@ + rest@);
        } else {
            assert(sep_width(home@) == 1);
            let prefix = home@.push(47u8);
            assert forall|j: int| 0 <= j < out@.len() implies out@[j] == (prefix + rest@)[j] by {
                if j < home@.len() {
                } else if j == home@.len() {
                } else {
                    assert(out@[j] == rest@[j - home@.len() - 1]);
                    assert((prefix + rest@)[j] == rest@[j - prefix.len()]);
                }
            }
            assert(prefix + rest@ =~= out@);
        }
    }
    out
}

pub fn expand_import_path(input: &Vec<u8>, home: Option<Vec<u8>>) -> (r: Vec<u8>)
    ensures
        r@ == expand_import_path_spec(input@, opt_home(home)),
{
    let (start, end) = trim_span(input);
    proof {
        assert(start as int == trim_start_len(input@));
        assert(end as int == trim_hi(input@));
        assert(start as int <= end as int);
        assert(end as int <= input@.len());
        assert(trimmed(input@) == input@.subrange(start as int, end as int));
    }
    let quotes = if end - start >= 2 {
        assert(start as int + 2 <= end as int);
        let first = input[start];
        let last = input[end - 1];
        (first == 34u8 && last == 34u8) || (first == 39u8 && last == 39u8)
    } else {
        false
    };
    let (qs, qe) = if quotes {
        (start + 1, end - 1)
    } else {
        (start, end)
    };
    proof {
        lemma_quotes_match(input@, start as int, end as int);
        if quotes {
            lemma_subrange_shift(input@, start as int, end as int, 1, end as int
                - start as int - 1);
            assert(input@.subrange(qs as int, qe as int) == input@.subrange(
                start as int,
                end as int,
            ).subrange(1, (end as int - start as int) - 1));
        }
        assert(input@.subrange(qs as int, qe as int) == unquoted(trimmed(input@)));
    }
    assert(qs <= qe);
    assert(qe <= input.len());
    if qe - qs == 1 && input[qs] == 126u8 {
        match home {
            Some(bytes) => return bytes,
            None => return copy_range(input, qs, qe),
        }
    }
    if qe - qs >= 2 && input[qs] == 126u8 && input[qs + 1] == 47u8 {
        assert(qs + 2 <= qe);
        let tail = copy_range(input, qs + 2, qe);
        match home {
            Some(bytes) => return path_join(&bytes, &tail),
            None => return copy_range(input, qs, qe),
        }
    }
    copy_range(input, qs, qe)
}


pub proof fn lemma_quotes_match(input: Seq<u8>, start: int, end: int)
    requires
        0 <= start <= end <= input.len(),
    ensures
        are_quotes(input.subrange(start, end)) == (end - start >= 2 && ((
        input[start] == 34u8 && input[end - 1] == 34u8) || (input[start] == 39u8
            && input[end - 1] == 39u8))),
{
    let t = input.subrange(start, end);
    if end - start >= 2 {
        assert(t.len() == end - start);
        assert(t[0] == input[start]);
        assert(t[t.len() - 1] == input[end - 1]);
    }
}

pub proof fn lemma_subrange_shift(input: Seq<u8>, lo: int, hi: int, a: int, b: int)
    requires
        0 <= lo <= lo + a <= lo + b <= hi <= input.len(),
    ensures
        input.subrange(lo + a, lo + b) == input.subrange(lo, hi).subrange(a, b),
{
    assert forall|i: int| 0 <= i < b - a implies #[trigger] input.subrange(
        lo + a,
        lo + b,
    )[i] == input.subrange(lo, hi).subrange(a, b)[i] by {
        assert(input.subrange(lo + a, lo + b)[i] == input[lo + a + i]);
        assert(input.subrange(lo, hi)[a + i] == input[lo + a + i]);
    }
    assert(input.subrange(lo + a, lo + b) =~= input.subrange(lo, hi).subrange(a, b));
}

pub proof fn lemma_trail_run(t: Seq<u8>, i: int)
    requires
        0 <= i <= t.len(),
        forall|j: int| i <= j < t.len() ==> is_ws(#[trigger] t[j]),
        i == 0 || !is_ws(t[i - 1]),
    ensures
        trail_ws_len(t) == t.len() - i,
    decreases t.len() - i,
{
    if i == t.len() {
        if t.len() > 0 {
            assert(!is_ws(t[t.len() - 1]));
        }
    } else {
        assert(is_ws(t[t.len() - 1]));
        let inner = t.subrange(0, t.len() - 1);
        assert forall|j: int| i <= j < inner.len() implies is_ws(#[trigger] inner[j]) by {
            assert(inner[j] == t[j]);
        }
        lemma_trail_run(inner, i);
    }
}

pub fn cookie_import_format(text: &Vec<u8>) -> (r: CookieImportFormat)
    ensures
        r == cookie_format_spec(text@),
{
    let start = trim_start_span(text);
    proof {
        lemma_trim_start_len_bound(text@);
    }
    if start < text.len() && (text[start] == 91u8 || text[start] == 123u8) {
        return CookieImportFormat::Json;
    }
    if has_netscape_line(text, start) {
        return CookieImportFormat::Netscape;
    }
    if contains_byte(text, start, 61u8) {
        return CookieImportFormat::Header;
    }
    CookieImportFormat::Unknown
}

} // verus!
