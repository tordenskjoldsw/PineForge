//! The watch used as a lamp: the panel itself is the light.
//!
//! The only screen whose *point* is its backlight, which is why it is the only
//! one the display task asks about before setting a level. It holds no
//! hardware - no screen does - so what it owns is a single fact, whether the
//! lamp is lit, and the task reads that when deciding how bright the panel
//! should be.
//!
//! It opens dark. A tap lights it, another puts it out, the way `InfiniTime`
//! does it. Opening straight into full white would blind whoever reached for
//! the wrong tile in the dark, and would start drawing the current a lit panel
//! costs before anyone asked for light.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use pineforge_state::{AppEvent, Button, ButtonBounds, ButtonOutcome, ScreenAction};

use crate::canvas::{Canvas, CanvasError};
use crate::font::ui_text;
use crate::{
    icons::{self, ICON_SIZE, draw_icon},
    render::{draw_mono_text_visible, draw_visible},
    screen::{Paint, Screen},
    theme,
};

/// Full white rather than the theme's background: this is the light, and every
/// pixel that is not white is light the watch is not giving.
const LIT: Rgb565 = Rgb565::WHITE;

const PANEL_SIDE: i32 = 240;
const PANEL: Rectangle = Rectangle::new(
    Point::zero(),
    Size::new(PANEL_SIDE.cast_unsigned(), PANEL_SIDE.cast_unsigned()),
);

/// Centred, with the hint below it.
const ICON_TOP: i32 = 92;
const HINT_BASELINE: i32 = 156;
/// One character of the UI face, for centring by hand.
const CHARACTER_WIDTH: i32 = 10;

const HINT_DARK: &str = "TAP FOR LIGHT";
const HINT_LIT: &str = "TAP TO CLOSE";

pub struct FlashlightScreen {
    /// The whole panel, as one control.
    ///
    /// A [`Button`] rather than a test for a release, which is what this screen
    /// shipped with and why one tap lit the lamp and put it straight back out:
    /// the router emits a `Touch` for every report that says the finger is up,
    /// and the `CST816S` sends more than one after a lift. A release with no
    /// press before it is not an activation, and this is the primitive that
    /// already knew that - every other screen reaches a control through it,
    /// which is why none of them had this bug.
    surface: Button,
    lit: bool,
    dirty: bool,
}

impl Default for FlashlightScreen {
    fn default() -> Self {
        Self {
            surface: Button::new(ButtonBounds::new(0, 0, PANEL_SIDE, PANEL_SIDE)),
            lit: false,
            dirty: false,
        }
    }
}

impl FlashlightScreen {
    /// Puts the lamp out.
    ///
    /// Called as the screen is entered, so it opens dark every time rather than
    /// resuming where it was left. Remembering would undo the reason it opens
    /// dark at all: reaching the tile would light the panel before anyone asked
    /// for light.
    ///
    /// Entering is always followed by a full repaint, so this owes no dirty
    /// mark - and clearing one left over from the last visit stops a stale mark
    /// costing a repaint of a screen that was just drawn anyway.
    pub const fn put_out(&mut self) {
        self.lit = false;
        self.dirty = false;
    }

    /// Whether the lamp is currently on.
    ///
    /// Read by the display task, which owns the backlight; this screen can no
    /// more brighten the panel than any other screen can.
    #[must_use]
    pub const fn is_lit(&self) -> bool {
        self.lit
    }

    fn paint(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let (ground, ink) = if self.lit {
            (LIT, theme::BACKGROUND)
        } else {
            (theme::BACKGROUND, theme::TEXT)
        };
        draw_visible(
            &PANEL.into_styled(PrimitiveStyle::with_fill(ground)),
            canvas,
        )?;

        // Only while dark. Lit, the icon would be a hole in the light for no
        // gain - the screen being white is the whole message.
        if !self.lit {
            draw_icon(
                &icons::TORCH,
                Point::new((PANEL_SIDE - ICON_SIZE) / 2, ICON_TOP),
                theme::ACCENT,
                canvas,
            )?;
        }

        let hint = if self.lit { HINT_LIT } else { HINT_DARK };
        let width = i32::try_from(hint.len()).unwrap_or(0) * CHARACTER_WIDTH;
        draw_mono_text_visible(
            hint,
            Point::new((PANEL_SIDE - width) / 2, HINT_BASELINE),
            ui_text(ink, ground),
            canvas,
        )
    }
}

