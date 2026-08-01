use embedded_graphics::{
    Drawable,
    geometry::{Dimensions, Point, Size},
    pixelcolor::Rgb565,
    primitives::{Primitive, PrimitiveStyle, Rectangle},
    text::Text,
};

use pineforge_state::{PageAxis, PagedList};

use crate::font::{AaTextStyle, JETBRAINS_MONO_6X14, hint_text, ui_text};
use crate::{
    canvas::{Canvas, CanvasError},
    theme,
};

/// Width of one character of the hint face, for centring a line by hand.
const HINT_WIDTH: i32 = JETBRAINS_MONO_6X14.cell.width.cast_signed();

/// A filled rectangle in panel coordinates, skipped when it has no area.
///
/// Every screen that paints its own bands rather than blanking the panel first
/// wants this, and it lives here because two of them had written it out
/// separately. An empty or negative extent is what layout arithmetic produces
/// when it runs out of room, and drawing nothing is what that means.
pub fn fill(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Rgb565,
) -> Result<(), CanvasError> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }
    Rectangle::new(
        Point::new(x, y),
        Size::new(width.unsigned_abs(), height.unsigned_abs()),
    )
    .into_styled(PrimitiveStyle::with_fill(color))
    .draw(canvas)
}

/// One line of the hint face, centred across the panel.
pub fn draw_centred(
    text: &str,
    baseline: i32,
    ink: Rgb565,
    canvas: &mut Canvas<'_>,
) -> Result<(), CanvasError> {
    let width = i32::try_from(text.len()).unwrap_or(0) * HINT_WIDTH;
    draw_mono_text_visible(
        text,
        Point::new((PANEL.size.width.cast_signed() - width) / 2, baseline),
        hint_text(ink, theme::BACKGROUND),
        canvas,
    )
}

/// The panel, as a rectangle to clip and measure against.
///
/// Written once here rather than at each screen that needs to reach an edge.
/// It is not a display-driver constant - the driver knows its own size - but
/// the layouts are `const`, and a `const` cannot ask the canvas how big it is.
pub const PANEL: Rectangle = Rectangle::new(Point::new(0, 0), Size::new(240, 240));

/// Geometry of the raised label/value row every menu screen is built from.
///
/// The rows live here rather than in one screen because a second screen using a
/// different face or a different value column would read as a different
/// product.
///
/// A row is a button, not a framed strip: it carries the same rounded fill as a
/// launcher tile, and it darkens under a finger the way a tile does. An outline
/// would say "field"; a filled shape says "press me", and pressing is the only
/// thing a menu row is for.
pub const ROW_X: i32 = 20;
pub const ROW_WIDTH: i32 = 200;
pub const ROW_HEIGHT: i32 = 42;
const LABEL_X_OFFSET: i32 = 12;
/// How far the right-hand column stops short of the row's right edge.
///
/// The same inset the label keeps from the left, so a row reads as one object
/// with even margins. It used to be a fixed offset of 110 from the left, which
/// on a 200-wide row put every marker ten pixels right of centre - close enough
/// to the middle to read as centred, and nowhere near the values it shares the
/// column with.
const VALUE_RIGHT_INSET: i32 = 12;
const TEXT_BASELINE_OFFSET: i32 = 27;

/// Background spans that mask a filled rectangle's corners, innermost last.
///
/// Rounding this way costs a handful of one-pixel fills instead of the
/// rounded-rectangle rasteriser embedded-graphics would otherwise pull in, and
/// it is the same curve on a tile and on a row.
const CORNER_INSET: [i32; 5] = [5, 3, 2, 1, 1];

