//! Listening statistics and the Wrapped story.

use eframe::egui::{self, Align2, Pos2, Ui, Vec2};
use ml_core::{Album, AlbumRef, Artist, ArtistItem, Song};
use ml_store::{
    AlbumStat, ArtistStat, MonthPlays, SongStat, StatsPeriod, StatsReport, StatsSummary,
};

use crate::art::ArtCache;
use crate::pure::{
    listening_time_text, month_short, stats_period_label, stats_summary_cards, wrapped_summary,
};
use crate::routes::Route;
use crate::services::Services;
use crate::state::{Load, StatsState};
use crate::theme::{
    bold_font, medium_font, regular_font, state_layer, Theme, DP12, DP16, DP24, DP4, DP8,
    TITLE_LARGE,
};
use crate::widgets::{art_or_placeholder, empty_state, filled_button, section_header, text};

use super::{error_block, skeleton_cards, skeleton_rows, Intent};

const PERIODS: [StatsPeriod; 4] = [
    StatsPeriod::Week,
    StatsPeriod::Month,
    StatsPeriod::Year,
    StatsPeriod::All,
];

fn period_chips(ui: &mut Ui, theme: &Theme, state: &StatsState, intents: &mut Vec<Intent>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        for period in PERIODS {
            let selected = period == state.period;
            if crate::widgets::chip(ui, theme, stats_period_label(period), selected).clicked()
                && !selected
            {
                intents.push(Intent::StatsPeriod(period));
            }
        }
    });
}

fn summary_cards(ui: &mut Ui, theme: &Theme, summary: &StatsSummary, width: f32) {
    let cards = stats_summary_cards(summary);
    let columns = if width >= 840.0 { 4 } else { 2 };
    let gap = DP8;
    let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(96.0);
    for chunk in cards.chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value) in chunk {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(card_width, 76.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 12, theme.scheme.surface_container);
                crate::widgets::elided(
                    ui.painter(),
                    rect.left_top() + Vec2::new(DP12, DP12),
                    label,
                    regular_font(11.0),
                    theme.scheme.on_surface_variant,
                    card_width - DP8,
                );
                crate::widgets::elided(
                    ui.painter(),
                    rect.left_top() + Vec2::new(DP12, 35.0),
                    value,
                    medium_font(20.0),
                    theme.scheme.on_surface,
                    card_width - DP8,
                );
            }
        });
        ui.add_space(gap);
    }
}

fn last_months(months: &[MonthPlays]) -> Vec<&MonthPlays> {
    months.iter().rev().take(12).rev().collect()
}

