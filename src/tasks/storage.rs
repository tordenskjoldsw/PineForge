//! Owns the external flash: display settings, the BLE bond, and the DFU slot.
//!
//! Boot decodes both settings slots and the bond record, publishing the
//! newest valid settings (or defaults) and the stored bond. UI-submitted
//! settings persist debounced across two slots; bond writes are one-shot.
//! All regions are documented in `docs/FLASH-MAP.md`.
//!
//! Named for the peripheral it owns rather than the first thing stored on it.
//! As `services::settings` it was the one hardware-owning task outside
//! `tasks`, and the name accounted for one of the three regions it writes.

use defmt::{info, warn};
use embassy_futures::select::{Either3, select3};
use embassy_time::{Duration, Instant, Timer, with_deadline};
use pineforge_state::{
    AppEvent, BOND_RECORD_LEN, CLOCK_JOURNAL_A_ADDRESS, CLOCK_JOURNAL_B_ADDRESS, CLOCK_RECORD_LEN,
    ClockSnapshot, DFU_SLOT_SIZE, DfuFailReason, DisplaySettings, FlashStatus, SETTINGS_RECORD_LEN,
    STORAGE_BASE, STORAGE_DATA_SECTOR_COUNT, STORAGE_FORMAT_VERSION, STORAGE_HEADER_LEN,
    STORAGE_PROGRESS_OFFSET, STORAGE_READY_HEADER_OFFSET, STORAGE_SECTOR_SIZE, SettingsSlot,
    StorageHeader, StorageState, bond_schema_tag, clock_sequence_is_newer, decode_storage_header,
    encode_storage_header, frame_bond, parse_bond, select_slot, storage_header_version,
};

use crate::{
    board::buses::FlashSpi,
    boot::watchdog::BootloaderWatchdog,
    drivers::xt25f32::{Error as FlashError, Xt25f32, is_supported_jedec_id},
    ipc::{
        BOND_LOADED, BOND_STORE, DFU_FLASH_COMMANDS, DFU_FLASH_RESULT, DISPLAY_SETTINGS,
        DfuFlashCommand, SETTINGS_COMMANDS, StoredBond, UI_EVENTS, WALL_CLOCK, wall_clock_receiver,
    },
};

async fn initialize_storage(
    flash: &mut Xt25f32<FlashSpi>,
    watchdog: BootloaderWatchdog,
) -> Result<(), FlashError<<FlashSpi as embedded_hal::spi::ErrorType>::Error>> {
    let mut formatting = [0_u8; STORAGE_HEADER_LEN];
    let mut ready = [0_u8; STORAGE_HEADER_LEN];
    flash.read(STORAGE_BASE, &mut formatting)?;
    flash.read(STORAGE_BASE + STORAGE_READY_HEADER_OFFSET, &mut ready)?;

    if decode_storage_header(&ready) == Some(StorageHeader::Ready) {
        info!("PineForge storage format is ready");
        return Ok(());
    }
    if [formatting, ready].iter().any(|header| {
        storage_header_version(header).is_some_and(|version| version != STORAGE_FORMAT_VERSION)
    }) {
        warn!("Unsupported PineForge storage version; refusing destructive downgrade");
        return Err(FlashError::VerifyFailed);
    }

    if decode_storage_header(&formatting) != Some(StorageHeader::Formatting) {
        flash.erase_sector(STORAGE_BASE).await?;
        watchdog.pet();
        formatting = encode_storage_header(StorageHeader::Formatting);
        flash.program_verified(STORAGE_BASE, &formatting).await?;
    }

    let mut last_percent = u8::MAX;
    for index in 0..STORAGE_DATA_SECTOR_COUNT {
        let marker_address =
            STORAGE_BASE + STORAGE_PROGRESS_OFFSET + u32::try_from(index).unwrap_or(u32::MAX);
        let mut marker = [0xff_u8; 1];
        flash.read(marker_address, &mut marker)?;
        if marker[0] != 0 {
            let sector_address =
                STORAGE_BASE + STORAGE_SECTOR_SIZE * (u32::try_from(index).unwrap_or(u32::MAX) + 1);
            flash.erase_sector(sector_address).await?;
            watchdog.pet();
            flash.program_verified(marker_address, &[0]).await?;
        }

        let percent = u8::try_from(((index + 1) * 100) / STORAGE_DATA_SECTOR_COUNT).unwrap_or(100);
        if percent != last_percent {
            let _ = UI_EVENTS.try_send(AppEvent::StorageUpdated(StorageState::Formatting(percent)));
            last_percent = percent;
        }
    }

    ready = encode_storage_header(StorageHeader::Ready);
    flash
        .program_verified(STORAGE_BASE + STORAGE_READY_HEADER_OFFSET, &ready)
        .await?;
    watchdog.pet();
    UI_EVENTS
        .send(AppEvent::StorageUpdated(StorageState::Ready))
        .await;
    info!("PineForge storage format complete");
    Ok(())
}

