# Safe Testing on a Sealed PineTime

This variant is intentionally an **unconfirmed MCUBoot test image**.

## Procedure

1. Build a Gadgetbridge DFU ZIP with `scripts/build-dfu.sh`.
2. Open the ZIP on Android with the Gadgetbridge firmware installer.
3. PineForge starts after the firmware swap, on its watchface. The status corner
   carries the unconfirmed mark, which is how you can tell the image is still on
   trial.
4. Try what you came to try. The **about** screen - swipe up, then `ABOUT` -
   names the release, commit and date of the image actually running, which is
   worth checking before trusting anything else you see.
5. To leave it, hold the side button for two seconds. That resets the watch; a
   short press is the back gesture and will not.
6. Because the image was not confirmed, MCUBoot restores the previous InfiniTime
   image on that reset.

Everything above happens without confirming, so nothing here can cost you the
rollback. That is the point of testing this way.

## Confirming the image

Once a build is trusted, it can be made permanent from the **firmware** screen:
swipe up from the watchface to reach the launcher, then tap **FIRMWARE** and
confirm. Swiping *down* from the watchface opens notifications, not this - the
route changed in v0.3.0 when the launcher replaced the provisional gestures.

Confirming writes `image_ok` to the primary-slot trailer via NVMC, after which:

- a side-button reset no longer rolls back - PineForge is now the primary image;
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
