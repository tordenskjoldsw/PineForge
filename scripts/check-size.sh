#!/usr/bin/env bash
set -euo pipefail

FEATURES="${1-}"
MAX_FLASH="${2:?maximum flash bytes are required}"
MAX_RAM="${3:?maximum RAM bytes are required}"

CARGO_ARGS=(--release)
if [[ -n "$FEATURES" ]]; then
  CARGO_ARGS+=(--features "$FEATURES")
fi

SECTIONS="$(cargo size "${CARGO_ARGS[@]}" -- -A)"
read -r FLASH RAM < <(
  awk '
    $1 == ".vector_table" || $1 == ".text" || $1 == ".rodata" || $1 == ".data" {
      flash += $2
    }
    $1 == ".data" || $1 == ".bss" || $1 == ".uninit" {
      ram += $2
    }
    END { print flash + 0, ram + 0 }
  ' <<<"$SECTIONS"
)

LABEL="${FEATURES:-production}"
echo "$LABEL: flash $FLASH / $MAX_FLASH bytes, RAM $RAM / $MAX_RAM bytes"

if (( FLASH > MAX_FLASH )); then
  echo "$LABEL exceeds its flash budget by $((FLASH - MAX_FLASH)) bytes" >&2
  exit 1
fi

if (( RAM > MAX_RAM )); then
  echo "$LABEL exceeds its RAM budget by $((RAM - MAX_RAM)) bytes" >&2
  exit 1
fi
