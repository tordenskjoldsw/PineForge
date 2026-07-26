//! The status corner: Bluetooth and charge, on every screen except a watchface.
//!
//! A watchface owns its whole surface and decides for itself what it shows -
//! there will be several, and they will differ. Everywhere else the two pieces
//! of state a user checks without thinking sit in the top-right corner, in a
//! strip 16 pixels tall.
//!
//! The symbols are drawn from geometry rather than from stored bitmaps: they
//! cost no flash, and a stripe can compose them without reading anything.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Line, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
};
use pineforge_state::{BatteryStatus, BleState};

use crate::ui::{
    canvas::{Canvas, CanvasError},
    render::draw_visible,
    screen::Paint,
    theme,
};

/// Height of the strip the corner lives in. Screen content starts below it.
pub const STATUS_HEIGHT: i32 = 16;

const SCREEN_WIDTH: i32 = 240;
const MARGIN: i32 = 4;

const BATTERY_WIDTH: i32 = 22;
const BATTERY_HEIGHT: i32 = 11;
const BATTERY_X: i32 = SCREEN_WIDTH - MARGIN - BATTERY_WIDTH - 2;
const BATTERY_Y: i32 = 2;
const TERMINAL_WIDTH: i32 = 2;
const TERMINAL_HEIGHT: i32 = 5;

const RUNE_WIDTH: i32 = 8;
const RUNE_HEIGHT: i32 = 14;
const RUNE_X: i32 = BATTERY_X - 10 - RUNE_WIDTH;
const RUNE_Y: i32 = 1;

/// What the corner shows. Both values arrive as events; absent means the
/// service has not reported yet, and nothing is drawn rather than a guess.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatusCorner {
    ble: Option<BleState>,
    battery: Option<BatteryStatus>,
}

impl StatusCorner {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ble: None,
            battery: None,
        }
    }

    /// Returns whether the corner needs redrawing.
    pub fn set_ble(&mut self, state: BleState) -> bool {
        let changed = self.ble != Some(state);
        self.ble = Some(state);
        changed
    }

    /// Returns whether the corner needs redrawing.
    ///
    /// The battery service publishes on a timer whether or not the reading
    /// moved, and the corner shows a coarse bar, so only a changed bar length
    /// or colour is worth the transaction.
    pub fn set_battery(&mut self, status: BatteryStatus) -> bool {
        let changed = self.battery.is_none_or(|previous| {
            previous.level() != status.level() || fill_width(previous) != fill_width(status)
        });
        self.battery = Some(status);
        changed
    }

    /// Draws the corner over its own background, so it can be refreshed without
    /// touching the screen underneath.
    pub fn draw(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        draw_visible(
            &Rectangle::new(
                Point::new(RUNE_X - 2, 0),
                Size::new((SCREEN_WIDTH - RUNE_X + 2) as u32, STATUS_HEIGHT as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND)),
            canvas,
        )?;

        if let Some(state) = self.ble {
            self.draw_rune(canvas, theme::bluetooth(state))?;
        }
        if let Some(status) = self.battery {
            self.draw_battery(canvas, status)?;
        }
        Ok(())
    }

    /// The Bluetooth rune: a stem crossed by two triangles, the shape everyone
    /// recognises without a legend.
    fn draw_rune(&self, canvas: &mut Canvas<'_>, color: Rgb565) -> Result<(), CanvasError> {
        let _ = self;
        let style = PrimitiveStyle::with_stroke(color, 1);
        let top = Point::new(RUNE_X + RUNE_WIDTH / 2, RUNE_Y);
        let bottom = Point::new(RUNE_X + RUNE_WIDTH / 2, RUNE_Y + RUNE_HEIGHT);
        let upper_right = Point::new(RUNE_X + RUNE_WIDTH, RUNE_Y + RUNE_HEIGHT / 4);
        let lower_right = Point::new(RUNE_X + RUNE_WIDTH, RUNE_Y + RUNE_HEIGHT * 3 / 4);
        let upper_left = Point::new(RUNE_X, RUNE_Y + RUNE_HEIGHT / 4);
        let lower_left = Point::new(RUNE_X, RUNE_Y + RUNE_HEIGHT * 3 / 4);

        for (from, to) in [
            (top, bottom),
            (top, upper_right),
            (upper_right, lower_left),
            (bottom, lower_right),
            (lower_right, upper_left),
        ] {
            draw_visible(&Line::new(from, to).into_styled(style), canvas)?;
        }
        Ok(())
    }

    /// A battery outline with a terminal, filled in proportion to charge and
    /// coloured by urgency.
    fn draw_battery(
        &self,
        canvas: &mut Canvas<'_>,
        status: BatteryStatus,
    ) -> Result<(), CanvasError> {
        let _ = self;
        let color = theme::battery(status.level());
        draw_visible(
            &Rectangle::new(
                Point::new(BATTERY_X, BATTERY_Y),
                Size::new(BATTERY_WIDTH as u32, BATTERY_HEIGHT as u32),
            )
            .into_styled(
                PrimitiveStyleBuilder::new()
                    .fill_color(theme::BACKGROUND)
                    .stroke_color(color)
                    .stroke_width(1)
                    .build(),
            ),
            canvas,
        )?;
        draw_visible(
            &Rectangle::new(
                Point::new(
                    BATTERY_X + BATTERY_WIDTH,
                    BATTERY_Y + (BATTERY_HEIGHT - TERMINAL_HEIGHT) / 2,
                ),
                Size::new(TERMINAL_WIDTH as u32, TERMINAL_HEIGHT as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(color)),
            canvas,
        )?;

        let fill = fill_width(status);
        if fill == 0 {
            return Ok(());
        }
        draw_visible(
            &Rectangle::new(
                Point::new(BATTERY_X + 2, BATTERY_Y + 2),
                Size::new(fill, u32::try_from(BATTERY_HEIGHT - 4).unwrap_or(0)),
            )
            .into_styled(PrimitiveStyle::with_fill(color)),
            canvas,
        )
    }
}

/// Width of the charge bar inside the outline, in pixels.
fn fill_width(status: BatteryStatus) -> u32 {
    let inner = u32::try_from(BATTERY_WIDTH - 4).unwrap_or(0);
    u32::from(status.percent.min(100)) * inner / 100
}

/// A screen painted together with the status corner above it.
///
/// Transitions compose whatever they are given once per stripe, so the corner
/// has to be part of that composition rather than something drawn afterwards.
/// Holds the screen as `&dyn Paint` rather than by type parameter, so the
/// composition below is compiled once instead of once per screen it wraps.
pub struct WithStatus<'a> {
    screen: &'a dyn Paint,
    status: &'a StatusCorner,
}

impl<'a> WithStatus<'a> {
    #[must_use]
    pub const fn new(screen: &'a dyn Paint, status: &'a StatusCorner) -> Self {
        Self { screen, status }
    }
}

impl Paint for WithStatus<'_> {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.screen.draw_full(canvas, keep_alive)?;
        keep_alive();
        self.status.draw(canvas)
    }
}

/// Whether this screen carries the corner. A watchface does not: it owns its
/// whole surface and shows what its own design calls for.
#[must_use]
pub const fn wears_status(screen: pineforge_state::ScreenId) -> bool {
    !matches!(screen, pineforge_state::ScreenId::Watchface)
}
