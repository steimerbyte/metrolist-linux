//! Podcast show pages (`browse MPSP…`) and episode enrichment (`newEpisodes`
//! gaps). Upstream reaches into singletons for enrichment; here the lookup is a
//! trait so the caller supplies either the real client or a fake (parity
//! `PodcastEnrichmentTest`).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use ml_core::{AlbumRef, Artist, Episode, MlResult, Playlist};
use serde_json::{json, Value};

use crate::context::ClientKind;
use crate::parse::renderers::multi_row_episode;
use crate::parse::{continuation, runs_from, text_of, thumbnail_url};

/// A podcast show page: header plus the first episode page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PodcastPage {
    pub show: Playlist,
    pub episodes: Vec<Episode>,
    pub continuation: Option<String>,
    pub channel_id: Option<String>,
}

fn header(value: &Value) -> Option<&Value> {
    value
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer")
        .or_else(|| {
            value.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer")
        })
}

fn episode_shelf(value: &Value) -> Option<&Value> {
    for path in [
        "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer",
        "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer",
    ] {
        if let Some(shelf) = value.pointer(path) {
            return Some(shelf);
        }
    }
    value
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|content| {
            content
                .get("musicShelfRenderer")
                .or_else(|| content.get("musicPlaylistShelfRenderer"))
        })
}

