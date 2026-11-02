# Building Metrolist Desktop

## Prerequisites

- Rust 1.99.1 (pinned by `rust-toolchain.toml`); `rustfmt`/`clippy`.
  (`material-colors 0.5` requires rustc >= 1.97, so 1.95 cannot build the UI.)
- Linux desktop toolchain: `pkg-config`, ALSA headers (`libasound2-dev` on
  Debian/Ubuntu) and a C compiler for bundled SQLite.
- For AppImage: Docker (for the glibc-floor build), `curl`, and
  `desktop-file-utils` (optional validation).
- No Android SDK, NDK, Gradle, or emulator is used.
- Compilation is pinned to **one CPU core** by default. The tracked
  `.cargo/config.toml.example` sets `[build] jobs = 1`; copy it to
  `.cargo/config.toml` (gitignored) for a local build. The maintainer's host
  config adds a `rustc-wrapper` — an **absolute** path to
  `scripts/rustc-single.sh`, which takes a global `flock` on
  `~/.cache/metrolist-rustc.lock`, so concurrent cargo runs (even with separate
  `CARGO_TARGET_DIR`s) never run two `rustc` processes at once.
- No wrapper path is committed: cargo requires an absolute path for
  `rustc-wrapper`, so a committed value would break every other checkout. CI
  sets `CARGO_BUILD_JOBS=1` and may set `RUSTC_WRAPPER` to the repo script.
  Only `RUSTC_WRAPPER=""` disables the cap; that is not to be used for
  shared/project builds.

## Local build (development only)

```sh
cargo build --release --locked
./target/release/metrolist
```

A host build uses the host's glibc and is **not distributable**. Confirm before
publishing:

```sh
objdump -T target/release/metrolist | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail
```

## Container build (release; glibc floor)

```sh
packaging/download-tools.sh
packaging/build-appimage.sh --container
```

The container is `rust:1.99.1-bookworm` (glibc 2.36; the resulting symbol
floor is checked by the `objdump` command below). The same build stages mpv
0.35.1 from Debian bookworm and its non-system library closure; the host mpv
0.41 cannot be used because it requires `GLIBC_2.38` and FFmpeg 7. The app
binary is built with `cargo build --release --locked`, then packaged on the
host with appimagetool. Output:

```text
dist/Metrolist-Desktop-<version>-x86_64.AppImage
dist/SHA256SUMS
```

## Validated artifact (2026-11-02)

```text
dist/Metrolist-Desktop-0.1.0-x86_64.AppImage
size:   120732152 bytes
sha256: b4b493874982982c7c4f8a7d81b20f4626865ae91e094111a610484dafc088ba
app:    max GLIBC_2.35 (container build)
mpv:    0.35.1, max GLIBC_2.34
```

Built from the tree frozen in `docs/SOURCE-FREEZE.sha256` (246 files, self-sha
`42e7c399…`, `sha256sum -c` passes) in `rust:1.99.1-bookworm`; the shipped
container binary is `build/target-container/release/metrolist`, sha256
`ff8cfa9186008a9b12f8997c1d07e90b7df10148093aa4e5eb50980a44ff24c3`
(61,012,744 bytes). The host binary is only a local reference and has a newer
GLIBC floor. `dist/SNAPSHOT.txt` records the checks, mpv bundle and runtime
dependencies.

Equivalent explicit commands:

```sh
docker run --rm --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e CARGO_HOME=/tmp/cargo \
  -e CARGO_TARGET_DIR=/work/build/target-container \
  -v "$PWD":/work -w /work rust:1.99.1-bookworm \
  cargo build --release --locked
packaging/bundle-mpv.sh --container --out build/mpv-bundle
MPV_BUNDLE="$PWD/build/mpv-bundle" MPV_BUNDLE_REUSE=1 \
  BIN="$PWD/build/target-container/release/metrolist" \
  packaging/build-appimage.sh --no-build
```

Local packaging without rebuilding, using an existing Debian-built mpv bundle:

```sh
MPV_BUNDLE=/path/to/mpv-bundle MPV_BUNDLE_REUSE=1 \
  packaging/build-appimage.sh --no-build
```

`SOURCE_DATE_EPOCH=0` is set by the script; appimagetool produced
byte-reproducible output in the probe with that setting.

## Runtime dependencies (host, not bundled)

The AppImage deliberately does not ship the graphics/audio stack:

- Wayland: `libwayland-client0`, `libwayland-egl1`, `libxkbcommon0`.
- X11 fallback: `libX11-6`, `libxcb1`, `libxkbcommon-x11-0`, `libxcursor1`,
  `libxrandr2`, `libxi6`, `libxinerama1`, `libxrender1`.
