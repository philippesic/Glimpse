#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    echo "The macOS bundle must be packaged on macOS." >&2
    exit 1
fi
cargo build --release --locked
mkdir -p build
stage=$(mktemp -d build/.glimpse-package.XXXXXX)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
app="$stage/Glimpse.app"
mkdir -p "$app/Contents/MacOS"
cp target/release/glimpse "$app/Contents/MacOS/glimpse"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>glimpse</string>
<key>CFBundleIdentifier</key><string>com.philippesic.glimpse</string>
<key>CFBundleName</key><string>Glimpse</string>
<key>CFBundleDisplayName</key><string>Glimpse</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --sign - "$app"
codesign --verify --strict "$app"
if [ -e build/Glimpse.app ]; then
    mv build/Glimpse.app "build/Glimpse.previous.$$.app"
fi
mv "$app" build/Glimpse.app
echo build/Glimpse.app
