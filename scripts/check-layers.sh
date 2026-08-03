#!/usr/bin/env bash
set -euo pipefail

# Holds the firmware crate's layer boundary, which is otherwise a convention
# that erodes one convenient import at a time. Every rule here was broken at
# least once before it was checked.
#
#   src/ipc.rs      the bus between tasks: declarations only
#   src/tasks/      everything the executor runs; the only layer that may
#                   name the board
#
# The runners this file used to police now live in `crates/pineforge-services`,
# and their boundary is held by cargo instead: that crate does not depend on
# `embassy-nrf`, so a runner cannot reach the chip whatever it writes. What was
# checked here was the spelling `embassy_nrf` in `src/services/`, which a
# `use crate::drivers::backlight::Backlight` would have walked straight past.
#
# The rule still runs one way: a task with nothing portable to extract needs no
# runner half.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

status=0

fail() {
  echo "layer violation: $1" >&2
  status=1
}

# Greps code only. The rules are stated in the doc comments of the very
# modules they govern, and a check that reads its own explanation as a
# breach of itself is worse than no check.
scan() {
  grep -rn "$1" "${@:2}" 2>/dev/null | grep -vE ':[0-9]+:[[:space:]]*(//|\*)' || true
}

# The runner crate must stay buildable without a chip, which is the whole of
# why it is a crate. Cargo enforces the dependency; this catches the manifest
# edit that would hand it back.
if matches="$(scan 'embassy-nrf|embassy-executor' crates/pineforge-services/Cargo.toml)" \
  && [[ -n "$matches" ]]; then
  fail "pineforge-services depends on the chip or the executor; it must build on a host"
  echo "$matches" >&2
fi

# The bus is a wiring diagram. A loop in it is a task that no longer appears
# in src/tasks/, so the list of what the executor runs stops being complete.
if matches="$(scan 'embassy_executor::task' src/ipc.rs)" && [[ -n "$matches" ]]; then
  fail "src/ipc.rs declares a task; it holds channel declarations only"
  echo "$matches" >&2
fi

# Every task is spawned from the composition root, so a module that is never
# reached is dead weight that still compiles.
for module in src/tasks/*.rs src/tasks/*/; do
  name="$(basename "$module" .rs)"
  [[ "$name" == "mod" ]] && continue
  if ! grep -q "tasks::${name}::" src/main.rs; then
    fail "src/tasks/${name} is never spawned from src/main.rs"
  fi
done

if [[ "$status" -eq 0 ]]; then
  echo "layers: the bus, the tasks and the runner crate are each within their boundary"
fi

exit "$status"
