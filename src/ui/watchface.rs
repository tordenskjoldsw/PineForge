use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, MonoTextStyleBuilder, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;

use crate::ui::{render::draw_mono_text_visible, screen::Screen};
#[cfg(feature = "diagnostics")]
use pineforge_state::{
    AccelerationSample, AccelerometerKind, FeatureEngineStatus, HeartRateRawSample,
    HeartRateSensorKind, PpgAnalysis,
};
use pineforge_state::{AppEvent, BatteryStatus, BleState, ScreenAction, ScreenId, SwipeDirection};

const ROW_HEIGHT: u32 = 25;
const VALUE_X: i32 = 70;
const UPTIME_ROW: Rectangle = Rectangle::new(Point::new(0, 50), Size::new(240, ROW_HEIGHT));
const BATTERY_ROW: Rectangle = Rectangle::new(Point::new(0, 75), Size::new(240, ROW_HEIGHT));
#[cfg(feature = "diagnostics")]
const MOTION_ROW: Rectangle = Rectangle::new(Point::new(0, 100), Size::new(240, ROW_HEIGHT));
#[cfg(feature = "diagnostics")]
const STEP_ROW: Rectangle = Rectangle::new(Point::new(0, 125), Size::new(240, ROW_HEIGHT));
#[cfg(not(feature = "diagnostics"))]
const STEP_ROW: Rectangle = Rectangle::new(Point::new(0, 100), Size::new(240, ROW_HEIGHT));
const RESERVED_ROW: Rectangle = Rectangle::new(Point::new(0, 150), Size::new(240, ROW_HEIGHT));
const STATUS_ROW: Rectangle = Rectangle::new(Point::new(0, 175), Size::new(240, ROW_HEIGHT));
const HEADER_AREA: Rectangle = Rectangle::new(Point::new(0, 0), Size::new(240, 25));
const FOOTER_AREA: Rectangle = Rectangle::new(Point::new(0, 200), Size::new(240, 40));

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
    Battery,
    #[cfg(feature = "diagnostics")]
    Motion,
    #[cfg(feature = "diagnostics")]
    HeartRate,
    Steps,
}

pub struct TerminalWatchface {
    previous_uptime_seconds: u64,
    uptime_seconds: u64,
    previous_ble: BleState,
    ble: BleState,
    battery: Option<BatteryStatus>,
    #[cfg(feature = "diagnostics")]
    accelerometer: Option<AccelerometerKind>,
    #[cfg(feature = "diagnostics")]
    acceleration: Option<AccelerationSample>,
    #[cfg(feature = "diagnostics")]
    feature_engine: Option<FeatureEngineStatus>,
    #[cfg(feature = "diagnostics")]
    heart_rate_sensor: Option<HeartRateSensorKind>,
    #[cfg(feature = "diagnostics")]
    heart_rate_raw: Option<HeartRateRawSample>,
    #[cfg(feature = "diagnostics")]
    heart_rate_analysis: Option<PpgAnalysis>,
    steps: Option<u32>,
    dirty: DirtyRegion,
}

impl Default for TerminalWatchface {
    fn default() -> Self {
        Self {
            previous_uptime_seconds: 0,
            uptime_seconds: 0,
            previous_ble: BleState::Off,
            ble: BleState::Off,
            battery: None,
            #[cfg(feature = "diagnostics")]
            accelerometer: None,
            #[cfg(feature = "diagnostics")]
            acceleration: None,
            #[cfg(feature = "diagnostics")]
            feature_engine: None,
            #[cfg(feature = "diagnostics")]
            heart_rate_sensor: None,
            #[cfg(feature = "diagnostics")]
            heart_rate_raw: None,
            #[cfg(feature = "diagnostics")]
            heart_rate_analysis: None,
            steps: None,
            dirty: DirtyRegion::None,
        }
    }
}

impl TerminalWatchface {
    fn format_uptime(seconds: u64) -> String<16> {
        let hours = seconds / 3_600;
        let minutes = (seconds / 60) % 60;
        let seconds = seconds % 60;
        let mut uptime = String::new();
        let _ = write!(uptime, "{hours:02}:{minutes:02}:{seconds:02}");
        uptime
    }