fn monthly_summary(months: &[&MonthPlays]) -> String {
    if months.is_empty() {
        "No monthly plays".to_string()
    } else {
        months
            .iter()
            .map(|month| {
                format!(
                    "{} {}: {}",
                    month_short(month.month),
                    month.year,
                    month.plays
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

fn monthly_chart(ui: &mut Ui, theme: &Theme, monthly: &[MonthPlays], width: f32) {
    let months = last_months(monthly);
    ui.label(
        egui::RichText::new("Plays by month")
            .font(medium_font(14.0))
            .color(theme.scheme.on_surface),
    );
    if months.is_empty() {
        ui.label(
            egui::RichText::new("No monthly plays yet")
                .font(regular_font(12.0))
                .color(theme.scheme.on_surface_variant),
        );
        return;
    }
    let accessible = monthly_summary(&months);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 150.0), egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Label,
            true,
            format!("Monthly plays: {accessible}"),
        )
    });
    let plot = egui::Rect::from_min_max(
        Pos2::new(rect.left() + DP8, rect.top() + 16.0),
        Pos2::new(rect.right() - DP8, rect.bottom() - 25.0),
    );
    ui.painter()
        .rect_filled(plot, 4, theme.scheme.surface_container_low);
    ui.painter().line_segment(
        [plot.left_bottom(), plot.right_bottom()],
        egui::Stroke::new(1.0, theme.scheme.outline_variant),
    );
    let max = months.iter().map(|month| month.plays).max().unwrap_or(0);
    let gap = 4.0;
    let bar_width = ((plot.width() - gap * (months.len().saturating_sub(1)) as f32)
        / months.len() as f32)
        .max(3.0);
    for (index, month) in months.iter().enumerate() {
        let height = if max == 0 {
            0.0
        } else {
            (month.plays as f32 / max as f32) * (plot.height() - 4.0)
        };
        let x = plot.left() + index as f32 * (bar_width + gap);
        let bar = egui::Rect::from_min_max(
            Pos2::new(x, plot.bottom() - height),
            Pos2::new(x + bar_width, plot.bottom()),
        );
        ui.painter().rect_filled(bar, 2, theme.scheme.primary);
        ui.painter().text(
            Pos2::new(x + bar_width / 2.0, plot.bottom() + 4.0),
            Align2::CENTER_TOP,
            month_short(month.month),
            regular_font(11.0),
            theme.scheme.on_surface_variant,
        );
        if month.plays > 0 {
            ui.painter().text(
                Pos2::new(x + bar_width / 2.0, bar.top() - 2.0),
                Align2::CENTER_BOTTOM,
                month.plays.to_string(),
                regular_font(11.0),
                theme.scheme.on_surface_variant,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stat_song_row(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    rank: usize,
    stat: &SongStat,
    current: Option<&Song>,
    width: f32,
) -> bool {
    let active = current.is_some_and(|song| song.id == stat.song.id);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 64.0), egui::Sense::click());
    if active || response.hovered() {
        let fill = if active {
            theme.scheme.secondary_container
        } else {
            state_layer(
                theme.scheme.surface_container_high,
                response.hovered(),
                response.is_pointer_button_down_on(),
            )
        };
        ui.painter().rect_filled(rect, 8, fill);
    }
    let art_rect =
        egui::Rect::from_min_size(rect.left_top() + Vec2::new(30.0, 10.0), Vec2::splat(44.0));
    let texture = art.texture(ui.ctx(), services, &stat.song.thumb(160));
    art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 4);
    text(
        ui.painter(),
        Pos2::new(rect.left() + DP8, rect.top() + 20.0),
        &rank.to_string(),
        medium_font(13.0),
        theme.scheme.on_surface_variant,
    );
    let text_x = art_rect.right() + DP8;
    let right = rect.right() - DP8;
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, rect.top() + 10.0),
        &stat.song.title,
        medium_font(14.0),
        theme.scheme.on_surface,
        (right - 82.0 - text_x).max(20.0),
    );
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, rect.top() + 31.0),
        &stat.song.artist_names(),
        regular_font(12.0),
        theme.scheme.on_surface_variant,
        (right - 82.0 - text_x).max(20.0),
    );
    ui.painter().text(
        Pos2::new(right, rect.top() + 12.0),
        Align2::RIGHT_TOP,
        format!("{} plays", stat.plays),
        medium_font(12.0),
        theme.scheme.on_surface,
    );
    ui.painter().text(
        Pos2::new(right, rect.top() + 33.0),
        Align2::RIGHT_TOP,
        listening_time_text(stat.ms),
        regular_font(11.0),
        theme.scheme.on_surface_variant,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            format!("{} — {} plays", stat.song.title, stat.plays),
        )
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

