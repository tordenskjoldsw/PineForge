# Architecture

## Crates

Three, and the split is what decides what can be tested without a watch.

- `pineforge-state`: product state and protocol logic. No Embassy, no nRF, no
  drawing. Every rule that fails silently - a settings invariant, a paginated
  list's arithmetic, what a dismissal does to the notification cursor - lives
  here and is tested on the host.
- `pineforge-ui`: every screen, drawn against `Canvas`, which borrows any
  `Surface`. On the watch that is the panel; in a test it is a recording buffer,
  so layout and the opaque-drawing contract are checked on the host too. The
  `diagnostics` feature is the only thing that pulls in Embassy, because timing
  a transition needs a clock. `registry::Screens` owns one of every screen and
  answers which one a `ScreenId` names - the display task holds no screen table
  of its own, and the host tests walk `ScreenId::ALL` through the same registry,
  so a screen that exists is a screen they check.
- `pineforge`: the firmware. Peripherals, tasks, and the composition root - the
  part that genuinely cannot run anywhere but the watch.

**The rule the split enforces:** if a piece of logic can be wrong without being
visibly wrong, it does not belong in the firmware crate. The boundary costs
about 1.2 KiB of flash in lost cross-crate inlining, which is the price of being
able to fail a build instead of noticing on the wrist.

## Layers within the firmware crate

- `board`: immutable PineTime hardware facts, especially pin assignments.
- `drivers`: device-local state machines implementing `embedded-hal` boundaries.
- `ipc`: the bus between tasks - channels, watches, signals, and the reasoning
  for each one's depth. Declarations only.
- `services`: executor-independent runners. Generic over the `embedded-hal`
  traits they need; never name `embassy_nrf` and never declare a task.
- `tasks`: everything the executor runs. Binds board resources to a runner, or
  does the work itself where there is no portable half.
- `main`: composition root only; owns concrete peripherals and scheduling.

**The rule between `services` and `tasks`, and which way it runs.** A service
owns a subsystem's lifecycle and cadence and exposes `run()`; the task is the
few lines that hand it a concrete bus and spawn it. `AccelerometerRunner` and
`HeartRateRunner` are the shape - both are generic over `I2c`, so what a sensor
does over time is readable without a watch attached.

The rule is one-directional: a service may not reach down into the executor or
the chip, but **a task needs no service half.** Where there is nothing portable
to extract - a motor that pulses, a watchdog that is petted, deadline
arithmetic that only `embassy-time` can do - the task is the whole subsystem
and no wrapper is invented for symmetry.

`scripts/check-layers.sh` holds this in CI, because it is otherwise a
convention that erodes one convenient import at a time - which is exactly how
it eroded before: `services::power` and `services::settings` were both tasks,
the second owning the external flash, and `services::events` was a message bus
filed under product behavior. The script also fails a task module that
`main.rs` never spawns, so the layer's contents and the composition root cannot
drift apart.

## RAM

65,528 bytes, filled from both ends. Statics grow up from the bottom and are
exact - the linker vends every byte, and CI holds the total to a budget. The
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
  state across its await points lives in a static the compiler sized exactly -
  `ble::run::POOL` is 12 KiB of it - so the stack only has to cover the deepest
  synchronous chain plus interrupts, not a worst case per task the way
  per-task stacks would.

The RAM budget in CI is a target, not the hardware ceiling. It exists to stop
the statics drifting into the space the stack needs; the size of that space is
what the measurement is for.

## Concurrency model

Embassy is the runtime. Each stateful peripheral is assigned to one long-running
owner task. The production firmware has:

- input task: owns the touch controller and publishes `UiEvent` values
- accelerometer runner: exclusively owns the BMA42x and its interrupt; the
  concrete Embassy task only binds PineTime peripherals and starts the runner
- heart-rate runner: exclusively owns the HRS3300, its 100 ms
  acquisition cadence, and its power transitions
- power task: owns the inactivity deadlines and publishes the system power state
- battery task: owns the SAADC and both charger-status inputs
- display task: owns the LCD and backlight, consumes UI events, and renders the
  active `Screen`
- storage task: owns the external flash and serializes settings, bonds, and DFU
  writes
- BLE task: owns the radio and stack, GATT services, pairing, time sync, music,
  and the DFU protocol
- button task: owns the side button, including back, panel-off, and held reset
- vibration task: owns the motor and plays bounded haptic patterns
- watchdog task: feeds the watchdog inherited from the bootloader

Use bounded `embassy-sync` channels for events. The current UI channel has a
fixed capacity of eight events. Never share the display or SPI peripheral
behind a global mutex merely for convenience; prefer single-owner tasks and
message passing.

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

A validated `PowerConfig` supplies the persisted dim and sleep timeouts without
changing consumers or hardware boundaries. The same coordinator boundary is
reserved for typed wake locks and a future nRF power backend. Wake fully
redraws the active screen before enabling the backlight, preventing stale
framebuffer content from becoming visible.

