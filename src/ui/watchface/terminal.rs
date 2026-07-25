//! A terminal-styled watchface: one labelled row per reading.

use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, MonoTextStyleBuilder, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;

use crate::ui::{render::draw_mono_text_visible, watchface::Watchface};
#[cfg(feature = "diagnostics")]
use pineforge_state::{AccelerometerKind, FeatureEngineStatus, HeartRateSensorKind, PpgAnalysis};
use pineforge_state::{BleState, CalendarDate, WatchField, WatchFields, WatchState};
#[cfg(not(feature = "diagnostics"))]
use pineforge_state::{HeartRateState, NotificationCategory};

const ROW_HEIGHT: u32 = 25;
const VALUE_X: i32 = 70;
/// Advance of `FONT_10X20`, used to place partial value redraws by character.
const GLYPH_WIDTH: i32 = 10;
const SCREEN_AREA: Rectangle = Rectangle::new(Point::new(0, 0), Size::new(240, 240));
const DATE_ROW: Rectangle = Rectangle::new(Point::new(0, 25), Size::new(240, ROW_HEIGHT));
const TIME_ROW: Rectangle = Rectangle::new(Point::new(0, 50), Size::new(240, ROW_HEIGHT));
const BATTERY_ROW: Rectangle = Rectangle::new(Point::new(0, 75), Size::new(240, ROW_HEIGHT));
#[cfg(feature = "diagnostics")]
const MOTION_ROW: Rectangle = Rectangle::new(Point::new(0, 100), Size::new(240, ROW_HEIGHT));
#[cfg(feature = "diagnostics")]
const STEP_ROW: Rectangle = Rectangle::new(Point::new(0, 125), Size::new(240, ROW_HEIGHT));
#[cfg(not(feature = "diagnostics"))]
const STEP_ROW: Rectangle = Rectangle::new(Point::new(0, 100), Size::new(240, ROW_HEIGHT));
#[cfg(not(feature = "diagnostics"))]
const HEART_RATE_ROW: Rectangle = Rectangle::new(Point::new(0, 125), Size::new(240, ROW_HEIGHT));
const RESERVED_ROW: Rectangle = Rectangle::new(Point::new(0, 150), Size::new(240, ROW_HEIGHT));
const STATUS_ROW: Rectangle = Rectangle::new(Point::new(0, 175), Size::new(240, ROW_HEIGHT));

const LIGHT_GRAY: Rgb565 = Rgb565::new(20, 40, 20);
const TERMINAL_GREEN: Rgb565 = Rgb565::new(4, 51, 10);
const TERMINAL_BLUE: Rgb565 = Rgb565::new(0, 31, 31);
const TERMINAL_ORANGE: Rgb565 = Rgb565::new(31, 32, 0);
const TERMINAL_RED: Rgb565 = Rgb565::new(31, 12, 0);

/// Stand-in date shown until a phone synchronizes the clock over BLE.
const UNSYNCHRONIZED_DATE: CalendarDate = CalendarDate {
    year: 2026,
    month: 1,
    day: 1,
};

/// Holds no readings of its own: everything it shows comes from the shared
/// [`WatchState`], so it is a layout and nothing else.
#[derive(Default)]
pub struct TerminalWatchface;

impl TerminalWatchface {
    /// Formats a time of day; before synchronization the uptime stands in for
    /// it, so the seconds wrap at a day rather than counting past 24 hours.
    fn format_clock(seconds: u64) -> String<16> {
        let of_day = seconds % 86_400;
        let hours = of_day / 3_600;
        let minutes = (of_day / 60) % 60;
        let seconds = of_day % 60;
        let mut clock = String::new();
        let _ = write!(clock, "{hours:02}:{minutes:02}:{seconds:02}");
        clock
    }

    fn format_date(date: CalendarDate) -> String<16> {
        let mut value = String::new();
        let _ = write!(value, "{:04}-{:02}-{:02}", date.year, date.month, date.day);
        value
    }

    fn draw_changed_value<D>(
        display: &mut D,
        old: &str,
        new: &str,
        baseline: i32,
        color: Rgb565,
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let first_changed = old
            .bytes()
            .zip(new.bytes())
            .position(|(old, new)| old != new)
            .unwrap_or_else(|| old.len().min(new.len()));
        if first_changed == new.len() && old.len() == new.len() {
            return Ok(());
        }

        let style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(color)
            .background_color(Rgb565::BLACK)
            .build();
        let x = VALUE_X + i32::try_from(first_changed).unwrap_or(0) * GLYPH_WIDTH;
        draw_mono_text_visible(
            &new[first_changed..],
            Point::new(x, baseline),
            style,
            display,
        )?;
        Ok(())
    }

