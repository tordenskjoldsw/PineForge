//! The Music Service, as `InfiniTime` defines it and Gadgetbridge speaks it.
//!
//! `InfiniTime` carries media control over a custom service,
//! `00000000-78fc-48fe-8e23-433b3a1942d0`, and Gadgetbridge drives it from the
//! phone's media session. The phone writes what is playing into one
//! characteristic per field, and the watch notifies a single byte back to ask
//! for play, pause, a track change or a volume step. This module is that
//! protocol and the product rules around it; the BLE task feeds it raw writes
//! and the screen reads the result.
//!
//! Two halves, because they are held by different tasks:
//!
//! - [`MusicState`] is the record the phone reports. The BLE task owns one,
//!   fills it in as writes arrive, and publishes it.
//! - [`MusicPlayback`] is what a screen holds. It takes a reported state and
//!   the local clock reading it landed at, which is what lets the elapsed time
//!   keep moving between updates.
//!
//! # Why the elapsed time is anchored rather than counted
//!
//! Gadgetbridge writes the position when it changes, not once a second, so a
//! watch that only showed the last reported value would show a clock that
//! stands still. `InfiniTime` solves that by incrementing its own counter on a
//! one-second timer.
//!
//! That does not work here. This watch sleeps twenty seconds after the last
//! touch and the music screen deliberately does not hold it awake - a screen
//! that did would be a battery bug wearing a feature's name. A counter that
//! only advances while the panel is on would therefore fall behind by exactly
//! however long the watch slept, every time.
//!
//! So the position is stored with the uptime it was reported at, and the
//! elapsed time is computed from the difference whenever it is asked for. Sleep
//! costs nothing, a missed tick costs nothing, and the arithmetic is testable
//! without a clock.

use heapless::String;

use crate::notification::sanitize;

/// Longest artist or track name kept, matching `InfiniTime`'s `MaxStringSize`.
///
/// Gadgetbridge truncates to this before it writes, so a longer buffer would
/// hold text that is never sent. It is also about what the panel can show: the
/// UI face fits 22 characters across the content column and the screen gives a
/// title two lines.
pub const MUSIC_TEXT_MAX: usize = 40;

/// Highest minute the elapsed time can display.
///
/// Two places is what the numerals across the panel come to. A podcast longer
/// than this reads as stopped there rather than wrapping back to zero and
/// claiming a position it is not at.
pub const MUSIC_MINUTES_MAX: u32 = 99;

/// What the watch asks the phone to do.
///
/// The byte values are `InfiniTime`'s, because Gadgetbridge decodes them: this
/// firmware advertises as `InfiniTime` and is recognized by its companion as
/// that device, so the event codes are protocol rather than choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicControl {
    Play,
    Pause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    /// Sent when the screen is opened, as `InfiniTime` sends it.
    ///
    /// It is the protocol's own way to ask "what is playing", and **Gadgetbridge
    /// does not answer it**: its music event handler switches over 0, 1, 3, 4,
    /// 5 and 6 and returns on anything else, so this byte is read and dropped.
    /// Checked against its source rather than assumed, because the opposite was
    /// assumed here first and it is the kind of thing that stays wrong quietly.
    ///
    /// Kept anyway. It costs one byte on a screen change, it is what the
    /// protocol says to send, and a companion that does implement it is the
    /// only way this screen ever fills in without the user touching something.
    /// What must not be built on it is the expectation that opening the screen
    /// shows the current track - with Gadgetbridge it does not, and the empty
    /// state has to say something the user can act on instead.
    Open,
}

impl MusicControl {
    /// The single byte notified on the event characteristic.
    #[must_use]
    pub const fn event_byte(self) -> u8 {
        match self {
            Self::Play => 0x00,
            Self::Pause => 0x01,
            Self::Next => 0x03,
            Self::Previous => 0x04,
            Self::VolumeUp => 0x05,
            Self::VolumeDown => 0x06,
            Self::Open => 0xe0,
        }
    }
}

