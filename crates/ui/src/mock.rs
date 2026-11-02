//! Deterministic mock content so the shell runs with no network or database.
//!
//! Artwork is generated procedurally at startup; replace `MockData` with the
//! real repositories once `ml-innertube` and `ml-store` land.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, Vec2};
use ml_core::{Album, Artist, HomeSection, SectionKind, Song, YtItem};

pub const HOME_FILTERS: [&str; 5] = ["Energize", "Workout", "Relax", "Feel good", "Commute"];

pub struct MockData {
    pub quick_picks: Vec<Song>,
    pub search_songs: Vec<Song>,
    pub home: Vec<HomeSection>,
    art: Vec<TextureHandle>,
}

impl MockData {
    pub fn new(ctx: &egui::Context) -> Self {
        let art: Vec<_> = (0..12).map(|index| make_art(ctx, index)).collect();
        let quick_picks = songs();
        let keep = keep_listening_albums();
        let new = new_release_albums();

        let home = vec![
            HomeSection {
                title: "Quick picks".to_string(),
                items: quick_picks.iter().cloned().map(YtItem::Song).collect(),
                continuation: None,
                kind: SectionKind::QuickPicks,
            },
            HomeSection {
                title: "Keep listening".to_string(),
                items: keep.into_iter().map(YtItem::Album).collect(),
                continuation: None,
                kind: SectionKind::KeepListening,
            },
            HomeSection {
                title: "New releases".to_string(),
                items: new.into_iter().map(YtItem::Album).collect(),
                continuation: None,
                kind: SectionKind::NewReleases,
            },
        ];

        Self {
            search_songs: quick_picks.clone(),
            quick_picks,
            home,
            art,
        }
    }

    pub fn texture(&self, id: &str) -> &TextureHandle {
        &self.art[hash_id(id) % self.art.len()]
    }

    pub fn section(&self, kind: SectionKind) -> Option<&HomeSection> {
        self.home.iter().find(|section| section.kind == kind)
    }
}

fn songs() -> Vec<Song> {
    [
        ("demo-peace1", "Peace Salam", "Abdulrahman Mohammed", 257),
        ("demo-where1", "Wherever You Go", "Beach House", 207),
        ("demo-siri01", "SIRI", "Shayhn", 214),
        ("demo-prayer", "A Prayer", "Faraj Suleiman", 241),
        ("demo-maly01", "Malyoun", "Jadal", 340),
        ("demo-qamar1", "Qamar", "Hamza Namira", 252),
        ("demo-night1", "Nights", "Nujoom", 238),
        ("demo-solst1", "Solstice", "Arooj Aftab", 362),
    ]
    .into_iter()
    .map(|(id, title, artist, duration)| Song {
        id: id.to_string(),
        title: title.to_string(),
        artists: vec![Artist::new(artist, None)],
        duration: Some(duration),
        ..Song::default()
    })
    .collect()
}

fn keep_listening_albums() -> Vec<Album> {
    [
        ("demo-album1", "Cairokeen", "Cairokee", 29),
        ("demo-album2", "Malyoun", "Jadal", 12),
        ("demo-album3", "Qarar Shakhs", "Hamza Namira", 14),
        ("demo-album4", "Nights", "Nujoom", 10),
        ("demo-album5", "Solstice", "Arooj Aftab", 8),
    ]
    .into_iter()
    .map(|(id, title, artist, count)| album(id, title, artist, count))
    .collect()
}

fn new_release_albums() -> Vec<Album> {
    [
        ("demo-new001", "A Prayer", "Faraj Suleiman", 11),
        ("demo-new002", "SIRI", "Shayhn", 9),
        ("demo-new003", "Wherever", "Beach House", 13),
        ("demo-new004", "Qamar", "Hamza Namira", 15),
        ("demo-new005", "Hekayat", "Cairokee", 12),
        ("demo-new006", "Riyadh", "Nujoom", 7),
    ]
    .into_iter()
    .map(|(id, title, artist, count)| album(id, title, artist, count))
    .collect()
}

fn album(id: &str, title: &str, artist: &str, song_count: u32) -> Album {
    Album {
        id: id.to_string(),
        playlist_id: format!("OLAK5uy_{id}"),
        title: title.to_string(),
        artists: vec![Artist::new(artist, None)],
        year: Some(2025),
        song_count: Some(song_count),
        ..Album::default()
    }
}

fn hash_id(id: &str) -> usize {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as usize
}

/// Deterministic gradient/ring placeholder artwork (no external assets).
fn make_art(ctx: &egui::Context, index: usize) -> TextureHandle {
    const SIZE: usize = 128;
    const PALETTES: [([u8; 3], [u8; 3]); 12] = [
        ([235, 85, 100], [40, 20, 60]),
        ([60, 120, 220], [10, 10, 30]),
        ([230, 140, 60], [30, 20, 10]),
        ([120, 200, 170], [10, 30, 40]),
        ([180, 120, 230], [20, 10, 40]),
        ([240, 220, 90], [40, 30, 10]),
        ([90, 200, 240], [10, 20, 40]),
        ([230, 200, 180], [30, 20, 30]),
        ([160, 240, 120], [10, 30, 10]),
        ([250, 130, 180], [45, 15, 35]),
        ([120, 140, 250], [15, 15, 50]),
        ([230, 170, 100], [35, 20, 20]),
    ];
    let (a, b) = PALETTES[index % PALETTES.len()];
    let mut pixels = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let nx = x as f32 / SIZE as f32;
            let ny = y as f32 / SIZE as f32;
            let t = nx * 0.6 + ny * 0.4;
            let mut c = [
                (f32::from(a[0]) * (1.0 - t) + f32::from(b[0]) * t) as i32,
                (f32::from(a[1]) * (1.0 - t) + f32::from(b[1]) * t) as i32,
                (f32::from(a[2]) * (1.0 - t) + f32::from(b[2]) * t) as i32,
            ];
            let cx = SIZE as f32 * 0.58;
            let cy = SIZE as f32 * 0.38;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let distance = (dx * dx + dy * dy).sqrt();
            if (distance as i32 % 21) < 3 {
                for channel in &mut c {
                    *channel = (*channel + 42).min(255);
                }
            }
            // Soft vignette keeps cards distinct on black surfaces.
            let vignette = 1.0 - 0.32 * (nx - 0.5).abs() - 0.28 * (ny - 0.5).abs();
            pixels.push(Color32::from_rgb(
                (c[0] as f32 * vignette).clamp(0.0, 255.0) as u8,
                (c[1] as f32 * vignette).clamp(0.0, 255.0) as u8,
                (c[2] as f32 * vignette).clamp(0.0, 255.0) as u8,
            ));
        }
    }
    let image = ColorImage {
        size: [SIZE, SIZE],
        pixels,
        source_size: Vec2::splat(SIZE as f32),
    };
    ctx.load_texture(format!("mock-art-{index}"), image, TextureOptions::LINEAR)
}