/// Mirrors the reserved region in `docs/FLASH-MAP.md`.
const BOND_ADDRESS: u32 = 0x003F_D000;
const SETTINGS_SLOT_A_ADDRESS: u32 = 0x003F_E000;
const SETTINGS_SLOT_B_ADDRESS: u32 = 0x003F_F000;
/// `MCUBoot` secondary slot in external flash; DFU stages the image here.
const DFU_SLOT_BASE: u32 = 0x0004_0000;

/// Collapses bursts of preset cycling into a single flash write.
const PERSIST_DEBOUNCE: Duration = Duration::from_secs(2);
const CLOCK_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(60 * 60);
const CLOCK_RECORDS_PER_SECTOR: usize = STORAGE_SECTOR_SIZE as usize / CLOCK_RECORD_LEN;

#[derive(Clone, Copy)]
struct ClockJournal {
    current: Option<ClockSnapshot>,
    active_sector: u32,
    next_address: Option<u32>,
    next_sequence: u32,
}

fn record_erased(record: &[u8; CLOCK_RECORD_LEN]) -> bool {
    record.iter().all(|byte| *byte == 0xff)
}

fn read_clock_journal(flash: &mut Xt25f32<FlashSpi>) -> ClockJournal {
    let mut latest: Option<(ClockSnapshot, u32, u32)> = None;
    let mut record = [0_u8; CLOCK_RECORD_LEN];
    for sector in [CLOCK_JOURNAL_A_ADDRESS, CLOCK_JOURNAL_B_ADDRESS] {
        for index in 0..CLOCK_RECORDS_PER_SECTOR {
            let address =
                sector + u32::try_from(index * CLOCK_RECORD_LEN).unwrap_or(STORAGE_SECTOR_SIZE);
            if flash.read(address, &mut record).is_err() {
                continue;
            }
            let Some((snapshot, sequence)) = ClockSnapshot::decode(&record) else {
                continue;
            };
            if latest.is_none_or(|(_, current, _)| clock_sequence_is_newer(sequence, current)) {
                latest = Some((snapshot, sequence, address));
            }
        }
    }

    let Some((snapshot, sequence, latest_address)) = latest else {
        let next_address = [CLOCK_JOURNAL_A_ADDRESS, CLOCK_JOURNAL_B_ADDRESS]
            .into_iter()
            .find_map(|sector| first_erased(flash, sector, sector));
        let active_sector =
            if next_address.is_some_and(|address| address >= CLOCK_JOURNAL_B_ADDRESS) {
                CLOCK_JOURNAL_B_ADDRESS
            } else {
                CLOCK_JOURNAL_A_ADDRESS
            };
        return ClockJournal {
            current: None,
            active_sector,
            next_address,
            next_sequence: 0,
        };
    };

    let active_sector = if latest_address < CLOCK_JOURNAL_B_ADDRESS {
        CLOCK_JOURNAL_A_ADDRESS
    } else {
        CLOCK_JOURNAL_B_ADDRESS
    };
    ClockJournal {
        current: Some(snapshot),
        active_sector,
        next_address: first_erased(
            flash,
            active_sector,
            latest_address + u32::try_from(CLOCK_RECORD_LEN).unwrap_or(32),
        ),
        next_sequence: sequence.wrapping_add(1),
    }
}

fn first_erased(flash: &mut Xt25f32<FlashSpi>, sector: u32, start: u32) -> Option<u32> {
    let end = sector + STORAGE_SECTOR_SIZE;
    let mut address = start;
    let mut record = [0_u8; CLOCK_RECORD_LEN];
    while address + u32::try_from(CLOCK_RECORD_LEN).ok()? <= end {
        flash.read(address, &mut record).ok()?;
        if record_erased(&record) {
            return Some(address);
        }
        address += u32::try_from(CLOCK_RECORD_LEN).ok()?;
    }
    None
}

