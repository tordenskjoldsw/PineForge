# External flash map

The PineTime carries a 4 MiB XT25F32B SPI-NOR flash (4 KiB erase sectors,
256 B program pages) shared on SPIM0 with the ST7789 LCD.

While PineForge is an unconfirmed image with stock InfiniTime as the rollback
target, InfiniTime's littlefs region remains read-only. On the first boot after
PineForge is confirmed as the primary firmware, PineForge initializes that
region as its own storage. Returning to InfiniTime remains possible, but
InfiniTime must then recreate its littlefs.

## Layout

| Range                 | Size      | Owner                                             | PineForge policy |
| --------------------- | --------- | ------------------------------------------------- | ---------------- |
| `0x000000`–`0x03FFFF` | 256 KiB   | MCUBoot bootloader graphics assets                | never write      |
| `0x040000`–`0x0B3FFF` | 464 KiB   | MCUBoot secondary slot / DFU staging              | DFU service only  |
| `0x0B4000`–`0x3FCFFF` | 3,364 KiB | **PineForge data/assets** after confirmation      | read/write; reserved while rollback exists |
| `0x3FD000`–`0x3FDFFF` | 4 KiB     | **PineForge BLE bond**                            | read/write       |
| `0x3FE000`–`0x3FEFFF` | 4 KiB     | **PineForge settings slot A**                     | read/write       |
| `0x3FF000`–`0x3FFFFF` | 4 KiB     | **PineForge settings slot B**                     | read/write       |

The named constants mirroring this table live in `src/services/settings.rs`
(`SETTINGS_SLOT_A_ADDRESS`, `SETTINGS_SLOT_B_ADDRESS`, `BOND_ADDRESS`). These
records sit at the top of the chip so they can stay put when the littlefs
region is later reclaimed.

## PineForge storage initialization

The range contains exactly 841 erase sectors. Sector 0 is a metadata sector;
the other 840 sectors provide 3,360 KiB for data and assets. The metadata
contains:

- a format-in-progress header with format version, geometry, and CRC32;
- a separate ready header with the same validation fields;
- one monotonic completion byte for each data sector.

Initialization erases one data sector at a time, pets the inherited bootloader
watchdog, writes and verifies that sector's completion byte, then yields while
the flash driver polls. Other Embassy tasks continue to run and the display
receives percentage updates. If power is lost, boot skips every sector whose
completion byte was verified and resumes the remainder. A sector interrupted
between erase and marker write is safely erased again.

The ready header is written and read back only after all sector markers are
complete. A valid ready header makes later boots constant-time. PineForge
refuses to erase a header carrying an unknown newer format version; a future
release must provide an explicit migration or reformat policy.

The BLE bond sector holds one CRC32-checked record with the serialized bond
keys, so a paired phone reconnects across reboots without re-pairing. A
corrupt or missing record simply falls back to re-pairing.

## Settings record

Each slot holds one 32-byte record (defined in
`crates/pineforge-state/src/settings.rs`, version-dispatched and
CRC32-checked). Format version 2 adds the background heart-rate enable flag
and measurement interval; version-1 records migrate with heart-rate disabled
and the five-minute default interval. Writes alternate between the slots: erase the inactive
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

## Internal flash

The nRF52832 has 512 KiB of internal flash. MCUBoot and the primary image
live here; PineForge only reads it, apart from the one confirmation word.

| Range                 | Owner                                        |
| --------------------- | -------------------------------------------- |
| `0x00000`–`0x07FFF`   | MCUBoot bootloader (never write)             |
| `0x08000`–`0x7BFFF`   | Primary image slot (32-byte imgtool header)  |
| `0x7BFE8`             | `image_ok` confirmation word (see below)     |
| `0x7C000`–`0x7FFFF`   | MCUBoot scratch / bootloader data            |

Confirming a running image writes `1` to the `image_ok` word at `0x0007_BFE8`
via NVMC (`src/boot/confirm.rs`), matching InfiniTime's `FirmwareValidator`.
The word sits in the erased trailer, so the single-word write needs no page
erase. Once set, the bootloader keeps the image instead of rolling back.
