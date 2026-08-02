//! The application launcher: four tiles per page, opened by swiping up and
//! paged by carrying on upwards.
//!
//! Up and down do everything here. The gesture that opened the launcher keeps
//! going through its pages, and the first page is where down means the
//! watchface again rather than the page before. It is worth the special case in
//! [`Screen::claims`]: the panel's controller can only slide its picture
//! vertically, so pages that turn this way are the ones that can be slid rather
//! than swept in.
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
    SwipeDirection,
};

use crate::canvas::{Canvas, CanvasError};
use crate::font::{LIBERATION_MONO_10X22, ui_text};
use crate::{
    icons::{self, ICON_SIZE, Icon, draw_icon},
    render::{PANEL, draw_mono_text_visible, draw_page_marks, draw_visible, round_corners},
    screen::{Dirty, Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

/// The panel's height, for centring the grid in what the corner leaves.
const PANEL_HEIGHT: i32 = PANEL.size.height.cast_signed();

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
    Tile {
        icon: &icons::HEART,
        label: "PULSE",
        target: ScreenId::Pulse,
    },
    Tile {
        icon: &icons::FOOT,
        label: "STEPS",
        target: ScreenId::Steps,
    },
    Tile {
        icon: &icons::NOTE,
        label: "MUSIC",
        target: ScreenId::Music,
    },
    Tile {
        icon: &icons::STOPWATCH,
        label: "STOPWATCH",
        target: ScreenId::Stopwatch,
    },
    Tile {
        icon: &icons::TIMER,
        label: "TIMER",
        target: ScreenId::Timer,
    },
    Tile {
        icon: &icons::CLOCK,
        label: "TIME",
        target: ScreenId::Time,
    },
    Tile {
        icon: &icons::CALENDAR,
        label: "DATE",
        target: ScreenId::Date,
    },
    Tile {
        icon: &icons::BLUETOOTH,
        label: "BLUETOOTH",
        target: ScreenId::Bluetooth,
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

/// How tall the two rows of tiles are together.
const GRID_HEIGHT: i32 = 2 * TILE_HEIGHT + GAP;

/// Where the tiles begin, below the status corner.
///
/// Centred in what the corner leaves rather than set at a fixed distance from
/// it. The rail moved to the right-hand edge when the pages did, which left the
/// strip under the grid empty; splitting that space above and below is what
/// keeps the grid from sitting high on the panel with a gap beneath it.
const TOP: i32 = STATUS_HEIGHT + (PANEL_HEIGHT - STATUS_HEIGHT - GRID_HEIGHT) / 2;

/// Centre of the page rail, in the strip the tiles leave to their right.
///
/// The rail runs along the axis the pages turn on, which is now the vertical
/// one, so it stands beside the grid instead of lying under it.
const RAIL_CENTER: Point = Point::new(MARGIN + 2 * TILE_WIDTH + GAP + 5, TOP + GRID_HEIGHT / 2);
/// Width of one character in the UI face, for centring a label by hand.
const CHARACTER_WIDTH: i32 = LIBERATION_MONO_10X22.cell.width.cast_signed();

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
    dirty: Dirty,
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
                // The pages turn the way the launcher was opened: on up, which
                // brought it in, and back down again. That axis is also the way
                // out, so it is shared rather than taken - see `claims` below,
                // which is what keeps the watchface one swipe from the first
                // page instead of unreachable.
                PageAxis::Vertical,
                TILES.len(),
            ),
            dirty: Dirty::Nothing,
        }
    }
}

