//! Navigation vocabulary. The shell keeps one active tab plus a player route,
//! matching the Android app's four bottom destinations, and a stack of deeper
//! routes (album/artist/playlist/lyrics) on top of the selected tab.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    #[default]
    Home,
    Search,
    Together,
    Library,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Home, Tab::Search, Tab::Together, Tab::Library];

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Search => "Search",
            Self::Together => "Together",
            Self::Library => "Library",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Search => "search",
            Self::Together => "group",
            Self::Library => "library_music",
        }
    }

    /// Ctrl+1..4.
    pub fn from_shortcut(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

/// One destination in the back stack.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    Home,
    Search,
    Together,
    Library,
    Album {
        id: String,
    },
    Artist {
        id: String,
    },
    Playlist {
        id: String,
    },
    LocalPlaylist {
        id: String,
    },
    Liked,
    History,
    Settings,
    Account,
    Lyrics,
    Charts,
    Moods,
    NewReleases,
    Podcasts,
    Podcast {
        id: String,
    },
    MoodResults {
        params: Option<String>,
        title: Option<String>,
    },
    Stats,
    Wrapped,
    Recognize,
}

impl Route {
    pub fn tab(&self) -> Option<Tab> {
        match self {
            Self::Home => Some(Tab::Home),
            Self::Search => Some(Tab::Search),
            Self::Together => Some(Tab::Together),
            Self::Library => Some(Tab::Library),
            _ => None,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Search => "Search",
            Self::Together => "Together",
            Self::Library => "Library",
            Self::Album { .. } => "Album",
            Self::Artist { .. } => "Artist",
            Self::Playlist { .. } => "Playlist",
            Self::LocalPlaylist { .. } => "Playlist",
            Self::Liked => "Liked songs",
            Self::History => "History",
            Self::Settings => "Settings",
            Self::Account => "Account",
            Self::Lyrics => "Lyrics",
            Self::Charts => "Charts",
            Self::Moods => "Moods & genres",
            Self::NewReleases => "New releases",
            Self::Podcasts => "Podcasts",
            Self::Podcast { .. } => "Podcast",
            Self::MoodResults { .. } => "Moods & genres",
            Self::Stats => "Your stats",
            Self::Wrapped => "Wrapped",
            Self::Recognize => "Recognize song",
        }
    }
}

impl From<Tab> for Route {
    fn from(tab: Tab) -> Self {
        match tab {
            Tab::Home => Self::Home,
            Tab::Search => Self::Search,
            Tab::Together => Self::Together,
            Tab::Library => Self::Library,
        }
    }
}