#[allow(clippy::too_many_arguments)]
fn stat_media_row(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    rank: usize,
    title: &str,
    subtitle: &str,
    thumbnail_url: Option<&str>,
    plays: u64,
    ms: i64,
    width: f32,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 60.0), egui::Sense::click());
    if response.hovered() {
        let fill = state_layer(
            theme.scheme.surface_container_high,
            response.hovered(),
            response.is_pointer_button_down_on(),
        );
        ui.painter().rect_filled(rect, 8, fill);
    }
    let art_rect =
        egui::Rect::from_min_size(rect.left_top() + Vec2::new(30.0, 8.0), Vec2::splat(44.0));
    let url = thumbnail_url
        .map(|url| ml_core::resize_thumbnail_url(url, 160))
        .unwrap_or_default();
    let texture = art.texture(ui.ctx(), services, &url);
    art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 4);
    text(
        ui.painter(),
        Pos2::new(rect.left() + DP8, rect.top() + 19.0),
        &rank.to_string(),
        medium_font(13.0),
        theme.scheme.on_surface_variant,
    );
    let text_x = art_rect.right() + DP8;
    let right = rect.right() - DP8;
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, rect.top() + 9.0),
        title,
        medium_font(14.0),
        theme.scheme.on_surface,
        (right - 82.0 - text_x).max(20.0),
    );
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, rect.top() + 30.0),
        subtitle,
        regular_font(12.0),
        theme.scheme.on_surface_variant,
        (right - 82.0 - text_x).max(20.0),
    );
    ui.painter().text(
        Pos2::new(right, rect.top() + 11.0),
        Align2::RIGHT_TOP,
        format!("{plays} plays"),
        medium_font(12.0),
        theme.scheme.on_surface,
    );
    ui.painter().text(
        Pos2::new(right, rect.top() + 32.0),
        Align2::RIGHT_TOP,
        listening_time_text(ms),
        regular_font(11.0),
        theme.scheme.on_surface_variant,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{title} — {plays} plays"))
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

fn top_songs(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    report: &StatsReport,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if report.top_songs.is_empty() {
        return intents;
    }
    section_header(
        ui,
        theme,
        &format!("Top songs · {}", report.top_songs.len()),
        width,
        false,
    );
    let songs: Vec<Song> = report
        .top_songs
        .iter()
        .map(|stat| stat.song.clone())
        .collect();
    for (index, stat) in report.top_songs.iter().enumerate() {
        if stat_song_row(ui, theme, services, art, index + 1, stat, current, width) {
            intents.push(Intent::PlayCatalog {
                songs: songs.clone(),
                index,
            });
        }
    }
    intents
}

fn top_artists(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    report: &StatsReport,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if report.top_artists.is_empty() {
        return intents;
    }
    section_header(
        ui,
        theme,
        &format!("Top artists · {}", report.top_artists.len()),
        width,
        false,
    );
    for (index, stat) in report.top_artists.iter().enumerate() {
        let subtitle = stat.artist.subscribers.as_deref().unwrap_or("Artist");
        if stat_media_row(
            ui,
            theme,
            services,
            art,
            index + 1,
            &stat.artist.name,
            subtitle,
            stat.artist.thumbnail_url.as_deref(),
            stat.plays,
            stat.ms,
            width,
        ) {
            intents.push(Intent::Open(Route::Artist {
                id: stat
                    .artist
                    .channel_id
                    .clone()
                    .unwrap_or_else(|| stat.artist.id.clone()),
            }));
        }
    }
    intents
}

fn top_albums(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    report: &StatsReport,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if report.top_albums.is_empty() {
        return intents;
    }
    section_header(
        ui,
        theme,
        &format!("Top albums · {}", report.top_albums.len()),
        width,
        false,
    );
    for (index, stat) in report.top_albums.iter().enumerate() {
        if stat_media_row(
            ui,
            theme,
            services,
            art,
            index + 1,
            &stat.album.title,
            &stat.album.artist_names(),
            stat.album.thumbnail_url.as_deref(),
            stat.plays,
            stat.ms,
            width,
        ) {
            intents.push(Intent::Open(Route::Album {
                id: stat.album.id.clone(),
            }));
        }
    }
    intents
}

fn ready_report<'a>(
    ui: &mut Ui,
    theme: &Theme,
    load: &'a Load<StatsReport>,
    width: f32,
    intents: &mut Vec<Intent>,
) -> Option<&'a StatsReport> {
    if load.initial_loading() {
        skeleton_cards(ui, theme, width);
        ui.add_space(DP8);
        skeleton_rows(ui, theme, width);
        ui.add_space(DP24);
        return None;
    }
    let Some(report) = load.data.as_ref() else {
        if let Some(error) = load.error.as_deref() {
            if error_block(ui, theme, error) {
                intents.push(Intent::Refresh);
            }
        } else {
            empty_stats(ui, theme, intents);
        }
        return None;
    };
    if report.summary.event_count == 0 {
        empty_stats(ui, theme, intents);
        return None;
    }
    Some(report)
}