    /// Draws a row in place, without blanking it first.
    ///
    /// Every glyph paints its own background, so redrawing a row with unchanged
    /// text leaves the panel visually untouched; clearing the row first would
    /// flash it black for the duration of the SPI transfer. Only the span a
    /// longer previous value may have left behind is cleared, and that span is
    /// already black whenever the value did not shrink.
    fn draw_row<D>(
        display: &mut D,
        area: Rectangle,
        label: &str,
        value: &str,
        value_color: Rgb565,
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let baseline = area.top_left.y + 20;
        let label_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(Rgb565::WHITE)
            .background_color(Rgb565::BLACK)
            .build();
        let value_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(value_color)
            .background_color(Rgb565::BLACK)
            .build();
        draw_mono_text_visible(label, Point::new(0, baseline), label_style, display)?;
        draw_mono_text_visible(value, Point::new(VALUE_X, baseline), value_style, display)?;

        let value_end = VALUE_X + i32::try_from(value.len()).unwrap_or(0) * GLYPH_WIDTH;
        let right_edge = area.top_left.x + i32::try_from(area.size.width).unwrap_or(0);
        let tail = u32::try_from(right_edge - value_end).unwrap_or(0);
        Rectangle::new(
            Point::new(value_end, area.top_left.y),
            Size::new(tail, area.size.height),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(display)
    }

    /// The date of the last synchronization, or a fixed stand-in before the
    /// first one arrives.
    fn draw_date<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let date = Self::format_date(state.date().unwrap_or(UNSYNCHRONIZED_DATE));
        Self::draw_row(display, DATE_ROW, "[DATE]", &date, TERMINAL_GREEN)
    }