The persisted wake-gesture picker offers independent switches for single tap,
double tap, and raise wrist, so any combination can be active. The side button
and charger remain unconditional wake sources even when all three switches are
off. While the panel sleeps, the input task discards every touch except an
enabled CST816S gesture, including the complete touch that caused wake so it
cannot activate the screen underneath. Whenever raise wrist is enabled, the
motion service maintains InfiniTime's eight-sample, 10 Hz BMA42x history before
and during sleep. The hardware-independent detector compares the stable newest
pair with the stable pair from roughly 0.6 seconds earlier and requires a
rotation of more than roughly 45 degrees into the PineTime viewing orientation.
Without raise wrist, motion sampling is suspended while sleeping, so touch-only
combinations do not pay the tilt-wake power cost.

The battery task owns SAADC plus PineTime's two active-low charger inputs, and
**watches both of them for changes**. Which one moves when a watch is set down
depends on how full it is: a flat one starts charging and moves both, but a
nearly full one never enters constant current, so the charge indication stays
put and external power is the only thing that changes. Watching one and merely
reading the other - the charge pin here, the power pin in InfiniTime - misses
one of those two cases each. A change on either pin also sends a
`PowerCommand::UserActivity`, so the watch lights up when it is set down, which
is what InfiniTime's `GoToRunning()` does on the same event.

What the pins *mean* is not decided in the task. `ChargerPins` in
`pineforge-state` turns a pair of levels into `charging`, and `PowerSource`
turns a `BatteryStatus` into the `CHG`/`PWR`/`BAT` tag the watchfaces show -
each with host tests over all four pin combinations, including the two that
only occur when something is wrong. Charging requires *both* pins: the charge
pin has no pull, so an unpowered charger can leave it floating, and that pair
must not read as a watch charging on a desk.

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

Heart-rate acquisition is part of the production firmware. The HRS3300 register
layer owns explicit configure, power-up, coherent register-block reads, and
power-down operations; BPM processing stays outside the driver. An
executor-independent runner owns the 100 ms acquisition cadence and sensor
lifecycle. It samples at the cadence used by InfiniTime and decodes coherent
HRS and ambient-light values from the same eight-byte transaction. Diagnostics
publishes the paired raw sample to the UI only once per second, keeping display
traffic independent from signal acquisition; production feeds the same 10 Hz
stream directly into the PPG processor. Board startup uses one-shot Embassy
signals to enforce touch, then motion, then heart-rate shared-bus
initialization, matching PineTime's proven sequential peripheral bring-up
rather than racing any clients at boot. The runner is deliberately independent
of the system power state: a reading has to be able to happen while the watch
is asleep, or a background interval means
nothing on a watch that sleeps twenty seconds after every touch. It used to
abandon a measurement the moment the display went dark, in three places, which
made the periodic setting effectively unreachable - the same shape of bug as a
charger reading that only arrived while the panel was on. What limits the
sensor now is the setting alone. Settings and background-measurement policy
belong above this runner rather than in the register driver.

Heart-rate acquisition is command-driven and disabled by default. The pulse app
asks for a reading now with `MeasureNow` and gives up on one with `Stop`;
neither disturbs the background setting. That setting is a single list, as
InfiniTime presents it - off, continuous, then intervals from thirty seconds to
thirty minutes - where continuous is an interval of zero rather than a mode
beside them, so the service needs no second persisted concept to honour it.
Interval and on-demand sessions stop after their first validated result (or
after the acquisition limit), but continuous mode keeps the sensor and PPG
window active and publishes every later validated BPM result. The watchface
therefore keeps updating after the first value without flashing back to a
measuring state between estimates, matching InfiniTime's foreground
acquisition behavior. The runner remains the sole sensor owner and always
disables the conversion engine and LED when a session ends. This command
boundary is independent from display rendering; persistent disabled, on-demand,
continuous, and periodic policies all reach the same runner through it.

The PPG processor is hardware-independent and heapless. It uses a
64-sample window at 10 Hz, linear detrending, a four-stage 0.5--4 Hz band-pass,
a generated Hann window, an in-place real FFT, unique-peak and signal-to-noise
validation, and three consistent overlapping windows before publishing a
40--230 BPM result. Host tests exercise synthetic 60 and 120 BPM signals plus
constant, competing-frequency, and ambient-light rejection. Raw samples remain
visible while the first 6.4-second window is collected; validated analysis
results then replace them in the same partial-redraw diagnostics UI row. The
production UI receives only session state and validated BPM results.

## The display task: ingest before render

The task's event loop has two halves, and the order between them is a
correctness property rather than a style.

**Ingest** runs first and may never be skipped. Every model the task owns - the
status corner, and the shared `WatchState` the watchface renders - absorbs the
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
ingest instead of being routed through the active screen. Ordinary repaint
ticks are the exemption: the display task raises them itself only while awake.
A running countdown adds one deliberate deadline to the sleeping select, so
reaching zero becomes an input that wakes the UI rather than a repaint that
waits until something else does.

This ordering is what a firmware bug came down to. The dispatch to the active
screen sat below the sleep gate, and the one path that fed the face directly
covered two event kinds and only while the face was *not* showing - so the
ordinary case, face showing and watch asleep, dropped every reading it
received. With sleep at twenty seconds and a battery sample every ten minutes,
that was nearly all of them: the percentage sat where it was at boot, and a
charger on the pad never turned `BAT` into `CHG`, because that reading arrives
exactly when the panel is off. `WatchState`'s host tests now assert that
anything the state absorbs is something `is_reading()` routes to it.