fn empty_stats(ui: &mut Ui, theme: &Theme, intents: &mut Vec<Intent>) {
    empty_state(
        ui,
        theme,
        "bar_chart",
        "No listening history yet",
        "Play a song for a while, then come back to see your stats.",
    );
    ui.add_space(DP8);
    ui.vertical_centered(|ui| {
        if filled_button(ui, theme, Some("home"), "Go to Home").clicked() {
            intents.push(Intent::Open(Route::Home));
        }
    });
    ui.add_space(DP24);
}

pub fn show(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut StatsState,
    current: Option<&Song>,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.add_space(DP4);
    period_chips(ui, theme, state, &mut intents);
    ui.add_space(DP12);
    let Some(report) = ready_report(ui, theme, &state.load, width, &mut intents) else {
        return intents;
    };
    summary_cards(ui, theme, &report.summary, width);
    ui.add_space(DP8);
    monthly_chart(ui, theme, &report.monthly, width);
    ui.add_space(DP12);
    intents.extend(top_songs(ui, theme, services, art, report, current, width));
    intents.extend(top_artists(ui, theme, services, art, report, width));
    intents.extend(top_albums(ui, theme, services, art, report, width));
    ui.add_space(DP24);
    intents
}

#[allow(clippy::too_many_arguments)]
fn feature(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    title: &str,
    subtitle: &str,
    art_url: Option<&str>,
    plays: u64,
    ms: i64,
    width: f32,
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 128.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 16, theme.scheme.surface_container_high);
    let art_rect =
        egui::Rect::from_min_size(rect.left_top() + Vec2::new(DP16, DP16), Vec2::splat(96.0));
    let url = art_url
        .map(|url| ml_core::resize_thumbnail_url(url, 320))
        .unwrap_or_default();
    let texture = art.texture(ui.ctx(), services, &url);
    art_or_placeholder(ui, theme, texture.as_ref(), art_rect, 8);
    let text_x = art_rect.right() + DP16;
    let max_width = (rect.right() - DP16 - text_x).max(40.0);
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, rect.top() + 24.0),
        title,
        bold_font(22.0),
        theme.scheme.on_surface,
        max_width,
    );
    crate::widgets::elided(
        ui.painter(),
        Pos2::new(text_x, rect.top() + 55.0),
        subtitle,
        regular_font(13.0),
        theme.scheme.on_surface_variant,
        max_width,
    );
    ui.painter().text(
        Pos2::new(text_x, rect.top() + 78.0),
        Align2::LEFT_TOP,
        format!("{plays} plays · {}", listening_time_text(ms)),
        medium_font(12.0),
        theme.scheme.primary,
    );
    rect
}

fn story_card(ui: &mut Ui, theme: &Theme, label: &str, value: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 88.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 16, theme.scheme.primary_container);
    ui.painter().text(
        rect.left_top() + Vec2::new(DP16, DP16),
        Align2::LEFT_TOP,
        label,
        regular_font(12.0),
        theme.scheme.on_primary_container,
    );
    ui.painter().text(
        rect.left_top() + Vec2::new(DP16, 38.0),
        Align2::LEFT_TOP,
        value,
        bold_font(28.0),
        theme.scheme.on_primary_container,
    );
}

