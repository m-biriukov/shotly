#!/bin/sh
# Builds Screenshoot.app (menu-bar only, ad-hoc signed) into target/.
# Usage: scripts/bundle.sh [install]   — "install" also copies it to /Applications.
set -eu
cd "$(dirname "$0")/.."

cargo build --release

APP=target/Screenshoot.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/screenshoot "$APP/Contents/MacOS/screenshoot"
mkdir -p "$APP/Contents/Resources"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Screenshoot</string>
    <key>CFBundleIdentifier</key><string>dev.screenshoot</string>
    <key>CFBundleExecutable</key><string>screenshoot</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Sign with a stable identity when available so macOS keeps the Screen
# Recording permission across rebuilds; otherwise fall back to ad hoc.
IDENTITY="${SCREENSHOOT_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ] && security find-identity -v -p codesigning 2>/dev/null | grep -q '"Screenshoot Dev"'; then
    IDENTITY="Screenshoot Dev"
fi
if [ -n "$IDENTITY" ]; then
    codesign --force --sign "$IDENTITY" --identifier dev.screenshoot "$APP"
    echo "Signed with \"$IDENTITY\""
else
    codesign --force --sign - --identifier dev.screenshoot "$APP"
    echo "Signed ad hoc (see README to keep permissions across rebuilds)"
fi
echo "Built $APP"

if [ "${1:-}" = "install" ]; then
    # Quit a running copy so the new build is the one that starts.
    pkill -f "/Screenshoot.app/Contents/MacOS/screenshoot" 2>/dev/null || true
    rm -rf "/Applications/Screenshoot.app"
    cp -R "$APP" /Applications/
    echo "Installed to /Applications/Screenshoot.app"
fi
