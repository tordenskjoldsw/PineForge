//! Every screen the firmware has, and which one a [`ScreenId`] names.
//!
//! This table used to sit in the display task, beside the SPI bus and the power
//! state machine. That put the answer to "what screens exist" in the one file
//! that also owns the panel, so adding an app meant editing the task that
//! drives the hardware - and meant the host tests had to name every screen a
//! second time, where forgetting one was silent.
//!
//! Here instead, next to the screens. The display task asks for the active one
//! and gets a `&dyn Screen`; the tests walk [`ScreenId::ALL`] through the same
//! registry, so a screen that exists is a screen they check.
//!
//! Fields are public where the firmware legitimately reaches past the active
//! screen: a firmware image is confirmed, a face is selected, a notification is
//! filed, whichever screen happens to be showing. What stays private is the
//! settings leaf, because nothing outside [`Screens::enter`] may point it
//! anywhere.

use pineforge_state::{
    AppEvent, CalendarDate, DisplaySettings, ScreenAction, ScreenId, SwipeDirection, WallTime,
};

#[cfg(feature = "diagnostics")]
use crate::test_screen::TestScreen;
use crate::{
    about::AboutScreen,
    battery::BatteryScreen,
    bluetooth::BluetoothScreen,
    canvas::{Canvas, CanvasError},
    clock_apps::{DateScreen, TimeScreen},
    firmware::FirmwareScreen,
    flashlight::FlashlightScreen,
    launcher::LauncherScreen,
    music::MusicScreen,
    notifications::NotificationScreen,
    pulse::PulseScreen,
    screen::{Paint, Screen},
    setting_picker::{SettingPickerScreen, setting_of},
    settings::DisplaySettingsScreen,
    status::{StatusCorner, WithStatus, wears_status},
    steps::StepsScreen,
    stopwatch::StopwatchScreen,
    timer::TimerScreen,
    watchface::WatchfaceScreen,
    weather::WeatherScreen,
};

/// Every screen instance, dispatching to whichever one the navigation state
/// says is active.
///
/// Screens are held for the lifetime of the firmware so their model state
/// survives navigation and sleep; only the active one receives events and
/// draws.
#[derive(Default)]
pub struct Screens {
    pub watchface: WatchfaceScreen,
    pub launcher: LauncherScreen,
    /// Holds the pending notifications, whether or not it is the screen showing.
    pub notifications: NotificationScreen,
    pub settings: DisplaySettingsScreen,
    /// The one settings leaf, pointed at whichever setting is being edited.
    picker: SettingPickerScreen,
    pub firmware: FirmwareScreen,
    pub about: AboutScreen,
    /// Holds whether the lamp is lit, which the display task reads when it
    /// decides how bright the panel should be.
    pub flashlight: FlashlightScreen,
    /// Holds the last heart-rate reading, fed from the event stream whatever is
    /// showing - see [`Screens::absorb`].
    pub pulse: PulseScreen,
    /// Holds the day's step count, fed the same way and for the same reason.
    pub steps: StepsScreen,
    /// Holds what the phone reported it is playing, fed the same way and for
    /// the same reason - a track that changed while the face was up must not
    /// leave a stale title here.
    pub music: MusicScreen,
    /// Keeps its monotonic anchor across navigation and display sleep.
    pub stopwatch: StopwatchScreen,
    /// Owns the selected duration and monotonic deadline across every screen.
    pub timer: TimerScreen,
    pub time: TimeScreen,
    pub date: DateScreen,
    pub bluetooth: BluetoothScreen,
    pub battery: BatteryScreen,
    /// What the phone last said the weather is, filed straight in by the
    /// display task the way a track is - the record is far too large to carry
    /// through the event channel.
    pub weather: WeatherScreen,
    #[cfg(feature = "diagnostics")]
    pub touch_test: TestScreen,
}

