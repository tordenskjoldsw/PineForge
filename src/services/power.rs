use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Instant, with_deadline};
use pineforge_state::{PowerCommand, PowerConfig, SystemPowerPolicy};

use crate::services::events::{POWER_COMMANDS, SYSTEM_POWER, display_settings_receiver};

enum Input {
    Command(PowerCommand),
    ConfigChanged,
    DeadlineReached,
}

/// Owns global inactivity policy and publishes the latest logical power state.
#[embassy_executor::task]
pub async fn run() {
    let started_at = Instant::now();
    let mut policy = SystemPowerPolicy::new(0, PowerConfig::DEFAULT);
    let state_sender = SYSTEM_POWER.sender();
    let mut settings_receiver = display_settings_receiver();
    state_sender.send(policy.state());

    loop {
        let inputs = select(POWER_COMMANDS.receive(), settings_receiver.changed());
        let input = if let Some(deadline_millis) = policy.next_deadline_millis() {
            let deadline = started_at + Duration::from_millis(deadline_millis);
            match with_deadline(deadline, inputs).await {
                Ok(Either::First(command)) => Input::Command(command),
                Ok(Either::Second(settings)) => {
                    policy.set_config(settings.power_config());
                    Input::ConfigChanged
                }
                Err(_) => Input::DeadlineReached,
            }
        } else {
            match inputs.await {
                Either::First(command) => Input::Command(command),
                Either::Second(settings) => {
                    policy.set_config(settings.power_config());
                    Input::ConfigChanged
                }
            }
        };

        let now_millis = Instant::now().duration_since(started_at).as_millis();
        let next = match input {
            Input::Command(PowerCommand::UserActivity) => policy.on_activity(now_millis),
            // A shortened timeout must take effect against current idle time.
            Input::ConfigChanged | Input::DeadlineReached => policy.advance(now_millis),
        };
        if let Some(state) = next {
            state_sender.send(state);
        }
    }
}
