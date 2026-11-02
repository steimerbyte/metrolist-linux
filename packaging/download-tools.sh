#!/usr/bin/env bash
# Fetch and verify the AppImage tooling. The upstream "continuous" assets move;
# the sha256 values pin the exact bytes, so update both URL and checksum
# deliberately in one reviewable change.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p tools

fetch() {
    local url=$1 out=$2 sha=$3
    if [ -f "$out" ] && echo "$sha  $out" | sha256sum -c --status; then
        chmod +x "$out"
        echo "ok (cached) $out"
        return
    fi
    curl -fsSL --retry 3 -o "$out" "$url"
    echo "$sha  $out" | sha256sum -c -
    chmod +x "$out"
}

fetch \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" \
    tools/appimagetool \
    62aebc77c2149e96b593b94da9bbd1881e0984a56399ff6d7a03b7ddd7c9ce68

# Kept for a future hand-rolled squashfs path; not used by build-appimage.sh.
fetch \
    "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64" \
    tools/runtime-x86_64 \
    617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46

echo "tools ready: $(APPIMAGE_EXTRACT_AND_RUN=1 tools/appimagetool --version 2>&1 | head -1)"
