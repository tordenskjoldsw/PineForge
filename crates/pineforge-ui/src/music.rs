//! What the phone is playing, and the three controls that change it.
//!
//! The watch holds no audio and decodes nothing. It shows what Gadgetbridge
//! reports over the music service and sends one byte back when a control is
//! pressed, which is the whole of what a wrist can usefully do about music
//! playing in a pocket.
//!
//! # The look
//!
//! `InfiniTime`'s music screen sets everything in type: a scrolling title, a
//! scrolling artist, and a row of glyphs from its icon font. This one follows
//! the FORGE face instead, and the split it draws is the rule the pulse and
//! steps apps already settled - **type names things, segments carry numbers.**
//!
//! So the elapsed time is built from rectangles, in the numerals the watchface
//! and the steps app use, and the title and artist are the only text on the
//! screen. Naming a track is not something segments can do, and setting a
//! second *value* in an atlas beside a value made of rectangles is the mixture
//! the face was designed to avoid. One number, one rendering technique for it.
//!
//! The transport glyphs are rectangles too, stepped the way the FORGE face
//! steps its charging bolt. A play triangle from an icon set would be the
//! eighth bitmap in a collection this screen otherwise has no use for, and the
//! shape only has to read at a glance.
//!
//! # The bar has no cells
//!
//! The steps gauge is divided into five, because each cell is two thousand
//! steps and a glance reads thousands. A track has no such unit - four minutes
//! divided into fifths is four arbitrary marks - so this is the plain shared
//! bar, and the division would have been decoration wearing the clothes of
//! information.
//!
//! # Gestures and controls
//!
//! Opened from a launcher tile, so it is left by swiping right, the way every
//! tile-opened screen is. That leaves up and down free, and `InfiniTime` spends
//! them on volume, which is worth borrowing: volume is the one control that is
//! wanted repeatedly and is worth nothing as a target to hit.
//!
//! The three buttons are disabled when no phone is connected. Everything this
//! screen does happens on the other end of a link, so with the link down a
//! control that lights up under a finger and does nothing is the one thing on
//! it that would be actively false.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::{Alignment, Text},
};
use pineforge_state::{
    AppEvent, BleState, Button, ButtonBounds, ButtonOutcome, ButtonState, MusicControl,
    MusicPlayback, MusicState, ScreenAction, SwipeDirection,
};

use crate::canvas::{Canvas, CanvasError};
use crate::font::{body_text, hint_text, ui_text};
use crate::{
    render::{
        PANEL, ROW_HEIGHT, ROW_WIDTH, ROW_X, draw_progress_bar, draw_visible, fill, round_corners,
    },
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell},
    theme,
};

const PANEL_WIDTH: i32 = PANEL.size.width.cast_signed();
const PANEL_HEIGHT: i32 = PANEL.size.height.cast_signed();

/// Baselines of the two lines that say what is playing.
///
/// The title is set in the UI face and the artist in the reading face, and the
/// hierarchy is carried by size rather than by a dimmer ink - `FRAME` is held
/// at the 3:1 a non-text element needs and is under-contrast as words, which is
/// the same reason the pulse app gives.
///
/// The artist is not set in the hint face, and that is worth stating because it
/// looks like the obvious choice for a second line. The hint face is 6 pixels
/// wide, and a lowercase `m` at that width has three stems in five usable
/// columns: it renders as a solid block. Every other use of that face in this
/// firmware is an uppercase label, so nothing had shown it before. An artist
/// name is content, and the reading face is the size that exists for content.
const TITLE_BASELINE_Y: i32 = 42;
const ARTIST_BASELINE_Y: i32 = 64;
/// The band the two lines occupy, cleared before either is redrawn. They are
/// centred, so a shorter line would otherwise leave the tail of a longer one
/// standing beside it.
const TEXT_BAND: Rectangle = Rectangle::new(Point::new(0, 22), Size::new(240, 48));

