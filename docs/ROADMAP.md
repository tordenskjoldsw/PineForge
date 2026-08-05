# Roadmap

This roadmap is ordered by technical dependencies rather than target dates. Each
milestone should leave the sealed PineTime in a testable and recoverable state.

## Current release direction

- `v0.1.0`: released experimental OTA proof of concept
- `v0.2.0`: released platform foundation - state-owned product policy, side
  button as back button, persistent BLE pairing
- `v0.2.1`: released gesture and memory fixes - a gesture consumes its own
  touch, and static RAM meets its design target
- `v0.3.0`: the navigation shell v0.2.0 shipped without, and the first
  applications to stand on it - launcher, notifications, flashlight, pulse,
  settings split into a root and a leaf per setting, and a watch that says which
  build it is running
- `v0.4.0`: released look and speed - the FORGE watchface as the default and
  finished in its own numerals, a steps application, notifications on a card, a
  reading size for prose, and a set of drawing changes measured rather than
  guessed at
- `v0.5.0`: released music state and transport over the `InfiniTime` service,
  with a dedicated FORGE-style application, plus a three-page
  About/system-status view. Both have run on the known watch. Music sleep,
  reconnection and media-application switching, rare status failures and other
  PineTime variants remain untested.
- `v0.6.0`: released FORGE stopwatch and countdown applications, including a
  system-wide wake and alarm path, plus separate TIME and DATE editors that
  work without BLE and journal CRC-checked checkpoints in external flash.
  Interaction, cell-level partial rendering, restoration across reboots and a
  subsequent phone-time override passed on the known watch.
- `v0.6.1` development: a dedicated Bluetooth application persistently enables
  or disables advertising and connections. OFF removes the status rune;
  disabling is refused during DFU. Model, migration and rendering paths are
  host-tested. Runtime disconnect, advertising restart, status-rune removal and
  persistence across reboots passed with the `0.6.1+1` image on the known watch.
- `v0.6.1` development: vertical transitions slide through the panel's own
  scroll window instead of being revealed in strips, which costs one frame
  rather than one frame per step. The launcher pages on that axis with it, and
  returns to the watchface from its first page. The claim rule that makes
  paging on the entry axis safe is host-tested; the movement itself passed with
  the `0.6.1+2` image on the known watch.
- `v0.6.2` development: a battery application shows the charge in the
  watchface's numerals with a drawn percent sign, whether the watch is
  charging, charged or discharging, and the cell voltage the estimate came
  from. The three charger states, the voltage formatting and the partial
  redraw are host-tested; the screen passed with the `0.6.2+1` image on the
  known watch.
- `v1.0.0`: released - the direction reverses, and what the watch measures
  leaves it. Steps and heart rate go out over the services InfiniTime defines,
  a notification arrives with the name of whoever sent it, and weather comes in
  from the phone as conditions and a five-day forecast drawn in nine symbols.
  Behind that, the sensor runners moved into `crates/pineforge-services`, so a
  layer boundary a grep used to guard is checked by `cargo` instead, and the RAM
  budget became a bound derived from a hardware measurement rather than an
  inherited design target - after twice putting the stack under what a pairing
  needs. The release number and the number reported over Bluetooth are separate
  constants from here on, which is why the development images between `v0.6.1`
  and this tag were numbered `0.6.2+n` and then `0.15.0+n`: they are steps
  toward it, not releases of their own. It runs on both watches as the
  `1.14.0+1` package, which differs from the tag only in which constant feeds
  the firmware revision - and both produce the same string.
- `v1.0.1`: released - the About build page names the version reported over
  Bluetooth beside the one PineForge calls itself, from the same constant the
  characteristic is built from. `v1.0.0` made the two numbers disagree on
  purpose and documented why; this is what lets the watch answer for that
  without the release notes in hand. Eight bytes of static RAM and no
  behavioural change. Flashed as the `1.0.0+1` package and seen on the watch.

Work that has landed on `main` but not in a tagged release is marked
*unreleased* below. Everything else in a **Complete** section has shipped.

[`V0.2-PLATFORM-FOUNDATION.md`](V0.2-PLATFORM-FOUNDATION.md) holds the scope and
acceptance criteria that shaped `v0.2`, and the two criteria carried forward
into `v0.3`. It is a design record, not a plan for current work.

## Milestone 0 - reproducible bring-up baseline

### Complete

