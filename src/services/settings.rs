//! Owns the external flash and the authoritative display settings snapshot.
//!
//! Boot decodes both record slots and publishes the newest valid settings or
//! the built-in defaults. UI-submitted snapshots are published immediately
//! and persisted debounced, alternating between the two slots documented in
//! `docs/FLASH-MAP.md`.

use defmt::{info, warn};
use embassy_time::{Duration, Instant, with_deadline};
use pineforge_state::{DisplaySettings, SETTINGS_RECORD_LEN, SettingsSlot, select_slot};

use crate::{
    board::buses::FlashSpi,
    drivers::xt25f32::{EXPECTED_JEDEC_ID, Xt25f32},
    services::events::{DISPLAY_SETTINGS, SETTINGS_COMMANDS},
};

/// Mirrors the reserved region in `docs/FLASH-MAP.md`.
const SETTINGS_SLOT_A_ADDRESS: u32 = 0x003F_E000;
const SETTINGS_SLOT_B_ADDRESS: u32 = 0x003F_F000;

/// Collapses bursts of preset cycling into a single flash write.
const PERSIST_DEBOUNCE: Duration = Duration::from_secs(2);

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

#[embassy_executor::task]
pub async fn run(spi: FlashSpi) {
    let mut flash = Xt25f32::new(spi);
    let sender = DISPLAY_SETTINGS.sender();

    // A missing or foreign chip degrades to RAM-only settings; the firmware
    // must stay fully usable without persistence.
    let mut writable = match flash.init().await {
        Ok(id) if id == EXPECTED_JEDEC_ID => true,
        Ok(id) => {
            warn!(
                "Unexpected flash JEDEC id {=[u8]:#04x}; settings stay in RAM",
                id
            );
            false
        }
        Err(_) => {
            warn!("Flash init failed; settings stay in RAM");
            false
        }
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

    loop {
        let mut pending = SETTINGS_COMMANDS.receive().await;
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
            if written.is_ok() {
                info!("Settings persisted with sequence {}", next_sequence);
                write_slot = write_slot.other();
                next_sequence = next_sequence.wrapping_add(1);
            } else {
                // Keep the snapshot live but stop wearing a failing chip.
                warn!("Settings write failed; continuing without persistence");
                writable = false;
            }
        }
        current = pending;
    }
}
