//! The notifications that have arrived and not been dismissed, one per page.
//!
//! Pulling down from the watchface brings this up, the way a phone's shade
//! comes down. One message fills the screen rather than a list of subject
//! lines: a 240-pixel panel fits about six lines of body text, so a list would
//! show almost nothing of any message and still cost a second screen to read
//! one.
//!
//! # Gestures
//!
//! Which gestures are free here is not this screen's choice. A screen is left
//! by the reverse of the gesture that opened it, so up is spent leaving, and
//! what remains is down and the horizontal pair:
//!
//! - **down** shows the next message, wrapping at the oldest - the same finger
//!   movement that opened the screen keeps going through it
//! - **right** dismisses the message showing, and dismissing the last one
//!   leaves the screen, because an empty inbox is nothing to stand in
//! - **up** goes back to the watchface
//!
//! Dismissing sits on the other axis from browsing on purpose. Both are single
//! swipes and one of them destroys something, so a gesture that lands short or
//! crooked must not be able to delete the message being read.
//!
//! The messages themselves live in [`NotificationInbox`], which is where every
//! rule about them is tested: what an arrival does to the cursor, where it
//! lands after a dismissal, and that it never points past the end.

use core::fmt::Write;

use embedded_graphics::{geometry::Point, pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use heapless::String;
use pineforge_state::{
    AppEvent, Notification, NotificationInbox, NotificationSummary, ScreenAction, SwipeDirection,
    wrap,
};

use crate::{
    canvas::{Canvas, CanvasError},
    font::{hint_text, ui_text},
    render::{ROW_WIDTH, ROW_X, draw_mono_text_visible},
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

const SCREEN_WIDTH: u32 = 240;

/// Baseline of the category and position line, just under the status corner.
const HEAD_BASELINE_Y: i32 = STATUS_HEIGHT + 14;
/// Baseline of the sender, set in the UI face.
const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 48;
/// Rule between the sender and the message.
const RULE_Y: i32 = STATUS_HEIGHT + 58;
const BODY_FIRST_BASELINE_Y: i32 = STATUS_HEIGHT + 80;
const BODY_LINE_STEP: i32 = 16;
/// Lines of body text between the rule and the hint at the foot.
const BODY_LINES: usize = 6;
/// Baseline of the hint, where every other screen puts it.
const HINT_BASELINE_Y: i32 = 232;

/// Characters that fit across the content width in each face.
///
/// Both faces are monospace, so this is the content width over the cell width
/// rather than a measurement. The assertion below is what keeps it true: a
/// changed row width would otherwise silently start cutting text a few pixels
/// early, or running it off the edge.
const TITLE_COLUMNS: usize = (ROW_WIDTH / 10) as usize;
const BODY_COLUMNS: usize = (ROW_WIDTH / 6) as usize;
const _: () = assert!(ROW_WIDTH == 200, "the column counts assume a 200px row");

/// Everything between the status corner and the hint, repainted as one when the
/// message showing changes.
///
/// Cleared rather than overwritten because the lines differ in length: the text
/// renderer paints each glyph's own cell, so a shorter line would leave the tail
/// of the previous one standing beside it.
const CONTENT_TOP: i32 = STATUS_HEIGHT;
const CONTENT_BOTTOM: i32 = HINT_BASELINE_Y - 20;

const HINT_BROWSING: &str = "> CLEAR  v NEXT";
const HINT_EMPTY: &str = "^ BACK";

/// The notification screen, and the inbox it reads from.
///
/// The inbox lives here rather than in the display task because this is the
/// only screen that reads the messages. What the rest of the firmware needs -
/// how many are pending - travels as a [`NotificationSummary`], so a watchface
/// showing a tally holds no notifications and cannot disagree with this screen
/// about how many there are.
#[derive(Default)]
pub struct NotificationScreen {
    inbox: NotificationInbox,
    dirty: bool,
}

impl NotificationScreen {
    /// Files an arriving notification and reports what is now pending.
    ///
    /// Called by the display task whichever screen is up: a message that
    /// arrived while the user was elsewhere still has to be there when they
    /// pull the shade down.
    pub fn file(&mut self, notification: Notification) -> NotificationSummary {
        self.dirty = true;
        self.inbox.file(notification)
    }

    /// What is pending, for whoever shows a tally rather than the messages.
    #[must_use]
    pub fn summary(&self) -> NotificationSummary {
        self.inbox.summary()
    }

    /// The message showing, or the empty state.
    ///
    /// The hint is not part of this: it does not change while a message is being
    /// read, so a page turn costs the message and nothing else.
    fn draw_message(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let Some(notification) = self.inbox.showing() else {
            return draw_title("NO MESSAGES", canvas);
        };

        draw_hint(
            notification.category.label(),
            ROW_X,
            HEAD_BASELINE_Y,
            theme::ACCENT,
            canvas,
        )?;
        self.draw_position(canvas)?;
        keep_alive();

        // A sender longer than the line is cut rather than wrapped: the second
        // line of a sender is worth less than the first line of the message.
        draw_title(truncate(&notification.title, TITLE_COLUMNS), canvas)?;
        canvas.fill_solid(
            &Rectangle::new(Point::new(ROW_X, RULE_Y), Size::new(ROW_WIDTH as u32, 1)),
            theme::FRAME,
        )?;
        keep_alive();

        for (index, line) in wrap(&notification.body, BODY_COLUMNS)
            .take(BODY_LINES)
            .enumerate()
        {
            let y = BODY_FIRST_BASELINE_Y + BODY_LINE_STEP * i32::try_from(index).unwrap_or(0);
            draw_hint(line, ROW_X, y, theme::TEXT, canvas)?;
            keep_alive();
        }
        Ok(())
    }

    /// What a gesture this screen owns does; see the module note on which ones
    /// reach it.
    fn swipe(&mut self, direction: SwipeDirection) -> ScreenAction {
        match direction {
            SwipeDirection::Down => {
                // A single message has nowhere to go, and repainting it would
                // flash the screen for a gesture that did nothing.
                self.dirty = self.inbox.show_next();
                ScreenAction::None
            }
            SwipeDirection::Right => {
                if !self.inbox.dismiss_showing() {
                    return ScreenAction::None;
                }
                self.dirty = true;
                if self.inbox.is_empty() {
                    // Nothing left to stand in front of. Leaving is also what
                    // repaints the watchface, which is how its tally learns the
                    // inbox is empty.
                    return ScreenAction::Back;
                }
                ScreenAction::None
            }
            // Left is unused, and up belongs to navigation.
            SwipeDirection::Left | SwipeDirection::Up => ScreenAction::None,
        }
    }

    /// The position within the inbox, right-aligned against the content edge so
    /// it cannot collide with a long category name on the left.
    fn draw_position(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let mut text: String<8> = String::new();
        let _ = write!(text, "{}/{}", self.inbox.position(), self.inbox.len());
        let width = i32::try_from(text.len()).unwrap_or(0) * 6;
        draw_hint(
            &text,
            ROW_X + ROW_WIDTH - width,
            HEAD_BASELINE_Y,
            theme::TEXT,
            canvas,
        )
    }
}

impl Paint for NotificationScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        canvas.clear(theme::BACKGROUND)?;
        keep_alive();
        self.draw_message(canvas, keep_alive)?;
        let hint = if self.inbox.is_empty() {
            HINT_EMPTY
        } else {
            HINT_BROWSING
        };
        draw_mono_text_visible(
            hint,
            Point::new(ROW_X, HINT_BASELINE_Y),
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
        )
    }
}