impl Screens {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The screen the navigation state says is active.
    ///
    /// One place naming every screen, rather than one per operation. `Screen`
    /// is object-safe since screens stopped being generic over their draw
    /// target, so adding a screen is an arm here and an arm in the mutable twin
    /// below - not an arm in every match that touches screens.
    #[must_use]
    pub fn active(&self, active: ScreenId) -> &dyn Screen {
        match active {
            ScreenId::Watchface => &self.watchface,
            ScreenId::Launcher => &self.launcher,
            ScreenId::Notifications => &self.notifications,
            ScreenId::DisplaySettings => &self.settings,
            ScreenId::Firmware => &self.firmware,
            ScreenId::About => &self.about,
            ScreenId::Flashlight => &self.flashlight,
            ScreenId::Pulse => &self.pulse,
            ScreenId::Steps => &self.steps,
            ScreenId::Music => &self.music,
            ScreenId::Stopwatch => &self.stopwatch,
            ScreenId::Timer => &self.timer,
            ScreenId::Time => &self.time,
            ScreenId::Date => &self.date,
            ScreenId::Bluetooth => &self.bluetooth,
            ScreenId::Battery => &self.battery,
            ScreenId::Weather => &self.weather,
            // Every leaf is the same screen; `enter` has pointed it at the one
            // this id names before it can be drawn or touched.
            ScreenId::Brightness
            | ScreenId::DimTimeout
            | ScreenId::OffTimeout
            | ScreenId::HeartRate
            | ScreenId::WakeGesture
            | ScreenId::WatchfaceSelect => &self.picker,
            #[cfg(feature = "diagnostics")]
            ScreenId::TouchTest => &self.touch_test,
        }
    }

    pub fn active_mut(&mut self, active: ScreenId) -> &mut dyn Screen {
        match active {
            ScreenId::Watchface => &mut self.watchface,
            ScreenId::Launcher => &mut self.launcher,
            ScreenId::Notifications => &mut self.notifications,
            ScreenId::DisplaySettings => &mut self.settings,
            ScreenId::Firmware => &mut self.firmware,
            ScreenId::About => &mut self.about,
            ScreenId::Flashlight => &mut self.flashlight,
            ScreenId::Pulse => &mut self.pulse,
            ScreenId::Steps => &mut self.steps,
            ScreenId::Music => &mut self.music,
            ScreenId::Stopwatch => &mut self.stopwatch,
            ScreenId::Timer => &mut self.timer,
            ScreenId::Time => &mut self.time,
            ScreenId::Date => &mut self.date,
            ScreenId::Bluetooth => &mut self.bluetooth,
            ScreenId::Battery => &mut self.battery,
            ScreenId::Weather => &mut self.weather,
            // Every leaf is the same screen; `enter` has pointed it at the one
            // this id names before it can be drawn or touched.
            ScreenId::Brightness
            | ScreenId::DimTimeout
            | ScreenId::OffTimeout
            | ScreenId::HeartRate
            | ScreenId::WakeGesture
            | ScreenId::WatchfaceSelect => &mut self.picker,
            #[cfg(feature = "diagnostics")]
            ScreenId::TouchTest => &mut self.touch_test,
        }
    }

