# Roadmap

## Milestone 1 — board bring-up

- display orientation/color validation
- button polarity validation
- backlight PWM/levels
- battery SAADC calibration
- charge status GPIO

## Milestone 2 — shared buses

- I²C manager for touch, accelerometer, and heart-rate sensor
- SPI arbitration for LCD and external flash
- per-device power gating where available

## Milestone 3 — product runtime

- event bus with bounded channels
- monotonic time and RTC synchronization
- low-power display wake/sleep policy
- settings persisted to external flash

## Milestone 4 — connectivity

- documented BLE stack choice
- GATT current-time service
- battery service
- notification transport
- memory map compatible with bootloader/DFU

## Milestone 5 — applications

- watchfaces
- notifications
- alarms/timers
- step counting
- heart-rate sampling
