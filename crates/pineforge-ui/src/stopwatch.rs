//! A stopwatch in the FORGE instrument language.
//!
//! Five rectangle-built numerals show `MM:SS.T`. The state is anchored to
//! monotonic uptime in `pineforge-state`, so this screen may sleep without a
//! background tick and catches up on the first observation after waking.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::{Alignment, Text},
};
use pineforge_state::{
    AppEvent, Button, ButtonBounds, ButtonOutcome, ButtonState, ScreenAction, StopwatchControl,
    StopwatchState,
};

use crate::canvas::{Canvas, CanvasError};
use crate::font::ui_text;
use crate::{
    render::{
        PANEL, ROW_HEIGHT, ROW_WIDTH, ROW_X, draw_centred, draw_instrument_centred, draw_visible,
        fill, round_corners,
    },
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell},
    theme,
};

const PANEL_WIDTH: i32 = PANEL.size.width.cast_signed();
const PANEL_HEIGHT: i32 = PANEL.size.height.cast_signed();
const TITLE_BASELINE_Y: i32 = 46;

/// `MM:SS.T`, large enough to read as FORGE rather than as menu text.
const DIGIT: SegmentSize = SegmentSize::new(36, 49, 6);
const DIGIT_GAP: i32 = 3;
const COLON_WIDTH: i32 = 6;
const DOT_WIDTH: i32 = 6;
const CLOCK_WIDTH: i32 = 210;
const CLOCK_X: i32 = (PANEL_WIDTH - CLOCK_WIDTH) / 2;
const CLOCK_Y: i32 = 65;
const CLOCK_PLACES: usize = 5;
const CLOCK_X_OF: [i32; CLOCK_PLACES] = [
    CLOCK_X,
    CLOCK_X + DIGIT.width + DIGIT_GAP,
    CLOCK_X + 2 * (DIGIT.width + DIGIT_GAP) + COLON_WIDTH + DIGIT_GAP,
    CLOCK_X + 3 * (DIGIT.width + DIGIT_GAP) + COLON_WIDTH + DIGIT_GAP,
    CLOCK_X + 4 * (DIGIT.width + DIGIT_GAP) + COLON_WIDTH + DOT_WIDTH + 2 * DIGIT_GAP,
];
const COLON_X: i32 = CLOCK_X + 2 * (DIGIT.width + DIGIT_GAP);
const DOT_X: i32 = CLOCK_X + 4 * (DIGIT.width + DIGIT_GAP) + COLON_WIDTH + DIGIT_GAP;

const STATE_BASELINE_Y: i32 = 145;
const CONTROLS_Y: i32 = 158;
const CONTROL_GAP: i32 = 10;
const CONTROL_WIDTH: i32 = (ROW_WIDTH - CONTROL_GAP) / 2;
const HINT_BASELINE_Y: i32 = 226;

const RESET: usize = 0;
const PRIMARY: usize = 1;
const CONTROL_COUNT: usize = 2;

const fn control_bounds(slot: usize) -> ButtonBounds {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let index = slot as i32;
    ButtonBounds::new(
        ROW_X + index * (CONTROL_WIDTH + CONTROL_GAP),
        CONTROLS_Y,
        CONTROL_WIDTH,
        ROW_HEIGHT,
    )
}

#[derive(Clone, Copy, Default)]
struct Pending {
    clock: bool,
    state: bool,
    controls: [bool; CONTROL_COUNT],
    everything: bool,
}

impl Pending {
    const fn is_clean(self) -> bool {
        !self.clock
            && !self.state
            && !self.controls[RESET]
            && !self.controls[PRIMARY]
            && !self.everything
    }
}

#[derive(Clone, Copy)]
struct Shown {
    cells: [Cell; CLOCK_PLACES],
    ink: Rgb565,
}

pub struct StopwatchScreen {
    stopwatch: StopwatchState,
    controls: [Button; CONTROL_COUNT],
    now_millis: u64,
    pending: Pending,
    shown: Shown,
}

impl StopwatchScreen {
    #[must_use]
    pub fn new() -> Self {
        let mut reset = Button::new(control_bounds(RESET));
        let _ = reset.set_enabled(false);
        Self {
            stopwatch: StopwatchState::new(),
            controls: [reset, Button::new(control_bounds(PRIMARY))],
            now_millis: 0,
            pending: Pending::default(),
            shown: Shown {
                cells: [Cell::Blank; CLOCK_PLACES],
                ink: theme::BACKGROUND,
            },
        }
    }

    #[must_use]
    pub const fn moved(&self) -> bool {
        !self.pending.is_clean()
    }