    fn draw_changed_value<D>(
        display: &mut D,
        old: &str,
        new: &str,
        baseline: i32,
        color: Rgb565,
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let first_changed = old
            .bytes()
            .zip(new.bytes())
            .position(|(old, new)| old != new)
            .unwrap_or_else(|| old.len().min(new.len()));
        if first_changed == new.len() && old.len() == new.len() {
            return Ok(());
        }

        let style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(color)
            .background_color(Rgb565::BLACK)
            .build();
        let x = VALUE_X + i32::try_from(first_changed).unwrap_or(0) * 10;
        draw_mono_text_visible(
            &new[first_changed..],
            Point::new(x, baseline),
            style,
            display,
        )?;
        Ok(())
    }

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
        draw_mono_text_visible(
            label,
            Point::new(0, baseline),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            display,
        )?;
        draw_mono_text_visible(
            value,
            Point::new(VALUE_X, baseline),
            MonoTextStyle::new(&FONT_10X20, value_color),
            display,
        )?;
        Ok(())
    }

    fn draw_uptime<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let uptime = Self::format_uptime(self.uptime_seconds);
        Self::draw_row(display, UPTIME_ROW, "[UPTM]", &uptime, TERMINAL_GREEN)
    }

    fn format_battery(&self) -> String<16> {
        let mut value = String::new();
        if let Some(status) = self.battery {
            let power = if status.charging {
                "CHG"
            } else if status.power_present {
                "PWR"
            } else {
                "BAT"
            };
            #[cfg(feature = "diagnostics")]
            let _ = write!(value, "{}mV {}% {power}", status.millivolts, status.percent);
            #[cfg(not(feature = "diagnostics"))]
            let _ = write!(value, "{}% {power}", status.percent);
        } else {
            let _ = value.push_str("---");
        }
        value
    }

    fn draw_battery<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Self::draw_row(
            display,
            BATTERY_ROW,
            "[BATT]",
            &self.format_battery(),
            TERMINAL_RED,
        )
    }

    #[cfg(feature = "diagnostics")]
    fn draw_accelerometer<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut value: String<20> = String::new();
        if self.feature_engine == Some(FeatureEngineStatus::Failed) {
            let _ = value.push_str("FEATURE ERROR");
        } else if let Some(sample) = self.acceleration {
            let _ = write!(value, "{:+} {:+} {:+}", sample.x, sample.y, sample.z);
        } else {
            match self.accelerometer {
                Some(AccelerometerKind::Bma421) => {
                    let _ = value.push_str("BMA421");
                }
                Some(AccelerometerKind::Bma425) => {
                    let _ = value.push_str("BMA425");
                }
                Some(AccelerometerKind::Unknown(chip_id)) => {
                    let _ = write!(value, "ID 0x{chip_id:02X}");
                }
                Some(AccelerometerKind::Unavailable) => {
                    let _ = value.push_str("ERROR");
                }
                None => {
                    let _ = value.push_str("---");
                }
            }
        }
        let label = if self.feature_engine == Some(FeatureEngineStatus::Ready) {
            "[IMU+]"
        } else {
            "[IMU ]"
        };
        Self::draw_row(display, MOTION_ROW, label, &value, TERMINAL_ORANGE)
    }

    /// Fixed-width so partial redraws blank the previous, longer value.
    const fn format_ble(state: BleState) -> &'static str {
        match state {
            BleState::Off => "off        ",
            BleState::Advertising => "advertising",
            BleState::Connected => "connected  ",
        }
    }

    fn draw_status<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Self::draw_row(
            display,
            STATUS_ROW,
            "[BLE ]",
            Self::format_ble(self.ble),
            TERMINAL_BLUE,
        )
    }

    fn draw_steps<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut value: String<16> = String::new();
        if let Some(steps) = self.steps {
            let _ = write!(value, "{steps}");
        } else {
            let _ = value.push_str("---");
        }
        Self::draw_row(display, STEP_ROW, "[STEP]", &value, TERMINAL_ORANGE)
    }

    #[cfg(feature = "diagnostics")]
    fn format_heart_rate(&self) -> String<12> {
        let mut value = String::new();
        match self.heart_rate_analysis {
            Some(PpgAnalysis::HeartRate { bpm }) => {
                let _ = write!(value, "{bpm} BPM");
                return value;
            }
            Some(PpgAnalysis::AmbientLight) => {
                let _ = value.push_str("AMBIENT");
                return value;
            }
            Some(PpgAnalysis::NoSignal) => {
                let _ = value.push_str("NO SIGNAL");
                return value;
            }
            Some(PpgAnalysis::Collecting { .. }) | None => {}
        }
        if let Some(sample) = self.heart_rate_raw {
            let _ = write!(value, "{} A{}", sample.hrs, sample.als);
            return value;
        }
        match self.heart_rate_sensor {
            Some(HeartRateSensorKind::Hrs3300) => {
                let _ = value.push_str("HRS3300");
            }
            Some(HeartRateSensorKind::Unknown(id)) => {
                let _ = write!(value, "HRS 0x{id:02X}");
            }
            Some(HeartRateSensorKind::Unavailable) => {
                let _ = value.push_str("HRS ERROR");
            }
            None => {
                let _ = value.push_str("HRS ---");
            }
        }
        value
    }

    #[cfg(feature = "diagnostics")]
    fn draw_heart_rate<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Self::draw_row(
            display,
            RESERVED_ROW,
            "[HRS ]",
            &self.format_heart_rate(),
            TERMINAL_RED,
        )
    }

    #[cfg(feature = "diagnostics")]
    fn draw_diagnostics_footer<D>(display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        FOOTER_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        draw_mono_text_visible(
            "swipe >",
            Point::new(0, 226),
            MonoTextStyle::new(&FONT_10X20, TERMINAL_GREEN),
            display,
        )
    }

    fn update_clock<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let old_uptime = Self::format_uptime(self.previous_uptime_seconds);
        let new_uptime = Self::format_uptime(self.uptime_seconds);
        Self::draw_changed_value(display, &old_uptime, &new_uptime, 70, TERMINAL_GREEN)
    }

    fn update_status<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Self::draw_changed_value(
            display,
            Self::format_ble(self.previous_ble),
            Self::format_ble(self.ble),
            195,
            TERMINAL_BLUE,
        )
    }
}

