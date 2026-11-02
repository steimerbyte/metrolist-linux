#!/usr/bin/env bash
# Stage mpv and the libraries it needs into a relocatable directory.
#
#   packaging/bundle-mpv.sh --container
#   MPV_BIN=/usr/bin/mpv packaging/bundle-mpv.sh --out /tmp/mpv-bundle
#
# --container installs Debian bookworm's mpv (0.35.1, glibc 2.36) and bundles
# it. The host's mpv 0.41 requires GLIBC_2.38 and FFmpeg 7, so it cannot run on
# the Debian 12/Ubuntu 24.04 glibc floor this AppImage targets.
#
# The output contains:
#   mpv, the shared libraries from `ldd` that are not part of the base desktop,
#   libs.txt, PROVENANCE.txt, licenses/.
#
# Do not add libc, libstdc++, Mesa/GL, X11/Wayland, ALSA/PipeWire, D-Bus or
# fontconfig: those must match the running host.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${MPV_BUNDLE_OUT:-/tmp/metrolist-mpv-bundle}
MPV_BIN=${MPV_BIN:-}
CONTAINER=0

usage() {
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --container) CONTAINER=1 ;;
        --mpv) shift; MPV_BIN=${1:?--mpv needs a path} ;;
        --mpv=*) MPV_BIN=${1#--mpv=} ;;
        --out) shift; OUT=${1:?--out needs a path} ;;
        --out=*) OUT=${1#--out=} ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

if [ "$CONTAINER" = 1 ]; then
    command -v docker >/dev/null || { echo "docker not found" >&2; exit 1; }
    out_abs=$(realpath -m "$OUT")
    parent=$(dirname "$out_abs")
    mkdir -p "$parent"
    host_uid=$(id -u)
    host_gid=$(id -g)
    echo "building mpv bundle in debian:12 (bookworm)"
    docker run --rm \
        -e HOST_UID="$host_uid" -e HOST_GID="$host_gid" \
        -v "$ROOT":/work:ro -v "$parent":/bundle \
        -w /work debian:12 bash -lc "
            set -e
            export DEBIAN_FRONTEND=noninteractive
            apt-get update -qq
            apt-get install -y -qq --no-install-recommends mpv >/dev/null
            /work/packaging/bundle-mpv.sh --mpv /usr/bin/mpv --out /bundle/$(basename "$out_abs")
            chown -R \"\$HOST_UID:\$HOST_GID\" /bundle/$(basename "$out_abs")
        "
    echo "mpv bundle ready: $out_abs"
    exit 0
fi

if [ -z "$MPV_BIN" ]; then
    MPV_BIN=$(command -v mpv 2>/dev/null || true)
fi
[ -n "$MPV_BIN" ] && [ -x "$MPV_BIN" ] || {
    echo "mpv not found; pass --mpv PATH or use --container" >&2
    exit 1
}

rm -rf "$OUT"
mkdir -p "$OUT/licenses"
cp -L "$MPV_BIN" "$OUT/mpv"
chmod 0755 "$OUT/mpv"
trap 'rm -f "$OUT/.libs-src"' EXIT

# Copy the dynamic closure except libraries that belong to the host desktop.
is_host_library() {
    case "$(basename "$1")" in
        linux-vdso.so.*|ld-linux*.so.*|libc.so.*|libm.so.*|libpthread.so.*|\
        libdl.so.*|librt.so.*|libgcc_s.so.*|libstdc++.so.*|\
        libGL.so.*|libEGL.so.*|libGLX.so.*|libGLdispatch.so.*|libOpenGL.so.*|\
        libgbm.so.*|libdrm.so.*|libvulkan.so.*|\
        libwayland-*.so.*|libX11.so.*|libX11-xcb.so.*|libXext.so.*|\
        libXrender.so.*|libXrandr.so.*|libXfixes.so.*|libXcursor.so.*|\
        libXi.so.*|libXinerama.so.*|libxcb*.so.*|libxkbcommon*.so.*|\
        libasound.so.*|libpulse*.so.*|libpipewire*.so.*|libdbus*.so.*|\
        libfontconfig*.so.*|libfreetype*.so.*|libharfbuzz*.so.*|\
        libssl.so.*|libcrypto.so.*|libsystemd*.so.*|libudev*.so.*|\
        libbz2.so.*|liblzma.so.*|libzstd.so.*|libz.so.*|\
        libexpat*.so.*)
            return 0
            ;;
    esac
    return 1
}

: > "$OUT/libs.txt"
: > "$OUT/.libs-src"
while IFS= read -r lib; do
    [ -f "$lib" ] || continue
    if is_host_library "$lib"; then
        continue
    fi
    base=$(basename "$lib")
    cp -L "$lib" "$OUT/$base"
    chmod 0644 "$OUT/$base"
    printf '%s\n' "$base" >> "$OUT/libs.txt"
    printf '%s\n' "$lib" >> "$OUT/.libs-src"
done < <(ldd "$MPV_BIN" 2>/dev/null | awk '
    /=> \// { print $3 }
    /^[[:space:]]*\/.* \(0x/ { print $1 }
' | sort -u)

# Package copyright files when the source distribution provides them. On a
# non-Debian host PROVENANCE.txt still records version, source and licenses.
copy_pkg_copyright() {
    pkg=$1
    doc="/usr/share/doc/$pkg"
    [ -d "$doc" ] || return 0
    for f in "$doc"/copyright "$doc"/copyright.d/*; do
        [ -f "$f" ] || continue
        cp -L "$f" "$OUT/licenses/$(basename "$pkg")-$(basename "$f")"
    done
}
for pkg in \
    mpv libavcodec59 libavformat59 libavutil57 libswresample4 libswscale6 \
    libavfilter8 libass9 libplacebo208 libluajit-5.1-2 libuchardet0; do
    copy_pkg_copyright "$pkg"
done
if command -v dpkg-query >/dev/null 2>&1; then
    while IFS= read -r lib; do
        pkg=$(dpkg-query -S "$lib" 2>/dev/null | sed 's/:.*//' | head -1 || true)
        if [ -n "$pkg" ]; then
            copy_pkg_copyright "$pkg"
        fi
    done < "$OUT/.libs-src"
fi

{
    echo "mpv bundle"
    echo "generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "host: $(uname -srm)"
    echo "binary: $MPV_BIN"
    echo "source: https://github.com/mpv-player/mpv"
    echo "license: GPL-2.0-or-later (mpv); see licenses/"
    echo "ffmpeg: LGPL-2.1-or-later for the Debian bookworm build; see licenses/"
    echo
    LD_LIBRARY_PATH="$OUT${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" LC_ALL=C \
        "$OUT/mpv" --version
    echo
    echo "bundled libraries:"
    sed 's/^/  /' "$OUT/libs.txt"
} > "$OUT/PROVENANCE.txt"

echo "bundled $(wc -l < "$OUT/libs.txt") libraries into $OUT"
