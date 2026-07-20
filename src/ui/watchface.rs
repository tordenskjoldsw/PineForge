use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use heapless::String;

use crate::{
    services::events::UiEvent,
    ui::screen::{Screen, ScreenAction},
};

const ROW_HEIGHT: u32 = 25;
const VALUE_X: i32 = 70;
const UPTIME_ROW: Rectangle = Rectangle::new(Point::new(0, 50), Size::new(240, ROW_HEIGHT));
const SAFETY_ROW: Rectangle = Rectangle::new(Point::new(0, 150), Size::new(240, ROW_HEIGHT));
const STATUS_ROW: Rectangle = Rectangle::new(Point::new(0, 175), Size::new(240, ROW_HEIGHT));
const SAFE_TIMEOUT_SECONDS: u64 = 60;

const LIGHT_GRAY: Rgb565 = Rgb565::new(20, 40, 20);
const TERMINAL_GREEN: Rgb565 = Rgb565::new(4, 51, 10);
const TERMINAL_BLUE: Rgb565 = Rgb565::new(0, 31, 31);
const TERMINAL_ORANGE: Rgb565 = Rgb565::new(31, 32, 0);
const TERMINAL_RED: Rgb565 = Rgb565::new(31, 12, 0);

#[derive(Clone, Copy, Default)]
enum DirtyRegion {
    #[default]
    None,
    Clock,
    Status,
}

#[derive(Default)]
pub struct TerminalWatchface {
    uptime_seconds: u64,
    touching: bool,
    dirty: DirtyRegion,
}

impl TerminalWatchface {
    fn draw_row<D>(
        display: &mut D,
        area: Rectangle,
        label: &str,
        value: &str,
        value_color: Rgb565,
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        area.into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        let baseline = area.top_left.y + 20;
        Text::new(
            label,
            Point::new(0, baseline),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
        )
        .draw(display)?;
        Text::new(
            value,
            Point::new(VALUE_X, baseline),
            MonoTextStyle::new(&FONT_10X20, value_color),
        )
        .draw(display)?;
        Ok(())
    }

    fn draw_uptime<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let hours = self.uptime_seconds / 3_600;
        let minutes = (self.uptime_seconds / 60) % 60;
        let seconds = self.uptime_seconds % 60;
        let mut uptime: String<16> = String::new();
        let _ = write!(uptime, "{hours:02}:{minutes:02}:{seconds:02}");
        Self::draw_row(display, UPTIME_ROW, "[UPTM]", &uptime, TERMINAL_GREEN)
    }

    fn draw_safety<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let remaining = SAFE_TIMEOUT_SECONDS.saturating_sub(self.uptime_seconds);
        let mut safety: String<16> = String::new();
        let _ = write!(safety, "{:02}:{:02}", remaining / 60, remaining % 60);
        Self::draw_row(display, SAFETY_ROW, "[SAFE]", &safety, TERMINAL_ORANGE)
    }

    fn draw_status<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let status = if self.touching {
            "Touch active"
        } else {
            "Touch ready"
        };
        Self::draw_row(display, STATUS_ROW, "[STAT]", status, TERMINAL_BLUE)
    }
}

impl Screen for TerminalWatchface {
    fn handle_event(&mut self, event: UiEvent) -> ScreenAction {
        match event {
            UiEvent::Tick { uptime_seconds } => {
                self.uptime_seconds = uptime_seconds;
                self.dirty = DirtyRegion::Clock;
            }
            UiEvent::Touch { pressed, .. } => {
                self.touching = pressed;
                self.dirty = DirtyRegion::Status;
            }
        }
        ScreenAction::None
    }

    fn draw<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        display.clear(Rgb565::BLACK)?;
        keep_alive();

        let prompt = MonoTextStyle::new(&FONT_10X20, LIGHT_GRAY);
        Text::new("user@watch:~ $ now", Point::new(0, 20), prompt).draw(display)?;
        keep_alive();

        Self::draw_row(
            display,
            Rectangle::new(Point::new(0, 25), Size::new(240, ROW_HEIGHT)),
            "[TIME]",
            "--:--:--",
            TERMINAL_GREEN,
        )?;
        keep_alive();
        self.draw_uptime(display)?;
        keep_alive();
        Self::draw_row(
            display,
            Rectangle::new(Point::new(0, 75), Size::new(240, ROW_HEIGHT)),
            "[BATT]",
            "---",
            TERMINAL_RED,
        )?;
        keep_alive();
        Self::draw_row(
            display,
            Rectangle::new(Point::new(0, 100), Size::new(240, ROW_HEIGHT)),
            "[STEP]",
            "---",
            TERMINAL_ORANGE,
        )?;
        keep_alive();
        Self::draw_row(
            display,
            Rectangle::new(Point::new(0, 125), Size::new(240, ROW_HEIGHT)),
            "[L_HR]",
            "---",
            Rgb565::new(20, 20, 20),
        )?;
        keep_alive();
        self.draw_safety(display)?;
        keep_alive();
        self.draw_status(display)?;
        keep_alive();

        Text::new("user@watch:~ $", Point::new(0, 220), prompt).draw(display)?;
        keep_alive();
        Ok(())
    }

    fn draw_update<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        match self.dirty {
            DirtyRegion::Clock => {
                self.draw_uptime(display)?;
                keep_alive();
                self.draw_safety(display)?;
            }
            DirtyRegion::Status => self.draw_status(display)?,
            DirtyRegion::None => {}
        }
        keep_alive();
        Ok(())
    }
}
