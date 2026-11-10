#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="${1:-$ROOT/dist/D-KEYS Desktop.app}"
TARGET="${CARGO_BUILD_TARGET:-}"
if [[ -n "$TARGET" ]]; then
    BINARY="$ROOT/target/$TARGET/release/dkeys-desktop"
else
    BINARY="$ROOT/target/release/dkeys-desktop"
fi

if [[ ! -x "$BINARY" ]]; then
    echo "Сначала выполните cargo build --release" >&2
    exit 1
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/dkeys-desktop"
cp "$ROOT/packaging/macos/Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/assets/icon.icns" "$APP/Contents/Resources/icon.icns"
chmod +x "$APP/Contents/MacOS/dkeys-desktop"

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
