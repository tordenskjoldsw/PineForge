# Architecture

## Layers

- `board`: immutable PineTime hardware facts, especially pin assignments.
- `drivers`: device-local state machines implementing `embedded-hal` boundaries.
- `services`: product behavior independent from concrete peripherals.
- `tasks`: long-running single-owner tasks for display, input, watchdog, and future subsystems.
- `ui`: framebuffer-free rendering using `embedded-graphics`.
- `main`: composition root only; owns concrete peripherals and scheduling.

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
the cadence used by InfiniTime but publishes raw HRS values to the UI only once
per second, keeping display traffic independent from signal acquisition. The
diagnostics runner waits on an explicit startup barrier until touch and motion
have completed their shared-bus initialization, matching PineTime's proven
sequential peripheral bring-up rather than racing three clients at boot. The
runner disables the conversion engine and LED during system sleep and performs
a fresh settling delay after wake. Future settings and background-measurement
policy belong above this runner rather than in the register driver.

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
Production is limited to 384 KiB flash and 48 KiB static RAM; diagnostics is
limited to 448 KiB flash and 56 KiB static RAM. These limits preserve flash
headroom in the 475,104-byte MCUBoot application region and reserve RAM for
runtime stack growth. Size changes remain visible in CI output even when they
stay below the hard limits.

## Error policy

Drivers return typed errors. Top-level product policy decides whether to retry, degrade, or reset. During bring-up, log recoverable failures using `defmt` and continue where safe.
