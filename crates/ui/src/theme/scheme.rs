//! Material 3 colour roles derived with `material-colors` (SPEC_2025/TonalSpot).

use eframe::egui::Color32;
use material_colors::{
    color::Rgb,
    dynamic_color::{SpecVersion, Variant},
    theme::ThemeBuilder,
};

/// The M3 role set used by the UI. All values are sRGB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorScheme {
    pub surface_dim: Color32,
    pub surface: Color32,
    pub surface_bright: Color32,
    pub surface_container_lowest: Color32,
    pub surface_container_low: Color32,
    pub surface_container: Color32,
    pub surface_container_high: Color32,
    pub surface_container_highest: Color32,
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,
    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,
    pub inverse_primary: Color32,
    pub secondary: Color32,
    pub on_secondary: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,
    pub tertiary: Color32,
    pub on_tertiary: Color32,
    pub tertiary_container: Color32,
    pub on_tertiary_container: Color32,
    pub error: Color32,
    pub on_error: Color32,
    pub error_container: Color32,
    pub on_error_container: Color32,
    pub inverse_surface: Color32,
    pub inverse_on_surface: Color32,
    pub surface_tint: Color32,
    pub scrim: Color32,
    pub shadow: Color32,
}

impl ColorScheme {
    /// Build a full role set from a seed (0xRRGGBB). `dark` selects the scheme.
    pub fn from_seed(seed: u32, dark: bool) -> Self {
        let theme = ThemeBuilder::with_source(Rgb::from_u32(seed))
            .spec_version(SpecVersion::Spec2025)
            .variant(Variant::TonalSpot)
            .build();
        let s = if dark {
            theme.schemes.dark
        } else {
            theme.schemes.light
        };
        let c = |rgb: Rgb| Color32::from_rgb(rgb.red, rgb.green, rgb.blue);
        Self {
            surface_dim: c(s.surface_dim),
            surface: c(s.surface),
            surface_bright: c(s.surface_bright),
            surface_container_lowest: c(s.surface_container_lowest),
            surface_container_low: c(s.surface_container_low),
            surface_container: c(s.surface_container),
            surface_container_high: c(s.surface_container_high),
            surface_container_highest: c(s.surface_container_highest),
            on_surface: c(s.on_surface),
            on_surface_variant: c(s.on_surface_variant),
            surface_variant: c(s.surface_variant),
            outline: c(s.outline),
            outline_variant: c(s.outline_variant),
            primary: c(s.primary),
            on_primary: c(s.on_primary),
            primary_container: c(s.primary_container),
            on_primary_container: c(s.on_primary_container),
            inverse_primary: c(s.inverse_primary),
            secondary: c(s.secondary),
            on_secondary: c(s.on_secondary),
            secondary_container: c(s.secondary_container),
            on_secondary_container: c(s.on_secondary_container),
            tertiary: c(s.tertiary),
            on_tertiary: c(s.on_tertiary),
            tertiary_container: c(s.tertiary_container),
            on_tertiary_container: c(s.on_tertiary_container),
            error: c(s.error),
            on_error: c(s.on_error),
            error_container: c(s.error_container),
            on_error_container: c(s.on_error_container),
            inverse_surface: c(s.inverse_surface),
            inverse_on_surface: c(s.inverse_on_surface),
            surface_tint: c(s.surface_tint),
            scrim: c(s.scrim),
            shadow: c(s.shadow),
        }
    }

    /// Android "pure black": every surface tier becomes black; role colours stay.
    pub fn pure_black(mut self) -> Self {
        let black = Color32::BLACK;
        self.surface_dim = black;
        self.surface = black;
        self.surface_bright = black;
        self.surface_container_lowest = black;
        self.surface_container_low = black;
        self.surface_container = black;
        self.surface_container_high = black;
        self.surface_container_highest = black;
        self.surface_variant = Color32::from_rgb(0x1A, 0x1A, 0x1A);
        self.outline_variant = Color32::from_rgb(0x33, 0x33, 0x33);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_produces_pink_dark_scheme() {
        let scheme = ColorScheme::from_seed(0x00ED_5564, true);
        // The probe measured primary #F8B6B7 for this seed/spec/variant.
        assert_eq!(scheme.primary, Color32::from_rgb(0xF8, 0xB6, 0xB7));
        assert!(scheme.surface.r() < 40);
    }

    #[test]
    fn pure_black_zeroes_surfaces() {
        let scheme = ColorScheme::from_seed(0x00ED_5564, true).pure_black();
        assert_eq!(scheme.surface, Color32::BLACK);
        assert_eq!(scheme.surface_container_high, Color32::BLACK);
        assert_ne!(scheme.primary, Color32::BLACK);
    }
}
