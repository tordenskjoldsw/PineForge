//! An on-demand heart-rate reading.
//!
//! The sensor, the acquisition cadence and the signal analysis were all already
//! running before this screen existed - the only way to reach them was the
//! periodic setting, which is not how anyone asks for their pulse. This screen
//! asks for one now and shows what comes back.
//!
//! It holds no sensor and runs no analysis. What it keeps is the last
//! [`HeartRateState`] the service published, which is fed to it from the event
//! stream whether or not it is the screen showing - a reading that arrived
//! while the watchface was up must not leave a stale number here.
//!
//! The layout follows `InfiniTime`'s: a value that changes colour while a
//! reading is running, a status line under it, and an offer that says to stop
//! rather than to start. Two things differ. The heart beats while a reading is
//! in flight, where `InfiniTime` shows no animation at all and a measurement is
//! visible only as a colour and a word. And the number is built from rectangles
//! rather than set in a face - the same numerals the FORGE watchface and the
//! steps app use, which is what makes three screens look like one watch.
//!
//! Those numerals are why the reading can be 84 pixels tall. `InfiniTime` sets
//! its own at 76 and pays for an atlas to do it; seven rectangles and a table of
//! which are lit cost nothing, so the size is a layout decision rather than a
//! flash one. The unused hundreds place stays drawn and unlit, which is what an
//! instrument does with a place it is not using.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use pineforge_state::{
    AppEvent, Button, ButtonBounds, ButtonOutcome, HeartRateState, ScreenAction,
};

use crate::canvas::{Canvas, CanvasError};
use crate::{
    icons::{self, ICON_SIZE, Icon, draw_icon},
    render::{PANEL, draw_centred, draw_instrument_centred, draw_visible},
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell, right_aligned},
    theme,
};

const PANEL_SIDE: i32 = PANEL.size.width.cast_signed();
/// The whole panel, not the strip below the status corner.
///
/// The corner paints its own background but only over itself, on the right;
/// the rest of that top row is this screen's to fill. Starting below it left a
/// band of whatever the previous screen had there - which is exactly what the
/// host opacity test is for, and what it caught.
const BODY: Rectangle = Rectangle::new(
    Point::zero(),
    Size::new(PANEL_SIDE.cast_unsigned(), PANEL_SIDE.cast_unsigned()),
);

/// Baseline of the label over the reading, level with the steps app's.
const LABEL_BASELINE_Y: i32 = 44;

/// Three places, which covers every heart rate a person has.
const PLACES: i32 = 3;
const PLACE_COUNT: usize = 3;
/// Half again the steps app's numerals. Three of them have room five do not, and
/// this screen exists to show one number.
const DIGIT: SegmentSize = SegmentSize::new(62, 84, 11);
const DIGIT_GAP: i32 = 8;
const DIGITS_WIDTH: i32 = PLACES * DIGIT.width + (PLACES - 1) * DIGIT_GAP;
const DIGITS_X: i32 = (PANEL_SIDE - DIGITS_WIDTH) / 2;
const DIGITS_Y: i32 = 62;

/// The heart sits where the steps app puts its gauge, so the two screens share a
/// rhythm as well as a face: label, number, band, two lines.
const HEART_Y: i32 = 158;
const STATUS_BASELINE: i32 = 202;
const OFFER_BASELINE: i32 = 224;

/// The box both heart frames are drawn in, and the only region a beat repaints.
const HEART_BOX: Rectangle = Rectangle::new(
    Point::new((PANEL_SIDE - ICON_SIZE) / 2, HEART_Y),
    Size::new(ICON_SIZE.cast_unsigned(), ICON_SIZE.cast_unsigned()),
);

/// What still owes a repaint.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Nothing,
    /// Only the heart moved, which is one small rectangle rather than a screen.
    Beat,
    /// Only the number moved: one reading replaced by another.
    ///
    /// Everything else this screen draws is the same for any `Result` - the
    /// status reads BPM, the offer says to measure again, the ink is the resting
    /// colour and the heart is at rest - so the three digit cells are the whole
    /// of the change. Each one fills its own background before it draws, so
    /// repainting them leaves nothing of the old number behind.
    Value,
    Everything,
}

