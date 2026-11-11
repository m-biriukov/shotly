#!/bin/sh
# Regenerates assets/AppIcon.icns from assets/icon-source.png (a rounded-square
# icon on a white background, with a soft shadow of its own).
#
# The icon body is cut out of the background, put on a transparent 1024 px
# canvas at the size macOS expects (824 px, the rest is margin and shadow), and
# scaled down into an .iconset for `iconutil`.
# Needs ImageMagick:  brew install imagemagick
set -eu
cd "$(dirname "$0")/.."

SRC=assets/icon-source.png
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Where the dark icon body sits in the source (the light shadow is left out).
BODY=$(magick "$SRC" -colorspace Gray -threshold 50% -negate -trim -format '%wx%h%X%Y' info:)

# Mask: the body with its light details (white ring and corners) filled in,
# smoothed so the edge is anti-aliased.
magick "$SRC" -colorspace Gray -threshold 50% -negate \
    -fill gray50 -draw 'color 0,0 floodfill' \
    -fill white +opaque gray50 -fill black -opaque gray50 \
    -blur 0x1.2 -level 35%,65% "$WORK/mask.png"

# Cut out the body and make it square.
magick "$SRC" "$WORK/mask.png" -alpha off -compose CopyOpacity -composite \
    -crop "$BODY" +repage -resize '824x824!' "$WORK/body.png"

# Set it on the 1024 px canvas with a soft shadow.
magick "$WORK/body.png" \
    \( +clone -background black -shadow 35x10+0+10 \) +swap \
    -background none -layers merge +repage -gravity center -extent 1024x1024 \
    -colorspace sRGB -type TrueColorAlpha -strip "$WORK/master.png"

SET="$WORK/AppIcon.iconset"
mkdir "$SET"
for size in 16 32 128 256 512; do
    magick "$WORK/master.png" -resize "${size}x${size}" "$SET/icon_${size}x${size}.png"
    magick "$WORK/master.png" -resize "$((size * 2))x$((size * 2))" "$SET/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$SET" -o assets/AppIcon.icns
echo "Wrote assets/AppIcon.icns"
