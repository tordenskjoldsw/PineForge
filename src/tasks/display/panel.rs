//! The panel, and the part of its frame memory that is not on screen.
//!
//! The ST7789 holds 240x320 pixels and shows 240 rows of them at a time. Which
//! 240 is a register: `VSCSAD` names the memory row displayed at the top of the
//! panel, and the window wraps at the end of memory. That register is the whole
//! animation budget this firmware has. Pixels written into rows that are not
//! showing cost exactly what pixels written into rows that are cost, and moving
//! the window costs nothing at all - so a screen can be slid in for the price of
//! one frame instead of one frame per step, which is the difference between an
//! animation and a slideshow at 8 MHz.
//!
//! The price is paid here. After a slide the window no longer starts at memory
//! row 0, so screen row `y` lives at memory row `(origin + y) mod 320` and a run
//! of rows can fall off the end of memory and continue at the top. Every write
//! is translated and, where it has to be, split - and nothing above this module
//! is allowed to know. Screens keep drawing in screen coordinates.
//!
//! It sits with the display task rather than in `drivers` because it names the
//! board: the visible height is the panel that is fitted, not a property of the
//! controller, and only a task may say which watch this is.

use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{Dimensions, Point, Size},
    pixelcolor::Rgb565,
    primitives::Rectangle,
};
#[cfg(feature = "ui-animations")]
use pineforge_ui::transition::ScrollingPanel;

#[cfg(feature = "ui-animations")]
use super::Panel;
use crate::board::pins;

/// Rows of frame memory in the controller, of which the panel shows a window.
///
/// A property of the ST7789 and not of the watch: the same chip drives 240x320
/// displays, and a 240x240 one simply leaves 80 rows of memory with nowhere to
/// go. Those 80 rows are the staging area every slide is composed into.
pub const MEMORY_ROWS: u16 = 320;

/// Rows the panel shows at once.
pub const VISIBLE_ROWS: u16 = pins::DISPLAY_HEIGHT;

/// A rectangle of memory rows, as the panel underneath addresses them.
struct MemoryRun {
    row: u16,
    height: u16,
}

/// The panel, addressed in screen rows, wherever its window happens to sit.
pub struct ScrollPanel<P> {
    panel: P,
    /// The memory row shown at the top of the panel.
    origin: u16,
}

impl<P> ScrollPanel<P>
where
    P: DrawTarget<Color = Rgb565>,
{
    /// Wraps a panel whose drawable area is the whole of its frame memory.
    ///
    /// The window starts where the controller leaves it after reset, which is
    /// memory row 0, so screen and memory agree until the first slide.
    pub const fn new(panel: P) -> Self {
        Self { panel, origin: 0 }
    }

    /// Paints full-width rows at an absolute memory row, wrapping at the end.
    ///
    /// This is the one entry point that addresses memory rather than the
    /// screen, because it is the one thing a slide needs and nothing else may
    /// want: it writes where the panel is not looking.
    #[cfg(feature = "ui-animations")]
    fn write_memory_rows(
        &mut self,
        row: u16,
        pixels: &[Rgb565],
    ) -> Result<(), <P as DrawTarget>::Error> {
        let width = u32::from(pins::DISPLAY_WIDTH);
        let Ok(height) = u16::try_from(pixels.len() / width as usize) else {
            return Ok(());
        };
        let mut written = 0_usize;
        for run in memory_runs(row, height) {
            let area = Rectangle::new(
                Point::new(0, i32::from(run.row)),
                Size::new(width, u32::from(run.height)),
            );
            let count = width as usize * run.height as usize;
            self.panel
                .fill_contiguous(&area, pixels[written..written + count].iter().copied())?;
            written += count;
        }
        Ok(())
    }

    /// Paints every row of frame memory, including the ones off screen.
    ///
    /// The hidden rows come up holding whatever the controller powered on with,
    /// and the first slide would carry that into view for as long as it takes
    /// the incoming screen to cover it.
    pub fn clear_memory(&mut self, color: Rgb565) -> Result<(), <P as DrawTarget>::Error> {
        let area = Rectangle::new(
            Point::zero(),
            Size::new(u32::from(pins::DISPLAY_WIDTH), u32::from(MEMORY_ROWS)),
        );
        self.panel.fill_solid(&area, color)
    }

    /// The panel underneath, for the commands that are not drawing.
    pub const fn inner(&mut self) -> &mut P {
        &mut self.panel
    }

    /// Where a run of screen rows lives in memory, in at most two pieces.
    ///
    /// The area is expected to have been clipped to the panel already, so a row
    /// that does not convert is a bug elsewhere and draws nothing rather than
    /// landing at an arbitrary place in memory.
    fn runs_of(&self, area: &Rectangle) -> [MemoryRun; 2] {
        let top = u16::try_from(area.top_left.y).unwrap_or(0);
        let height = u16::try_from(area.size.height).unwrap_or(0);
        memory_runs(self.origin + top, height)
    }
}

/// What a slide needs, and the reason this wrapper exists at all.
///
/// The window register belongs to the panel underneath; the origin it leaves
/// behind belongs here, because every later draw is translated by it.
#[cfg(feature = "ui-animations")]
impl ScrollingPanel for ScrollPanel<Panel> {
    const MEMORY_ROWS: u16 = MEMORY_ROWS;

    fn origin(&self) -> u16 {
        self.origin
    }

