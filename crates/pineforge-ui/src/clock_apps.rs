//! Manual TIME and DATE editors over one shared wall clock.

use embedded_graphics::{
    prelude::*,
    primitives::Rectangle,
    text::{Alignment, Text},
};
use pineforge_state::{
    AppEvent, Button, ButtonBounds, ButtonOutcome, CalendarDate, DateEditor, DateField,
    ScreenAction, TimeEditor, TimeField, WallTime,
};

use crate::{
    canvas::{Canvas, CanvasError},
    font::ui_text,
    render::{PANEL, draw_instrument_centred, draw_visible, fill, round_corners},
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell},
    theme,
};

const PANEL_WIDTH: i32 = PANEL.size.width.cast_signed();
const PANEL_HEIGHT: i32 = PANEL.size.height.cast_signed();
const TITLE_BASELINE: i32 = 46;
const FIELD_BASELINE: i32 = 145;

const CONTROL_X: i32 = 20;
const CONTROL_GAP: i32 = 10;
const CONTROL_WIDTH: i32 = 95;
const ADJUST_Y: i32 = 158;
const ADJUST_HEIGHT: i32 = 34;
const ACTION_Y: i32 = 198;
const ACTION_HEIGHT: i32 = 40;

const DECREASE: usize = 0;
const INCREASE: usize = 1;
const NEXT: usize = 2;
const APPLY: usize = 3;
const CONTROL_COUNT: usize = 4;

const TIME_DIGIT: SegmentSize = SegmentSize::new(42, 58, 7);
const TIME_GAP: i32 = 4;
const TIME_COLON_WIDTH: i32 = 8;
const TIME_WIDTH: i32 = 4 * TIME_DIGIT.width + 3 * TIME_GAP + TIME_COLON_WIDTH;
const TIME_X: i32 = (PANEL_WIDTH - TIME_WIDTH) / 2;
const TIME_Y: i32 = 64;
const TIME_COLON_X: i32 = TIME_X + 2 * (TIME_DIGIT.width + TIME_GAP);
const TIME_X_OF: [i32; 4] = [
    TIME_X,
    TIME_X + TIME_DIGIT.width + TIME_GAP,
    TIME_COLON_X + TIME_COLON_WIDTH + TIME_GAP,
    TIME_COLON_X + TIME_COLON_WIDTH + 2 * TIME_GAP + TIME_DIGIT.width,
];

const DATE_DIGIT: SegmentSize = SegmentSize::new(20, 32, 4);
const DATE_Y: i32 = 72;
const DATE_X_OF: [i32; 8] = [19, 41, 63, 85, 121, 143, 179, 201];
const DATE_DASH_X: [i32; 2] = [109, 167];

const fn control_bounds(slot: usize) -> ButtonBounds {
    let right = matches!(slot, INCREASE | APPLY);
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
    value: bool,
    field: bool,
    controls: [bool; CONTROL_COUNT],
}

impl Pending {
    const fn all() -> Self {
        Self {
            value: true,
            field: true,
            controls: [true; CONTROL_COUNT],
        }
    }

    const fn clean(self) -> bool {
        !self.value && !self.field && !any(self.controls)
    }
}

const fn any(values: [bool; CONTROL_COUNT]) -> bool {
    let mut index = 0;
    while index < values.len() {
        if values[index] {
            return true;
        }
        index += 1;
    }
    false
}

struct Controls {
    buttons: [Button; CONTROL_COUNT],
}

impl Controls {
    const fn new() -> Self {
        Self {
            buttons: [
                Button::new(control_bounds(DECREASE)),
                Button::new(control_bounds(INCREASE)),
                Button::new(control_bounds(NEXT)),
                Button::new(control_bounds(APPLY)),
            ],
        }
    }

    fn handle(&mut self, event: AppEvent, pending: &mut Pending) -> Option<usize> {
        for slot in 0..CONTROL_COUNT {
            match self.buttons[slot].handle_event(event) {
                ButtonOutcome::Activated => {
                    pending.controls[slot] = true;
                    return Some(slot);
                }
                ButtonOutcome::Redraw => {
                    pending.controls[slot] = true;
                    return None;
                }
                ButtonOutcome::None => {}
            }
        }
        None
    }