## The one bus that runs outwards

Every channel in `ipc` carries a subsystem reporting to the display: a battery
reading, a notification, a DFU result. Music control is the first that goes the
other way, and it is worth naming because the shape is different.

A control on the watch puts a `MusicControl` on `MUSIC_CONTROL`, and the BLE
task turns it into one notified byte on the music service's event
characteristic. Two rules keep that honest. The channel is **emptied when a
connection opens**, so a button pressed with no phone connected cannot fire
minutes later on reconnect - the same reasoning that stops a DFU flash result
from one connection being reaped by the next. And the sender uses `try_send`,
so a stalled radio drops the command rather than blocking a repaint: a dropped
transport command is a button that did nothing, which is recoverable by
pressing it again.

The state comes back on a `Watch` rather than a channel. The phone reports one
field per characteristic, so a track change arrives as a burst of writes; a
bounded channel would have to block the GATT loop or drop one, and a dropped
write leaves the wrong artist standing under the right title. The BLE task
assembles the record and publishes it whole, so a subscriber that misses an
update misses nothing.

**Where the elapsed time is anchored, and why it has to be there.** The phone
writes a position when it changes, not once a second, so the watch keeps it
moving itself. It does that by storing the position with the uptime it arrived
at rather than by counting seconds - the music screen does not hold the panel
on, so a counter would fall behind by exactly however long the watch slept. The
anchoring happens in the *display* task, not the BLE task, and that is not a
detail: the two tasks read their clocks from different zeros, and the elapsed
time is a difference between two readings. Only the task that raises the tick
can supply the base the tick uses.

## Countdown deadline and alarm ownership

The timer is not decremented by UI ticks. `TimerState` stores one monotonic
deadline while running and one remaining duration while paused; every displayed
second is derived from those values. The display task already owns the retained
screen models, Embassy time and the system-modal boundary, so it schedules that
deadline beside its normal repaint wake instead of adding a second task and an
IPC copy of the same clock. This also avoids another statically allocated task
future in the nRF52832's tight RAM budget.

The deadline remains in both the awake and sleeping `select`. Crossing it once
changes the retained model to `Expired`, latches the distinct alarm haptic and
raises `Modal::TimerExpired`; showing that modal sends the existing activity
command, which wakes the power coordinator and panel. Pairing, DFU and storage
formatting outrank the alarm, but `ModalState` retains a pending bit and restores
the alarm when the higher-priority flow closes. Only deliberate touch, swipe or
side-button input dismisses it; Bluetooth state changes cannot acknowledge an
alarm on the user's behalf. The alarm repeats for about five seconds, and the
same acknowledgement sends a cancellation signal so the motor stops at once.

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
Production is limited to 420 KiB flash and 48 KiB static RAM, diagnostics to
448 KiB and 56 KiB. The production RAM target reserves 16 KiB of the
65,528-byte RAM region for stack growth; diagnostics reserves 8 KiB. Size
changes remain visible in CI output even when they stay below the limits.

Both are design targets rather than hard limits, but they are not held to the
same margin, because they fail in opposite ways:

- **RAM** is the tight one. A static that outgrows its budget eats into the
  stack, and the failure is silent, on hardware, and late. The production
  reserve is 16 KiB against a measured 10,432-byte peak. The current production
  image uses 48,316 bytes of static RAM, 836 bytes below that design target.
- **Flash** is the loose one. An image that outgrows the 475,104-byte slot is
  refused by imgtool at packaging time, so the worst case is a build that
  produces nothing. The target exists to catch unnoticed growth, not to
  prevent a failure. The earlier 360 KiB target left 104 KiB of the slot unused
  and had begun shaping features rather than catching bloat.

BLE remains the dominant flash contributor. For scale, the current production
image uses 421,448 bytes of flash, while a build without default features uses
154,280 bytes.

The display-transition scratch is capped at 8 KiB and currently uses 5.6 KiB;
it is the one large allocation whose size trades purely against render time,
because the screen is composed once per stripe. BLE and DFU capacity reductions
require a complete OTA hardware test because their previous undersizing caused
an end-of-transfer deadlock.

The four largest static allocations are the BLE task future, the BLE packet
pool, this scratch, and the Nordic controller memory; together they hold about
two thirds of static RAM. Every screen in the firmware shares the display task
future rather than owning one future apiece. Measure with:

```bash
rust-nm --print-size --size-sort --radix=d \
  target/thumbv7em-none-eabihf/release/pineforge |
  awk '$3 ~ /^[bBdD]$/ {print $2, $3, $4}' | tail -20
```

## Error policy

Drivers return typed errors. The state crate owns the product policies that have
been made explicit - DFU, storage, pairing, and modal precedence - while sensor
and I2C failures are still handled locally by their runners. A general policy
for a bus or sensor that stops answering remains open. Recoverable failures are
logged with `defmt`, and a failed subsystem exits or retries without bringing
down unrelated tasks where that is safe.
