#!/usr/bin/env bash
# Builds NotchZapInator.app: release binary for Apple Silicon, Info.plist,
# ad-hoc signature. Pass extra cargo flags through, e.g.
#   scripts/bundle.sh --no-default-features   # leave the sound out
set -euo pipefail

cd "$(dirname "$0")/.."

TARGET=aarch64-apple-darwin
APP=dist/NotchZapInator.app
BIN=notch-zap-inator
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)

echo "==> cargo build --release --target $TARGET $*"
cargo build --release --target "$TARGET" "$@"

echo "==> assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$TARGET/release/$BIN" "$APP/Contents/MacOS/$BIN"
sed "s/__VERSION__/$VERSION/g" Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
plutil -lint "$APP/Contents/Info.plist" >/dev/null
# Setup scripts travel inside the bundle, so every way of installing (curl,
# Homebrew, make install) runs the same ones.
cp scripts/agent.sh scripts/chime.sh scripts/launchagent.plist "$APP/Contents/Resources/"

echo "==> ad-hoc signing"
codesign --force --deep -s - "$APP"
codesign --verify --strict "$APP"

echo "==> done: $APP ($VERSION)"
