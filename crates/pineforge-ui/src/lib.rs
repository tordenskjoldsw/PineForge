#![no_std]
// Layout invariants written as assertions between constants.
//
// Where every part of one is const-evaluable it has been moved to a
// `const _: () = assert!(...)`, which holds for the firmware build as well and
// fails at the line that moved. The ones left here sit beside assertions that
// are not - a rectangle intersection, a slot built at runtime - and splitting
// an invariant across two places to satisfy a lint would cost more than it
// buys.
#![cfg_attr(test, allow(clippy::assertions_on_constants))]
// Lints the host test build raises and the firmware never can.
//
// `usize` is 32 bits on `thumbv7em-none-eabihf`, so a cast from it to `u32` in a
// test fixture cannot truncate anything on the target this code ships to - the
// warning exists only because tests are cross-compiled to a 64-bit host, and
// the values are literals a few bytes long either way. Denying them would mean
// writing `try_from` around test data to satisfy a machine the firmware never
// runs on.
#![cfg_attr(
    test,
    allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )
)]
// The recording surface a screen is drawn into is a panel's worth of pixels by
// definition - see `probe`. On the host that is a large local, and it is meant
// to be.
#![cfg_attr(test, allow(clippy::large_stack_arrays))]

//! Every screen `PineForge` draws, and the primitives they are built from.
//!
//! Separate from the firmware because nothing here needs a watch. A screen
//! paints into a [`Canvas`], which borrows any [`canvas::Surface`] - the panel
//! on the wrist in a firmware build, a recording buffer in a test. So the part
//! that decides where a row lands, what a menu shows, and how a message is
//! broken into lines can be checked without flashing anything.
//!
//! That was already true before this crate existed; it just could not be acted
//! on, because the code sat inside a binary that pulls in `embassy-nrf` and
//! only builds for the watch. Drawing a crate boundary is what turns the
//! property into tests.
//!
//! The one exception is instrumentation: timing a transition needs a clock, so
//! the `diagnostics` feature pulls in `embassy-time`. It is off in every build
//! the host tests run.
//!
//! [`Canvas`]: canvas::Canvas

#[cfg(test)]
extern crate std;

pub mod about;
pub mod battery;
pub mod bluetooth;
pub mod canvas;
pub mod clock_apps;
pub mod dfu;
pub mod firmware;
pub mod flashlight;
pub mod font;
pub mod icons;
pub mod launcher;
pub mod menu;
#[cfg(feature = "diagnostics")]
pub mod metrics;
pub mod modal;
pub mod music;
pub mod notifications;
pub mod pairing;
#[cfg(test)]
mod probe;
pub mod pulse;
pub mod registry;
pub mod render;
#[cfg(feature = "ui-animations")]
pub mod scratch;
pub mod screen;
pub mod segment;
pub mod setting_picker;
pub mod settings;
pub mod status;
pub mod steps;
pub mod stopwatch;
#[cfg(feature = "diagnostics")]
pub mod test_screen;
pub mod theme;
pub mod timer;
#[cfg(feature = "ui-animations")]
pub mod transition;
pub mod watchface;

#[cfg(test)]
mod tests {
    use pineforge_state::{AppEvent, DisplaySettings, ScreenId};

    use crate::{
        canvas::Canvas, probe::Probe, registry::Screens, screen::Paint, status::StatusCorner,
    };