pub fn wrapped(
    ui: &mut Ui,
    theme: &Theme,
    services: &Services,
    art: &mut ArtCache,
    state: &mut StatsState,
    width: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.add_space(DP4);
    period_chips(ui, theme, state, &mut intents);
    ui.add_space(DP12);
    let Some(report) = ready_report(ui, theme, &state.load, width, &mut intents) else {
        return intents;
    };
    ui.label(
        egui::RichText::new(format!(
            "Your {} wrapped",
            stats_period_label(state.period).to_lowercase()
        ))
        .font(bold_font(TITLE_LARGE))
        .color(theme.scheme.on_surface),
    );
    ui.label(
        egui::RichText::new("A small story made from your listening history.")
            .font(regular_font(13.0))
            .color(theme.scheme.on_surface_variant),
    );
    ui.add_space(DP12);
    story_card(
        ui,
        theme,
        "You listened for",
        &listening_time_text(report.summary.total_ms),
        width,
    );
    ui.add_space(DP8);
    let columns = if width >= 840.0 { 3 } else { 1 };
    let gap = DP8;
    let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(100.0);
    for chunk in [
        ("Plays", report.summary.event_count.to_string()),
        (
            "Longest streak",
            format!("{} days", report.summary.longest_streak_days),
        ),
        ("Unique artists", report.summary.unique_artists.to_string()),
    ]
    .chunks(columns)
    {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value) in chunk {
                story_card(ui, theme, label, value, card_width);
            }
        });
        ui.add_space(gap);
    }
    if let Some(top) = report.top_songs.first() {
        ui.add_space(DP12);
        ui.label(
            egui::RichText::new("Top song")
                .font(medium_font(14.0))
                .color(theme.scheme.on_surface),
        );
        feature(
            ui,
            theme,
            services,
            art,
            &top.song.title,
            &top.song.artist_names(),
            top.song.thumbnail_url.as_deref(),
            top.plays,
            top.ms,
            width,
        );
    }
    if let Some(top) = report.top_artists.first() {
        ui.add_space(DP12);
        ui.label(
            egui::RichText::new("Top artist")
                .font(medium_font(14.0))
                .color(theme.scheme.on_surface),
        );
        feature(
            ui,
            theme,
            services,
            art,
            &top.artist.name,
            "Artist",
            top.artist.thumbnail_url.as_deref(),
            top.plays,
            top.ms,
            width,
        );
    }
    if let Some(top) = report.top_albums.first() {
        ui.add_space(DP12);
        ui.label(
            egui::RichText::new("Top album")
                .font(medium_font(14.0))
                .color(theme.scheme.on_surface),
        );
        feature(
            ui,
            theme,
            services,
            art,
            &top.album.title,
            &top.album.artist_names(),
            top.album.thumbnail_url.as_deref(),
            top.plays,
            top.ms,
            width,
        );
    }
    ui.add_space(DP16);
    monthly_chart(ui, theme, &report.monthly, width);
    ui.add_space(DP12);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = DP8;
        if filled_button(ui, theme, Some("content_copy"), "Copy summary").clicked() {
            ui.ctx().copy_text(wrapped_summary(report, state.period));
            state.copied = true;
        }
        if state.copied {
            ui.label(
                egui::RichText::new("Copied to clipboard")
                    .font(regular_font(12.0))
                    .color(theme.scheme.primary),
            );
        }
    });
    ui.add_space(DP24);
    intents
}

