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

- an explicit self-test before a release build invites confirmation
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
  transport built from rectangles. *Unreleased.* It is the first thing on the
  event bus that travels from the UI outwards, and the first component with a
  disabled state - with no phone connected there is nothing its three buttons
  could do. Not yet exercised against a running Gadgetbridge; see
  [`TESTED-CONFIGURATIONS.md`](TESTED-CONFIGURATIONS.md).

### Open

- notification text in a face the panel can show - the screen reads the ASCII
  range the atlas covers, so accented characters draw as gaps
- alarms, timers, and stopwatch
- activity summaries over time; steps are counted and shown for the day, but
  not retained across days
- weather
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
