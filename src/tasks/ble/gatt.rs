//! GATT services: Device Information, Battery, and Current Time.
//!
//! Gadgetbridge reads the firmware revision from the Device Information
//! Service and writes the phone's time into the Current Time characteristic
//! right after connecting.

use defmt::{info, warn};
use embassy_futures::select::select;
use embassy_time::{Duration, Instant, with_deadline};
use pineforge_state::{
    AppEvent, BOND_PAYLOAD_MAX, BleState, DfuEngine, DfuFailReason, Notification, VibrationPattern,
    parse_cts, parse_new_alert,
};
use trouble_host::prelude::*;

use crate::{
    ipc::{
        BOND_STORE, BatteryStatusReceiver, NOTIFICATIONS, StoredBond, UI_EVENTS,
        VIBRATION_COMMANDS, WALL_CLOCK,
    },
    tasks::ble::dfu,
};

/// Maximum value bytes in an ATT write at the stack's negotiated 251-byte
/// MTU (one opcode byte and a two-byte attribute handle precede the value).
const DFU_PACKET_MAX: usize = DefaultPacketPool::MTU - 3;

/// How long a transfer in flight may go without a DFU write before the watch
/// abandons it. `InfiniTime` uses the same ten seconds, restarted on every
/// access to its DFU service; a streaming host beats this by orders of
/// magnitude, so only one that has genuinely stopped ever hits it.
const DFU_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

#[gatt_server]
pub struct Server {
    pub device_information: DeviceInformationService,
    pub battery: BatteryService,
    pub current_time: CurrentTimeService,
    pub dfu: DfuService,
    pub alert_notification: AlertNotificationService,
}

/// Largest New Alert write accepted, one ATT payload at the negotiated MTU, so
/// no single-packet notification is ever rejected for length. Text past the
/// parser's title/body bounds is truncated, not dropped.
const NEW_ALERT_MAX: usize = DefaultPacketPool::MTU - 3;

/// Standard Alert Notification Service. Gadgetbridge writes phone
/// notifications to the New Alert characteristic exactly as it does for
/// InfiniTime: a `[category, count, reserved]` header followed by the text.
#[gatt_service(uuid = service::ALERT_NOTIFICATION)]
pub struct AlertNotificationService {
    #[characteristic(uuid = characteristic::NEW_ALERT, write)]
    pub new_alert: heapless::Vec<u8, NEW_ALERT_MAX>,
}

