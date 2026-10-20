#!/usr/bin/env bash
# Compila Kast.app para macOS (binario universal arm64 + x86_64).
#   mac/build.sh            versión "dev"
#   mac/build.sh 1.2.0      versión que muestran la app y `kast version`
set -euo pipefail
VERSION="${1:-dev}"
[[ "$VERSION" =~ ^[0-9A-Za-z][0-9A-Za-z.-]*$ ]] || { echo "versión inválida: $VERSION" >&2; exit 1; }
cd "$(dirname "$0")"

BUILD=build
APP="$BUILD/Kast.app"
rm -rf "$BUILD"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
printf 'let kastVersion = "%s"\n' "$VERSION" > "$BUILD/Version.swift"

for arch in arm64 x86_64; do
  swiftc -O -whole-module-optimization -target "$arch-apple-macos12.0" \
    -o "$BUILD/Kast-$arch" Sources/*.swift "$BUILD/Version.swift"
done
lipo -create -output "$APP/Contents/MacOS/Kast" "$BUILD/Kast-arm64" "$BUILD/Kast-x86_64"

SHORT="$(printf '%s' "$VERSION" | sed -E 's/^([0-9]+(\.[0-9]+)*).*/\1/')"
[[ "$SHORT" =~ ^[0-9] ]] || SHORT="0.0.0"
sed -e "s/__VERSION__/$VERSION/g" -e "s/__SHORT__/$SHORT/g" Info.plist > "$APP/Contents/Info.plist"
cp ../src/ui.html "$APP/Contents/Resources/ui.html"
cp ../kast.ini "$APP/Contents/Resources/kast.ini"

ICONSET="$BUILD/Kast.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" ../assets/kast-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) ../assets/kast-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Kast.icns"

# Firma ad-hoc (sin cuenta de desarrollador de Apple): necesaria para arm64.
codesign --force --deep --sign - "$APP"
echo "OK: $APP ($VERSION)"
