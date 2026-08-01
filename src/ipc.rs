//! The bus every task reaches the others through.
//!
//! Declarations only - a channel, a watch, or a signal, and the reasoning for
//! each one's depth. No task loop and no hardware live here, which is what
//! keeps this readable as the firmware's wiring diagram: the set of things that
//! cross a task boundary is this file, and nothing else.
//!
//! It sat under `services` while the distinction between a service and a task
//! was being worked out, and was neither. It is not product behavior, and the
//! name `events` covered only the channels - the watches carry state and the
//! signals carry bring-up order.
//!
//! Three kinds, chosen by what a late reader should see:
//!
//! - [`Channel`] where every message matters (a keystroke, a flash page).
//! - [`Watch`] where only the latest does (power state, battery, settings). Its
//!   subscriber count is fixed at compile time and handed out by the
//!   `*_receiver()` helpers below.
//! - [`Signal`] for one-shot ordering, which is how the shared I²C bus is
//!   brought up in the sequence the `PineTime` wants.

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
    watch::{Receiver, Watch},
};

use pineforge_state::{
    AppEvent, BatteryStatus, DfuFailReason, DisplaySettings, HeartRateCommand, MusicControl,
    MusicState, Notification, PowerCommand, SystemPowerState, VibrationPattern, WallClockReference,
};

pub static UI_EVENTS: Channel<CriticalSectionRawMutex, AppEvent, 8> = Channel::new();

/// Phone notifications on their way from the BLE task to the inbox.
///
/// Separate from [`UI_EVENTS`] because of what a notification weighs: two text
/// buffers, well over a hundred bytes, against an `AppEvent` of a dozen. Putting
/// one in the event enum would multiply that by the event channel's capacity for
/// a message that arrives a few times an hour.
///
/// Two deep, which is what it takes for a second alert to survive the redraw the
/// first one triggers - a full repaint outlasts two messages arriving together.
/// The display task is the only receiver, and it owns the inbox, so a
/// notification is stored exactly once and by the task that shows it.
pub static NOTIFICATIONS: Channel<CriticalSectionRawMutex, Notification, 2> = Channel::new();

pub static POWER_COMMANDS: Channel<CriticalSectionRawMutex, PowerCommand, 8> = Channel::new();

/// Enforces `PineTime`'s proven touch -> motion -> heart-rate bus bring-up.
pub static TOUCH_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
pub static MOTION_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
pub static HEART_RATE_COMMANDS: Channel<CriticalSectionRawMutex, HeartRateCommand, 2> =
    Channel::new();

/// Latest logical system state for display, motion, and future services.
pub static SYSTEM_POWER: Watch<CriticalSectionRawMutex, SystemPowerState, 3> = Watch::new();

pub type SystemPowerReceiver = Receiver<'static, CriticalSectionRawMutex, SystemPowerState, 3>;

/// Reserves one of the fixed subscriptions: the display and motion.
///
/// The heart-rate service held the third until background measurement stopped
/// being conditional on the watch being awake. The capacity is left where it is
/// rather than trimmed to what is currently taken - a spare costs nothing, and
/// the next consumer would otherwise have to raise it before it could ask.
pub fn system_power_receiver() -> SystemPowerReceiver {
    SYSTEM_POWER
        .receiver()
        .expect("system power receiver capacity is fixed by architecture")
}

/// Latest validated display settings, published by the settings service.
pub static DISPLAY_SETTINGS: Watch<CriticalSectionRawMutex, DisplaySettings, 2> = Watch::new();

pub type DisplaySettingsReceiver = Receiver<'static, CriticalSectionRawMutex, DisplaySettings, 2>;

/// Reserves one of the fixed display and power subscriptions.
pub fn display_settings_receiver() -> DisplaySettingsReceiver {
    DISPLAY_SETTINGS
        .receiver()
        .expect("display settings receiver capacity is fixed by architecture")
}

/// Validated settings snapshots requested by UI screens.
pub static SETTINGS_COMMANDS: Channel<CriticalSectionRawMutex, DisplaySettings, 2> = Channel::new();

/// Serialized BLE bond keys, sized for one LESC bond record.
pub type StoredBond = heapless::Vec<u8, { pineforge_state::BOND_PAYLOAD_MAX }>;

/// A bond the BLE task asks the storage service to persist.
pub static BOND_STORE: Channel<CriticalSectionRawMutex, StoredBond, 1> = Channel::new();

/// Maximum program payload, one flash page.
pub const DFU_PROGRAM_MAX: usize = 256;

