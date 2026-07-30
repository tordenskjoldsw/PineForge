> [!CAUTION]
> **PineForge is experimental proof-of-concept firmware, not a production
> release.** Flashing may leave a sealed PineTime unusable or require recovery.
> Keep an official InfiniTime DFU package available, read
> [`GETTING-STARTED.md`](GETTING-STARTED.md), and use released binaries at your
> own risk.

# PineForge — Experimental Rust Firmware for PineTime

A modern Rust and Embassy foundation for a **sealed PineTime** with the existing InfiniTime MCUBoot bootloader.

PineForge is an independent ground-up reimplementation informed by earlier PineTime Rust projects.

PineForge is an independent community project. It is not affiliated with, supported by, or maintained by PINE64.

## Project status

PineForge is an early hardware-tested proof of concept. The `v0.1.0` milestone
demonstrates a Rust/Embassy firmware that can boot through the InfiniTime
MCUBoot bootloader, use the PineTime hardware, and perform a
Gadgetbridge-compatible OTA update. A complete PineForge-to-InfiniTime OTA
transfer reached 100%, validated, rebooted, and returned to InfiniTime on real
hardware.

`v0.2.0` turns that bring-up into a platform: product policy moved into a
host-tested state crate, the side button became the back button once a software
restart existed to replace it as the recovery path, and BLE pairings survive
reboots and updates. `v0.2.1` follows with a gesture no longer activating the
control under the finger, and static RAM inside its design target. The launcher
and quick-settings screens are still outstanding — see
[`docs/releases/v0.2.1.md`](docs/releases/v0.2.1.md).

That successful test does **not** establish general safety across PineTime
hardware revisions, bootloader versions, phones, or future images. Expect
missing features, bugs, slow OTA transfers, resets, and possible recovery work.

## Safety and recovery model

- New PineForge images initially run as unconfirmed MCUBoot test images.
- The user can confirm a tested image on the watch; OTA is disabled until the
  running image is confirmed, preserving the rollback image.
- Before confirmation, a reset allows MCUBoot to roll back to the previously
  installed image.
- The physical side button provides an explicit reset/rollback path while the
  image remains unconfirmed.
- Holding the side button during boot until the boot logo turns red starts the
  minimal InfiniTime recovery image. Its Bluetooth DFU service can install a
  known-good firmware ZIP even when the normal application cannot boot.
- Keep a known-good official InfiniTime DFU ZIP on the paired phone before
  testing PineForge.

## Features

- Embassy on the nRF52832
- ST7789 display
- CST816S touch controller over I²C
- BMA421 motion and HRS3300 heart-rate hardware integration
- persistent display settings and BLE bonding
- Gadgetbridge-compatible pairing and time synchronization
- Nordic Legacy DFU service with on-watch progress and failure reporting
- MCUBoot image confirmation, OTA activation, reset, and rollback paths
- physical side-button reset
- watchdog feeding for the WDT started by the bootloader
- MCUBoot linker layout starting at `0x8020`
- reproducible script for generating a Gadgetbridge-compatible DFU ZIP

## Touch pin assignment

- SDA: P0.06
- SCL: P0.07
- Reset: P0.10
- Interrupt: P0.28

## Build

`setup-build-tools.sh` installs the Rust components, a pinned MCUBoot checkout
for `imgtool`, and a `.venv` with pinned Python tooling. It is safe to re-run.

```bash
./scripts/setup-build-tools.sh
./scripts/build-dfu.sh
```

`build-dfu.sh` takes the version from `Cargo.toml` when called without an
argument. Pass one to add build metadata that distinguishes two packages of the
same release, for example `./scripts/build-dfu.sh 0.2.1+7`.

The DFU script builds the production profile with normal UI animations by
default. Diagnostic screens and render metrics are opt-in:

```bash
PINEFORGE_FEATURES=diagnostics ./scripts/build-dfu.sh
```

Release artifacts and routine OTA tests must use the default production build
unless the artifact is explicitly labeled as diagnostic.

Output:

```text
dist/pineforge-mcuboot-app-dfu-0.2.1.zip
```

This ZIP can be installed through the Gadgetbridge firmware installer.

Before flashing, follow
[`GETTING-STARTED.md`](GETTING-STARTED.md) and
[`docs/SEALED-PINETIME-TESTING.md`](docs/SEALED-PINETIME-TESTING.md).

## References and acknowledgements

PineForge was written as a new codebase. It does not copy either earlier project's architecture wholesale, but their practical PineTime work informed hardware bring-up and the organization of this firmware:

- [`dbrgn/pinetime-rtic`](https://github.com/dbrgn/pinetime-rtic) — PineTime pin assignments and proven display initialization details.
- [`thecodechemist99/pinetime-rust`](https://github.com/thecodechemist99/pinetime-rust) — prior art for modular Rust peripheral drivers and asynchronous task organization.
- [`InfiniTimeOrg/pinetime-mcuboot-bootloader`](https://github.com/InfiniTimeOrg/pinetime-mcuboot-bootloader) — the bootloader memory layout, DFU format, trial boot, and rollback behavior targeted by PineForge.
- [`embassy-rs/embassy`](https://github.com/embassy-rs/embassy) — the modern asynchronous embedded Rust runtime used by PineForge.

Thanks to these projects and the wider PineTime community for documenting and testing the hardware.
