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
use pineforge_state::{
    AppEvent, ButtonBounds, ButtonState, ListOutcome, ListSlots, PageAxis, PagedList, ScreenId,
};

use crate::{
    canvas::{Canvas, CanvasError},
    font::ui_text,
    render::{ROW_HEIGHT, ROW_WIDTH, ROW_X, draw_mono_text_visible, draw_page_marks, draw_row},
    theme,
};

/// Baseline of the hint every menu carries at its foot.
const HINT_BASELINE_Y: i32 = 232;

/// Centre of the page rail: the gutter to the right of the rows, halfway down
/// the panel.
///
/// The rows span x = 20 to 220 and are centred, so both gutters are 20 wide and
/// this sits in the middle of the right one. Menus differ in how many rows they
/// carry and whether they wear a title, so the rail is centred on the panel
/// rather than on the rows - it stays put between screens instead of shifting
/// with whatever the screen happens to list.
const PAGE_RAIL: Point = Point::new(230, 118);

/// A menu's heading, with the baseline it sits on.
///
/// The baseline is stated rather than derived from the first row: a title is
/// set in a different rhythm than the rows below it, and deriving it would move
/// every heading whenever a row's spacing changed.
pub struct MenuTitle {
    pub text: &'static str,
    pub baseline_y: i32,
}

/// Marker in the right-hand column of a row that leads somewhere.
const LEADS_ON: &str = ">";
/// Marker in the right-hand column of the chosen option.
const CHOSEN: &str = "*";

/// What a row is, which decides both what its right-hand column shows and what
/// choosing it means.
///
/// A row naming its own consequence is what lets a screen drop the table that
/// used to map a row's position onto an action - and a position-to-action table
/// is exactly the thing that goes wrong when a row is inserted above it.
#[derive(Clone, Copy)]
pub enum MenuRow {
    /// Opens another screen. The menu resolves this itself.
    Navigate {
        label: &'static str,
        target: ScreenId,
    },
    /// Shows a value the screen supplies. Choosing it is the screen's business.
    Value { label: &'static str },
    /// One of the values a setting can take, marked while it is active.
    Choice { label: &'static str },
}

impl MenuRow {
    const fn label(&self) -> &'static str {
        match self {
            Self::Navigate { label, .. } | Self::Value { label } | Self::Choice { label } => label,
        }
    }
}

/// What a menu's right-hand column draws from.
///
/// One value for the whole menu rather than one per row, because a menu is
/// homogeneous in practice: a root of navigation rows, a screen of values, a
/// picker of choices. A mixed menu still works - each row reads what its own
/// kind calls for and ignores the rest.
#[derive(Clone, Copy)]
pub enum MenuColumn<'a> {
    /// The right-hand column of each entry, in entry order.
    Values(&'a [&'a str]),
    /// One bit per entry that is currently chosen.
    Selections(u32),
}

/// What drawing needs to know about the showing page.
///
/// A concrete type rather than the row count generic `MenuState` carries, so
/// the drawing below is compiled once however many menus of however many rows
/// the firmware grows.
#[derive(Clone, Copy)]
pub struct MenuPage<'a> {
    pub list: &'a PagedList,
    /// The slot under a finger, if any.
    pub pressed: Option<usize>,
}

/// What an event did to a menu.
pub enum MenuOutcome {
    None,
    /// A navigation row was chosen; its target came from the description.
    Navigate(ScreenId),
    /// A value or choice row was chosen, by entry index.
    Chose(usize),
}

