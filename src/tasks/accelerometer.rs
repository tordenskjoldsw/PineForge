use embassy_nrf::gpio::{Input, Pull};

use crate::{
    board::{buses::SensorI2c, peripherals::AccelerometerResources},
    services::motion::AccelerometerRunner,
};

/// Binds concrete `PineTime` peripherals to the executor-independent runner.
#[embassy_executor::task]
pub async fn run(resources: AccelerometerResources, i2c: SensorI2c) {
    let interrupt = Input::new(resources.interrupt, Pull::Down);
    AccelerometerRunner::new(i2c, interrupt).run().await;
}
