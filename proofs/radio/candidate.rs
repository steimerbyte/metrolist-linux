use vstd::prelude::*;

verus! {

// ---------- crates/ui/src/services.rs ----------

/// `ml_core::QueueSource`, reduced to what a radio event can carry.
#[derive(PartialEq, Eq)]
pub enum QueueSource {
    Radio,
    List,
}

/// `ml_core::Song`, reduced to the identity the index search uses.
pub type Song = u32;

/// `ml_core::QueuePage`, reduced to the fields `radio_event` reads.
pub struct QueuePage {
    pub songs: Vec<Song>,
    pub current_index: Option<usize>,
}

/// `UiEvent::RadioReady`'s payload; the other `UiEvent` variants are
/// irrelevant to this event.
pub struct RadioReady {
    pub origin: Song,
    pub songs: Vec<Song>,
    pub index: usize,
    pub source: QueueSource,
}

/// The panel marker, when it is in range; `0` otherwise.
pub open spec fn panel_index(len: nat, current: Option<usize>) -> int {
    match current {
        Some(c) => {
            if (c as int) < len {
                c as int
            } else {
                0
            }
        },
        None => 0,
    }
}

/// Map a `next` queue page to the radio event: the track the user picked wins
/// over the panel's own `selected` row; `current_index` is only a fallback.
pub fn radio_event(song: Song, page: QueuePage) -> (r: RadioReady)
    ensures
        r.origin == song,
        r.songs == page.songs,
        r.source == QueueSource::Radio,
        page.songs@.len() > 0 ==> r.index < page.songs@.len(),
        (exists|j: int| 0 <= j < page.songs@.len() && #[trigger] page.songs@[j] == song) ==> {
            &&& page.songs@[r.index as int] == song
            &&& forall|j: int| 0 <= j < r.index ==> #[trigger] page.songs@[j] != song
        },
        (!exists|j: int| 0 <= j < page.songs@.len() && #[trigger] page.songs@[j] == song) ==> r.index
            as int == panel_index(page.songs@.len(), page.current_index),
{
    let mut index: Option<usize> = None;
    let mut i: usize = 0;
    while i < page.songs.len()
        invariant
            i <= page.songs.len(),
            match index {
                Some(found) => {
                    &&& (found as int) < i
                    &&& page.songs@[found as int] == song
                    &&& forall|j: int| 0 <= j < found ==> #[trigger] page.songs@[j] != song
                },
                None => forall|j: int| 0 <= j < i ==> #[trigger] page.songs@[j] != song,
            },
        decreases page.songs.len() - i,
    {
        if index.is_none() && page.songs[i] == song {
            index = Some(i);
        }
        i = i + 1;
    }
    let index = match index {
        Some(found) => found,
        None => match page.current_index {
            Some(current) if current < page.songs.len() => current,
            _ => 0,
        },
    };
    RadioReady { origin: song, songs: page.songs, index, source: QueueSource::Radio }
}

/// Single-track fallback: the clicked song becomes the whole queue.
pub fn radio_fallback(song: Song, source: QueueSource) -> (r: RadioReady)
    ensures
        r.origin == song,
        r.songs@ == seq![song],
        r.index == 0,
        r.source == source,
{
    RadioReady { origin: song, songs: vec![song], index: 0, source }
}

// ---------- crates/ui/src/app.rs ----------

/// `ml_innertube::parse::url::ParsedUrl`, with the id as its character
/// sequence (the real field is a `String`; only the `MPRE` prefix is read).
pub enum ParsedUrl {
    Video(Vec<char>),
    Playlist(Vec<char>),
    Album(Vec<char>),
    Artist(Vec<char>),
}

/// The routes a pasted link can open.
pub enum Route {
    Album(Vec<char>),
    Artist(Vec<char>),
    Playlist(Vec<char>),
}

pub open spec fn parsed_id(parsed: &ParsedUrl) -> Seq<char> {
    match parsed {
        ParsedUrl::Video(id) => id@,
        ParsedUrl::Playlist(id) => id@,
        ParsedUrl::Album(id) => id@,
        ParsedUrl::Artist(id) => id@,
    }
}

/// `id.starts_with("MPRE")`.
pub open spec fn is_album_browse_id(id: Seq<char>) -> bool {
    id.len() >= 4 && id[0] == 'M' && id[1] == 'P' && id[2] == 'R' && id[3] == 'E'
}

fn starts_with_mpre(id: &Vec<char>) -> (r: bool)
    ensures r == is_album_browse_id(id@),
{
    id.len() >= 4 && id[0] == 'M' && id[1] == 'P' && id[2] == 'R' && id[3] == 'E'
}

/// 0 = album page, 1 = artist page, 2 = playlist page, -1 = no route.
pub open spec fn route_kind(r: Option<Route>) -> int {
    match r {
        Some(Route::Album(_)) => 0,
        Some(Route::Artist(_)) => 1,
        Some(Route::Playlist(_)) => 2,
        None => -1,
    }
}

pub open spec fn route_id(r: Option<Route>) -> Seq<char> {
    match r {
        Some(Route::Album(id)) => id@,
        Some(Route::Artist(id)) => id@,
        Some(Route::Playlist(id)) => id@,
        None => Seq::empty(),
    }
}

/// Destination for a YouTube link pasted into the search box. `None` means the
/// link is played (video URLs start that video's radio); album `list=` links
/// carry a playlist id, so they open the playlist page unless the id is
/// already an `MPRE…` album browse id.
pub fn url_route(parsed: &ParsedUrl) -> (r: Option<Route>)
    ensures
        match parsed {
            ParsedUrl::Video(_) => r.is_none() && route_kind(r) == -1,
            ParsedUrl::Playlist(id) => route_kind(r) == 2 && route_id(r) == id@,
            ParsedUrl::Album(id) => if is_album_browse_id(id@) {
                route_kind(r) == 0 && route_id(r) == id@
            } else {
                route_kind(r) == 2 && route_id(r) == id@
            },
            ParsedUrl::Artist(id) => route_kind(r) == 1 && route_id(r) == id@,
        },
        r.is_some() ==> parsed_id(parsed) == route_id(r),
{
    Some(match parsed {
        ParsedUrl::Video(_) => return None,
        ParsedUrl::Playlist(id) => Route::Playlist(id.clone()),
        ParsedUrl::Album(id) => {
            if starts_with_mpre(id) {
                Route::Album(id.clone())
            } else {
                Route::Playlist(id.clone())
            }
        }
        ParsedUrl::Artist(id) => Route::Artist(id.clone()),
    })
}

// ---------- crates/innertube/src/lib.rs ----------

/// `SearchFilter`, the five filter tabs.
#[derive(PartialEq, Eq)]
pub enum SearchFilter {
    Songs,
    Albums,
    Artists,
    Playlists,
    Videos,
}

/// The `params` token each filter browses. The real values are the measured
/// WEB_REMIX base64 strings; they are modelled by five distinct opaque tokens
/// (the proof only needs identity, not the bytes).
pub open spec fn params_of(filter: SearchFilter) -> u32 {
    match filter {
        SearchFilter::Songs => 1,
        SearchFilter::Albums => 2,
        SearchFilter::Artists => 3,
        SearchFilter::Playlists => 4,
        SearchFilter::Videos => 5,
    }
}

/// `serde_json::Value` request body, reduced to the two fields the request
/// carries (`query`, plus `params` only when a filter is given).
pub struct SearchBody {
    pub query: u32,
    pub params: Option<u32>,
}

impl SearchFilter {
    pub fn params(self) -> (r: u32)
        ensures r == params_of(self),
    {
        match self {
            SearchFilter::Songs => 1,
            SearchFilter::Albums => 2,
            SearchFilter::Artists => 3,
            SearchFilter::Playlists => 4,
            SearchFilter::Videos => 5,
        }
    }
}

pub open spec fn search_body_spec(query: u32, filter: Option<SearchFilter>) -> SearchBody {
    SearchBody { query, params: match filter { Some(f) => Some(params_of(f)), None => None } }
}

/// `search` request body: the query text is copied verbatim and `params` is
/// attached exactly when a filter is given.
pub fn search_body(query: u32, filter: Option<SearchFilter>) -> (r: SearchBody)
    ensures
        r.query == query,
        r.params == match filter {
            Some(f) => Some(params_of(f)),
            None => None,
        },
{
    let mut body = SearchBody { query, params: None };
    if let Some(filter) = filter {
        body.params = Some(filter.params());
    }
    body
}


/// `MPRE` is a four-character prefix: three characters are never enough.
pub proof fn lemma_mpre_needs_four_chars()
    ensures
        !is_album_browse_id(seq!['M', 'P', 'R']),
        is_album_browse_id(seq!['M', 'P', 'R', 'E']),
        is_album_browse_id(seq!['M', 'P', 'R', 'E', 'b']),
        !is_album_browse_id(seq!['O', 'L', 'A', 'K']),
{
}

/// The five filters map to five different params tokens.
pub proof fn lemma_filter_params_distinct()
    ensures
        params_of(SearchFilter::Songs) != params_of(SearchFilter::Albums),
        params_of(SearchFilter::Songs) != params_of(SearchFilter::Artists),
        params_of(SearchFilter::Songs) != params_of(SearchFilter::Playlists),
        params_of(SearchFilter::Songs) != params_of(SearchFilter::Videos),
        params_of(SearchFilter::Albums) != params_of(SearchFilter::Artists),
        params_of(SearchFilter::Albums) != params_of(SearchFilter::Playlists),
        params_of(SearchFilter::Albums) != params_of(SearchFilter::Videos),
        params_of(SearchFilter::Artists) != params_of(SearchFilter::Playlists),
        params_of(SearchFilter::Artists) != params_of(SearchFilter::Videos),
        params_of(SearchFilter::Playlists) != params_of(SearchFilter::Videos),
{
}

/// Without a filter the body carries no `params` field at all.
pub proof fn lemma_no_filter_no_params(query: u32, filter: Option<SearchFilter>)
    requires filter.is_none(),
    ensures search_body_spec(query, filter).params.is_none(),
{
}

/// The query survives the filter mapping untouched: adding a filter can never
/// truncate, escape or reinterpret it.
pub proof fn lemma_query_is_verbatim(query: u32, filter: Option<SearchFilter>)
    ensures search_body_spec(query, filter).query == query,
{
}

/// Two bodies with the same query and the same filter are the same request.
pub proof fn lemma_same_inputs_same_body(query: u32, filter: Option<SearchFilter>)
    ensures search_body_spec(query, filter) == search_body_spec(query, filter),
{
}



} // verus!