/// The elapsed time, in the steps app's numerals.
const DIGIT: SegmentSize = SegmentSize::new(40, 54, 7);
const DIGIT_GAP: i32 = 5;
/// The colon between minutes and seconds, drawn as two squares of the numerals'
/// own stroke weight - the same move the FORGE face makes with its date
/// separator, so the mark reads as part of the numerals rather than as
/// punctuation borrowed from a typeface.
const COLON_WIDTH: i32 = 8;
const CLOCK_WIDTH: i32 = 4 * DIGIT.width + 4 * DIGIT_GAP + COLON_WIDTH;
const CLOCK_X: i32 = (PANEL_WIDTH - CLOCK_WIDTH) / 2;
const CLOCK_Y: i32 = 78;
/// Left edges of the four numerals and of the colon between them.
const MINUTES_X: [i32; 2] = [CLOCK_X, CLOCK_X + DIGIT.width + DIGIT_GAP];
const COLON_X: i32 = CLOCK_X + 2 * (DIGIT.width + DIGIT_GAP);
const SECONDS_X: [i32; 2] = [
    COLON_X + COLON_WIDTH + DIGIT_GAP,
    COLON_X + COLON_WIDTH + 2 * DIGIT_GAP + DIGIT.width,
];

/// The bar, in the column a menu row and a notification card occupy.
const BAR: Rectangle = Rectangle::new(Point::new(ROW_X, 148), Size::new(200, 16));
const _: () = assert!(
    BAR.size.width.cast_signed() == ROW_WIDTH,
    "the bar left the column every other component keeps"
);

/// The transport: three controls across the same column, on the row rhythm.
const TRANSPORT_Y: i32 = 176;
const TRANSPORT_GAP: i32 = 10;
const TRANSPORT_WIDTH: i32 = (ROW_WIDTH - 2 * TRANSPORT_GAP) / 3;

/// Baseline of the line that says what the free gestures do.
///
/// Volume has no on-screen control, so it needs saying somewhere. A watch is
/// not a device anybody reads a manual for, and an unlabelled gesture is one
/// nobody finds.
const HINT_BASELINE_Y: i32 = 232;

/// Characters of each line that fit across the panel, from the faces
/// themselves so a later change of face moves the cut with them.
const TITLE_COLUMNS: usize =
    (PANEL.size.width / crate::font::JETBRAINS_MONO_10X22.cell.width - 2) as usize;
const ARTIST_COLUMNS: usize =
    (PANEL.size.width / crate::font::JETBRAINS_MONO_8X18.cell.width - 2) as usize;

/// Which control a slot holds. Ordered as they are drawn, left to right.
const SLOTS: usize = 3;

/// What still owes a repaint.
///
/// Ordered by how much it costs, because the mark is only ever raised: a
/// screen collects changes while something else is on the panel, and a tick
/// arriving after a track change must not talk the repaint down from the whole
/// screen to the clock. Only [`MusicScreen::mark_painted`] lowers it, and it is
/// called when the panel has actually been painted.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pending {
    Nothing,
    /// A second passed: the numerals and the bar moved, and nothing else did.
    Clock,
    Everything,
}

/// What the phone is playing, and the transport that changes it.
pub struct MusicScreen {
    playback: MusicPlayback,
    /// Whether a phone is reachable. Everything here happens at the other end
    /// of the link, so this decides whether the controls do anything.
    connected: bool,
    controls: [Button; SLOTS],
    /// The uptime the screen last saw, which is what the elapsed time is drawn
    /// against. Drawing takes `&self` and is repeated once per stripe, so it
    /// cannot ask a clock; the tick brings the time in instead.
    now: u64,
    pending: Pending,
}

impl Default for MusicScreen {
    fn default() -> Self {
        let mut screen = Self {
            playback: MusicPlayback::new(),
            connected: false,
            controls: [
                Button::new(control_bounds(0)),
                Button::new(control_bounds(1)),
                Button::new(control_bounds(2)),
            ],
            now: 0,
            pending: Pending::Nothing,
        };
        // Nothing is connected until something says so, and the controls have
        // to agree with that from the start. A button that is enabled until the
        // first Bluetooth event arrives is enabled for exactly the window in
        // which the watch has just booted and nobody has paired yet.
        for control in &mut screen.controls {
            let _ = control.set_enabled(false);
        }
        screen
    }
}

