//! Material 3 spacing scale: one step per 4 dp of the 4/8/12/16 rhythm.
//!
//! Values are logical points; the app runs at 1× so 1 pt == 1 dp. Screens and
//! widgets use these tokens instead of literals so the gaps stay on the same
//! rhythm across the shell, the card grids and the settings list.

/// 4 dp — hairline gap, title↔subtitle, icon↔label inside one control.
pub const DP4: f32 = 4.0;
/// 8 dp — gap between related controls (chips, buttons, list rows).
pub const DP8: f32 = 8.0;
/// 12 dp — gap inside a group (art↔text, card rows).
pub const DP12: f32 = 12.0;
/// 16 dp — page/panel inset, section header↔content.
pub const DP16: f32 = 16.0;
/// 20 dp — navigation-rail inset.
pub const DP20: f32 = 20.0;
/// 24 dp — gap between sections, page bottom inset.
pub const DP24: f32 = 24.0;
/// 32 dp — large block break (dialogs, empty states).
pub const DP32: f32 = 32.0;
/// 48 dp — empty-state top inset.
pub const DP48: f32 = 48.0;

/// Minimum interactive target (M3 40 dp).
pub const TOUCH: f32 = 40.0;

/// Column and row gap of the album/playlist card grids.
pub const GRID_GAP: f32 = DP12;
