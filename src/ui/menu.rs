//! Menu screens described as data rather than written as drawing code.
//!
//! Every menu in this firmware is the same shape: an optional title, a column
//! of framed label/value rows on a fixed rhythm, and a hint at the foot. Written
//! out per screen, that shape costs its drawing code once per screen - and each
//! row states its geometry three times over, as a `Y` constant, as the bounds of
//! its [`Button`], and again at the call that draws it. Three places to keep in
//! step is two too many, and the drawing code is identical every time.
//!
//! A [`Menu`] states the shape once, in `.rodata`, and [`draw`] is the single
//! interpreter of it. Positions are derived from the rhythm, so a row cannot
//! disagree with its own hit box. What a screen still owns is the part that is
//! genuinely its own: which values the rows show, and what activating one does.
//!
//! This is the layer a redesign should grow: a second row style or a section
//! heading becomes a variant here and arrives on every menu at once, rather than
//! a change swept through each screen by hand.

use embedded_graphics::{geometry::Point, pixelcolor::Rgb565, prelude::*};
use pineforge_state::{AppEvent, Button, ButtonBounds, ButtonOutcome};

use crate::ui::{
    canvas::{Canvas, CanvasError},
    font::ui_text,
    render::{ROW_HEIGHT, ROW_WIDTH, ROW_X, draw_mono_text_visible, draw_row},
    theme,
};

/// Baseline of the hint every menu carries at its foot.
const HINT_BASELINE_Y: i32 = 232;

/// A menu's heading, with the baseline it sits on.
///
/// The baseline is stated rather than derived from the first row: a title is
/// set in a different rhythm than the rows below it, and deriving it would move
/// every heading whenever a row's spacing changed.
pub struct MenuTitle {
    pub text: &'static str,
    pub baseline_y: i32,
}

/// The fixed part of a menu: everything that does not depend on state.
///
/// Held as a `'static` description so a screen costs a constant rather than a
/// function. The values the rows show are supplied per draw; see [`draw`].
pub struct Menu {
    /// Shown above the rows. `None` leaves the space to them.
    pub title: Option<MenuTitle>,
    /// Top edge of the first row.
    pub first_row_y: i32,
    /// Distance between the top edges of consecutive rows.
    pub row_step: i32,
    /// The left-hand label of each row, in order.
    pub labels: &'static [&'static str],
    /// Shown at the foot, usually naming the way back.
    pub hint: &'static str,
}

impl Menu {
    /// Top edge of a row, which is also the top of its hit box.
    #[must_use]
    pub fn row_y(&self, index: usize) -> i32 {
        self.first_row_y + self.row_step * i32::try_from(index).unwrap_or(0)
    }

    /// Hit box of a row, derived from the same rhythm that draws it.
    #[must_use]
    pub fn row_bounds(&self, index: usize) -> ButtonBounds {
        ButtonBounds::new(ROW_X, self.row_y(index), ROW_WIDTH, ROW_HEIGHT)
    }

    /// One [`Button`] per row, ready for the screen to own.
    #[must_use]
    pub fn buttons<const N: usize>(&self) -> [Button; N] {
        debug_assert!(
            N == self.labels.len(),
            "a menu's hit boxes must match its labels"
        );
        core::array::from_fn(|index| Button::new(self.row_bounds(index)))
    }
}

/// Paints a whole menu opaquely, as [`Paint::draw_full`] requires.
///
/// `values` supplies the right-hand column in row order. A row without a value
/// draws an empty column rather than being skipped, so a short slice cannot
/// silently shift the remaining rows upwards.
///
/// [`Paint::draw_full`]: crate::ui::screen::Paint::draw_full
pub fn draw(
    menu: &Menu,
    values: &[&str],
    canvas: &mut Canvas<'_>,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    canvas.clear(theme::BACKGROUND)?;
    keep_alive();

    if let Some(title) = &menu.title {
        draw_text(title.text, title.baseline_y, theme::TEXT, canvas)?;
        keep_alive();
    }

    draw_rows(menu, values, canvas, keep_alive)?;
    draw_text(menu.hint, HINT_BASELINE_Y, theme::TEXT, canvas)
}

/// Repaints the rows alone, for a screen whose values moved.
///
/// The frame and the hint do not change with state, so a value that ticks costs
/// its rows and nothing else.
pub fn draw_rows(
    menu: &Menu,
    values: &[&str],
    canvas: &mut Canvas<'_>,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    for (index, label) in menu.labels.iter().enumerate() {
        draw_row(
            canvas,
            menu.row_y(index),
            label,
            values.get(index).copied().unwrap_or(""),
        )?;
        keep_alive();
    }
    Ok(())
}

fn draw_text(
    text: &str,
    baseline: i32,
    ink: Rgb565,
    canvas: &mut Canvas<'_>,
) -> Result<(), CanvasError> {
    draw_mono_text_visible(
        text,
        Point::new(ROW_X, baseline),
        ui_text(ink, theme::BACKGROUND),
        canvas,
    )
}

/// A menu's rows and which of them the user is touching.
///
/// Kept beside [`Menu`] rather than inside it because a description is
/// `'static` and shared, while press state belongs to the screen instance.
///
/// `N` is the row count, so the hit boxes cost exactly the rows that exist. A
/// growable buffer sized for the largest conceivable menu would have wasted
/// more RAM on the two screens here than the whole abstraction saves, and RAM
/// is the scarcer of the two budgets on this chip.
pub struct MenuState<const N: usize> {
    buttons: [Button; N],
    dirty: bool,
}

impl<const N: usize> MenuState<N> {
    #[must_use]
    pub fn new(menu: &Menu) -> Self {
        Self {
            buttons: menu.buttons(),
            dirty: false,
        }
    }

    /// Feeds an event to every row, reporting the one that was activated.
    ///
    /// Clears the dirty flag first, so a screen's `draw_dirty` reflects only the
    /// event it has just handled.
    pub fn handle(&mut self, event: AppEvent) -> Option<usize> {
        self.dirty = false;
        let mut activated = None;
        for (index, button) in self.buttons.iter_mut().enumerate() {
            if button.handle_event(event) == ButtonOutcome::Activated {
                activated = Some(index);
            }
        }
        activated
    }

    /// Marks the rows as needing a repaint, for state that arrives from
    /// somewhere other than a touch.
    pub const fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }
}