/// Whether a reading is in flight.
///
/// The service reports three states as it powers the sensor up and fills its
/// window; they are one situation to the person waiting, and the one situation
/// this screen shows differently.
const fn measuring(state: HeartRateState) -> bool {
    matches!(
        state,
        HeartRateState::Starting | HeartRateState::Collecting | HeartRateState::Measuring
    )
}

/// The status line, and the offer under it.
///
/// Both come from one place so the two cannot describe different situations -
/// the status saying a number arrived while the offer still says to stop
/// waiting for one.
const fn readout(state: HeartRateState) -> (&'static str, &'static str) {
    match state {
        HeartRateState::Disabled => ("READY", "TAP TO MEASURE"),
        HeartRateState::Starting | HeartRateState::Collecting | HeartRateState::Measuring => {
            ("MEASURING", "KEEP STILL - TAP TO STOP")
        }
        HeartRateState::Result(_) => ("BPM", "TAP TO MEASURE AGAIN"),
        HeartRateState::NoSignal => ("NO SIGNAL", "WEAR SNUG - TAP TO RETRY"),
        HeartRateState::AmbientLight => ("TOO BRIGHT", "SHIELD THE SENSOR"),
        HeartRateState::Error => ("SENSOR ERROR", "NOT AVAILABLE"),
    }
}

pub struct PulseScreen {
    /// The whole panel, as one control: anywhere is a fine place to tap, and
    /// asking someone to hit a target is the wrong thing to do to a hand that
    /// is about to be held still.
    surface: Button,
    state: HeartRateState,
    /// Which frame of the beat is showing. Only meaningful while measuring.
    beat: bool,
    pending: Pending,
}

impl Default for PulseScreen {
    fn default() -> Self {
        Self {
            surface: Button::new(ButtonBounds::new(0, 0, PANEL_SIDE, PANEL_SIDE)),
            state: HeartRateState::Disabled,
            beat: false,
            pending: Pending::Nothing,
        }
    }
}

impl PulseScreen {
    /// Whether the last event left anything to repaint.
    #[must_use]
    pub const fn moved(&self) -> bool {
        !matches!(self.pending, Pending::Nothing)
    }

    /// The reading, as three cells.
    ///
    /// Blank places rather than zeroes when there is no number to show. Nobody
    /// has a heart rate of zero, so a row of noughts would be a reading rather
    /// than the absence of one - and an unlit place is exactly what an
    /// instrument shows for a value it does not have. The zero padding this
    /// replaces was there to stop a centred number shifting as it crossed from
    /// 99 to 100; a blank place holds that position without claiming a digit.
    fn cells(&self) -> [Cell; PLACE_COUNT] {
        match self.state {
            HeartRateState::Result(bpm) => right_aligned::<PLACE_COUNT>(u32::from(bpm)),
            _ => [Cell::Blank; PLACE_COUNT],
        }
    }

    /// The reading, in the numerals the watchface and the steps app use.
    fn draw_value(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let ink = self.value_ink();
        for (place, cell) in self.cells().into_iter().enumerate() {
            let x = DIGITS_X + i32::try_from(place).unwrap_or(0) * (DIGIT.width + DIGIT_GAP);
            draw_cell(canvas, DIGIT, x, DIGITS_Y, cell, ink)?;
            keep_alive();
        }
        Ok(())
    }

    /// The value's colour says whether this number is live.
    const fn value_ink(&self) -> Rgb565 {
        if measuring(self.state) {
            theme::ACCENT
        } else {
            theme::TEXT
        }
    }

    /// Between beats only while measuring. At rest the heart is simply there -
    /// a resting screen that twitched would report something not happening.
    const fn frame(&self) -> &'static Icon {
        if measuring(self.state) && self.beat {
            &icons::HEART_SMALL
        } else {
            &icons::HEART
        }
    }

    /// Repaints the heart alone.
    fn draw_heart(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        draw_visible(
            &HEART_BOX.into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND)),
            canvas,
        )?;
        draw_icon(self.frame(), HEART_BOX.top_left, theme::ACCENT, canvas)
    }

    fn paint(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        draw_visible(
            &BODY.into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND)),
            canvas,
        )?;
        keep_alive();
        draw_instrument_centred("PULSE", LABEL_BASELINE_Y, theme::ACCENT, canvas)?;
        self.draw_value(canvas, keep_alive)?;
        draw_icon(self.frame(), HEART_BOX.top_left, theme::ACCENT, canvas)?;

        // The status and the offer are set smaller, not dimmer. `theme::FRAME`
        // is held at the 3:1 a non-text element needs and would be
        // under-contrast as words; the size is what carries the hierarchy.
        let (status, offer) = readout(self.state);
        draw_instrument_centred(status, STATUS_BASELINE, theme::ACCENT, canvas)?;
        draw_centred(offer, OFFER_BASELINE, theme::TEXT, canvas)
    }
}

