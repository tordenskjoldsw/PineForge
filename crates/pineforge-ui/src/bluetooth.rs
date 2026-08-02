//! User-facing Bluetooth radio control in the FORGE instrument language.

use embedded_graphics::{
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};
use pineforge_state::{
    AppEvent, BleState, Button, ButtonBounds, ButtonOutcome, ButtonState, DisplaySettings,
    ScreenAction,
};

use crate::{
    canvas::{Canvas, CanvasError},
    font::ui_text,
    render::{PANEL, draw_instrument_centred, draw_visible, fill, round_corners},
    screen::{Paint, Screen},
    theme,
};

const PANEL_WIDTH: i32 = PANEL.size.width.cast_signed();
const PANEL_HEIGHT: i32 = PANEL.size.height.cast_signed();
const TITLE_BASELINE: i32 = 46;
const STATE_BASELINE: i32 = 170;
const VISUAL_TOP: i32 = 55;
const VISUAL_HEIGHT: i32 = 125;

const BUTTON: ButtonBounds = ButtonBounds::new(20, 190, 200, 40);

const RUNE_CENTRE_X: i32 = 120;
const RUNE_TOP: i32 = 62;
const RUNE_BOTTOM: i32 = 132;
const RUNE_HALF_WIDTH: i32 = 25;

pub struct BluetoothScreen {
    settings: DisplaySettings,
    state: BleState,
    button: Button,
    visual_dirty: bool,
    button_dirty: bool,
}

impl BluetoothScreen {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            settings: DisplaySettings::DEFAULT,
            state: BleState::Off,
            button: Button::new(BUTTON),
            visual_dirty: false,
            button_dirty: false,
        }
    }

    pub fn open(&mut self, settings: DisplaySettings) {
        self.settings = settings;
        self.sync_button();
        self.visual_dirty = false;
        self.button_dirty = false;
    }

    pub const fn mark_painted(&mut self) {
        self.visual_dirty = false;
        self.button_dirty = false;
    }

    #[must_use]
    pub const fn moved(&self) -> bool {
        self.visual_dirty || self.button_dirty
    }

    fn sync_button(&mut self) {
        if self
            .button
            .set_enabled(!matches!(self.state, BleState::DfuProgress(_)))
            == ButtonOutcome::Redraw
        {
            self.button_dirty = true;
        }
    }

    const fn state_text(&self) -> &'static str {
        if !self.settings.ble_enabled() {
            return "OFF";
        }
        match self.state {
            BleState::Off => "STARTING",
            BleState::Advertising => "ADVERTISING",
            BleState::Pairing(_) => "PAIRING",
            BleState::Connected => "CONNECTED",
            BleState::DfuProgress(_) => "UPDATING",
            BleState::DfuFailed(_) => "BLE ERROR",
        }
    }

    const fn visual_ink(&self) -> embedded_graphics::pixelcolor::Rgb565 {
        if !self.settings.ble_enabled() || matches!(self.state, BleState::Off) {
            theme::MUTED
        } else if matches!(self.state, BleState::DfuFailed(_)) {
            theme::DANGER
        } else {
            theme::ACCENT
        }
    }

    const fn button_text(&self) -> &'static str {
        if self.settings.ble_enabled() {
            "DISABLE"
        } else {
            "ENABLE"
        }
    }

    fn draw_rune(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let style = PrimitiveStyle::with_stroke(self.visual_ink(), 6);
        let centre_top = Point::new(RUNE_CENTRE_X, RUNE_TOP);
        let centre_bottom = Point::new(RUNE_CENTRE_X, RUNE_BOTTOM);
        let upper_right = Point::new(RUNE_CENTRE_X + RUNE_HALF_WIDTH, RUNE_TOP + 18);
        let lower_right = Point::new(RUNE_CENTRE_X + RUNE_HALF_WIDTH, RUNE_BOTTOM - 18);
        let upper_left = Point::new(RUNE_CENTRE_X - RUNE_HALF_WIDTH, RUNE_TOP + 25);
        let lower_left = Point::new(RUNE_CENTRE_X - RUNE_HALF_WIDTH, RUNE_BOTTOM - 25);
        for (from, to) in [
            (centre_top, centre_bottom),
            (centre_top, upper_right),
            (upper_right, lower_left),
            (centre_bottom, lower_right),
            (lower_right, upper_left),
        ] {
            draw_visible(&Line::new(from, to).into_styled(style), canvas)?;
        }
        Ok(())
    }

    fn draw_visual(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(
            canvas,
            0,
            VISUAL_TOP,
            PANEL_WIDTH,
            VISUAL_HEIGHT,
            theme::BACKGROUND,
        )?;
        self.draw_rune(canvas)?;
        draw_instrument_centred(self.state_text(), STATE_BASELINE, self.visual_ink(), canvas)
    }

    fn draw_button(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let area = Rectangle::new(
            Point::new(BUTTON.x(), BUTTON.y()),
            Size::new(
                BUTTON.width().unsigned_abs(),
                BUTTON.height().unsigned_abs(),
            ),
        );
        let (face, ink) = match self.button.state() {
            ButtonState::Pressed => (theme::ACCENT, theme::BACKGROUND),
            ButtonState::Disabled => (theme::SURFACE, theme::MUTED),
            ButtonState::Idle => (theme::SURFACE, theme::ACCENT),
        };
        fill(
            canvas,
            BUTTON.x(),
            BUTTON.y(),
            BUTTON.width(),
            BUTTON.height(),
            face,
        )?;
        round_corners(&area, canvas)?;
        draw_visible(
            &Text::with_alignment(
                self.button_text(),
                Point::new(
                    BUTTON.x() + BUTTON.width() / 2,
                    BUTTON.y() + BUTTON.height() / 2 + 7,
                ),
                ui_text(ink, face),
                Alignment::Center,
            ),
            canvas,
        )
    }

    fn paint(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        fill(canvas, 0, 0, PANEL_WIDTH, PANEL_HEIGHT, theme::BACKGROUND)?;
        draw_instrument_centred("BLUETOOTH", TITLE_BASELINE, theme::ACCENT, canvas)?;
        self.draw_visual(canvas)?;
        self.draw_button(canvas)
    }
}