async fn persist_clock(
    flash: &mut Xt25f32<FlashSpi>,
    journal: &mut ClockJournal,
    snapshot: ClockSnapshot,
) -> Result<(), FlashError<<FlashSpi as embedded_hal::spi::ErrorType>::Error>> {
    if journal.current == Some(snapshot) {
        return Ok(());
    }
    let address = if let Some(address) = journal.next_address {
        address
    } else {
        let next_sector = if journal.active_sector == CLOCK_JOURNAL_A_ADDRESS {
            CLOCK_JOURNAL_B_ADDRESS
        } else {
            CLOCK_JOURNAL_A_ADDRESS
        };
        flash.erase_sector(next_sector).await?;
        journal.active_sector = next_sector;
        next_sector
    };
    let record = snapshot.encode(journal.next_sequence);
    flash.program_verified(address, &record).await?;
    journal.current = Some(snapshot);
    journal.next_sequence = journal.next_sequence.wrapping_add(1);
    let following = address + u32::try_from(CLOCK_RECORD_LEN).unwrap_or(32);
    journal.next_address =
        (following < journal.active_sector + STORAGE_SECTOR_SIZE).then_some(following);
    Ok(())
}

/// Names the serialized bond layout this build reads and writes.
///
/// `build.rs` derives it from the locked BLE stack version, so an update that
/// changes the layout changes the tag without anyone remembering to.
const BOND_SCHEMA: &str = env!("PINEFORGE_BOND_SCHEMA");
/// The layout that records written before tagging carry implicitly.
///
/// Such a record is only trustworthy while this build still reads that same
/// layout - which stops being true by itself as soon as `BOND_SCHEMA` moves,
/// and then the record is discarded in favour of re-pairing.
const UNTAGGED_BOND_SCHEMA: &str = "0.7.0";

const fn slot_address(slot: SettingsSlot) -> u32 {
    match slot {
        SettingsSlot::A => SETTINGS_SLOT_A_ADDRESS,
        SettingsSlot::B => SETTINGS_SLOT_B_ADDRESS,
    }
}

fn read_slot(flash: &mut Xt25f32<FlashSpi>, slot: SettingsSlot) -> Option<(DisplaySettings, u32)> {
    let mut record = [0_u8; SETTINGS_RECORD_LEN];
    flash.read(slot_address(slot), &mut record).ok()?;
    DisplaySettings::decode(&record).ok()
}

fn read_bond(flash: &mut Xt25f32<FlashSpi>) -> Option<StoredBond> {
    let mut record = [0_u8; BOND_RECORD_LEN];
    flash.read(BOND_ADDRESS, &mut record).ok()?;
    let stored = parse_bond(&record)?;

    // The record is intact; the question is whether its bytes still mean what
    // this build would read them to mean. Installing keys decoded under the
    // wrong layout would leave the phone believing in a bond the watch cannot
    // honour, which is worse than pairing again.
    match stored.schema {
        Some(schema) if bond_schema_tag(BOND_SCHEMA) == Some(schema) => Some(stored.payload),
        None if BOND_SCHEMA == UNTAGGED_BOND_SCHEMA => Some(stored.payload),
        _ => {
            warn!("Stored bond was written for another BLE layout; re-pairing");
            None
        }
    }
}

/// Maps a low-level flash error to the on-screen DFU failure reason,
/// defaulting to the operation the caller was performing.
const fn fail_reason<E>(error: &FlashError<E>, default: DfuFailReason) -> DfuFailReason {
    match error {
        FlashError::VerifyFailed => DfuFailReason::VerifyFailed,
        FlashError::Spi(_) | FlashError::BusyTimeout => default,
    }
}

/// Executes one DFU flash op, rebasing and range-checking the image-relative
/// offset so a write can only ever land inside the secondary slot.
async fn run_dfu_command(
    flash: &mut Xt25f32<FlashSpi>,
    command: DfuFlashCommand,
) -> Result<(), DfuFailReason> {
    match command {
        DfuFlashCommand::Erase(offset) => {
            if offset >= DFU_SLOT_SIZE {
                return Err(DfuFailReason::EraseFailed);
            }
            flash
                .erase_sector(DFU_SLOT_BASE + offset)
                .await
                .map_err(|e| fail_reason(&e, DfuFailReason::EraseFailed))
        }
        DfuFlashCommand::Program { offset, data } => {
            let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
            if offset.saturating_add(len) > DFU_SLOT_SIZE {
                return Err(DfuFailReason::ProgramFailed);
            }
            flash
                .program(DFU_SLOT_BASE + offset, &data)
                .await
                .map_err(|e| fail_reason(&e, DfuFailReason::ProgramFailed))
        }
    }
}