- MCUBoot-compatible application layout and Gadgetbridge DFU package
- continuous integration for formatting, Clippy, host tests, release builds, and size budgets
- unconfirmed test-image workflow with explicit rollback
- physical side-button reset and rollback
- bootloader watchdog handover
- ST7789 display initialization
- InfiniTime-compatible CST816S initialization and interrupt handling
- touch press/release events and partial display updates
- current stable Rust, Embassy, and display dependencies
- initialized Git repository
- a version tag per baseline: `v0.1.0`, `v0.2.0`, `v0.2.1`
- recorded flash, RAM, and stack usage, enforced as CI budgets rather than only
  documented - see [`ARCHITECTURE.md`](ARCHITECTURE.md)
- a repeatable sealed-device test procedure -
  see [`SEALED-PINETIME-TESTING.md`](SEALED-PINETIME-TESTING.md)
- reproducible build tooling: pinned toolchain, pinned MCUBoot
  checkout for `imgtool`, pinned Python tooling, and a committed `Cargo.lock`.
  The MCUBoot image is byte-reproducible from a given commit; the DFU archive
  around it is not, because `adafruit-nrfutil` records timestamps.

### Remaining

- explicit product policy for sensor and bus errors. The system paths - DFU,
  storage, pairing, modal precedence - are modelled in the state crate and
  host-tested; I²C and sensor failures below them are still handled locally at
  the driver.

## Milestone 1 - UI foundation

### Complete

- centralized dirty-region rendering
- screen stack and typed navigation actions
- reusable status bar and layout primitives
- consistent side-button navigation behavior
- display timeout, sleep, wake, and redraw behavior
- touch gesture filtering and debouncing
- host-side tests for hit testing, state transitions, and navigation
- simple application launcher

### Superseded

- reusable button component with pressed, released, and disabled states. The
  screens are built from menu rows, launcher tiles and setting pickers, each
  host-tested against a recording surface; a general button never earned its
  place.

## Milestone 2 - power and time

### Complete

- battery voltage measurement through SAADC
- charging-state GPIO support
- calibrated battery percentage model
- monotonic software clock
- backlight levels and brightness policy
- inactivity and display timeout policy
- low-power sleep and wake behavior

### Remaining

- measured current consumption in active and sleeping states. Nothing here has
  been measured on hardware, so PineForge makes no battery-life claim.

## Milestone 3 - shared buses and motion sensor

### Complete

- shared I2C bus manager for touch, accelerometer, and heart-rate sensor
- BMA421 device detection and initialization
- interrupt-driven motion samples
- basic step and wake-gesture data exposed as service events
- SPI ownership plan for the LCD and external flash

### Remaining

- bounded transactions and error reporting across the shared bus. Recovery after
  a motion-sensor reset exists; a general policy for a bus that stops answering
  does not.
- validate and tune step counting against a counted walk. The count is produced
  and shown, and has never been checked against anything.
- validate heart-rate readings against a reference monitor, and measure what the
  background sampling interval costs in battery life. Both are unknown.

## Milestone 4 - persistent settings

### Complete

- documented external-flash memory map that preserves MCUBoot and InfiniTime
  data - see [`FLASH-MAP.md`](FLASH-MAP.md)
- external SPI-flash driver and arbitration
- versioned settings format
- atomic or recoverable settings updates
- persistence for brightness, timeouts, and watchface selection
- validation and migration of stored settings. Every format version back to the
  first is still read and migrated on decode, with a test per version; a record
  whose CRC or version does not check out falls back to defaults rather than
  being misread.

## Milestone 5 - BLE, time synchronization, and OTA

### Complete

- BLE stack choice (TrouBLE host + Nordic SoftDevice Controller via nrf-sdc)
- advertising, connection, and passkey-bonded pairing with persistent bonds
- GATT Current Time, Battery, and Device Information services
- time synchronization from Gadgetbridge, shown on the watchface
- Nordic legacy DFU service for over-the-air updates via Gadgetbridge
- MCUBoot image confirmation, gating DFU until the image is confirmed
- memory-map and DFU compatibility with the stock bootloader preserved

## Milestone 6 - daily-use MVP

This is the milestone that decides whether PineForge is usable as a watch rather
than as a firmware experiment. It is not reached.

### Complete

- time and date display
- battery and charging status
- reliable touch and side-button interaction
- display timeout and low-power sleep
- persistent user settings
- BLE time synchronization
- safe firmware updates
- separate unconfirmed bring-up and confirmable release build policies
- image-confirmation flow for release builds

### Complete

- one polished watchface. FORGE is a designed face rather than a bring-up
  aesthetic, and it is what a watch with no stored choice opens with; `Terminal`
  remains as the borrowed one. Its readings are set in the same numerals as its
  clock, so the face carries no font at all.

### Remaining