/// What the phone last reported about the track it is playing.
///
/// One field per characteristic the screen uses. The service carries six more
/// (album, track number, track total, playback speed, repeat and shuffle),
/// which the watch accepts and drops: nothing on a 240-pixel panel says more
/// with them than the title, the artist and the position say without.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MusicState {
    pub track: String<MUSIC_TEXT_MAX>,
    pub artist: String<MUSIC_TEXT_MAX>,
    pub playing: bool,
    /// Where the phone last placed the track, in seconds.
    pub position_seconds: u32,
    /// The track's length in seconds, or zero when the phone has not said.
    pub length_seconds: u32,
}

impl MusicState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            track: String::new(),
            artist: String::new(),
            playing: false,
            position_seconds: 0,
            length_seconds: 0,
        }
    }

    /// Whether anything has been reported yet.
    ///
    /// A phone that has never written leaves this empty, and the screen says so
    /// rather than drawing an empty card that looks like a fault.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.track.is_empty() && self.artist.is_empty()
    }

    /// Takes a track-name write. Reports whether the value changed.
    pub fn set_track(&mut self, payload: &[u8]) -> bool {
        set_text(&mut self.track, payload)
    }

    /// Takes an artist write. Reports whether the value changed.
    pub fn set_artist(&mut self, payload: &[u8]) -> bool {
        set_text(&mut self.artist, payload)
    }

    /// Takes a status write: the first byte, non-zero for playing.
    pub const fn set_playing(&mut self, payload: &[u8]) -> bool {
        let Some(&first) = payload.first() else {
            return false;
        };
        let playing = first != 0;
        let moved = self.playing != playing;
        self.playing = playing;
        moved
    }

    /// Takes a position write, in seconds.
    pub fn set_position(&mut self, payload: &[u8]) -> bool {
        let Some(position) = parse_be_u32(payload) else {
            return false;
        };
        let moved = self.position_seconds != position;
        self.position_seconds = position;
        moved
    }

    /// Takes a total-length write, in seconds.
    pub fn set_length(&mut self, payload: &[u8]) -> bool {
        let Some(length) = parse_be_u32(payload) else {
            return false;
        };
        let moved = self.length_seconds != length;
        self.length_seconds = length;
        moved
    }
}

/// Copies a text write into a bounded string, reporting whether it changed.
fn set_text<const N: usize>(field: &mut String<N>, payload: &[u8]) -> bool {
    let next = sanitize::<N>(payload);
    if *field == next {
        return false;
    }
    *field = next;
    true
}

/// Reads the big-endian `u32` the service encodes its numbers as.
///
/// Rejects a short write rather than reading past it. `InfiniTime` indexes the
/// first four bytes unconditionally, which is safe there only because its
/// stack hands it a buffer that is always long enough; nothing about the
/// protocol promises it.
#[must_use]
pub fn parse_be_u32(payload: &[u8]) -> Option<u32> {
    payload.get(..4).map(|bytes| {
        // The slice is exactly four bytes, so the conversion cannot fail.
        u32::from_be_bytes(bytes.try_into().unwrap_or([0; 4]))
    })
}

/// A reported [`MusicState`] together with the local clock, so the elapsed time
/// keeps moving between the phone's updates.
///
/// The uptime handed to [`Self::apply`] and [`Self::elapsed`] has to come from
/// one base. The display task's tick is that base, and it is also the task that
/// owns this - the BLE task never anchors anything, because its own clock reads
/// from a different zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MusicPlayback {
    state: MusicState,
    /// Where the track stands at [`Self::anchored_at`]: the phone's reported
    /// position after a seek, or where the clock was frozen by a pause.
    position_seconds: u32,
    /// The position as the phone last reported it, kept only to tell a genuine
    /// seek from a write that repeated a position we had already moved past.
    reported_seconds: u32,
    anchored_at: u64,
}

