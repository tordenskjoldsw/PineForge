use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

use pineforge_state::AppEvent;

pub static UI_EVENTS: Channel<CriticalSectionRawMutex, AppEvent, 8> = Channel::new();
