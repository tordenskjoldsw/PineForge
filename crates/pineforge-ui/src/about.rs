//! Build identity and the system facts needed to diagnose a sealed watch.

use core::fmt::Write;

use embedded_graphics::{draw_target::DrawTarget, geometry::Point};
use heapless::String;
use pineforge_state::{
    AccelerometerKind, AppEvent, BleState, DfuFailReason, FirmwareImageState, FlashStatus,
    HeartRateSensorKind, PageAxis, PagedList, PeripheralStatus, ScreenAction, SwipeDirection,
    SystemFault, SystemStatus,
};

use crate::canvas::{Canvas, CanvasError};
use crate::font::ui_text;
use crate::{
    render::{ROW_X, draw_mono_text_visible, draw_page_marks},
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

const PAGE_COUNT: usize = 3;
const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;
const FIRST_LINE_Y: i32 = TITLE_BASELINE_Y + 34;
const LINE_STEP: i32 = 31;
const VALUE_X: i32 = ROW_X + 90;
const HINT_BASELINE_Y: i32 = 232;
const PAGE_RAIL: Point = Point::new(229, 132);

/// What the firmware knows about itself at compile time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: &'static str,
    pub commit: &'static str,
    pub date: &'static str,
    pub bootloader: &'static str,
}

impl BuildInfo {
    pub const UNKNOWN: Self = Self {
        version: "unknown",
        commit: "unknown",
        date: "unknown",
        bootloader: "unknown",
    };
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

/// Three vertically paged views: build, hardware, and live system state.
pub struct AboutScreen {
    build: BuildInfo,
    status: SystemStatus,
    pages: PagedList,
    dirty: bool,
}

impl AboutScreen {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            build: BuildInfo::UNKNOWN,
            status: SystemStatus::new(),
            pages: PagedList::new(PAGE_COUNT, 1),
            dirty: false,
        }
    }

    pub const fn set_build(&mut self, build: BuildInfo) {
        self.build = build;
    }

    pub fn set_image(&mut self, confirmed: bool) {
        let _ = self
            .status
            .apply(AppEvent::FirmwareImageUpdated(if confirmed {
                FirmwareImageState::Confirmed
            } else {
                FirmwareImageState::Trial
            }));
    }

    #[must_use]
    pub const fn moved(&self) -> bool {
        self.dirty
    }

    pub const fn mark_painted(&mut self) {
        self.dirty = false;
    }

    fn draw_row(
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
        index: usize,
        name: &str,
        text: &str,
    ) -> Result<(), CanvasError> {
        let y = FIRST_LINE_Y + LINE_STEP * i32::try_from(index).unwrap_or(0);
        draw_mono_text_visible(
            name,
            Point::new(ROW_X, y),
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
        )?;
        draw_mono_text_visible(
            text,
            Point::new(VALUE_X, y),
            ui_text(theme::ACCENT, theme::BACKGROUND),
            canvas,
        )?;
        keep_alive();
        Ok(())
    }

    fn draw_about(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        for (index, (name, value)) in [
            ("BUILD", self.build.version),
            ("COMMIT", self.build.commit),
            ("DATE", self.build.date),
            ("BOOT", self.build.bootloader),
        ]
        .into_iter()
        .enumerate()
        {
            Self::draw_row(canvas, keep_alive, index, name, value)?;
        }
        Ok(())
    }

    fn draw_hardware(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let touch = match self.status.touch {
            None => "WAIT",
            Some(PeripheralStatus::Ready) => "OK",
            Some(PeripheralStatus::Unavailable) => "FAIL",
        };
        let motion = match self.status.motion {
            None => "WAIT",
            Some(AccelerometerKind::Bma421) => "BMA421",
            Some(AccelerometerKind::Bma425) => "BMA425",
            Some(AccelerometerKind::Unknown(_)) => "UNKNOWN",
            Some(AccelerometerKind::Unavailable) => "FAIL",
        };
        let pulse = match self.status.heart_rate {
            None => "WAIT",
            Some(HeartRateSensorKind::Hrs3300) => "HRS3300",
            Some(HeartRateSensorKind::Unknown(_)) => "UNKNOWN",
            Some(HeartRateSensorKind::Unavailable) => "FAIL",
        };
        let mut flash_text = String::<12>::new();
        match self.status.flash {
            None => flash_text.push_str("WAIT").ok(),
            Some(FlashStatus::Ready(id)) => {
                write!(flash_text, "{:02X}{:02X}{:02X}", id[0], id[1], id[2]).ok()
            }
            Some(FlashStatus::Unrecognized(id)) => {
                write!(flash_text, "?{:02X}{:02X}{:02X}", id[0], id[1], id[2]).ok()
            }
            Some(FlashStatus::Unavailable) => flash_text.push_str("FAIL").ok(),
        };

        for (index, (name, value)) in [
            ("TOUCH", touch),
            ("MOTION", motion),
            ("PULSE", pulse),
            ("FLASH", flash_text.as_str()),
        ]
        .into_iter()
        .enumerate()
        {
            Self::draw_row(canvas, keep_alive, index, name, value)?;
        }
        Ok(())
    }

    fn draw_system(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let image = match self.status.image {
            FirmwareImageState::Trial => "TRIAL",
            FirmwareImageState::Confirmed => "CONFIRMED",
        };
        let mut ble_text = String::<12>::new();
        match self.status.ble {
            BleState::Off => ble_text.push_str("OFF").ok(),
            BleState::Advertising => ble_text.push_str("ADVERT").ok(),
            BleState::Pairing(_) => ble_text.push_str("PAIRING").ok(),
            BleState::Connected => ble_text.push_str("CONNECTED").ok(),
            BleState::DfuProgress(percent) => write!(ble_text, "DFU {percent}%").ok(),
            BleState::DfuFailed(_) => ble_text.push_str("DFU FAIL").ok(),
        };
        let mut stack_text = String::<12>::new();
        if let Some(stack) = self.status.stack {
            write!(stack_text, "{}/{}", stack.used, stack.capacity).ok();
        } else {
            stack_text.push_str("WAIT").ok();
        }
        let fault = match self.status.last_fault {
            None => "NONE",
            Some(SystemFault::Touch) => "TOUCH",
            Some(SystemFault::Motion) => "MOTION",
            Some(SystemFault::HeartRate) => "PULSE",
            Some(SystemFault::Flash) => "FLASH",
            Some(SystemFault::Storage) => "STORAGE",
            Some(SystemFault::Dfu(reason)) => match reason {
                DfuFailReason::FlashUnrecognized(_) => "DFU FLASH",
                DfuFailReason::FlashInitFailed => "DFU INIT",
                DfuFailReason::EraseFailed => "DFU ERASE",
                DfuFailReason::ProgramFailed => "DFU WRITE",
                DfuFailReason::VerifyFailed => "DFU VERIFY",
                DfuFailReason::TimedOut => "DFU TIMEOUT",
                DfuFailReason::NotConfirmed => "DFU TRIAL",
            },
        };

        for (index, (name, value)) in [
            ("IMAGE", image),
            ("BLE", ble_text.as_str()),
            ("STACK", stack_text.as_str()),
            ("LAST", fault),
        ]
        .into_iter()
        .enumerate()
        {
            Self::draw_row(canvas, keep_alive, index, name, value)?;
        }
        Ok(())
    }
}

