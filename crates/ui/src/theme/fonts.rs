//! Bundled fonts. Roboto (OFL-1.1) at 400/500/700 and Material Symbols
//! Rounded (Apache-2.0) for ligature icons.
//!
//! `assets/fonts/` is the packaging hook: replace the files there and rebuild.
//! The development binaries are copied from `/usr/share/fonts/TTF` (Roboto) and
//! `arch/ui-probe/fonts` (Material Symbols); both notices are in `NOTICE`.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};

pub const FAMILY_REGULAR: &str = "Roboto Regular";
pub const FAMILY_MEDIUM: &str = "Roboto Medium";
pub const FAMILY_BOLD: &str = "Roboto Bold";
pub const ICON_FAMILY: &str = "Material Symbols Rounded";

pub fn regular() -> FontFamily {
    FontFamily::Name(FAMILY_REGULAR.into())
}

pub fn medium() -> FontFamily {
    FontFamily::Name(FAMILY_MEDIUM.into())
}

pub fn bold() -> FontFamily {
    FontFamily::Name(FAMILY_BOLD.into())
}

pub fn icons() -> FontFamily {
    FontFamily::Name(ICON_FAMILY.into())
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

/// Install the bundled families. Must be called once, before the first frame.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "roboto-regular".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Roboto-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "roboto-medium".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Roboto-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        "roboto-bold".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Roboto-Bold.ttf"
        ))),
    );
    fonts.font_data.insert(
        "material-symbols".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/MaterialSymbolsRounded.ttf"
        ))),
    );

    fonts.families.insert(
        regular(),
        vec!["roboto-regular".into(), "material-symbols".into()],
    );
    fonts.families.insert(
        medium(),
        vec!["roboto-medium".into(), "material-symbols".into()],
    );
    fonts.families.insert(
        bold(),
        vec!["roboto-bold".into(), "material-symbols".into()],
    );
    fonts
        .families
        .insert(icons(), vec!["material-symbols".into()]);

    // Roboto first for Latin; the system provider (eframe `system_fonts`)
    // supplies CJK, Arabic and emoji fallbacks behind it.
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "roboto-regular".into());
    proportional.retain(|name| name != "Ubuntu-Light");

    ctx.set_fonts(fonts);
}
