use embedded_graphics::{
    Drawable,
    geometry::{Dimensions, Point, Size},
    pixelcolor::Rgb565,
    primitives::{Primitive, PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::ui::font::{AaTextStyle, ui_text};
use crate::ui::{
    canvas::{Canvas, CanvasError},
    theme,
};

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
const VALUE_X_OFFSET: i32 = 110;
const TEXT_BASELINE_OFFSET: i32 = 27;

/// Background spans that mask a filled rectangle's corners, innermost last.
///
/// Rounding this way costs a handful of one-pixel fills instead of the
/// rounded-rectangle rasteriser embedded-graphics would otherwise pull in, and
/// it is the same curve on a tile and on a row.
const CORNER_INSET: [i32; 5] = [5, 3, 2, 1, 1];

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
    draw_mono_text_visible(
        value,
        Point::new(ROW_X + VALUE_X_OFFSET, y + TEXT_BASELINE_OFFSET),
        ui_text(accent, fill),
        canvas,
    )
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
