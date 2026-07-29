# Architecture

## Crates

Three, and the split is what decides what can be tested without a watch.

- `pineforge-state`: product state and protocol logic. No Embassy, no nRF, no
  drawing. Every rule that fails silently — a settings invariant, a paginated
  list's arithmetic, what a dismissal does to the notification cursor — lives
  here and is tested on the host.
- `pineforge-ui`: every screen, drawn against `Canvas`, which borrows any
  `Surface`. On the watch that is the panel; in a test it is a recording buffer,
  so layout and the opaque-drawing contract are checked on the host too. The
  `diagnostics` feature is the only thing that pulls in Embassy, because timing
  a transition needs a clock. `registry::Screens` owns one of every screen and
  answers which one a `ScreenId` names — the display task holds no screen table
  of its own, and the host tests walk `ScreenId::ALL` through the same registry,
  so a screen that exists is a screen they check.
- `pineforge`: the firmware. Peripherals, tasks, and the composition root — the
  part that genuinely cannot run anywhere but the watch.

**The rule the split enforces:** if a piece of logic can be wrong without being
visibly wrong, it does not belong in the firmware crate. The boundary costs
about 1.2 KiB of flash in lost cross-crate inlining, which is the price of being
able to fail a build instead of noticing on the wrist.

## Layers within the firmware crate

- `board`: immutable PineTime hardware facts, especially pin assignments.
- `drivers`: device-local state machines implementing `embedded-hal` boundaries.
- `ipc`: the bus between tasks — channels, watches, signals, and the reasoning
  for each one's depth. Declarations only.
- `services`: executor-independent runners. Generic over the `embedded-hal`
  traits they need; never name `embassy_nrf` and never declare a task.
- `tasks`: everything the executor runs. Binds board resources to a runner, or
  does the work itself where there is no portable half.
- `main`: composition root only; owns concrete peripherals and scheduling.

**The rule between `services` and `tasks`, and which way it runs.** A service
owns a subsystem's lifecycle and cadence and exposes `run()`; the task is the
few lines that hand it a concrete bus and spawn it. `AccelerometerRunner` and
`HeartRateRunner` are the shape — both are generic over `I2c`, so what a sensor
does over time is readable without a watch attached.

The rule is one-directional: a service may not reach down into the executor or
the chip, but **a task needs no service half.** Where there is nothing portable
to extract — a motor that pulses, a watchdog that is petted, deadline
arithmetic that only `embassy-time` can do — the task is the whole subsystem
and no wrapper is invented for symmetry.

`scripts/check-layers.sh` holds this in CI, because it is otherwise a
convention that erodes one convenient import at a time — which is exactly how
it eroded before: `services::power` and `services::settings` were both tasks,
the second owning the external flash, and `services::events` was a message bus
filed under product behavior. The script also fails a task module that
`main.rs` never spawns, so the layer's contents and the composition root cannot
drift apart.

## RAM

65,528 bytes, filled from both ends. Statics grow up from the bottom and are
exact — the linker vends every byte, and CI holds the total to a budget. The
stack grows down from the top and is not exact: it moves with call depth, and
the deepest it goes depends on which paths overlap.

Two decisions follow, and they are separate:

- **`flip-link` links the stack below the statics** rather than above them. On
  the default layout an overflow grows down into `.bss` and silently overwrites
  a suspended task's state, so the watch misbehaves later and elsewhere.
  Flipped, it grows out of RAM into unmapped addresses and faults at the point
  of overflow. Rearrangement only; no runtime cost.
- **The firmware measures its own high-water mark.** `src/boot/stack.rs` paints
  the region before `main` and the watchdog task reports the deepest point
  whenever it grows. Async is why the reserve can be modest at all: a task's
  state across its await points lives in a static the compiler sized exactly —
  `ble::run::POOL` is 11 KiB of it — so the stack only has to cover the deepest
  synchronous chain plus interrupts, not a worst case per task the way
  per-task stacks would.

The RAM budget in CI is a target, not the hardware ceiling. It exists to stop
the statics drifting into the space the stack needs; the size of that space is
what the measurement is for.

## Concurrency model

Embassy is the runtime. Each stateful peripheral is assigned to one long-running owner task. The current hardware-test baseline has:

- input task: owns the touch controller and publishes `UiEvent` values
- accelerometer runner: exclusively owns the BMA42x and its interrupt; the
  concrete Embassy task only binds PineTime peripherals and starts the runner