impl Default for BluetoothScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Paint for BluetoothScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        _keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas)
    }
}

impl Screen for BluetoothScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        match event {
            AppEvent::BleUpdated(state) if state != self.state => {
                self.state = state;
                self.visual_dirty = true;
                self.sync_button();
            }
            AppEvent::DisplaySettingsUpdated(settings) if settings != self.settings => {
                self.settings = settings;
                self.visual_dirty = true;
                self.button_dirty = true;
                self.sync_button();
            }
            _ => {}
        }

        match self.button.handle_event(event) {
            ButtonOutcome::Activated => {
                self.settings = self.settings.with_ble_enabled(!self.settings.ble_enabled());
                self.visual_dirty = true;
                self.button_dirty = true;
                ScreenAction::ApplySettings(self.settings)
            }
            ButtonOutcome::Redraw => {
                self.button_dirty = true;
                ScreenAction::None
            }
            ButtonOutcome::None => ScreenAction::None,
        }
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        _keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.visual_dirty {
            self.draw_visual(canvas)?;
        }
        if self.button_dirty {
            self.draw_button(canvas)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(screen: &mut BluetoothScreen) -> ScreenAction {
        let x = BUTTON.x() + BUTTON.width() / 2;
        let y = BUTTON.y() + BUTTON.height() / 2;
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
    fn toggle_returns_one_persistable_settings_snapshot() {
        let mut screen = BluetoothScreen::new();
        assert_eq!(
            tap(&mut screen),
            ScreenAction::ApplySettings(DisplaySettings::DEFAULT.with_ble_enabled(false))
        );
        assert_eq!(
            tap(&mut screen),
            ScreenAction::ApplySettings(DisplaySettings::DEFAULT)
        );
    }

    #[test]
    fn an_active_firmware_transfer_cannot_disable_the_radio() {
        let mut screen = BluetoothScreen::new();
        let _ = screen.handle_event(AppEvent::BleUpdated(BleState::DfuProgress(42)));
        assert_eq!(tap(&mut screen), ScreenAction::None);
    }
}
