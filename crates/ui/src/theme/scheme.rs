//! Nothing OS colour roles: one monochrome palette plus semantic accents.
//!
//! Dark `error` (#d71921) is a 4.05:1 interrupt on #000 — fills and borders
//! only. Red text on dark uses `on_error_container` (#ff6b70, 6.29:1).

use eframe::egui::Color32;

/// The Nothing role set used by the UI. All values are sRGB.
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
    pub success: Color32,
    pub warning: Color32,
}

impl ColorScheme {
    /// Build the palette for `dark`. Nothing is monochrome, so `seed` is
    /// ignored; it stays in the signature because settings store an accent.
    pub fn from_seed(_seed: Color32, dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    fn dark() -> Self {
        let c = Color32::from_rgb;
        Self {
            surface_dim: c(0x00, 0x00, 0x00),
            surface: c(0x00, 0x00, 0x00),
            surface_bright: c(0x1A, 0x1A, 0x1A),
            surface_container_lowest: c(0x00, 0x00, 0x00),
            surface_container_low: c(0x0A, 0x0A, 0x0A),
            surface_container: c(0x11, 0x11, 0x11),
            surface_container_high: c(0x1A, 0x1A, 0x1A),
            surface_container_highest: c(0x26, 0x26, 0x26),
            on_surface: c(0xE8, 0xE8, 0xE8),
            on_surface_variant: c(0x99, 0x99, 0x99),
            surface_variant: c(0x1A, 0x1A, 0x1A),
            outline: c(0x33, 0x33, 0x33),
            outline_variant: c(0x22, 0x22, 0x22),
            primary: c(0xE8, 0xE8, 0xE8),
            on_primary: c(0x00, 0x00, 0x00),
            primary_container: c(0x1A, 0x1A, 0x1A),
            on_primary_container: c(0xE8, 0xE8, 0xE8),
            inverse_primary: c(0xE8, 0xE8, 0xE8),
            secondary: c(0xCC, 0xCC, 0xCC),
            on_secondary: c(0x00, 0x00, 0x00),
            secondary_container: c(0x1A, 0x1A, 0x1A),
            on_secondary_container: c(0xE8, 0xE8, 0xE8),
            tertiary: c(0x99, 0x99, 0x99),
            on_tertiary: c(0x00, 0x00, 0x00),
            tertiary_container: c(0x1A, 0x1A, 0x1A),
            on_tertiary_container: c(0xE8, 0xE8, 0xE8),
            error: c(0xD7, 0x19, 0x21),
            on_error: c(0xFF, 0xFF, 0xFF),
            error_container: c(0x1A, 0x1A, 0x1A),
            on_error_container: c(0xFF, 0x6B, 0x70),
            inverse_surface: c(0xF5, 0xF5, 0xF5),
            inverse_on_surface: c(0x1A, 0x1A, 0x1A),
            surface_tint: c(0xE8, 0xE8, 0xE8),
            scrim: c(0x00, 0x00, 0x00),
            shadow: c(0x00, 0x00, 0x00),
            success: c(0x4A, 0x9E, 0x5C),
            warning: c(0xD4, 0xA8, 0x43),
        }
    }

    fn light() -> Self {
        let c = Color32::from_rgb;
        Self {
            surface_dim: c(0xEB, 0xEB, 0xEB),
            surface: c(0xF5, 0xF5, 0xF5),
            surface_bright: c(0xFF, 0xFF, 0xFF),
            surface_container_lowest: c(0xFF, 0xFF, 0xFF),
            surface_container_low: c(0xFF, 0xFF, 0xFF),
            surface_container: c(0xFF, 0xFF, 0xFF),
            surface_container_high: c(0xF0, 0xF0, 0xF0),
            surface_container_highest: c(0xE8, 0xE8, 0xE8),
            on_surface: c(0x1A, 0x1A, 0x1A),
            on_surface_variant: c(0x66, 0x66, 0x66),
            surface_variant: c(0xF0, 0xF0, 0xF0),
            outline: c(0xCC, 0xCC, 0xCC),
            outline_variant: c(0xE8, 0xE8, 0xE8),
            primary: c(0x1A, 0x1A, 0x1A),
            on_primary: c(0xF5, 0xF5, 0xF5),
            primary_container: c(0xE8, 0xE8, 0xE8),
            on_primary_container: c(0x1A, 0x1A, 0x1A),
            inverse_primary: c(0x1A, 0x1A, 0x1A),
            secondary: c(0x4A, 0x4A, 0x4A),
            on_secondary: c(0xF5, 0xF5, 0xF5),
            secondary_container: c(0xEE, 0xEE, 0xEE),
            on_secondary_container: c(0x1A, 0x1A, 0x1A),
            tertiary: c(0x66, 0x66, 0x66),
            on_tertiary: c(0xF5, 0xF5, 0xF5),
            tertiary_container: c(0xEE, 0xEE, 0xEE),
            on_tertiary_container: c(0x1A, 0x1A, 0x1A),
            error: c(0xC4, 0x16, 0x1E),
            on_error: c(0xFF, 0xFF, 0xFF),
            error_container: c(0xFD, 0xE8, 0xE9),
            on_error_container: c(0x8C, 0x0F, 0x15),
            inverse_surface: c(0x1A, 0x1A, 0x1A),
            inverse_on_surface: c(0xF5, 0xF5, 0xF5),
            surface_tint: c(0x1A, 0x1A, 0x1A),
            scrim: c(0x00, 0x00, 0x00),
            shadow: c(0x00, 0x00, 0x00),
            success: c(0x2B, 0x74, 0x40),
            warning: c(0x8A, 0x5F, 0x00),
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

    fn luminance(color: Color32) -> f32 {
        let channel = |value: u8| {
            let value = value as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn dark_and_light_differ() {
        let dark = ColorScheme::from_seed(Color32::TRANSPARENT, true);
        let light = ColorScheme::from_seed(Color32::TRANSPARENT, false);
        assert_ne!(dark, light);
        assert_eq!(dark.surface, Color32::BLACK);
        assert_eq!(light.surface, Color32::from_rgb(0xF5, 0xF5, 0xF5));
    }

    #[test]
    fn text_roles_keep_contrast() {
        for scheme in [
            ColorScheme::from_seed(Color32::TRANSPARENT, true),
            ColorScheme::from_seed(Color32::TRANSPARENT, false),
        ] {
            assert!(contrast(scheme.on_surface, scheme.surface) >= 4.5);
            assert!(contrast(scheme.on_surface_variant, scheme.surface) >= 4.5);
            assert!(contrast(scheme.on_error, scheme.error) >= 4.5);
        }
    }

    #[test]
    fn no_role_is_transparent() {
        let schemes = [
            ColorScheme::from_seed(Color32::TRANSPARENT, true),
            ColorScheme::from_seed(Color32::TRANSPARENT, false),
        ];
        for scheme in schemes {
            let roles = [
                scheme.surface_dim,
                scheme.surface,
                scheme.surface_bright,
                scheme.surface_container_lowest,
                scheme.surface_container_low,
                scheme.surface_container,
                scheme.surface_container_high,
                scheme.surface_container_highest,
                scheme.on_surface,
                scheme.on_surface_variant,
                scheme.surface_variant,
                scheme.outline,
                scheme.outline_variant,
                scheme.primary,
                scheme.on_primary,
                scheme.primary_container,
                scheme.on_primary_container,
                scheme.inverse_primary,
                scheme.secondary,
                scheme.on_secondary,
                scheme.secondary_container,
                scheme.on_secondary_container,
                scheme.tertiary,
                scheme.on_tertiary,
                scheme.tertiary_container,
                scheme.on_tertiary_container,
                scheme.error,
                scheme.on_error,
                scheme.error_container,
                scheme.on_error_container,
                scheme.inverse_surface,
                scheme.inverse_on_surface,
                scheme.surface_tint,
                scheme.scrim,
                scheme.shadow,
                scheme.success,
                scheme.warning,
            ];
            assert!(roles.iter().all(|color| color.a() == 255));
        }
    }
}