    fn draw(&self, slot: usize, label: &str, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let bounds = control_bounds(slot);
        let area = Rectangle::new(
            Point::new(bounds.x(), bounds.y()),
            Size::new(
                bounds.width().unsigned_abs(),
                bounds.height().unsigned_abs(),
            ),
        );
        let pressed = self.buttons[slot].state() == pineforge_state::ButtonState::Pressed;
        let face = if pressed {
            theme::FRAME
        } else {
            theme::SURFACE
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
                label,
                Point::new(
                    bounds.x() + bounds.width() / 2,
                    bounds.y() + bounds.height() / 2 + 7,
                ),
                ui_text(theme::ACCENT, face),
                Alignment::Center,
            ),
            canvas,
        )
    }
}

pub struct TimeScreen {
    editor: TimeEditor,
    shown: TimeEditor,
    controls: Controls,
    pending: Pending,
}

impl TimeScreen {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            editor: TimeEditor::new(WallTime::MIDNIGHT),
            shown: TimeEditor::new(WallTime::MIDNIGHT),
            controls: Controls::new(),
            pending: Pending::all(),
        }
    }

    pub const fn open(&mut self, current: WallTime) {
        self.editor = TimeEditor::new(current);
        self.pending = Pending::all();
    }

    pub fn mark_painted(&mut self) {
        self.shown = self.editor;
        self.pending = Pending::default();
    }

    #[must_use]
    pub const fn moved(&self) -> bool {
        !self.pending.clean()
    }

    const fn field_name(&self) -> &'static str {
        match self.editor.field() {
            TimeField::Hour => "HOUR",
            TimeField::Minute => "MINUTE",
        }
    }

    const fn control_label(&self, slot: usize) -> &'static str {
        match slot {
            DECREASE => match self.editor.field() {
                TimeField::Hour => "- HOUR",
                TimeField::Minute => "- MIN",
            },
            INCREASE => match self.editor.field() {
                TimeField::Hour => "+ HOUR",
                TimeField::Minute => "+ MIN",
            },
            NEXT => "NEXT",
            APPLY => "APPLY",
            _ => "",
        }
    }

    const fn digits(editor: TimeEditor) -> [u8; 4] {
        let value = editor.value();
        [
            value.hour / 10,
            value.hour % 10,
            value.minute / 10,
            value.minute % 10,
        ]
    }

    const fn selected(editor: TimeEditor, place: usize) -> bool {
        match editor.field() {
            TimeField::Hour => place < 2,
            TimeField::Minute => place >= 2,
        }
    }

    fn draw_value(&self, full: bool, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let digits = Self::digits(self.editor);
        let shown = Self::digits(self.shown);
        for (place, digit) in digits.into_iter().enumerate() {
            let selected = Self::selected(self.editor, place);
            if !full && digit == shown[place] && selected == Self::selected(self.shown, place) {
                continue;
            }
            draw_cell(
                canvas,
                TIME_DIGIT,
                TIME_X_OF[place],
                TIME_Y,
                Cell::Digit(digit),
                if selected { theme::ACCENT } else { theme::TEXT },
            )?;
        }
        if !full {
            return Ok(());
        }
        let colon_x = TIME_COLON_X + (TIME_COLON_WIDTH - TIME_DIGIT.stroke) / 2;
        for third in 1..=2 {
            let y = TIME_Y + TIME_DIGIT.height * third / 3 - TIME_DIGIT.stroke / 2;
            fill(
                canvas,
                colon_x,
                y,
                TIME_DIGIT.stroke,
                TIME_DIGIT.stroke,
                theme::ACCENT,
            )?;
        }
        Ok(())
    }

    fn draw_field(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(canvas, 0, 123, PANEL_WIDTH, 32, theme::BACKGROUND)?;
        draw_instrument_centred(self.field_name(), FIELD_BASELINE, theme::ACCENT, canvas)
    }

    fn paint(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(canvas, 0, 0, PANEL_WIDTH, PANEL_HEIGHT, theme::BACKGROUND)?;
        draw_instrument_centred("TIME", TITLE_BASELINE, theme::ACCENT, canvas)?;
        self.draw_value(true, canvas)?;
        self.draw_field(canvas)?;
        for slot in 0..CONTROL_COUNT {
            self.controls.draw(slot, self.control_label(slot), canvas)?;
        }
        Ok(())
    }
}

impl Default for TimeScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Paint for TimeScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        _keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas)
    }
}

