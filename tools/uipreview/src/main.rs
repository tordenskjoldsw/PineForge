//! Renders every screen to a PNG, so the UI can be looked at without a watch.
//!
//! The screens already draw into a `Canvas`, which borrows any embedded-graphics
//! target over `Rgb565`. On the watch that target is the panel; here it is a
//! buffer this writes out. Nothing in `pineforge-ui` knows the difference, so
//! what comes out is the same drawing code the firmware runs - not a mock of it.
//!
//! What it is not: a colour-accurate preview. A PNG is sRGB on whatever monitor
//! shows it, while the panel is a backlit TFT whose lowest brightness level is
//! where half the palette decisions were actually made. Use this for layout,
//! spacing, and relative comparisons; use the watch for whether a colour works.

use std::{fs::File, io::BufWriter, path::Path};

use embedded_graphics::{
    Pixel,
    geometry::Size,
    pixelcolor::{Rgb565, RgbColor},
    prelude::*,
};
use pineforge_state::{
    AccelerometerKind, AppEvent, BatteryStatus, BleState, CalendarDate, DfuFailReason,
    DisplaySettings, FirmwareImageState, FlashStatus, HeartRateSensorKind, HeartRateState,
    MusicState, PeripheralStatus, ScreenId, StackUsage, StopwatchControl, SwipeDirection,
    TimerControl, WallTime, parse_new_alert,
};
use pineforge_ui::{
    about::BuildInfo,
    canvas::{Canvas, CanvasError},
    dfu::{draw_dfu_failed, draw_dfu_progress, draw_storage_progress},
    pairing::draw_pairing,
    registry::Screens,
    status::StatusCorner,
    timer::draw_expired_modal,
};

const SIDE: u32 = 240;
/// Nearest-neighbour magnification. The panel's pixels are the unit the design
/// is in, so they are enlarged rather than smoothed - a blurred preview would
/// hide exactly the single-pixel decisions worth reviewing.
const SCALE: u32 = 3;

/// A `DrawTarget` that keeps what was drawn.
struct Framebuffer {
    pixels: Vec<Rgb565>,
}

impl Framebuffer {
    fn new() -> Self {
        Self {
            pixels: vec![Rgb565::BLACK; (SIDE * SIDE) as usize],
        }
    }

    /// Expands 5-6-5 to 8 bits per channel the way the panel does, by
    /// replicating the high bits into the low ones. Scaling by 255/31 instead
    /// would put white at 255 but leave every midtone slightly off.
    fn rgb888(color: Rgb565) -> [u8; 3] {
        let (r, g, b) = (color.r(), color.g(), color.b());
        [
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        ]
    }

    fn write_png(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let side = SIDE * SCALE;
        let mut data = Vec::with_capacity((side * side * 3) as usize);
        for y in 0..side {
            for x in 0..side {
                let source = (y / SCALE) * SIDE + (x / SCALE);
                data.extend_from_slice(&Self::rgb888(self.pixels[source as usize]));
            }
        }

        let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), side, side);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&data)?;
        Ok(())
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(SIDE, SIDE)
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            // Clipping rather than panicking: a screen is allowed to draw past
            // the edge, and the panel silently ignores it too.
            if (0..SIDE as i32).contains(&point.x) && (0..SIDE as i32).contains(&point.y) {
                self.pixels[(point.y as u32 * SIDE + point.x as u32) as usize] = color;
            }
        }
        Ok(())
    }
}

