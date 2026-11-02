# Release check — 2026-11-02

Release gate for the Rust port (repo root; run every command from there).
Verdict: **RELEASE READY**.

- Toolchain: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, edition 2021, GPL-3.0-only.
- Workspace: `crates/{core,innertube,lyrics,store,downloads,audio,integrations,ui}`,
  binary `metrolist` (`crates/ui/src/main.rs`).
- The tree described here is the published source of the v0.1.0 release.

## Gate commands

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (no diff) |
| `cargo check --workspace --locked` | pass |
| `cargo test --workspace --no-fail-fast` | pass — **702 passed, 0 failed, 30 ignored** (re-measured after the shutdown/leak fix) |
| `cargo clippy --workspace --all-targets --locked` | pass (exit 0) — 17 distinct warning sites, all in `ml-ui` (clippy summary: 14 lib + 15 lib test, 13 duplicated), listed below |
| `cargo build --release --locked` | pass |
| `target/release/metrolist --smoke 120` | `SMOKE_OK frames=120 backend=Vulkan home_sections=2` — exit 0 |
| `./proofs/check-drift.sh` | `no source drift (21 declared sources match)` |
| secrets/cookie scan | clean (see below) |

`--smoke` needs a live Wayland/X11 session (`DISPLAY` or `WAYLAND_DISPLAY`);
on this host `DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0`, wgpu backend Vulkan on
`Intel(R) Iris(R) Xe Graphics (RPL-U)`.

## Test counts per package

Counts are from `cargo test -p <package> --locked` on the earlier 313-test
revision; the workspace-wide re-measurement after the shutdown/leak fix is
**702 passed / 0 failed / 30 ignored** (the per-package split below was not
re-measured and is informational only).

| Package | Binaries | Passed | Failed | Ignored |
|---|---|---|---|---|
| ml-core | 2 | 18 | 0 | 0 |
| ml-innertube | 9 | 65 | 0 | 0 |
| ml-lyrics | 6 | 68 | 0 | 1 |
| ml-store | 3 | 56 | 0 | 0 |
| ml-downloads | 5 | 41 | 0 | 0 |
| ml-audio | 4 | 26 | 0 | 5 |
| ml-integrations | 6 | 19 | 0 | 7 |
| ml-ui | 6 | 20 | 0 | 1 |
| **Total** | **41** | **313** | **0** | **14** |

Ignored tests are the live tests that need mpv, D-Bus/tray/notification
services or the single-instance socket (`ml-audio live`, `ml-integrations
{instance,mpris,notify,tray}_live`, `ml-lyrics providers`).

## Binary

Host reference binary, built 14:05 from an **earlier** revision on CachyOS
(**not** the shipped artifact):

- Path: `target/release/metrolist`
- Size: 56,641,408 bytes (54 MiB, stripped, thin LTO)
- `sha256sum target/release/metrolist`:
  `d2ee934fc1872a491c43a9293f0f416da38b94955d4f1b4a539624efe63b5aee`

The shipped container build is
`build/target-container/release/metrolist`: 61,012,744 bytes, sha256
`ff8cfa9186008a9b12f8997c1d07e90b7df10148093aa4e5eb50980a44ff24c3`.

## Shipped artifact

The AppImage is the container build from the tree frozen in
`docs/SOURCE-FREEZE.sha256` (246 files, `sha256sum -c` passes), recorded in
`dist/SNAPSHOT.txt` and `docs/BUILD.md`:

```text
dist/Metrolist-Desktop-0.1.0-x86_64.AppImage
size:   120732152 bytes (115.1 MiB)
sha256: b4b493874982982c7c4f8a7d81b20f4626865ae91e094111a610484dafc088ba
app:    max GLIBC_2.35   mpv 0.35.1, max GLIBC_2.34
```

`sha256sum -c dist/SHA256SUMS` passes. Do not publish the host binary: it needs
a newer glibc than the AppImage's Debian 12 / Ubuntu 24.04 floor.

Release verification of this exact file (also installed at
`~/AppImages/metrolist_desktop.appimage`, mode 755, same size and sha256):

- Clean environment, Wayland: `--appimage-extract-and-run --smoke 120` →
  `SMOKE_OK frames=120 backend=Vulkan home_sections=2`; screenshot 405×900.
- Playback through the bundled, muted mpv over JSON IPC: `mute=yes volume=0
  ao=null`, opus, position 3.956/3600.02 s.
- Leak checks: 0 mpv processes, 0 engine sockets, 0 instance socket after
  smoke, screenshot and SIGTERM (exit in 68 ms); SIGKILL leaves 0 children and
  the next launch sweeps stale sockets.
- Tests: `cargo test --workspace --no-fail-fast` → 702 passed / 0 failed /
  30 ignored. Proofs: 12 pairs, 274 functions verified, 0 errors (freeze
  `a561191c…`, gate `7a9f0f1f…`).

## Source changes made by this release check

Both are release-blocking integration fixes; no features, no refactors.

1. `crates/ui/src/pure.rs:462` (test) — `0xFF_ED55_64` → `0xFFED_5564`.
   Same value (0xFFED5564); the underscore before `64` made clippy treat it as a
   mistyped literal suffix, a deny-by-default lint that failed
   `clippy --all-targets` with `error: mistyped literal suffix`.
