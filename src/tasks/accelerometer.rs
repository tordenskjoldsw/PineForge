use crate::{
    board::buses::{MotionI2c, SensorBus},
    services::motion::AccelerometerRunner,
};

/// Binds concrete `PineTime` peripherals to the executor-independent runner.
#[embassy_executor::task]
pub async fn run(i2c: MotionI2c, bus: &'static SensorBus) {
    AccelerometerRunner::new(i2c, bus).run().await;
}
