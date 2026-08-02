//! BLE bring-up: MPSL + `SoftDevice` Controller + `TrouBLE` host.
//!
//! Advertises as "`InfiniTime`" so Gadgetbridge's `InfiniTime` coordinator
//! recognizes the watch. Every fallible init step degrades to a warning and
//! task exit instead of panicking, so a broken stack leaves the rest of the
//! firmware running (same philosophy as the RAM-only settings fallback).

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_nrf::{bind_interrupts, mode::Async, peripherals, rng};
use embassy_time::{Duration, with_timeout};
use nrf_sdc::{
    self as sdc,
    mpsl::{self, MultiprotocolServiceLayer},
};
use pineforge_state::{AppEvent, BleState, VibrationPattern};
use static_cell::StaticCell;
use trouble_host::prelude::*;

use crate::{
    board::peripherals::BleResources,
    ipc::{
        BOND_LOADED, BatteryStatusReceiver, DisplaySettingsReceiver, UI_EVENTS, VIBRATION_COMMANDS,
        battery_status_receiver, display_settings_receiver,
    },
};

mod dfu;
mod gatt;
mod music;
use gatt::Server;

bind_interrupts!(struct BleIrqs {
    RNG => rng::InterruptHandler<peripherals::RNG>;
    EGU0_SWI0 => mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => mpsl::ClockInterruptHandler;
    RADIO => mpsl::HighPrioInterruptHandler;
    TIMER0 => mpsl::HighPrioInterruptHandler;
    RTC0 => mpsl::HighPrioInterruptHandler;
});

/// Gadgetbridge matches its `InfiniTime` device coordinator on this name.
const DEVICE_NAME: &str = "InfiniTime";

const CONNECTIONS_MAX: usize = 1;
/// L2CAP signalling + ATT.
const L2CAP_CHANNELS_MAX: usize = 2;
const L2CAP_TXQ: u8 = 3;
const L2CAP_RXQ: u8 = 3;
const L2CAP_MTU: u16 = 251;
const _: () = assert!(DefaultPacketPool::MTU == L2CAP_MTU as usize);
const LFCLK_SOURCE_XTAL: u8 = mpsl::raw::MPSL_CLOCK_LF_SRC_XTAL.to_le_bytes()[0];

static MPSL: StaticCell<MultiprotocolServiceLayer<'static>> = StaticCell::new();
static SDC_MEM: StaticCell<sdc::Mem<4720>> = StaticCell::new();
static SDC_RNG: StaticCell<rng::Rng<'static, Async>> = StaticCell::new();

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

/// Builds a static random device address from the factory-programmed FICR
/// address, mirroring how stock firmwares derive their identity.
fn device_address() -> Address {
    let ficr = nrf_pac::FICR;
    let low = ficr.deviceaddr(0).read().to_le_bytes();
    let high = ficr.deviceaddr(1).read().to_le_bytes();
    Address::random([
        low[0],
        low[1],
        low[2],
        low[3],
        high[0],
        // A static random address requires the two most significant bits set.
        high[1] | 0xc0,
    ])
}

#[embassy_executor::task]
pub async fn run(resources: BleResources, spawner: Spawner) {
    let mpsl_peripherals = mpsl::Peripherals::new(
        resources.rtc0,
        resources.timer0,
        resources.temp,
        resources.ppi_ch19,
        resources.ppi_ch30,
        resources.ppi_ch31,
    );
    // The board runs the external 32.768 kHz crystal (see main.rs); 50 ppm is
    // a conservative accuracy bound for the PineTime LFXO.
    let lfclk_config = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: LFCLK_SOURCE_XTAL,
        rc_ctiv: 0,
        rc_temp_ctiv: 0,
        accuracy_ppm: 50,
        skip_wait_lfclk_started: mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
    };
    let mpsl = match mpsl::MultiprotocolServiceLayer::new(mpsl_peripherals, BleIrqs, lfclk_config) {
        Ok(mpsl) => MPSL.init(mpsl),
        Err(error) => {
            warn!("MPSL init failed: {}; continuing without BLE", error);
            return;
        }
    };
    let Ok(mpsl_token) = mpsl_task(mpsl) else {
        warn!("MPSL task spawn failed; continuing without BLE");
        return;
    };
    spawner.spawn(mpsl_token);

    let sdc_peripherals = sdc::Peripherals::new(
        resources.ppi_ch17,
        resources.ppi_ch18,
        resources.ppi_ch20,
        resources.ppi_ch21,
        resources.ppi_ch22,
        resources.ppi_ch23,
        resources.ppi_ch24,
        resources.ppi_ch25,
        resources.ppi_ch26,
        resources.ppi_ch27,
        resources.ppi_ch28,
        resources.ppi_ch29,
    );
    let sdc_rng = SDC_RNG.init(rng::Rng::new(resources.rng, BleIrqs));
    let sdc_mem = SDC_MEM.init(sdc::Mem::new());
    let controller = match build_sdc(sdc_peripherals, sdc_rng, mpsl, sdc_mem) {
        Ok(controller) => controller,
        Err(error) => {
            warn!("SDC init failed: {}; continuing without BLE", error);
            return;
        }
    };

    let address = device_address();
    info!("BLE address: {}", defmt::Debug2Format(&address.addr));
    let mut host_resources: HostResources<
        sdc::SoftdeviceController<'_>,
        DefaultPacketPool,
        CONNECTIONS_MAX,
        L2CAP_CHANNELS_MAX,
    > = HostResources::new();
    let stack = trouble_host::new(controller, &mut host_resources)
        .set_random_address(address)
        // Present a passkey for the central to confirm, matching InfiniTime's
        // bonding so Gadgetbridge completes pairing and syncs the time.
        .set_io_capabilities(IoCapabilities::DisplayOnly)
        .build();
    restore_bond(&stack).await;
    let mut runner = stack.runner();
    let mut peripheral = stack.peripheral();
    let server = match Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: DEVICE_NAME,
        appearance: &appearance::watch::SMARTWATCH,
    })) {
        Ok(server) => server,
        Err(error) => {
            warn!("GATT server init failed: {}; continuing without BLE", error);
            return;
        }
    };
    // The device-information service is served from the attribute table alone;
    // binding it here marks it intentionally live for the borrow checker.
    let _dis = &server.device_information;
    let mut battery = battery_status_receiver();
    let mut settings = display_settings_receiver();

    embassy_futures::join::join(
        ble_runner(&mut runner),
        manage_radio(&mut peripheral, &server, &mut battery, &mut settings),
    )
    .await;
}