- diagnostics heart-rate runner: exclusively owns the HRS3300, its 100 ms
  acquisition cadence, and its power transitions
- display task: owns the LCD and backlight, consumes UI events, and renders the active `Screen`
- watchdog task: feeds the watchdog inherited from the bootloader
- rollback task: monitors the physical side button for an explicit test-image reset

Planned subsystem tasks include:

- power task: battery, charging, sleep policy
- BLE task: owns the radio/stack

Use bounded `embassy-sync` channels for events. The current UI channel has a fixed capacity of eight events. Never share the display or SPI peripheral behind a global mutex merely for convenience; prefer single-owner tasks and message passing.

Long-running services follow Embassy's runner pattern: executor-independent
runner objects own their state and expose `run()`, while small concrete task
functions bind board peripherals and spawn them. PineTime-specific register
sequences are cross-checked against InfiniTime and Bosch's Sensor API; their
central FreeRTOS `SystemTask` architecture is not copied into the async design.

The touch controller and accelerometer are an intentional exception at the bus
transport boundary because PineTime wires both devices to the same TWIM
peripheral. They receive independent `embedded-hal-async` I²C device handles
backed by Embassy's blocking shared-bus adapter. Short transactions are
serialized on the single thread executor. The latency-sensitive CST816S path
uses `BlockingAsync` directly, while motion additionally uses `YieldingAsync`
to preserve cooperative scheduling. Each driver still owns its local state,
and no driver can access another device.

The board layer programs TWIM with InfiniTime's nRF52832 workaround value of
approximately 390 kHz instead of the faulty exact-400-kHz timing. Every shared
transaction uses Embassy's bounded blocking API with a 10 ms timeout before it
is adapted to async device drivers, preventing a frozen peripheral from
starving the cooperative executor until the bootloader watchdog resets it.

## Power policy

The pure `SystemPowerPolicy` lives in `pineforge-state`, independently from
Embassy and hardware. A dedicated coordinator owns inactivity deadlines and
publishes the latest `SystemPowerState` through an Embassy `Watch`. Input sends
activity commands through a bounded channel; display and motion independently
subscribe to the resulting state. The display task only applies LCD and
backlight transitions and no longer decides global policy.

A validated `PowerConfig` currently supplies defaults of ten seconds until
idle and twenty seconds until sleep. A future settings service can replace
those defaults without changing consumers or hardware boundaries. The same
coordinator boundary is reserved for typed wake locks and a future nRF power
backend. While sleeping, periodic UI and motion updates are suspended. Wake
fully redraws the active screen before enabling the backlight, preventing stale
framebuffer content from becoming visible.

The battery task owns SAADC plus PineTime's active-low charge-status and
external-power inputs. It uses Embassy one-shot conversions every ten minutes
while discharging in production, every minute on external power, and every 30
seconds in diagnostics. Charger edges publish power flags immediately without
taking a capacity sample. A hardware-independent estimator applies the
InfiniTime direction rule and limits visible movement to one percentage point
per elapsed minute. This keeps diagnostic cadence from accelerating capacity
changes and prevents charger terminal voltage from becoming an immediate
percentage jump. The diagnostics UI retains raw millivolts for validation;
future low-voltage protection must continue to use that raw measurement.

Heart-rate bring-up remains diagnostics-only. The HRS3300 register layer owns
explicit configure, power-up, coherent register-block reads, and power-down
operations; BPM processing stays outside the driver. An executor-independent
runner owns the 100 ms acquisition cadence and sensor lifecycle. It samples at
the cadence used by InfiniTime and decodes coherent HRS and ambient-light values
from the same eight-byte transaction. It publishes the paired raw sample to the
UI only once per second, keeping display traffic independent from signal
acquisition. The
board startup uses one-shot Embassy signals to enforce touch, then motion, then
heart-rate shared-bus initialization, matching PineTime's proven sequential
peripheral bring-up rather than racing any clients at boot. The
runner disables the conversion engine and LED during system sleep and performs
a fresh settling delay after wake. Future settings and background-measurement
policy belong above this runner rather than in the register driver.

Heart-rate acquisition is command-driven and disabled by default. Entering the
dedicated diagnostics screen sends `Start`; leaving it or entering system sleep
sends or implies `Stop`. The runner remains the sole sensor owner and always
disables the conversion engine and LED when a session ends. This command
boundary is independent from display rendering and can later accept persistent
disabled, on-demand, continuous, or periodic measurement policy.