/// A watch that has been worn for a while: time synchronised, a step count
/// worth four digits, a heart rate, and a phone connected.
///
/// Placeholder-looking state is worse than useless for reviewing a layout - a
/// zeroed step count never shows whether the column is wide enough.
fn populate(screens: &mut Screens, status: &mut StatusCorner) {
    let battery = BatteryStatus {
        millivolts: 3_940,
        percent: 72,
        charging: false,
        power_present: false,
    };
    let events = [
        AppEvent::Tick {
            uptime_seconds: 51_129,
            wall_time: Some(WallTime {
                hour: 14,
                minute: 32,
                second: 9,
            }),
            date: Some(CalendarDate {
                year: 2026,
                month: 7,
                day: 28,
            }),
        },
        AppEvent::BatteryUpdated(battery),
        AppEvent::StepsUpdated(8_214),
        AppEvent::HeartRateStateUpdated(HeartRateState::Result(63)),
        AppEvent::BleUpdated(BleState::Connected),
        AppEvent::TouchControllerUpdated(PeripheralStatus::Ready),
        AppEvent::AccelerometerDetected(AccelerometerKind::Bma421),
        AppEvent::HeartRateSensorDetected(HeartRateSensorKind::Hrs3300),
        AppEvent::FlashUpdated(FlashStatus::Ready([0x0b, 0x40, 0x16])),
        AppEvent::FirmwareImageUpdated(FirmwareImageState::Confirmed),
        AppEvent::StackUpdated(StackUsage {
            used: 10_432,
            capacity: 16_384,
        }),
    ];
    for event in events {
        // Routed the way the display task routes it: a reading is a fact about
        // the watch and reaches every screen that keeps one, while a tick is
        // addressed to whichever screen is up. Sending everything to the
        // watchface instead left the pulse and steps apps rendering their
        // empty state, which is not what these sheets are for.
        if event.is_reading() {
            let _ = screens.absorb(ScreenId::Watchface, event);
        } else {
            let _ = screens.handle(ScreenId::Watchface, event);
        }
    }

    status.set_battery(battery);
    status.set_ble(BleState::Connected);
    // Shown rather than hidden: this mark is the whole reason a firmware update
    // can be refused, and it is worth seeing what it looks like beside the
    // other two rather than only in the rare state that produces it.
    status.set_unconfirmed(true);

    // The firmware supplies this on the watch; here it is stated, so the About
    // screen and the firmware screen show a plausible width rather than the
    // placeholder that would hide a column running off the panel.
    let build = BuildInfo {
        version: "0.2.1+21",
        commit: "b56b4ab",
        date: "2026-07-28",
        bootloader: "MCUBOOT",
    };
    screens.about.set_build(build);
    screens.firmware.set_version(build.version);

    // A track part-way through, so the music screen shows its bar somewhere
    // other than at either end and its title at a width worth judging. Applied
    // directly, the way the display task applies it: the record travels on its
    // own watch rather than through the event channel.
    let mut music = MusicState::new();
    let _ = music.set_track(b"All My Friends");
    let _ = music.set_artist(b"LCD Soundsystem");
    let _ = music.set_playing(&[1]);
    let _ = music.set_position(&134_u32.to_be_bytes());
    let _ = music.set_length(&463_u32.to_be_bytes());
    let _ = screens.music.apply(&music, 0);

    // A running value with carries in every field exercises the whole FORGE
    // stopwatch layout and its active colour.
    screens.stopwatch.control(StopwatchControl::Start, 1_000);
    let _ = screens.handle(ScreenId::Stopwatch, AppEvent::StopwatchTick(754_490));

    let _ = screens.timer.control(TimerControl::Start, 1_000);
    let _ = screens.handle(ScreenId::Timer, AppEvent::TimerTick(67_000));

    screens.time.open(WallTime {
        hour: 14,
        minute: 32,
        second: 0,
    });
    screens.date.open(CalendarDate {
        year: 2026,
        month: 7,
        day: 28,
    });

    // Two messages, so the notification screen shows its paging rather than its
    // empty state.
    for payload in [
        b"\x01\x01\x00Signal\x00Are we still on for tomorrow?".as_slice(),
        b"\x03\x01\x00Calendar\x00Standup at 09:30".as_slice(),
    ] {
        if let Some(notification) = parse_new_alert(payload) {
            let _ = screens.notifications.file(notification);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("out");
    std::fs::create_dir_all(&out)?;

    let mut screens = Screens::new();
    let mut status = StatusCorner::new();
    populate(&mut screens, &mut status);

    for screen in ScreenId::ALL {
        // Every screen is reached through `enter` on the watch, and the
        // settings leaves are only correct once it has pointed them somewhere.
        screens.enter(screen, DisplaySettings::DEFAULT);

        let mut framebuffer = Framebuffer::new();
        // `CanvasError` is deliberately opaque and carries no cause - nothing
        // above it in the firmware can act on one. It is also unreachable here,
        // the buffer being infallible, so it is named rather than converted.
        screens
            .draw_full(
                screen,
                &status,
                &mut Canvas::new(&mut framebuffer),
                &mut || {},
            )
            .map_err(|_| format!("drawing {screen:?} failed"))?;

        let name = format!("{screen:?}").to_lowercase();
        let path = out.join(format!("{name}.png"));
        framebuffer.write_png(&path)?;
        println!("{}", path.display());
    }

    // About owns three pages; render the two that its registry entry cannot
    // reach without gestures as separate review artifacts.
    for name in ["about-hardware", "about-system"] {
        let _ = screens.handle(ScreenId::About, AppEvent::Swipe(SwipeDirection::Up));
        let mut framebuffer = Framebuffer::new();
        screens
            .draw_full(
                ScreenId::About,
                &status,
                &mut Canvas::new(&mut framebuffer),
                &mut || {},
            )
            .map_err(|_| format!("drawing {name} failed"))?;
        let path = out.join(format!("{name}.png"));
        framebuffer.write_png(&path)?;
        println!("{}", path.display());
    }

    // The system modals are not screens and are not in the registry, so they
    // are named here. They own the whole panel while they show, which is why
    // they are worth seeing at full size rather than inferred from a screen.
    /// One modal, drawn at a value worth looking at.
    type Modal<'a> = (
        &'a str,
        &'a dyn Fn(&mut Canvas<'_>) -> Result<(), CanvasError>,
    );

    let modals: [Modal<'_>; 7] = [
        ("modal-dfu-progress", &|c| {
            draw_dfu_progress(c, 42, &mut || {})
        }),
        ("modal-dfu-complete", &|c| {
            draw_dfu_progress(c, 100, &mut || {})
        }),
        ("modal-dfu-failed", &|c| {
            draw_dfu_failed(c, DfuFailReason::TimedOut, &mut || {})
        }),
        ("modal-dfu-unconfirmed", &|c| {
            draw_dfu_failed(c, DfuFailReason::NotConfirmed, &mut || {})
        }),
        ("modal-storage", &|c| {
            draw_storage_progress(c, 66, &mut || {})
        }),
        ("modal-pairing", &|c| draw_pairing(c, 123_456, &mut || {})),
        ("modal-timer", &|c| draw_expired_modal(c, &mut || {})),
    ];
    for (name, draw) in modals {
        let mut framebuffer = Framebuffer::new();
        draw(&mut Canvas::new(&mut framebuffer)).map_err(|_| format!("drawing {name} failed"))?;
        let path = out.join(format!("{name}.png"));
        framebuffer.write_png(&path)?;
        println!("{}", path.display());
    }

    Ok(())
}
