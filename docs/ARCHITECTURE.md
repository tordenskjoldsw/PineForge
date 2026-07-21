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
- accelerometer task: probes the motion sensor without configuring features
- display task: owns the LCD and backlight, consumes UI events, and renders the active `Screen`
- watchdog task: feeds the watchdog inherited from the bootloader
- rollback tasks: monitor the physical side button and the test-image safety timeout

Planned subsystem tasks include:

- sensor task: accelerometer and heart-rate sampling
- power task: battery, charging, sleep policy
- BLE task: owns the radio/stack

Use bounded `embassy-sync` channels for events. The current UI channel has a fixed capacity of eight events. Never share the display or SPI peripheral behind a global mutex merely for convenience; prefer single-owner tasks and message passing.

The touch controller and accelerometer are an intentional exception at the bus
transport boundary because PineTime wires both devices to the same TWIM
peripheral. They receive independent `embedded-hal-async` I²C device handles
backed by one asynchronous mutex; critical sections protect only the mutex
state, not the I²C transaction. Each driver still owns its local state, and no
driver can access another device.

## Power policy

Display power and future system power are separate state machines. The pure
`DisplayPowerPolicy` lives in `pineforge-state`; the display task applies its
decisions because that task exclusively owns the LCD and backlight. A validated
`DisplayPowerConfig` currently supplies defaults of ten seconds until dimming
and twenty seconds until sleep. A future settings service can replace those
defaults without changing the policy or hardware boundary. While asleep,
periodic UI ticks are suspended. Input wakes and fully redraws the active screen
before the backlight is enabled, preventing stale framebuffer content from
becoming visible.

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