/// The fixed part of a menu: everything that does not depend on state.
///
/// Held as a `'static` description so a screen costs a constant rather than a
/// function. What the rows show is supplied per draw; see [`draw`].
pub struct Menu {
    /// Shown above the rows. `None` leaves the space to them.
    pub title: Option<MenuTitle>,
    /// Top edge of the first row on every page.
    pub first_row_y: i32,
    /// Distance between the top edges of consecutive rows.
    pub row_step: i32,
    /// The entries, in order. Entries beyond one page are reached by paging.
    pub rows: &'static [MenuRow],
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
/// [`Paint::draw_full`]: crate::screen::Paint::draw_full
pub fn draw(
    menu: &Menu,
    page: MenuPage<'_>,
    column: MenuColumn<'_>,
    canvas: &mut Canvas<'_>,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    canvas.clear(theme::BACKGROUND)?;
    keep_alive();

    if let Some(title) = &menu.title {
        draw_text(title.text, title.baseline_y, canvas)?;
        keep_alive();
    }

    draw_rows(menu, page, column, canvas, keep_alive)?;
    draw_text(menu.hint, HINT_BASELINE_Y, canvas)
}

/// Repaints the rows and the page rail, for a screen whose values moved.
///
/// The title and the hint do not change with state, so a value that ticks costs
/// its rows and nothing else. Slots the page does not fill are cleared rather
/// than skipped, so a short last page cannot leave the previous page's rows
/// standing under it.
///
/// The rail belongs to this pass rather than to [`draw`]: turning a page is the
/// one thing that moves it, and turning a page comes through here.
pub fn draw_rows(
    menu: &Menu,
    page: MenuPage<'_>,
    column: MenuColumn<'_>,
    canvas: &mut Canvas<'_>,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    for slot in 0..page.list.per_page() {
        let y = menu.slot_y(slot);
        match page
            .list
            .entry_at(slot)
            .and_then(|entry| menu.rows.get(entry).map(|row| (entry, row)))
        {
            Some((entry, row)) => draw_row(
                canvas,
                y,
                row.label(),
                column_of(row, entry, column),
                page.pressed == Some(slot),
            )?,
            None => clear_slot(y, canvas)?,
        }
        keep_alive();
    }
    draw_page_marks(canvas, PageAxis::Vertical, page.list, PAGE_RAIL)
}

/// What a row's right-hand column reads, given what the screen supplied.
fn column_of<'a>(row: &MenuRow, entry: usize, column: MenuColumn<'a>) -> &'a str {
    match (row, column) {
        (MenuRow::Navigate { .. }, _) => LEADS_ON,
        (MenuRow::Value { .. }, MenuColumn::Values(values)) => {
            values.get(entry).copied().unwrap_or("")
        }
        (MenuRow::Choice { .. }, MenuColumn::Selections(selections))
            if entry < 32 && selections & (1_u32 << entry) != 0 =>
        {
            CHOSEN
        }
        // An unchosen option, and a row whose kind and column disagree, both
        // draw an empty column rather than a wrong one.
        _ => "",
    }
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
                menu.rows.len(),
            ),
            dirty: false,
        }
    }

    /// Which page is showing, what it holds, and what is under a finger.
    #[must_use]
    pub fn page(&self) -> MenuPage<'_> {
        MenuPage {
            list: self.slots.list(),
            pressed: (0..N).find(|&slot| self.slots.slot_state(slot) == Some(ButtonState::Pressed)),
        }
    }

    /// Feeds an event to the page, reporting what the chosen row means.
    ///
    /// A navigation row is resolved from the description here, so a screen only
    /// hears about the rows whose effect is its own business.
    pub fn handle(&mut self, menu: &Menu, event: AppEvent) -> MenuOutcome {
        match self.slots.handle_event(event) {
            ListOutcome::Activated(entry) => {
                self.dirty = true;
                match menu.rows.get(entry) {
                    Some(MenuRow::Navigate { target, .. }) => MenuOutcome::Navigate(*target),
                    Some(MenuRow::Value { .. } | MenuRow::Choice { .. }) => {
                        MenuOutcome::Chose(entry)
                    }
                    None => MenuOutcome::None,
                }
            }
            // A new page obviously needs painting, and so does a press: a row
            // carries a pressed fill, so the finger going down changes what the
            // rows look like even though nothing was chosen.
            ListOutcome::Paged | ListOutcome::Redraw => {
                self.dirty = true;
                MenuOutcome::None
            }
            ListOutcome::None => {
                self.dirty = false;
                MenuOutcome::None
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
