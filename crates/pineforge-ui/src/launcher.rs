//! The application launcher: four tiles per page, opened by swiping up.
//!
//! What a tile means lives in the table below, not in the drawing code and not
//! in the list model: entries are indices everywhere else. Adding an
//! application is a row here plus its screen.

use embedded_graphics::{
    draw_target::DrawTarget,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use pineforge_state::{
    AppEvent, ButtonBounds, ButtonState, ListOutcome, ListSlots, PageAxis, ScreenAction, ScreenId,
};

use crate::canvas::{Canvas, CanvasError};
use crate::font::{JETBRAINS_MONO_10X22, ui_text};
use crate::{
    icons::{self, ICON_SIZE, Icon, draw_icon},
    render::{draw_mono_text_visible, draw_page_marks, draw_visible, round_corners},
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

/// One tile: what it shows, what it says, and where it goes.
struct Tile {
    icon: &'static Icon,
    label: &'static str,
    target: ScreenId,
}

const TILES: &[Tile] = &[
    Tile {
        icon: &icons::GEAR,
        label: "SETTINGS",
        target: ScreenId::DisplaySettings,
    },
    Tile {
        icon: &icons::CHIP,
        label: "FIRMWARE",
        target: ScreenId::Firmware,
    },
    Tile {
        icon: &icons::INFO,
        label: "ABOUT",
        target: ScreenId::About,
    },
    Tile {
        icon: &icons::TORCH,
        label: "LIGHT",
        target: ScreenId::Flashlight,
    },
    #[cfg(feature = "diagnostics")]
    Tile {
        icon: &icons::CROSSHAIR,
        label: "TOUCH",
        target: ScreenId::TouchTest,
    },
];

const SLOTS: usize = 4;

// The grid deliberately does not reach the edges of the panel.
//
// It used to: four to a page at 115x106, with a two-pixel gap and a four-pixel
// margin, came to exactly 240 across. Every tile was then as large as the
// layout could make it, which is what made the launcher feel heavy rather than
// any one tile being wrong. `InfiniTime` leaves 16 pixels across and 60 down
// unused and reads as tidy at a much smaller tile.
//
// The count stays at four. `InfiniTime` shows six because it has around ten
// applications; this build has three, four with diagnostics, so a six-slot page
// would be mostly empty - borrowing the proportions is worth more here than
// borrowing the grid.
const MARGIN: i32 = 10;
const GAP: i32 = 10;
const TILE_WIDTH: i32 = 105;
const TILE_HEIGHT: i32 = 88;
/// Clear of the status corner rather than flush against it.
const TOP: i32 = STATUS_HEIGHT + 10;

/// Where the tiles stop, and the strip below them begins.
const GRID_BOTTOM: i32 = TOP + 2 * TILE_HEIGHT + GAP;
/// Centre line of the page rail, in the strip the tiles leave below them.
/// Placed so the rail's thickness clears the bottom edge of the panel.
const DOT_Y: i32 = GRID_BOTTOM + 14;
/// Width of one character in the UI face, for centring a label by hand.
const CHARACTER_WIDTH: i32 = JETBRAINS_MONO_10X22.cell.width.cast_signed();

// Kept proportional to the tile as it shrank, so the icon still sits above the
// label with the label nearer the foot than the icon is to the head.
const ICON_TOP_OFFSET: i32 = 18;
const LABEL_BASELINE_OFFSET: i32 = 66;

/// Where a slot sits on the page. Slot 0 is top-left, then across and down.
const fn tile_bounds(slot: usize) -> ButtonBounds {
    let column = layout_index(slot % 2);
    let row = layout_index(slot / 2);
    ButtonBounds::new(
        MARGIN + column * (TILE_WIDTH + GAP),
        TOP + row * (TILE_HEIGHT + GAP),
        TILE_WIDTH,
        TILE_HEIGHT,
    )
}

/// Positions are small by construction - there are four slots and a handful of
/// pages - so the conversions below cannot lose anything.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
const fn layout_index(value: usize) -> i32 {
    value as i32
}

pub struct LauncherScreen {
    slots: ListSlots<SLOTS>,
    dirty: bool,
}

impl Default for LauncherScreen {
    fn default() -> Self {
        Self {
            slots: ListSlots::new(
                [
                    tile_bounds(0),
                    tile_bounds(1),
                    tile_bounds(2),
                    tile_bounds(3),
                ],
                // Opened by swiping up, so navigation owns the vertical axis
                // and the pages turn horizontally.
                PageAxis::Horizontal,
                TILES.len(),
            ),
            dirty: false,
        }
    }
}

impl LauncherScreen {
    fn draw_tiles(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        for slot in 0..SLOTS {
            let bounds = tile_bounds(slot);
            let area = Rectangle::new(
                Point::new(bounds.x(), bounds.y()),
                Size::new(
                    u32::try_from(bounds.width()).unwrap_or(0),
                    u32::try_from(bounds.height()).unwrap_or(0),
                ),
            );
            let Some(entry) = self.slots.list().entry_at(slot) else {
                // An empty slot still paints: the surface has to be opaque, and
                // the page underneath must not show through.
                draw_visible(
                    &area.into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND)),
                    canvas,
                )?;
                continue;
            };
            let pressed = self.slots.slot_state(slot) == Some(ButtonState::Pressed);
            let (fill, ink) = if pressed {
                (theme::ACCENT, theme::BACKGROUND)
            } else {
                (theme::SURFACE, theme::TEXT)
            };
            draw_visible(&area.into_styled(PrimitiveStyle::with_fill(fill)), canvas)?;
            round_corners(&area, canvas)?;

            draw_icon(
                TILES[entry].icon,
                Point::new(
                    bounds.x() + (TILE_WIDTH - ICON_SIZE) / 2,
                    bounds.y() + ICON_TOP_OFFSET,
                ),
                if pressed {
                    theme::BACKGROUND
                } else {
                    theme::ACCENT
                },
                canvas,
            )?;

            let label = TILES[entry].label;
            let text_width = layout_index(label.len()) * CHARACTER_WIDTH;
            draw_mono_text_visible(
                label,
                Point::new(
                    bounds.x() + (TILE_WIDTH - text_width) / 2,
                    bounds.y() + LABEL_BASELINE_OFFSET,
                ),
                ui_text(ink, fill),
                canvas,
            )?;
            keep_alive();
        }
        Ok(())
    }

    /// The page rail, drawn along the axis this screen pages on.
    ///
    /// Only the position is local; the marks and the clearing behind them are
    /// the shared component every paginated screen uses.
    fn draw_pages(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        draw_page_marks(
            canvas,
            PageAxis::Horizontal,
            self.slots.list(),
            Point::new(120, DOT_Y),
        )
    }
}

impl Paint for LauncherScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        canvas.clear(theme::BACKGROUND)?;
        keep_alive();
        self.draw_tiles(canvas, keep_alive)?;
        self.draw_pages(canvas)
    }
}

impl Screen for LauncherScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        self.dirty = false;
        match self.slots.handle_event(event) {
            ListOutcome::Activated(entry) => ScreenAction::Push(TILES[entry].target),
            ListOutcome::Redraw | ListOutcome::Paged => {
                self.dirty = true;
                ScreenAction::None
            }
            ListOutcome::None => ScreenAction::None,
        }
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if !self.dirty {
            return Ok(());
        }
        self.draw_tiles(canvas, keep_alive)?;
        self.draw_pages(canvas)
    }
}
