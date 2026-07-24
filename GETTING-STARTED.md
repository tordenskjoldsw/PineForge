# Getting Started — Sealed PineTime

## Status

This repository contains experimental proof-of-concept firmware, not a
production release. A complete PineForge-to-InfiniTime OTA replacement path
has succeeded on real hardware, but coverage is limited to the tested setup.
Only flash a DFU ZIP that has been built successfully and inspected, and keep
an official InfiniTime recovery package available.

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

## 2. Build environment on Arch Linux

```bash
sudo pacman -S --needed base-devel git python python-pip rustup
rustup default stable
rustup target add thumbv7em-none-eabihf
rustup component add llvm-tools
cargo install cargo-binutils --locked
python -m venv .venv
source .venv/bin/activate
pip install adafruit-nrfutil
```

## 3. Set up the MCUBoot imgtool

The build requires `scripts/imgtool.py` from the official MCUBoot repository:

```bash
git clone --depth 1 https://github.com/mcu-tools/mcuboot.git tools/mcuboot-src
mkdir -p tools/mcuboot
cp tools/mcuboot-src/scripts/imgtool.py tools/mcuboot/imgtool.py
pip install -r tools/mcuboot-src/scripts/requirements.txt
```

## 4. Check the project

```bash
cargo fmt --all -- --check
cargo clippy --release --target thumbv7em-none-eabihf -- -D warnings
cargo build --release --target thumbv7em-none-eabihf
```

Do not flash if any command fails.

## 5. Create the DFU ZIP

```bash
./scripts/build-dfu.sh 0.1.0
```

Expected file:

```text
dist/pineforge-mcuboot-app-dfu-0.1.0.zip
```

## 6. Inspect before flashing

```bash
unzip -l dist/pineforge-mcuboot-app-dfu-0.1.0.zip
sha256sum dist/pineforge-mcuboot-app-dfu-0.1.0.zip
```

The ZIP must contain at least the manifest and application payload of the Nordic Legacy DFU package.

## 7. Install through Gadgetbridge

1. Open Gadgetbridge and verify the connection to the PineTime.
2. Open the Android file manager.
3. Tap `pineforge-mcuboot-app-dfu-0.1.0.zip`.
4. Open it with the Gadgetbridge firmware installer.
5. Read the warning and start the installation.
6. Keep the watch and phone close together and do not disable Bluetooth during the transfer.
7. Inspect the Rust test interface after the reboot.

## 8. Confirm PineForge and use OTA

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

## 9. Return to InfiniTime

Before confirming PineForge:

1. Press the physical side button.
2. MCUBoot should roll back to the previous InfiniTime image after the reset.

After confirming PineForge:

1. Install the saved official InfiniTime DFU ZIP through PineForge's OTA
   service as described above.
2. Wait for 100%, validation, and reboot.
3. InfiniTime should start as the newly staged image.

## 10. If InfiniTime does not start

- Do not repeatedly install additional experimental ZIP files.
- If PineForge is still unconfirmed, try the side-button rollback again.
- If PineForge was confirmed and still boots, retry the known-good official
  InfiniTime package through its OTA service.
- To enter the bootloader's minimal InfiniTime recovery image, hold the
  physical side button during boot until the boot logo turns red. Connect to
  the recovery image over Bluetooth and install a known-good DFU ZIP.
- If neither the watchdog nor another reset path works, fully discharging a sealed PineTime may be the only way to force a power cycle.
- Afterwards, install the previously saved official InfiniTime DFU through
  Gadgetbridge if a working DFU firmware is available.

## Still experimental or not broadly validated

- compatibility across PineTime hardware and external-flash revisions
- watchdog takeover with every bootloader version
- OTA behavior across Android devices and Gadgetbridge versions
- recovery behavior for interrupted or corrupt transfers
- long-term stability and power consumption

The complete confirmed-PineForge-to-InfiniTime OTA path has been validated once on a
sealed PineTime: the transfer reached 100%, validated, rebooted, and returned
to InfiniTime. Treat that as proof of concept, not a general safety guarantee.
