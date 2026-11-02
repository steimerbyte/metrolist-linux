//! Live InnerTube probe: `innertube-probe <home|search|album|playlist|artist|queue|queue-append|charts|moods|new-releases|new-release-albums|podcasts|podcast|browse|more|see-all|stream> [args]`.
//!
//! Prints counts and a sample of parsed fields so a run proves both transport and
//! parsers against the live API. `stream` also performs one ranged GET.

use std::env;
use std::process::ExitCode;

use ml_core::AudioQuality;
use ml_innertube::{InnertubeClient, SearchFilter};
use serde_json::{json, Value};

fn arg(args: &[String], index: usize) -> String {
    args.get(index).cloned().unwrap_or_else(|| usage())
}

fn usage() -> ! {
    eprintln!(
        "usage: innertube-probe home|search <q>|album <browseId>|playlist <id>|artist <channelId>|queue <videoId>|queue-append <videoId> [rounds]|charts|moods|new-releases|new-release-albums|podcasts|podcast <showId>|browse <browseId> [params]|stream <videoId> [--download <path>] [--quality low|high]"
    );
    std::process::exit(2)
}

fn item_summary(item: &ml_core::YtItem) -> Value {
    json!({
        "kind": match item {
            ml_core::YtItem::Song(_) => "song",
            ml_core::YtItem::Album(_) => "album",
            ml_core::YtItem::Artist(_) => "artist",
            ml_core::YtItem::Playlist(_) => "playlist",
            ml_core::YtItem::Episode(_) => "episode",
            ml_core::YtItem::Video(_) => "video",
        },
        "id": item.id(),
        "title": item.title(),
        "artists": match item {
            ml_core::YtItem::Song(song) => song.artists.iter().map(|a| a.name.clone()).collect(),
            ml_core::YtItem::Album(album) => album.artists.iter().map(|a| a.name.clone()).collect(),
            _ => Vec::<String>::new(),
        },
        "duration": match item {
            ml_core::YtItem::Song(song) => song.duration,
            ml_core::YtItem::Video(video) => video.duration,
            ml_core::YtItem::Episode(episode) => episode.duration,
            _ => None,
        },
    })
}

fn print(value: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    );
}