/// Where a transport control sits, left to right.
const fn control_bounds(slot: usize) -> ButtonBounds {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let index = slot as i32;
    ButtonBounds::new(
        ROW_X + index * (TRANSPORT_WIDTH + TRANSPORT_GAP),
        TRANSPORT_Y,
        TRANSPORT_WIDTH,
        ROW_HEIGHT,
    )
}

/// The first `columns` characters of `text`, cut on a character boundary.
fn clipped(text: &str, columns: usize) -> &str {
    match text.char_indices().nth(columns) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

impl MusicScreen {
    /// Whether the last event left anything to repaint.
    #[must_use]
    pub const fn moved(&self) -> bool {
        !matches!(self.pending, Pending::Nothing)
    }

    /// Says the panel now shows what this screen holds.
    pub const fn mark_painted(&mut self) {
        self.pending = Pending::Nothing;
    }

    /// Raises what is owed, never lowers it.
    fn mark(&mut self, pending: Pending) {
        self.pending = self.pending.max(pending);
    }

    /// Takes a state the phone reported, at the uptime it was received.
    ///
    /// Fed from the event stream whether or not this screen is showing, the way
    /// the pulse and steps apps take their readings: a track that changed while
    /// the watchface was up must not leave a stale title here.
    pub fn apply(&mut self, state: &MusicState, now: u64) -> bool {
        self.now = now;
        if self.playback.apply(state, now) {
            self.mark(Pending::Everything);
        }
        self.moved()
    }

    /// Takes whether a phone is reachable, which is what enables the controls.
    pub fn set_connected(&mut self, state: BleState) -> bool {
        let connected = matches!(
            state,
            BleState::Connected | BleState::Pairing(_) | BleState::DfuProgress(_)
        );
        if self.connected == connected {
            return false;
        }
        self.connected = connected;
        for control in &mut self.controls {
            let _ = control.set_enabled(connected);
        }
        self.mark(Pending::Everything);
        true
    }

    /// The two lines at the head: what is playing, or why nothing is.
    ///
    /// A blank card would read as a fault. Both empty states say which of the
    /// two situations it is, because they need different things done about
    /// them - one wants the phone brought closer, the other wants a control
    /// pressed.
    ///
    /// The second line is not "start a track", though that was the obvious
    /// wording. Gadgetbridge writes what is playing when the phone's media
    /// session *changes*, and answers no request to send it, so this screen is
    /// blank exactly as often when music is already playing as when none is.
    /// Telling someone to start a track they can already hear is the empty
    /// state failing at the one job it has. Pressing any control changes the
    /// session, which is what makes the metadata arrive - so that is what it
    /// says.
    fn heading(&self) -> (&str, &str) {
        if !self.connected {
            ("NO PHONE", "CONNECT TO CONTROL MUSIC")
        } else if self.playback.is_empty() {
            ("NOTHING PLAYING", "PRESS PLAY OR SKIP TO SYNC")
        } else {
            (self.playback.track(), self.playback.artist())
        }
    }

    /// The elapsed time's colour, which is what says whether it is running.
    const fn clock_ink(&self) -> Rgb565 {
        if self.playback.playing() {
            theme::ACCENT
        } else {
            theme::TEXT
        }
    }

    /// The elapsed time and the bar under it: everything a passing second
    /// moves, and nothing it does not.
    fn draw_clock(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let (minutes, seconds) = self.playback.elapsed_minutes_seconds(self.now);
        let ink = self.clock_ink();

        // Minutes keep an unlit leading place rather than a zero, which is what
        // an instrument does with a place it is not using. Seconds are padded,
        // because 2:04 is a time and 2:4 is not.
        let places = [
            (MINUTES_X[0], minute_cell(minutes / 10)),
            (MINUTES_X[1], Cell::Digit(digit(minutes % 10))),
            (SECONDS_X[0], Cell::Digit(digit(seconds / 10))),
            (SECONDS_X[1], Cell::Digit(digit(seconds % 10))),
        ];
        for (x, cell) in places {
            draw_cell(canvas, DIGIT, x, CLOCK_Y, cell, ink)?;
            keep_alive();
        }
        Self::draw_colon(canvas, ink)?;

        draw_progress_bar(
            canvas,
            &BAR,
            self.playback.filled(self.now, BAR.size.width),
            ink,
        )?;
        keep_alive();
        Ok(())
    }

    /// Two squares between the pairs, in the numerals' own stroke weight.
    fn draw_colon(canvas: &mut Canvas<'_>, ink: Rgb565) -> Result<(), CanvasError> {
        let stroke = DIGIT.stroke;
        let x = COLON_X + (COLON_WIDTH - stroke) / 2;
        // Painted opaque, background and all, so the partial repaint that
        // follows a second does not have to know what stood here before.
        fill(
            canvas,
            COLON_X,
            CLOCK_Y,
            COLON_WIDTH,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        for third in 1..=2 {
            let y = CLOCK_Y + DIGIT.height * third / 3 - stroke / 2;
            fill(canvas, x, y, stroke, stroke, ink)?;
        }
        Ok(())
    }

    /// One transport control: its face, its curve, and its glyph.
    fn draw_control(&self, slot: usize, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let bounds = control_bounds(slot);
        let area = Rectangle::new(
            Point::new(bounds.x(), bounds.y()),
            Size::new(
                bounds.width().unsigned_abs(),
                bounds.height().unsigned_abs(),
            ),
        );
        let state = self.controls[slot].state();
        // Pressed inverts rather than merely recolouring, because inversion is
        // the one treatment that survives the lowest backlight level.
        let (face, ink) = match state {
            ButtonState::Pressed => (theme::ACCENT, theme::BACKGROUND),
            ButtonState::Disabled => (theme::SURFACE, theme::MUTED),
            ButtonState::Idle => (theme::SURFACE, theme::ACCENT),
        };
        fill(
            canvas,
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height(),
            face,
        )?;
        round_corners(&area, canvas)?;

        let centre = Point::new(
            bounds.x() + bounds.width() / 2,
            bounds.y() + bounds.height() / 2,
        );
        match slot {
            0 => draw_skip(canvas, centre, false, ink),
            1 if self.playback.playing() => draw_pause(canvas, centre, ink),
            1 => draw_triangle(canvas, centre.x - GLYPH_WIDTH / 2, centre.y, true, ink),
            _ => draw_skip(canvas, centre, true, ink),
        }
    }

    /// The heading, cleared first because both lines are centred.
    fn draw_heading(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let (title, artist) = self.heading();
        fill(
            canvas,
            TEXT_BAND.top_left.x,
            TEXT_BAND.top_left.y,
            TEXT_BAND.size.width.cast_signed(),
            TEXT_BAND.size.height.cast_signed(),
            theme::BACKGROUND,
        )?;
        draw_visible(
            &Text::with_alignment(
                clipped(title, TITLE_COLUMNS),
                Point::new(PANEL_WIDTH / 2, TITLE_BASELINE_Y),
                ui_text(theme::TEXT, theme::BACKGROUND),
                Alignment::Center,
            ),
            canvas,
        )?;
        draw_visible(
            &Text::with_alignment(
                clipped(artist, ARTIST_COLUMNS),
                Point::new(PANEL_WIDTH / 2, ARTIST_BASELINE_Y),
                body_text(theme::TEXT, theme::BACKGROUND),
                Alignment::Center,
            ),
            canvas,
        )
    }

    fn paint(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        // The whole panel, not the strip below the status corner. The corner
        // paints its own background but only over itself, on the right; the
        // rest of that top row is this screen's to fill, and starting below it
        // would leave a band of whatever the previous screen had there.
        fill(canvas, 0, 0, PANEL_WIDTH, CLOCK_Y, theme::BACKGROUND)?;
        keep_alive();
        self.draw_heading(canvas)?;

        // Beside the numerals and in the gaps between them, which no cell
        // covers. The cells paint their own ground, so the panel is never
        // blanked whole - the same reason the watchfaces stopped doing it.
        fill(canvas, 0, CLOCK_Y, CLOCK_X, DIGIT.height, theme::BACKGROUND)?;
        let clock_right = CLOCK_X + CLOCK_WIDTH;
        fill(
            canvas,
            clock_right,
            CLOCK_Y,
            PANEL_WIDTH - clock_right,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        for x in [MINUTES_X[1], COLON_X, SECONDS_X[0], SECONDS_X[1]] {
            fill(
                canvas,
                x - DIGIT_GAP,
                CLOCK_Y,
                DIGIT_GAP,
                DIGIT.height,
                theme::BACKGROUND,
            )?;
        }
        self.draw_clock(canvas, keep_alive)?;

        // Between the numerals and the bar, beside it, and under it down to the
        // transport.
        let clock_bottom = CLOCK_Y + DIGIT.height;
        fill(
            canvas,
            0,
            clock_bottom,
            PANEL_WIDTH,
            BAR.top_left.y - clock_bottom,
            theme::BACKGROUND,
        )?;
        fill(canvas, 0, BAR.top_left.y, ROW_X, 16, theme::BACKGROUND)?;
        fill(
            canvas,
            ROW_X + ROW_WIDTH,
            BAR.top_left.y,
            PANEL_WIDTH - ROW_X - ROW_WIDTH,
            16,
            theme::BACKGROUND,
        )?;
        let bar_bottom = BAR.top_left.y + 16;
        fill(
            canvas,
            0,
            bar_bottom,
            PANEL_WIDTH,
            PANEL_HEIGHT - bar_bottom,
            theme::BACKGROUND,
        )?;

        for slot in 0..SLOTS {
            self.draw_control(slot, canvas)?;
            keep_alive();
        }
        draw_visible(
            &Text::with_alignment(
                "^ LOUDER   v SOFTER",
                Point::new(PANEL_WIDTH / 2, HINT_BASELINE_Y),
                hint_text(
                    if self.connected {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    },
                    theme::BACKGROUND,
                ),
                Alignment::Center,
            ),
            canvas,
        )
    }
}

/// A minute's tens place: unlit below ten rather than a leading zero.
const fn minute_cell(tens: u32) -> Cell {
    if tens == 0 {
        Cell::Blank
    } else {
        Cell::Digit(digit(tens))
    }
}

/// A single decimal place as the numerals want it. Values above nine cannot
/// occur here - both callers divide a bounded number - and are clamped rather
/// than wrapped so a bug shows as a nine instead of as a different digit.
#[allow(clippy::cast_possible_truncation)]
const fn digit(value: u32) -> u8 {
    if value > 9 { 9 } else { value as u8 }
}

/// Width and height of a transport glyph.
const GLYPH_WIDTH: i32 = 14;
const GLYPH_HEIGHT: i32 = 18;
/// The upright bar a skip glyph carries, and its gap from the triangle.
const SKIP_BAR: i32 = 3;
const SKIP_GAP: i32 = 2;

/// A triangle, stepped one row at a time.
///
/// Rectangles rather than a rasteriser or a bitmap, which is what the rest of
/// this firmware's shapes are: the FORGE face steps its charging bolt the same
/// way, and a row of a triangle is a fill whose width is arithmetic. Each row
/// is one `fill`, so a stripe can compute it without knowing anything outside
/// itself.
fn draw_triangle(
    canvas: &mut Canvas<'_>,
    x: i32,
    centre_y: i32,
    pointing_right: bool,
    ink: Rgb565,
) -> Result<(), CanvasError> {
    let top = centre_y - GLYPH_HEIGHT / 2;
    for row in 0..GLYPH_HEIGHT {
        // Full width at the middle row, tapering to nothing at both ends.
        let from_centre = (row - GLYPH_HEIGHT / 2).abs();
        let width = GLYPH_WIDTH - GLYPH_WIDTH * from_centre * 2 / GLYPH_HEIGHT;
        if width <= 0 {
            continue;
        }
        let left = if pointing_right {
            x
        } else {
            x + GLYPH_WIDTH - width
        };
        fill(canvas, left, top + row, width, 1, ink)?;
    }
    Ok(())
}

/// Two uprights, which is the one transport glyph that is already rectangles.
fn draw_pause(canvas: &mut Canvas<'_>, centre: Point, ink: Rgb565) -> Result<(), CanvasError> {
    const BAR_WIDTH: i32 = 5;
    const BAR_GAP: i32 = 4;
    let top = centre.y - GLYPH_HEIGHT / 2;
    let left = centre.x - i32::midpoint(2 * BAR_WIDTH, BAR_GAP);
    fill(canvas, left, top, BAR_WIDTH, GLYPH_HEIGHT, ink)?;
    fill(
        canvas,
        left + BAR_WIDTH + BAR_GAP,
        top,
        BAR_WIDTH,
        GLYPH_HEIGHT,
        ink,
    )
}

/// A triangle against an upright: next when it points right, previous when it
/// points left and the bar changes sides with it.
fn draw_skip(
    canvas: &mut Canvas<'_>,
    centre: Point,
    forward: bool,
    ink: Rgb565,
) -> Result<(), CanvasError> {
    let total = GLYPH_WIDTH + SKIP_GAP + SKIP_BAR;
    let left = centre.x - total / 2;
    let top = centre.y - GLYPH_HEIGHT / 2;
    let (triangle_x, bar_x) = if forward {
        (left, left + GLYPH_WIDTH + SKIP_GAP)
    } else {
        (left + SKIP_BAR + SKIP_GAP, left)
    };
    draw_triangle(canvas, triangle_x, centre.y, forward, ink)?;
    fill(canvas, bar_x, top, SKIP_BAR, GLYPH_HEIGHT, ink)
}

impl Paint for MusicScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas, keep_alive)
    }
}

