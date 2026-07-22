# Roadmap

This roadmap is ordered by technical dependencies rather than target dates. Each
milestone should leave the sealed PineTime in a testable and recoverable state.

## Milestone 0 — reproducible bring-up baseline

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

### Remaining

- preserve the bring-up baseline with a version tag
- make build dependencies and the DFU process reproducible
- inspect and record flash, RAM, and stack usage
- replace intentionally ignored hardware errors with explicit product policy
- keep a repeatable sealed-device hardware test checklist

## Milestone 1 — UI foundation

- reusable button component with pressed, released, and disabled states
- centralized dirty-region rendering
- screen stack and typed navigation actions
- reusable status bar and layout primitives
- simple application launcher
- consistent side-button navigation behavior
- display timeout, sleep, wake, and redraw behavior
- touch gesture filtering and debouncing
- host-side tests for hit testing, state transitions, and navigation

## Milestone 2 — power and time

- battery voltage measurement through SAADC
- charging-state GPIO support
- calibrated battery percentage model
- monotonic software clock
- backlight levels and brightness policy
- inactivity and display timeout policy
- low-power sleep and wake behavior
- measured current consumption in active and sleeping states

## Milestone 3 — shared buses and motion sensor

- shared I2C bus manager for touch, accelerometer, and heart-rate sensor
- bounded transactions, error reporting, and bus recovery
- BMA421 device detection and initialization
- interrupt-driven motion samples
- basic step and wake-gesture data exposed as service events
- SPI ownership plan for the LCD and external flash

## Milestone 4 — persistent settings

- documented external-flash memory map that preserves MCUBoot and InfiniTime data
- external SPI-flash driver and arbitration
- versioned settings format
- atomic or recoverable settings updates
- persistence for brightness, timeouts, and watchface selection
- validation and migration of stored settings

## Milestone 5 — BLE and time synchronization

- documented BLE stack choice based on licensing, maintenance, RAM, and flash use
- advertising, connection, and reconnection lifecycle
- GATT Current Time Service
- GATT Battery Service
- time synchronization from a companion device
- initial Gadgetbridge compatibility testing
- memory-map and DFU compatibility preserved

## Milestone 6 — daily-use MVP

- time and date display
- one polished watchface
- battery and charging status
- reliable touch and side-button interaction
- display timeout and low-power sleep
- persistent user settings
- BLE time synchronization
- safe firmware updates
- separate unconfirmed bring-up and confirmable release build policies
- explicit self-test and image-confirmation flow for release builds

## Later milestones

- notifications
- alarms, timers, and stopwatch
- step counting and activity summaries
- heart-rate sampling
- weather
- music controls
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
