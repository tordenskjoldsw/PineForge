//! GATT services: Device Information, Battery, and Current Time.
//!
//! Gadgetbridge reads the firmware revision from the Device Information
//! Service and writes the phone's time into the Current Time characteristic
//! right after connecting.

use defmt::{info, warn};
use embassy_futures::select::select;
use embassy_time::Instant;
use pineforge_state::{AppEvent, BleState, VibrationPattern, parse_cts};
use trouble_host::prelude::*;

use pineforge_state::BOND_PAYLOAD_MAX;

use crate::services::events::{
    BOND_STORE, BatteryStatusReceiver, StoredBond, UI_EVENTS, VIBRATION_COMMANDS, WALL_CLOCK,
};

#[gatt_server]
pub struct Server {
    pub device_information: DeviceInformationService,
    pub battery: BatteryService,
    pub current_time: CurrentTimeService,
}

// Gadgetbridge identifies InfiniTime devices by these Device Information
// strings — in particular the software revision "InfiniTime". They must match
// stock InfiniTime for the companion to run its full init and sync the time.
#[gatt_service(uuid = service::DEVICE_INFORMATION)]
pub struct DeviceInformationService {
    #[characteristic(
        uuid = characteristic::MANUFACTURER_NAME_STRING,
        read,
        value = heapless::String::try_from("PINE64").unwrap()
    )]
    manufacturer: heapless::String<16>,
    #[characteristic(
        uuid = characteristic::MODEL_NUMBER_STRING,
        read,
        value = heapless::String::try_from("PineTime").unwrap()
    )]
    model: heapless::String<16>,
    #[characteristic(
        uuid = characteristic::SERIAL_NUMBER_STRING,
        read,
        value = heapless::String::try_from("0").unwrap()
    )]
    serial_number: heapless::String<16>,
    #[characteristic(
        uuid = characteristic::FIRMWARE_REVISION_STRING,
        read,
        value = heapless::String::try_from(env!("CARGO_PKG_VERSION")).unwrap()
    )]
    firmware_revision: heapless::String<16>,
    #[characteristic(
        uuid = characteristic::HARDWARE_REVISION_STRING,
        read,
        value = heapless::String::try_from("1.0.0").unwrap()
    )]
    hardware_revision: heapless::String<16>,
    #[characteristic(
        uuid = characteristic::SOFTWARE_REVISION_STRING,
        read,
        value = heapless::String::try_from("InfiniTime").unwrap()
    )]
    software_revision: heapless::String<16>,
}

#[gatt_service(uuid = service::BATTERY)]
pub struct BatteryService {
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 0)]
    pub level: u8,
}

#[gatt_service(uuid = service::CURRENT_TIME)]
pub struct CurrentTimeService {
    /// Standard current-time layout: year, month, day, h, m, s, weekday,
    /// fractions, adjust reason.
    #[characteristic(uuid = characteristic::CURRENT_TIME, read, write, value = [0; 10])]
    pub current_time: [u8; 10],
    /// Timezone and DST offset. Accepted so the companion's time-sync
    /// sequence completes; the value itself is not yet applied.
    #[characteristic(uuid = characteristic::LOCAL_TIME_INFORMATION, read, write, value = [0; 2])]
    pub local_time: [u8; 2],
}

/// Serves GATT events and battery notifications until the central
/// disconnects.
pub async fn serve(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, DefaultPacketPool>,
    battery: &mut BatteryStatusReceiver,
) {
    select(
        gatt_events(server, connection),
        notify_battery(server, connection, battery),
    )
    .await;
}

async fn gatt_events(server: &Server<'_>, connection: &GattConnection<'_, '_, DefaultPacketPool>) {
    let cts_handle = server.current_time.current_time.handle;
    loop {
        match connection.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                info!("BLE disconnected: {}", defmt::Debug2Format(&reason));
                return;
            }
            // Gadgetbridge bonds like it does with InfiniTime: show the passkey
            // so the user confirms it on the phone.
            GattConnectionEvent::PassKeyDisplay(passkey) => {
                info!("Pairing passkey: {}", passkey.value());
                UI_EVENTS
                    .send(AppEvent::BleUpdated(BleState::Pairing(passkey.value())))
                    .await;
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Long);
            }
            GattConnectionEvent::PairingComplete {
                security_level,
                bond,
            } => {
                info!("Pairing complete: {}", defmt::Debug2Format(&security_level));
                UI_EVENTS
                    .send(AppEvent::BleUpdated(BleState::Connected))
                    .await;
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Double);
                // Persist the bond so the pairing survives a reboot. postcard
                // ships a different heapless version, so serialize into a plain
                // buffer and copy into our own vector type.
                if let Some(bond) = bond {
                    let mut buffer = [0_u8; BOND_PAYLOAD_MAX];
                    if let Ok(used) = postcard::to_slice(&bond, &mut buffer) {
                        if let Ok(payload) = StoredBond::from_slice(used) {
                            let _ = BOND_STORE.try_send(payload);
                        }
                    } else {
                        warn!("Bond too large to serialize");
                    }
                }
            }
            GattConnectionEvent::PairingFailed(error) => {
                warn!("Pairing failed: {}", defmt::Debug2Format(&error));
                UI_EVENTS
                    .send(AppEvent::BleUpdated(BleState::Connected))
                    .await;
            }
            GattConnectionEvent::Gatt { event } => {
                if let GattEvent::Write(write) = &event {
                    if write.handle() == cts_handle {
                        let uptime_seconds = Instant::now().as_secs();
                        let reference = write.with_data(|offset, data| {
                            (offset == 0)
                                .then(|| parse_cts(data, uptime_seconds))
                                .flatten()
                        });
                        if let Some(reference) = reference {
                            info!(
                                "Time synchronized: {:02}:{:02}:{:02}",
                                reference.hour, reference.minute, reference.second
                            );
                            WALL_CLOCK.sender().send(reference);
                            // Tactile confirmation that a time sync landed;
                            // distinct from the single connect tick.
                            let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Double);
                        } else {
                            warn!("Rejecting malformed current-time write");
                        }
                    }
                }
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(error) => warn!("GATT reply error: {}", defmt::Debug2Format(&error)),
                }
            }
            _ => {}
        }
    }
}

async fn notify_battery(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, DefaultPacketPool>,
    battery: &mut BatteryStatusReceiver,
) {
    if let Some(status) = battery.try_get() {
        let _ = server
            .battery
            .level
            .notify(connection, &status.percent, true)
            .await;
    }
    loop {
        let status = battery.changed().await;
        let _ = server
            .battery
            .level
            .notify(connection, &status.percent, true)
            .await;
    }
}
