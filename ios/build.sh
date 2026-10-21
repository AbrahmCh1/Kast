#!/usr/bin/env bash
# Compila Kast para iOS (requiere Xcode y Rust con los targets de iOS).
#   ios/build.sh sim       simulador (sin firmar)        -> ios/build/Release-iphonesimulator/Kast.app
#   ios/build.sh device    iPhone/iPad (firma de desarrollo automática, con la cuenta de Xcode)
#                                                         -> ios/build/Release-iphoneos/Kast.app
set -euo pipefail
cd "$(dirname "$0")"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export PATH="$HOME/.cargo/bin:$PATH"

case "${1:-sim}" in
  sim)    TARGET=aarch64-apple-ios-sim; SDK=iphonesimulator; SIGN=(CODE_SIGNING_ALLOWED=NO) ;;
  device) TARGET=aarch64-apple-ios;     SDK=iphoneos;        SIGN=(-allowProvisioningUpdates) ;;
  *) echo "uso: ios/build.sh sim|device" >&2; exit 1 ;;
esac

# Núcleo de Kast Link (Rust) como biblioteca estática.
(cd ../link && cargo rustc --release --locked --lib --crate-type staticlib --target "$TARGET")

xcodebuild -project Kast.xcodeproj -target Kast -configuration Release -sdk "$SDK" ARCHS=arm64 \
  SYMROOT="$PWD/build" "${SIGN[@]}" -quiet
echo "OK: ios/build/Release-$SDK/Kast.app"