impl Screen for TimeScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        let Some(slot) = self.controls.handle(event, &mut self.pending) else {
            return ScreenAction::None;
        };
        match slot {
            DECREASE => {
                self.editor.adjust(false);
                self.pending.value = true;
            }
            INCREASE => {
                self.editor.adjust(true);
                self.pending.value = true;
            }
            NEXT => {
                self.editor.next();
                self.pending.value = true;
                self.pending.field = true;
                self.pending.controls[DECREASE] = true;
                self.pending.controls[INCREASE] = true;
            }
            APPLY => return ScreenAction::SetTime(self.editor.value()),
            _ => return ScreenAction::None,
        }
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        _keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.pending.value {
            self.draw_value(false, canvas)?;
        }
        if self.pending.field {
            self.draw_field(canvas)?;
        }
        for slot in 0..CONTROL_COUNT {
            if self.pending.controls[slot] {
                self.controls.draw(slot, self.control_label(slot), canvas)?;
            }
        }
        Ok(())
    }
}

pub struct DateScreen {
    editor: DateEditor,
    shown: DateEditor,
    controls: Controls,
    pending: Pending,
}

impl DateScreen {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            editor: DateEditor::new(CalendarDate::DEFAULT),
            shown: DateEditor::new(CalendarDate::DEFAULT),
            controls: Controls::new(),
            pending: Pending::all(),
        }
    }

    pub const fn open(&mut self, current: CalendarDate) {
        self.editor = DateEditor::new(current);
        self.pending = Pending::all();
    }

    pub fn mark_painted(&mut self) {
        self.shown = self.editor;
        self.pending = Pending::default();
    }

    #[must_use]
    pub const fn moved(&self) -> bool {
        !self.pending.clean()
    }

    const fn field_name(&self) -> &'static str {
        match self.editor.field() {
            DateField::Year => "YEAR",
            DateField::Month => "MONTH",
            DateField::Day => "DAY",
        }
    }

    const fn control_label(&self, slot: usize) -> &'static str {
        match slot {
            DECREASE => match self.editor.field() {
                DateField::Year => "- YEAR",
                DateField::Month => "- MONTH",
                DateField::Day => "- DAY",
            },
            INCREASE => match self.editor.field() {
                DateField::Year => "+ YEAR",
                DateField::Month => "+ MONTH",
                DateField::Day => "+ DAY",
            },
            NEXT => "NEXT",
            APPLY => "APPLY",
            _ => "",
        }
    }

    const fn digits(editor: DateEditor) -> [u8; 8] {
        let value = editor.value();
        [
            ((value.year / 1_000) % 10) as u8,
            ((value.year / 100) % 10) as u8,
            ((value.year / 10) % 10) as u8,
            (value.year % 10) as u8,
            value.month / 10,
            value.month % 10,
            value.day / 10,
            value.day % 10,
        ]
    }

    fn selected(editor: DateEditor, place: usize) -> bool {
        match editor.field() {
            DateField::Year => place < 4,
            DateField::Month => (4..6).contains(&place),
            DateField::Day => place >= 6,
        }
    }

    fn draw_value(&self, full: bool, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let digits = Self::digits(self.editor);
        let shown = Self::digits(self.shown);
        for (place, digit) in digits.into_iter().enumerate() {
            let selected = Self::selected(self.editor, place);
            if !full && digit == shown[place] && selected == Self::selected(self.shown, place) {
                continue;
            }
            draw_cell(
                canvas,
                DATE_DIGIT,
                DATE_X_OF[place],
                DATE_Y,
                Cell::Digit(digit),
                if selected { theme::ACCENT } else { theme::TEXT },
            )?;
        }
        if !full {
            return Ok(());
        }
        for x in DATE_DASH_X {
            fill(
                canvas,
                x,
                DATE_Y + DATE_DIGIT.height / 2 - DATE_DIGIT.stroke / 2,
                7,
                DATE_DIGIT.stroke,
                theme::TEXT,
            )?;
        }
        Ok(())
    }

    fn draw_field(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(canvas, 0, 116, PANEL_WIDTH, 39, theme::BACKGROUND)?;
        draw_instrument_centred(self.field_name(), FIELD_BASELINE, theme::ACCENT, canvas)
    }

    fn paint(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(canvas, 0, 0, PANEL_WIDTH, PANEL_HEIGHT, theme::BACKGROUND)?;
        draw_instrument_centred("DATE", TITLE_BASELINE, theme::ACCENT, canvas)?;
        self.draw_value(true, canvas)?;
        self.draw_field(canvas)?;
        for slot in 0..CONTROL_COUNT {
            self.controls.draw(slot, self.control_label(slot), canvas)?;
        }
        Ok(())
    }
}

impl Default for DateScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Paint for DateScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        _keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas)
    }
}

