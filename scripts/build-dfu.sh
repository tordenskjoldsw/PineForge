#!/usr/bin/env bash
set -euo pipefail

# Release/production, including normal UI animations, is the safe default.
# Diagnostic screens and render metrics must be requested explicitly.
FEATURES="${PINEFORGE_FEATURES-}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Defaults to the release in Cargo.toml so the package cannot be named after a
# version the firmware is not. Pass an argument to add build metadata - the
# `0.2.1+7` form that distinguishes two packages of the same release - since
# Cargo.toml carries the release alone.
VERSION="${1:-$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d '"' -f2)}"
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
  echo "adafruit-nrfutil is missing (checked .venv and PATH). Run ./scripts/setup-build-tools.sh" >&2
  exit 1
}
[[ -f "$MCUBOOT" ]] || {
  echo "tools/mcuboot-src/scripts/imgtool.py is missing. Run ./scripts/setup-build-tools.sh" >&2
  exit 1
}

# A package is named by its version alone, so a second build of the same version
# would replace the first without saying so - including the archive that was
# published for a release, whose checksum is quoted in the release notes and
# cannot be regenerated (adafruit-nrfutil records timestamps). Refuse instead.
[[ ! -e "$PACKAGE" || -n "${PINEFORGE_OVERWRITE-}" ]] || {
  # Suggest the next build of this release, not one appended to whatever
  # metadata is already there - `0.2.1+7+1` is not a version the check above
  # would accept.
  RELEASE="${VERSION%%+*}"
  BUILD="${VERSION##*+}"
  [[ "$BUILD" == "$VERSION" ]] && BUILD=0
  echo "$PACKAGE already exists." >&2
  echo "Pass build metadata to name a new one, for example $RELEASE+$((BUILD + 1))," >&2
  echo "or set PINEFORGE_OVERWRITE=1 to replace it." >&2
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