/// A filled track: the completed part in `ink`, the rest in `SURFACE`, and the
/// row curve on both ends.
///
/// One bar, wherever something reports how far through it is - a firmware
/// update, a first-boot format, a track playing. It lived beside the update
/// screen until the music app needed the same shape, and a second copy of it
/// would have been two bars that drift apart rather than one that is shared.
///
/// The unfilled part is `SURFACE` rather than the background, which is the
/// difference between a bar and an outlined empty box. A track that is visible
/// the whole way across says how far there is left to go; an outline around
/// nothing only says where the bar would be if it had one.
///
/// Both halves are painted rather than only the filled one, so a bar that ever
/// moves backwards - a transfer restarted, a track seeked - does not keep the
/// fill it had. The rounding happens here too, because this is also the partial
/// path: corners masked by the caller once would be filled straight back in.
pub fn draw_progress_bar(
    canvas: &mut Canvas<'_>,
    area: &Rectangle,
    filled: u32,
    ink: Rgb565,
) -> Result<(), CanvasError> {
    let filled = filled.min(area.size.width).cast_signed();
    let width = area.size.width.cast_signed();
    let height = area.size.height.cast_signed();
    fill(
        canvas,
        area.top_left.x,
        area.top_left.y,
        filled,
        height,
        ink,
    )?;
    fill(
        canvas,
        area.top_left.x + filled,
        area.top_left.y,
        width - filled,
        height,
        theme::SURFACE,
    )?;
    round_corners(area, canvas)
}

