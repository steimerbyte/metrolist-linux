# Metrolist Desktop packaging

This directory owns the Linux AppImage. It does not build the Rust workspace by
itself; `build-appimage.sh` wraps `cargo build` and package assembly.

## glibc-floor rule

An AppImage bundles files, not glibc. A binary built on Arch/CachyOS can require
`GLIBC_2.43` and fail on Debian 12/Ubuntu 24.04. **Release builds must happen in
an old-glibc container**:

```sh
packaging/build-appimage.sh --container   # rust:1.99.1-bookworm, max GLIBC_2.36
```

A host build is for local smoke testing only. Check the floor before publishing:

```sh
objdump -T target/release/metrolist | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail
```

If Ubuntu 22.04 must be supported, use a glibc 2.31 builder (Debian bullseye)
instead; 2.36 is the floor of the bookworm image used here.

## Layout

```text
packaging/
  AppRun                         AppImage entry point
  Metrolist-Desktop.desktop      desktop entry
  build-appimage.sh              appimagetool + staged AppDir
  bundle-mpv.sh                  mpv + non-system libs under usr/lib/metrolist/mpv
  download-tools.sh              pinned appimagetool/runtime
  tools/                         downloaded on demand (run download-tools.sh)
  README.md
```

`build-appimage.sh` stages:

```text
AppDir/
  AppRun
  Metrolist-Desktop.desktop -> usr/share/applications/...
  metrolist-desktop.png      -> usr/share/icons/hicolor/512x512/apps/...
  .DirIcon
  usr/bin/metrolist
  usr/bin/mpv                -> ../lib/metrolist/mpv/mpv
  usr/lib/metrolist/mpv/     mpv + flattened ldd closure + PROVENANCE.txt
  usr/share/applications/Metrolist-Desktop.desktop
  usr/share/icons/hicolor/{128,256,512}x.../apps/metrolist-desktop.png
  usr/share/metrolist/assets/fonts, icons, i18n/en.json
  usr/share/licenses/metrolist-desktop/{LICENSE,NOTICE,mpv/}
```

Output: `dist/Metrolist-Desktop-<version>-x86_64.AppImage` plus `SHA256SUMS`.

## Libraries: what not to bundle

Keep the host graphics/audio stack out of the AppImage:

- Do **not** bundle `libGL`, `libEGL`, Mesa DRI/gallium, `libgbm`, `libdrm`.
- Do **not** bundle `libwayland-client`, `libxkbcommon`, `libX11`/`libxcb`.
- Do **not** bundle `libasound`, PipeWire, WirePlumber, D-Bus, fontconfig or
  freetype. (The UI fonts and locales are bundled as data files, not through
  fontconfig.)

`AppRun` only adds `usr/lib/metrolist/mpv` to `LD_LIBRARY_PATH`, and only when
that directory exists. The UI binary still uses the host's graphics stack;
the bundled mpv also uses the host's audio server, X11/Wayland and Mesa
libraries.

## mpv bundle

The audio engine runs mpv as a separate process. `bundle-mpv.sh` stages
Debian bookworm's mpv 0.35.1 and the non-system libraries from its dynamic
closure; `AppRun` adds `usr/lib/metrolist/mpv` to `LD_LIBRARY_PATH` and
`usr/bin/mpv` points at the bundled binary. The engine checks that path first.

```sh
packaging/bundle-mpv.sh --container --out build/mpv-bundle
MPV_BUNDLE="$PWD/build/mpv-bundle" MPV_BUNDLE_REUSE=1 \
  packaging/build-appimage.sh --no-build
```

`--container` installs mpv inside `debian:12` so every shared object is built
against glibc 2.36. A host bundle is possible with `--mpv /path/to/mpv`, but
the host's mpv 0.41 requires `GLIBC_2.38`, so it is not release-compatible.

Excluded from the bundle on purpose: glibc, libgcc/libstdc++, Mesa/GL/Vulkan,
X11/Wayland/xkbcommon, ALSA/PulseAudio/PipeWire, D-Bus, fontconfig/freetype/
harfbuzz, zlib/SSL and other base-system libraries. They must match the
running desktop. FFmpeg, libass, libplacebo and codec libraries are bundled.

Licensing: mpv is GPL-2.0-or-later; Debian FFmpeg is LGPL-2.1-or-later; libass
is ISC; libplacebo is LGPL-2.1-or-later. Debian copyright files and
`PROVENANCE.txt` are staged into the AppImage. Keep the GPL source offer in
`NOTICE` and ship corresponding source for the GPL components.

## Assets

- `assets/fonts/`: Roboto Regular/Medium/Bold (OFL-1.1), Material Symbols
  Rounded (Apache-2.0), and their license texts. See `SOURCES.md`.
- `assets/i18n/`: generated from upstream Android XML; `INDEX.md` records
  coverage. Regenerate with:

  ```sh
  packaging/tools/extract-i18n.py --source ../metrolist/app/src/main/res \
      --output assets/i18n
  ```

- `assets/icons/`: 128/256/512 PNGs, original icon (not upstream artwork).

The binary may read assets from `$APPDIR/usr/share/metrolist/assets` via the
`METROLIST_ASSETS` variable exported by `AppRun`, or embed them at compile time.
Do not point the binary at the build host's asset path in a release.
