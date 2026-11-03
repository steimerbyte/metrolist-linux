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
        decreases data_dir@.len() - i,
    {
        let b = data_dir[i];
        out.push(b);
        i = i + 1;
    }
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

pub fn seed_color_value(value: u32) -> (r: u32)
    ensures
        r == (value & 0x00FF_FFFFu32),
        r <= 0x00FF_FFFFu32,
        (r & 0xFF00_0000u32) == 0,
{
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
        decreases text@.len() - i,
    {
        i = i + 1;
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
        decreases marker@.len() - j,
    {
        if j >= text.len() - from || text[from + j] != marker[j] {
            ok = false;
        } else {
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
        decreases text@.len() - j,
    {
        if text[j] == 9u8 {
            count = count + 1;
        }
        j = j + 1;
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
        decreases text@.len() - lead,
    {
        lead = lead + 1;
    }
    if lead == text.len() {
        return (lead, lead);
    }
    let mut end = text.len();
    while end > lead && (text[end - 1] == 32u8 || text[end - 1] == 9u8
        || text[end - 1] == 10u8 || text[end - 1] == 13u8 || text[end - 1] == 11u8
        || text[end - 1] == 12u8)
        decreases end - lead,
    {
        end = end - 1;
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
        decreases hi - i,
    {
        out.push(text[i]);
        i = i + 1;
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
    let mut out = copy_range(home, 0, home.len());
    if home[home.len() - 1] != 47u8 {
        out.push(47u8);
    }
    let mut i = 0;
    while i < rest.len()
        decreases rest@.len() - i,
    {
        out.push(rest[i]);
        i = i + 1;
    }
    out
}

pub fn expand_import_path(input: &Vec<u8>, home: Option<Vec<u8>>) -> (r: Vec<u8>)
    ensures
        r@ == expand_import_path_spec(input@, opt_home(home)),
{
    let (start, end) = trim_span(input);
    let quotes = if end - start >= 2 {
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
    if qe - qs == 1 && input[qs] == 126u8 {
        match home {
            Some(bytes) => return bytes,
            None => return copy_range(input, qs, qe),
        }
    }
    if qe - qs >= 2 && input[qs] == 126u8 && input[qs + 1] == 47u8 {
        let tail = copy_range(input, qs + 2, qe);
        match home {
            Some(bytes) => return path_join(&bytes, &tail),
            None => return copy_range(input, qs, qe),
        }
    }
    copy_range(input, qs, qe)
}

pub fn cookie_import_format(text: &Vec<u8>) -> (r: CookieImportFormat)
    ensures
        r == cookie_format_spec(text@),
{
    let start = trim_start_span(text);
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

// ===== play_starts_radio (v0.1.3 UI tranche) =====
//
// `ml_core::QueueSource` with its full six-variant shape (the `radio` pair only
// needs two): the predicate compares against `List`. `App::play_intent` routes
// a row tap through this predicate and an explicit "Play all" around it.

#[derive(PartialEq, Eq)]
pub enum QueueSource {
    List,
    Album,
    Playlist,
    Radio,
    Artist,
    Local,
}

/// `!play_all && song_count == 1 && source == List`, exactly the source body.
pub open spec fn play_starts_radio_spec(
    song_count: usize,
    source: QueueSource,
    play_all: bool,
) -> bool {
    !play_all && song_count == 1 && source == QueueSource::List
}

pub fn play_starts_radio(song_count: usize, source: QueueSource, play_all: bool) -> (r: bool)
    ensures
        r == play_starts_radio_spec(song_count, source, play_all),
        r ==> song_count == 1,
        r ==> source == QueueSource::List,
        play_all ==> !r,
        song_count == 0 ==> !r,
{
    // The source compares `source == QueueSource::List`; the derived `PartialEq`
    // is opaque to Verus, so the fieldless enum is tested by variant here.
    !play_all && song_count == 1 && match source {
        QueueSource::List => true,
        _ => false,
    }
}

// ===== PlaylistOrder::sort_mode (v0.1.3 UI tranche) =====
//
// The two display enums, reduced to their variants. Labels are `&'static str`
// and stay CANDIDATE (see SOURCE.md); the sorters themselves are Vec/String and
// out of subset.

pub enum SortMode {
    RecentlyAdded,
    Title,
    Artist,
    Album,
    Duration,
}

pub enum PlaylistOrder {
    Default,
    RecentlyAdded,
    Title,
    Artist,
    Duration,
}

pub open spec fn sort_mode_spec(order: PlaylistOrder) -> Option<SortMode> {
    match order {
        PlaylistOrder::Default => None,
        PlaylistOrder::RecentlyAdded => Some(SortMode::RecentlyAdded),
        PlaylistOrder::Title => Some(SortMode::Title),
        PlaylistOrder::Artist => Some(SortMode::Artist),
        PlaylistOrder::Duration => Some(SortMode::Duration),
    }
}

pub open spec fn sort_mode_is_none(order: PlaylistOrder) -> bool {
    match sort_mode_spec(order) {
        None => true,
        Some(_) => false,
    }
}

impl PlaylistOrder {
    fn sort_mode(self) -> (r: Option<SortMode>)
        ensures
            r == sort_mode_spec(self),
    {
        match self {
            PlaylistOrder::Default => None,
            PlaylistOrder::RecentlyAdded => Some(SortMode::RecentlyAdded),
            PlaylistOrder::Title => Some(SortMode::Title),
            PlaylistOrder::Artist => Some(SortMode::Artist),
            PlaylistOrder::Duration => Some(SortMode::Duration),
        }
    }
}
} // verus!