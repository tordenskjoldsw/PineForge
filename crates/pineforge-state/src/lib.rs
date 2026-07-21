#![no_std]

//! Pure application state transitions for `PineForge`.
//!
//! This crate deliberately has no Embassy or hardware dependencies so its
//! transitions can be exercised on a host.

use heapless::Vec;

pub const SCREEN_STACK_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenId {
    Watchface,
    #[cfg(feature = "diagnostics")]
    TouchTest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    Touch { x: i32, y: i32, pressed: bool },
    Swipe(SwipeDirection),
    Tick { uptime_seconds: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenAction {
    None,
    Push(ScreenId),
    Back,
    RequestRollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEffect {
    None,
    Navigate(NavigationDirection),
    RequestRollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ButtonBounds {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Idle,
    Pressed,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonOutcome {
    None,
    Redraw,
    Activated,
}

/// Heap-free button interaction state with press-and-release activation.
pub struct Button {
    bounds: ButtonBounds,
    state: ButtonState,
}

impl Button {
    #[must_use]
    pub const fn new(bounds: ButtonBounds) -> Self {
        Self {
            bounds,
            state: ButtonState::Idle,
        }
    }

    #[must_use]
    pub const fn state(&self) -> ButtonState {
        self.state
    }

    pub fn set_enabled(&mut self, enabled: bool) -> ButtonOutcome {
        let next = if enabled {
            ButtonState::Idle
        } else {
            ButtonState::Disabled
        };
        if self.state == next {
            ButtonOutcome::None
        } else {
            self.state = next;
            ButtonOutcome::Redraw
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) -> ButtonOutcome {
        let AppEvent::Touch { x, y, pressed } = event else {
            return ButtonOutcome::None;
        };
        if self.state == ButtonState::Disabled {
            return ButtonOutcome::None;
        }

        let inside = self.bounds.contains(x, y);
        match (self.state, pressed, inside) {
            (ButtonState::Idle, true, true) => {
                self.state = ButtonState::Pressed;
                ButtonOutcome::Redraw
            }
            (ButtonState::Pressed, false, true) => {
                self.state = ButtonState::Idle;
                ButtonOutcome::Activated
            }
            (ButtonState::Pressed, _, false) => {
                self.state = ButtonState::Idle;
                ButtonOutcome::Redraw
            }
            _ => ButtonOutcome::None,
        }
    }
}

/// Owns application-wide navigation state.
///
/// Peripheral state remains owned by its Embassy task; this controller only
/// contains deterministic product state.
pub struct AppState {
    screens: Vec<ScreenId, SCREEN_STACK_CAPACITY>,
}

impl AppState {
    #[must_use]
    pub fn new(root: ScreenId) -> Self {
        let mut screens = Vec::new();
        screens
            .push(root)
            .expect("the empty screen stack always has room for its root");
        Self { screens }
    }

    #[must_use]
    pub fn active_screen(&self) -> ScreenId {
        *self
            .screens
            .last()
            .expect("AppState always retains a root screen")
    }

    /// Applies a high-level action returned by the active screen.
    pub fn transition(&mut self, action: ScreenAction) -> AppEffect {
        match action {
            ScreenAction::None => AppEffect::None,
            ScreenAction::RequestRollback => AppEffect::RequestRollback,
            ScreenAction::Back => {
                if self.screens.len() > 1 {
                    self.screens.pop();
                    AppEffect::Navigate(NavigationDirection::Backward)
                } else {
                    AppEffect::None
                }
            }
            ScreenAction::Push(screen) => {
                if self.active_screen() == screen {
                    return AppEffect::None;
                }

                if self.screens.push(screen).is_ok() {
                    AppEffect::Navigate(NavigationDirection::Forward)
                } else {
                    AppEffect::None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_cannot_be_popped() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(app.transition(ScreenAction::Back), AppEffect::None);
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn push_and_back_change_the_active_screen() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::TouchTest)),
            AppEffect::Navigate(NavigationDirection::Forward)
        );
        assert_eq!(app.active_screen(), ScreenId::TouchTest);
        assert_eq!(
            app.transition(ScreenAction::Back),
            AppEffect::Navigate(NavigationDirection::Backward)
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn duplicate_push_is_ignored() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::Watchface)),
            AppEffect::None
        );
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn full_stack_rejects_another_screen_without_losing_state() {
        let mut app = AppState::new(ScreenId::Watchface);
        for screen in [
            ScreenId::TouchTest,
            ScreenId::Watchface,
            ScreenId::TouchTest,
        ] {
            assert_eq!(
                app.transition(ScreenAction::Push(screen)),
                AppEffect::Navigate(NavigationDirection::Forward)
            );
        }

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::Watchface)),
            AppEffect::None
        );
        assert_eq!(app.active_screen(), ScreenId::TouchTest);
    }

    #[test]
    fn rollback_is_an_explicit_effect() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::RequestRollback),
            AppEffect::RequestRollback
        );
    }

    #[test]
    fn button_activates_only_after_press_and_release_inside() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));

        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: true,
            }),
            ButtonOutcome::Redraw
        );
        assert_eq!(button.state(), ButtonState::Pressed);
        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: false,
            }),
            ButtonOutcome::Activated
        );
        assert_eq!(button.state(), ButtonState::Idle);
    }

    #[test]
    fn dragging_outside_cancels_button_activation() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));
        let _ = button.handle_event(AppEvent::Touch {
            x: 20,
            y: 30,
            pressed: true,
        });

        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 200,
                y: 30,
                pressed: true,
            }),
            ButtonOutcome::Redraw
        );
        assert_eq!(button.state(), ButtonState::Idle);
    }

    #[test]
    fn disabled_button_ignores_touch() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));
        assert_eq!(button.set_enabled(false), ButtonOutcome::Redraw);

        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: true,
            }),
            ButtonOutcome::None
        );
        assert_eq!(button.state(), ButtonState::Disabled);
    }
}
