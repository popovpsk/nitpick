#!/usr/bin/env bash
#
# Generate application icons for all platforms from crates/ui_resources/assets/lgtm-ish.svg.
#
# Outputs into crates/ui_resources/assets/app-icon/:
#   icon.icns                 - macOS bundle icon
#   icon.ico                  - Windows exe + installer icon (multi-size, PNG-compressed)
#   32x32.png 128x128.png
#   128x128@2x.png 256x256.png
#   icon.png                  - Linux / AppImage / .deb / Flatpak + cargo-packager sources
#
# Rasterization prefers rsvg-convert or ImageMagick if present, otherwise falls
# back to AppKit via scripts/macos/svg2png.swift (keeps transparency), and last
# to macOS qlmanage (Quick Look, opaque white background). Downscaling uses
# sips (macOS). icns is built with iconutil (macOS). The .ico is packed with a
# stdlib-only Python script, so no ImageMagick/Pillow is required.
#
# Re-run this whenever crates/ui_resources/assets/lgtm-ish.svg changes.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ASSETS="$ROOT/crates/ui_resources/assets"
SVG="$ASSETS/lgtm-ish.svg"
OUT="$ASSETS/app-icon"
MASTER_SIZE=1024

mkdir -p "$OUT"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

master="$WORK/master.png"

# 1. Rasterize SVG -> master PNG at MASTER_SIZE.
if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w "$MASTER_SIZE" -h "$MASTER_SIZE" "$SVG" -o "$master"
elif command -v magick >/dev/null 2>&1; then
    magick -background none -density 384 "$SVG" -resize "${MASTER_SIZE}x${MASTER_SIZE}" "$master"
elif command -v swift >/dev/null 2>&1 && [ "$(uname)" = "Darwin" ]; then
    swift "$ROOT/scripts/macos/svg2png.swift" "$SVG" "$master" "$MASTER_SIZE"
elif command -v qlmanage >/dev/null 2>&1; then
    echo "warn: qlmanage renders on an opaque white background; icon corners will not be transparent" >&2
    qlmanage -t -s "$MASTER_SIZE" -o "$WORK" "$SVG" >/dev/null 2>&1
    mv "$WORK/$(basename "$SVG").png" "$master"
else
    echo "error: need rsvg-convert, magick, swift, or qlmanage to rasterize SVG" >&2
    exit 1
fi

# Downscale the master PNG to an arbitrary square size.
resize() { # <size> <dest>
    if command -v sips >/dev/null 2>&1; then
        sips -z "$1" "$1" "$master" --out "$2" >/dev/null
    elif command -v magick >/dev/null 2>&1; then
        magick "$master" -resize "${1}x${1}" "$2"
    else
        echo "error: need sips or magick to resize" >&2; exit 1
    fi
}

# 2. PNGs consumed directly by cargo-packager (Linux/AppImage/deb).
resize 32   "$OUT/32x32.png"
resize 128  "$OUT/128x128.png"
resize 256  "$OUT/128x128@2x.png"
resize 256  "$OUT/256x256.png"
cp "$master" "$OUT/icon.png"

# 3. macOS .icns via iconset + iconutil.
if command -v iconutil >/dev/null 2>&1; then
    set="$WORK/icon.iconset"; mkdir -p "$set"
    resize 16   "$set/icon_16x16.png"
    resize 32   "$set/icon_16x16@2x.png"
    resize 32   "$set/icon_32x32.png"
    resize 64   "$set/icon_32x32@2x.png"
    resize 128  "$set/icon_128x128.png"
    resize 256  "$set/icon_128x128@2x.png"
    resize 256  "$set/icon_256x256.png"
    resize 512  "$set/icon_256x256@2x.png"
    resize 512  "$set/icon_512x512.png"
    cp "$master" "$set/icon_512x512@2x.png"
    iconutil -c icns "$set" -o "$OUT/icon.icns"
else
    echo "warn: iconutil not found, skipping icon.icns" >&2
fi

# 4. Windows .ico: pack PNGs (Vista+ PNG-compressed entries) with stdlib Python.
ico_sizes=(16 32 48 64 128 256)
ico_pngs=()
for s in "${ico_sizes[@]}"; do
    p="$WORK/ico_$s.png"; resize "$s" "$p"; ico_pngs+=("$p")
done
python3 "$ROOT/scripts/png2ico.py" "$OUT/icon.ico" "${ico_pngs[@]}"

echo "Icons written to $OUT:"
ls -1 "$OUT"