impl Screen for MusicScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        // The mark is not cleared here, the way the pulse screen clears its
        // own. This screen takes what the phone reports through `apply` rather
        // than through an event, so an unrelated event arriving between that
        // and the repaint would otherwise retire a repaint it knows nothing
        // about. `mark_painted` is what clears it, and the panel is what calls
        // that - the same arrangement the steps app and the inbox use.
        match event {
            // A second passing moves the numerals and the bar and nothing else.
            // This screen does not hold the watch awake, so the ticks stop when
            // it sleeps - which costs nothing, because the elapsed time is
            // computed from the clock rather than counted up by these.
            AppEvent::Tick { uptime_seconds, .. } => {
                let before = self.playback.elapsed(self.now);
                self.now = uptime_seconds;
                if self.playback.elapsed(uptime_seconds) != before {
                    self.mark(Pending::Clock);
                }
                return ScreenAction::None;
            }
            AppEvent::BleUpdated(state) => {
                let _ = self.set_connected(state);
                return ScreenAction::None;
            }
            // Volume, which `InfiniTime` puts on the same two gestures. Right
            // is spent leaving, and there is nothing left of the screen to page
            // through, so these are free.
            AppEvent::Swipe(SwipeDirection::Up) => {
                return self.command(MusicControl::VolumeUp);
            }
            AppEvent::Swipe(SwipeDirection::Down) => {
                return self.command(MusicControl::VolumeDown);
            }
            _ => {}
        }

        for slot in 0..SLOTS {
            match self.controls[slot].handle_event(event) {
                ButtonOutcome::Activated => {
                    // The face changes with the state the phone reports back,
                    // not with the press: a play button that became a pause
                    // button before anything started playing would be showing
                    // an outcome it does not know it got.
                    self.mark(Pending::Everything);
                    return self.command(match slot {
                        0 => MusicControl::Previous,
                        1 => self.playback.toggle(),
                        _ => MusicControl::Next,
                    });
                }
                ButtonOutcome::Redraw => {
                    self.mark(Pending::Everything);
                    return ScreenAction::None;
                }
                ButtonOutcome::None => {}
            }
        }
        ScreenAction::None
    }

    /// Repaints what a second moves, or the screen when more did.
    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        match self.pending {
            Pending::Nothing => Ok(()),
            Pending::Clock => self.draw_clock(canvas, keep_alive),
            Pending::Everything => self.paint(canvas, keep_alive),
        }
    }
}

