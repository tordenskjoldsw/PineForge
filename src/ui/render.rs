use embedded_graphics::{
    Drawable,
    draw_target::DrawTarget,
    geometry::{Dimensions, Size},
};

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