impl LauncherScreen {
    /// Paints the tiles `paint` covers, leaving the rest of the page alone.
    fn draw_tiles(
        &self,
        paint: Dirty,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        for slot in 0..SLOTS {
            if !paint.covers(slot) {
                continue;
            }
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
        draw_page_marks(canvas, PageAxis::Vertical, self.slots.list(), RAIL_CENTER)
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
        self.draw_tiles(Dirty::Everything, canvas, keep_alive)?;
        self.draw_pages(canvas)
    }
}

impl Screen for LauncherScreen {
    /// The swipe is the launcher's for as long as it has a page that way.
    ///
    /// Down from the first page is deliberately not claimed: that is the
    /// watchface, and the way back to it has to stay one gesture from where the
    /// launcher opens.
    fn claims(&self, direction: SwipeDirection) -> bool {
        self.slots.claims(direction)
    }

    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        self.dirty = Dirty::Nothing;
        match self.slots.handle_event(event) {
            ListOutcome::Activated(entry) => ScreenAction::Push(TILES[entry].target),
            // A press and its release each move one tile, and the list says
            // which. Only a count it cannot attribute falls back to the page.
            ListOutcome::Redraw(slot) => {
                self.dirty = slot.map_or(Dirty::Everything, Dirty::Slot);
                ScreenAction::None
            }
            ListOutcome::Paged(motion) => {
                self.dirty = Dirty::Everything;
                // A page that moved under a finger is slid in; one that moved
                // because its entries changed has nowhere to have come from.
                motion.map_or(ScreenAction::None, ScreenAction::Paged)
            }
            ListOutcome::None => ScreenAction::None,
        }
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.dirty.is_clean() {
            return Ok(());
        }
        self.draw_tiles(self.dirty, canvas, keep_alive)?;
        // Only paging moves the rail, and paging is the whole-page case. A tile
        // taking or losing its pressed fill leaves it exactly as it was.
        if self.dirty == Dirty::Everything {
            self.draw_pages(canvas)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use embedded_graphics::{
        geometry::{Point, Size},
        primitives::Rectangle,
    };
    use pineforge_state::{AppEvent, ScreenAction, SwipeDirection};

    use super::{LauncherScreen, RAIL_CENTER, SLOTS, TILE_WIDTH, tile_bounds};
    use crate::{canvas::Canvas, probe::Probe, screen::Screen};

    /// The panel area one tile occupies.
    fn area_of(slot: usize) -> Rectangle {
        let bounds = tile_bounds(slot);
        Rectangle::new(
            Point::new(bounds.x(), bounds.y()),
            Size::new(
                u32::try_from(bounds.width()).unwrap(),
                u32::try_from(bounds.height()).unwrap(),
            ),
        )
    }

    /// A touch at the middle of a tile.
    fn touch(slot: usize, pressed: bool) -> AppEvent {
        let bounds = tile_bounds(slot);
        AppEvent::Touch {
            x: bounds.x() + bounds.width() / 2,
            y: bounds.y() + bounds.height() / 2,
            pressed,
        }
    }

    /// What the finger costs.
    ///
    /// Nothing buffers pixels, so a repaint is an SPI transfer of exactly the
    /// area it names. Pressing one tile changes one tile; repainting the page
    /// was four times the transfer for the same picture, and at 8 MHz that is
    /// tens of milliseconds the highlight visibly lags behind the finger.
    #[test]
    fn pressing_a_tile_repaints_that_tile_and_no_other() {
        for pressed_slot in 0..SLOTS {
            let mut launcher = LauncherScreen::default();
            let _ = launcher.handle_event(touch(pressed_slot, true));

            let mut probe = Probe::new();
            launcher
                .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
                .expect("the probe accepts every operation");

            assert!(
                probe.painted_within(area_of(pressed_slot)),
                "slot {pressed_slot} took the press and must be repainted"
            );
            for other in (0..SLOTS).filter(|slot| *slot != pressed_slot) {
                assert!(
                    !probe.painted_within(area_of(other)),
                    "slot {other} did not move but was repainted \
                     while slot {pressed_slot} was pressed"
                );
            }
        }
    }

    /// Releasing un-presses the tile, which is again one tile's worth of work.
    #[test]
    fn releasing_repaints_only_the_tile_that_was_pressed() {
        let mut launcher = LauncherScreen::default();
        let _ = launcher.handle_event(touch(1, true));
        let _ = launcher.handle_event(AppEvent::TouchCancelled);

        let mut probe = Probe::new();
        launcher
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");

        assert!(probe.painted_within(area_of(1)));
        for other in [0, 2, 3] {
            assert!(
                !probe.painted_within(area_of(other)),
                "slot {other} was repainted for a release it had no part in"
            );
        }
    }

    /// The one gesture the launcher must never keep for itself.
    ///
    /// It pages on the axis it was opened on, which is only safe because the
    /// first page hands the downward swipe back: that is the way to the
    /// watchface, and it has to be one gesture from where the launcher opens.
    #[test]
    fn the_first_page_leaves_the_way_back_to_the_watchface() {
        let mut launcher = LauncherScreen::default();

        assert!(!launcher.claims(SwipeDirection::Down));
        assert!(launcher.claims(SwipeDirection::Up));

        // Paging reports which way it went, because that is what decides which
        // edge the page is slid in from.
        assert_eq!(
            launcher.handle_event(AppEvent::Swipe(SwipeDirection::Up)),
            ScreenAction::Paged(SwipeDirection::Up)
        );
        // A page in, down is the page before rather than the way out.
        assert!(launcher.claims(SwipeDirection::Down));
        assert_eq!(
            launcher.handle_event(AppEvent::Swipe(SwipeDirection::Down)),
            ScreenAction::Paged(SwipeDirection::Down)
        );
        assert!(!launcher.claims(SwipeDirection::Down));
    }

    /// The rail moved to the right-hand edge when the pages turned vertical,
    /// and it has to stand clear of the tiles it reports on.
    #[test]
    fn the_page_rail_stands_beside_the_grid_rather_than_over_it() {
        let right_edge = tile_bounds(1).x() + TILE_WIDTH;

        assert!(
            RAIL_CENTER.x > right_edge,
            "the rail at {} overlaps the tiles, which end at {right_edge}",
            RAIL_CENTER.x
        );
        assert!(RAIL_CENTER.x < super::PANEL.size.width.cast_signed());
    }
}
