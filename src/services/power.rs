use embassy_time::{Duration, Instant, with_deadline};
use pineforge_state::{PowerCommand, PowerConfig, SystemPowerPolicy};

use crate::services::events::{POWER_COMMANDS, SYSTEM_POWER};

/// Owns global inactivity policy and publishes the latest logical power state.
#[embassy_executor::task]
pub async fn run() {
    let started_at = Instant::now();
    let mut policy = SystemPowerPolicy::new(0, PowerConfig::DEFAULT);
    let state_sender = SYSTEM_POWER.sender();
    state_sender.send(policy.state());

    loop {
        let command = if let Some(deadline_millis) = policy.next_deadline_millis() {
            let deadline = started_at + Duration::from_millis(deadline_millis);
            with_deadline(deadline, POWER_COMMANDS.receive()).await.ok()
        } else {
            Some(POWER_COMMANDS.receive().await)
        };

        let now_millis = Instant::now().duration_since(started_at).as_millis();
        let next = match command {
            Some(PowerCommand::UserActivity) => policy.on_activity(now_millis),
            None => policy.advance(now_millis),
        };
        if let Some(state) = next {
            state_sender.send(state);
        }
    }
}
