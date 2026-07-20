# Getting Started — Sealed PineTime

## Status

This repository contains experimental source code. Only flash a DFU ZIP that has been built successfully and inspected locally.

## Safety principle

- The Rust image is never confirmed.
- The running firmware feeds the watchdog started by the bootloader.
- Pressing the side button triggers a system reset after a short debounce delay.
- A 60-second safety timeout also triggers a system reset.
- On the next boot, MCUBoot should roll back to the previous InfiniTime firmware.

## 1. Preparation

1. Fully charge the PineTime.
2. Save the latest official InfiniTime DFU ZIP on the Android device.
3. Verify that Gadgetbridge is connected to InfiniTime.
4. Record the current InfiniTime and bootloader versions.
5. Do not confirm the Rust image.

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
dist/pineforge-gadgetbridge-dfu.zip
```

## 6. Inspect before flashing

```bash
unzip -l dist/pineforge-gadgetbridge-dfu.zip
sha256sum dist/pineforge-gadgetbridge-dfu.zip
```

The ZIP must contain at least the manifest and application payload of the Nordic Legacy DFU package.

## 7. Install through Gadgetbridge

1. Open Gadgetbridge and verify the connection to the PineTime.
2. Open the Android file manager.
3. Tap `pineforge-gadgetbridge-dfu.zip`.
4. Open it with the Gadgetbridge firmware installer.
5. Read the warning and start the installation.
6. Keep the watch and phone close together and do not disable Bluetooth during the transfer.
7. Inspect the Rust test interface after the reboot.

## 8. Return to InfiniTime

Normal path:

1. Press the physical side button.
2. The firmware performs a system reset.
3. Because the image was not confirmed, MCUBoot should roll back to the previous InfiniTime version.

Fallback:

1. If no button input is received, wait for the 60-second safety timeout.
2. If the executor stalls, the inherited hardware watchdog should reset the device after approximately seven seconds.
3. After a successful rollback, InfiniTime should start again.

## 9. If InfiniTime does not start

- Do not repeatedly install additional experimental ZIP files.
- Try the bootloader rollback again.
- If neither the watchdog nor another reset path works, fully discharging a sealed PineTime may be the only way to force a power cycle.
- Afterwards, install the previously saved official InfiniTime DFU through Gadgetbridge if the recovery firmware is available.

## Not yet validated as safe

- CST816S driver integration with Embassy TWIM
- interrupt bindings of the Embassy version in use
- watchdog takeover with every bootloader version
- complete OTA and rollback behavior on a sealed PineTime