impl Paint for FlashlightScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        // No clear first: `paint` fills the whole panel itself, so clearing
        // would send a second full frame over SPI for a surface that is about
        // to be covered anyway.
        keep_alive();
        self.paint(canvas)
    }
}

impl Screen for FlashlightScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        self.dirty = false;
        // Only a completed press-and-lift. `Redraw` is ignored on purpose: this
        // control is the whole panel, and flashing it on the way down would be
        // the light stuttering rather than feedback.
        if self.surface.handle_event(event) == ButtonOutcome::Activated {
            self.lit = !self.lit;
            self.dirty = true;
        }
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        // Only a toggle is worth anything here, and it inverts the whole panel,
        // so the repaint is all-or-nothing.
        //
        // The mark is what makes it nothing most of the time. A tick arrives
        // every second and reaches the active screen, and this screen is the
        // one that guarantees they never stop - it holds the watch awake.
        // Repainting unconditionally therefore refilled all 240x240 pixels once
        // a second for as long as the lamp was open, which is visible as
        // flicker rather than as a cost you have to measure.
        if !self.dirty {
            return Ok(());
        }
        self.paint(canvas)?;
        keep_alive();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    fn touch(pressed: bool) -> AppEvent {
        AppEvent::Touch {
            x: 120,
            y: 120,
            pressed,
        }
    }

    const TICK: AppEvent = AppEvent::Tick {
        uptime_seconds: 1,
        wall_time: None,
        date: None,
    };

    /// Draws whatever the screen thinks is dirty and reports what it touched.
    fn repaint(screen: &FlashlightScreen) -> Probe {
        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    /// The lamp went on and straight back off on a single tap.
    ///
    /// Toggling on the release alone was the cause: the router emits a `Touch`
    /// for every report saying the finger is up, and the controller sends more
    /// than one after a lift, so the second one put the light out again. A
    /// release with no press before it must do nothing.
    #[test]
    fn a_release_with_no_press_before_it_leaves_the_lamp_alone() {
        let mut screen = FlashlightScreen::default();
        assert!(!screen.is_lit(), "the lamp opens dark");

        let _ = screen.handle_event(touch(true));
        let _ = screen.handle_event(touch(false));
        assert!(screen.is_lit(), "a tap lights it");

        // The stray lift the controller adds. Exactly one, which is what the
        // hardware does - two of them would toggle twice under the old
        // release-only rule and land back on lit, hiding the bug.
        let _ = screen.handle_event(touch(false));
        assert!(screen.is_lit(), "a second lift must not put it out");

        // A real second tap still does.
        let _ = screen.handle_event(touch(true));
        let _ = screen.handle_event(touch(false));
        assert!(!screen.is_lit(), "tapping again puts it out");
    }

    /// A press that is dragged off the panel is abandoned, not an activation.
    #[test]
    fn a_press_taken_back_by_a_gesture_does_not_toggle() {
        let mut screen = FlashlightScreen::default();

        let _ = screen.handle_event(touch(true));
        let _ = screen.handle_event(AppEvent::TouchCancelled);
        let _ = screen.handle_event(touch(false));

        assert!(!screen.is_lit(), "a cancelled press lights nothing");
    }

    /// This screen holds the watch awake, so ticks never stop arriving. An
    /// unconditional repaint therefore refilled the whole panel once a second
    /// for as long as the lamp was open, which showed up as flicker.
    #[test]
    fn a_tick_costs_no_pixels() {
        let mut screen = FlashlightScreen::default();
        let _ = screen.handle_event(touch(true));
        let _ = screen.handle_event(touch(false));
        assert!(repaint(&screen).unpainted() < 240 * 240, "the tap paints");

        let _ = screen.handle_event(TICK);
        assert_eq!(
            repaint(&screen).unpainted(),
            240 * 240,
            "a tick repainted the panel"
        );
    }

    /// Entering must not inherit a repaint owed from the last visit, because
    /// navigation has already drawn the screen in full by then.
    #[test]
    fn opening_the_lamp_owes_no_repaint() {
        let mut screen = FlashlightScreen::default();
        let _ = screen.handle_event(touch(true));
        let _ = screen.handle_event(touch(false));

        screen.put_out();
        assert!(!screen.is_lit());
        assert_eq!(repaint(&screen).unpainted(), 240 * 240);
    }
}