- GPU: the UI renders through wgpu, which was measured on Vulkan
  (`backend=Vulkan`), so `libvulkan1` plus the GPU vendor ICD (e.g.
  `vulkan-intel`, `vulkan-radeon`, `nvidia-utils`) is the supported path;
  `libgl1`/`libegl1` and Mesa DRI drivers are still needed by the bundled mpv
  and by wgpu's GL fallback. A software fallback may be available with
  `LIBGL_ALWAYS_SOFTWARE=1` / a lavapipe ICD.
- Audio: PipeWire (`pipewire`, `wireplumber`, `libpipewire-0.3-0`) or
  PulseAudio (`pulseaudio`, `libpulse0`), plus `libasound2` — the bundled mpv
  links all three; the Rust side only talks to mpv over IPC.
- D-Bus for MPRIS, the StatusNotifierItem tray and notifications. No Secret
  Service yet: proxy credentials are stored in plain JSON.

Fontconfig is not required by the AppImage: UI fonts are data files in
`usr/share/metrolist/assets/fonts` (or embedded).

## Running the AppImage

```sh
chmod +x dist/Metrolist-Desktop-*.AppImage
./dist/Metrolist-Desktop-*.AppImage
```

AppImage type 2 normally needs FUSE. On systems without FUSE/libfuse2, run:

```sh
./dist/Metrolist-Desktop-*.AppImage --appimage-extract-and-run
# or
APPIMAGE_EXTRACT_AND_RUN=1 ./dist/Metrolist-Desktop-*.AppImage
```

The build script already uses `APPIMAGE_EXTRACT_AND_RUN=1` for appimagetool so
CI/containers do not need FUSE. FUSE is only needed when users execute the
finished AppImage directly.

If Wayland/EGL is broken on a particular driver, force X11:

```sh
env -u WAYLAND_DISPLAY ./dist/Metrolist-Desktop-*.AppImage
```

## Assets and i18n

```sh
packaging/tools/extract-i18n.py --source ../metrolist/app/src/main/res \
    --output assets/i18n
```

This reads upstream `values*/metrolist_strings.xml` and `values*/strings.xml`,
writes complete `en.json` and partial locale overlays, and regenerates
`assets/i18n/INDEX.md`. Placeholders and plurals are preserved; see the index.
The AppImage bundles only `assets/i18n/en.json`; the other locale overlays stay
in the repository for future runtime i18n.

## Bundled mpv

The audio engine is a separate mpv process, not libmpv. The AppImage ships:

- `usr/bin/mpv` -> `../lib/metrolist/mpv/mpv` (the path the engine checks).
- `usr/lib/metrolist/mpv/mpv` plus the non-system `ldd` closure, flattened.
- `usr/share/licenses/metrolist-desktop/mpv/` with Debian copyright files and
  `PROVENANCE.txt`.

`AppRun` prepends `usr/lib/metrolist/mpv` to `LD_LIBRARY_PATH` and exports
`METROLIST_MPV`. The binary still uses the host's PipeWire/PulseAudio/ALSA,
X11/Wayland and Mesa libraries; those are intentionally excluded from the
bundle.

The bundled version is Debian bookworm's mpv 0.35.1 (FFmpeg 5.1), which
supports every option the engine passes (`--gapless-audio`,
`--prefetch-playlist`, `--input-ipc-server`, …). The host's mpv 0.41 is newer
but requires `GLIBC_2.38` and FFmpeg 7, so it cannot run on the AppImage's
Debian 12/Ubuntu 24.04 glibc floor. Rebuild the bundle with:

```sh
packaging/bundle-mpv.sh --container --out build/mpv-bundle
```

mpv is GPL-2.0-or-later, Debian's FFmpeg libraries are LGPL-2.1-or-later,
libass is ISC and libplacebo is LGPL-2.1-or-later. The AppImage's GPL source
offer and these notices are in `NOTICE` and
`usr/share/licenses/metrolist-desktop/`.

## Troubleshooting

- `version 'GLIBC_2.xx' not found`: the binary was built on a newer host; use
  `--container`.
- Missing `libEGL.so.1`/`libGL.so.1`: install the host Mesa/EGL packages; do
  not bundle them.
- No audio: verify PipeWire/PulseAudio is running; the bundled mpv uses the
  host's `libpipewire`, `libpulse` or `libasound`, not bundled copies.
- `mpv: error while loading shared libraries`: the bundle is incomplete or was
  built on a newer host. Rebuild with `packaging/bundle-mpv.sh --container` and
  `MPV_BUNDLE_REUSE=1`.
- AppImage will not start and FUSE is unavailable: use
  `--appimage-extract-and-run`.
