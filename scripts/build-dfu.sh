#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.1.0}"
# Release/production is the safe default. Diagnostic screens, render metrics,
# and optional animations must be requested explicitly for a hardware test.
FEATURES="${PINEFORGE_FEATURES-}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
MCUBOOT="$ROOT/tools/mcuboot-src/scripts/imgtool.py"
PACKAGE="$DIST/pineforge-mcuboot-app-dfu-$VERSION.zip"

[[ "$VERSION" =~ ^[0-9]+(\.[0-9]+){0,2}(\+[0-9]+)?$ ]] || {
  echo "invalid MCUBoot version (expected maj[.min[.rev]][+build]): $VERSION" >&2
  exit 1
}

command -v cargo >/dev/null || { echo "cargo is missing" >&2; exit 1; }
command -v adafruit-nrfutil >/dev/null || { echo "adafruit-nrfutil is missing" >&2; exit 1; }
[[ -f "$MCUBOOT" ]] || {
  echo "tools/mcuboot-src/scripts/imgtool.py is missing. Copy imgtool.py from the official MCUBoot project to this location." >&2
  exit 1
}

mkdir -p "$DIST"
CARGO_ARGS=(--release)
if [[ -n "$FEATURES" ]]; then
  CARGO_ARGS+=(--features "$FEATURES")
fi
cargo build "${CARGO_ARGS[@]}"
cargo objcopy "${CARGO_ARGS[@]}" -- -O binary "$DIST/pineforge.bin"
python3 "$MCUBOOT" create \
  --align 4 \
  --version "$VERSION" \
  --header-size 32 \
  --slot-size 475136 \
  --pad-header \
  "$DIST/pineforge.bin" \
  "$DIST/pineforge-image.bin"
adafruit-nrfutil dfu genpkg \
  --dev-type 0x0052 \
  --application "$DIST/pineforge-image.bin" \
  "$PACKAGE"

echo "Created: $PACKAGE"