    fn fill_memory_rows(&mut self, row: u16, pixels: &[Rgb565]) -> Result<(), Self::Error> {
        self.write_memory_rows(row, pixels)
    }

    fn show_from(&mut self, row: u16) -> Result<(), Self::Error> {
        let row = row % MEMORY_ROWS;
        self.panel.set_vertical_scroll_offset(row)?;
        self.origin = row;
        Ok(())
    }
}

/// Splits a run of memory rows where it falls off the end of memory.
///
/// The second piece is empty for a run that does not wrap, which is every run
/// until the first slide moves the window.
fn memory_runs(row: u16, height: u16) -> [MemoryRun; 2] {
    let row = row % MEMORY_ROWS;
    let first = height.min(MEMORY_ROWS - row);
    [
        MemoryRun { row, height: first },
        MemoryRun {
            row: 0,
            height: height - first,
        },
    ]
}

impl<P> Dimensions for ScrollPanel<P> {
    /// The panel, not its memory. Everything above draws on a 240x240 watch.
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(
            Point::zero(),
            Size::new(u32::from(pins::DISPLAY_WIDTH), u32::from(VISIBLE_ROWS)),
        )
    }
}

impl<P> DrawTarget for ScrollPanel<P>
where
    P: DrawTarget<Color = Rgb565>,
{
    type Color = Rgb565;
    type Error = <P as DrawTarget>::Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let origin = i32::from(self.origin);
        let memory = i32::from(MEMORY_ROWS);
        let visible = i32::from(VISIBLE_ROWS);
        self.panel
            .draw_iter(pixels.into_iter().filter_map(move |Pixel(point, color)| {
                if point.y < 0 || point.y >= visible {
                    return None;
                }
                Some(Pixel(
                    Point::new(point.x, (origin + point.y) % memory),
                    color,
                ))
            }))
    }

    /// Draws a run of colours, translated and split where memory ends.
    ///
    /// The colours arrive row-major across `area`, which may reach outside the
    /// panel; the piece that lands on it is what the panel underneath is given,
    /// and the rest has to be dropped here rather than there. The panel does
    /// clip, but it clips against its own memory - 320 rows of it - so a run
    /// hanging off the bottom of the screen would be drawn into the staging
    /// rows instead of discarded.
    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let visible = area.intersection(&self.bounding_box());
        if visible.size.width == 0 || visible.size.height == 0 {
            return Ok(());
        }
        let [first, second] = self.runs_of(&visible);

        // One piece of memory, nothing clipped away: the case that covers every
        // draw the watch makes until a slide moves the window, and the one that
        // must not pay for the other two.
        if visible == *area && second.height == 0 {
            return self
                .panel
                .fill_contiguous(&memory_area(&visible, &first), colors);
        }

        let mut colors = Clipped::new(colors.into_iter(), area, &visible);
        for run in [first, second] {
            if run.height == 0 {
                continue;
            }
            self.panel
                .fill_contiguous(&memory_area(&visible, &run), &mut colors)?;
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let visible = area.intersection(&self.bounding_box());
        if visible.size.width == 0 || visible.size.height == 0 {
            return Ok(());
        }
        for run in self.runs_of(&visible) {
            if run.height == 0 {
                continue;
            }
            self.panel.fill_solid(&memory_area(&visible, &run), color)?;
        }
        Ok(())
    }
}

/// The same rectangle, moved to the memory rows it was resolved to.
fn memory_area(area: &Rectangle, run: &MemoryRun) -> Rectangle {
    Rectangle::new(
        Point::new(area.top_left.x, i32::from(run.row)),
        Size::new(area.size.width, u32::from(run.height)),
    )
}

/// The colours of a clipped rectangle, from the colours of the whole one.
///
/// A caller producing a run wider than the panel - a line of text reaching past
/// an edge - hands over colours for every pixel it meant to draw. Only the ones
/// inside are wanted, and they have to be picked out row by row: taking the
/// intersection's width and marching on would pull the next row's colours into
/// this one, which shears the picture rather than cropping it.
struct Clipped<I> {
    colors: I,
    lead: u32,
    take: u32,
    skip: u32,
    taken: u32,
}

impl<I> Clipped<I>
where
    I: Iterator<Item = Rgb565>,
{
    const fn new(colors: I, area: &Rectangle, visible: &Rectangle) -> Self {
        let rows_above = visible.top_left.y.abs_diff(area.top_left.y);
        let before = visible.top_left.x.abs_diff(area.top_left.x);
        let take = visible.size.width;
        Self {
            colors,
            lead: rows_above * area.size.width + before,
            take,
            skip: area.size.width - take,
            taken: 0,
        }
    }

    /// Drops `count` colours, reporting whether any survived.
    fn drop_ahead(&mut self, count: u32) -> Option<()> {
        if count == 0 {
            return Some(());
        }
        let count = usize::try_from(count).ok()?;
        self.colors.nth(count - 1).map(drop)
    }
}

impl<I> Iterator for Clipped<I>
where
    I: Iterator<Item = Rgb565>,
{
    type Item = Rgb565;

    fn next(&mut self) -> Option<Self::Item> {
        if self.take == 0 {
            return None;
        }
        if self.lead > 0 {
            let lead = core::mem::take(&mut self.lead);
            self.drop_ahead(lead)?;
        }
        if self.taken == self.take {
            self.drop_ahead(self.skip)?;
            self.taken = 0;
        }
        self.taken += 1;
        self.colors.next()
    }
}
