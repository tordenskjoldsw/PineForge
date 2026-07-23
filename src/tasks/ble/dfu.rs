//! Executes the DFU engine's steps: routes flash operations to the storage
//! service, sends control-point notifications, and reboots on activation.

use defmt::{info, warn};
use embassy_time::Timer;
use pineforge_state::{DfuEngine, DfuStep};
use trouble_host::prelude::*;

use crate::{
    services::events::{DFU_FLASH_COMMANDS, DFU_FLASH_RESULT, DfuFlashCommand},
    tasks::ble::gatt::DfuService,
};

/// Feeds one characteristic write to the engine and executes the resulting
/// steps in order.
pub async fn handle_write(
    engine: &mut DfuEngine,
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
                let _ = service
                    .control_point
                    .notify_raw(connection, &bytes, false)
                    .await;
            }
            DfuStep::Erase(offset) => {
                DFU_FLASH_COMMANDS
                    .send(DfuFlashCommand::Erase(offset))
                    .await;
                if !DFU_FLASH_RESULT.receive().await {
                    warn!("DFU erase failed at offset {}", offset);
                    return;
                }
            }
            DfuStep::Program { offset, data } => {
                DFU_FLASH_COMMANDS
                    .send(DfuFlashCommand::Program { offset, data })
                    .await;
                if !DFU_FLASH_RESULT.receive().await {
                    warn!("DFU program failed at offset {}", offset);
                    return;
                }
            }
            DfuStep::Reset => {
                info!("DFU activate: resetting to apply image");
                // Let the final notification flush before the reset.
                Timer::after_millis(200).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
    }
}
