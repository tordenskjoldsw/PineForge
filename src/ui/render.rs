use embedded_graphics::{
    Drawable,
    draw_target::DrawTarget,
    geometry::{Dimensions, Point, Size},
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::{PixelColor, Rgb565, RgbColor, WebColors},
    primitives::{Primitive, PrimitiveStyleBuilder, Rectangle},
    text::Text,
};

/// Geometry of the framed label/value row every menu screen is built from.
///
/// The rows live here rather than in one screen because a second screen using a
/// different frame or a different value column would read as a different
/// product. The pagination model that eventually replaces the fixed rows starts
/// from this one drawing.
pub const ROW_X: i32 = 20;
pub const ROW_WIDTH: i32 = 200;
pub const ROW_HEIGHT: i32 = 34;
const LABEL_X_OFFSET: i32 = 8;
const VALUE_X_OFFSET: i32 = 110;
const TEXT_BASELINE_OFFSET: i32 = 23;

/// Draws one framed row: a white label on the left, a value in the right column.
pub fn draw_row<D>(display: &mut D, y: i32, label: &str, value: &str) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let bounds = Rectangle::new(
        Point::new(ROW_X, y),
        Size::new(ROW_WIDTH as u32, ROW_HEIGHT as u32),
    );
    bounds
        .into_styled(
            PrimitiveStyleBuilder::new()
                .fill_color(Rgb565::BLACK)
                .stroke_color(Rgb565::WHITE)
                .stroke_width(1)
                .build(),
        )
        .draw(display)?;
    draw_mono_text_visible(
        label,
        Point::new(ROW_X + LABEL_X_OFFSET, y + TEXT_BASELINE_OFFSET),
        MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
        display,
    )?;
    draw_mono_text_visible(
        value,
        Point::new(ROW_X + VALUE_X_OFFSET, y + TEXT_BASELINE_OFFSET),
        MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_ORANGE),
        display,
    )
}

/// Draws an object only when its bounds intersect the target's current region.
///
/// This is effectively free for a full display target and prevents needless
/// rasterization when rendering into a clipped transition tile.
pub fn draw_visible<T, D>(drawable: &T, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget,
    T: Drawable<Color = D::Color> + Dimensions,
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
pub fn draw_mono_text_visible<C, D>(
    text: &str,
    position: Point,
    style: MonoTextStyle<'_, C>,
    target: &mut D,
) -> Result<(), D::Error>
where
    C: PixelColor,
    D: DrawTarget<Color = C>,
{
    if !text.is_ascii() {
        return draw_visible(&Text::new(text, position, style), target);
    }

    let stride = style
        .font
        .character_size
        .width
        .saturating_add(style.font.character_spacing);
    let stride = i32::try_from(stride).unwrap_or(i32::MAX);
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