The diagnostics PPG processor is hardware-independent and heapless. It uses a
64-sample window at 10 Hz, linear detrending, a four-stage 0.5--4 Hz band-pass,
a generated Hann window, an in-place real FFT, unique-peak and signal-to-noise
validation, and three consistent overlapping windows before publishing a
40--230 BPM result. Host tests exercise synthetic 60 and 120 BPM signals plus
constant, competing-frequency, and ambient-light rejection. Raw samples remain
visible while the first 6.4-second window is collected; validated analysis
results then replace them in the same partial-redraw UI row.

## The display task: ingest before render

The task's event loop has two halves, and the order between them is a
correctness property rather than a style.

**Ingest** runs first and may never be skipped. Every model the task owns — the
status corner, and the shared `WatchState` the watchface renders — absorbs the
event here. An event is consumed from `UI_EVENTS` whichever way the rest of the
loop goes, so a model that misses one never sees it again: a dropped reading is
not shown late, it is not shown at all.

**Render** runs second and may be skipped freely. A sleeping watch paints
nothing, a modal claims the panel, an event that reaches no screen draws
nothing. All of those are `continue`, and all of them are safe precisely
because the models above are already current.

`AppEvent::is_reading()` is what tells the two halves apart. A reading is a fact
about the watch rather than something addressed to whichever screen is up, and
the watchface is the only screen that keeps one, so it is applied once during
ingest instead of being routed through the active screen. The tick is the sole
exemption: the display task raises it itself, only while awake, and only when
it is about to repaint, so no gate can drop it.

This ordering is what a firmware bug came down to. The dispatch to the active
screen sat below the sleep gate, and the one path that fed the face directly
covered two event kinds and only while the face was *not* showing — so the
ordinary case, face showing and watch asleep, dropped every reading it
received. With sleep at twenty seconds and a battery sample every ten minutes,
that was nearly all of them: the percentage sat where it was at boot, and a
charger on the pad never turned `BAT` into `CHG`, because that reading arrives
exactly when the panel is off. `WatchState`'s host tests now assert that
anything the state absorbs is something `is_reading()` routes to it.

## UI contract

Screens implement the `Screen` trait. A screen receives hardware-independent `UiEvent` values, updates its private state, renders through an `embedded-graphics` draw target, and may return a high-level `ScreenAction`. It does not own or access touch, SPI, BLE, or sensor peripherals directly. Watchfaces will use the same boundary with application state supplied by services.

## Memory policy

- no heap
- no full-screen RGB565 framebuffer
- fixed-capacity `heapless` containers
- bounded channels
- stack usage inspected from linker map
- release builds use LTO and size optimization

CI enforces capacity budgets rather than early-project baseline sizes.
Production is limited to 420 KiB flash and 44 KiB static RAM, diagnostics to
448 KiB and 56 KiB. Size changes remain visible in CI output even when they
stay below the limits.

Both are design targets rather than hard limits, but they are not held to the
same margin, because they fail in opposite ways:

- **RAM** is the tight one. A static that outgrows its budget eats into the
  stack, and the failure is silent, on hardware, and late. The reserve is a
  factor of two over a measured peak, which is why the budget stays where it
  is even though it leaves only a few hundred bytes free.
- **Flash** is the loose one. An image that outgrows the 475,104-byte slot is
  refused by imgtool at packaging time, so the worst case is a build that
  produces nothing. The target exists to catch unnoticed growth, not to
  prevent a failure. The earlier 360 KiB target left 104 KiB of the slot unused
  and had begun shaping features rather than catching bloat.

Two thirds of the production image is the BLE stack: a build without the `ble`
feature is 125,708 bytes.

The display-transition scratch is capped at 8 KiB and currently uses 5.6 KiB;
it is the one large allocation whose size trades purely against render time,
because the screen is composed once per stripe. BLE and DFU capacity reductions
require a complete OTA hardware test because their previous undersizing caused
an end-of-transfer deadlock.

The four largest static allocations are the BLE task future, the BLE packet
pool, this scratch, and the Nordic controller memory; together they hold about
70% of static RAM, while every screen in the firmware shares the display task's
544-byte future. Measure with:

```bash
rust-nm --print-size --size-sort --radix=d \
  target/thumbv7em-none-eabihf/release/pineforge |
  awk '$3 ~ /^[bBdD]$/ {print $2, $3, $4}' | tail -20
```

## Error policy

Drivers return typed errors. Top-level product policy decides whether to retry, degrade, or reset. During bring-up, log recoverable failures using `defmt` and continue where safe.