/// A single DFU flash operation on the secondary slot, using image-relative
/// offsets that the storage service rebases and range-checks.
// The inline page cannot be boxed on a heapless target.
#[allow(clippy::large_enum_variant)]
pub enum DfuFlashCommand {
    Erase(u32),
    Program {
        offset: u32,
        data: heapless::Vec<u8, DFU_PROGRAM_MAX>,
    },
}

/// DFU flash requests from the BLE task. Eight page-sized operations provide
/// enough overlap for storage to erase/program in parallel while keeping the
/// inline queue within the nRF52832's tight static RAM budget. The BLE host's
/// separate event queue absorbs the remainder of a packet-receipt window.
pub const DFU_FLASH_QUEUE_SIZE: usize = 8;
pub static DFU_FLASH_COMMANDS: Channel<
    CriticalSectionRawMutex,
    DfuFlashCommand,
    DFU_FLASH_QUEUE_SIZE,
> = Channel::new();

/// Outcome of the most recent DFU flash command: `Ok` on success, or the
/// concrete reason it failed so the watch can show it on-screen.
pub static DFU_FLASH_RESULT: Channel<
    CriticalSectionRawMutex,
    Result<(), DfuFailReason>,
    DFU_FLASH_QUEUE_SIZE,
> = Channel::new();

/// The bond loaded from flash at boot (`None` if absent or invalid), published
/// once by the storage service for the BLE task to install before advertising.
pub static BOND_LOADED: Signal<CriticalSectionRawMutex, Option<StoredBond>> = Signal::new();

/// Haptic requests for the vibration task. Senders use `try_send` so a busy
/// motor drops feedback instead of ever stalling the UI.
pub static VIBRATION_COMMANDS: Channel<CriticalSectionRawMutex, VibrationPattern, 4> =
    Channel::new();

/// Latest battery measurement for the BLE battery service.
pub static BATTERY_STATUS: Watch<CriticalSectionRawMutex, BatteryStatus, 2> = Watch::new();

pub type BatteryStatusReceiver = Receiver<'static, CriticalSectionRawMutex, BatteryStatus, 2>;

/// Reserves one of the fixed BLE and future consumer subscriptions.
pub fn battery_status_receiver() -> BatteryStatusReceiver {
    BATTERY_STATUS
        .receiver()
        .expect("battery status receiver capacity is fixed by architecture")
}

/// Transport commands from the music screen to the BLE task.
///
/// The first thing on this bus that travels from the UI outwards. Everything
/// else here is a subsystem reporting to the display; this is a button on the
/// watch reaching the phone, and the BLE task turns each one into a single
/// notified byte.
///
/// Four deep, which holds a finger tapping "next" faster than the radio can
/// send. Senders use `try_send` so a stalled connection drops the command
/// rather than blocking the display task mid-repaint - and a dropped transport
/// command is a button that did nothing, which is recoverable by pressing it
/// again. The BLE task empties this when a connection opens, so a command
/// raised while nothing was connected cannot fire minutes later.
pub static MUSIC_CONTROL: Channel<CriticalSectionRawMutex, MusicControl, 4> = Channel::new();

/// What the phone last said it is playing.
///
/// A `Watch` rather than a channel, because only the latest matters and none of
/// it may be lost. The phone reports one field per characteristic, so a track
/// change arrives as a burst of five writes; a bounded channel would have to
/// either block the GATT loop or drop one, and a dropped write leaves the wrong
/// artist standing under the right title. The BLE task assembles the fields it
/// receives and publishes the whole record, so every subscriber sees a state
/// that is complete and current.
pub static MUSIC_STATE: Watch<CriticalSectionRawMutex, MusicState, 2> = Watch::new();

pub type MusicStateReceiver = Receiver<'static, CriticalSectionRawMutex, MusicState, 2>;

/// Reserves one of the fixed display and future consumer subscriptions.
pub fn music_state_receiver() -> MusicStateReceiver {
    MUSIC_STATE
        .receiver()
        .expect("music state receiver capacity is fixed by architecture")
}

/// Wall-clock anchor written by the BLE Current Time Service.
pub static WALL_CLOCK: Watch<CriticalSectionRawMutex, WallClockReference, 2> = Watch::new();

pub type WallClockReceiver = Receiver<'static, CriticalSectionRawMutex, WallClockReference, 2>;

/// Reserves one of the fixed display and future consumer subscriptions.
pub fn wall_clock_receiver() -> WallClockReceiver {
    WALL_CLOCK
        .receiver()
        .expect("wall clock receiver capacity is fixed by architecture")
}
