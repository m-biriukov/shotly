#!/bin/sh
# Regenerates assets/AppIcon.icns from examples/icon.rs.
set -eu
cd "$(dirname "$0")/.."
cargo run --release --example icon -- assets/AppIcon.iconset
iconutil -c icns assets/AppIcon.iconset -o assets/AppIcon.icns
rm -rf assets/AppIcon.iconset
echo "Wrote assets/AppIcon.icns"