impl MusicPlayback {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: MusicState::new(),
            position_seconds: 0,
            reported_seconds: 0,
            anchored_at: 0,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &MusicState {
        &self.state
    }

    #[must_use]
    pub fn track(&self) -> &str {
        self.state.track.as_str()
    }

    #[must_use]
    pub fn artist(&self) -> &str {
        self.state.artist.as_str()
    }

    #[must_use]
    pub const fn playing(&self) -> bool {
        self.state.playing
    }

    #[must_use]
    pub const fn length_seconds(&self) -> u32 {
        self.state.length_seconds
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }

    /// Takes a state the phone reported at `now`, reporting whether anything a
    /// screen draws moved with it.
    ///
    /// The elapsed clock is re-anchored in exactly two cases, and neither of
    /// them is "an update arrived":
    ///
    /// - **the phone seeked**, so its position is the truth and ours is stale
    /// - **play or pause flipped**, so the clock freezes where it actually
    ///   stood rather than where it was last reported, and resumes from there
    ///
    /// Anything else - a track name arriving a moment after the position, a
    /// repeated write - leaves the anchor alone. Re-anchoring on every update
    /// would drag the elapsed time backwards to the last reported position
    /// every time the phone mentioned anything.
    pub fn apply(&mut self, state: &MusicState, now: u64) -> bool {
        let elapsed_now = self.elapsed(now);
        let seeked = state.position_seconds != self.reported_seconds;
        let flipped = state.playing != self.state.playing;

        if seeked {
            self.position_seconds = state.position_seconds;
            self.anchored_at = now;
        } else if flipped {
            self.position_seconds = elapsed_now;
            self.anchored_at = now;
        }
        self.reported_seconds = state.position_seconds;

        let moved = self.state != *state;
        self.state = state.clone();
        // A seek moves the number without changing the record it came in, so
        // the repaint it earned has to be reported on its own.
        moved || seeked || flipped
    }

    /// How far into the track it is at `now`, in seconds.
    ///
    /// Clamped to the track's length, so a phone that stops writing at the end
    /// of a track leaves the display standing at the end rather than counting
    /// past it. A length of zero means the phone has not said, and then there
    /// is nothing to clamp against.
    #[must_use]
    pub fn elapsed(&self, now: u64) -> u32 {
        let mut elapsed = self.position_seconds;
        if self.state.playing {
            let advanced = now.saturating_sub(self.anchored_at);
            elapsed = elapsed.saturating_add(u32::try_from(advanced).unwrap_or(u32::MAX));
        }
        if self.state.length_seconds > 0 {
            elapsed = elapsed.min(self.state.length_seconds);
        }
        elapsed.min(MUSIC_MINUTES_MAX * 60 + 59)
    }

    /// The elapsed time as minutes and seconds, for a display with two places
    /// for each.
    #[must_use]
    pub fn elapsed_minutes_seconds(&self, now: u64) -> (u32, u32) {
        let elapsed = self.elapsed(now);
        (elapsed / 60, elapsed % 60)
    }

    /// How far through the track it is, as a fraction of `width`.
    ///
    /// Multiplied before dividing so a position under a hundredth of the track
    /// still moves the bar, and zero while the length is unknown - a bar that
    /// filled itself against a length nobody reported would be inventing
    /// progress.
    #[must_use]
    pub fn filled(&self, now: u64, width: u32) -> u32 {
        if self.state.length_seconds == 0 {
            return 0;
        }
        let filled =
            u64::from(self.elapsed(now)) * u64::from(width) / u64::from(self.state.length_seconds);
        u32::try_from(filled).unwrap_or(width).min(width)
    }

