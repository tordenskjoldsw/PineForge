//! What the phone last said the weather is.
//!
//! The same rhythm the pulse and steps applications have - a label, one number
//! set large, and context under it - because the number is what you opened the
//! screen for and everything else is there to place it.
//!
//! Nothing here fetches anything. The BLE task receives the companion's writes,
//! `pineforge_state::parse_simple_weather` decides what they mean, and this
//! screen keeps the last record it was handed whether or not it is showing.
//!
//! # Two records, one screen
//!
//! Conditions and the five-day forecast are separate writes on the same
//! characteristic, and a phone may send either without the other. So they are
//! held as two `Option`s and drawn independently: a screen that waited for both
//! would show nothing at all in the common case where only one arrived.
//!
//! # Symbols rather than words
//!
//! The condition is one of nine 24x24 icons, matching the set `InfiniTime`
//! draws and the set the wire format enumerates. A word would be less ambiguous
//! read one at a time - `THUNDERSTORM` is never a guess - but the forecast puts
//! five conditions in a row 48 pixels wide apiece, and there is no face on this
//! watch that fits a condition into 48 pixels legibly.
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
    text::{Alignment, Text},
};
use heapless::String;
use pineforge_state::{AppEvent, CurrentWeather, Forecast, ScreenAction, WeatherIcon};

use crate::{
    canvas::{Canvas, CanvasError},
    font::body_text,
    icons::{self, ICON_SIZE, Icon, draw_icon},
    render::{PANEL, draw_centred, draw_centred_at, draw_visible},
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell, signed_aligned},
    theme,
};

/// The whole panel. The status corner paints its own background over its own
/// cell; the rest of that row belongs to this screen, and starting below it
/// would leave a band of whatever was there before.
const BODY: Rectangle = Rectangle::new(Point::zero(), PANEL.size);

/// Baseline of the location over the reading, level with the other apps'.
///
/// Set in the reading face and not the instrument one, unlike every other
/// application's label. Those are constants this firmware writes - `PULSE`,
/// `STEPS` - and the FORGE face carries `U+0041..=U+005A` and nothing else,
/// which is enough for a word chosen here. A place name is not: it arrives from
/// the phone in whatever case its weather service uses, and drawn in that face
/// `Bad Oldesloe` comes out as `B    O`.
const LABEL_BASELINE_Y: i32 = 44;

/// Places the label has room for before it is cut.
const LABEL_COLUMNS: usize =
    (PANEL.size.width / crate::font::LIBERATION_MONO_8X18.cell.width) as usize - 2;

/// The condition, as a symbol, in the corner the status rune does not use.
///
/// Beside the location rather than under the number: the three numerals span
/// almost the whole panel, so there is no room next to them, and the row below
/// belongs to the forecast.
const CONDITION_ICON: Point = Point::new(8, 26);

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

/// Today's low and high, under the reading they bracket.
const RANGE_BASELINE: i32 = 166;

/// The forecast: five columns across the panel, an icon over a temperature.
const FORECAST_COLUMNS: i32 = 5;
const FORECAST_COLUMN: i32 = PANEL.size.width.cast_signed() / FORECAST_COLUMNS;
const FORECAST_ICON_Y: i32 = 180;
const FORECAST_BASELINE: i32 = 226;

const _: () = assert!(
    DIGITS_X >= 0,
    "three of these numerals do not fit across the panel"
);
const _: () = assert!(
    DIGITS_Y + DIGIT.height < RANGE_BASELINE - 14,
    "the reading runs into the range under it"
);
const _: () = assert!(
    FORECAST_ICON_Y + ICON_SIZE < FORECAST_BASELINE - 14,
    "the forecast icons run into the temperatures under them"
);
const _: () = assert!(
    FORECAST_COLUMN >= ICON_SIZE,
    "five forecast columns are narrower than the icon in one"
);

/// The condition, as one of the nine symbols.
///
/// `Unknown` has none of its own and takes the cloud, which is the least wrong
/// thing to draw for a condition the phone could not map: a watch showing a sun
/// for weather nobody identified would be making a claim.
const fn symbol(icon: WeatherIcon) -> &'static Icon {
    match icon {
        WeatherIcon::Sun => &icons::WEATHER_SUN,
        WeatherIcon::CloudsSun => &icons::WEATHER_CLOUDS_SUN,
        WeatherIcon::Clouds | WeatherIcon::Unknown => &icons::WEATHER_CLOUDS,
        WeatherIcon::BrokenClouds => &icons::WEATHER_BROKEN_CLOUDS,
        WeatherIcon::CloudShowerHeavy => &icons::WEATHER_SHOWER,
        WeatherIcon::CloudSunRain => &icons::WEATHER_RAIN,
        WeatherIcon::Thunderstorm => &icons::WEATHER_THUNDERSTORM,
        WeatherIcon::Snow => &icons::WEATHER_SNOW,
        WeatherIcon::Smog => &icons::WEATHER_MIST,
    }
}

/// One whole degree, as text.
fn degrees(value: i16) -> String<8> {
    let mut text = String::new();
    let _ = write!(text, "{value}");
    text
}

/// The last record the phone sent, or nothing yet.
#[derive(Default)]
pub struct WeatherScreen {
    current: Option<CurrentWeather>,
    /// The five days, kept apart from the conditions because they arrive as
    /// two separate writes and either can turn up without the other.
    forecast: Option<Forecast>,
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

