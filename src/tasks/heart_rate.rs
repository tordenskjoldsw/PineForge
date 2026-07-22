use embassy_nrf::gpio::{Input, Pull};

use crate::{
    board::{buses::HeartRateI2c, peripherals::HeartRateResources},
    services::heart_rate::HeartRateRunner,
};

/// Binds the `PineTime` resources to the executor-independent heart-rate runner.
/// The interrupt stays owned as an input until interrupt-driven acquisition is
/// implemented.
#[embassy_executor::task]
pub async fn run(resources: HeartRateResources, i2c: HeartRateI2c) {
    let _interrupt = Input::new(resources.interrupt, Pull::None);
    HeartRateRunner::new(i2c).run().await;
}
