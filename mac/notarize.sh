#!/bin/bash
# Notariza con Apple y grapa el resultado en una app (.app) o una imagen de disco (.dmg).
#   ASC_KEY_P8=… ASC_KEY_ID=… ASC_ISSUER_ID=… bash mac/notarize.sh mac/build/Kast.app
# (clave de API de App Store Connect; en la CI vienen de los secretos del repositorio).
set -euo pipefail
TARGET="$1"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
KEY="$TMP/AuthKey.p8"
printf '%s\n' "$ASC_KEY_P8" > "$KEY"

# Una app se envía comprimida; la imagen de disco, tal cual.
SUBMIT="$TARGET"
if [[ "$TARGET" == *.app ]]; then
  SUBMIT="$TMP/notarizar.zip"
  ditto -c -k --keepParent "$TARGET" "$SUBMIT"
fi
xcrun notarytool submit "$SUBMIT" --key "$KEY" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" \
  --wait --timeout 30m --output-format json | tee "$TMP/notary.json" || true
if ! grep -q '"status" *: *"Accepted"' "$TMP/notary.json"; then
  SUB="$(sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' "$TMP/notary.json" | head -1)"
  [ -n "$SUB" ] && xcrun notarytool log "$SUB" --key "$KEY" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" || true
  echo "::error::Apple rechazó la notarización de $(basename "$TARGET")"
  exit 1
fi
xcrun stapler staple "$TARGET"
