use crate::{board::buses::MotionI2c, services::motion::AccelerometerRunner};

/// Binds concrete `PineTime` peripherals to the executor-independent runner.
#[embassy_executor::task]
pub async fn run(i2c: MotionI2c) {
    AccelerometerRunner::new(i2c).run().await;
}
