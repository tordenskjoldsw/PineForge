# Safe Testing on a Sealed PineTime

This variant is intentionally an **unconfirmed MCUBoot test image**.

## Procedure

1. Build a Gadgetbridge DFU ZIP with `scripts/build-dfu.sh 0.1.0`.
2. Open the ZIP on Android with the Gadgetbridge firmware installer.
3. The Rust test firmware starts after the firmware swap.
4. Test touch input. The most recently read coordinates are displayed.
5. Press the physical side button, or wait for the 10-minute safety timeout.
6. The firmware performs only a system reset.
7. Because the image was not confirmed, MCUBoot restores the previous InfiniTime image.

## Important

- This codebase deliberately provides no function that confirms the image.
- An ordinary reset before confirmation also causes a rollback.
- The bootloader is neither modified nor overwritten.
- The build uses a 32-byte MCUBoot header and the 475,136-byte PineTime slot.
- Test in an emulator first and use only small, traceable changes on the watch.