    pub fn mark_painted(&mut self) {
        self.shown = Shown {
            cells: self.cells(),
            ink: self.clock_ink(),
        };
        self.pending = Pending::default();
    }

    /// Applies a control with the display task's precise monotonic timestamp.
    pub fn control(&mut self, control: StopwatchControl, now_millis: u64) {
        self.now_millis = now_millis;
        if !self.stopwatch.apply(control, now_millis) {
            return;
        }
        self.pending.clock = true;
        self.pending.state = true;
        self.pending.controls = [true; CONTROL_COUNT];
        let reset_enabled = self.stopwatch.can_reset();
        let _ = self.controls[RESET].set_enabled(reset_enabled);
    }

    fn cells(&self) -> [Cell; CLOCK_PLACES] {
        // The layout has two minute places. Saturating makes the limit visible
        // instead of wrapping a long run back to zero.
        let tenths = (self.stopwatch.elapsed_millis(self.now_millis) / 100).min(59_999);
        let minutes = tenths / 600;
        let seconds = tenths / 10 % 60;
        [
            Cell::Digit(decimal(minutes / 10)),
            Cell::Digit(decimal(minutes)),
            Cell::Digit(decimal(seconds / 10)),
            Cell::Digit(decimal(seconds)),
            Cell::Digit(decimal(tenths)),
        ]
    }

    const fn clock_ink(&self) -> Rgb565 {
        if self.stopwatch.is_running() {
            theme::ACCENT
        } else {
            theme::TEXT
        }
    }

    const fn state_text(&self) -> &'static str {
        if self.stopwatch.is_running() {
            "RUNNING"
        } else if self.stopwatch.can_reset() {
            "PAUSED"
        } else {
            "READY"
        }
    }

    const fn primary_text(&self) -> &'static str {
        if self.stopwatch.is_running() {
            "PAUSE"
        } else if self.stopwatch.can_reset() {
            "RESUME"
        } else {
            "START"
        }
    }

    fn draw_separator(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let ink = self.clock_ink();
        fill(
            canvas,
            COLON_X,
            CLOCK_Y,
            COLON_WIDTH,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        let colon_x = COLON_X + (COLON_WIDTH - DIGIT.stroke) / 2;
        for third in 1..=2 {
            let y = CLOCK_Y + DIGIT.height * third / 3 - DIGIT.stroke / 2;
            fill(canvas, colon_x, y, DIGIT.stroke, DIGIT.stroke, ink)?;
        }
        fill(
            canvas,
            DOT_X,
            CLOCK_Y,
            DOT_WIDTH,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        fill(
            canvas,
            DOT_X,
            CLOCK_Y + DIGIT.height - DIGIT.stroke,
            DOT_WIDTH,
            DIGIT.stroke,
            ink,
        )
    }

    fn draw_clock(
        &self,
        full: bool,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let cells = self.cells();
        let ink = self.clock_ink();
        for (place, cell) in cells.into_iter().enumerate() {
            if full || cell != self.shown.cells[place] || ink != self.shown.ink {
                draw_cell(canvas, DIGIT, CLOCK_X_OF[place], CLOCK_Y, cell, ink)?;
                keep_alive();
            }
        }
        if full || ink != self.shown.ink {
            self.draw_separator(canvas)?;
        }
        Ok(())
    }

    fn draw_state(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(canvas, 0, 115, PANEL_WIDTH, 36, theme::BACKGROUND)?;
        draw_instrument_centred(self.state_text(), STATE_BASELINE_Y, theme::ACCENT, canvas)
    }

    fn draw_control(&self, slot: usize, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let bounds = control_bounds(slot);
        let area = Rectangle::new(
            Point::new(bounds.x(), bounds.y()),
            Size::new(
                bounds.width().unsigned_abs(),
                bounds.height().unsigned_abs(),
            ),
        );
        let (face, ink) = match self.controls[slot].state() {
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
        let text = if slot == RESET {
            "RESET"
        } else {
            self.primary_text()
        };
        draw_visible(
            &Text::with_alignment(
                text,
                Point::new(bounds.x() + bounds.width() / 2, bounds.y() + 27),
                ui_text(ink, face),
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
        fill(canvas, 0, 0, PANEL_WIDTH, PANEL_HEIGHT, theme::BACKGROUND)?;
        keep_alive();
        draw_instrument_centred("STOPWATCH", TITLE_BASELINE_Y, theme::ACCENT, canvas)?;
        self.draw_clock(true, canvas, keep_alive)?;
        self.draw_state(canvas)?;
        for slot in 0..CONTROL_COUNT {
            self.draw_control(slot, canvas)?;
            keep_alive();
        }
        draw_centred("TAP A CONTROL", HINT_BASELINE_Y, theme::TEXT, canvas)
    }
}

impl Default for StopwatchScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Paint for StopwatchScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas, keep_alive)
    }
}

impl Screen for StopwatchScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        if let AppEvent::StopwatchTick(now_millis) = event {
            let before = self.cells();
            self.now_millis = now_millis;
            if self.stopwatch.is_running() && self.cells() != before {
                self.pending.clock = true;
            }
            return ScreenAction::None;
        }

        for slot in 0..CONTROL_COUNT {
            match self.controls[slot].handle_event(event) {
                ButtonOutcome::Activated => {
                    self.pending.controls[slot] = true;
                    let control = if slot == RESET {
                        StopwatchControl::Reset
                    } else if self.stopwatch.is_running() {
                        StopwatchControl::Pause
                    } else {
                        StopwatchControl::Start
                    };
                    return ScreenAction::StopwatchControl(control);
                }
                ButtonOutcome::Redraw => {
                    self.pending.controls[slot] = true;
                    return ScreenAction::None;
                }
                ButtonOutcome::None => {}
            }
        }
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.pending.everything {
            return self.paint(canvas, keep_alive);
        }
        if self.pending.clock {
            self.draw_clock(false, canvas, keep_alive)?;
        }
        if self.pending.state {
            self.draw_state(canvas)?;
        }
        for slot in 0..CONTROL_COUNT {
            if self.pending.controls[slot] {
                self.draw_control(slot, canvas)?;
            }
        }
        Ok(())
    }
}

