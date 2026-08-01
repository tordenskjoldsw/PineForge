//! The music service's two directions: writes coming in, commands going out.
//!
//! Beside [`super::dfu`] and for the same reason - the GATT loop dispatches,
//! and what a characteristic means is worked out here. It is the only place in
//! this firmware where the watch talks first.

use defmt::warn;
use pineforge_state::MusicState;
use trouble_host::prelude::*;

use crate::{
    ipc::{MUSIC_CONTROL, MUSIC_STATE},
    tasks::ble::gatt::{MusicService, Server},
};

/// The music service's attribute handles, resolved once per connection.
///
/// Held as a struct rather than read from the service at each comparison, so
/// the GATT loop keeps one music arm instead of five. That loop already carries
/// the time, DFU and alert paths and is long enough to be marked so.
pub struct Handles {
    status: u16,
    artist: u16,
    track: u16,
    position: u16,
    total_length: u16,
}

impl Handles {
    pub const fn new(service: &MusicService) -> Self {
        Self {
            status: service.status.handle,
            artist: service.artist.handle,
            track: service.track.handle,
            position: service.position.handle,
            total_length: service.total_length.handle,
        }
    }
}

/// Takes one write into `state`, reporting whether anything the watch shows
/// moved.
///
/// A handle belonging to no music characteristic reports `false` rather than
/// being refused, which is what lets the GATT loop end its chain here without
/// asking first: every earlier arm has already claimed what is theirs, and a
/// write this firmware knows nothing about was accepted at the ATT layer and
/// has nowhere else to go.
pub fn take_write(handles: &Handles, state: &mut MusicState, handle: u16, data: &[u8]) -> bool {
    if handle == handles.track {
        state.set_track(data)
    } else if handle == handles.artist {
        state.set_artist(data)
    } else if handle == handles.status {
        state.set_playing(data)
    } else if handle == handles.position {
        state.set_position(data)
    } else if handle == handles.total_length {
        state.set_length(data)
    } else {
        false
    }
}

/// Publishes the assembled state to whoever is showing it.
///
/// The whole record every time rather than the field that moved, which is what
/// makes the `Watch` correct: a subscriber that misses an update misses
/// nothing, because the next one carries everything.
pub fn publish(state: &MusicState) {
    MUSIC_STATE.sender().send(state.clone());
}

/// Notifies transport commands raised on the watch until the connection ends.
///
/// Never returns on its own - it is selected against the GATT loop, which is
/// what ends it.
pub async fn notify_events(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, DefaultPacketPool>,
) {
    // A command raised while nothing was connected must not fire now. The
    // screen lets one be pressed whether or not a phone is listening, and a
    // "next track" that arrives at the moment of reconnection - possibly
    // minutes later, possibly in a pocket - is worse than one that did nothing.
    // The same reasoning as the DFU flash results: no leftovers from one
    // connection are left for the next to reap.
    while MUSIC_CONTROL.try_receive().is_ok() {}

    loop {
        let control = MUSIC_CONTROL.receive().await;
        let payload = [control.event_byte()];
        // `store` is false: this is a command, not a value. Writing it back
        // into the attribute table would leave the last button pressed
        // readable as though it were the characteristic's state.
        if let Err(error) = server
            .music
            .event
            .notify_raw(connection, &payload, false)
            .await
        {
            warn!(
                "Music command not delivered: {}",
                defmt::Debug2Format(&error)
            );
        }
    }
}