/// Paints the background back over the corners of a filled area.
pub fn round_corners(area: &Rectangle, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
    let width = i32::try_from(area.size.width).unwrap_or(0);
    let height = i32::try_from(area.size.height).unwrap_or(0);
    for (row, inset) in CORNER_INSET.into_iter().enumerate() {
        let offset = i32::try_from(row).unwrap_or(0);
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

/// Draws one row as a rounded button: a label on the left, a value on the right.
///
/// `pressed` swaps fill and ink rather than tinting them, which is the loudest
/// feedback a panel without animation can give and the same one a launcher tile
/// uses.
pub fn draw_row(
    canvas: &mut Canvas<'_>,
    y: i32,
    label: &str,
    value: &str,
    pressed: bool,
) -> Result<(), CanvasError> {
    let (fill, ink, accent) = if pressed {
        (theme::ACCENT, theme::BACKGROUND, theme::BACKGROUND)
    } else {
        (theme::SURFACE, theme::TEXT, theme::ACCENT)
    };
    let bounds = Rectangle::new(
        Point::new(ROW_X, y),
        Size::new(ROW_WIDTH as u32, ROW_HEIGHT as u32),
    );
    draw_visible(&bounds.into_styled(PrimitiveStyle::with_fill(fill)), canvas)?;
    round_corners(&bounds, canvas)?;
    draw_mono_text_visible(
        label,
        Point::new(ROW_X + LABEL_X_OFFSET, y + TEXT_BASELINE_OFFSET),
        ui_text(ink, fill),
        canvas,
    )?;
    let style = ui_text(accent, fill);
    let cell = i32::try_from(style.font.cell.width).unwrap_or(0);
    draw_mono_text_visible(
        value,
        Point::new(value_origin_x(value, cell), y + TEXT_BASELINE_OFFSET),
        style,
        canvas,
    )
}

/// Where a row's right-hand column starts.
///
/// Right-aligned, so a marker sits against the edge and values of different
/// lengths line up by their last character rather than their first. A value too
/// long for the space overlaps the label rather than spilling onto the
/// background outside the row, which is the more legible of the two failures.
fn value_origin_x(value: &str, cell_width: i32) -> i32 {
    let width = i32::try_from(value.len()).unwrap_or(0) * cell_width;
    (ROW_X + ROW_WIDTH - VALUE_RIGHT_INSET - width).max(ROW_X + LABEL_X_OFFSET)
}

/// Thickness of a page mark across the rail it sits on.
///
/// Six rather than four, which is the same step the rows are spaced by and the
/// same depth the corner mask cuts. Four was thin enough that the rail read as
/// a scratch on the panel rather than as a control's worth of information.
const MARK_THICKNESS: i32 = 6;
/// Length of a page that is not the one showing: a square.
const MARK_DOT: i32 = 6;
/// Length of the page that is - four times the square, so position is legible
/// from the shape alone.
const MARK_ACTIVE: i32 = 24;
const MARK_GAP: i32 = 6;

/// Length of the whole run of marks.
///
/// Takes only the page count, and that is the property the clearing rests on:
/// exactly one mark is long whichever page shows, so the run keeps its length
/// and its box while the long mark moves about inside it.
const fn rail_span(pages: i32) -> i32 {
    MARK_ACTIVE + (pages - 1) * (MARK_DOT + MARK_GAP)
}

const fn mark_length(showing: bool) -> i32 {
    if showing { MARK_ACTIVE } else { MARK_DOT }
}

/// Draws which page of a list is showing, as a rail of marks.
///
/// The rail runs along the axis the list pages on, because that is the whole
/// content of the message: a column of marks beside a list that pages up and
/// down says "there is more that way", while the same marks in a row underneath
/// would point across a movement that never goes across.
///
/// The showing page is a bar and the rest are squares, rather than all of them
/// squares in two colours. Two treatments carry it where colour alone has to be
/// seen accurately at backlight level 1 to carry anything, and the long mark
/// still reads as position when the palette is barely there.
///
/// Squares because a circle is a rasteriser this firmware needs nowhere else,
/// and at four pixels the two shapes are indistinguishable anyway.
///
/// Nothing is drawn for a single page. There is no position to report when
/// there is only one place to be, and a lone mark would read as a control.
pub fn draw_page_marks(
    canvas: &mut Canvas<'_>,
    axis: PageAxis,
    list: &PagedList,
    center: Point,
) -> Result<(), CanvasError> {
    let pages = list.page_count();
    let span = rail_span(i32::try_from(pages).unwrap_or(1));

    // Cleared before anything is drawn, and on the paths that draw nothing.
    // The run keeps its length but the long mark moves inside it, so a partial
    // redraw that only painted the new marks would leave the tail of the old
    // one standing - a stripe of accent beside a row it no longer refers to.
    let (clear_at, clear_size) = match axis {
        PageAxis::Horizontal => (
            Point::new(center.x - span / 2, center.y - MARK_THICKNESS / 2),
            Size::new(as_u32(span), as_u32(MARK_THICKNESS)),
        ),
        PageAxis::Vertical => (
            Point::new(center.x - MARK_THICKNESS / 2, center.y - span / 2),
            Size::new(as_u32(MARK_THICKNESS), as_u32(span)),
        ),
    };
    draw_visible(
        &Rectangle::new(clear_at, clear_size)
            .into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND)),
        canvas,
    )?;
    if pages < 2 {
        return Ok(());
    }

    let mut along = -span / 2;
    for page in 0..pages {
        let showing = page == list.page();
        let length = mark_length(showing);
        let (top_left, size) = match axis {
            PageAxis::Horizontal => (
                Point::new(center.x + along, center.y - MARK_THICKNESS / 2),
                Size::new(as_u32(length), as_u32(MARK_THICKNESS)),
            ),
            PageAxis::Vertical => (
                Point::new(center.x - MARK_THICKNESS / 2, center.y + along),
                Size::new(as_u32(MARK_THICKNESS), as_u32(length)),
            ),
        };
        let color = if showing { theme::ACCENT } else { theme::FRAME };
        draw_visible(
            &Rectangle::new(top_left, size).into_styled(PrimitiveStyle::with_fill(color)),
            canvas,
        )?;
        along += length + MARK_GAP;
    }
    Ok(())
}

