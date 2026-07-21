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
    TouchTest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    Touch { x: i32, y: i32, pressed: bool },
    Tick { uptime_seconds: u64 },
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
    Redraw,
    RequestRollback,
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
                    AppEffect::Redraw
                } else {
                    AppEffect::None
                }
            }
            ScreenAction::Push(screen) => {
                if self.active_screen() == screen {
                    return AppEffect::None;
                }

                if self.screens.push(screen).is_ok() {
                    AppEffect::Redraw
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

    #[test]
    fn push_and_back_change_the_active_screen() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::TouchTest)),
            AppEffect::Redraw
        );
        assert_eq!(app.active_screen(), ScreenId::TouchTest);
        assert_eq!(app.transition(ScreenAction::Back), AppEffect::Redraw);
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
                AppEffect::Redraw
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
}