/// Nordic legacy DFU service, as spoken by Gadgetbridge's InfiniTime
/// firmware installer.
#[gatt_service(uuid = "00001530-1212-efde-1523-785feabcd123")]
pub struct DfuService {
    #[characteristic(uuid = "00001531-1212-efde-1523-785feabcd123", write, notify, value = [0; 20])]
    pub control_point: [u8; 20],
    #[characteristic(uuid = "00001532-1212-efde-1523-785feabcd123", write_without_response)]
    pub packet: heapless::Vec<u8, DFU_PACKET_MAX>,
    #[characteristic(uuid = "00001534-1212-efde-1523-785feabcd123", read, value = 8)]
    pub revision: u16,
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
        value = heapless::String::try_from(env!("PINEFORGE_VERSION")).unwrap()
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

#[allow(clippy::too_many_lines)]
async fn gatt_events(server: &Server<'_>, connection: &GattConnection<'_, '_, DefaultPacketPool>) {
    let cts_handle = server.current_time.current_time.handle;
    let dfu_control_handle = server.dfu.control_point.handle;
    let dfu_packet_handle = server.dfu.packet.handle;
    let new_alert_handle = server.alert_notification.new_alert.handle;
    let mut engine = DfuEngine::new();
    let mut dfu_flash = dfu::FlashPipeline::new();
    // Last percent pushed to the update screen, so we only redraw on change.
    let mut last_dfu_pct: Option<u8> = None;
    // When a transfer in flight must have been fed by, so a host that stops
    // sending without disconnecting cannot wedge the engine. Held as a
    // deadline rather than a per-wait timeout so unrelated traffic - a time
    // sync, a battery subscription - does not keep a dead transfer alive.
    let mut dfu_deadline: Option<Instant> = None;
    loop {
        let next = match dfu_deadline {
            Some(deadline) => with_deadline(deadline, connection.next()).await,
            None => Ok(connection.next().await),
        };
        let Ok(event) = next else {
            warn!("DFU stalled; abandoning the transfer");
            dfu_deadline = None;
            last_dfu_pct = None;
            // Queued flash is joined before the engine forgets what it was
            // writing, so no result from this transfer is left for the next
            // one to reap. A concrete flash failure has already been reported
            // and outranks the stall.
            if dfu::finish_pending_flash(&mut dfu_flash).await {
                let _ = UI_EVENTS.try_send(AppEvent::BleUpdated(BleState::DfuFailed(
                    DfuFailReason::TimedOut,
                )));
            }
            engine.abort();
            continue;
        };
        match event {
            GattConnectionEvent::Disconnected { reason } => {
                info!("BLE disconnected: {}", defmt::Debug2Format(&reason));
                // Do not leave results from this connection in the global
                // channel for a later DFU session to mistake as its own.
                let _ = dfu::finish_pending_flash(&mut dfu_flash).await;
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
                // Captured DFU write (is_control_point, buffer, length), acted
                // on after the write is accepted so notifications flow cleanly.
                let mut dfu_write: Option<(bool, [u8; DFU_PACKET_MAX], usize)> = None;
                // A parsed phone notification, likewise acted on after accept.
                let mut alert: Option<Notification> = None;
                if let GattEvent::Write(write) = &event {
                    let handle = write.handle();
                    if handle == cts_handle {
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
                    } else if handle == dfu_control_handle || handle == dfu_packet_handle {
                        let mut buffer = [0_u8; DFU_PACKET_MAX];
                        let len = write.with_data(|_, data| {
                            let len = data.len().min(buffer.len());
                            buffer[..len].copy_from_slice(&data[..len]);
                            len
                        });
                        dfu_write = Some((handle == dfu_control_handle, buffer, len));
                    } else if handle == new_alert_handle {
                        alert = write.with_data(|_, data| parse_new_alert(data));
                    }
                }
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(error) => warn!("GATT reply error: {}", defmt::Debug2Format(&error)),
                }
                if let Some((is_control_point, buffer, len)) = dfu_write {
                    dfu::handle_write(
                        &mut engine,
                        &mut dfu_flash,
                        &server.dfu,
                        connection,
                        is_control_point,
                        &buffer[..len],
                    )
                    .await;
                    report_dfu_progress(&engine, &mut last_dfu_pct);
                    // Set after the write is executed, so the flash time this
                    // very packet cost is not charged against the host's next
                    // one. An engine back at idle - finished, aborted, or
                    // reset by the host - carries no deadline at all.
                    dfu_deadline = engine
                        .is_active()
                        .then(|| Instant::now() + DFU_IDLE_TIMEOUT);
                }
                if let Some(notification) = alert {
                    info!(
                        "Notification [{}]: {} - {}",
                        defmt::Debug2Format(&notification.category),
                        notification.title.as_str(),
                        notification.body.as_str()
                    );
                    // One long alert pulse. The message itself goes to the
                    // display task, which files it and reports the new tally -
                    // one place decides what is pending, so the watchface can
                    // never disagree with the notification screen.
                    let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Long);
                    if NOTIFICATIONS.try_send(notification).is_err() {
                        // A full queue means the display task has not run since
                        // the last two arrived. Dropping is the honest outcome:
                        // blocking the GATT loop here would stall every other
                        // characteristic, DFU included.
                        warn!("Notification dropped: the inbox queue is full");
                    }
                }
            }
            _ => {}
        }
    }
}

/// Surfaces DFU transfer progress on the watch's update screen, but only
/// when the whole percent changed so a full image triggers at most 101
/// redraws, not one per packet.
fn report_dfu_progress(engine: &DfuEngine, last_pct: &mut Option<u8>) {
    let pct = engine.progress_percent();
    if pct != *last_pct {
        *last_pct = pct;
        if let Some(pct) = pct {
            let _ = UI_EVENTS.try_send(AppEvent::BleUpdated(BleState::DfuProgress(pct)));
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