impl MusicScreen {
    /// Asks the phone for something, unless there is no phone to ask.
    ///
    /// The controls are disabled while disconnected and never activate, so this
    /// only catches the gestures - which have no disabled state to wear.
    const fn command(&self, control: MusicControl) -> ScreenAction {
        if self.connected {
            ScreenAction::MusicControl(control)
        } else {
            ScreenAction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    fn tick(uptime_seconds: u64) -> AppEvent {
        AppEvent::Tick {
            uptime_seconds,
            wall_time: None,
            date: None,
        }
    }

    fn touch(slot: usize, pressed: bool) -> AppEvent {
        let bounds = control_bounds(slot);
        AppEvent::Touch {
            x: bounds.x() + bounds.width() / 2,
            y: bounds.y() + bounds.height() / 2,
            pressed,
        }
    }

    fn press(screen: &mut MusicScreen, slot: usize) -> ScreenAction {
        let _ = screen.handle_event(touch(slot, true));
        screen.handle_event(touch(slot, false))
    }

    /// A connected watch with a track playing from its start at uptime 100.
    fn playing() -> MusicScreen {
        let mut screen = MusicScreen::default();
        let _ = screen.handle_event(AppEvent::BleUpdated(BleState::Connected));
        let mut state = MusicState::new();
        let _ = state.set_track(b"All My Friends");
        let _ = state.set_artist(b"LCD Soundsystem");
        let _ = state.set_playing(&[1]);
        let _ = state.set_length(&420_u32.to_be_bytes());
        let _ = screen.apply(&state, 100);
        screen.mark_painted();
        screen
    }

    fn repaint(screen: &MusicScreen) -> Probe {
        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    /// The three controls are the point of the screen, so each has to send the
    /// command it draws. A transposed pair here is a next button that goes back.
    #[test]
    fn each_control_sends_the_command_it_draws() {
        let mut screen = playing();
        assert_eq!(
            press(&mut screen, 0),
            ScreenAction::MusicControl(MusicControl::Previous)
        );
        assert_eq!(
            press(&mut screen, 2),
            ScreenAction::MusicControl(MusicControl::Next)
        );
        // The middle one asks for the opposite of what is happening.
        assert_eq!(
            press(&mut screen, 1),
            ScreenAction::MusicControl(MusicControl::Pause)
        );
    }

    #[test]
    fn the_free_gestures_are_volume() {
        let mut screen = playing();
        assert_eq!(
            screen.handle_event(AppEvent::Swipe(SwipeDirection::Up)),
            ScreenAction::MusicControl(MusicControl::VolumeUp)
        );
        assert_eq!(
            screen.handle_event(AppEvent::Swipe(SwipeDirection::Down)),
            ScreenAction::MusicControl(MusicControl::VolumeDown)
        );
        // Right leaves the screen, so it is never the screen's to act on.
        assert_eq!(
            screen.handle_event(AppEvent::Swipe(SwipeDirection::Right)),
            ScreenAction::None
        );
    }

    /// With no phone there is nothing to command, and the screen must not
    /// pretend otherwise - by a control that activates or by a gesture that
    /// sends into nothing.
    #[test]
    fn nothing_is_asked_of_a_phone_that_is_not_there() {
        let mut screen = MusicScreen::default();
        let _ = screen.handle_event(AppEvent::BleUpdated(BleState::Advertising));

        for slot in 0..SLOTS {
            assert_eq!(
                press(&mut screen, slot),
                ScreenAction::None,
                "control {slot} activated with no phone connected"
            );
        }
        assert_eq!(
            screen.handle_event(AppEvent::Swipe(SwipeDirection::Up)),
            ScreenAction::None
        );
        for control in &screen.controls {
            assert_eq!(control.state(), ButtonState::Disabled);
        }

        // Connecting brings them back without anything else happening.
        let _ = screen.handle_event(AppEvent::BleUpdated(BleState::Connected));
        for control in &screen.controls {
            assert_eq!(control.state(), ButtonState::Idle);
        }
    }

    /// A second is a repaint of the numerals and the bar, not of the panel.
    /// This screen ticks for as long as it is open, and repainting it once a
    /// second is what flicker is.
    #[test]
    fn a_passing_second_repaints_the_clock_and_not_the_panel() {
        let mut screen = playing();
        let _ = screen.handle_event(tick(101));

        let painted = 240 * 240 - repaint(&screen).unpainted();
        assert!(painted > 0, "a second drew nothing");
        let clock_area = (CLOCK_WIDTH * DIGIT.height + ROW_WIDTH * 16) as usize;
        assert!(
            painted <= clock_area,
            "a second painted {painted} pixels, more than the clock's {clock_area}"
        );
    }

    /// A paused track does not move, so a tick has nothing to repaint. Without
    /// this the screen would send a frame a second forever while sitting still.
    #[test]
    fn a_tick_costs_nothing_while_the_track_is_paused() {
        let mut screen = playing();
        let mut state = MusicState::new();
        let _ = state.set_track(b"All My Friends");
        let _ = screen.apply(&state, 100);
        screen.mark_painted();

        let _ = screen.handle_event(tick(140));
        assert!(!screen.moved(), "a paused track asked for a repaint");
        assert_eq!(repaint(&screen).unpainted(), 240 * 240);
    }

    /// The elapsed time has to survive the watch sleeping through it, which is
    /// the whole reason the position is anchored rather than counted. The
    /// screen's part of that is passing the tick's uptime through unchanged.
    #[test]
    fn the_clock_catches_up_after_a_sleep_it_counted_no_seconds_through() {
        let mut screen = playing();
        // One tick, then nothing for five minutes while the panel is dark.
        let _ = screen.handle_event(tick(101));
        let _ = screen.handle_event(tick(400));
        assert_eq!(screen.playback.elapsed_minutes_seconds(screen.now), (5, 0));
    }

    /// Both empty states have to say which one they are: one wants the phone
    /// brought closer and the other wants something played, and a blank screen
    /// asks for neither.
    #[test]
    fn an_empty_screen_says_why_it_is_empty() {
        let mut screen = MusicScreen::default();
        assert_eq!(screen.heading().0, "NO PHONE");

        let _ = screen.handle_event(AppEvent::BleUpdated(BleState::Connected));
        assert_eq!(screen.heading().0, "NOTHING PLAYING");

        let mut state = MusicState::new();
        let _ = state.set_track(b"Someone Great");
        let _ = screen.apply(&state, 10);
        assert_eq!(screen.heading().0, "Someone Great");
    }

    /// A title longer than the panel is cut, not run off the edge, and the cut
    /// lands on a character rather than inside one.
    #[test]
    fn a_long_title_is_cut_to_what_the_panel_holds() {
        assert_eq!(clipped("short", TITLE_COLUMNS), "short");
        let long = "a title that is very much longer than the panel";
        assert_eq!(clipped(long, TITLE_COLUMNS).chars().count(), TITLE_COLUMNS);
        // A multi-byte character must not be split down the middle.
        assert_eq!(clipped("äöüäöüäöü", 4).chars().count(), 4);
    }

    /// The colour is what says whether the clock is running; the numerals
    /// themselves look the same either way.
    #[test]
    fn the_clock_is_the_accent_only_while_the_track_runs() {
        let screen = playing();
        assert_eq!(screen.clock_ink(), theme::ACCENT);

        let mut paused = playing();
        let mut state = MusicState::new();
        let _ = state.set_playing(&[0]);
        let _ = paused.apply(&state, 200);
        assert_eq!(paused.clock_ink(), theme::TEXT);
    }
}