    fn draw_time<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let clock = Self::format_clock(state.clock_seconds());
        Self::draw_row(display, TIME_ROW, "[TIME]", &clock, TERMINAL_GREEN)
    }

    /// Redraws only the clock characters that changed, which is a single digit
    /// on most seconds.
    fn update_clock<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let old_clock = Self::format_clock(state.previous_clock_seconds());
        let new_clock = Self::format_clock(state.clock_seconds());
        Self::draw_changed_value(
            display,
            &old_clock,
            &new_clock,
            TIME_ROW.top_left.y + 20,
            TERMINAL_GREEN,
        )
    }

    fn format_battery(state: &WatchState) -> String<16> {
        let mut value = String::new();
        if let Some(status) = state.battery() {
            let power = if status.charging {
                "CHG"
            } else if status.power_present {
                "PWR"
            } else {
                "BAT"
            };
            #[cfg(feature = "diagnostics")]
            let _ = write!(value, "{}mV {}% {power}", status.millivolts, status.percent);
            #[cfg(not(feature = "diagnostics"))]
            let _ = write!(value, "{}% {power}", status.percent);
        } else {
            let _ = value.push_str("---");
        }
        value
    }

    fn draw_battery<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Self::draw_row(
            display,
            BATTERY_ROW,
            "[BATT]",
            &Self::format_battery(state),
            TERMINAL_RED,
        )
    }

    #[cfg(feature = "diagnostics")]
    fn draw_accelerometer<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut value: String<20> = String::new();
        if state.feature_engine() == Some(FeatureEngineStatus::Failed) {
            let _ = value.push_str("FEATURE ERROR");
        } else if let Some(sample) = state.acceleration() {
            let _ = write!(value, "{:+} {:+} {:+}", sample.x, sample.y, sample.z);
        } else {
            match state.accelerometer() {
                Some(AccelerometerKind::Bma421) => {
                    let _ = value.push_str("BMA421");
                }
                Some(AccelerometerKind::Bma425) => {
                    let _ = value.push_str("BMA425");
                }
                Some(AccelerometerKind::Unknown(chip_id)) => {
                    let _ = write!(value, "ID 0x{chip_id:02X}");
                }
                Some(AccelerometerKind::Unavailable) => {
                    let _ = value.push_str("ERROR");
                }
                None => {
                    let _ = value.push_str("---");
                }
            }
        }
        let label = if state.feature_engine() == Some(FeatureEngineStatus::Ready) {
            "[IMU+]"
        } else {
            "[IMU ]"
        };
        Self::draw_row(display, MOTION_ROW, label, &value, TERMINAL_ORANGE)
    }

    /// Padded to a fixed width so partial redraws blank a longer prior value.
    fn format_ble(state: BleState) -> String<16> {
        let mut value = String::new();
        match state {
            BleState::Off => {
                let _ = value.push_str("off        ");
            }
            BleState::Advertising => {
                let _ = value.push_str("advertising");
            }
            BleState::Pairing(passkey) => {
                let _ = write!(value, "PAIR {passkey:06}");
            }
            BleState::Connected => {
                let _ = value.push_str("connected  ");
            }
            BleState::DfuProgress(percent) => {
                let _ = write!(value, "DFU {percent}%");
            }
            BleState::DfuFailed(_) => {
                let _ = value.push_str("DFU failed ");
            }
        }
        value
    }

    fn draw_status<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        // A passkey during pairing takes visual priority.
        let color = if matches!(state.ble(), BleState::Pairing(_)) {
            TERMINAL_ORANGE
        } else {
            TERMINAL_BLUE
        };
        Self::draw_row(
            display,
            STATUS_ROW,
            "[BLE ]",
            &Self::format_ble(state.ble()),
            color,
        )
    }

    fn draw_steps<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut value: String<16> = String::new();
        if let Some(steps) = state.steps() {
            let _ = write!(value, "{steps}");
        } else {
            let _ = value.push_str("---");
        }
        Self::draw_row(display, STEP_ROW, "[STEP]", &value, TERMINAL_ORANGE)
    }

    #[cfg(not(feature = "diagnostics"))]
    const fn category_label(category: NotificationCategory) -> &'static str {
        match category {
            NotificationCategory::Call => "CALL",
            NotificationCategory::MissedCall => "MISSED",
            NotificationCategory::Sms => "SMS",
            NotificationCategory::Email => "EMAIL",
            NotificationCategory::InstantMessage => "IM",
            NotificationCategory::News => "NEWS",
            NotificationCategory::VoiceMail => "VMAIL",
            NotificationCategory::Schedule => "CAL",
            NotificationCategory::HighPriority => "ALERT",
            NotificationCategory::SimpleAlert | NotificationCategory::Other(_) => "MSG",
        }
    }

    /// Shows the session's notification count and the latest category, in the
    /// row the non-diagnostics build otherwise leaves blank. A full per-message
    /// view arrives with the notification screen in the UI redesign.
    #[cfg(not(feature = "diagnostics"))]
    fn draw_notifications<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut value: String<16> = String::new();
        if let Some(category) = state.last_category() {
            let _ = write!(
                value,
                "{} {}",
                state.notifications(),
                Self::category_label(category)
            );
        } else {
            let _ = value.push_str("---");
        }
        Self::draw_row(display, RESERVED_ROW, "[MSG ]", &value, TERMINAL_BLUE)
    }

    #[cfg(feature = "diagnostics")]
    fn format_heart_rate(state: &WatchState) -> String<12> {
        let mut value = String::new();
        match state.heart_rate_analysis() {
            Some(PpgAnalysis::HeartRate { bpm }) => {
                let _ = write!(value, "{bpm} BPM");
                return value;
            }
            Some(PpgAnalysis::AmbientLight) => {
                let _ = value.push_str("AMBIENT");
                return value;
            }
            Some(PpgAnalysis::NoSignal) => {
                let _ = value.push_str("NO SIGNAL");
                return value;
            }
            Some(PpgAnalysis::Collecting { .. }) | None => {}
        }
        if let Some(sample) = state.heart_rate_raw() {
            let _ = write!(value, "{} A{}", sample.hrs, sample.als);
            return value;
        }
        match state.heart_rate_sensor() {
            Some(HeartRateSensorKind::Hrs3300) => {
                let _ = value.push_str("HRS3300");
            }
            Some(HeartRateSensorKind::Unknown(id)) => {
                let _ = write!(value, "HRS 0x{id:02X}");
            }
            Some(HeartRateSensorKind::Unavailable) => {
                let _ = value.push_str("HRS ERROR");
            }
            None => {
                let _ = value.push_str("HRS ---");
            }
        }
        value
    }

    #[cfg(feature = "diagnostics")]
    fn draw_heart_rate<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Self::draw_row(
            display,
            RESERVED_ROW,
            "[HRS ]",
            &Self::format_heart_rate(state),
            TERMINAL_RED,
        )
    }

    #[cfg(not(feature = "diagnostics"))]
    fn draw_heart_rate<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut value: String<16> = String::new();
        match state.heart_rate() {
            HeartRateState::Disabled => {
                let _ = value.push_str("OFF");
            }
            HeartRateState::Starting | HeartRateState::Collecting | HeartRateState::Measuring => {
                let _ = value.push_str("MEASURING");
            }
            HeartRateState::Result(bpm) => {
                let _ = write!(value, "{bpm} BPM");
            }
            HeartRateState::NoSignal => {
                let _ = value.push_str("NO SIGNAL");
            }
            HeartRateState::AmbientLight => {
                let _ = value.push_str("AMBIENT");
            }
            HeartRateState::Error => {
                let _ = value.push_str("ERROR");
            }
        }
        Self::draw_row(display, HEART_RATE_ROW, "[HRT ]", &value, TERMINAL_RED)
    }

    #[cfg(feature = "diagnostics")]
    fn draw_diagnostics_footer<D>(display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        draw_mono_text_visible(
            "swipe >",
            Point::new(0, 226),
            MonoTextStyle::new(&FONT_10X20, TERMINAL_GREEN),
            display,
        )
    }
}

