#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.1.0}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
MCUBOOT="$ROOT/tools/mcuboot-src/scripts/imgtool.py"

command -v cargo >/dev/null || { echo "cargo is missing" >&2; exit 1; }
command -v adafruit-nrfutil >/dev/null || { echo "adafruit-nrfutil is missing" >&2; exit 1; }
[[ -f "$MCUBOOT" ]] || {
  echo "tools/mcuboot-src/scripts/imgtool.py is missing. Copy imgtool.py from the official MCUBoot project to this location." >&2
  exit 1
}

mkdir -p "$DIST"
cargo build --release
cargo objcopy --release -- -O binary "$DIST/pineforge.bin"
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
  "$DIST/pineforge-gadgetbridge-dfu.zip"

echo "Created: $DIST/pineforge-gadgetbridge-dfu.zip"