impl Default for AboutScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Paint for AboutScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        canvas.clear(theme::BACKGROUND)?;
        keep_alive();
        let title = match self.pages.page() {
            0 => "ABOUT",
            1 => "HARDWARE",
            _ => "SYSTEM",
        };
        draw_mono_text_visible(
            title,
            Point::new(ROW_X, TITLE_BASELINE_Y),
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
        )?;
        match self.pages.page() {
            0 => self.draw_about(canvas, keep_alive)?,
            1 => self.draw_hardware(canvas, keep_alive)?,
            _ => self.draw_system(canvas, keep_alive)?,
        }
        draw_page_marks(canvas, PageAxis::Vertical, &self.pages, PAGE_RAIL)?;
        draw_mono_text_visible(
            "< back",
            Point::new(ROW_X, HINT_BASELINE_Y),
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
        )
    }
}

impl Screen for AboutScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        let moved = match event {
            AppEvent::Swipe(SwipeDirection::Up) => self.pages.next_page(),
            AppEvent::Swipe(SwipeDirection::Down) => self.pages.previous_page(),
            _ => self.status.apply(event),
        };
        self.dirty |= moved;
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.dirty {
            self.draw_full(canvas, keep_alive)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_vertically_and_stops_at_both_ends() {
        let mut screen = AboutScreen::new();
        let _ = screen.handle_event(AppEvent::Swipe(SwipeDirection::Down));
        assert_eq!(screen.pages.page(), 0);
        let _ = screen.handle_event(AppEvent::Swipe(SwipeDirection::Up));
        let _ = screen.handle_event(AppEvent::Swipe(SwipeDirection::Up));
        let _ = screen.handle_event(AppEvent::Swipe(SwipeDirection::Up));
        assert_eq!(screen.pages.page(), 2);
    }

    #[test]
    fn keeps_status_that_arrives_while_elsewhere() {
        let mut screen = AboutScreen::new();
        let _ = screen.handle_event(AppEvent::FlashUpdated(FlashStatus::Ready([
            0x0b, 0x40, 0x16,
        ])));
        assert_eq!(
            screen.status.flash,
            Some(FlashStatus::Ready([0x0b, 0x40, 0x16]))
        );
        assert!(screen.moved());
    }
}
