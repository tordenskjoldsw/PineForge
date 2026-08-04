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
| Three-page About/system status | Yes, flashed and displayed on the known sealed PineTime: build, hardware and system pages are reachable. Rare failure values and alternative hardware IDs have not been deliberately exercised. |
| FORGE stopwatch | Yes, flashed as the `0.6.0+1` development image and exercised on the known sealed PineTime: display, start, pause, resume, reset, navigation away and display sleep all work. |
| FORGE timer | The countdown and alarm work on the known sealed PineTime. The final larger phase labels, unified typography and approximately five-second cancellable repeating vibration were checked in the v0.6.0 development sequence. |
| Manual TIME and DATE applications | `0.6.0+9` was flashed on the known sealed PineTime: both editors, APPLY, optimized cell-level partial redraw, restoration across reboots and subsequent override by the phone time all work. Field wrapping, leap dates, invalid-date clamping, CRC records and sequence wrap additionally pass host tests. |
| Bluetooth application | `0.6.1+1` was exercised on the known sealed PineTime: disabling BLE disconnects the phone and removes the status rune, OFF persists across reboot, enabling restarts advertising and Gadgetbridge reconnects without re-pairing, and ON also persists across reboot. Settings migration, DFU lockout and rendering additionally pass host tests. |
| Sliding vertical transitions | `0.6.1+2` was exercised on the known sealed PineTime: the launcher and the notification screen slide in and out through the panel's scroll window, and vertical page turns slide with them. The window arithmetic, the wrap at the end of frame memory and the translation of later partial redraws all hold in ordinary use. Tearing at the band boundary and the smoothness of the 12-row step were looked at, not measured. |
| Vertical launcher paging | `0.6.1+2` on the known sealed PineTime: up pages on, down pages back, and down from the first page returns to the watchface. The rule that a paged screen may claim only the gestures it has a page for additionally passes host tests. |
| Battery application | `0.6.2+1` was exercised on the known sealed PineTime: the charge, the direction and the cell voltage all display and follow the charger. The three charger states, the voltage formatting and the partial redraw additionally pass host tests. The percentage itself is the capacity estimator's, which has never been compared against a reference discharge. |
| Step count over BLE | `0.15.0+7` on the known sealed PineTime, against the known Gadgetbridge: the motion service is discovered, the count reads back correctly over a generic BLE client and steps accumulate in the app. The midnight reset that the companion's daily accounting depends on has **not** been observed across an actual midnight. |
| Heart rate over BLE | `0.15.0+7` on the known sealed PineTime: a validated reading reaches Gadgetbridge. Only the on-demand path has been seen; the background interval has not been watched through a reading. |
| Pairing after the device is forgotten | `0.15.0+8`, the shipping build, after removing the watch from Android entirely: a full key exchange completes. This is the path that a stack shortfall breaks, and it broke twice in development before the reserve was measured. |
| Stack high-water mark under pairing | Measured at 16,900 bytes on a `0.15.0+4` diagnostics build, with the phone made to forget the device first so the connection was a genuine pairing rather than a resumed bond. The RAM budget is derived from this. |
| Pulse partial redraw | `0.15.0+8` on the known sealed PineTime: a new reading repaints the number rather than the panel. |
| Weather over BLE | `1.14.0` on the known sealed PineTime, driven from Gadgetbridge's debug menu: the simple weather service is discovered, the write is accepted, the packet parses and the screen shows the location, the temperature, the condition and the day's range. That the manual send works also rules out a stale attribute table, since a cached one would have made `safeWriteToCharacteristic` skip in silence. Only the current conditions - the forecast packet has never been sent to it. That build named the condition in words; `1.14.0+1` replaced the word with a symbol and added the forecast row, and neither has been on the watch. |
| Weather arriving unprompted | **Not established.** Gadgetbridge pushes weather only when a broadcast arrives while its service is running, never on connect, so a record that arrived before the watch did stays in its singleton. Nothing has yet been observed reaching the watch without the debug menu. |
| Stack high-water mark, second reading | 17,292 bytes on a `0.15.0+9` diagnostics build, taken the same way as the first and after the heart-rate and weather services were added. The 392-byte rise over the earlier 16,900 is what a BLE service costs the stack, beside what it costs the statics. |
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
| Heart-rate accuracy | Readings are produced on demand and on an interval, and now also reach a companion, but have never been compared against a reference monitor. Neither the accuracy nor the effect of the sampling interval on battery life is known. |
| Weather below zero, and a long location | Both are drawn correctly in an off-watch render of the screen - a minus built from the segment strokes, a long name cut between characters - and neither has been seen with real data. Every reading a phone has sent so far was positive and every place name short. |
| The weather forecast, on a watch | The five-day packet is parsed, host-tested and now drawn, and the row was checked by rendering the screen off the watch and looking at it. No forecast has been received from a phone, so nothing about it is established on hardware. |
| The nine weather symbols, on the panel | Checked as rendered images, which settles the shapes and the layout and says nothing about how 24x24 monochrome glyphs read on the physical display. Rain is the weakest of the nine and is the one to look at first. |
| The step counter crossing midnight | The reset is implemented and the day boundary is host-tested, but no watch has been observed through an actual midnight, and Gadgetbridge's daily accounting depends on receiving the zero. |
| DFU stack depth | The pairing peak is measured; a firmware transfer streaming into external flash is the other deep path and is not, because the reboot a completed update performs clears the mark. Part of the reserve is held for it. |
| Music robustness | Core display and control paths work against the known Gadgetbridge setup. Progress after display sleep, Bluetooth disconnection and reconnection, and switching media applications mid-track have not yet been exercised. |
| Stopwatch long-run behaviour | Core controls, navigation and display sleep work on hardware, but an extended run has not been compared against a reference clock. |

## Reporting a configuration

If you run PineForge on a setup that is not listed here, please open an issue
with the watch revision, bootloader version, phone, Android version and
Gadgetbridge version - whether it worked or not. A failure on a named
configuration is more useful to this table than a success on an unnamed one.
