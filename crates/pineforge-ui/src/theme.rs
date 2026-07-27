//! The one place that names a colour.
//!
//! Screens ask for a role - text, frame, accent, danger - and never for a
//! value, so restyling the firmware is an edit to this file rather than a
//! sweep through every screen. See `docs/UI-DESIGN.md` for what each role
//! means and why the palette looks like this.
//!
//! Values are written as RGB565 components because that is what the panel
//! stores: five bits of red, six of green, five of blue. Picking colours as
//! web hex and converting shifts them visibly, and green shifts differently
//! from red and blue.

use embedded_graphics::pixelcolor::Rgb565;
use pineforge_state::{BleState, ChargeLevel};

/// Every screen's ground. Also the label colour on a filled component, which
/// is why it is named rather than written as black at the call site.
pub const BACKGROUND: Rgb565 = Rgb565::new(0, 0, 0);
pub const TEXT: Rgb565 = Rgb565::new(31, 63, 31);
/// Borders of rows and tiles.
pub const FRAME: Rgb565 = Rgb565::new(8, 16, 8);
/// The face of a raised component - a launcher tile - against the background.
///
/// Dark enough to keep the display mostly black, which is what the panel is
/// good at, but far enough above it that a tile reads as a surface rather than
/// as an outline.
pub const SURFACE: Rgb565 = Rgb565::new(4, 9, 5);
/// Values, icons, and the fill of a pressed or selected component.
///
/// Indigo rather than the amber this started with: it is the one hue no status
/// colour uses, so nothing interactive can be mistaken for a warning, and both
/// its channels are bright enough to survive the lowest backlight level.
pub const ACCENT: Rgb565 = Rgb565::new(15, 23, 31);
/// Rollback, DFU failure, critical charge. Reserved for state that deserves
/// attention; never decoration.
pub const DANGER: Rgb565 = Rgb565::new(31, 0, 0);

/// Bluetooth connected.
///
/// Azure rather than pure blue: pure blue is the first colour to disappear when
/// the backlight dims to its lowest level, and a status symbol that vanishes
/// before the watch sleeps is worse than none.
pub const LINK: Rgb565 = Rgb565::new(8, 40, 31);
pub const OK: Rgb565 = Rgb565::new(0, 63, 0);
pub const WARN: Rgb565 = Rgb565::new(31, 63, 0);

// Muted text for disabled entries lands with the component states that need
// it; a constant nothing uses would only be deleted again.

/// Colour of the battery symbol. The thresholds behind the level are product
/// policy and live in the state crate, host-tested.
#[must_use]
pub const fn battery(level: ChargeLevel) -> Rgb565 {
    match level {
        ChargeLevel::Good => OK,
        ChargeLevel::Low => WARN,
        ChargeLevel::Critical => DANGER,
    }
}

/// Colour of the Bluetooth symbol: a phone is either reachable or it is not,
/// and advertising into an empty room counts as not.
#[must_use]
pub const fn bluetooth(state: BleState) -> Rgb565 {
    match state {
        BleState::Connected | BleState::Pairing(_) | BleState::DfuProgress(_) => LINK,
        BleState::Off | BleState::Advertising | BleState::DfuFailed(_) => DANGER,
    }
}
