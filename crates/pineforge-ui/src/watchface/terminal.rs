//! A terminal-styled watchface: one labelled row per reading.

use core::fmt::Write;

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use pineforge_state::{HeartRateState, WatchField, WatchFields, WatchState};

use crate::canvas::{Canvas, CanvasError};
use crate::font::ui_text;
use crate::{
    render::draw_mono_text_visible,
    watchface::{
        Watchface,
        row::{
            self, BATTERY_ROW, DATE_ROW, LIGHT_GRAY, SCREEN_AREA, TERMINAL_GREEN, TERMINAL_ORANGE,
            TERMINAL_RED, TIME_ROW, UNSYNCHRONIZED_DATE, row_at,
        },
    },
};

const STEP_ROW: Rectangle = row_at(100);
const HEART_RATE_ROW: Rectangle = row_at(125);
/// No notification row: what is pending is read on the notification screen, and
/// a count on the face would only be a second place to keep it right.
const STATUS_ROW: Rectangle = row_at(150);

/// Holds no readings of its own: everything it shows comes from the shared
/// [`WatchState`], so it is a layout and nothing else.
#[derive(Default)]
pub struct TerminalWatchface;

impl TerminalWatchface {
    /// The date of the last synchronization, or a fixed stand-in before the
    /// first one arrives.
    fn draw_date(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let date = row::format_date(state.date().unwrap_or(UNSYNCHRONIZED_DATE));
        row::draw(canvas, DATE_ROW, "[DATE]", &date, TERMINAL_GREEN)
    }

    fn draw_time(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let clock = row::format_clock(state.clock_seconds());
        row::draw(canvas, TIME_ROW, "[TIME]", &clock, TERMINAL_GREEN)
    }

    fn update_clock(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let old_clock = row::format_clock(state.previous_clock_seconds());
        let new_clock = row::format_clock(state.clock_seconds());
        row::draw_changed_value(canvas, TIME_ROW, &old_clock, &new_clock, TERMINAL_GREEN)
    }

    fn draw_battery(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let mut value: String<16> = String::new();
        if let Some(status) = state.battery() {
            let _ = write!(value, "{}% {}", status.percent, status.source().label());
        } else {
            let _ = value.push_str("---");
        }
        row::draw(canvas, BATTERY_ROW, "[BATT]", &value, TERMINAL_RED)
    }

    fn draw_steps(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let mut value: String<16> = String::new();
        if let Some(steps) = state.steps() {
            let _ = write!(value, "{steps}");
        } else {
            let _ = value.push_str("---");
        }
        row::draw(canvas, STEP_ROW, "[STEP]", &value, TERMINAL_ORANGE)
    }

    fn draw_heart_rate(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
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
        row::draw(canvas, HEART_RATE_ROW, "[HRT ]", &value, TERMINAL_RED)
    }

    fn draw_status(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        row::draw(
            canvas,
            STATUS_ROW,
            "[BLE ]",
            &row::format_ble(state.ble()),
            row::ble_color(state.ble()),
        )
    }
}

impl Watchface for TerminalWatchface {
    fn draw_full(
        &self,
        state: &WatchState,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        // The rows draw in place, so the one blanking pass belongs here: a full
        // redraw follows a modal or a screen change and repaints everything
        // anyway.
        SCREEN_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(canvas)?;
        let prompt = ui_text(LIGHT_GRAY, Rgb565::BLACK);
        draw_mono_text_visible("user@watch:~ $ now", Point::new(0, 20), prompt, canvas)?;
        keep_alive();

        Self::draw_date(state, canvas)?;
        keep_alive();
        Self::draw_time(state, canvas)?;
        keep_alive();
        Self::draw_battery(state, canvas)?;
        keep_alive();
        Self::draw_steps(state, canvas)?;
        keep_alive();
        Self::draw_heart_rate(state, canvas)?;
        keep_alive();
        Self::draw_status(state, canvas)?;
        keep_alive();

        draw_mono_text_visible("user@watch:~ $", Point::new(0, 226), prompt, canvas)?;
        keep_alive();
        Ok(())
    }

    fn draw_changed(
        &self,
        state: &WatchState,
        changed: WatchFields,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        // A tick crossing midnight moves the clock and the date at once, so
        // these are independent tests rather than one choice.
        if changed.contains(WatchField::Date) {
            Self::draw_date(state, canvas)?;
            keep_alive();
        }
        if changed.contains(WatchField::Clock) {
            Self::update_clock(state, canvas)?;
            keep_alive();
        }
        if changed.contains(WatchField::Battery) {
            Self::draw_battery(state, canvas)?;
            keep_alive();
        }
        if changed.contains(WatchField::Steps) {
            Self::draw_steps(state, canvas)?;
            keep_alive();
        }
        if changed.contains(WatchField::HeartRate) {
            Self::draw_heart_rate(state, canvas)?;
            keep_alive();
        }
        // Nothing for `WatchField::Notifications`: this face shows no tally, so
        // an arrival costs it no redraw at all.
        if changed.contains(WatchField::Ble) {
            // The pairing passkey changes both text and colour, so the whole
            // row is redrawn rather than diffed.
            Self::draw_status(state, canvas)?;
            keep_alive();
        }
        Ok(())
    }
}