    /// Draws a surface into a fresh probe and reports what it did.
    fn paint(surface: &dyn Paint) -> Probe {
        let mut probe = Probe::new();
        surface
            .draw_full(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    /// Runs a check over every screen the firmware has.
    ///
    /// Driven by the registry rather than a list written out here, which is the
    /// point of the registry: a screen that exists is a screen these tests
    /// cover, without anyone remembering to add it. Each one is drawn the way
    /// the display task draws it, so a screen under the status corner is
    /// checked with the corner on it.
    fn every_screen(each: &mut dyn FnMut(ScreenId, &dyn Paint)) {
        let mut screens = Screens::new();
        let corner = StatusCorner::new();
        for id in ScreenId::ALL {
            // As the display task does before anything paints, so a settings
            // leaf is pointed at a setting rather than left unconfigured.
            screens.enter(id, DisplaySettings::DEFAULT);
            screens.surface(id, &corner, &mut |surface| each(id, surface));
        }
    }

    /// The contract the slide transition rests on, and the one nothing checked
    /// until this crate could be built for the host.
    ///
    /// `draw_full` must leave no pixel untouched. The stripe renderer never
    /// clears behind a screen - clearing would double the SPI traffic of every
    /// transition - so a gap does not come out black, it comes out as whatever
    /// the previous screen had there.
    #[test]
    fn a_full_repaint_leaves_no_pixel_of_the_panel_behind() {
        every_screen(&mut |id, surface| {
            let unpainted = paint(surface).unpainted();
            assert_eq!(unpainted, 0, "{id:?} left {unpainted} pixels unpainted");
        });
    }

    /// The same contract, per face rather than per screen.
    ///
    /// `every_screen` reaches the watchface screen showing whichever face is
    /// selected, so on its own it checks one face and silently skips the rest.
    /// That was tolerable while there was one; it is how the second one would
    /// ship with a seam. Driven by `WatchfaceId::ALL`, which has its own test
    /// keeping it complete.
    ///
    /// The face is the whole panel here - a watchface wears no status corner,
    /// so nothing else is going to cover a gap it leaves.
    #[test]
    fn every_watchface_paints_the_whole_panel() {
        use pineforge_state::WatchfaceId;

        for &face in WatchfaceId::ALL {
            let mut screens = Screens::new();
            screens.enter(ScreenId::Watchface, DisplaySettings::DEFAULT);
            // Reports whether a repaint is owed, not whether the switch took:
            // selecting the face already showing is legitimately `false`. An id
            // this build lacks cannot be named here at all, because its variant
            // does not exist to be written.
            let _ = screens.watchface.select(face);

            let mut probe = Probe::new();
            screens
                .watchface
                .draw_full(&mut Canvas::new(&mut probe), &mut || {})
                .expect("the probe accepts every operation");

            let unpainted = probe.unpainted();
            assert_eq!(unpainted, 0, "{face:?} left {unpainted} pixels unpainted");
            let stray = probe.out_of_bounds();
            assert!(
                stray.is_empty(),
                "{face:?} drew outside the panel: {stray:?}"
            );
        }
    }

    /// A screen that draws past the panel is clipped by the display driver and
    /// looks almost right, which is what makes it worth failing a build over.
    #[test]
    fn no_screen_draws_outside_the_panel() {
        every_screen(&mut |id, surface| {
            let stray = paint(surface).out_of_bounds();
            assert!(stray.is_empty(), "{id:?} drew outside the panel: {stray:?}");
        });
    }

    /// Every prompt that can own the panel, with a value that exercises its
    /// widest layout: the longest passkey, a percentage of three digits, and
    /// the failure carrying the most text.
    fn every_modal() -> [pineforge_state::Modal; 6] {
        use pineforge_state::{DfuFailReason, Modal};
        [
            Modal::StorageFormat(100),
            Modal::Pairing(999_999),
            Modal::DfuProgress(100),
            Modal::DfuFailed(DfuFailReason::FlashUnrecognized([0xde, 0xad, 0xbe])),
            Modal::DfuFailed(DfuFailReason::NotConfirmed),
            Modal::TimerExpired,
        ]
    }

    /// A modal owes the same opacity contract a screen does, and for a sharper
    /// reason: nothing clears behind it, so a gap shows the screen underneath
    /// rather than black. This went unchecked while the dispatch lived in the
    /// display task, where drawing needed a panel.
    #[test]
    fn a_modal_covers_every_pixel_it_is_drawn_over() {
        for showing in every_modal() {
            let mut probe = Probe::new();
            crate::modal::draw(&mut Canvas::new(&mut probe), showing, &mut || {})
                .expect("the probe accepts every operation");
            assert_eq!(
                probe.unpainted(),
                0,
                "{showing:?} left {} pixels of the screen behind it showing",
                probe.unpainted()
            );
            assert!(
                probe.out_of_bounds().is_empty(),
                "{showing:?} drew outside the panel: {:?}",
                probe.out_of_bounds()
            );
        }
    }

    /// The partial path a transfer takes a hundred times. It may leave the rest
    /// of the prompt standing - that is the point of it - but it must not draw
    /// outside the panel while doing so.
    #[test]
    fn refreshing_a_prompt_stays_on_the_panel() {
        for showing in every_modal() {
            let mut probe = Probe::new();
            crate::modal::refresh(&mut Canvas::new(&mut probe), showing, &mut || {})
                .expect("the probe accepts every operation");
            assert!(
                probe.out_of_bounds().is_empty(),
                "{showing:?} refreshed outside the panel: {:?}",
                probe.out_of_bounds()
            );
        }
    }

    /// Builds a notification the way the parser produces one.
    fn notification(title: &str, body: &str) -> pineforge_state::Notification {
        pineforge_state::Notification {
            category: pineforge_state::NotificationCategory::Sms,
            title: heapless::String::try_from(title).expect("the title fits"),
            body: heapless::String::try_from(body).expect("the body fits"),
        }
    }

    /// Draws the notification screen exactly as the display task would.
    fn paint_notifications(screens: &Screens) -> Probe {
        screens.surface(ScreenId::Notifications, &StatusCorner::new(), &mut paint)
    }

    /// The empty inbox and a full one are different layouts - one has a title
    /// where the other has a message - so both owe the same contract. Only the
    /// empty one is reached by the sweep above, which starts every screen fresh.
    #[test]
    fn a_notification_screen_is_opaque_with_and_without_messages() {
        let mut screens = Screens::new();
        assert_eq!(
            paint_notifications(&screens).unpainted(),
            0,
            "the empty inbox left a gap"
        );

        screens
            .notifications
            .file(notification("Alice", "See you at eight"));
        let probe = paint_notifications(&screens);
        assert_eq!(probe.unpainted(), 0, "a message left a gap");
        assert!(probe.out_of_bounds().is_empty());
    }

    /// An empty inbox and a message are one layout, not two.
    ///
    /// They used to be two: a bare line of text on the background against a
    /// card, so arriving at this screen looked like arriving at a different
    /// screen depending on what was pending. The card is the thing that has to
    /// be in the same place either way, and its surface is what says so - the
    /// text inside it is legitimately different.
    #[test]
    fn an_empty_inbox_and_a_message_draw_the_same_card() {
        use embedded_graphics::{geometry::Point, prelude::Size, primitives::Rectangle};

        // A patch inside the card, clear of the text, in both states.
        let card = Rectangle::new(Point::new(28, 190), Size::new(20, 10));

        let mut screens = Screens::new();
        assert!(
            paint_notifications(&screens).painted_in(card, crate::theme::SURFACE),
            "the empty inbox drew no card"
        );

        screens
            .notifications
            .file(notification("Alice", "See you at eight"));
        assert!(
            paint_notifications(&screens).painted_in(card, crate::theme::SURFACE),
            "a message drew no card where the empty inbox had one"
        );
    }

    /// The longest text the parser can produce, in the tightest layout.
    ///
    /// A title is cut to the line and a body is wrapped, but both are done by
    /// arithmetic over a font cell - and arithmetic that is one column out
    /// still draws, just off the edge.
    #[test]
    fn the_longest_message_the_parser_accepts_still_fits_the_panel() {
        let mut screens = Screens::new();
        screens.notifications.file(notification(
            &"W".repeat(pineforge_state::NOTIFICATION_TITLE_MAX),
            &"supercalifragilistic ".repeat(5)[..pineforge_state::NOTIFICATION_BODY_MAX],
        ));

        let probe = paint_notifications(&screens);
        assert!(
            probe.out_of_bounds().is_empty(),
            "an overlong message ran off the panel: {:?}",
            probe.out_of_bounds()
        );
    }

    /// A partial repaint must not touch the hint at the foot.
    ///
    /// Browsing repaints only the message, which is what keeps a page turn
    /// cheap; the hint is drawn once by `draw_full`. If the cleared region
    /// reached the hint's glyphs it would erase them and never put them back,
    /// so the hint would vanish on the first swipe and stay gone.
    #[test]
    fn browsing_a_notification_leaves_the_hint_standing() {
        use embedded_graphics::{geometry::Point, primitives::Rectangle};

        let mut screens = Screens::new();
        screens.notifications.file(notification("Alice", "first"));
        screens.notifications.file(notification("Bob", "second"));
        let _ = screens.handle(
            ScreenId::Notifications,
            AppEvent::Swipe(pineforge_state::SwipeDirection::Down),
        );

        let mut probe = Probe::new();
        screens
            .draw_dirty(
                ScreenId::Notifications,
                &mut Canvas::new(&mut probe),
                &mut || {},
            )
            .expect("the probe accepts every operation");

        // The hint's cell in the UI face, at the baseline every screen uses.
        let hint = Rectangle::new(
            Point::new(0, 215),
            embedded_graphics::geometry::Size::new(240, 25),
        );
        assert!(
            !probe.painted_within(hint),
            "the partial repaint reached into the hint"
        );
    }
}