fn as_u32(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

/// Draws an object only when its bounds intersect the target's current region.
///
/// This is effectively free for a full display target and prevents needless
/// rasterization when rendering into a clipped transition tile.
pub fn draw_visible<T>(drawable: &T, target: &mut Canvas<'_>) -> Result<(), CanvasError>
where
    T: Drawable<Color = Rgb565> + Dimensions,
{
    if drawable
        .bounding_box()
        .intersection(&target.bounding_box())
        .size
        == Size::zero()
    {
        return Ok(());
    }

    drawable.draw(target).map(drop)
}

/// Draws only the visible character range of left-aligned ASCII monospace text.
///
/// A character on either side of the calculated range is retained to preserve
/// partially clipped glyphs and character spacing at tile boundaries.
pub fn draw_mono_text_visible(
    text: &str,
    position: Point,
    style: AaTextStyle,
    target: &mut Canvas<'_>,
) -> Result<(), CanvasError> {
    if !text.is_ascii() {
        return draw_visible(&Text::new(text, position, style), target);
    }

    let stride = i32::try_from(style.font.cell.width).unwrap_or(i32::MAX);
    let clip = target.bounding_box();
    let clip_left = clip.top_left.x;
    let clip_right = clip
        .top_left
        .x
        .saturating_add(i32::try_from(clip.size.width).unwrap_or(i32::MAX));

    let first = if clip_left <= position.x {
        0
    } else {
        usize::try_from((clip_left - position.x) / stride)
            .unwrap_or(0)
            .saturating_sub(1)
    };
    let end = if clip_right <= position.x {
        0
    } else {
        let distance = clip_right - position.x;
        usize::try_from(distance.saturating_add(stride - 1) / stride)
            .unwrap_or(text.len())
            .saturating_add(1)
    }
    .min(text.len());

    if first >= end {
        return Ok(());
    }

    let x = position.x.saturating_add(
        i32::try_from(first)
            .unwrap_or(i32::MAX)
            .saturating_mul(stride),
    );
    draw_visible(
        &Text::new(&text[first..end], Point::new(x, position.y), style),
        target,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the page rail's clearing rests on.
    ///
    /// Turning a page moves the long mark within the run without changing how
    /// much room the run needs, so one fixed box can be cleared before the
    /// marks are drawn. If a later change made a mark's length depend on where
    /// it sits, the box would stop covering the run and a partial redraw would
    /// leave the tail of the previous position standing beside the rows.
    #[test]
    fn a_page_rail_fills_the_same_run_whichever_page_shows() {
        for pages in 2..=8_usize {
            for showing in 0..pages {
                let mut along = 0;
                for page in 0..pages {
                    along += mark_length(page == showing);
                    if page + 1 < pages {
                        along += MARK_GAP;
                    }
                }
                assert_eq!(
                    along,
                    rail_span(i32::try_from(pages).unwrap()),
                    "{pages} pages showing {showing}"
                );
            }
        }
    }

    /// The showing page has to be told apart from the rest without relying on
    /// colour, which at the lowest backlight level is barely there.
    #[test]
    fn the_showing_page_is_the_long_mark() {
        assert!(mark_length(true) > mark_length(false));
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;

    /// One character of the UI face.
    const CELL: i32 = 10;

    /// A marker belongs against the right edge, not near the middle.
    ///
    /// It sat at a fixed offset of 110 into a 200-wide row - ten pixels right
    /// of centre, which reads as centred rather than as a column.
    #[test]
    fn the_right_hand_column_ends_at_the_rows_edge() {
        for value in [">", "*", "ON", "ROLLBACK"] {
            let right = value_origin_x(value, CELL) + i32::try_from(value.len()).unwrap() * CELL;
            assert_eq!(
                right,
                ROW_X + ROW_WIDTH - VALUE_RIGHT_INSET,
                "{value:?} did not end at the row's right inset"
            );
        }
    }

    /// The inset matches the label's, so a row reads as one object with even
    /// margins rather than as two columns that happen to share a box.
    #[test]
    fn both_margins_of_a_row_are_the_same() {
        assert_eq!(LABEL_X_OFFSET, VALUE_RIGHT_INSET);
    }

    /// A value with no room left stops at the label rather than running off the
    /// row onto the background.
    #[test]
    fn an_overlong_value_stays_inside_the_row() {
        let absurd = "0.2.1+34-WITH-A-VERY-LONG-SUFFIX";
        assert_eq!(value_origin_x(absurd, CELL), ROW_X + LABEL_X_OFFSET);
    }
}
