#!/bin/sh
# Builds Shotly.app (menu-bar only, ad-hoc signed) into target/.
# Usage: scripts/bundle.sh [install]   — "install" also copies it to /Applications.
set -eu
cd "$(dirname "$0")/.."

cargo build --release

APP=target/Shotly.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/shotly "$APP/Contents/MacOS/shotly"
mkdir -p "$APP/Contents/Resources"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Shotly</string>
    <key>CFBundleIdentifier</key><string>dev.shotly</string>
    <key>CFBundleExecutable</key><string>shotly</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>15.0</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Shotly records your voice with screen recordings when you turn on "Record microphone" in Settings.</string>
</dict>
</plist>
PLIST

# Sign with a stable identity when available so macOS keeps the Screen
# Recording permission across rebuilds; otherwise fall back to ad hoc.
IDENTITY="${SHOTLY_SIGN_IDENTITY:-${SCREENSHOOT_SIGN_IDENTITY:-}}"
if [ -z "$IDENTITY" ]; then
    # "Screenshoot Dev" is the certificate name from before the rename.
    for name in "Shotly Dev" "Screenshoot Dev"; do
        if security find-identity -v -p codesigning 2>/dev/null | grep -q "\"$name\""; then
            IDENTITY="$name"
            break
        fi
    done
fi
if [ -n "$IDENTITY" ]; then
    codesign --force --sign "$IDENTITY" --identifier dev.shotly "$APP"
    echo "Signed with \"$IDENTITY\""
else
    codesign --force --sign - --identifier dev.shotly "$APP"
    echo "Signed ad hoc (see README to keep permissions across rebuilds)"
fi
echo "Built $APP"

if [ "${1:-}" = "install" ]; then
    # Quit a running copy so the new build is the one that starts.
    pkill -f "/Shotly.app/Contents/MacOS/shotly" 2>/dev/null || true
    # The app was called Screenshoot before; remove that copy too.
    pkill -f "/Screenshoot.app/Contents/MacOS/screenshoot" 2>/dev/null || true
    rm -rf "/Applications/Screenshoot.app"
    rm -rf "/Applications/Shotly.app"
    cp -R "$APP" /Applications/
    echo "Installed to /Applications/Shotly.app"
fi