/// Runs advertising and connections only while the persisted user setting is
/// enabled. Dropping the advertising future cancels its controller command;
/// dropping a live connection releases its last handle, which asks Trouble to
/// disconnect it. The runner stays alive so those cancellations are processed
/// and enabling again does not rebuild the controller or lose the bond.
async fn manage_radio<C: Controller>(
    peripheral: &mut Peripheral<'_, C, DefaultPacketPool>,
    server: &Server<'_>,
    battery: &mut BatteryStatusReceiver,
    settings: &mut DisplaySettingsReceiver,
) {
    let mut enabled = settings.get().await.ble_enabled();
    loop {
        if !enabled {
            UI_EVENTS.send(AppEvent::BleUpdated(BleState::Off)).await;
            loop {
                if settings.changed().await.ble_enabled() {
                    enabled = true;
                    break;
                }
            }
        }

        match select(
            advertise_and_serve(peripheral, server, battery),
            wait_until_disabled(settings),
        )
        .await
        {
            Either::First(Ok(())) => {}
            Either::First(Err(error)) => {
                warn!("BLE advertise error: {}", defmt::Debug2Format(&error));
                embassy_time::Timer::after_secs(1).await;
            }
            Either::Second(()) => enabled = false,
        }
    }
}

async fn wait_until_disabled(settings: &mut DisplaySettingsReceiver) {
    loop {
        if !settings.changed().await.ble_enabled() {
            return;
        }
    }
}

/// Installs the bond persisted by the storage service so a paired phone
/// reconnects without re-pairing. Waits briefly for the storage service to
/// publish it, then proceeds regardless.
async fn restore_bond(stack: &Stack<'_, sdc::SoftdeviceController<'_>, DefaultPacketPool>) {
    let Ok(loaded) = with_timeout(Duration::from_secs(3), BOND_LOADED.wait()).await else {
        warn!("Bond load timed out; continuing unbonded");
        return;
    };
    let Some(payload) = loaded else {
        info!("No stored BLE bond");
        return;
    };
    let Ok(bond) = postcard::from_bytes::<BondInformation>(&payload) else {
        warn!("Stored bond failed to deserialize");
        return;
    };
    match stack.add_bond_information(bond) {
        Ok(()) => info!("Restored BLE bond"),
        Err(error) => warn!("Bond install failed: {}", defmt::Debug2Format(&error)),
    }
}

fn build_sdc<'d, const N: usize>(
    peripherals: sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<'static, Async>,
    mpsl: &'d MultiprotocolServiceLayer<'static>,
    mem: &'d mut sdc::Mem<N>,
) -> Result<sdc::SoftdeviceController<'d>, sdc::Error> {
    sdc::Builder::new()?
        .support_adv()
        .support_peripheral()
        .peripheral_count(1)?
        .buffer_cfg(L2CAP_MTU, L2CAP_MTU, L2CAP_TXQ, L2CAP_RXQ)?
        .build(peripherals, rng, mpsl, mem)
}

async fn ble_runner<C: Controller, P: PacketPool>(runner: &mut Runner<'_, C, P>) {
    loop {
        if let Err(error) = runner.run().await {
            warn!("BLE runner error: {}", defmt::Debug2Format(&error));
            embassy_time::Timer::after_secs(1).await;
        }
    }
}

async fn advertise_and_serve<C: Controller>(
    peripheral: &mut Peripheral<'_, C, DefaultPacketPool>,
    server: &Server<'_>,
    battery: &mut BatteryStatusReceiver,
) -> Result<(), BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(DEVICE_NAME.as_bytes()),
        ],
        &mut advertiser_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &AdvertisementParameters::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    info!("BLE advertising as {}", DEVICE_NAME);
    UI_EVENTS
        .send(AppEvent::BleUpdated(BleState::Advertising))
        .await;

    let connection = advertiser.accept().await?.with_attribute_server(server)?;
    info!("BLE central connected");
    // Connections are not bondable by default; without this, pairing completes
    // without producing a bond, nothing is persisted, and the phone must
    // re-pair after every reboot. trouble-host's own bonding example marks each
    // connection bondable for exactly this reason.
    if let Err(error) = connection.raw().set_bondable(true) {
        warn!(
            "Failed to mark connection bondable: {}",
            defmt::Debug2Format(&error)
        );
    }
    UI_EVENTS
        .send(AppEvent::BleUpdated(BleState::Connected))
        .await;
    let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);

    gatt::serve(server, &connection, battery).await;
    Ok(())
}
