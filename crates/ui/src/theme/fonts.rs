//! Bundled fonts. Space Grotesk (OFL-1.1) for body and UI, Material Symbols
//! Rounded (Apache-2.0) for ligature icons.
//!
//! The Nothing OS triad adds Doto Bold (OFL-1.1) for display sizes (>= 36 px)
//! and Space Mono Regular (OFL-1.1) for uppercase labels (11-12 px).
//!
//! `assets/fonts/` is the packaging hook: replace the files there and rebuild.
//! All notices are in `NOTICE` and the checksums in `assets/fonts/SOURCES.md`.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};

pub const FAMILY_REGULAR: &str = "Space Grotesk";
pub const FAMILY_MEDIUM: &str = "Space Grotesk Medium";
pub const FAMILY_BOLD: &str = FAMILY_MEDIUM;
pub const ICON_FAMILY: &str = "Material Symbols Rounded";
pub const FAMILY_DISPLAY: &str = "Doto";
pub const FAMILY_LABEL: &str = "Space Mono";

pub fn regular() -> FontFamily {
    FontFamily::Name(FAMILY_REGULAR.into())
}

pub fn medium() -> FontFamily {
    FontFamily::Name(FAMILY_MEDIUM.into())
}

/// Nothing has no separate bold file; bold text uses the medium cut.
pub fn bold() -> FontFamily {
    FontFamily::Name(FAMILY_BOLD.into())
}

pub fn icons() -> FontFamily {
    FontFamily::Name(ICON_FAMILY.into())
}

pub fn display() -> FontFamily {
    FontFamily::Name(FAMILY_DISPLAY.into())
}

pub fn label() -> FontFamily {
    FontFamily::Name(FAMILY_LABEL.into())
}

pub fn font_regular(size: f32) -> FontId {
    FontId::new(size, regular())
}

pub fn font_medium(size: f32) -> FontId {
    FontId::new(size, medium())
}

pub fn font_bold(size: f32) -> FontId {
    FontId::new(size, bold())
}

pub fn icon(size: f32) -> FontId {
    FontId::new(size, icons())
}

pub fn font_display(size: f32) -> FontId {
    FontId::new(size, display())
}

pub fn font_label(size: f32) -> FontId {
    FontId::new(size, label())
}

/// Install the bundled families. Must be called once, before the first frame.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "space-grotesk-regular".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/SpaceGrotesk-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "space-grotesk-medium".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/SpaceGrotesk-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        "material-symbols".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/MaterialSymbolsRounded.ttf"
        ))),
    );
    fonts.font_data.insert(
        "doto-bold".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Doto-Bold.ttf"
        ))),
    );
    fonts.font_data.insert(
        "space-mono".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/SpaceMono-Regular.ttf"
        ))),
    );

    fonts.families.insert(
        regular(),
        vec!["space-grotesk-regular".into(), "material-symbols".into()],
    );
    // `bold()` shares this family, so one insert covers both.
    fonts.families.insert(
        medium(),
        vec!["space-grotesk-medium".into(), "material-symbols".into()],
    );
    fonts
        .families
        .insert(icons(), vec!["material-symbols".into()]);
    fonts.families.insert(display(), vec!["doto-bold".into()]);
    fonts.families.insert(label(), vec!["space-mono".into()]);

    // Space Grotesk first for Latin; the system provider (eframe `system_fonts`)
    // supplies CJK, Arabic and emoji fallbacks behind it.
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "space-grotesk-regular".into());
    proportional.retain(|name| name != "Ubuntu-Light");

    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.insert(0, "space-mono".into());

    ctx.set_fonts(fonts);
}
