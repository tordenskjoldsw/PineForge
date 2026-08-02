//! A countdown timer in the FORGE instrument language.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::{Alignment, Text},
};
use pineforge_state::{
    AppEvent, Button, ButtonBounds, ButtonOutcome, ButtonState, ScreenAction, TIMER_MAX_MINUTES,
    TIMER_MIN_MINUTES, TimerControl, TimerOutcome, TimerPhase, TimerState,
};

use crate::canvas::{Canvas, CanvasError};
use crate::font::ui_text;
use crate::{
    render::{PANEL, draw_centred, draw_instrument_centred, draw_visible, fill, round_corners},
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell},
    theme,
};

const PANEL_WIDTH: i32 = PANEL.size.width.cast_signed();
const PANEL_HEIGHT: i32 = PANEL.size.height.cast_signed();
const TITLE_BASELINE_Y: i32 = 46;

const DIGIT: SegmentSize = SegmentSize::new(42, 58, 7);
const DIGIT_GAP: i32 = 4;
const COLON_WIDTH: i32 = 8;
const CLOCK_WIDTH: i32 = 4 * DIGIT.width + 3 * DIGIT_GAP + COLON_WIDTH;
const CLOCK_X: i32 = (PANEL_WIDTH - CLOCK_WIDTH) / 2;
const CLOCK_Y: i32 = 61;
const CLOCK_PLACES: usize = 4;
const COLON_X: i32 = CLOCK_X + 2 * (DIGIT.width + DIGIT_GAP);
const CLOCK_X_OF: [i32; CLOCK_PLACES] = [
    CLOCK_X,
    CLOCK_X + DIGIT.width + DIGIT_GAP,
    COLON_X + COLON_WIDTH + DIGIT_GAP,
    COLON_X + COLON_WIDTH + 2 * DIGIT_GAP + DIGIT.width,
];

const STATE_BASELINE_Y: i32 = 145;
const CONTROL_X: i32 = 20;
const CONTROL_GAP: i32 = 10;
const CONTROL_WIDTH: i32 = 95;
const ADJUST_Y: i32 = 151;
const ADJUST_HEIGHT: i32 = 34;
const ACTION_Y: i32 = 190;
const ACTION_HEIGHT: i32 = 40;

const DECREASE: usize = 0;
const INCREASE: usize = 1;
const RESET: usize = 2;
const PRIMARY: usize = 3;
const CONTROL_COUNT: usize = 4;

const fn control_bounds(slot: usize) -> ButtonBounds {
    let right = matches!(slot, INCREASE | PRIMARY);
    let x = if right {
        CONTROL_X + CONTROL_WIDTH + CONTROL_GAP
    } else {
        CONTROL_X
    };
    let (y, height) = if matches!(slot, DECREASE | INCREASE) {
        (ADJUST_Y, ADJUST_HEIGHT)
    } else {
        (ACTION_Y, ACTION_HEIGHT)
    };
    ButtonBounds::new(x, y, CONTROL_WIDTH, height)
}

#[derive(Clone, Copy, Default)]
struct Pending {
    clock: bool,
    state: bool,
    controls: [bool; CONTROL_COUNT],
}

impl Pending {
    const fn is_clean(self) -> bool {
        !self.clock && !self.state && !any(self.controls)
    }
}

const fn any(values: [bool; CONTROL_COUNT]) -> bool {
    values[0] || values[1] || values[2] || values[3]
}

#[derive(Clone, Copy)]
struct Shown {
    cells: [Cell; CLOCK_PLACES],
    ink: Rgb565,
}

pub struct TimerScreen {
    timer: TimerState,
    controls: [Button; CONTROL_COUNT],
    now_millis: u64,
    pending: Pending,
    shown: Shown,
}

impl TimerScreen {
    #[must_use]
    pub fn new() -> Self {
        let mut screen = Self {
            timer: TimerState::new(),
            controls: [
                Button::new(control_bounds(DECREASE)),
                Button::new(control_bounds(INCREASE)),
                Button::new(control_bounds(RESET)),
                Button::new(control_bounds(PRIMARY)),
            ],
            now_millis: 0,
            pending: Pending::default(),
            shown: Shown {
                cells: [Cell::Blank; CLOCK_PLACES],
                ink: theme::BACKGROUND,
            },
        };
        screen.sync_controls();
        screen
    }