fn page_summary(page: &ml_innertube::pages::browse::BrowsePage) -> Value {
    json!({
        "title": page.title,
        "sections": page.sections.len(),
        "continuation": page.continuation.is_some(),
        "token": page.continuation,
        "section_titles": page.sections.iter().map(|section| json!({
            "title": section.title,
            "kind": format!("{:?}", section.kind),
            "items": section.items.len(),
            "sample": section.items.iter().take(2).map(item_summary).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or_else(|| usage());

    let mut config = ml_innertube::ClientConfig {
        country: "DE".to_string(),
        ..Default::default()
    };
    if let Ok(cookie) = env::var("IT_COOKIE") {
        config.cookie = Some(cookie);
    }
    let client = InnertubeClient::new(config);

    match command {
        "home" => {
            let feed = client.home().await.map_err(|e| e.to_string())?;
            print(json!({
                "sections": feed.sections.len(),
                "continuation": feed.continuation.is_some(),
                "token": feed.continuation,
                "first": feed.sections.iter().map(|s| json!({
                    "title": s.title,
                    "kind": format!("{:?}", s.kind),
                    "items": s.items.len(),
                    "sample": s.items.iter().take(2).map(item_summary).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }));
        }
        "search" => {
            let query = arg(&args, 2);
            let filter = args.get(3).map(|name| match name.as_str() {
                "songs" => SearchFilter::Songs,
                "albums" => SearchFilter::Albums,
                "artists" => SearchFilter::Artists,
                "playlists" => SearchFilter::Playlists,
                "videos" => SearchFilter::Videos,
                _ => usage(),
            });
            let page = client
                .search(&query, filter)
                .await
                .map_err(|e| e.to_string())?;
            let suggestions = client
                .suggestions(&query)
                .await
                .map_err(|e| e.to_string())?;
            print(json!({
                "query": page.query,
                "items": page.items.len(),
                "songs": page.songs.len(),
                "continuation": page.continuation.is_some(),
                "token": page.continuation,
                "suggestions": suggestions,
                "sample": page.items.iter().take(5).map(item_summary).collect::<Vec<_>>(),
            }));
        }
        "album" => {
            let browse_id = arg(&args, 2);
            let page = client.album(&browse_id).await.map_err(|e| e.to_string())?;
            print(json!({
                "album": {
                    "id": page.album.id,
                    "playlist_id": page.album.playlist_id,
                    "title": page.album.title,
                    "artists": page.album.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                    "year": page.album.year,
                    "songs": page.album.song_count,
                    "duration": page.album.duration,
                    "thumbnail": page.album.thumbnail_url.is_some(),
                },
                "songs": page.songs.len(),
                "continuation": page.continuation.is_some(),
                "token": page.continuation,
                "sample": page.songs.iter().take(3).map(|song| json!({
                    "id": song.id,
                    "title": song.title,
                    "artists": song.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                    "duration": song.duration,
                    "album": song.album.as_ref().map(|a| a.id.clone()),
                    "set_video_id": song.set_video_id.is_some(),
                })).collect::<Vec<_>>(),
            }));
        }
        "playlist" => {
            let id = arg(&args, 2);
            let page = client.playlist(&id).await.map_err(|e| e.to_string())?;
            print(json!({
                "playlist": {
                    "id": page.playlist.id,
                    "title": page.playlist.title,
                    "author": page.playlist.author.as_ref().map(|a| a.name.clone()),
                    "song_count_text": page.playlist.song_count_text,
                    "editable": page.playlist.is_editable,
                },
                "songs": page.songs.len(),
                "continuation": page.continuation.is_some(),
                "token": page.continuation,
                "sample": page.songs.iter().take(3).map(|song| json!({
                    "id": song.id,
                    "title": song.title,
                    "artists": song.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                    "duration": song.duration,
                    "set_video_id": song.set_video_id.is_some(),
                })).collect::<Vec<_>>(),
            }));
        }
        "artist" => {
            let channel = arg(&args, 2);
            let details = client
                .artist_details(&channel)
                .await
                .map_err(|e| e.to_string())?;
            print(json!({
                "artist": {
                    "id": details.page.artist.id,
                    "name": details.page.artist.name,
                    "subscribers": details.page.artist.subscribers,
                    "monthly_listeners": details.extras.monthly_listeners,
                },
                "top_songs": details.page.top_songs.len(),
                "sections": details.page.sections.iter().map(|s| json!({
                    "title": s.title,
                    "items": s.items.len(),
                })).collect::<Vec<_>>(),
                "see_all": details.extras.see_all.iter().map(|see| json!({
                    "title": see.title,
                    "browse_id": see.browse_id,
                    "params": see.params,
                })).collect::<Vec<_>>(),
                "description": details.page.description.is_some(),
                "continuation": details.page.continuation.is_some(),
                "token": details.page.continuation,
            }));
        }
        "queue" => {
            let video_id = arg(&args, 2);
            let page = client
                .queue(&video_id, None)
                .await
                .map_err(|e| e.to_string())?;
            print(json!({
                "title": page.title,
                "songs": page.songs.len(),
                "current_index": page.current_index,
                "continuation": page.continuation.is_some(),
                "token": page.continuation,
                "sample": page.songs.iter().take(3).map(|song| json!({
                    "id": song.id,
                    "title": song.title,
                    "artists": song.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                    "duration": song.duration,
                })).collect::<Vec<_>>(),
            }));
        }
        "queue-append" => {
            let video_id = arg(&args, 2);
            let rounds = args
                .get(3)
                .and_then(|rounds| rounds.parse::<usize>().ok())
                .unwrap_or(1);
            let mut page = client
                .queue(&video_id, None)
                .await
                .map_err(|e| e.to_string())?;
            let initial = page.songs.len();
            let current_index = page.current_index;
            let mut added = 0;
            for _ in 0..rounds {
                let appended = client
                    .queue_append(&mut page)
                    .await
                    .map_err(|e| e.to_string())?;
                added += appended;
                if appended == 0 || page.continuation.is_none() {
                    break;
                }
            }
            print(json!({
                "initial": initial,
                "added": added,
                "total": page.songs.len(),
                "current_index_before": current_index,
                "current_index_after": page.current_index,
                "continuation": page.continuation.is_some(),
                "first": page.songs.first().map(|song| song.id.clone()),
                "last": page.songs.last().map(|song| song.id.clone()),
            }));
        }
        "more" => {
            let kind = arg(&args, 2);
            let token = arg(&args, 3);
            match kind.as_str() {
                "home" => {
                    let feed = client
                        .home_continuation(&token)
                        .await
                        .map_err(|e| e.to_string())?;
                    print(
                        json!({ "sections": feed.sections.len(), "continuation": feed.continuation.is_some() }),
                    );
                }
                "search" => {
                    let page = client
                        .search_continuation(&token)
                        .await
                        .map_err(|e| e.to_string())?;
                    print(
                        json!({ "items": page.items.len(), "songs": page.songs.len(), "continuation": page.continuation.is_some() }),
                    );
                }
                "album" => {
                    let page = client
                        .album_continuation(&token)
                        .await
                        .map_err(|e| e.to_string())?;
                    print(
                        json!({ "songs": page.songs.len(), "continuation": page.continuation.is_some() }),
                    );
                }
                "artist" => {
                    let feed = client
                        .artist_continuation(&token)
                        .await
                        .map_err(|e| e.to_string())?;
                    print(
                        json!({ "sections": feed.sections.len(), "continuation": feed.continuation.is_some() }),
                    );
                }
                "playlist" => {
                    let page = client
                        .playlist_continuation(&token)
                        .await
                        .map_err(|e| e.to_string())?;
                    print(
                        json!({ "songs": page.songs.len(), "continuation": page.continuation.is_some() }),
                    );
                }
                "queue" => {
                    let page = client
                        .queue_continuation(&token)
                        .await
                        .map_err(|e| e.to_string())?;
                    print(
                        json!({ "songs": page.songs.len(), "continuation": page.continuation.is_some() }),
                    );
                }
                "page" => {
                    let page = client
                        .browse_continuation(&token)
                        .await
                        .map_err(|e| e.to_string())?;
                    print(page_summary(&page));
                }
                _ => usage(),
            }
        }
        "see-all" => {
            let browse_id = arg(&args, 2);
            let params = args.get(3).map(String::as_str);
            let feed = client
                .artist_items(&browse_id, params)
                .await
                .map_err(|e| e.to_string())?;
            print(json!({
                "sections": feed.sections.len(),
                "items": feed.sections.first().map(|s| s.items.len()).unwrap_or(0),
                "continuation": feed.continuation.is_some(),
                "sample": feed.sections.first().map(|s| s.items.iter().take(3).map(item_summary).collect::<Vec<_>>()).unwrap_or_default(),
            }));
        }
        "charts" => {
            let page = client.charts().await.map_err(|e| e.to_string())?;
            print(page_summary(&page));
        }
        "moods" => {
            let page = client.moods_and_genres().await.map_err(|e| e.to_string())?;
            print(json!({
                "groups": page.groups.len(),
                "sections": page.groups.iter().map(|group| json!({
                    "title": group.title,
                    "items": group.items.len(),
                    "sample": group.items.iter().take(3).map(|item| json!({
                        "title": item.title,
                        "browse_id": item.browse_id,
                        "params": item.params,
                        "stripe_color": item.stripe_color,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }));
        }
        "new-releases" => {
            let page = client.new_releases().await.map_err(|e| e.to_string())?;
            print(page_summary(&page));
        }
        "new-release-albums" => {
            let albums = client
                .new_release_albums()
                .await
                .map_err(|e| e.to_string())?;
            print(json!({
                "albums": albums.len(),
                "sample": albums.iter().take(3).map(|album| json!({
                    "id": album.id,
                    "playlist_id": album.playlist_id,
                    "title": album.title,
                    "artists": album.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }));
        }
        "podcasts" => {
            let feed = client.podcasts().await.map_err(|e| e.to_string())?;
            print(json!({
                "sections": feed.sections.len(),
                "continuation": feed.continuation.is_some(),
                "section_titles": feed.sections.iter().map(|section| json!({
                    "title": section.title,
                    "items": section.items.len(),
                    "sample": section.items.iter().take(2).map(item_summary).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }));
        }
        "podcast" => {
            let show_id = arg(&args, 2);
            let page = client.podcast(&show_id).await.map_err(|e| e.to_string())?;
            print(json!({
                "show": {
                    "id": page.show.id,
                    "title": page.show.title,
                    "author": page.show.author.as_ref().map(|a| a.name.clone()),
                    "author_id": page.show.author.as_ref().and_then(|a| a.id.clone()),
                    "episode_count_text": page.show.song_count_text,
                    "description": page.show.description.is_some(),
                    "channel_id": page.channel_id,
                },
                "episodes": page.episodes.len(),
                "continuation": page.continuation.is_some(),
                "token": page.continuation,
                "sample": page.episodes.iter().take(2).map(|episode| json!({
                    "id": episode.id,
                    "title": episode.title,
                    "author": episode.author.as_ref().map(|a| a.name.clone()),
                    "podcast": episode.podcast.as_ref().map(|p| p.id.clone()),
                    "date": episode.publish_date_text,
                })).collect::<Vec<_>>(),
            }));
        }
        "browse" => {
            let browse_id = arg(&args, 2);
            let params = args.get(3).map(String::as_str);
            let page = client
                .browse(&browse_id, params)
                .await
                .map_err(|e| e.to_string())?;
            print(page_summary(&page));
        }
        "stream" => {
            let video_id = arg(&args, 2);
            let quality = match args
                .iter()
                .position(|arg| arg == "--quality")
                .and_then(|index| args.get(index + 1))
                .map(String::as_str)
            {
                Some("low") => AudioQuality::Low,
                Some("high") => AudioQuality::High,
                _ => AudioQuality::Auto,
            };
            let media = client
                .media_info(&video_id)
                .await
                .map_err(|e| e.to_string())?;
            let info = client
                .stream_info(&video_id, quality)
                .await
                .map_err(|e| e.to_string())?;
            let probe = client
                .http()
                .get(&info.url)
                .header(reqwest::header::RANGE, "bytes=0-262143")
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = probe.status();
            let content_range = probe
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let bytes = probe.bytes().await.map_err(|e| e.to_string())?.len();
            print(json!({
                "media": {
                    "title": media.title,
                    "author": media.author,
                },
                "stream": {
                    "itag": info.itag,
                    "mime": info.mime,
                    "bitrate": info.bitrate,
                    "content_length": info.content_length,
                    "expires_at": info.expires_at,
                    "url_len": info.url.len(),
                },
                "range_probe": {
                    "status": status.as_u16(),
                    "content_range": content_range,
                    "bytes": bytes,
                },
            }));
            if let Some(index) = args.iter().position(|arg| arg == "--download") {
                let path = arg(&args, index + 1);
                let mut last = 0u64;
                match client
                    .download_audio(&video_id, quality, &path, |done, total| {
                        if done == 0 || done >= last + 262_144 {
                            last = done;
                            eprintln!("progress {done}/{total}");
                        }
                    })
                    .await
                {
                    Ok(()) => print(json!({ "downloaded": path })),
                    Err(error) => return Err(format!("download failed: {error}")),
                }
            }
        }
        other => {
            eprintln!("unknown command: {other}");
            usage();
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
