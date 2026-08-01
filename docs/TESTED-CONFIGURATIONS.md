# Tested Configurations

What PineForge has actually been run on, and what it has not. This is a record
of evidence, not a compatibility promise: every row below describes a single
setup, and nothing here generalises to PineTime hardware revisions, bootloader
versions, phones or Android releases that are not named.

If a row says **not recorded**, nobody has written the value down - treat it as
unknown rather than as working.

## Hardware and companion software

| Area | Tested |
|---|---|
| Watch | Sealed PineTime. The revision is unknown and largely unknowable without opening the case: nothing on a sealed unit reports it, and the hardware-revision string PineForge advertises over BLE is a constant in the firmware, not a reading. |
| Number of units | One. Everything in this document was established on that single watch, without SWD and therefore without RTT logging or breakpoints. A second unit has been ordered so the first can be opened for SWD; **nothing here has yet been re-established on a second watch**, and this row is what to update when it has. |
| Bootloader | `InfiniTimeOrg/pinetime-mcuboot-bootloader` **1.0.1**. Not the same versioning as upstream `mcu-tools/mcuboot`, from which PineForge uses only `imgtool` as a host build tool. |
| Prior firmware | InfiniTime **1.16.1**, the release PineForge replaced and the one the rollback and OTA-return paths were tested against. |
| Phone | Android, kept current. Device model and exact OS version not recorded. |
| Companion app | Gadgetbridge, running the current release as of July 2026. Exact version string not recorded. |
| External flash | XT25F32 as fitted; alternative parts on later revisions untested. |

## Paths that have been exercised

| Path | Status |
|---|---|
| InfiniTime → PineForge over Gadgetbridge DFU | Repeatedly, and it is the normal install route. |
| Boot through the stock MCUBoot bootloader | Repeatedly. |
| Unconfirmed image rolled back by a reset | Repeatedly. |
| Image confirmation writing `image_ok` | Yes. |
| Confirmed PineForge → InfiniTime OTA | Once, end to end with the InfiniTime 1.16.1 package: 100%, validated, rebooted, returned to InfiniTime. |
| Bootloader recovery image over Bluetooth | Documented and available; not exercised from a genuinely unbootable state. |
| Passkey pairing and bonds surviving a reboot or update | Yes. |
| Time synchronisation from Gadgetbridge | Yes. |
| Music control against a phone | Yes, on the one known Gadgetbridge setup: title and artist display correctly; previous, next, pause, resume, volume up, and volume down all work from the watch. |
| Daily wear | Since late July 2026, on my own watch - waking, timekeeping, notifications, settings and charging in ordinary use. Days, not months. |

## Not established

| Area | Why it is open |
|---|---|
| Long-term stability | A few days of daily wear is not a long-term test. Nothing is known about drift, leaks or wear over weeks. |
| Battery life | Current consumption has never been measured in either the active or the sleeping state. |
| Other PineTime revisions | Only one unit has run this firmware so far. A second unit has been ordered, but no result from it is recorded; even a second same-revision unit would not establish compatibility with a different hardware revision. |
| Other bootloader versions | Everything here rests on bootloader `1.0.1`. The watchdog handover, the flash map and the trial-boot and rollback behaviour have not been checked against `1.0.2` or against anything older. |
| Other phones or Gadgetbridge versions | The DFU path has one known-good combination. |
| Interrupted or corrupt OTA transfers | Recovery behaviour has not been deliberately provoked. |
| Step-count accuracy | Steps are counted and shown, but never checked against a counted walk or another tracker. The figure is indicative; tuning is expected. |
| Heart-rate accuracy | Readings are produced on demand and on an interval, but never compared against a reference monitor. Neither the accuracy nor the effect of the sampling interval on battery life is known. |
| Music robustness | Core display and control paths work against the known Gadgetbridge setup. Progress after display sleep, Bluetooth disconnection and reconnection, and switching media applications mid-track have not yet been exercised. |

## Reporting a configuration

If you run PineForge on a setup that is not listed here, please open an issue
with the watch revision, bootloader version, phone, Android version and
Gadgetbridge version - whether it worked or not. A failure on a named
configuration is more useful to this table than a success on an unnamed one.