impl Screen for DateScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        let Some(slot) = self.controls.handle(event, &mut self.pending) else {
            return ScreenAction::None;
        };
        match slot {
            DECREASE => {
                self.editor.adjust(false);
                self.pending.value = true;
            }
            INCREASE => {
                self.editor.adjust(true);
                self.pending.value = true;
            }
            NEXT => {
                self.editor.next();
                self.pending.value = true;
                self.pending.field = true;
                self.pending.controls[DECREASE] = true;
                self.pending.controls[INCREASE] = true;
            }
            APPLY => return ScreenAction::SetDate(self.editor.value()),
            _ => return ScreenAction::None,
        }
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        _keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.pending.value {
            self.draw_value(false, canvas)?;
        }
        if self.pending.field {
            self.draw_field(canvas)?;
        }
        for slot in 0..CONTROL_COUNT {
            if self.pending.controls[slot] {
                self.controls.draw(slot, self.control_label(slot), canvas)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    fn tap<S: Screen>(screen: &mut S, slot: usize) -> ScreenAction {
        let bounds = control_bounds(slot);
        let x = bounds.x() + bounds.width() / 2;
        let y = bounds.y() + bounds.height() / 2;
        let _ = screen.handle_event(AppEvent::Touch {
            x,
            y,
            pressed: true,
        });
        screen.handle_event(AppEvent::Touch {
            x,
            y,
            pressed: false,
        })
    }

    #[test]
    fn time_apply_returns_the_edited_value() {
        let mut screen = TimeScreen::new();
        assert_eq!(tap(&mut screen, INCREASE), ScreenAction::None);
        assert_eq!(tap(&mut screen, NEXT), ScreenAction::None);
        assert_eq!(tap(&mut screen, INCREASE), ScreenAction::None);
        assert_eq!(
            tap(&mut screen, APPLY),
            ScreenAction::SetTime(WallTime::new(1, 1, 0).unwrap())
        );
    }

    #[test]
    fn date_apply_returns_one_valid_complete_date() {
        let mut screen = DateScreen::new();
        screen.open(CalendarDate::new(2028, 1, 31).unwrap());
        let _ = tap(&mut screen, NEXT);
        let _ = tap(&mut screen, INCREASE);
        assert_eq!(
            tap(&mut screen, APPLY),
            ScreenAction::SetDate(CalendarDate::new(2028, 2, 29).unwrap())
        );
    }

    #[test]
    fn time_adjustment_repaints_one_digit_and_one_button() {
        let mut screen = TimeScreen::new();
        screen.mark_painted();
        assert_eq!(tap(&mut screen, INCREASE), ScreenAction::None);

        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");

        assert!(probe.painted_within(TIME_DIGIT.cell(TIME_X_OF[1], TIME_Y)));
        assert!(!probe.painted_within(TIME_DIGIT.cell(TIME_X_OF[0], TIME_Y)));
        assert!(!probe.painted_within(TIME_DIGIT.cell(TIME_X_OF[2], TIME_Y)));
        assert!(!probe.painted_within(TIME_DIGIT.cell(TIME_X_OF[3], TIME_Y)));
        assert!(probe.painted_within(control_area(INCREASE)));
        assert!(!probe.painted_within(control_area(DECREASE)));
        assert!(!probe.painted_within(control_area(NEXT)));
        assert!(!probe.painted_within(control_area(APPLY)));
    }

    #[test]
    fn date_adjustment_repaints_one_digit_and_one_button() {
        let mut screen = DateScreen::new();
        screen.mark_painted();
        assert_eq!(tap(&mut screen, INCREASE), ScreenAction::None);

        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");

        assert!(probe.painted_within(DATE_DIGIT.cell(DATE_X_OF[3], DATE_Y)));
        for place in [0, 1, 2, 4, 5, 6, 7] {
            assert!(!probe.painted_within(DATE_DIGIT.cell(DATE_X_OF[place], DATE_Y)));
        }
        assert!(probe.painted_within(control_area(INCREASE)));
        assert!(!probe.painted_within(control_area(DECREASE)));
        assert!(!probe.painted_within(control_area(NEXT)));
        assert!(!probe.painted_within(control_area(APPLY)));
    }

    fn control_area(slot: usize) -> Rectangle {
        let bounds = control_bounds(slot);
        Rectangle::new(
            Point::new(bounds.x(), bounds.y()),
            Size::new(
                bounds.width().unsigned_abs(),
                bounds.height().unsigned_abs(),
            ),
        )
    }
}
