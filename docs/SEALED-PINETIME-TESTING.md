# Safe Testing on a Sealed PineTime

This variant is intentionally an **unconfirmed MCUBoot test image**.

## Procedure

1. Build a Gadgetbridge DFU ZIP with `scripts/build-dfu.sh 0.1.0`.
2. Open the ZIP on Android with the Gadgetbridge firmware installer.
3. The Rust test firmware starts after the firmware swap.
4. Test touch input. The most recently read coordinates are displayed.
5. Press the physical side button when you want to leave the test firmware.
6. The firmware performs only a system reset.
7. Because the image was not confirmed, MCUBoot restores the previous InfiniTime image.

## Confirming the image

Once a build is trusted, it can be made permanent from **Settings > FW >
CONFIRM** (swipe down on the watchface). Confirming writes `image_ok` to the
primary-slot trailer via NVMC, after which:

- a side-button reset no longer rolls back — PineForge is now the primary image;
- the DFU service accepts updates, so the next firmware can be installed over
  the air with Gadgetbridge (see `GETTING-STARTED.md`);
- InfiniTime can always be reinstalled later by feeding an InfiniTime DFU ZIP
  to PineForge's own DFU.

Until an image is confirmed it stays a safe test image: the DFU service refuses
`StartDFU`, so the InfiniTime rollback staged in the external-flash secondary
slot is never overwritten.

## Important

- There is no automatic rollback timeout; use the physical side button to reset.
- An ordinary reset before confirmation also causes a rollback.
- For bootloader recovery, hold the side button during boot until the boot logo
  turns red. The minimal InfiniTime recovery image then exposes Bluetooth DFU
  so a known-good firmware ZIP can be installed.
- The bootloader is neither modified nor overwritten.
- The build uses a 32-byte MCUBoot header and the 475,136-byte PineTime slot.
- Test in an emulator first and use only small, traceable changes on the watch.