impl Watchface for TerminalWatchface {
    fn draw_full<D>(
        &self,
        state: &WatchState,
        display: &mut D,
        mut keep_alive: impl FnMut(),
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let prompt = MonoTextStyle::new(&FONT_10X20, LIGHT_GRAY);
        // The rows draw in place, so the one blanking pass belongs here: a full
        // redraw follows a modal or a screen change and repaints everything
        // anyway.
        SCREEN_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        draw_mono_text_visible("user@watch:~ $ now", Point::new(0, 20), prompt, display)?;
        keep_alive();

        Self::draw_date(state, display)?;
        keep_alive();
        Self::draw_time(state, display)?;
        keep_alive();
        Self::draw_battery(state, display)?;
        keep_alive();
        #[cfg(feature = "diagnostics")]
        Self::draw_accelerometer(state, display)?;
        Self::draw_steps(state, display)?;
        keep_alive();
        Self::draw_heart_rate(state, display)?;
        keep_alive();
        #[cfg(not(feature = "diagnostics"))]
        Self::draw_notifications(state, display)?;
        Self::draw_status(state, display)?;
        keep_alive();

        #[cfg(feature = "diagnostics")]
        Self::draw_diagnostics_footer(display)?;
        #[cfg(not(feature = "diagnostics"))]
        draw_mono_text_visible("user@watch:~ $", Point::new(0, 226), prompt, display)?;
        keep_alive();
        Ok(())
    }

    fn draw_changed<D>(
        &self,
        state: &WatchState,
        changed: WatchFields,
        display: &mut D,
        mut keep_alive: impl FnMut(),
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        // A tick crossing midnight moves the clock and the date at once, so
        // these are independent tests rather than one choice.
        if changed.contains(WatchField::Date) {
            Self::draw_date(state, display)?;
            keep_alive();
        }
        if changed.contains(WatchField::Clock) {
            Self::update_clock(state, display)?;
            keep_alive();
        }
        if changed.contains(WatchField::Battery) {
            Self::draw_battery(state, display)?;
            keep_alive();
        }
        if changed.contains(WatchField::Steps) {
            Self::draw_steps(state, display)?;
            keep_alive();
        }
        if changed.contains(WatchField::HeartRate) {
            Self::draw_heart_rate(state, display)?;
            keep_alive();
        }
        if changed.contains(WatchField::Ble) {
            // The pairing passkey changes both text and colour, so the whole
            // row is redrawn rather than diffed.
            Self::draw_status(state, display)?;
            keep_alive();
        }
        #[cfg(not(feature = "diagnostics"))]
        if changed.contains(WatchField::Notifications) {
            Self::draw_notifications(state, display)?;
            keep_alive();
        }
        #[cfg(feature = "diagnostics")]
        if changed.contains(WatchField::Motion) {
            Self::draw_accelerometer(state, display)?;
            keep_alive();
        }
        Ok(())
    }
}
