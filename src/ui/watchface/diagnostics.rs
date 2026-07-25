//! A bring-up watchface: raw sensor readings instead of finished values.
//!
//! It shows what the terminal face deliberately hides - millivolts rather than
//! a percentage, acceleration counts, the photoplethysmograph's own numbers -
//! so a sealed watch can still be diagnosed by looking at it.

use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use pineforge_state::{
    AccelerometerKind, FeatureEngineStatus, HeartRateSensorKind, PpgAnalysis, WatchField,
    WatchFields, WatchState,
};

use crate::ui::{
    render::draw_mono_text_visible,
    watchface::{
        Watchface,
        row::{
            self, BATTERY_ROW, DATE_ROW, LIGHT_GRAY, SCREEN_AREA, STATUS_ROW, TERMINAL_GREEN,
            TERMINAL_ORANGE, TERMINAL_RED, TIME_ROW, UNSYNCHRONIZED_DATE, row_at,
        },
    },
};

const MOTION_ROW: Rectangle = row_at(100);
const STEP_ROW: Rectangle = row_at(125);
const HEART_RATE_ROW: Rectangle = row_at(150);

/// Holds no readings of its own; see [`TerminalWatchface`].
///
/// [`TerminalWatchface`]: super::TerminalWatchface
#[derive(Default)]
pub struct DiagnosticsWatchface;

impl DiagnosticsWatchface {
    fn draw_date<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let date = row::format_date(state.date().unwrap_or(UNSYNCHRONIZED_DATE));
        row::draw(display, DATE_ROW, "[DATE]", &date, TERMINAL_GREEN)
    }

    fn draw_time<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let clock = row::format_clock(state.clock_seconds());
        row::draw(display, TIME_ROW, "[TIME]", &clock, TERMINAL_GREEN)
    }

    fn update_clock<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let old_clock = row::format_clock(state.previous_clock_seconds());
        let new_clock = row::format_clock(state.clock_seconds());
        row::draw_changed_value(display, TIME_ROW, &old_clock, &new_clock, TERMINAL_GREEN)
    }

    /// Terminal voltage as well as the estimate derived from it, so a suspect
    /// capacity curve can be checked against the raw reading.
    fn draw_battery<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut value: String<16> = String::new();
        if let Some(status) = state.battery() {
            let power = if status.charging {
                "CHG"
            } else if status.power_present {
                "PWR"
            } else {
                "BAT"
            };
            let _ = write!(value, "{}mV {}% {power}", status.millivolts, status.percent);
        } else {
            let _ = value.push_str("---");
        }
        row::draw(display, BATTERY_ROW, "[BATT]", &value, TERMINAL_RED)
    }

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
        row::draw(display, MOTION_ROW, label, &value, TERMINAL_ORANGE)
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
        row::draw(display, STEP_ROW, "[STEP]", &value, TERMINAL_ORANGE)
    }

    /// Falls back through the analysis, the raw counts, and finally the sensor
    /// identity, so the row says something useful at every bring-up stage.
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

    fn draw_heart_rate<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        row::draw(
            display,
            HEART_RATE_ROW,
            "[HRS ]",
            &Self::format_heart_rate(state),
            TERMINAL_RED,
        )
    }

    fn draw_status<D>(state: &WatchState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        row::draw(
            display,
            STATUS_ROW,
            "[BLE ]",
            &row::format_ble(state.ble()),
            row::ble_color(state.ble()),
        )
    }
}

impl Watchface for DiagnosticsWatchface {
    fn draw_full<D>(
        &self,
        state: &WatchState,
        display: &mut D,
        mut keep_alive: impl FnMut(),
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        SCREEN_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        let prompt = MonoTextStyle::new(&FONT_10X20, LIGHT_GRAY);
        draw_mono_text_visible("user@watch:~ $ now", Point::new(0, 20), prompt, display)?;
        keep_alive();

        Self::draw_date(state, display)?;
        keep_alive();
        Self::draw_time(state, display)?;
        keep_alive();
        Self::draw_battery(state, display)?;
        keep_alive();
        Self::draw_accelerometer(state, display)?;
        keep_alive();
        Self::draw_steps(state, display)?;
        keep_alive();
        Self::draw_heart_rate(state, display)?;
        keep_alive();
        Self::draw_status(state, display)?;
        keep_alive();

        draw_mono_text_visible(
            "swipe >",
            Point::new(0, 226),
            MonoTextStyle::new(&FONT_10X20, TERMINAL_GREEN),
            display,
        )?;
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
        if changed.contains(WatchField::Motion) {
            Self::draw_accelerometer(state, display)?;
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
            Self::draw_status(state, display)?;
            keep_alive();
        }
        Ok(())
    }
}