#[embassy_executor::task]
#[allow(clippy::too_many_lines)]
pub async fn run(spi: FlashSpi, watchdog: BootloaderWatchdog) {
    let mut flash = Xt25f32::new(spi);
    let sender = DISPLAY_SETTINGS.sender();
    let mut clock_receiver = wall_clock_receiver();

    // A missing or foreign chip degrades to RAM-only settings; the firmware
    // must stay fully usable without persistence. When it is not writable we
    // remember why, so a later DFU attempt can surface the cause on-screen
    // rather than the sealed watch's only symptom being lost settings.
    let mut writable = true;
    let mut flash_fault = DfuFailReason::FlashInitFailed;
    let flash_status = match flash.init().await {
        Ok(id) if is_supported_jedec_id(id) => FlashStatus::Ready(id),
        Ok(id) => {
            warn!(
                "Unexpected flash JEDEC id {=[u8]:#04x}; settings stay in RAM",
                id
            );
            writable = false;
            flash_fault = DfuFailReason::FlashUnrecognized(id);
            FlashStatus::Unrecognized(id)
        }
        Err(_) => {
            warn!("Flash init failed; settings stay in RAM");
            writable = false;
            flash_fault = DfuFailReason::FlashInitFailed;
            FlashStatus::Unavailable
        }
    };
    UI_EVENTS.send(AppEvent::FlashUpdated(flash_status)).await;

    // Publish the stored bond before the potentially long one-time storage
    // format below. The BLE task waits only briefly for it in restore_bond, so
    // signalling here (the bond record lives outside the formatted range) keeps
    // bond restore from racing the format and timing out on the first confirmed
    // boot. Always signal, even when absent, so the BLE task never blocks.
    let bond = if writable {
        read_bond(&mut flash)
    } else {
        None
    };
    info!("BLE bond loaded: stored={}", bond.is_some());
    BOND_LOADED.signal(bond);

    let storage_ready = if writable && crate::boot::confirm::is_validated() {
        if initialize_storage(&mut flash, watchdog).await.is_err() {
            warn!("PineForge storage initialization failed");
            UI_EVENTS
                .send(AppEvent::StorageUpdated(StorageState::Failed))
                .await;
            false
        } else {
            true
        }
    } else if writable {
        info!("Storage remains reserved while firmware rollback is possible");
        false
    } else {
        false
    };

    let decision = if writable {
        select_slot(
            read_slot(&mut flash, SettingsSlot::A),
            read_slot(&mut flash, SettingsSlot::B),
        )
    } else {
        select_slot(None, None)
    };
    let mut current = decision
        .current
        .map_or(DisplaySettings::DEFAULT, |(settings, _)| settings);
    let mut write_slot = decision.write_slot;
    let mut next_sequence = decision.next_sequence;
    info!(
        "Settings loaded: stored={} sequence={}",
        decision.current.is_some(),
        next_sequence
    );
    sender.send(current);

    // The clock journal deliberately lives in the confirmed-image data area.
    // A trial image must never initialize or mutate it, because MCUBoot may
    // still roll back to firmware that knows nothing about PineForge storage.
    let mut clock_journal = storage_ready.then(|| read_clock_journal(&mut flash));
    let mut current_reference = clock_receiver.try_changed();
    if current_reference.is_none()
        && let Some(snapshot) = clock_journal.as_ref().and_then(|journal| journal.current)
    {
        let now = Instant::now().as_secs();
        let reference = snapshot.reference_at(now);
        current_reference = Some(reference);
        WALL_CLOCK.sender().send(reference);
        info!("Wall clock restored from external flash");
    }
    // A BLE reference may have arrived while storage was formatting. Since
    // `try_changed` consumes it above, persist it here rather than waiting for
    // the first hourly checkpoint. A restored reference is a duplicate and
    // `persist_clock` intentionally turns that into no write.
    if let Some(reference) = current_reference
        && let Some(journal) = clock_journal.as_mut()
        && let Err(error) = persist_clock(
            &mut flash,
            journal,
            ClockSnapshot::from_reference(reference, Instant::now().as_secs()),
        )
        .await
    {
        warn!("Initial clock write failed; continuing without persistence");
        writable = false;
        flash_fault = fail_reason(&error, DfuFailReason::ProgramFailed);
    }
    let mut next_clock_checkpoint = if current_reference.is_some() {
        Instant::now() + CLOCK_CHECKPOINT_INTERVAL
    } else {
        Instant::MAX
    };

    loop {
        match select3(
            select3(
                SETTINGS_COMMANDS.receive(),
                BOND_STORE.receive(),
                DFU_FLASH_COMMANDS.receive(),
            ),
            clock_receiver.changed(),
            Timer::at(next_clock_checkpoint),
        )
        .await
        {
            Either3::First(Either3::First(first)) => {
                let mut pending = first;
                sender.send(pending);
                while let Ok(next) = with_deadline(
                    Instant::now() + PERSIST_DEBOUNCE,
                    SETTINGS_COMMANDS.receive(),
                )
                .await
                {
                    pending = next;
                    sender.send(pending);
                }

                if pending == current {
                    continue;
                }
                if writable {
                    let address = slot_address(write_slot);
                    let record = pending.encode(next_sequence);
                    let written = match flash.erase_sector(address).await {
                        Ok(()) => flash.program_verified(address, &record).await,
                        Err(error) => Err(error),
                    };
                    match written {
                        Ok(()) => {
                            info!("Settings persisted with sequence {}", next_sequence);
                            write_slot = write_slot.other();
                            next_sequence = next_sequence.wrapping_add(1);
                        }
                        Err(error) => {
                            // Keep the snapshot live but stop wearing a failing
                            // chip; remember why for a later DFU diagnosis.
                            warn!("Settings write failed; continuing without persistence");
                            writable = false;
                            flash_fault = fail_reason(&error, DfuFailReason::ProgramFailed);
                        }
                    }
                }
                current = pending;
            }
            Either3::First(Either3::Second(payload)) => {
                if !writable {
                    continue;
                }
                let Some(record) =
                    bond_schema_tag(BOND_SCHEMA).and_then(|schema| frame_bond(schema, &payload))
                else {
                    warn!("Bond payload too large to persist");
                    continue;
                };
                let written = match flash.erase_sector(BOND_ADDRESS).await {
                    Ok(()) => flash.program_verified(BOND_ADDRESS, &record).await,
                    Err(error) => Err(error),
                };
                if written.is_ok() {
                    info!("BLE bond persisted");
                } else {
                    warn!("Bond write failed; pairing will not survive reboot");
                }
            }
            Either3::First(Either3::Third(command)) => {
                let result = if writable {
                    run_dfu_command(&mut flash, command).await
                } else {
                    // Never writable, so no op ran; report why it can't.
                    Err(flash_fault)
                };
                DFU_FLASH_RESULT.send(result).await;
            }
            Either3::Second(reference) => {
                current_reference = Some(reference);
                next_clock_checkpoint = Instant::now() + CLOCK_CHECKPOINT_INTERVAL;
                let now = Instant::now().as_secs();
                if writable
                    && let Some(journal) = clock_journal.as_mut()
                    && let Err(error) = persist_clock(
                        &mut flash,
                        journal,
                        ClockSnapshot::from_reference(reference, now),
                    )
                    .await
                {
                    warn!("Clock write failed; continuing without persistence");
                    writable = false;
                    flash_fault = fail_reason(&error, DfuFailReason::ProgramFailed);
                }
            }
            Either3::Third(()) => {
                let now = Instant::now();
                if let Some(reference) = current_reference
                    && writable
                    && let Some(journal) = clock_journal.as_mut()
                    && let Err(error) = persist_clock(
                        &mut flash,
                        journal,
                        ClockSnapshot::from_reference(reference, now.as_secs()),
                    )
                    .await
                {
                    warn!("Clock checkpoint failed; continuing without persistence");
                    writable = false;
                    flash_fault = fail_reason(&error, DfuFailReason::ProgramFailed);
                }
                next_clock_checkpoint = now + CLOCK_CHECKPOINT_INTERVAL;
            }
        }
    }
}
