#!/usr/bin/env bash
# Download the pinned Verus release, verify its sha256, and unpack it.
# Usage: install-verus.sh [DEST_DIR]   (default: $HOME/.local/opt)
# Prints the path to the verus binary.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1091
. "$here/verus.env"
dest=${1:-"$HOME/.local/opt"}
mkdir -p "$dest"
zip="$dest/verus-$VERUS_VERSION-x86-linux.zip"

if [ ! -f "$zip" ]; then
    curl -fsSL -o "$zip.part" "$VERUS_URL"
    mv "$zip.part" "$zip"
fi
echo "$VERUS_ZIP_SHA256  $zip" | sha256sum -c --quiet -

rm -rf "$dest/verus-x86-linux"
unzip -q "$zip" -d "$dest"
binary="$dest/verus-x86-linux/verus"
[ -x "$binary" ] || { echo "verus binary missing after unpack: $binary" >&2; exit 1; }
"$binary" --version | grep -F "$VERUS_VERSION" >/dev/null || {
    echo "verus version mismatch: expected $VERUS_VERSION" >&2
    exit 1
}
echo "$binary"
