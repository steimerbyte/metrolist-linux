# UI spacing, density & shape

Normative contract for the Nothing shell (`crates/ui/src/theme/`). Every value
below is a `pub const` in `theme/spacing.rs` or `theme/type_scale.rs`; screens
and widgets use the tokens instead of literals so the rhythm holds across the
rail, the card grids and the settings list.

## Spacing (`theme/spacing.rs`)

| Token | dp | Used for |
|---|---|---|
| `DP4` | 4 | hairline gap, title↔subtitle, icon↔label inside one control |
| `DP8` | 8 | chips, buttons, list rows, action rows |
| `DP12` | 12 | art↔text, card grid gap (`GRID_GAP`) |
| `DP16` | 16 | page/panel inset, section header↔content |
| `DP20` | 20 | navigation-rail inset |
| `DP24` | 24 | section gap, page bottom |
| `DP32` | 32 | button body height, reserve |
| `DP48` | 48 | section-header band, empty-state top |
| `TOUCH` | 44 | minimum interactive target (`style.spacing.interact_size`) |

`TOUCH` is the floor, not a suggestion: icon buttons, transport controls, nav
destinations and row controls all allocate at least 44×44 dp. Nothing has no
denser tier for touch.

## Shape (`theme/type_scale.rs`)

| Token | Value | Used for |
|---|---:|---|
| `SHAPE_EXTRA_SMALL` | 0 | inputs, rows, table cells: square, 1-px rule |
| `SHAPE_SMALL` | 8 | adaptive icon buttons |
| `SHAPE_MEDIUM` | 12 | egui widget frames (sliders, combo boxes) |
| `SHAPE_LARGE` | 16 | cards, dialogs, menus, player art frame |
| `SHAPE_EXTRA_LARGE` | 16 | alias: Nothing has no 28 dp container |
| pill | `u8::MAX` | chips, primary/secondary buttons, badges |

Rules that go with the numbers:

- Every edge is a 1-px stroke (`Theme::hairline()`, `scheme.outline_variant`);
  a visible divider uses `scheme.outline`.
- No shadows, no blur, no decorative gradients, no elevation steps. Depth is
  expressed by a hairline and a surface step (#000 → #111 → #1a1a1a in dark,
  #f5f5f5 → #fff → #f0f0f0 in light).
- Cards never nest a second rounded container; lists never round rows.

## Type (`theme/type_scale.rs`)

| Token | px | Role |
|---|---:|---|
| `DISPLAY_LARGE` | 72 | Doto, landing numerals |
| `DISPLAY_MEDIUM` | 48 | Doto, hero numbers |
| `DISPLAY_SMALL` | 36 | Doto floor — below this Doto is not used |
| `HEADLINE_LARGE` | 36 | Space Grotesk Medium |
| `HEADLINE_MEDIUM` | 28 | Space Grotesk Medium |
| `HEADLINE_SMALL` | 24 | screen titles |
| `TITLE_LARGE` | 24 | egui `Heading` |
| `TITLE_MEDIUM` | 18 | card/row titles |
| `TITLE_SMALL` | 16 | compact titles |
| `BODY_LARGE` | 16 | body |
| `BODY_MEDIUM` | 14 | secondary body |
| `BODY_SMALL` | 12 | captions |
| `LABEL_LARGE` | 14 | buttons |
| `LABEL_MEDIUM` | 12 | chips, section labels (Space Mono, UPPERCASE) |
| `LABEL_SMALL` | 11 | metadata, counters, nav labels (Space Mono, UPPERCASE) |

Families: Space Grotesk Regular/Medium for body and titles (Nothing has no
separate bold cut), Doto ≥ 36 px for display only, Space Mono UPPERCASE for
labels at 11–12 px, Material Symbols Rounded for the icon slots.

## Content column

The shell centres a column inside the page inset: with the rail (window
≥ 840 dp) the content is `min(available − 88 − 32, 960)` dp; without it the
column is `min(available − 32, 560)` dp, and the player narrows further to
`min(available − 48, 560)` with a 240 dp floor. The rail is 88 dp wide when
expanded (`is_expanded`, width ≥ 840 dp); below that the navigation moves to the
bottom bar with five destinations.

## Verification

- The `theme` unit tests (dark ≠ light, contrast pairs, no transparent roles)
  and the `motion` tests run in the `ml-ui` lib test target.
- Proof captures: `--screenshot` at 405×900 (compact) and the default window
  (expanded), dark and light — see `docs/UI-AUDIT.md`.
