//! What a runner reports through, handed to it rather than reached for.
//!
//! Every endpoint here is capacity-erased. A channel's depth is a decision
//! about the bus, argued where the bus is declared - `src/ipc.rs` in the
//! firmware - and a runner that named the depth in its own type would force
//! that argument to be repeated here and kept in step by hand.
//!
//! `DynamicSender` and friends cost one indirect call per send, against a
//! sensor read that already went over a bus. What they buy is a runner that a
//! test can hand a channel of its own.

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{DynamicReceiver, DynamicSender},
    signal::Signal,
    watch::{DynAnonReceiver, DynReceiver, DynSender},
};
use pineforge_state::{
    AppEvent, DisplaySettings, HeartRateCommand, PowerCommand, SystemPowerState,
};

/// One-shot ordering between the tasks that share the sensor bus.
pub type ReadySignal = Signal<CriticalSectionRawMutex, ()>;

/// What the accelerometer runner reads and reports through.
pub struct MotionPorts<'a> {
    /// Readings on their way to whatever is showing them.
    pub events: DynamicSender<'a, AppEvent>,
    /// Raise-to-wake reaches the power coordinator here, never the panel.
    pub power_commands: DynamicSender<'a, PowerCommand>,
    /// The state the sensor's own power mode and cadence follow.
    pub power: DynReceiver<'a, SystemPowerState>,
    /// Read rather than subscribed to: the runner only ever asks what the wake
    /// gestures currently are, and an anonymous receiver takes none of the
    /// `Watch`'s fixed subscriber slots to do it.
    pub settings: DynAnonReceiver<'a, DisplaySettings>,
    /// The step count as the sensor last reported it, for anything that needs
    /// the number without being the screen showing it - the phone, today.
    pub steps: DynSender<'a, u32>,
    /// Raised when the calendar day changed under the counter, which is the
    /// only thing that puts it back to zero.
    pub reset_steps: &'a ReadySignal,
    /// Waited on before touching the bus, and raised once this runner is done
    /// with it - the `PineTime`'s proven touch, motion, heart-rate bring-up.
    pub touch_ready: &'a ReadySignal,
    pub motion_ready: &'a ReadySignal,
}

/// What the heart-rate runner reads and reports through.
pub struct HeartRatePorts<'a> {
    pub events: DynamicSender<'a, AppEvent>,
    /// Requests to measure now, to stop, and the persisted background policy.
    pub commands: DynamicReceiver<'a, HeartRateCommand>,
    /// Third in the bring-up order, so it waits on the runner before it.
    pub motion_ready: &'a ReadySignal,
}
