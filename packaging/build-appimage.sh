#!/usr/bin/env bash
# Build Metrolist Desktop as an AppImage.
#
#   packaging/download-tools.sh
#   packaging/build-appimage.sh                    # build here, package here
#   packaging/build-appimage.sh --container        # build in rust:1.99.1-bookworm
#   BIN=/tmp/metrolist MPV_BUNDLE=/tmp/mpv MPV_BUNDLE_REUSE=1 \
#       packaging/build-appimage.sh --no-build
#
# A host build is for development only: it may require a newer GLIBC than the
# target distro and its mpv cannot be shipped. Release builds MUST use
# --container. The AppImage bundles mpv and its non-system libraries under
# usr/lib/metrolist/mpv; AppRun puts that directory on LD_LIBRARY_PATH.
set -euo pipefail
source_dir=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$source_dir/.." && pwd)
CONTAINER=0
NO_BUILD=0
while [ $# -gt 0 ]; do
    case "$1" in
        --container) CONTAINER=1 ;;
        --no-build) NO_BUILD=1 ;;
        -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

VERSION=${VERSION:-$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}
[ -n "$VERSION" ] || VERSION=$(git -C "$ROOT" describe --tags --always 2>/dev/null || echo 0.0.0)
DIST=${DIST:-$ROOT/dist}
WORK=${WORK:-$ROOT/build}
APPDIR=$WORK/AppDir
MPV_BUNDLE=${MPV_BUNDLE:-$WORK/mpv-bundle}
BIN=${BIN:-}
OUT=$DIST/Metrolist-Desktop-${VERSION}-x86_64.AppImage
export SOURCE_DATE_EPOCH=0

if [ "$NO_BUILD" = 0 ]; then
    if [ "$CONTAINER" = 1 ]; then
        command -v docker >/dev/null || { echo "docker not found" >&2; exit 1; }
        mkdir -p "$WORK/target-container"
        echo "building in rust:1.99.1-bookworm (glibc 2.36 floor)"
        # .cargo/config.toml points rustc-wrapper at a host-absolute path that
        # does not exist in the container; an empty RUSTC_WRAPPER disables it.
        # The image has no ALSA dev headers (alsa-sys fails there), so they are
        # installed as root then the build drops to the caller's uid.
        docker run --rm \
            -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
            -e HOME=/tmp -e CARGO_HOME=/tmp/cargo \
            -e CARGO_TARGET_DIR=/work/build/target-container \
            -e RUSTC_WRAPPER= -e CARGO_BUILD_JOBS=1 \
            -v "$ROOT":/work -w /work \
            rust:1.99.1-bookworm sh -c '
                set -e
                apt-get update -qq
                apt-get install -y -qq --no-install-recommends libasound2-dev \
                    >/dev/null
                exec setpriv --reuid="$HOST_UID" --regid="$HOST_GID" --clear-groups \
                    cargo build --release --locked'
        BIN=${BIN:-$WORK/target-container/release/metrolist}
    else
        cargo build --release --locked
        BIN=${BIN:-$ROOT/target/release/metrolist}
    fi
fi
BIN=${BIN:-$ROOT/target/release/metrolist}
[ -x "$BIN" ] || { echo "missing binary: $BIN (build it first, or use --container)" >&2; exit 1; }

# The bookworm mpv is required: the host's mpv 0.41 needs GLIBC_2.38 and
# FFmpeg 7. Use MPV_BUNDLE_REUSE=1 to package an already-built bundle.
if [ "${MPV_BUNDLE_REUSE:-0}" != 1 ]; then
    if [ "$CONTAINER" = 1 ]; then
        "$source_dir/bundle-mpv.sh" --container --out "$MPV_BUNDLE"
    else
        echo "refusing to bundle the host mpv; use --container or MPV_BUNDLE_REUSE=1" >&2
        echo "with MPV_BUNDLE pointing at a Debian bookworm bundle." >&2
        exit 1
    fi
fi
[ -x "$MPV_BUNDLE/mpv" ] || { echo "missing mpv bundle: $MPV_BUNDLE/mpv" >&2; exit 1; }
[ -f "$MPV_BUNDLE/libs.txt" ] || { echo "missing $MPV_BUNDLE/libs.txt" >&2; exit 1; }

for asset in assets/fonts assets/i18n assets/icons; do
    [ -d "$ROOT/$asset" ] || { echo "missing $asset" >&2; exit 1; }