    /// Hands a reading to every screen that keeps one, and reports whether the
    /// screen currently showing moved because of it.
    ///
    /// Readings are facts about the watch rather than messages to whichever
    /// screen happens to be up, and more than one screen holds them: the face
    /// shows a pulse alongside everything else, the pulse app shows the same
    /// reading on its own. Feeding them here, once, is what keeps a number that
    /// arrived while the face was up from going stale behind the app.
    ///
    /// The display task must not then hand the same event to the active screen
    /// again - a second apply finds nothing changed and cancels the repaint the
    /// first one earned. [`Self::holds_readings`] says which screens are
    /// covered here and therefore must be skipped there.
    pub fn absorb(&mut self, active: ScreenId, event: AppEvent) -> bool {
        let _ = self.watchface.handle_event(event);
        let _ = self.pulse.handle_event(event);
        let _ = self.steps.handle_event(event);
        // The music screen takes the connection state the same way, so its
        // controls are right the moment it is opened rather than at the next
        // Bluetooth event after that.
        let _ = self.music.handle_event(event);
        // System probes happen once near boot and stack/BLE updates may arrive
        // while another app is showing. About is a retained status surface,
        // not a live-only diagnostic view.
        let _ = self.about.handle_event(event);
        let _ = self.bluetooth.handle_event(event);
        // The charge arrives on a ten-minute timer, so a screen that only took
        // it while showing would open on whatever was true when it was last up.
        let _ = self.battery.handle_event(event);
        match active {
            ScreenId::Watchface => self.watchface.moved(),
            ScreenId::Pulse => self.pulse.moved(),
            ScreenId::Steps => self.steps.moved(),
            ScreenId::Music => self.music.moved(),
            ScreenId::About => self.about.moved(),
            ScreenId::Bluetooth => self.bluetooth.moved(),
            ScreenId::Battery => self.battery.moved(),
            _ => false,
        }
    }

    /// Whether this screen took its reading from [`Self::absorb`] already.
    #[must_use]
    pub const fn holds_readings(active: ScreenId) -> bool {
        matches!(
            active,
            ScreenId::Watchface
                | ScreenId::Pulse
                | ScreenId::Steps
                | ScreenId::Music
                | ScreenId::About
                | ScreenId::Bluetooth
                | ScreenId::Battery
        )
    }

    pub fn handle(&mut self, active: ScreenId, event: AppEvent) -> ScreenAction {
        self.active_mut(active).handle_event(event)
    }

    /// Whether the active screen will spend this swipe on itself.
    ///
    /// Asked before navigation is offered the gesture, and answered no by
    /// everything except a screen paging along the axis it was entered on. See
    /// [`Screen::claims`] for why that question has to be put in advance.
    #[must_use]
    pub fn claims(&self, active: ScreenId, direction: SwipeDirection) -> bool {
        self.active(active).claims(direction)
    }

    /// Hands the screen that is now on top whatever it needs to be correct.
    ///
    /// A settings leaf needs the record that is current rather than one it kept
    /// from an earlier visit, and a watchface's notification tally is the
    /// inbox's rather than a count of its own. Both are the same rule: a screen
    /// is given shared state as it is entered instead of keeping a copy across
    /// visits, so neither can show something the firmware no longer believes.
    ///
    /// Called on every navigation, so a screen cannot be reached without it.
    pub fn enter(&mut self, active: ScreenId, settings: DisplaySettings) {
        if let Some(setting) = setting_of(active) {
            self.picker.open(setting, settings);
        }
        // The lamp opens dark, whichever way it was left.
        if active == ScreenId::Flashlight {
            self.flashlight.put_out();
        }
        if active == ScreenId::Bluetooth {
            self.bluetooth.open(settings);
        }
        // This is what carries a dismissal back to the face: the notification
        // screen changes the inbox, and leaving it is when the face is told.
        let _ = self
            .watchface
            .handle_event(AppEvent::NotificationsChanged(self.notifications.summary()));
    }

    /// Opens a manual clock editor on the value current at navigation time.
    pub fn enter_clock(
        &mut self,
        active: ScreenId,
        wall_time: Option<WallTime>,
        date: Option<CalendarDate>,
    ) {
        if active == ScreenId::Time {
            self.time.open(wall_time.unwrap_or(WallTime::MIDNIGHT));
        } else if active == ScreenId::Date {
            self.date.open(date.unwrap_or(CalendarDate::DEFAULT));
        }
    }

    /// Paints a screen and, unless it is a watchface, the status corner over
    /// it. The two are drawn together so a transition composing this stripe by
    /// stripe carries the corner with it instead of adding it afterwards.
    pub fn draw_full(
        &mut self,
        active: ScreenId,
        status: &StatusCorner,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let painted = self.surface(active, status, &mut |surface| {
            surface.draw_full(canvas, keep_alive)
        });
        self.painted();
        painted
    }

