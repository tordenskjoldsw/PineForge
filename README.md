> [!CAUTION]
> Experimental firmware under active hardware testing. Do not flash it until `cargo build --release` succeeds locally and the DFU package has been inspected. See `GETTING-STARTED.md`.

# PineForge — Touch and Rollback Test Firmware

A modern Rust and Embassy foundation for a **sealed PineTime** with the existing InfiniTime MCUBoot bootloader.

PineForge is an independent ground-up reimplementation informed by earlier PineTime Rust projects.

PineForge is an independent community project. It is not affiliated with, supported by, or maintained by PINE64.

## Safety model

This firmware is installed as an unconfirmed test image and never confirms itself. Pressing the physical side button triggers a reset, after which MCUBoot rolls back to the previously installed InfiniTime version. There is no automatic timeout, allowing long-running sensor and power tests.

## Features

- Embassy on the nRF52832
- ST7789 display
- CST816S touch controller over I²C
- touch-coordinate display
- physical side-button reset and rollback
- watchdog feeding for the WDT started by the bootloader
- MCUBoot linker layout starting at `0x8020`
- script for generating a Gadgetbridge-compatible DFU ZIP

## Touch pin assignment

- SDA: P0.06
- SCL: P0.07
- Reset: P0.10
- Interrupt: P0.28

## Build

```bash
rustup target add thumbv7em-none-eabihf
cargo install cargo-binutils
rustup component add llvm-tools
pip install adafruit-nrfutil
mkdir -p tools/mcuboot
# Copy the official MCUBoot imgtool.py to tools/mcuboot/imgtool.py
./scripts/build-dfu.sh 0.1.0
```

The DFU script enables the `diagnostics` feature by default for sealed-device
hardware testing. Use `PINEFORGE_FEATURES= ./scripts/build-dfu.sh 0.1.0` to
build without the touch-test screen and render telemetry.

Output:

```text
dist/pineforge-mcuboot-app-dfu-0.1.0.zip
```

This ZIP can be installed through the Gadgetbridge firmware installer.

See also [`docs/SEALED-PINETIME-TESTING.md`](docs/SEALED-PINETIME-TESTING.md).

## References and acknowledgements

PineForge was written as a new codebase. It does not copy either earlier project's architecture wholesale, but their practical PineTime work informed hardware bring-up and the organization of this firmware:

- [`dbrgn/pinetime-rtic`](https://github.com/dbrgn/pinetime-rtic) — PineTime pin assignments and proven display initialization details.
- [`thecodechemist99/pinetime-rust`](https://github.com/thecodechemist99/pinetime-rust) — prior art for modular Rust peripheral drivers and asynchronous task organization.
- [`InfiniTimeOrg/pinetime-mcuboot-bootloader`](https://github.com/InfiniTimeOrg/pinetime-mcuboot-bootloader) — the bootloader memory layout, DFU format, trial boot, and rollback behavior targeted by PineForge.
- [`embassy-rs/embassy`](https://github.com/embassy-rs/embassy) — the modern asynchronous embedded Rust runtime used by PineForge.

Thanks to these projects and the wider PineTime community for documenting and testing the hardware.