- an explicit self-test before a release build invites confirmation. The human
  half of this exists as [`PRE-RELEASE.md`](PRE-RELEASE.md), written out of
  three regressions that reached hardware while every automated gate stayed
  green; what the watch could check about itself, unattended, is still nothing.
- measured battery life over a normal day

## Later milestones

### Complete

- step counting exposed as service events, and an application showing the day's
  count against a goal
- heart-rate sampling: an on-demand reading and a background interval
- a first pair of applications - flashlight and pulse - behind the launcher
- a second watchface, selectable and persisted: FORGE draws its numerals from
  rectangles rather than from a glyph atlas, which is why it fits in 2.2 KB and
  paints faster than a face made of text
- notifications received and read on a screen of their own, one per page, on a
  card, with dismissal. Deliberately no tally on the watchface: a count there
  would be a second place to keep the same fact right.
- music control over `InfiniTime`'s music service, in the FORGE look: the
  elapsed time in the watchface's numerals, the shared progress bar, and a
  transport built from rectangles. It is the first thing on the
  event bus that travels from the UI outwards, and the first component with a
  disabled state - with no phone connected there is nothing its three buttons
  could do. On the known Gadgetbridge setup it shows title and artist, controls
  previous and next, pauses and resumes playback, and changes volume in both
  directions; the remaining robustness cases are recorded in
  [`TESTED-CONFIGURATIONS.md`](TESTED-CONFIGURATIONS.md).
- build and system diagnostics in About, paged vertically: build identity and
  bootloader; touch, motion, pulse and external-flash probe results including
  the JEDEC ID; and image confirmation, BLE state, stack high-water mark and
  the latest retained fault. These are typed service events kept current
  through sleep rather than strings recovered from logs. All three pages have
  been flashed and displayed on the known watch; rare failure states and other
  hardware variants remain untested.
- a FORGE-style stopwatch with start, pause, resume and reset.
  Its elapsed time is anchored to monotonic uptime rather than repaint ticks,
  and controls, navigation away and display sleep have passed on the known
  watch as well as in host tests.
- a 1-to-99-minute FORGE countdown with pause, resume and cancel.
  A monotonic deadline remains scheduled while another app is showing or the
  panel sleeps; at zero it raises a full-screen dismissible modal, wakes the
  display and plays a cancellable alarm pattern for about five seconds. Its
  primary labels use a dedicated heavier FORGE cut. Model, modal precedence and
  rendering are host-tested, and the final interaction and alarm path passed on
  hardware.
- what the watch measures, sent outwards: the step count over InfiniTime's
  motion service and the heart rate over the standard Heart Rate Measurement
  characteristic. The count resets at the date change and publishes the zero at
  once, because a companion reads it as steps *so far today* and its daily
  accounting turns on receiving that zero. Only validated heart-rate results go
  out, and only changes - a phone charting a zero would be charting a sensor
  that was not there.
- weather from the phone over InfiniTime's Simple Weather service, on a screen
  of its own: the current conditions as one of nine 24x24 symbols over a
  temperature in the watchface's numerals, and a five-day forecast under it.
  Conditions and forecast arrive as separate writes and are held apart, so
  whichever one a phone sends is drawn without waiting for the other.

### Open

- notification text in a face the panel can show - the screen reads the ASCII
  range the atlas covers, so accented characters draw as gaps
- wall-clock alarms and reminders can reuse the countdown's system wake and
  modal path once the timer has passed hardware validation
- activity summaries over time; steps are counted and shown for the day, but
  not retained across days
- weather that arrives without being asked for. Gadgetbridge pushes a record
  only when a broadcast reaches it while its service is running, never on
  connect, so a record that arrived before the watch did stays where it is.
  Everything seen on a watch so far was sent from the debug menu.
- room for the next feature before the next feature. The statics sit close
  enough to the ceiling that a GATT characteristic or another large buffer has
  to be paid for rather than added, and the ceiling is a measured stack reserve
  rather than a number anyone may raise. `./scripts/check-size.sh` prints what
  is left; [`ARCHITECTURE.md`](ARCHITECTURE.md) explains why it is not a
  preference.
- additional watchfaces and applications
- external resource packages
- broader companion-app and OTA integration

## Project constraints

- preserve the stock PineTime MCUBoot bootloader and recovery path
- keep bring-up images unconfirmed and recoverable through an explicit reset
- do not write outside explicitly assigned external-flash regions
- use no heap and no full-screen RGB565 framebuffer
- use bounded channels and fixed-capacity data structures
- assign each stateful peripheral to a single owner task
- require formatting, Clippy, and release builds before hardware testing
