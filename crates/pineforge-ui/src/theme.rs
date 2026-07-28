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
/// Page marks off the current page, and rules.
///
/// Held at 3:1 against the background, which is what WCAG 1.4.11 asks of a
/// non-text element that carries meaning. It used to be 2.07:1 and read as
/// absent: a page rail showed one bright mark and, as far as the eye was
/// concerned, nothing beside it - so a screen with two pages looked like a
/// screen with one and a stray decoration.
pub const FRAME: Rgb565 = Rgb565::new(11, 22, 11);
/// The face of a raised component - a launcher tile - against the background.
///
/// Dark enough to keep the display mostly black, which is what the panel is
/// good at, but far enough above it that a tile reads as a surface rather than
/// as an outline.
pub const SURFACE: Rgb565 = Rgb565::new(4, 9, 5);
/// Values, icons, and the fill of a pressed or selected component.
///
/// Verdigris: the colour forged copper ages into, and the colour a pine needle
/// is in shade. Both halves of the name land on it, which is the whole reason
/// it is this and not something else - an accent is the one colour a product
/// gets to be recognised by.
///
/// It replaces an indigo that was never chosen so much as left over. The palette
/// spends red, yellow, green and azure on status, and the accent had been pushed
/// into the last free sector to keep it from reading as a warning. That
/// constraint is real and this colour still answers it, but by separation in
/// saturation rather than in hue: `OK` is pure green with no blue at all, while
/// this is visibly blue-green, so the two do not trade places at a glance.
///
/// Green and blue are also the channels that survive dimming - green has six
/// bits where the others have five, and pure blue is the first to vanish - so
/// this stays legible at backlight level 1, where a darker or warmer accent
/// would not.
pub const ACCENT: Rgb565 = Rgb565::new(3, 49, 19);
/// Rollback, DFU failure, critical charge. Reserved for state that deserves
/// attention; never decoration.
pub const DANGER: Rgb565 = Rgb565::new(31, 0, 0);

/// Bluetooth connected.
///
/// Azure rather than pure blue: pure blue is the first colour to disappear when
/// the backlight dims to its lowest level, and a status symbol that vanishes
/// before the watch sleeps is worse than none.
pub const LINK: Rgb565 = Rgb565::new(8, 40, 31);
/// Nothing needs attention: a healthy battery, and the only status colour that
/// is on screen almost all the time.
///
/// A leaf green rather than the maximal `0x07E0`. Pure green is the value a
/// channel takes when nobody chose it, and being permanently in the corner it
/// was the loudest thing on a screen whose whole job was to be quiet. Shifting
/// it a little toward yellow rather than merely darkening it is what puts real
/// distance between it and [`ACCENT`] - the two sit together on every menu, and
/// hue separates them where brightness would not.
pub const OK: Rgb565 = Rgb565::new(10, 50, 6);
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