    #[must_use]
    pub const fn deadline_millis(&self) -> Option<u64> {
        self.timer.deadline_millis()
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

    /// Applies a control at the display task's one precise monotonic reading.
    pub fn control(&mut self, control: TimerControl, now_millis: u64) -> TimerOutcome {
        self.now_millis = now_millis;
        let outcome = self.timer.apply(control, now_millis);
        if !matches!(outcome, TimerOutcome::Unchanged) {
            self.pending.clock = true;
            self.pending.state = true;
            self.pending.controls = [true; CONTROL_COUNT];
            self.sync_controls();
        }
        outcome
    }

    /// Observes time whether or not this screen is currently visible.
    pub fn observe(&mut self, now_millis: u64) -> TimerOutcome {
        let before_cells = self.cells();
        let before_phase = self.timer.phase();
        self.now_millis = now_millis;
        let outcome = self.timer.observe(now_millis);
        if self.cells() != before_cells {
            self.pending.clock = true;
        }
        if self.timer.phase() != before_phase {
            self.pending.clock = true;
            self.pending.state = true;
            self.pending.controls = [true; CONTROL_COUNT];
            self.sync_controls();
        }
        outcome
    }

    fn sync_controls(&mut self) {
        let phase = self.timer.phase();
        let ready = matches!(phase, TimerPhase::Ready);
        let _ = self.controls[DECREASE]
            .set_enabled(ready && self.timer.selected_minutes() > TIMER_MIN_MINUTES);
        let _ = self.controls[INCREASE]
            .set_enabled(ready && self.timer.selected_minutes() < TIMER_MAX_MINUTES);
        let _ = self.controls[RESET].set_enabled(!ready);
        let _ = self.controls[PRIMARY].set_enabled(!matches!(phase, TimerPhase::Expired));
    }

    fn cells(&self) -> [Cell; CLOCK_PLACES] {
        let seconds = self
            .timer
            .remaining_seconds(self.now_millis)
            .min(99 * 60 + 59);
        let minutes = seconds / 60;
        let seconds = seconds % 60;
        [
            Cell::Digit(decimal(minutes / 10)),
            Cell::Digit(decimal(minutes)),
            Cell::Digit(decimal(seconds / 10)),
            Cell::Digit(decimal(seconds)),
        ]
    }

    const fn clock_ink(&self) -> Rgb565 {
        match self.timer.phase() {
            TimerPhase::Running => theme::ACCENT,
            TimerPhase::Expired => theme::DANGER,
            TimerPhase::Ready | TimerPhase::Paused => theme::TEXT,
        }
    }

    const fn label_ink(&self) -> Rgb565 {
        if matches!(self.timer.phase(), TimerPhase::Expired) {
            theme::DANGER
        } else {
            theme::ACCENT
        }
    }

    const fn state_text(&self) -> &'static str {
        match self.timer.phase() {
            TimerPhase::Ready => "READY",
            TimerPhase::Running => "RUNNING",
            TimerPhase::Paused => "PAUSED",
            TimerPhase::Expired => "DONE",
        }
    }

    const fn control_text(&self, slot: usize) -> &'static str {
        match slot {
            DECREASE => "-1 MIN",
            INCREASE => "+1 MIN",
            RESET if matches!(self.timer.phase(), TimerPhase::Running) => "CANCEL",
            RESET => "RESET",
            PRIMARY if matches!(self.timer.phase(), TimerPhase::Running) => "PAUSE",
            PRIMARY if matches!(self.timer.phase(), TimerPhase::Paused) => "RESUME",
            PRIMARY if matches!(self.timer.phase(), TimerPhase::Expired) => "DONE",
            PRIMARY => "START",
            _ => "",
        }
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
            draw_colon(canvas, ink)?;
        }
        Ok(())
    }

    fn draw_state(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(canvas, 0, 121, PANEL_WIDTH, 28, theme::BACKGROUND)?;
        draw_instrument_centred(
            self.state_text(),
            STATE_BASELINE_Y,
            self.label_ink(),
            canvas,
        )
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
        draw_visible(
            &Text::with_alignment(
                self.control_text(slot),
                Point::new(
                    bounds.x() + bounds.width() / 2,
                    bounds.y() + bounds.height() / 2 + 7,
                ),
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
        draw_instrument_centred("TIMER", TITLE_BASELINE_Y, theme::ACCENT, canvas)?;
        self.draw_clock(true, canvas, keep_alive)?;
        self.draw_state(canvas)?;
        for slot in 0..CONTROL_COUNT {
            self.draw_control(slot, canvas)?;
        }
        Ok(())
    }
}

impl Default for TimerScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Paint for TimerScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas, keep_alive)
    }
}