impl Screen for NotificationScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        match event {
            AppEvent::Swipe(direction) => self.swipe(direction),
            // The inbox changed underneath this screen; `file` has already
            // marked it, so this must not clear the mark below.
            AppEvent::NotificationsChanged(_) => ScreenAction::None,
            // A tick arrives every second, and a repaint it does not need would
            // flash the message once a second.
            _ => {
                self.dirty = false;
                ScreenAction::None
            }
        }
    }
    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if !self.dirty {
            return Ok(());
        }
        canvas.fill_solid(
            &Rectangle::new(
                Point::new(0, CONTENT_TOP),
                Size::new(
                    SCREEN_WIDTH,
                    u32::try_from(CONTENT_BOTTOM - CONTENT_TOP).unwrap_or(0),
                ),
            ),
            theme::BACKGROUND,
        )?;
        self.draw_message(canvas, keep_alive)
    }
}

fn draw_title(title: &str, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
    draw_mono_text_visible(
        title,
        Point::new(ROW_X, TITLE_BASELINE_Y),
        ui_text(theme::TEXT, theme::BACKGROUND),
        canvas,
    )
}

fn draw_hint(
    text: &str,
    x: i32,
    baseline: i32,
    ink: Rgb565,
    canvas: &mut Canvas<'_>,
) -> Result<(), CanvasError> {
    draw_mono_text_visible(
        text,
        Point::new(x, baseline),
        hint_text(ink, theme::BACKGROUND),
        canvas,
    )
}

/// The first `columns` characters, cut on a character boundary.
fn truncate(text: &str, columns: usize) -> &str {
    match text.char_indices().nth(columns) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}