impl Paint for PulseScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas, keep_alive)
    }
}

impl Screen for PulseScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        self.pending = Pending::Nothing;

        // A reading is not an interaction: it arrives on its own and only ever
        // changes what is shown.
        if let AppEvent::HeartRateStateUpdated(state) = event {
            if self.state != state {
                // One number replacing another changes nothing else on the
                // screen, and continuous measurement produces exactly that
                // several times a minute. Repainting the panel for it is what
                // makes the app flicker - and this screen holds the watch awake,
                // so it would keep doing it for as long as the app is open.
                let value_only = matches!(
                    (self.state, state),
                    (HeartRateState::Result(_), HeartRateState::Result(_))
                );
                self.state = state;
                if value_only {
                    self.pending = Pending::Value;
                } else {
                    // A fresh reading starts the beat from the same frame every
                    // time, so two measurements in a row look alike.
                    self.beat = false;
                    self.pending = Pending::Everything;
                }
            }
            return ScreenAction::None;
        }

        // One beat a second, the rate the tick already arrives at. Nothing here
        // asks for a faster one: the point is to show the reading is alive, not
        // to render a waveform.
        if matches!(event, AppEvent::Tick { .. }) {
            if measuring(self.state) {
                self.beat = !self.beat;
                self.pending = Pending::Beat;
            }
            return ScreenAction::None;
        }

        // The service owns the sensor, so this screen can only ask. Tapping
        // during a reading gives up on it; the periodic setting is not this
        // screen's to change either way.
        if self.surface.handle_event(event) == ButtonOutcome::Activated {
            return if measuring(self.state) {
                ScreenAction::StopHeartRate
            } else {
                ScreenAction::MeasureHeartRate
            };
        }
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        // A beat repaints its own box and nothing else. Repainting the panel
        // once a second is what flicker is, and this screen holds the watch
        // awake, so the ticks driving it never stop.
        match self.pending {
            Pending::Nothing => Ok(()),
            Pending::Beat => self.draw_heart(canvas),
            Pending::Value => self.draw_value(canvas, keep_alive),
            Pending::Everything => self.paint(canvas, keep_alive),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    const TICK: AppEvent = AppEvent::Tick {
        uptime_seconds: 1,
        wall_time: None,
        date: None,
    };

    fn touch(pressed: bool) -> AppEvent {
        AppEvent::Touch {
            x: 120,
            y: 120,
            pressed,
        }
    }

    fn tap(screen: &mut PulseScreen) -> ScreenAction {
        let _ = screen.handle_event(touch(true));
        screen.handle_event(touch(false))
    }

    fn repaint(screen: &PulseScreen) -> Probe {
        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    /// Tapping asks for a reading, and tapping again during one gives up on it
    /// rather than asking a second time.
    #[test]
    fn a_tap_starts_a_reading_and_the_next_one_stops_it() {
        let mut screen = PulseScreen::default();
        assert_eq!(tap(&mut screen), ScreenAction::MeasureHeartRate);

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Collecting));
        assert_eq!(tap(&mut screen), ScreenAction::StopHeartRate);

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Result(72)));
        assert_eq!(tap(&mut screen), ScreenAction::MeasureHeartRate);
    }

    /// A second reading replacing a first repaints the number and nothing else.
    ///
    /// Continuous measurement produces one of these every few seconds, and the
    /// screen holds the watch awake, so a full repaint here is a panel redrawn
    /// over and over for a two-digit change - which is what it looked like.
    #[test]
    fn a_new_reading_repaints_the_number_and_not_the_panel() {
        let mut screen = PulseScreen::default();
        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Result(72)));
        let _ = repaint(&screen);

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Result(73)));
        let probe = repaint(&screen);
        let painted = 240 * 240 - probe.unpainted();
        let digits = (DIGITS_WIDTH * DIGIT.height) as usize;
        assert!(painted > 0, "the new reading drew nothing");
        assert!(
            painted <= digits,
            "a new reading painted {painted} pixels, more than the {digits} the digits occupy"
        );
        // The lines under the number say the same thing for any result, so a
        // value change must not have touched them.
        assert!(
            !probe.painted_within(Rectangle::new(
                Point::new(0, STATUS_BASELINE - 16),
                Size::new(240, 40),
            )),
            "a value change repainted the status and offer lines"
        );
    }

    /// Anything that is not one result following another still repaints the
    /// screen, because the status, the offer, the ink and the heart all move
    /// with it.
    #[test]
    fn arriving_at_a_result_repaints_the_screen() {
        let mut screen = PulseScreen::default();
        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Measuring));
        let _ = repaint(&screen);

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Result(72)));
        assert_eq!(
            repaint(&screen).unpainted(),
            0,
            "the first result left part of the screen showing the measuring state"
        );
    }

    /// The beat is what makes the screen look busy, and it must cost one small
    /// rectangle: this screen holds the watch awake, so a tick that repainted
    /// the panel would keep doing it for as long as the app is open.
    #[test]
    fn a_beat_repaints_the_heart_and_not_the_panel() {
        let mut screen = PulseScreen::default();
        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Measuring));
        let _ = repaint(&screen);

        let _ = screen.handle_event(TICK);
        let painted = 240 * 240 - repaint(&screen).unpainted();
        let box_area = (ICON_SIZE * ICON_SIZE).cast_unsigned() as usize;
        assert!(painted > 0, "the beat drew nothing");
        assert!(
            painted <= box_area,
            "a beat painted {painted} pixels, more than the heart's {box_area}"
        );
    }

    /// At rest the screen is still. A heart that twitched while nothing was
    /// being measured would be reporting something that is not happening.
    #[test]
    fn a_tick_costs_nothing_when_no_reading_is_running() {
        let mut screen = PulseScreen::default();
        for state in [
            HeartRateState::Disabled,
            HeartRateState::Result(72),
            HeartRateState::NoSignal,
        ] {
            let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(state));
            let _ = repaint(&screen);

            let _ = screen.handle_event(TICK);
            assert_eq!(
                repaint(&screen).unpainted(),
                240 * 240,
                "{state:?} let a tick repaint"
            );
        }
    }

    /// A reading fills its places from the right and leaves the rest unlit; no
    /// reading at all leaves every place unlit.
    ///
    /// The distinction is the whole reason for blanks over zeroes: `000` is a
    /// heart rate nobody has, and a screen showing one claims a measurement it
    /// does not hold.
    #[test]
    fn a_reading_lights_its_places_and_nothing_else_lights_any() {
        let mut screen = PulseScreen::default();
        assert_eq!(
            screen.cells(),
            [Cell::Blank; 3],
            "an idle screen showed a number"
        );

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Result(63)));
        assert_eq!(
            screen.cells(),
            [Cell::Blank, Cell::Digit(6), Cell::Digit(3)]
        );

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Result(128)));
        assert_eq!(
            screen.cells(),
            [Cell::Digit(1), Cell::Digit(2), Cell::Digit(8)],
            "a three-figure rate lost its hundreds"
        );

        // A measurement that failed is not a measurement.
        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::NoSignal));
        assert_eq!(screen.cells(), [Cell::Blank; 3]);
    }

    /// The value carries whether it is live, which is how `InfiniTime` shows a
    /// reading in progress - the colour is not decoration.
    #[test]
    fn the_value_is_the_accent_only_while_a_reading_runs() {
        let mut screen = PulseScreen::default();
        assert_eq!(screen.value_ink(), theme::TEXT);

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Collecting));
        assert_eq!(screen.value_ink(), theme::ACCENT);

        let _ = screen.handle_event(AppEvent::HeartRateStateUpdated(HeartRateState::Result(72)));
        assert_eq!(screen.value_ink(), theme::TEXT);
    }
}
