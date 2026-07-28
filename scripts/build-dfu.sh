#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.1.0}"
# Release/production, including normal UI animations, is the safe default.
# Diagnostic screens and render metrics must be requested explicitly.
FEATURES="${PINEFORGE_FEATURES-}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
MCUBOOT="$ROOT/tools/mcuboot-src/scripts/imgtool.py"
PACKAGE="$DIST/pineforge-mcuboot-app-dfu-$VERSION.zip"
PYTHON="${PINEFORGE_PYTHON:-$ROOT/.venv/bin/python}"
NRFUTIL="${PINEFORGE_NRFUTIL:-$ROOT/.venv/bin/adafruit-nrfutil}"

if [[ ! -x "$PYTHON" ]]; then
  PYTHON="$(command -v python3 || true)"
fi
if [[ ! -x "$NRFUTIL" ]]; then
  NRFUTIL="$(command -v adafruit-nrfutil || true)"
fi

[[ "$VERSION" =~ ^[0-9]+(\.[0-9]+){0,2}(\+[0-9]+)?$ ]] || {
  echo "invalid MCUBoot version (expected maj[.min[.rev]][+build]): $VERSION" >&2
  exit 1
}

command -v cargo >/dev/null || { echo "cargo is missing" >&2; exit 1; }
[[ -x "$PYTHON" ]] || { echo "python3 is missing" >&2; exit 1; }
[[ -x "$NRFUTIL" ]] || {
  echo "adafruit-nrfutil is missing (checked .venv and PATH)" >&2
  exit 1
}
[[ -f "$MCUBOOT" ]] || {
  echo "tools/mcuboot-src/scripts/imgtool.py is missing. Copy imgtool.py from the official MCUBoot project to this location." >&2
  exit 1
}

mkdir -p "$DIST"

# What the watch will say it is. The build number lives here and nowhere else -
# Cargo.toml carries only the release - so it has to be handed to the build
# script, or the firmware reports a version that cannot tell two packages apart.
# The commit is marked dirty when the tree has uncommitted changes, because an
# id that names a commit the binary was not built from is worse than none.
export PINEFORGE_VERSION="$VERSION"
if command -v git >/dev/null && git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1; then
  PINEFORGE_COMMIT="$(git -C "$ROOT" rev-parse --short=7 HEAD)"
  if [[ -n "$(git -C "$ROOT" status --porcelain)" ]]; then
    PINEFORGE_COMMIT="$PINEFORGE_COMMIT-dirty"
  fi
  export PINEFORGE_COMMIT
  export PINEFORGE_DATE="$(git -C "$ROOT" log -1 --format=%cs)"
fi

CARGO_ARGS=(--release)
if [[ -n "$FEATURES" ]]; then
  CARGO_ARGS+=(--features "$FEATURES")
fi
cargo build "${CARGO_ARGS[@]}"
cargo objcopy "${CARGO_ARGS[@]}" -- -O binary "$DIST/pineforge.bin"
"$PYTHON" "$MCUBOOT" create \
  --align 4 \
  --version "$VERSION" \
  --header-size 32 \
  --slot-size 475136 \
  --pad-header \
  "$DIST/pineforge.bin" \
  "$DIST/pineforge-image.bin"
"$NRFUTIL" dfu genpkg \
  --dev-type 0x0052 \
  --application "$DIST/pineforge-image.bin" \
  "$PACKAGE"

echo "Created: $PACKAGE"