#[allow(clippy::cast_possible_truncation)]
const fn decimal(value: u64) -> u8 {
    (value % 10) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    fn touch(slot: usize, pressed: bool) -> AppEvent {
        let bounds = control_bounds(slot);
        AppEvent::Touch {
            x: bounds.x() + bounds.width() / 2,
            y: bounds.y() + bounds.height() / 2,
            pressed,
        }
    }

    fn tap(screen: &mut StopwatchScreen, slot: usize) -> ScreenAction {
        let _ = screen.handle_event(touch(slot, true));
        screen.handle_event(touch(slot, false))
    }

    fn repaint(screen: &StopwatchScreen) -> Probe {
        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    #[test]
    fn primary_control_cycles_start_pause_and_resume() {
        let mut screen = StopwatchScreen::new();
        assert_eq!(
            tap(&mut screen, PRIMARY),
            ScreenAction::StopwatchControl(StopwatchControl::Start)
        );
        screen.control(StopwatchControl::Start, 1_000);
        assert_eq!(
            tap(&mut screen, PRIMARY),
            ScreenAction::StopwatchControl(StopwatchControl::Pause)
        );
        screen.control(StopwatchControl::Pause, 2_500);
        assert_eq!(
            tap(&mut screen, PRIMARY),
            ScreenAction::StopwatchControl(StopwatchControl::Start)
        );
    }

    #[test]
    fn reset_is_disabled_until_a_run_is_paused() {
        let mut screen = StopwatchScreen::new();
        assert_eq!(tap(&mut screen, RESET), ScreenAction::None);
        screen.control(StopwatchControl::Start, 0);
        assert_eq!(tap(&mut screen, RESET), ScreenAction::None);
        screen.control(StopwatchControl::Pause, 1_000);
        assert_eq!(
            tap(&mut screen, RESET),
            ScreenAction::StopwatchControl(StopwatchControl::Reset)
        );
    }

    #[test]
    fn the_display_is_minutes_seconds_and_tenths() {
        let mut screen = StopwatchScreen::new();
        screen.control(StopwatchControl::Start, 1_000);
        let _ = screen.handle_event(AppEvent::StopwatchTick(754_490));
        assert_eq!(
            screen.cells(),
            [
                Cell::Digit(1),
                Cell::Digit(2),
                Cell::Digit(3),
                Cell::Digit(3),
                Cell::Digit(4)
            ]
        );
    }

    #[test]
    fn one_tenth_repaints_less_than_the_panel() {
        let mut screen = StopwatchScreen::new();
        screen.control(StopwatchControl::Start, 0);
        screen.mark_painted();
        let _ = screen.handle_event(AppEvent::StopwatchTick(100));
        let painted = PANEL_WIDTH as usize * PANEL_HEIGHT as usize - repaint(&screen).unpainted();
        assert!(painted > 0);
        assert!(painted <= (DIGIT.width * DIGIT.height) as usize);
    }
}
