#!/usr/bin/env bash
set -euo pipefail

# Holds the firmware crate's layer boundary, which is otherwise a convention
# that erodes one convenient import at a time. Every rule here was broken at
# least once before it was checked.
#
#   src/ipc.rs      the bus between tasks: declarations only
#   src/services/   executor-independent runners, generic over embedded-hal
#   src/tasks/      everything the executor runs; the only layer that may
#                   name the board
#
# The rule runs one way: a service may not reach down into the executor or the
# chip, but a task with nothing portable to extract needs no service half.

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

# A service that spawns is a task wearing the wrong name - which is what
# `services::power` and `services::settings` were.
if matches="$(scan 'embassy_executor::task' src/services/)" && [[ -n "$matches" ]]; then
  fail "a service declares an executor task; it belongs in src/tasks/"
  echo "$matches" >&2
fi

# The chip belongs to the layer that owns pins, so a runner stays testable
# against any embedded-hal implementation rather than one nRF52832.
if matches="$(scan 'embassy_nrf' src/services/)" && [[ -n "$matches" ]]; then
  fail "a service names embassy_nrf; bind the peripheral in src/tasks/ instead"
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
  echo "layers: ipc, services, and tasks are each within their boundary"
fi

exit "$status"
