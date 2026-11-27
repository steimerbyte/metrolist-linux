//! Nothing OS theming — one monochrome palette, 1-px hairline strokes, no
//! elevation. Typography uses bundled Space Grotesk, Doto (display) and
//! Space Mono (uppercase labels); Material Symbols is kept for the icon slots.

pub mod fonts;
pub mod grain;
mod scheme;
mod spacing;
mod type_scale;

pub use fonts::{
    bold as bold_family, display as display_family, font_bold as bold_font, font_display,
    font_label, font_medium as medium_font, font_regular as regular_font, icon as icon_font,
    install_fonts, label as label_family, medium as medium_family, regular as regular_family,
    FAMILY_DISPLAY, FAMILY_LABEL, ICON_FAMILY,
};
pub use grain::paint_grain;
pub use scheme::ColorScheme;
pub use spacing::*;
pub use type_scale::*;

use eframe::egui::{
    self, Color32, FontId, Margin, Shadow, Stroke, TextStyle, ThemePreference, Vec2,
};

use ml_core::{Settings, ThemeMode};

/// Resolved theme for the current frame. Cheap to clone (colours only).
#[derive(Debug, Clone)]
pub struct Theme {
    pub scheme: ColorScheme,
    pub dark: bool,
    pub pure_black: bool,
    /// Collapse every animation to an instant state change.
    pub reduce_motion: bool,
}

impl Theme {
    pub fn from_settings(settings: &Settings) -> Self {
        let dark = match settings.theme_mode {
            ThemeMode::System | ThemeMode::Dark | ThemeMode::PureBlack => true,
            ThemeMode::Light => false,
        };
        let pure_black = settings.theme_mode == ThemeMode::PureBlack;
        let (r, g, b) = settings.seed_rgb();
        let mut scheme = ColorScheme::from_seed(Color32::from_rgb(r, g, b), dark);
        if pure_black {
            scheme = scheme.pure_black();
        }
        Self {
            scheme,
            dark,
            pure_black,
            reduce_motion: settings.reduce_motion,
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

    /// The 1-px divider stroke every Nothing edge uses.
    pub fn hairline(&self) -> egui::Stroke {
        egui::Stroke::new(1.0, self.scheme.outline_variant)
    }

    /// Install fonts-adjacent egui style (call fonts separately) plus the palette.
    pub fn install(&self, ctx: &egui::Context) {
        let s = &self.scheme;
        let mut style = (*ctx.global_style()).clone();
        style.visuals = self.visuals();
        style.visuals.window_shadow = Shadow::NONE;
        style.visuals.popup_shadow = Shadow::NONE;
        style.visuals.window_stroke = self.hairline();
        style.visuals.window_corner_radius = egui::CornerRadius::same(SHAPE_LARGE);
        style.visuals.menu_corner_radius = egui::CornerRadius::same(SHAPE_LARGE);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(SHAPE_MEDIUM);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(SHAPE_MEDIUM);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(SHAPE_MEDIUM);
        style.visuals.widgets.open.corner_radius = egui::CornerRadius::same(SHAPE_MEDIUM);
        style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, s.outline);
        style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, s.outline);
        style.visuals.widgets.open.bg_stroke = Stroke::new(1.0, s.outline);
        style.visuals.selection.bg_fill = s.on_surface.gamma_multiply(0.15);
        style.visuals.selection.stroke = Stroke::new(1.0, s.on_surface);
        style.visuals.hyperlink_color = s.on_surface;
        style.visuals.text_cursor.stroke = Stroke::new(1.0, s.on_surface);
        style.visuals.text_cursor.preview = false;
        style.spacing.item_spacing = Vec2::splat(DP8);
        style.spacing.button_padding = Vec2::new(DP16, DP8);
        style.spacing.interact_size = Vec2::splat(TOUCH);
        style.spacing.menu_margin = Margin::same(DP8 as i8);
        style.spacing.indent = DP20;
        style.animation_time = crate::motion::HOVER;
        style.scroll_animation = if self.reduce_motion {
            egui::style::ScrollAnimation::none()
        } else {
            egui::style::ScrollAnimation::default()
        };
        style.text_styles.clear();
        style.text_styles.extend([
            (
                TextStyle::Heading,
                FontId::new(TITLE_LARGE, fonts::medium()),
            ),
            (TextStyle::Body, FontId::new(BODY_LARGE, fonts::regular())),
            (TextStyle::Button, FontId::new(LABEL_LARGE, fonts::medium())),
            (TextStyle::Small, FontId::new(BODY_SMALL, fonts::regular())),
            (TextStyle::Monospace, FontId::new(13.0, fonts::label())),
            (
                TextStyle::Name("m3_title_large".into()),
                FontId::new(TITLE_LARGE, fonts::medium()),
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
                FontId::new(LABEL_MEDIUM, fonts::label()),
            ),
        ]);

        let preference = if self.dark {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        };
        ctx.set_theme(preference);
        crate::motion::set_reduced(ctx, self.reduce_motion);
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
        visuals.window_fill = self.card();
        visuals.faint_bg_color = s.surface_container_low;
        visuals.extreme_bg_color = s.surface_container;
        visuals.text_edit_bg_color = Some(s.surface_container_low);
        visuals.code_bg_color = s.surface_container_high;
        visuals.warn_fg_color = s.warning;
        visuals.error_fg_color = s.on_error_container;
        visuals.widgets.noninteractive.bg_fill = s.surface_container_low;
        visuals.widgets.noninteractive.weak_bg_fill = s.surface_container_low;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, s.outline_variant);
        visuals.widgets.inactive.bg_fill = s.surface_container;
        visuals.widgets.inactive.weak_bg_fill = s.surface_container_low;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, s.on_surface_variant);
        visuals.widgets.hovered.weak_bg_fill = s.surface_container_high;
        visuals.widgets.hovered.bg_fill = s.surface_container_high;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.active.weak_bg_fill = s.surface_container_high;
        visuals.widgets.active.bg_fill = s.surface_container_high;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals.widgets.open.weak_bg_fill = s.surface_container_high;
        visuals.widgets.open.bg_fill = s.surface_container_high;
        visuals.widgets.open.fg_stroke = Stroke::new(1.0, s.on_surface);
        visuals
    }
}
