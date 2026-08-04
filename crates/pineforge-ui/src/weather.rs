//! What the phone last said the weather is.
//!
//! The same rhythm the pulse and steps applications have - a label, one number
//! set large, and two lines under it - because the number is what you opened the
//! screen for and everything else is context for it.
//!
//! Nothing here fetches anything. The BLE task receives the companion's writes,
//! `pineforge_state::parse_simple_weather` decides what they mean, and this
//! screen keeps the last record it was handed whether or not it is showing.
//!
//! # No icons yet
//!
//! `InfiniTime` draws nine weather symbols. This screen names the condition
//! instead, in the instrument face the other applications use for their status
//! line. Nine new glyphs are a drawing decision and a flash cost of their own,
//! and a word is not a placeholder: on a 240-pixel panel `THUNDERSTORM` is
//! unambiguous where a small symbol is a guess.
//!
//! # Degrees, not hundredths
//!
//! The wire carries hundredths and the screen shows whole degrees, rounded to
//! nearest. A watch that reads one degree cooler than the phone it synced from
//! would be a watch nobody trusts, and truncation makes that the normal case
//! rather than the edge.

use core::fmt::Write;

use embedded_graphics::{
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use pineforge_state::{AppEvent, CurrentWeather, ScreenAction, WeatherIcon};

use crate::{
    canvas::{Canvas, CanvasError},
    render::{PANEL, draw_centred, draw_instrument_centred, draw_visible},
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell, signed_aligned},
    theme,
};

/// The whole panel. The status corner paints its own background over its own
/// cell; the rest of that row belongs to this screen, and starting below it
/// would leave a band of whatever was there before.
const BODY: Rectangle = Rectangle::new(Point::zero(), PANEL.size);

/// Baseline of the location over the reading, level with the other apps'.
const LABEL_BASELINE_Y: i32 = 44;

/// Three places: a sign and two digits, which covers every temperature this
/// watch will be worn in. The size is the pulse application's, because a
/// temperature is the same kind of number as a heart rate - one value, read at
/// a glance.
/// Kept as both, the way the steps application does, rather than cast at every
/// use: the layout wants an `i32` and the cell array wants a length.
const PLACES: i32 = 3;
const PLACE_COUNT: usize = 3;
const DIGIT: SegmentSize = SegmentSize::new(62, 84, 11);
const DIGIT_GAP: i32 = 8;
const DIGITS_WIDTH: i32 = PLACES * DIGIT.width + (PLACES - 1) * DIGIT_GAP;
const DIGITS_X: i32 = (PANEL.size.width.cast_signed() - DIGITS_WIDTH) / 2;
const DIGITS_Y: i32 = 62;

const CONDITION_BASELINE: i32 = 176;
const RANGE_BASELINE: i32 = 202;
const FOOTER_BASELINE: i32 = 224;

const _: () = assert!(
    DIGITS_X >= 0,
    "three of these numerals do not fit across the panel"
);
const _: () = assert!(
    DIGITS_Y + DIGIT.height < CONDITION_BASELINE,
    "the reading runs into the condition"
);

/// The condition, as a word.
const fn condition(icon: WeatherIcon) -> &'static str {
    match icon {
        WeatherIcon::Sun => "CLEAR",
        WeatherIcon::CloudsSun => "FAIR",
        WeatherIcon::Clouds => "CLOUDY",
        WeatherIcon::BrokenClouds => "OVERCAST",
        WeatherIcon::CloudShowerHeavy => "SHOWERS",
        WeatherIcon::CloudSunRain => "RAIN",
        WeatherIcon::Thunderstorm => "STORM",
        WeatherIcon::Snow => "SNOW",
        WeatherIcon::Smog => "MIST",
        WeatherIcon::Unknown => "-",
    }
}

/// The last record the phone sent, or nothing yet.
#[derive(Default)]
pub struct WeatherScreen {
    current: Option<CurrentWeather>,
    dirty: bool,
}

impl WeatherScreen {
    /// Whether the last event left anything to repaint.
    #[must_use]
    pub const fn moved(&self) -> bool {
        self.dirty
    }

    /// Says the panel now shows the record this screen holds.
    pub const fn mark_painted(&mut self) {
        self.dirty = false;
    }

    /// Takes a record the BLE task received, and reports whether it moved.
    pub fn apply(&mut self, current: &CurrentWeather) -> bool {
        if self.current.as_ref() == Some(current) {
            return false;
        }
        self.current = Some(current.clone());
        self.dirty = true;
        true
    }

    /// The location, or what to say instead before a phone has sent one.
    ///
    /// A blank line would read as a screen that failed to draw. Saying the
    /// watch is waiting says which of the two it is.
    fn label(&self) -> &str {
        match &self.current {
            Some(current) if !current.location.is_empty() => current.location.as_str(),
            Some(_) => "WEATHER",
            None => "NO DATA",
        }
    }

