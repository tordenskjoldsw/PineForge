//! Menu screens described as data rather than written as drawing code.
//!
//! Every menu in this firmware is the same shape: an optional title, a column
//! of framed label/value rows on a fixed rhythm, and a hint at the foot. Written
//! out per screen, that shape costs its drawing code once per screen - and each
//! row states its geometry three times over, as a `Y` constant, as the bounds of
//! its hit box, and again at the call that draws it. Three places to keep in
//! step is two too many, and the drawing code is identical every time.
//!
//! A [`Menu`] states the shape once, in `.rodata`, and [`draw`] is the single
//! interpreter of it. Positions are derived from the rhythm, so a row cannot
//! disagree with its own hit box. What a screen still owns is the part that is
//! genuinely its own: which values the rows show, and what activating one does.
//!
//! Touch and pagination are not reimplemented here. [`ListSlots`] already owns
//! them for the launcher, host-tested, including the rule a plain array of
//! buttons gets wrong: a press belongs to the row it started in, so a finger
//! dragged down the screen cancels instead of choosing whichever row it is
//! lifted over.

use embedded_graphics::{geometry::Point, prelude::*, primitives::Rectangle};
use pineforge_state::{AppEvent, ButtonBounds, ListOutcome, ListSlots, PageAxis, PagedList};

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
    /// Top edge of the first row on every page.
    pub first_row_y: i32,
    /// Distance between the top edges of consecutive rows.
    pub row_step: i32,
    /// The left-hand label of each entry, in order. Entries beyond one page are
    /// reached by paging.
    pub labels: &'static [&'static str],
    /// Shown at the foot, usually naming the way back.
    pub hint: &'static str,
}

impl Menu {
    /// Top edge of a slot, which is also the top of its hit box.
    ///
    /// Indexed by slot rather than by entry: slot 0 is drawn in the same place
    /// on every page, whichever entry it currently shows.
    #[must_use]
    pub fn slot_y(&self, slot: usize) -> i32 {
        self.first_row_y + self.row_step * i32::try_from(slot).unwrap_or(0)
    }

    /// Hit box of a slot, derived from the same rhythm that draws it.
    #[must_use]
    pub fn slot_bounds(&self, slot: usize) -> ButtonBounds {
        ButtonBounds::new(ROW_X, self.slot_y(slot), ROW_WIDTH, ROW_HEIGHT)
    }
}

/// Paints a whole menu opaquely, as [`Paint::draw_full`] requires.
///
/// [`Paint::draw_full`]: crate::ui::screen::Paint::draw_full
pub fn draw(
    menu: &Menu,
    list: &PagedList,
    values: &[&str],
    canvas: &mut Canvas<'_>,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    canvas.clear(theme::BACKGROUND)?;
    keep_alive();

    if let Some(title) = &menu.title {
        draw_text(title.text, title.baseline_y, canvas)?;
        keep_alive();
    }

    draw_rows(menu, list, values, canvas, keep_alive)?;
    draw_text(menu.hint, HINT_BASELINE_Y, canvas)
}

/// Repaints the rows alone, for a screen whose values moved.
///
/// The title and the hint do not change with state, so a value that ticks costs
/// its rows and nothing else. Slots the page does not fill are cleared rather
/// than skipped, so a short last page cannot leave the previous page's rows
/// standing under it.
pub fn draw_rows(
    menu: &Menu,
    list: &PagedList,
    values: &[&str],
    canvas: &mut Canvas<'_>,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    for slot in 0..list.per_page() {
        let y = menu.slot_y(slot);
        match list.entry_at(slot) {
            Some(entry) => draw_row(
                canvas,
                y,
                menu.labels.get(entry).copied().unwrap_or(""),
                values.get(entry).copied().unwrap_or(""),
            )?,
            None => clear_slot(y, canvas)?,
        }
        keep_alive();
    }
    Ok(())
}

fn clear_slot(y: i32, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
    canvas.fill_solid(
        &Rectangle::new(
            Point::new(ROW_X, y),
            Size::new(
                u32::try_from(ROW_WIDTH).unwrap_or(0),
                u32::try_from(ROW_HEIGHT).unwrap_or(0),
            ),
        ),
        theme::BACKGROUND,
    )
}

fn draw_text(text: &str, baseline: i32, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
    draw_mono_text_visible(
        text,
        Point::new(ROW_X, baseline),
        ui_text(theme::TEXT, theme::BACKGROUND),
        canvas,
    )
}

/// A menu's slots and which of them the user is touching.
///
/// Kept beside [`Menu`] rather than inside it because a description is
/// `'static` and shared, while press state belongs to the screen instance.
///
/// `N` is the number of rows on a page, not the number of entries: a menu that
/// outgrows its page pages rather than scrolling, which is what a display
/// without a framebuffer can afford.
pub struct MenuState<const N: usize> {
    slots: ListSlots<N>,
    dirty: bool,
}

impl<const N: usize> MenuState<N> {
    #[must_use]
    pub fn new(menu: &Menu) -> Self {
        Self {
            slots: ListSlots::new(
                core::array::from_fn(|slot| menu.slot_bounds(slot)),
                // A menu is opened by a tap and left by swiping back along the
                // horizontal, so that axis belongs to navigation and paging has
                // to take the other one.
                PageAxis::Vertical,
                menu.labels.len(),
            ),
            dirty: false,
        }
    }

    /// Which page is showing and what it holds, for drawing.
    #[must_use]
    pub const fn list(&self) -> &PagedList {
        self.slots.list()
    }

    /// Feeds an event to the page, reporting the entry that was activated.
    pub fn handle(&mut self, event: AppEvent) -> Option<usize> {
        match self.slots.handle_event(event) {
            ListOutcome::Activated(entry) => {
                self.dirty = true;
                Some(entry)
            }
            ListOutcome::Paged => {
                self.dirty = true;
                None
            }
            // A row has no pressed appearance yet, so a press that only changes
            // slot state has nothing to repaint. Giving rows the launcher's
            // pressed fill is what would turn this arm into a redraw.
            ListOutcome::Redraw | ListOutcome::None => {
                self.dirty = false;
                None
            }
        }
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