impl Screen for TimerScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        if let AppEvent::TimerTick(now_millis) = event {
            let _ = self.observe(now_millis);
            return ScreenAction::None;
        }

        for slot in 0..CONTROL_COUNT {
            match self.controls[slot].handle_event(event) {
                ButtonOutcome::Activated => {
                    self.pending.controls[slot] = true;
                    let control = match slot {
                        DECREASE => TimerControl::Decrease,
                        INCREASE => TimerControl::Increase,
                        RESET => TimerControl::Reset,
                        PRIMARY if matches!(self.timer.phase(), TimerPhase::Running) => {
                            TimerControl::Pause
                        }
                        PRIMARY => TimerControl::Start,
                        _ => return ScreenAction::None,
                    };
                    return ScreenAction::TimerControl(control);
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

fn draw_colon(canvas: &mut Canvas<'_>, ink: Rgb565) -> Result<(), CanvasError> {
    fill(
        canvas,
        COLON_X,
        CLOCK_Y,
        COLON_WIDTH,
        DIGIT.height,
        theme::BACKGROUND,
    )?;
    let x = COLON_X + (COLON_WIDTH - DIGIT.stroke) / 2;
    for third in 1..=2 {
        let y = CLOCK_Y + DIGIT.height * third / 3 - DIGIT.stroke / 2;
        fill(canvas, x, y, DIGIT.stroke, DIGIT.stroke, ink)?;
    }
    Ok(())
}

/// Full-screen timer alarm shown above whichever application was open.
pub fn draw_expired_modal(
    canvas: &mut Canvas<'_>,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    fill(canvas, 0, 0, PANEL_WIDTH, PANEL_HEIGHT, theme::BACKGROUND)?;
    keep_alive();
    draw_instrument_centred("TIMER DONE", 60, theme::DANGER, canvas)?;
    let cells = [Cell::Digit(0); CLOCK_PLACES];
    for (place, cell) in cells.into_iter().enumerate() {
        draw_cell(canvas, DIGIT, CLOCK_X_OF[place], 83, cell, theme::DANGER)?;
    }
    // The modal clock is lower than the screen clock, so its separator is
    // drawn directly rather than borrowing the screen's fixed coordinate.
    let x = COLON_X + (COLON_WIDTH - DIGIT.stroke) / 2;
    for third in 1..=2 {
        let y = 83 + DIGIT.height * third / 3 - DIGIT.stroke / 2;
        fill(canvas, x, y, DIGIT.stroke, DIGIT.stroke, theme::DANGER)?;
    }
    draw_centred("TAP OR PRESS TO DISMISS", 205, theme::TEXT, canvas)
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

    fn tap(screen: &mut TimerScreen, slot: usize) -> ScreenAction {
        let _ = screen.handle_event(touch(slot, true));
        screen.handle_event(touch(slot, false))
    }

    #[test]
    fn ready_controls_adjust_and_start() {
        let mut screen = TimerScreen::new();
        assert_eq!(
            tap(&mut screen, INCREASE),
            ScreenAction::TimerControl(TimerControl::Increase)
        );
        let _ = screen.control(TimerControl::Increase, 0);
        assert_eq!(
            screen.cells(),
            [
                Cell::Digit(0),
                Cell::Digit(6),
                Cell::Digit(0),
                Cell::Digit(0)
            ]
        );
        assert_eq!(
            tap(&mut screen, PRIMARY),
            ScreenAction::TimerControl(TimerControl::Start)
        );
    }

    #[test]
    fn running_controls_pause_and_cancel() {
        let mut screen = TimerScreen::new();
        let _ = screen.control(TimerControl::Start, 0);
        assert_eq!(
            tap(&mut screen, PRIMARY),
            ScreenAction::TimerControl(TimerControl::Pause)
        );
        let _ = screen.control(TimerControl::Pause, 1_000);
        assert_eq!(
            tap(&mut screen, PRIMARY),
            ScreenAction::TimerControl(TimerControl::Start)
        );
        assert_eq!(
            tap(&mut screen, RESET),
            ScreenAction::TimerControl(TimerControl::Reset)
        );
    }

    #[test]
    fn expiry_recolours_and_disables_start() {
        let mut screen = TimerScreen::new();
        let _ = screen.control(TimerControl::Start, 0);
        assert_eq!(screen.observe(300_000), TimerOutcome::Expired);
        assert_eq!(screen.clock_ink(), theme::DANGER);
        assert_eq!(tap(&mut screen, PRIMARY), ScreenAction::None);
        assert_eq!(
            tap(&mut screen, RESET),
            ScreenAction::TimerControl(TimerControl::Reset)
        );
    }

    #[test]
    fn a_second_repaints_less_than_the_panel() {
        let mut screen = TimerScreen::new();
        let _ = screen.control(TimerControl::Start, 0);
        screen.mark_painted();
        let _ = screen.observe(1_000);
        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        let painted = PANEL_WIDTH as usize * PANEL_HEIGHT as usize - probe.unpainted();
        assert!(painted > 0);
        // 05:00 -> 04:59 carries through three places, but still leaves the
        // title, state, controls and the unchanged leading digit untouched.
        assert!(painted <= (3 * DIGIT.width * DIGIT.height) as usize);
    }
}