    pub fn draw_dirty(
        &mut self,
        active: ScreenId,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let painted = self.active(active).draw_dirty(canvas, keep_alive);
        self.painted();
        painted
    }

    /// Draws the line under a pass that reached the panel.
    ///
    /// Two screens need it, and for the same reason: both collect changes while
    /// something else is on the panel. The face takes readings whatever is
    /// showing, and the inbox is filed into by the display task from wherever
    /// the user happens to be. Neither can clear its own mark, because a screen
    /// draws from a shared reference.
    ///
    /// Called from the two methods above, and by hand on the transition path,
    /// which composes through [`Self::surface`] instead.
    ///
    /// Cleared whichever screen was painted, on purpose. Clearing only when the
    /// screen itself was drawn would mean a mark that never empties while the
    /// user is anywhere else - so everything that arrived behind a menu would
    /// look owed, and coming back would repaint what had not moved. The full
    /// repaint that navigation performs is what makes clearing here correct.
    pub fn painted(&mut self) {
        self.watchface.mark_painted();
        self.notifications.mark_painted();
        self.steps.mark_painted();
        self.music.mark_painted();
        self.stopwatch.mark_painted();
        self.timer.mark_painted();
        self.time.mark_painted();
        self.date.mark_painted();
        self.bluetooth.mark_painted();
        self.battery.mark_painted();
        self.weather.mark_painted();
        self.about.mark_painted();
    }

    /// Lends the active screen as the opaque surface it is drawn as - the
    /// screen itself, or the screen wearing the status corner.
    ///
    /// A callback rather than a return value because the wrapper borrows both
    /// the screen and the corner, so it cannot outlive the call. This is what
    /// the transition composes, and what a test asks for its coverage: both
    /// have to see the same thing the panel does, corner included.
    pub fn surface<T>(
        &self,
        active: ScreenId,
        status: &StatusCorner,
        with: &mut dyn FnMut(&dyn Paint) -> T,
    ) -> T {
        let screen = self.active(active);
        if wears_status(active) {
            with(&WithStatus::new(screen, status))
        } else {
            with(screen)
        }
    }
}

/// The three lists above have to agree, and none of them can see the others.
///
/// [`Screens::absorb`] names the screens that are fed a reading,
/// [`Screens::holds_readings`] names the ones the display task must therefore
/// not hand the same event to a second time, and [`Screens::painted`] names the
/// ones whose dirty mark a pass over the panel clears. A screen added to one and
/// forgotten in another is not a compile error and not a visibly wrong screen -
/// it is a repaint that silently stops happening, which is the same shape as the
/// bug that left the battery percentage sitting where it was at boot.
///
/// So the agreement is checked here rather than left to whoever adds the next
/// screen. Driven by `ScreenId::ALL`, the way the opaque-paint tests are, so a
/// screen that exists is a screen these cover.
#[cfg(test)]
mod tests {
    use pineforge_state::{
        AccelerometerKind, AppEvent, BatteryStatus, BleState, DisplaySettings, FirmwareImageState,
        FlashStatus, HeartRateSensorKind, HeartRateState, NotificationSummary, PeripheralStatus,
        PpgAnalysis, ScreenId, StackUsage, StorageState,
    };

    use super::Screens;
    use crate::{canvas::Canvas, probe::Probe};

