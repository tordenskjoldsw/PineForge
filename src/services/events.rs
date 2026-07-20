use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

pub static UI_EVENTS: Channel<CriticalSectionRawMutex, UiEvent, 8> = Channel::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEvent {
    Touch { x: i32, y: i32, pressed: bool },
}