impl Screen for TerminalWatchface {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        match event {
            AppEvent::Tick { uptime_seconds } => {
                self.previous_uptime_seconds = self.uptime_seconds;
                self.uptime_seconds = uptime_seconds;
                self.dirty = DirtyRegion::Clock;
            }
            // Touch state is visualized on the diagnostics touch-test screen.
            AppEvent::Touch { .. } => {}
            AppEvent::BatteryUpdated(status) => {
                self.battery = Some(status);
                self.dirty = DirtyRegion::Battery;
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::AccelerometerDetected(kind) => {
                self.accelerometer = Some(kind);
                self.dirty = DirtyRegion::Motion;
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::AccelerationUpdated(sample) => {
                self.acceleration = Some(sample);
                self.dirty = DirtyRegion::Motion;
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::FeatureEngineUpdated(status) => {
                self.feature_engine = Some(status);
                self.dirty = DirtyRegion::Motion;
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateSensorDetected(kind) => {
                self.heart_rate_sensor = Some(kind);
                self.dirty = DirtyRegion::HeartRate;
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateRawSampleUpdated(raw) => {
                self.heart_rate_raw = Some(raw);
                self.dirty = DirtyRegion::HeartRate;
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateAnalysisUpdated(analysis) => {
                self.heart_rate_analysis = Some(analysis);
                self.dirty = DirtyRegion::HeartRate;
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateStateUpdated(_) => {}
            AppEvent::StepsUpdated(steps) => {
                self.steps = Some(steps);
                self.dirty = DirtyRegion::Steps;
            }
            AppEvent::DisplaySettingsUpdated(_) => {}
            AppEvent::BleUpdated(state) => {
                self.previous_ble = self.ble;
                self.ble = state;
                self.dirty = DirtyRegion::Status;
            }
            AppEvent::Swipe(direction) => {
                self.dirty = DirtyRegion::None;
                if direction == SwipeDirection::Down {
                    return ScreenAction::Push(ScreenId::DisplaySettings);
                }
                #[cfg(feature = "diagnostics")]
                if direction == SwipeDirection::Left {
                    return ScreenAction::Push(ScreenId::TouchTest);
                }
                #[cfg(feature = "diagnostics")]
                if direction == SwipeDirection::Up {
                    return ScreenAction::Push(ScreenId::HeartRate);
                }
            }
        }
        ScreenAction::None
    }

    fn draw_full<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let prompt = MonoTextStyle::new(&FONT_10X20, LIGHT_GRAY);
        HEADER_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        draw_mono_text_visible("user@watch:~ $ now", Point::new(0, 20), prompt, display)?;
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
        self.draw_battery(display)?;
        keep_alive();
        #[cfg(feature = "diagnostics")]
        self.draw_accelerometer(display)?;
        #[cfg(not(feature = "diagnostics"))]
        self.draw_steps(display)?;
        keep_alive();
        #[cfg(not(feature = "diagnostics"))]
        Self::draw_row(
            display,
            Rectangle::new(Point::new(0, 125), Size::new(240, ROW_HEIGHT)),
            "[L_HR]",
            "---",
            Rgb565::new(20, 20, 20),
        )?;
        #[cfg(feature = "diagnostics")]
        self.draw_steps(display)?;
        keep_alive();
        #[cfg(feature = "diagnostics")]
        self.draw_heart_rate(display)?;
        #[cfg(not(feature = "diagnostics"))]
        RESERVED_ROW
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        keep_alive();
        self.draw_status(display)?;
        keep_alive();

        FOOTER_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        #[cfg(feature = "diagnostics")]
        Self::draw_diagnostics_footer(display)?;
        #[cfg(not(feature = "diagnostics"))]
        draw_mono_text_visible("user@watch:~ $", Point::new(0, 226), prompt, display)?;
        keep_alive();
        Ok(())
    }

    fn draw_dirty<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        match self.dirty {
            DirtyRegion::Clock => {
                self.update_clock(display)?;
                keep_alive();
                keep_alive();
            }
            DirtyRegion::Status => self.update_status(display)?,
            DirtyRegion::Battery => self.draw_battery(display)?,
            #[cfg(feature = "diagnostics")]
            DirtyRegion::Motion => self.draw_accelerometer(display)?,
            #[cfg(feature = "diagnostics")]
            DirtyRegion::HeartRate => self.draw_heart_rate(display)?,
            DirtyRegion::Steps => self.draw_steps(display)?,
            DirtyRegion::None => {}
        }
        keep_alive();
        Ok(())
    }
}