    /// The reading as cells, blank until a record arrives.
    ///
    /// Blank rather than zero, for the reason the pulse screen uses blanks: a
    /// watch that has not been told the weather and a watch that was told zero
    /// degrees are different things, and one of them is not a reading.
    fn cells(&self) -> [Cell; PLACE_COUNT] {
        self.current
            .as_ref()
            .map_or([Cell::Blank; PLACE_COUNT], |current| {
                signed_aligned(i32::from(current.temperature.celsius()))
            })
    }

    fn draw_value(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        for (place, cell) in self.cells().into_iter().enumerate() {
            let x = DIGITS_X + i32::try_from(place).unwrap_or(0) * (DIGIT.width + DIGIT_GAP);
            draw_cell(canvas, DIGIT, x, DIGITS_Y, cell, theme::ACCENT)?;
            keep_alive();
        }
        Ok(())
    }

    /// Today's low and high, as one line.
    ///
    /// Empty before a record arrives, which is what the caller draws: a range
    /// with nothing behind it would be two numbers the phone never sent.
    fn range(&self) -> String<16> {
        let mut line = String::new();
        if let Some(current) = &self.current {
            let _ = write!(
                line,
                "{} / {}",
                current.minimum.celsius(),
                current.maximum.celsius()
            );
        }
        line
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

        draw_instrument_centred(self.label(), LABEL_BASELINE_Y, theme::ACCENT, canvas)?;
        self.draw_value(canvas, keep_alive)?;

        let condition = self
            .current
            .as_ref()
            .map_or("WAITING FOR A PHONE", |current| condition(current.icon));
        draw_instrument_centred(condition, CONDITION_BASELINE, theme::ACCENT, canvas)?;
        draw_centred(self.range().as_str(), RANGE_BASELINE, theme::TEXT, canvas)?;
        draw_centred("DEGREES CELSIUS", FOOTER_BASELINE, theme::TEXT, canvas)
    }
}

impl Paint for WeatherScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas, keep_alive)
    }
}

impl Screen for WeatherScreen {
    fn handle_event(&mut self, _event: AppEvent) -> ScreenAction {
        // Nothing on this screen is touched. The record arrives through
        // `apply`, the way a reading reaches the face.
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if !self.dirty {
            return Ok(());
        }
        // A record changes the location, the reading, the condition and the
        // range together, so there is no partial case worth naming.
        self.paint(canvas, keep_alive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;
    use pineforge_state::Temperature;

    fn record(temperature: i16, minimum: i16, maximum: i16, place: &str) -> CurrentWeather {
        CurrentWeather {
            timestamp: 1_700_000_000,
            temperature: Temperature::from_hundredths(temperature),
            minimum: Temperature::from_hundredths(minimum),
            maximum: Temperature::from_hundredths(maximum),
            icon: WeatherIcon::Snow,
            location: String::try_from(place).unwrap(),
            sunrise: Some(389),
            sunset: Some(1_260),
        }
    }

    fn repaint(screen: &WeatherScreen) -> Probe {
        let mut probe = Probe::new();
        screen
            .draw_full(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    #[test]
    fn a_screen_with_no_record_still_covers_the_panel() {
        assert_eq!(repaint(&WeatherScreen::default()).unpainted(), 0);
    }

    #[test]
    fn a_record_covers_the_panel_too() {
        let mut screen = WeatherScreen::default();
        assert!(screen.apply(&record(-450, -800, 120, "Kiel")));
        assert_eq!(repaint(&screen).unpainted(), 0);
    }

    /// The same record twice is not a repaint. Gadgetbridge rewrites the
    /// characteristic on its own schedule, and a screen that redrew for every
    /// write would flicker for weather that had not changed.
    #[test]
    fn the_same_record_does_not_ask_for_a_repaint() {
        let mut screen = WeatherScreen::default();
        assert!(screen.apply(&record(500, 100, 900, "Kiel")));
        screen.mark_painted();
        assert!(!screen.apply(&record(500, 100, 900, "Kiel")));
        assert!(!screen.moved());
    }

    #[test]
    fn the_range_reads_as_low_over_high_in_whole_degrees() {
        let mut screen = WeatherScreen::default();
        let _ = screen.apply(&record(0, -450, 1_260, "Kiel"));
        assert_eq!(screen.range().as_str(), "-5 / 13");
    }

    /// A location the phone did not send leaves the label with something to
    /// say. A blank line reads as a screen that failed to draw.
    #[test]
    fn a_missing_location_says_so_rather_than_leaving_a_gap() {
        assert_eq!(WeatherScreen::default().label(), "NO DATA");
        let mut screen = WeatherScreen::default();
        let _ = screen.apply(&record(0, 0, 0, ""));
        assert_eq!(screen.label(), "WEATHER");
    }

    /// Blank places before a phone has said anything: nought degrees is a
    /// reading and no reading is not.
    #[test]
    fn no_record_shows_blank_places_rather_than_zero() {
        assert_eq!(WeatherScreen::default().cells(), [Cell::Blank; PLACE_COUNT]);
    }

    #[test]
    fn a_temperature_below_zero_carries_its_sign() {
        let mut screen = WeatherScreen::default();
        let _ = screen.apply(&record(-450, -800, 120, "Kiel"));
        assert_eq!(screen.cells(), [Cell::Blank, Cell::Minus, Cell::Digit(5)]);
    }
}
