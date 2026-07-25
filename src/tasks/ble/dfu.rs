//! Executes the DFU engine's steps: routes flash operations to the storage
//! service, sends control-point notifications, and reboots on activation.

use defmt::{info, warn};
use embassy_time::Timer;
use pineforge_state::{AppEvent, BleState, DfuEngine, DfuFailReason, DfuStep};
use trouble_host::prelude::*;

use crate::{
    services::events::{DFU_FLASH_COMMANDS, DFU_FLASH_RESULT, DfuFlashCommand, UI_EVENTS},
    tasks::ble::gatt::DfuService,
};

pub struct FlashPipeline {
    pending: usize,
    first_error: Option<DfuFailReason>,
}

impl FlashPipeline {
    pub const fn new() -> Self {
        Self {
            pending: 0,
            first_error: None,
        }
    }

    fn reap_ready(&mut self) {
        while self.pending != 0 {
            let Ok(result) = DFU_FLASH_RESULT.try_receive() else {
                break;
            };
            if let Err(reason) = result {
                self.first_error = self.first_error.or(Some(reason));
            }
            self.pending -= 1;
        }
    }

    /// Reports the first recorded flash failure to the update screen. Returns
    /// `false` when one occurred so the caller aborts the transfer; the image
    /// then never validates and the secondary slot is never activated.
    fn report_ok(&mut self) -> bool {
        self.first_error.take().is_none_or(|reason| {
            warn!("DFU pipelined flash operation failed");
            let _ = UI_EVENTS.try_send(AppEvent::BleUpdated(BleState::DfuFailed(reason)));
            false
        })
    }
}

/// Feeds one characteristic write to the engine and executes the resulting
/// steps in order.
pub async fn handle_write(
    engine: &mut DfuEngine,
    flash: &mut FlashPipeline,
    service: &DfuService,
    connection: &GattConnection<'_, '_, DefaultPacketPool>,
    is_control_point: bool,
    data: &[u8],
) {
    let steps = if is_control_point {
        engine.control_write(data)
    } else {
        engine.packet_write(data)
    };
    for step in steps {
        match step {
            DfuStep::Notify(bytes) => {
                // A protocol ack the host acts on: drain the pipeline first so
                // it never advances past flash that is still in flight or has
                // failed.
                if !finish_pending_flash(flash).await {
                    return;
                }
                let _ = service
                    .control_point
                    .notify_raw(connection, &bytes, false)
                    .await;
            }
            DfuStep::Receipt(bytes) => {
                // Pure flow control: releasing this lets Gadgetbridge send the
                // next batch, so keep the radio moving instead of stalling on
                // in-flight flash. Only surface an already-known failure; a
                // later one is caught at the next ack. This overlap of receive
                // and flash is the pipeline's whole purpose.
                flash.reap_ready();
                if !flash.report_ok() {
                    return;
                }
                let _ = service
                    .control_point
                    .notify_raw(connection, &bytes, false)
                    .await;
            }
            DfuStep::Erase(offset) => {
                flash.reap_ready();
                DFU_FLASH_COMMANDS
                    .send(DfuFlashCommand::Erase(offset))
                    .await;
                flash.pending += 1;
            }
            DfuStep::Program { offset, data } => {
                flash.reap_ready();
                DFU_FLASH_COMMANDS
                    .send(DfuFlashCommand::Program { offset, data })
                    .await;
                flash.pending += 1;
            }
            DfuStep::Reset => {
                if !finish_pending_flash(flash).await {
                    return;
                }
                info!("DFU activate: resetting to apply image");
                // Let the final notification flush before the reset.
                Timer::after_millis(200).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
    }
}

/// Joins every queued flash operation before a protocol acknowledgement.
/// Results stay ordered because the storage service executes commands FIFO.
pub async fn finish_pending_flash(flash: &mut FlashPipeline) -> bool {
    flash.reap_ready();
    while flash.pending != 0 {
        if let Err(reason) = DFU_FLASH_RESULT.receive().await {
            flash.first_error = flash.first_error.or(Some(reason));
        }
        flash.pending -= 1;
    }
    flash.report_ok()
}
