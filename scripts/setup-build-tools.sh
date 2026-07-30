#!/usr/bin/env bash
# Installs everything `scripts/build-dfu.sh` needs, at pinned versions.
#
# Every version this script chooses is written down here rather than resolved at
# install time. A DFU package is the artifact people flash onto a sealed watch,
# so "the same PineForge commit built twice" has to mean the same image both
# times - including on a machine that set the tooling up months later.
#
# Safe to re-run: an existing checkout is fetched and reset rather than
# re-cloned, and pip is asked for exact versions it may already have.
set -euo pipefail

# Upstream MCUBoot ships imgtool, which writes the 32-byte header the bootloader
# reads at boot. This is `mcu-tools/mcuboot` and it is a host build tool only -
# not the bootloader on the watch. That one is
# `InfiniTimeOrg/pinetime-mcuboot-bootloader`, versioned 1.0.x on its own
# schedule; PineForge neither builds nor touches it, and only has to write a
# header it can read.
# Pinned to a release tag and verified by commit, because a tag can be moved and
# the header format is what makes an image bootable at all. v2.4.0's imgtool was
# checked against the unpinned `main` checkout used through v0.2.1: identical
# output bytes for the PineTime image parameters, so this pin does not
# invalidate the reproducibility claim in the v0.2.1 release notes.
MCUBOOT_TAG="v2.4.0"
MCUBOOT_COMMIT="6d3b3d2c38ab20c242e5b9abb04d050086383eb2"

# The linker (see .cargo/config.toml) and `cargo size`, used by the size budget.
FLIP_LINK_VERSION="0.1.12"
CARGO_BINUTILS_VERSION="0.4.0"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MCUBOOT_SRC="$ROOT/tools/mcuboot-src"
VENV="$ROOT/.venv"

command -v git >/dev/null || { echo "git is missing" >&2; exit 1; }
command -v rustup >/dev/null || {
  echo "rustup is missing - install it from https://rustup.rs" >&2
  exit 1
}
command -v python3 >/dev/null || { echo "python3 is missing" >&2; exit 1; }

# rust-toolchain.toml already pins the channel, target and components; this only
# makes rustup act on it now instead of during the first build.
echo "==> Rust toolchain (pinned by rust-toolchain.toml)"
rustup show active-toolchain >/dev/null

echo "==> flip-link $FLIP_LINK_VERSION and cargo-binutils $CARGO_BINUTILS_VERSION"
cargo install flip-link --version "$FLIP_LINK_VERSION" --locked
cargo install cargo-binutils --version "$CARGO_BINUTILS_VERSION" --locked

echo "==> MCUBoot $MCUBOOT_TAG in tools/mcuboot-src"
if [[ ! -d "$MCUBOOT_SRC/.git" ]]; then
  # blob:none keeps this to the trees and the blobs actually checked out; the
  # full history of MCUBoot is not something a firmware build needs.
  git clone --filter=blob:none --no-checkout \
    https://github.com/mcu-tools/mcuboot.git "$MCUBOOT_SRC"
fi
git -C "$MCUBOOT_SRC" fetch --filter=blob:none origin "$MCUBOOT_COMMIT"
git -C "$MCUBOOT_SRC" checkout --quiet --force "$MCUBOOT_COMMIT"

ACTUAL_COMMIT="$(git -C "$MCUBOOT_SRC" rev-parse HEAD)"
[[ "$ACTUAL_COMMIT" == "$MCUBOOT_COMMIT" ]] || {
  echo "MCUBoot checkout is $ACTUAL_COMMIT, expected $MCUBOOT_COMMIT" >&2
  exit 1
}

echo "==> Python environment in .venv"
[[ -d "$VENV" ]] || python3 -m venv "$VENV"
"$VENV/bin/pip" install --quiet --upgrade pip
"$VENV/bin/pip" install --quiet --requirement "$ROOT/tools/requirements.txt"

echo
echo "Ready. Build a DFU package with:"
echo "  ./scripts/build-dfu.sh $(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d '"' -f2)"
