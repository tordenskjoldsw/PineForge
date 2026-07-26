//! The application launcher: four tiles per page, opened by swiping up.
//!
//! What a tile means lives in the table below, not in the drawing code and not
//! in the list model: entries are indices everywhere else. Adding an
//! application is a row here plus its screen.

use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use pineforge_state::{
    AppEvent, ButtonBounds, ButtonState, ListOutcome, ListSlots, PageAxis, ScreenAction, ScreenId,
};

use crate::ui::canvas::{Canvas, CanvasError};
use crate::ui::{
    icons::{self, ICON_SIZE, Icon, draw_icon},
    render::{draw_mono_text_visible, draw_visible},
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
    #[cfg(feature = "diagnostics")]
    Tile {
        icon: &icons::CROSSHAIR,
        label: "TOUCH",
        target: ScreenId::TouchTest,
    },
];

const SLOTS: usize = 4;
const MARGIN: i32 = 4;
const GAP: i32 = 2;
const TILE_WIDTH: i32 = 115;
const TILE_HEIGHT: i32 = 106;
const TOP: i32 = STATUS_HEIGHT + 2;

/// The page indicator is drawn as squares rather than dots: a circle is a
/// rasteriser this firmware otherwise never needs, and at four pixels the shape
/// is indistinguishable anyway.
const DOT_SIZE: i32 = 4;
const DOT_SPACING: i32 = 10;
const DOT_Y: i32 = TOP + 2 * TILE_HEIGHT + GAP + 2;
/// Width of one character in `FONT_10X20`, for centring a label by hand.
const CHARACTER_WIDTH: i32 = 10;

const ICON_TOP_OFFSET: i32 = 24;
const LABEL_BASELINE_OFFSET: i32 = 78;

/// How far a filled corner is inset on each of its first rows, which rounds it.
///
/// A rounded rectangle primitive would rasterise this properly and cost a
/// rasteriser the firmware needs nowhere else; eight numbers and two fills per
/// row give the same silhouette at this size.
const CORNER_INSET: [i32; 5] = [5, 3, 2, 1, 1];

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

/// Paints the background back over the corners of a filled tile, which rounds
/// it without a rounded-rectangle rasteriser.
fn round_corners(area: &Rectangle, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
    let width = i32::try_from(area.size.width).unwrap_or(0);
    let height = i32::try_from(area.size.height).unwrap_or(0);
    for (row, inset) in CORNER_INSET.into_iter().enumerate() {
        let offset = layout_index(row);
        let size = Size::new(u32::try_from(inset).unwrap_or(0), 1);
        for y in [
            area.top_left.y + offset,
            area.top_left.y + height - 1 - offset,
        ] {
            for x in [area.top_left.x, area.top_left.x + width - inset] {
                draw_visible(
                    &Rectangle::new(Point::new(x, y), size)
                        .into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND)),
                    canvas,
                )?;
            }
        }
    }
    Ok(())
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
                MonoTextStyle::new(&FONT_10X20, ink),
                canvas,
            )?;
            keep_alive();
        }
        Ok(())
    }

    /// One dot per page, the current one filled. Absent for a single page:
    /// there is nothing to indicate.
    fn draw_pages(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let list = self.slots.list();
        let pages = list.page_count();
        draw_visible(
            &Rectangle::new(
                Point::new(0, DOT_Y),
                Size::new(240, u32::try_from(DOT_SIZE).unwrap_or(0)),
            )
            .into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND)),
            canvas,
        )?;
        if pages < 2 {
            return Ok(());
        }

        let span = layout_index(pages) * DOT_SPACING;
        let first = 120 - span / 2 + (DOT_SPACING - DOT_SIZE) / 2;
        let size = u32::try_from(DOT_SIZE).unwrap_or(0);
        for page in 0..pages {
            let left = first + layout_index(page) * DOT_SPACING;
            let color = if page == list.page() {
                theme::ACCENT
            } else {
                theme::FRAME
            };
            draw_visible(
                &Rectangle::new(Point::new(left, DOT_Y), Size::new(size, size))
                    .into_styled(PrimitiveStyle::with_fill(color)),
                canvas,
            )?;
        }
        Ok(())
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