    /// One reading of every kind the display task routes through `absorb`,
    /// each carrying a value a freshly built screen does not already hold.
    ///
    /// Written out rather than derived, because `AppEvent` cannot be
    /// enumerated. The test below keeps the list honest in the one direction
    /// that can be checked - every entry really is a reading - and
    /// `every_screen_that_holds_readings_is_fed_by_absorb` catches the other
    /// direction wherever it matters: a reading missing here that some screen
    /// depends on leaves that screen with nothing to move it.
    fn every_reading() -> [AppEvent; 14] {
        [
            AppEvent::BatteryUpdated(BatteryStatus {
                millivolts: 3900,
                percent: 71,
                charging: false,
                power_present: false,
            }),
            AppEvent::StepsUpdated(4321),
            AppEvent::BleUpdated(BleState::Connected),
            AppEvent::NotificationsChanged(NotificationSummary {
                count: 2,
                latest: None,
            }),
            AppEvent::HeartRateStateUpdated(HeartRateState::Measuring),
            AppEvent::HeartRateAnalysisUpdated(PpgAnalysis::HeartRate { bpm: 64 }),
            AppEvent::HeartRateSensorDetected(HeartRateSensorKind::Hrs3300),
            AppEvent::MusicUpdated,
            AppEvent::AccelerometerDetected(AccelerometerKind::Bma421),
            AppEvent::TouchControllerUpdated(PeripheralStatus::Ready),
            AppEvent::FlashUpdated(FlashStatus::Ready([0xC8, 0x40, 0x16])),
            AppEvent::FirmwareImageUpdated(FirmwareImageState::Confirmed),
            AppEvent::StackUpdated(StackUsage {
                used: 10_432,
                capacity: 16_384,
            }),
            AppEvent::StorageUpdated(StorageState::Ready),
        ]
    }

    /// A screen entered the way the display task enters it, so a settings leaf
    /// is pointed at a setting rather than left unconfigured.
    fn entered(active: ScreenId) -> Screens {
        let mut screens = Screens::new();
        screens.enter(active, DisplaySettings::DEFAULT);
        screens
    }

    #[test]
    fn the_events_driven_here_are_the_ones_the_display_task_absorbs() {
        for event in every_reading() {
            assert!(
                event.is_reading(),
                "{event:?} is not a reading, so the display task never absorbs it"
            );
        }
    }

    /// `absorb` reporting a move is what earns a partial repaint, and the
    /// display task only performs it for a screen `holds_readings` names. A
    /// screen that moves without being named draws nothing at all: the event is
    /// consumed, the model is current, and the panel keeps showing the old
    /// value until something unrelated repaints it.
    #[test]
    fn a_screen_that_moves_on_a_reading_is_one_the_display_task_will_repaint() {
        for event in every_reading() {
            for active in ScreenId::ALL {
                let mut screens = entered(active);
                assert!(
                    !screens.absorb(active, event) || Screens::holds_readings(active),
                    "{active:?} moved on {event:?} but holds_readings does not name it, \
                     so the repaint it earned is never drawn"
                );
            }
        }
    }

    /// The other direction. `holds_readings` makes the display task skip the
    /// ordinary dispatch and rely on `absorb` alone, so a screen named here that
    /// `absorb` does not feed never sees a reading by either route.
    #[test]
    fn every_screen_that_holds_readings_is_fed_by_absorb() {
        for active in ScreenId::ALL {
            if !Screens::holds_readings(active) {
                continue;
            }
            let moved = every_reading()
                .into_iter()
                .any(|event| entered(active).absorb(active, event));
            assert!(
                moved,
                "{active:?} is named by holds_readings, so the display task hands it \
                 no reading of its own - but absorb moves it with none either"
            );
        }
    }

    /// `painted` draws the line under a pass that reached the panel. A screen
    /// `absorb` marks and `painted` does not clear stays dirty for the life of
    /// the firmware, so every later repaint redraws what has not moved.
    #[test]
    fn painting_clears_every_mark_a_reading_set() {
        for active in ScreenId::ALL {
            let mut screens = entered(active);
            for event in every_reading() {
                let _ = screens.absorb(active, event);
            }
            screens.painted();

            let mut probe = Probe::new();
            screens
                .draw_dirty(active, &mut Canvas::new(&mut probe), &mut || {})
                .expect("the probe accepts every operation");
            assert_eq!(
                probe.unpainted(),
                240 * 240,
                "{active:?} still had a mark to paint after painted() cleared them"
            );
        }
    }
}
