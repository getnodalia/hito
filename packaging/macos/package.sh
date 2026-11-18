#!/usr/bin/env bash
# Build and package Hito for macOS:
#
#   $DIST/hito-<version>-macos-<arch>.dmg            Hito.app on a drag-to-Applications DMG
#   $DIST/hito-server-<version>-macos-<arch>.zip     the server, for running it on a Mac
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build]
#
# Everything is signed ad hoc: Hito has no Apple Developer ID, so macOS asks for confirmation the
# first time the app is opened (right-click > Open, or System Settings > Privacy & Security).
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"

ARCH=universal
SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/Hito.app"
DMG="$DIST/hito-$VERSION-macos-$ARCH.dmg"
SERVER_ZIP="$DIST/hito-server-$VERSION-macos-$ARCH.zip"

echo "==> Hito $VERSION for macOS ($ARCH), signed ad hoc"

# ---- build -------------------------------------------------------------------------------------
if [[ "$SKIP_BUILD" = 0 ]]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked -p hito-desktop -p hito-server --bins "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
for bin in hito hito-server; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

sign() {
  codesign --force --sign - --timestamp=none "$@"
}

# ---- Hito.app ---------------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$WORK/bin/hito" "$APP/Contents/MacOS/Hito"
# The .icns is made here from the committed 1024 px tile (already on Apple's icon grid) with the
# system's own sips and iconutil, so no generated binary is committed for it.
ICONSET="$WORK/Hito.iconset"
TILE="$ROOT/assets/icons/hicolor/1024x1024/apps/$APP_ID.png"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$TILE" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) "$TILE" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns -o "$APP/Contents/Resources/Hito.icns" "$ICONSET"
cp "$ROOT/ATTRIBUTION.md" "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$APP/Contents/Resources/"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${HITO_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/Hito"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Hito.app"
ln -s /Applications "$STAGE/Applications"
# Finder window layout: background, icon size and positions (packaging/macos/dmg/README.md).
mkdir -p "$STAGE/.background"
cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
rm -f "$DMG" "$WORK/raw.dmg"
# makehybrid + convert builds the image without attaching a device, unlike `create -srcfolder`,
# which is flaky on CI runners ("Resource busy").
# The volume name has no version: .DS_Store finds the background through an alias that includes it.
hdiutil makehybrid -hfs -hfs-volume-name "Hito" -hfs-openfolder "$STAGE" -o "$WORK/raw.dmg" "$STAGE"
hdiutil convert "$WORK/raw.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
rm -f "$WORK/raw.dmg"
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"

# ---- server ------------------------------------------------------------------------------------
echo "==> building $SERVER_ZIP"
SERVER_DIR="$WORK/hito-server-$VERSION-macos-$ARCH"
mkdir -p "$SERVER_DIR"
cp "$WORK/bin/hito-server" "$SERVER_DIR/"
copy_docs "$SERVER_DIR"
sign --options runtime --identifier dev.danielmiguelt.Hito-server "$SERVER_DIR/hito-server"
rm -f "$SERVER_ZIP"
ditto -c -k --keepParent "$SERVER_DIR" "$SERVER_ZIP"

"$WORK/bin/hito" --version
"$WORK/bin/hito-server" --version
echo "==> done"
ls -lh "$DMG" "$SERVER_ZIP"
