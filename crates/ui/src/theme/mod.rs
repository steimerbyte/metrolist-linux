//! Material 3 theming — a small in-house layer over egui.
//!
//! Colour roles come from `material-colors` with the app's default seed
//! (`#ED5564`, SPEC_2025/TonalSpot). Typography uses bundled Roboto at 400/500/700
//! and Material Symbols Rounded for ligature icons.

mod fonts;
mod scheme;
mod spacing;
mod type_scale;

pub use fonts::{
    bold as bold_family, font_bold as bold_font, font_medium as medium_font,
    font_regular as regular_font, icon as icon_font, install_fonts, medium as medium_family,
    regular as regular_family, ICON_FAMILY,
};
pub use scheme::ColorScheme;
pub use spacing::*;
pub use type_scale::*;

use eframe::egui::{
    self, Color32, FontFamily, FontId, Margin, Stroke, TextStyle, ThemePreference, Vec2,
};

use ml_core::{Settings, ThemeMode};

/// Resolved theme for the current frame. Cheap to clone (colours only).
#[derive(Debug, Clone)]
pub struct Theme {
    pub scheme: ColorScheme,
    pub dark: bool,
    pub pure_black: bool,
}

impl Theme {
    pub fn from_settings(settings: &Settings) -> Self {
        let dark = match settings.theme_mode {
            ThemeMode::System | ThemeMode::Dark | ThemeMode::PureBlack => true,
            ThemeMode::Light => false,
        };
        let pure_black = settings.theme_mode == ThemeMode::PureBlack;
        let mut scheme = ColorScheme::from_seed(settings.seed_color, dark);
        if pure_black {
            scheme = scheme.pure_black();
        }
        Self {
            scheme,
            dark,
            pure_black,
        }
    }

    /// App background. Dark themes use true black like the Android app.
    pub fn bg(&self) -> Color32 {
        if self.dark {
            Color32::BLACK
        } else {
            self.scheme.surface
        }
    }

    /// Cards, mini player, dialogs.
    pub fn card(&self) -> Color32 {
        self.scheme.surface_container
    }

    /// Install fonts-adjacent egui style (call fonts separately) plus the M3 palette.
    pub fn install(&self, ctx: &egui::Context) {
        let mut style = (*ctx.global_style()).clone();
        style.visuals = self.visuals();
        style.spacing.item_spacing = Vec2::splat(DP8);
        style.spacing.button_padding = Vec2::new(DP16, DP8);
        style.spacing.interact_size = Vec2::splat(TOUCH);
        style.spacing.menu_margin = Margin::same(DP8 as i8);
        style.spacing.indent = DP20;
        style.text_styles.clear();
        style.text_styles.extend([
            (TextStyle::Heading, FontId::new(TITLE_LARGE, fonts::bold())),
            (TextStyle::Body, FontId::new(BODY_LARGE, fonts::regular())),
            (TextStyle::Button, FontId::new(LABEL_LARGE, fonts::medium())),
            (TextStyle::Small, FontId::new(BODY_SMALL, fonts::regular())),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
            (
                TextStyle::Name("m3_title_large".into()),
                FontId::new(TITLE_LARGE, fonts::bold()),
            ),
            (
                TextStyle::Name("m3_title_medium".into()),
                FontId::new(TITLE_MEDIUM, fonts::medium()),
            ),
            (
                TextStyle::Name("m3_body_medium".into()),
                FontId::new(BODY_MEDIUM, fonts::regular()),
            ),
            (
                TextStyle::Name("m3_label_medium".into()),
                FontId::new(LABEL_MEDIUM, fonts::medium()),
            ),
        ]);

        let preference = if self.dark {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        };
        ctx.set_theme(preference);
        // Apply to both theme slots: eframe may flip the preference if the
        // desktop portal changes while the app runs.
        ctx.set_style_of(egui::Theme::Dark, style.clone());
        ctx.set_style_of(egui::Theme::Light, style);
    }

    fn visuals(&self) -> egui::Visuals {
        let s = &self.scheme;
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.override_text_color = Some(s.on_surface);
        visuals.panel_fill = self.bg();
        visuals.window_fill = self.bg();
        visuals.faint_bg_color = s.surface_container_low;
        visuals.extreme_bg_color = s.surface_container_lowest;
        visuals.code_bg_color = s.surface_container_high;
        visuals.window_stroke = Stroke::new(1.0, s.outline_variant);
        visuals.selection.bg_fill = s.primary.gamma_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.noninteractive.bg_fill = s.surface_container_low;
        visuals.widgets.noninteractive.weak_bg_fill = s.surface_container_low;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, s.outline_variant);
        visuals.widgets.inactive.bg_fill = s.surface_container_high;
        visuals.widgets.inactive.weak_bg_fill = s.surface_container;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, s.on_surface_variant);
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.hovered.weak_bg_fill = s.surface_container_high;
        visuals.widgets.hovered.bg_fill = s.surface_container_highest;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, s.outline);
        visuals.widgets.active.weak_bg_fill = s.surface_container_high;
        visuals.widgets.active.bg_fill = s.surface_container_highest;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, s.primary);
        visuals.widgets.open.weak_bg_fill = s.surface_container_high;
        visuals.widgets.open.bg_fill = s.surface_container_highest;
        visuals.widgets.open.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, s.outline);
        visuals
    }
}

/// State-layer overlay helper used by custom widgets.
pub fn state_layer(base: Color32, hovered: bool, pressed: bool) -> Color32 {
    if pressed {
        base.lerp_to_gamma(Color32::WHITE, 0.12)
    } else if hovered {
        base.lerp_to_gamma(Color32::WHITE, 0.08)
    } else {
        base
    }
}