/// Channel the header offers a SUBSCRIBE toggle for (absent when signed out).
fn subscribe_channel(header: &Value) -> Option<String> {
    header
        .get("buttons")?
        .as_array()?
        .iter()
        .flat_map(|button| button.pointer("/menuRenderer/items").into_iter())
        .flat_map(|items| items.as_array().map(Vec::as_slice).unwrap_or(&[]))
        .find_map(|item| {
            let toggle = item.get("toggleMenuServiceItemRenderer")?;
            if toggle.pointer("/defaultIcon/iconType")?.as_str()? != "SUBSCRIBE" {
                return None;
            }
            toggle
                .pointer("/defaultServiceEndpoint/subscribeEndpoint/channelIds/0")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

/// Show header → [`Playlist`] (`YouTube.podcast` header rules).
pub fn show_from_header(header: &Value, show_id: &str) -> Playlist {
    let author = header.pointer("/straplineTextOne/runs/0").map(|run| {
        Artist::new(
            run.get("text").and_then(Value::as_str).unwrap_or_default(),
            run.pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
                .map(str::to_string),
        )
    });
    Playlist {
        id: show_id.to_string(),
        title: header
            .get("title")
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .unwrap_or_default(),
        author,
        song_count_text: header
            .get("secondSubtitle")
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .filter(|text| !text.is_empty()),
        thumbnail_url: thumbnail_url(header.get("thumbnail")),
        description: header
            .pointer("/description/musicDescriptionShelfRenderer/description")
            .map(runs_from)
            .map(|runs| text_of(&runs))
            .filter(|text| !text.is_empty()),
        is_podcast: true,
        ..Playlist::default()
    }
}

/// Episodes of a show keep the header credits when the row carries none.
pub fn attach_show(episode: &mut Episode, show: &Playlist) {
    if episode.author.is_none() {
        episode.author = show.author.clone();
    }
    if episode.podcast.is_none() {
        episode.podcast = Some(AlbumRef {
            id: show.id.clone(),
            title: show.title.clone(),
        });
    }
}

fn episodes_of(shelf: Option<&Value>, show: &Playlist) -> (Vec<Episode>, Option<String>) {
    let Some(shelf) = shelf else {
        return (Vec::new(), None);
    };
    let episodes = shelf
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|content| content.get("musicMultiRowListItemRenderer"))
        .filter_map(multi_row_episode)
        .map(|mut episode| {
            attach_show(&mut episode, show);
            episode
        })
        .collect();
    (episodes, continuation(shelf.get("continuations")))
}

/// Parse the podcast show browse response.
pub fn from_show_browse(value: &Value, show_id: &str) -> PodcastPage {
    let show = header(value)
        .map(|header| show_from_header(header, show_id))
        .unwrap_or_else(|| Playlist {
            id: show_id.to_string(),
            is_podcast: true,
            ..Playlist::default()
        });
    let (episodes, continuation) = episodes_of(episode_shelf(value), &show);
    PodcastPage {
        channel_id: header(value).and_then(|header| {
            header
                .pointer("/straplineTextOne/runs/0/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| subscribe_channel(header))
        }),
        show,
        episodes,
        continuation,
    }
}

/// Parse `browse {continuation}` for a show's later episodes.
pub fn from_show_continuation(value: &Value) -> (Vec<Episode>, Option<String>) {
    let shelf = value
        .pointer("/continuationContents/musicShelfContinuation")
        .or_else(|| value.pointer("/continuationContents/musicPlaylistShelfContinuation"));
    episodes_of(shelf, &Playlist::default())
}

/// One `next` lookup per unresolved episode.
pub trait EpisodeLookup: Send + Sync {
    fn episode_owner<'a>(
        &'a self,
        video_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = MlResult<Option<Artist>>> + Send + 'a>>;
}

/// Fill missing authors; linked/unresolved/failed episodes stay untouched and
/// each id is queried at most once.
pub async fn enrich_episodes<L: EpisodeLookup + ?Sized>(
    lookup: &L,
    episodes: &mut [Episode],
) -> MlResult<()> {
    let mut cache: HashMap<String, Option<Artist>> = HashMap::new();
    for episode in episodes.iter_mut() {
        if episode.author.is_some() {
            continue;
        }
        let id = episode.id.clone();
        if let Some(cached) = cache.get(&id) {
            if episode.author.is_none() {
                episode.author = cached.clone();
            }
            continue;
        }
        let owner = lookup.episode_owner(&id).await.unwrap_or(None);
        if episode.author.is_none() {
            episode.author = owner.clone();
        }
        cache.insert(id, owner);
    }
    Ok(())
}

impl crate::InnertubeClient {
    /// Resolve an episode's owner via its `next` queue: the first queue artist,
    /// else the watch-next `videoSecondaryInfoRenderer` owner channel.
    pub async fn episode_owner(&self, video_id: &str) -> MlResult<Option<Artist>> {
        let response = self
            .post(
                ClientKind::WebRemix,
                "next",
                json!({
                    "videoId": video_id,
                    "playlistId": format!("RDAMVM{video_id}"),
                    "isAudioOnly": true,
                    "params": "wAEB",
                }),
                false,
                None,
            )
            .await?;
        Ok(super::next::from_response(&response)
            .songs
            .into_iter()
            .find_map(|song| song.artists.into_iter().next())
            .or_else(|| super::next::owner(&response)))
    }
}

/// Convenience for callers with a client in hand.
pub async fn enrich_with_client(
    client: &crate::InnertubeClient,
    episodes: &mut [Episode],
) -> MlResult<()> {
    struct ClientLookup<'a>(&'a crate::InnertubeClient);
    impl EpisodeLookup for ClientLookup<'_> {
        fn episode_owner<'a>(
            &'a self,
            video_id: &'a str,
        ) -> Pin<Box<dyn Future<Output = MlResult<Option<Artist>>> + Send + 'a>> {
            Box::pin(self.0.episode_owner(video_id))
        }
    }
    enrich_episodes(&ClientLookup(client), episodes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        resolved: HashMap<String, Artist>,
        failed: Vec<String>,
        calls: Mutex<Vec<String>>,
    }

    impl EpisodeLookup for Fake {
        fn episode_owner<'a>(
            &'a self,
            video_id: &'a str,
        ) -> Pin<Box<dyn Future<Output = MlResult<Option<Artist>>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(video_id.to_string());
                if self.failed.iter().any(|id| id == video_id) {
                    return Ok(None);
                }
                Ok(self.resolved.get(video_id).cloned())
            })
        }
    }

    fn episode(id: &str) -> Episode {
        Episode {
            id: id.to_string(),
            ..Episode::default()
        }
    }

    #[tokio::test]
    async fn missing_authors_are_resolved_once_and_failures_stay_empty() {
        let fake = Fake {
            resolved: HashMap::from([(
                "missing".to_string(),
                Artist::new("Resolved Host", Some("UChost".into())),
            )]),
            failed: vec!["failed".to_string()],
            calls: Mutex::new(Vec::new()),
        };
        let mut linked = episode("linked");
        linked.author = Some(Artist::new("Linked", Some("UClinked".into())));
        let mut episodes = vec![
            episode("missing"),
            linked,
            episode("unknown"),
            episode("failed"),
            episode("missing"),
        ];
        enrich_episodes(&fake, &mut episodes).await.unwrap();
        assert_eq!(
            episodes[0].author,
            Some(Artist::new("Resolved Host", Some("UChost".into())))
        );
        assert_eq!(
            episodes[1].author,
            Some(Artist::new("Linked", Some("UClinked".into())))
        );
        assert!(episodes[2].author.is_none());
        assert!(episodes[3].author.is_none());
        assert_eq!(episodes[4].author, episodes[0].author);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(&*calls, &["missing", "unknown", "failed"]);
    }
}