2. `crates/ui/src/desktop.rs:212` (`tray_icon`) — also resolve the tray icon
   from `$METROLIST_ASSETS/icons/metrolist-desktop-128.png`, which `packaging/AppRun`
   already exports. Previously only the compile-time build path and
   `/usr/share/icons/...` were checked, so a packaged AppImage had no tray icon.

After both edits: `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets --locked`, `cargo test --workspace --locked` (702/0/30),
`cargo build --release --locked`, `--smoke 120` and `check-drift.sh` were re-run
with the results above.

Documentation updated to remove dead skeleton/stub labels:
`README.md` (intro, status table, crate layout, UI summary, settings note) and
`docs/ARCHITECTURE.md` (§1 status/crate/UI file list, §2.3 lyrics, MockData
notes).

## Panic / stub inventory (shipped code)

Scanned `crates/*/src/**` for `todo!`, `unimplemented!`, `dbg!`, `panic!`,
`unreachable!`, `.unwrap()`, `.expect(`, excluding `#[cfg(test)]` modules and
`tests/` directories:

- Production call sites: **1**
  - `crates/ui/src/services.rs:157` — `.expect("start tokio runtime for
    ml-innertube/ml-lyrics")`. Reachable from `Services::new` at startup; a failed
    runtime build leaves the app without network/lyrics, so it is fatal by
    design. Left unchanged (no obvious graceful fallback).
- `todo!` 0, `unimplemented!` 0, `dbg!` 0, `panic!` 0, `unreachable!` 0,
  `.unwrap()` 0 in production code. The other 125 matches are test code
  (`mod tests`, `tests/`).
- `MlError::Unimplemented` remains only for Cyrillic languages other than
  Macedonian in `crates/lyrics/src/romanize.rs::romanize_line` (Japanese, Korean,
  Chinese and Hindi are implemented).

## Clippy warnings (non-blocking, `ml-ui` only)

`cargo clippy` exits 0; these are lint warnings requested to be reported, not
fixed during the freeze:

- `clippy::manual_clamp` — `crates/ui/src/app.rs` (`min().max()` chain)
- `clippy::collapsible_if` — `crates/ui/src/screens/detail.rs`, `screens/search.rs`
- `clippy::unnecessary_sort_by` — `crates/ui/src/screens/history.rs`
- `clippy::needless_range_loop` — `crates/ui/src/screens/history.rs`
- `clippy::too_many_arguments` (8/7) — `screens/mod.rs` (×3), `screens/library.rs` (×2)
- `clippy::large_enum_variant` — `crates/ui/src/screens/queue.rs`
- `clippy::needless_lifetimes` — `crates/ui/src/screens/settings.rs`
- `clippy::field_reassign_with_default` — `crates/ui/src/state.rs`, `crates/ui/src/settings_store.rs`

## Assets and packaging

- Embedded at compile time: `crates/ui/assets/fonts/{Roboto-Regular,Roboto-Medium,
  Roboto-Bold,MaterialSymbolsRounded}.ttf` (`theme/fonts.rs`, `include_bytes!`).
- Shipped: `assets/fonts/` (fonts + licence texts), `assets/i18n/` (`en.json`
  with 1110 keys + 55 locale overlays + `INDEX.md`), and
  `assets/icons/metrolist-desktop-{128,256,512}.png`.
- `packaging/build-appimage.sh` installs the binary at `usr/bin/metrolist`,
  AppRun execs `$APPDIR/usr/bin/metrolist`, icons go to
  `usr/share/icons/hicolor/<size>/apps/metrolist-desktop.png` (matching
  `Metrolist-Desktop.desktop` `Icon=metrolist-desktop`), and `assets/` is copied
  to `usr/share/metrolist/assets` with `METROLIST_ASSETS` exported by AppRun.
  The script's structural checks (binary, bundled mpv, `assets/i18n/en.json`,
  mpv licences) match that layout.

Notes (reported, not blocking): the root `assets/fonts/Roboto-*.ttf` copies
differ byte-wise from the embedded `crates/ui/assets/fonts` copies
(`MaterialSymbolsRounded.ttf` is identical); fonts are embedded, so the packaged
copies are unused. No code currently reads `assets/i18n/*.json` — translations
ship but the UI is English.

## Secrets

No cookie jar, `*cookie*` file or exported session was found. Hits for
`SAPISID` are the `SAPISIDHASH` implementation in `crates/innertube/src/context.rs`
and fake literals in its test module (`SAPISID=session`, `SAPISID=abc`,
`SAPISID\tnetscape`); no `__Secure-3PAPISID` or
`music.youtube.com_cookies` content.

## Source freeze

`docs/SOURCE-FREEZE.sha256` — 246 sorted `sha256sum` lines covering every file
under `crates/` plus root `Cargo.toml` and `Cargo.lock`
(`sha256sum docs/SOURCE-FREEZE.sha256` =
`42e7c399433d0ed9310dec3cca4202fc1ebc6dbc8cd36bb8d2eb088fb3355b68`).
This manifest is for packaging provenance, not the Verus/proof freeze in
`proofs/FREEZE.sha256`.

The adversarial proof gate (`proofs/run-gates.sh`) is owned by the proof
workstream; this check ran only `proofs/check-drift.sh`, which is green.
