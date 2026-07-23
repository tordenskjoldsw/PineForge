# External flash map

The PineTime carries a 4 MiB XT25F32B SPI-NOR flash (4 KiB erase sectors,
256 B program pages) shared on SPIM0 with the ST7789 LCD.

PineForge is currently installed as an unconfirmed image with stock
InfiniTime as the rollback target, so the regions InfiniTime and the stock
MCUBoot bootloader use are treated as read-only **for now**. Once PineForge
is confirmed as the primary firmware and no longer relies on the InfiniTime
rollback path, the littlefs region below becomes reclaimable for PineForge's
own storage (assets, logs, a future filesystem).

## Layout

| Range                 | Size      | Owner                                             | PineForge policy |
| --------------------- | --------- | ------------------------------------------------- | ---------------- |
| `0x000000`–`0x03FFFF` | 256 KiB   | MCUBoot bootloader graphics assets                | never write      |
| `0x040000`–`0x0B3FFF` | 464 KiB   | MCUBoot secondary slot / DFU staging              | never write      |
| `0x0B4000`–`0x3FDFFF` | ~3.3 MiB  | InfiniTime littlefs (temporary, rollback era)     | reserved for future PineForge storage; no writes while rollback exists |
| `0x3FE000`–`0x3FEFFF` | 4 KiB     | **PineForge settings slot A**                     | read/write       |
| `0x3FF000`–`0x3FFFFF` | 4 KiB     | **PineForge settings slot B**                     | read/write       |

The named constants mirroring this table live in `src/services/settings.rs`
(`SETTINGS_SLOT_A_ADDRESS`, `SETTINGS_SLOT_B_ADDRESS`). The settings slots
sit at the top of the chip so they can stay put when the littlefs region is
later reclaimed.

## Settings record

Each slot holds one 32-byte record (defined in
`crates/pineforge-state/src/settings.rs`, version-dispatched and
CRC32-checked). Writes alternate between the slots: erase the inactive
sector, program the record, read it back, and only then treat it as current.
A power loss at any point leaves the previous record intact; boot picks the
valid record with the newer wrapping sequence number and falls back to
built-in defaults when neither slot decodes.

## Rollback-era trade-off

While InfiniTime remains installed for rollback, the two settings sectors
technically lie at the end of the span stock InfiniTime formats as littlefs:

- InfiniTime only allocates these blocks when its filesystem is nearly full,
  and littlefs tolerates and reclaims foreign block content when it does.
- If a rollback-then-reinstall cycle lets littlefs overwrite a slot, the
  record fails its CRC and PineForge falls back to the other slot or to
  built-in defaults.

Both directions are recoverable; bootloader assets and the DFU staging area
are never touched. This entire constraint disappears with the rollback era.