    /// Takes a forecast, and reports whether it moved.
    pub fn apply_forecast(&mut self, forecast: &Forecast) -> bool {
        if self.forecast.as_ref() == Some(forecast) {
            return false;
        }
        self.forecast = Some(forecast.clone());
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

    /// The label, cut to the columns the panel has for it.
    ///
    /// Cut by characters and not by bytes. The name arrives from the phone as
    /// UTF-8 and nothing upstream of here reduces it to ASCII, so `Munchen`
    /// spelled the way its inhabitants spell it is eight bytes and seven
    /// columns. Slicing that by a byte index can land inside a character, which
    /// is a panic, and would cut a long name early besides.
    fn short_label(&self) -> &str {
        let label = self.label();
        match label.char_indices().nth(LABEL_COLUMNS) {
            Some((end, _)) => &label[..end],
            None => label,
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

    /// The forecast row: an icon over a whole-degree maximum, five across.
    ///
    /// The maximum alone, not the pair. Two numbers in a 48-pixel column are
    /// unreadable at this face, and the high is what somebody deciding on a
    /// coat is after - today's low is on the line above, where it belongs to the
    /// reading it brackets.
    fn draw_forecast(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let Some(forecast) = &self.forecast else {
            return Ok(());
        };
        for (index, day) in forecast.days.iter().enumerate() {
            let column = i32::try_from(index).unwrap_or(0) * FORECAST_COLUMN;
            let centre = column + FORECAST_COLUMN / 2;
            draw_icon(
                symbol(day.icon),
                Point::new(centre - ICON_SIZE / 2, FORECAST_ICON_Y),
                theme::TEXT,
                canvas,
            )?;
            draw_centred_at(
                degrees(day.maximum.celsius()).as_str(),
                centre,
                FORECAST_BASELINE,
                theme::TEXT,
                canvas,
            )?;
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

        draw_visible(
            &Text::with_alignment(
                self.short_label(),
                Point::new(PANEL.size.width.cast_signed() / 2, LABEL_BASELINE_Y),
                body_text(theme::ACCENT, theme::BACKGROUND),
                Alignment::Center,
            ),
            canvas,
        )?;
        if let Some(current) = &self.current {
            draw_icon(symbol(current.icon), CONDITION_ICON, theme::ACCENT, canvas)?;
        }
        self.draw_value(canvas, keep_alive)?;
        keep_alive();

        draw_centred(self.range().as_str(), RANGE_BASELINE, theme::TEXT, canvas)?;
        self.draw_forecast(canvas, keep_alive)
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
    use heapless::Vec;
    use pineforge_state::{ForecastDay, Temperature};

    fn forecast(days: &[(i16, i16, WeatherIcon)]) -> Forecast {
        let mut list: Vec<ForecastDay, 5> = Vec::new();
        for (minimum, maximum, icon) in days {
            let _ = list.push(ForecastDay {
                minimum: Temperature::from_hundredths(*minimum),
                maximum: Temperature::from_hundredths(*maximum),
                icon: *icon,
            });
        }
        Forecast {
            timestamp: 1_700_000_000,
            days: list,
        }
    }

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

    /// A place name too long for the panel is cut on a character boundary.
    ///
    /// The cut is by characters, so the one here lands after the umlaut rather
    /// than inside it. Cutting by bytes at the same limit splits that character
    /// and panics, which is why the name is built with one straddling the
    /// column the label runs out at.
    #[test]
    fn a_long_place_name_with_an_umlaut_is_cut_between_characters() {
        let mut place: heapless::String<32> = heapless::String::new();
        for _ in 0..LABEL_COLUMNS - 1 {
            place.push('a').unwrap();
        }
        place.push('\u{fc}').unwrap();
        place.push_str("bc").unwrap();
        assert!(
            place.chars().count() > LABEL_COLUMNS,
            "the name has to be long enough to cut"
        );

        let mut screen = WeatherScreen::default();
        let _ = screen.apply(&record(1_000, 0, 2_000, place.as_str()));
        assert_eq!(screen.short_label().chars().count(), LABEL_COLUMNS);
        assert!(screen.short_label().ends_with('\u{fc}'));
        assert_eq!(repaint(&screen).unpainted(), 0);
    }

    /// A forecast alone, with no conditions behind it, still has to cover the
    /// panel: the two arrive as separate writes and either can turn up first.
    #[test]
    fn a_forecast_without_conditions_still_covers_the_panel() {
        let mut screen = WeatherScreen::default();
        assert!(screen.apply_forecast(&forecast(&[
            (-500, 300, WeatherIcon::Snow),
            (100, 900, WeatherIcon::CloudsSun),
        ])));
        assert_eq!(repaint(&screen).unpainted(), 0);
    }

    #[test]
    fn a_full_five_day_forecast_covers_the_panel() {
        let mut screen = WeatherScreen::default();
        let _ = screen.apply(&record(1_200, -450, 1_800, "Kiel"));
        assert!(screen.apply_forecast(&forecast(&[
            (0, 1_800, WeatherIcon::Sun),
            (-200, 900, WeatherIcon::Clouds),
            (-1_250, 100, WeatherIcon::Snow),
            (500, 2_100, WeatherIcon::Thunderstorm),
            (300, 1_600, WeatherIcon::CloudShowerHeavy),
        ])));
        assert_eq!(repaint(&screen).unpainted(), 0);
    }

    #[test]
    fn the_same_forecast_does_not_ask_for_a_repaint() {
        let mut screen = WeatherScreen::default();
        let days = forecast(&[(0, 100, WeatherIcon::Sun)]);
        assert!(screen.apply_forecast(&days));
        screen.mark_painted();
        assert!(!screen.apply_forecast(&days));
        assert!(!screen.moved());
    }

    #[test]
    fn a_temperature_below_zero_carries_its_sign() {
        let mut screen = WeatherScreen::default();
        let _ = screen.apply(&record(-450, -800, 120, "Kiel"));
        assert_eq!(screen.cells(), [Cell::Blank, Cell::Minus, Cell::Digit(5)]);
    }
}
