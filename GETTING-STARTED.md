# Getting Started — Sealed PineTime

## Status

PineForge works as an everyday watch firmware — it has run on my own watch since
late July 2026 — but coverage is limited to that one tested setup, and a sealed
PineTime is awkward to recover. Only flash a DFU ZIP that has been built
successfully and inspected, and keep an official InfiniTime recovery package
available.

## Safety principle

- A newly installed PineForge image starts as an unconfirmed MCUBoot test image.
- While it is unconfirmed, resetting rolls back to the previous InfiniTime
  image and PineForge refuses OTA updates to protect that rollback image.
- After PineForge has been tested, it must be explicitly confirmed through
  **Settings > FW > CONFIRM** before its OTA service will accept an update.
- Confirmation makes PineForge the permanent primary image: resetting no
  longer rolls back to InfiniTime.
- The running firmware feeds the watchdog started by the bootloader.
- Pressing the side button triggers a system reset after a short debounce delay.

## 1. Preparation

1. Fully charge the PineTime.
2. Save the latest official InfiniTime DFU ZIP on the Android device.
3. Verify that Gadgetbridge is connected to InfiniTime.
4. Record the current InfiniTime and bootloader versions.
5. Do not confirm PineForge until its display, touch input, side-button reset,
   and general stability have been tested. Confirmation is intentionally
   required later if you want to use PineForge's OTA service.

## 2. Build environment

Install `git`, `python3` and [`rustup`](https://rustup.rs) with your
distribution's package manager. On Arch Linux:

```bash
sudo pacman -S --needed base-devel git python python-pip rustup
rustup default stable
```

Then run the setup script, which installs everything else at pinned versions:

```bash
./scripts/setup-build-tools.sh
```

It installs:

- the toolchain, target and components pinned by `rust-toolchain.toml`;
- `flip-link`, the linker for the firmware target. It places the stack below the
  statics so that an overflow faults instead of quietly overwriting them;
  without it every firmware link fails. See `docs/ARCHITECTURE.md`;
- `cargo-binutils`, used by the size budget and to produce the raw binary;
- MCUBoot at a pinned commit in `tools/mcuboot-src`, for `imgtool`;
- a `.venv` with the pinned Python tooling in `tools/requirements.txt`,
  including `adafruit-nrfutil`.

The script is safe to re-run and does not modify anything outside the repository
except the Cargo binaries in `~/.cargo/bin`.

## 3. Check the project

```bash
cargo fmt --all -- --check
cargo clippy --release --target thumbv7em-none-eabihf -- -D warnings
cargo build --release --target thumbv7em-none-eabihf
```

Do not flash if any command fails.

## 4. Create the DFU ZIP

The default and recommended artifact is the production image. Called without an
argument, the script names the package after the version in `Cargo.toml`:

```bash
./scripts/build-dfu.sh
```

Only enable diagnostic screens for a deliberately labeled hardware test:

```bash
PINEFORGE_FEATURES=diagnostics ./scripts/build-dfu.sh
```

Expected file, for the `0.2.1` in `Cargo.toml`:

```text
dist/pineforge-mcuboot-app-dfu-0.2.1.zip
```

## 5. Inspect before flashing

```bash
unzip -l dist/pineforge-mcuboot-app-dfu-0.2.1.zip
sha256sum dist/pineforge-mcuboot-app-dfu-0.2.1.zip
```

The ZIP must contain at least the manifest and application payload of the Nordic Legacy DFU package.

## 6. Install through Gadgetbridge

1. Open Gadgetbridge and verify the connection to the PineTime.
2. Open the Android file manager.
3. Tap `pineforge-mcuboot-app-dfu-0.2.1.zip`.
4. Open it with the Gadgetbridge firmware installer.
5. Read the warning and start the installation.
6. Keep the watch and phone close together and do not disable Bluetooth during the transfer.
7. Inspect the Rust test interface after the reboot.

## 7. Confirm PineForge and use OTA

Only continue after testing the unconfirmed image and deciding to give up its
automatic rollback:

1. Open **Settings > FW > CONFIRM** in PineForge.
2. Confirm the warning on the watch.
3. PineForge writes MCUBoot's `image_ok` marker and enables its DFU service.
4. Open a PineForge or official InfiniTime DFU ZIP with Gadgetbridge.
5. Keep the watch and phone close until the transfer reaches 100%, validates,
   and reboots.

Confirmation cannot be undone by an ordinary reset. After confirmation, the
side button only reboots PineForge.

## 8. Return to InfiniTime

Before confirming PineForge:

1. Press the physical side button.
2. MCUBoot should roll back to the previous InfiniTime image after the reset.

After confirming PineForge:

1. Install the saved official InfiniTime DFU ZIP through PineForge's OTA
   service as described above.
2. Wait for 100%, validation, and reboot.
3. InfiniTime should start as the newly staged image.

## 9. If InfiniTime does not start

- Do not repeatedly install further untested ZIP files.
- If PineForge is still unconfirmed, try the side-button rollback again.
- If PineForge was confirmed and still boots, retry the known-good official
  InfiniTime package through its OTA service.
- To enter the bootloader's minimal InfiniTime recovery image, hold the
  physical side button during boot until the boot logo turns red. Connect to
  the recovery image over Bluetooth and install a known-good DFU ZIP.
- If neither the watchdog nor another reset path works, fully discharging a sealed PineTime may be the only way to force a power cycle.
- Afterwards, install the previously saved official InfiniTime DFU through
  Gadgetbridge if a working DFU firmware is available.

## Not broadly validated

- compatibility across PineTime hardware and external-flash revisions
- watchdog takeover with every bootloader version
- OTA behavior across Android devices and Gadgetbridge versions
- recovery behavior for interrupted or corrupt transfers
- stability and power consumption beyond a few days
- step-count and heart-rate accuracy, neither compared against a reference

The complete confirmed-PineForge-to-InfiniTime OTA path has been validated once
on a sealed PineTime: the transfer reached 100%, validated, rebooted, and
returned to InfiniTime. One success on one watch is not a general safety
guarantee. See [`docs/TESTED-CONFIGURATIONS.md`](docs/TESTED-CONFIGURATIONS.md).