    /// What the transport asks for next, given what is playing now.
    #[must_use]
    pub const fn toggle(&self) -> MusicControl {
        if self.state.playing {
            MusicControl::Pause
        } else {
            MusicControl::Play
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The event bytes are Gadgetbridge's to decode, so they are pinned rather
    /// than described. Getting one wrong is a button that does the wrong thing
    /// on the phone, which no test on this side would otherwise catch.
    #[test]
    fn the_event_bytes_are_the_ones_infinitime_defines() {
        assert_eq!(MusicControl::Play.event_byte(), 0x00);
        assert_eq!(MusicControl::Pause.event_byte(), 0x01);
        assert_eq!(MusicControl::Next.event_byte(), 0x03);
        assert_eq!(MusicControl::Previous.event_byte(), 0x04);
        assert_eq!(MusicControl::VolumeUp.event_byte(), 0x05);
        assert_eq!(MusicControl::VolumeDown.event_byte(), 0x06);
        assert_eq!(MusicControl::Open.event_byte(), 0xe0);
    }

    #[test]
    fn a_number_is_read_big_endian_and_a_short_write_is_refused() {
        assert_eq!(parse_be_u32(&[0x00, 0x00, 0x01, 0x2c]), Some(300));
        assert_eq!(parse_be_u32(&[0xff, 0xff, 0xff, 0xff]), Some(u32::MAX));
        // Trailing bytes are the phone's business, not ours.
        assert_eq!(parse_be_u32(&[0x00, 0x00, 0x00, 0x01, 0x99]), Some(1));
        assert_eq!(parse_be_u32(&[0x00, 0x00, 0x01]), None);
        assert_eq!(parse_be_u32(&[]), None);
    }

    #[test]
    fn a_write_reports_whether_it_changed_anything() {
        let mut state = MusicState::new();
        assert!(state.is_empty());

        assert!(state.set_track(b"Dance Yrself Clean"));
        assert!(!state.set_track(b"Dance Yrself Clean"));
        assert_eq!(state.track.as_str(), "Dance Yrself Clean");
        assert!(!state.is_empty());

        assert!(state.set_playing(&[1]));
        assert!(!state.set_playing(&[1]));
        assert!(state.set_playing(&[0]));
        // A status write with no payload says nothing and changes nothing.
        assert!(!state.set_playing(&[]));
        assert!(!state.playing);

        assert!(state.set_length(&[0x00, 0x00, 0x02, 0x1c]));
        assert_eq!(state.length_seconds, 540);
        // A malformed write leaves the last good value standing.
        assert!(!state.set_length(&[0x02]));
        assert_eq!(state.length_seconds, 540);
    }

    #[test]
    fn an_overlong_or_invalid_name_is_truncated_rather_than_refused() {
        let mut state = MusicState::new();
        let long = [b'x'; 200];
        assert!(state.set_track(&long));
        assert_eq!(state.track.len(), MUSIC_TEXT_MAX);

        // Valid text then a lone continuation byte that starts no code point.
        assert!(state.set_artist(b"LCD\xff\xfe"));
        assert_eq!(state.artist.as_str(), "LCD");
    }

    /// A track playing from the start, reported once.
    fn playing_track(length: u32) -> (MusicPlayback, MusicState) {
        let mut state = MusicState::new();
        let _ = state.set_track(b"All My Friends");
        let _ = state.set_artist(b"LCD Soundsystem");
        let _ = state.set_playing(&[1]);
        let _ = state.set_length(&length.to_be_bytes());
        let mut playback = MusicPlayback::new();
        assert!(playback.apply(&state, 1_000));
        (playback, state)
    }

    /// The whole reason the position is anchored: the phone reports it once and
    /// the watch has to keep it moving, including across a sleep it did not
    /// count seconds through.
    #[test]
    fn the_elapsed_time_advances_without_another_update() {
        let (playback, _) = playing_track(420);
        assert_eq!(playback.elapsed(1_000), 0);
        assert_eq!(playback.elapsed(1_030), 30);
        // Twenty seconds awake, five minutes asleep, and no ticks in between.
        assert_eq!(playback.elapsed(1_320), 320);
        assert_eq!(playback.elapsed_minutes_seconds(1_320), (5, 20));
    }

    /// A paused track stands still, and it stands still where it actually got
    /// to rather than at the position the phone last mentioned.
    #[test]
    fn pausing_freezes_the_clock_where_it_stood() {
        let (mut playback, mut state) = playing_track(420);
        assert_eq!(playback.elapsed(1_100), 100);

        let _ = state.set_playing(&[0]);
        assert!(playback.apply(&state, 1_100));
        assert_eq!(playback.elapsed(1_100), 100);
        assert_eq!(playback.elapsed(1_400), 100, "a paused track kept counting");

        // Resuming picks up from the freeze, not from the phone's stale zero.
        let _ = state.set_playing(&[1]);
        assert!(playback.apply(&state, 1_400));
        assert_eq!(playback.elapsed(1_400), 100);
        assert_eq!(playback.elapsed(1_430), 130);
    }

    /// A seek is the phone overruling us, and it has to win.
    #[test]
    fn a_seek_moves_the_clock_and_asks_for_a_repaint() {
        let (mut playback, mut state) = playing_track(420);
        assert_eq!(playback.elapsed(1_060), 60);

        let _ = state.set_position(&300_u32.to_be_bytes());
        assert!(playback.apply(&state, 1_060), "a seek drew nothing");
        assert_eq!(playback.elapsed(1_060), 300);
        assert_eq!(playback.elapsed(1_090), 330);
    }

    /// An update that carries no new position must not drag the clock back to
    /// the last one the phone mentioned. This is the failure the reported
    /// position is kept for.
    #[test]
    fn an_unrelated_update_leaves_the_clock_where_it_is() {
        let (mut playback, mut state) = playing_track(420);
        assert_eq!(playback.elapsed(1_060), 60);

        let _ = state.set_artist(b"LCD Soundsystem (Live)");
        assert!(playback.apply(&state, 1_060));
        assert_eq!(
            playback.elapsed(1_060),
            60,
            "a name arriving reset the elapsed time"
        );
    }

    /// The track ends where the phone said it ends. A player that stops writing
    /// at the last second must not leave the watch counting into a track that
    /// is over.
    #[test]
    fn the_clock_stops_at_the_end_of_the_track() {
        let (playback, _) = playing_track(180);
        assert_eq!(playback.elapsed(1_180), 180);
        assert_eq!(playback.elapsed(9_999), 180);
    }

    /// Two places for the minutes, and a podcast longer than they hold stands
    /// at the last time they can show rather than wrapping to a smaller one.
    #[test]
    fn an_overlong_track_stands_at_the_last_time_it_can_show() {
        let (playback, _) = playing_track(0);
        assert_eq!(playback.elapsed_minutes_seconds(1_000 + 5_999), (99, 59));
        assert_eq!(playback.elapsed_minutes_seconds(1_000 + 9_999), (99, 59));
    }

    #[test]
    fn the_bar_runs_from_empty_to_full_and_invents_nothing() {
        let (playback, _) = playing_track(400);
        assert_eq!(playback.filled(1_000, 200), 0);
        assert_eq!(playback.filled(1_200, 200), 100);
        assert_eq!(playback.filled(1_400, 200), 200);
        assert_eq!(
            playback.filled(9_999, 200),
            200,
            "the bar ran past its own end"
        );

        // No length reported is no progress to show, whatever the position.
        let (unknown, _) = playing_track(0);
        assert_eq!(unknown.filled(1_300, 200), 0);
    }

    #[test]
    fn the_transport_asks_for_the_opposite_of_what_is_happening() {
        let (mut playback, mut state) = playing_track(420);
        assert_eq!(playback.toggle(), MusicControl::Pause);

        let _ = state.set_playing(&[0]);
        let _ = playback.apply(&state, 1_100);
        assert_eq!(playback.toggle(), MusicControl::Play);
    }
}
