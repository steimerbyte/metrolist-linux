#!/bin/sh
# Validate the desktop icon set: SVG master plus RGBA PNGs at every size.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
[ -f "$ROOT/assets/icons/metrolist-desktop.svg" ] || {
    echo "missing assets/icons/metrolist-desktop.svg" >&2
    exit 1
}
python3 - "$ROOT" <<'PY'
import pathlib
import struct
import sys
import zlib
import xml.etree.ElementTree as ET

def require(ok, message):
    if not ok:
        raise SystemExit(f"icon check failed: {message}")

root = pathlib.Path(sys.argv[1])
svg_path = root / "assets/icons/metrolist-desktop.svg"
svg = ET.parse(svg_path).getroot()
require(svg.tag.endswith("svg"), "root element is not <svg>")
require(svg.get("viewBox") == "0 0 1024 1024", "SVG must use a 1024 viewBox")
require(not list(svg.iter("{http://www.w3.org/2000/svg}script")),
        "SVG must not contain script")
for el in svg.iter():
    require(not any("href" in key for key in el.attrib),
            f"external reference in <{el.tag}>")
    require(not el.tag.endswith("image"), "SVG must not embed raster images")

SIZES = (16, 32, 48, 64, 128, 256, 512, 1024)

def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)

def corner_alphas(path, w, h):
    data = path.read_bytes()
    require(data[:8] == b"\x89PNG\r\n\x1a\n", f"{path}: bad PNG signature")
    pos, idat, ihdr = 8, bytearray(), None
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            ihdr = body
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        pos += 12 + length
    require(ihdr is not None and idat, f"{path}: missing IHDR/IDAT")
    raw = zlib.decompress(bytes(idat))
    stride, prev, rows = w * 4, bytearray(w * 4), {}
    pos = 0
    for y in range(h):
        ftype = raw[pos]
        pos += 1
        row = bytearray(raw[pos:pos + stride])
        pos += stride
        if ftype == 1:
            for i in range(4, stride):
                row[i] = (row[i] + row[i - 4]) & 255
        elif ftype == 2:
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 255
        elif ftype == 3:
            for i in range(stride):
                left = row[i - 4] if i >= 4 else 0
                row[i] = (row[i] + ((left + prev[i]) >> 1)) & 255
        elif ftype == 4:
            for i in range(stride):
                left = row[i - 4] if i >= 4 else 0
                up = prev[i]
                ul = prev[i - 4] if i >= 4 else 0
                row[i] = (row[i] + paeth(left, up, ul)) & 255
        elif ftype != 0:
            raise SystemExit(f"icon check failed: {path}: bad PNG filter {ftype}")
        if y == 0 or y == h - 1:
            rows[y] = row
        prev = row
    return [rows[0][3], rows[0][-1], rows[h - 1][3], rows[h - 1][-1]]

for size in SIZES:
    path = root / f"assets/icons/metrolist-desktop-{size}.png"
    require(path.is_file(), f"missing {path.relative_to(root)}")
    data = path.read_bytes()
    w, h, depth, color = struct.unpack(">IIBB", data[16:26])
    require((w, h) == (size, size),
            f"{path.name}: {w}x{h}, expected {size}x{size}")
    require(depth == 8 and color == 6, f"{path.name}: need 8-bit RGBA")
    corners = corner_alphas(path, w, h)
    require(corners == [0, 0, 0, 0],
            f"{path.name}: corners are not transparent: {corners}")

print("icon ok:", ", ".join(f"{s}x{s}" for s in SIZES), "RGBA with transparent corners")
PY
