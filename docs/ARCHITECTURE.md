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
- display task: owns the LCD and backlight, consumes UI events, and renders the active `Screen`
- watchdog task: feeds the watchdog inherited from the bootloader
- rollback tasks: monitor the physical side button and the test-image safety timeout

Planned subsystem tasks include:

- sensor task: accelerometer and heart-rate sampling
- power task: battery, charging, sleep policy
- BLE task: owns the radio/stack

Use bounded `embassy-sync` channels for events. The current UI channel has a fixed capacity of eight events. Never share the display or SPI peripheral behind a global mutex merely for convenience; prefer single-owner tasks and message passing.

## UI contract

Screens implement the `Screen` trait. A screen receives hardware-independent `UiEvent` values, updates its private state, renders through an `embedded-graphics` draw target, and may return a high-level `ScreenAction`. It does not own or access touch, SPI, BLE, or sensor peripherals directly. Watchfaces will use the same boundary with application state supplied by services.

## Memory policy

- no heap
- no full-screen RGB565 framebuffer
- fixed-capacity `heapless` containers
- bounded channels
- stack usage inspected from linker map
- release builds use LTO and size optimization

## Error policy

Drivers return typed errors. Top-level product policy decides whether to retry, degrade, or reset. During bring-up, log recoverable failures using `defmt` and continue where safe.
