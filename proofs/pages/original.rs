use vstd::prelude::*;

verus! {

/// `ml_core::QueuePage` with `Song.id` modelled as a `u32` id token and the
/// continuation token as `Option<u32>`; `title` is out of subset and omitted.
pub struct Page {
    pub songs: Vec<u32>,
    pub current_index: Option<usize>,
    pub continuation: Option<u32>,
}

/// Model of `page.songs.iter().any(|queued| queued.id == song.id)`:
/// Verus has no spec linking `Iterator::any` to `Seq::contains`, so the
/// predicate is an exec helper with the equivalent contract.
pub fn contains_id(ids: &Vec<u32>, id: u32) -> (r: bool)
    ensures r == ids@.contains(id),
{
    let mut j: usize = 0;
    while j < ids.len()
        decreases ids@.len() - j,
    {
        if ids[j] == id {
            return true;
        }
        j = j + 1;
    }
    false
}

/// `pages::next::append` over id tokens: each id of `next` is appended once, in
/// server order, iff it is not already queued; the pre-existing order is kept.
pub open spec fn append_ids(page: Seq<u32>, next: Seq<u32>) -> Seq<u32>
    decreases next.len()
{
    if next.len() == 0 {
        page
    } else if page.contains(next[0]) {
        append_ids(page, next.subrange(1, next.len() as int))
    } else {
        append_ids(page.push(next[0]), next.subrange(1, next.len() as int))
    }
}

pub fn append(page: &mut Page, next: Page) -> (r: usize)
    ensures
        final(page).songs@ == append_ids(old(page).songs@, next.songs@),
        r == final(page).songs@.len() - old(page).songs@.len(),
        final(page).continuation == next.continuation,
        final(page).current_index == old(page).current_index,
{
    let before = page.songs.len();
    let mut i: usize = 0;
    while i < next.songs.len()
        decreases next.songs.len() - i,
    {
        let song = next.songs[i];
        if !contains_id(&page.songs, song) {
            page.songs.push(song);
        }
        i = i + 1;
    }
    page.continuation = next.continuation;
    page.songs.len() - before
}

} // verus!