/// Deterministic screenshot fixture. Not used by normal navigation.
pub fn demo_report() -> StatsReport {
    let song = Song {
        id: "demo_stats_song".to_string(),
        title: "Demo track".to_string(),
        artists: vec![Artist::new("Demo artist", None)],
        album: Some(AlbumRef {
            id: "demo_stats_album".to_string(),
            title: "Demo album".to_string(),
        }),
        duration: Some(214),
        thumbnail_url: None,
        ..Song::default()
    };
    let artist = ArtistItem {
        id: "UCdemo_stats".to_string(),
        name: "Demo artist".to_string(),
        thumbnail_url: None,
        channel_id: Some("UCdemo_stats".to_string()),
        subscribers: Some("1.2M subscribers".to_string()),
    };
    let album = Album {
        id: "demo_stats_album".to_string(),
        playlist_id: "OLAKdemo_stats".to_string(),
        title: "Demo album".to_string(),
        artists: vec![Artist::new("Demo artist", None)],
        year: Some(2026),
        thumbnail_url: None,
        explicit: false,
        song_count: Some(10),
        duration: Some(2100),
    };
    let now = 1_782_000_000_000i64;
    StatsReport {
        summary: StatsSummary {
            range: StatsPeriod::Month.range(now),
            total_ms: 15_120_000,
            event_count: 48,
            unique_songs: 12,
            unique_artists: 5,
            unique_albums: 6,
            first_play_ms: Some(now - 12 * 86_400_000),
            last_play_ms: Some(now - 3_600_000),
            longest_streak_days: 4,
        },
        monthly: vec![
            MonthPlays {
                year: 2025,
                month: 12,
                plays: 18,
                songs: 8,
            },
            MonthPlays {
                year: 2026,
                month: 1,
                plays: 24,
                songs: 10,
            },
            MonthPlays {
                year: 2026,
                month: 2,
                plays: 31,
                songs: 11,
            },
            MonthPlays {
                year: 2026,
                month: 3,
                plays: 17,
                songs: 7,
            },
            MonthPlays {
                year: 2026,
                month: 4,
                plays: 42,
                songs: 12,
            },
            MonthPlays {
                year: 2026,
                month: 5,
                plays: 38,
                songs: 10,
            },
            MonthPlays {
                year: 2026,
                month: 6,
                plays: 29,
                songs: 9,
            },
            MonthPlays {
                year: 2026,
                month: 7,
                plays: 46,
                songs: 13,
            },
            MonthPlays {
                year: 2026,
                month: 8,
                plays: 51,
                songs: 14,
            },
            MonthPlays {
                year: 2026,
                month: 9,
                plays: 33,
                songs: 11,
            },
            MonthPlays {
                year: 2026,
                month: 10,
                plays: 48,
                songs: 12,
            },
            MonthPlays {
                year: 2026,
                month: 11,
                plays: 12,
                songs: 5,
            },
        ],
        top_songs: vec![SongStat {
            song,
            plays: 9,
            ms: 1_926_000,
        }],
        top_artists: vec![ArtistStat {
            artist,
            plays: 20,
            ms: 4_280_000,
        }],
        top_albums: vec![AlbumStat {
            album,
            plays: 12,
            ms: 2_520_000,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_store::SettingsPaths;
    use crate::theme::Theme;
    use std::path::PathBuf;

    fn services() -> (Services, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "metrolist-stats-render-{}-{}",
            std::process::id(),
            ml_core::now_millis()
        ));
        let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
        let ctx = egui::Context::default();
        let (services, _notes) = Services::new(&ctx, paths, &Default::default());
        (services, root)
    }

    fn render_at(
        width: f32,
        height: f32,
        mut draw: impl FnMut(&mut Ui, &mut ArtCache) -> Vec<Intent>,
    ) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, height),
            )),
            ..Default::default()
        };
        let mut art = ArtCache::new();
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    actions = draw(ui, &mut art);
                });
            });
        });
        output.textures_delta.clear();
        assert!(actions.is_empty(), "unexpected stats action at {width}");
    }

    #[test]
    fn stats_and_wrapped_render_at_both_sizes() {
        let (services, root) = services();
        let theme = Theme::from_settings(&Default::default());
        let report = demo_report();
        for (width, height) in [(405.0, 900.0), (1100.0, 760.0)] {
            let mut stats_state = StatsState {
                load: Load {
                    data: Some(report.clone()),
                    ..Load::default()
                },
                ..StatsState::default()
            };
            render_at(width, height, |ui, art| {
                show(ui, &theme, &services, art, &mut stats_state, None, width)
            });
            let mut wrapped_state = StatsState {
                load: Load {
                    data: Some(report.clone()),
                    ..Load::default()
                },
                ..StatsState::default()
            };
            render_at(width, height, |ui, art| {
                wrapped(ui, &theme, &services, art, &mut wrapped_state, width)
            });
        }
        drop(services);
        let _ = std::fs::remove_dir_all(root);
    }
}