done

rm -rf "$APPDIR"
install -d "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/metrolist" "$APPDIR/usr/share/licenses/metrolist-desktop" \
    "$APPDIR/usr/lib/metrolist/mpv" \
    "$APPDIR/usr/share/icons/hicolor/128x128/apps" \
    "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
    "$APPDIR/usr/share/icons/hicolor/512x512/apps"
install -m 0755 "$BIN" "$APPDIR/usr/bin/metrolist"
install -m 0755 "$source_dir/AppRun" "$APPDIR/AppRun"
install -m 0644 "$source_dir/Metrolist-Desktop.desktop" \
    "$APPDIR/usr/share/applications/Metrolist-Desktop.desktop"
for size in 128 256 512; do
    install -m 0644 "$ROOT/assets/icons/metrolist-desktop-$size.png" \
        "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps/metrolist-desktop.png"
done
ln -s usr/share/applications/Metrolist-Desktop.desktop "$APPDIR/Metrolist-Desktop.desktop"
ln -s usr/share/icons/hicolor/512x512/apps/metrolist-desktop.png "$APPDIR/metrolist-desktop.png"
ln -s metrolist-desktop.png "$APPDIR/.DirIcon"
install -d "$APPDIR/usr/share/metrolist/assets"
cp -a "$ROOT/assets/fonts" "$APPDIR/usr/share/metrolist/assets/"
cp -a "$ROOT/assets/icons" "$APPDIR/usr/share/metrolist/assets/"
install -d "$APPDIR/usr/share/metrolist/assets/i18n"
install -m 0644 "$ROOT/assets/i18n/en.json" "$APPDIR/usr/share/metrolist/assets/i18n/en.json"
install -m 0644 "$ROOT/LICENSE" "$APPDIR/usr/share/licenses/metrolist-desktop/LICENSE" 2>/dev/null || true
[ -f "$ROOT/NOTICE" ] && install -m 0644 "$ROOT/NOTICE" "$APPDIR/usr/share/licenses/metrolist-desktop/NOTICE"

# The audio engine resolves $APPDIR/usr/bin/mpv before PATH. Keep the real
# binary and its libraries together, and expose the path it already checks.
cp -a "$MPV_BUNDLE/." "$APPDIR/usr/lib/metrolist/mpv/"
ln -s ../lib/metrolist/mpv/mpv "$APPDIR/usr/bin/mpv"
install -d "$APPDIR/usr/share/licenses/metrolist-desktop/mpv"
if [ -d "$MPV_BUNDLE/licenses" ]; then
    cp -a "$MPV_BUNDLE/licenses/." "$APPDIR/usr/share/licenses/metrolist-desktop/mpv/"
fi
install -m 0644 "$MPV_BUNDLE/PROVENANCE.txt" \
    "$APPDIR/usr/share/licenses/metrolist-desktop/mpv/PROVENANCE.txt"

if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$source_dir/Metrolist-Desktop.desktop"
fi

TOOL=${APPIMAGETOOL:-$ROOT/packaging/tools/appimagetool}
[ -x "$TOOL" ] || { echo "missing $TOOL; run packaging/download-tools.sh" >&2; exit 1; }
mkdir -p "$DIST"
rm -f "$OUT"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" "$APPDIR" "$OUT" >/dev/null

# Structural check, safe without a display.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
(cd "$TMP" && "$OUT" --appimage-extract >/dev/null)
test -x "$TMP/squashfs-root/AppRun"
test -x "$TMP/squashfs-root/usr/bin/metrolist"
test -x "$TMP/squashfs-root/usr/bin/mpv"
test -x "$TMP/squashfs-root/usr/lib/metrolist/mpv/mpv"
test -f "$TMP/squashfs-root/usr/share/metrolist/assets/i18n/en.json"
test -f "$TMP/squashfs-root/usr/share/licenses/metrolist-desktop/mpv/PROVENANCE.txt"
(
    cd "$TMP/squashfs-root"
    LD_LIBRARY_PATH="$PWD/usr/lib/metrolist/mpv" ./usr/lib/metrolist/mpv/mpv --version \
        | sed -n '1p'
)
(cd "$DIST" && sha256sum "$(basename "$OUT")") | tee "$DIST/SHA256SUMS"
echo "built $OUT ($(stat -c%s "$OUT") bytes)"
