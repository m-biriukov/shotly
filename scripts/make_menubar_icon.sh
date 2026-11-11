#!/bin/sh
# Regenerates assets/menubar-icon.png (the menu-bar icon) from
# assets/menubar-source.png: a white glyph with a soft shadow, both in the
# alpha channel.
#
# The menu bar wants a "template": black shapes on transparency, which macOS
# tints for light and dark menu bars and highlighting. The glyph (the opaque
# part of the source, without its shadow) becomes the shape. The picture is
# 88 px tall, which the app shows at 22 pt (2x, with a little margin).
# Needs ImageMagick:  brew install imagemagick
set -eu
cd "$(dirname "$0")/.."

SRC=assets/menubar-source.png
OUT=assets/menubar-icon.png
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The glyph: pixels that are (nearly) fully opaque; the shadow is faint.
magick "$SRC" -alpha extract -threshold 50% -trim +repage -resize x72 "$WORK/trim.png"
W=$(magick identify -format '%w' "$WORK/trim.png")
magick "$WORK/trim.png" -background black -gravity center -extent "$((W + 16))x88" "$WORK/mask.png"

# Black everywhere, with the glyph as its opacity.
magick "$WORK/mask.png" -alpha off -fill black -colorize 100 "$WORK/black.png"
magick "$WORK/black.png" "$WORK/mask.png" -alpha off -compose CopyOpacity -composite \
    -colorspace sRGB -type TrueColorAlpha -strip "$OUT"
echo "Wrote $OUT"
