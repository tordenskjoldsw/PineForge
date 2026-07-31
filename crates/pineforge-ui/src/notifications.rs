//! The notifications that have arrived and not been dismissed, one per page.
//!
//! Pulling down from the watchface brings this up, the way a phone's shade
//! comes down. One message fills the screen rather than a list of subject
//! lines: a 240-pixel panel fits about six lines of body text, so a list would
//! show almost nothing of any message and still cost a second screen to read
//! one.
//!
//! # The card
//!
//! A message is drawn as a raised surface with rounded corners - the same shape
//! a launcher tile and a menu row have, from the same primitives. That is not
//! decoration: the gesture that clears a message is a swipe, and a swipe reads
//! as moving something. Text on the background has nothing to move; a card
//! does, so the screen says what the gesture will do before it is made.
//!
//! It also puts the message inside a boundary, which is what a wrapped
//! paragraph needs to look deliberate rather than merely left-aligned.
//!
//! The position within the inbox is the page rail, beside the card rather than
//! inside it. Vertical, because the rail runs along the axis the list pages on
//! and this list pages downward - see [`draw_page_marks`]. Outside, because a
//! rail within the card would take its width from every line of text for the
//! sake of a mark that is absent whenever there is only one message.
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

use embedded_graphics::{
    geometry::Point,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use pineforge_state::{
    AppEvent, NOTIFICATION_BODY_MAX, Notification, NotificationInbox, NotificationSummary,
    PageAxis, PagedList, ScreenAction, SwipeDirection, wrap,
};

use crate::{
    canvas::{Canvas, CanvasError},
    font::{JETBRAINS_MONO_8X18, JETBRAINS_MONO_10X22, body_text, hint_text, ui_text},
    render::{PANEL, ROW_WIDTH, ROW_X, draw_mono_text_visible, draw_page_marks, round_corners},
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

/// The card: the same column a launcher tile and a menu row occupy, so the
/// three read as one product rather than three layouts that share a panel.
const CARD_X: i32 = ROW_X;
const CARD_WIDTH: i32 = ROW_WIDTH;
/// Clear of the status corner, and stopping short of the hint at the foot.
const CARD_TOP: i32 = STATUS_HEIGHT + 4;
const CARD_BOTTOM: i32 = 208;
/// Inset of the card's content from its edge, matching a menu row's margins.
const CARD_PADDING: i32 = 12;

const TEXT_X: i32 = CARD_X + CARD_PADDING;
/// Width the text has inside the card, which is what the column counts divide.
const CONTENT_WIDTH: i32 = CARD_WIDTH - 2 * CARD_PADDING;

/// Baselines within the card, measured from its top edge so moving the card
/// moves its contents with it.
const CATEGORY_BASELINE_Y: i32 = CARD_TOP + 18;
const TITLE_BASELINE_Y: i32 = CARD_TOP + 44;
const RULE_Y: i32 = CARD_TOP + 52;
const BODY_FIRST_BASELINE_Y: i32 = CARD_TOP + 76;
/// Two pixels of leading over the cell, which is what stops a wrapped
/// paragraph reading as one block of texture.
const BODY_LINE_STEP: i32 = 20;
/// Lines of body text between the rule and the foot of the card.
///
/// Six at 22 columns is 132 characters, against a body the parser caps at 100 -
/// so the longest message the watch can receive still wraps inside the card
/// rather than being cut. The assertion below keeps that true if either number
/// moves.
const BODY_LINES: usize = 6;

/// Centre of the page rail, in the margin to the right of the card.
const RAIL_CENTER: Point = Point::new(230, CARD_TOP.midpoint(CARD_BOTTOM));

/// Baseline of the hint, where every other screen puts it.
const HINT_BASELINE_Y: i32 = 232;

/// Characters that fit across the card's content in each face.
///
/// Every face is monospace, so this is the content width over the cell width
/// rather than a measurement. Taken from the fonts themselves rather than
/// written out, so a later change of face moves the wrap with it instead of
/// silently cutting text a few pixels early or running it off the card.
const CONTENT_WIDTH_PX: u32 = CONTENT_WIDTH.unsigned_abs();
const TITLE_COLUMNS: usize = (CONTENT_WIDTH_PX / JETBRAINS_MONO_10X22.cell.width) as usize;
const BODY_COLUMNS: usize = (CONTENT_WIDTH_PX / JETBRAINS_MONO_8X18.cell.width) as usize;
const _: () = assert!(
    BODY_COLUMNS * BODY_LINES >= NOTIFICATION_BODY_MAX,
    "the card cannot hold the longest body the parser accepts"
);

/// What the card says when the inbox is empty, in the slots a message uses.
const EMPTY_CATEGORY: &str = "INBOX";
const EMPTY_TITLE: &str = "NO MESSAGES";

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
    /// Whether the message showing has changed since the panel last drew it.
    ///
    /// Set by anything that moves the cursor or the contents, and cleared only
    /// by [`Self::mark_painted`] - never by an unrelated event. It used to be
    /// cleared by the catch-all arm of `handle_event`, which meant any event
    /// this screen did not care about could retire a repaint that was still
    /// owed. The same shape cost the watchface every reading it took while the
    /// panel was dark.
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

    /// Says the panel now shows the message this screen holds.
    pub const fn mark_painted(&mut self) {
        self.dirty = false;
    }

    /// The inbox as a list of one-message pages, which is what the rail draws.
    ///
    /// Built here rather than kept, because the inbox is the record and a
    /// second copy of where the cursor is could disagree with it. Advancing by
    /// `next_page` rather than by a setter is bounded by
    /// `NOTIFICATION_CAPACITY`, which is four.
    fn pages(&self) -> PagedList {
        let mut list = PagedList::new(self.inbox.len(), 1);
        for _ in 1..self.inbox.position() {
            let _ = list.next_page();
        }
        list
    }

    /// Fills the card and rounds its corners, without its contents.
    fn draw_card_face(canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let card = Rectangle::new(
            Point::new(CARD_X, CARD_TOP),
            Size::new(
                u32::try_from(CARD_WIDTH).unwrap_or(0),
                u32::try_from(CARD_BOTTOM - CARD_TOP).unwrap_or(0),
            ),
        );
        card.into_styled(PrimitiveStyle::with_fill(theme::SURFACE))
            .draw(canvas)?;
        round_corners(&card, canvas)
    }

    /// What the card shows: a message, or the fact that there is none.
    ///
    /// One shape for both, and that is the point. An empty inbox used to be a
    /// bare line of text on the background while a message got a card, so the
    /// two states of one screen were two layouts - and the jump between them
    /// was the most conspicuous thing about arriving here. Now the card is
    /// always the card; only what is written in it changes.
    fn contents(&self) -> (&str, &str, &str) {
        self.inbox
            .showing()
            .map_or((EMPTY_CATEGORY, EMPTY_TITLE, ""), |notification| {
                (
                    notification.category.label(),
                    // A sender longer than the line is cut rather than wrapped:
                    // the second line of a sender is worth less than the first
                    // line of the message.
                    truncate(&notification.title, TITLE_COLUMNS),
                    notification.body.as_str(),
                )
            })
    }

    /// The card's text, over a card that is already there.
    ///
    /// Every glyph paints its own cell against [`theme::SURFACE`], so a shorter
    /// line overwrites a longer one without the card being cleared first - what
    /// is filled here is only the tail no glyph covers.
    fn draw_card_content(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let (category, title, body) = self.contents();

        draw_line(
            category,
            TEXT_X,
            CATEGORY_BASELINE_Y,
            hint_text(theme::ACCENT, theme::SURFACE),
            canvas,
        )?;
        keep_alive();

        draw_line(
            title,
            TEXT_X,
            TITLE_BASELINE_Y,
            ui_text(theme::TEXT, theme::SURFACE),
            canvas,
        )?;
        canvas.fill_solid(
            &Rectangle::new(
                Point::new(TEXT_X, RULE_Y),
                Size::new(u32::try_from(CONTENT_WIDTH).unwrap_or(0), 1),
            ),
            theme::FRAME,
        )?;
        keep_alive();

        let style = body_text(theme::TEXT, theme::SURFACE);
        let mut drawn = 0;
        for line in wrap(body, BODY_COLUMNS).take(BODY_LINES) {
            let y = BODY_FIRST_BASELINE_Y + BODY_LINE_STEP * i32::try_from(drawn).unwrap_or(0);
            draw_line(line, TEXT_X, y, style, canvas)?;
            drawn += 1;
            keep_alive();
        }
        // The rows a shorter message leaves standing from the one before it.
        // Only reached on a partial redraw; on a full one the card face below
        // has just covered them.
        for index in drawn..BODY_LINES {
            let y = BODY_FIRST_BASELINE_Y + BODY_LINE_STEP * i32::try_from(index).unwrap_or(0);
            draw_line("", TEXT_X, y, style, canvas)?;
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
                self.dirty = self.inbox.show_next() || self.dirty;
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
}

impl Paint for NotificationScreen {
    /// Opaque without a blanking pass.
    ///
    /// The margins around the card, the card itself and the band under it cover
    /// the panel between them, so nothing is painted twice. Clearing first cost
    /// a full 240x240 transfer before anything was drawn, and in a transition it
    /// filled every stripe a second time - the same thing both watchfaces were
    /// taught to stop doing.
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let panel_right = PANEL.size.width.cast_signed();
        let panel_bottom = PANEL.size.height.cast_signed();

        // Above the card, including the band the status corner will cover.
        fill(canvas, 0, 0, panel_right, CARD_TOP)?;
        keep_alive();

        // The margins either side, over the card's rows.
        let card_height = CARD_BOTTOM - CARD_TOP;
        fill(canvas, 0, CARD_TOP, CARD_X, card_height)?;
        let right_x = CARD_X + CARD_WIDTH;
        fill(
            canvas,
            right_x,
            CARD_TOP,
            panel_right - right_x,
            card_height,
        )?;
        // Draws its own background over that margin, and nothing at all when
        // one message - or none - leaves no position to report.
        draw_page_marks(canvas, PageAxis::Vertical, &self.pages(), RAIL_CENTER)?;
        keep_alive();

        Self::draw_card_face(canvas)?;
        keep_alive();
        self.draw_card_content(canvas, keep_alive)?;
        keep_alive();

        // Under the card, and the hint on it.
        fill(
            canvas,
            0,
            CARD_BOTTOM,
            panel_right,
            panel_bottom - CARD_BOTTOM,
        )?;
        let hint = if self.inbox.is_empty() {
            HINT_EMPTY
        } else {
            HINT_BROWSING
        };
        draw_line(
            hint,
            CARD_X,
            HINT_BASELINE_Y,
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
        )
    }
}

impl Screen for NotificationScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        match event {
            AppEvent::Swipe(direction) => self.swipe(direction),
            // Everything else - a tick, a reading, the inbox reporting itself -
            // leaves the mark alone. `file` has already set it where it matters.
            _ => ScreenAction::None,
        }
    }

    /// Repaints the message, and the rail if the position moved with it.
    ///
    /// The card face stays: it does not change between messages, so a page turn
    /// costs the text inside it rather than the shape around it. Neither does
    /// the hint, which is what keeps a page turn from flashing the foot.
    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if !self.dirty || self.inbox.is_empty() {
            return Ok(());
        }
        draw_page_marks(canvas, PageAxis::Vertical, &self.pages(), RAIL_CENTER)?;
        keep_alive();
        self.draw_card_content(canvas, keep_alive)
    }
}

/// Fills a rectangle with the screen's ground, skipping empty ones.
fn fill(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), CanvasError> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }
    Rectangle::new(
        Point::new(x, y),
        Size::new(
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
        ),
    )
    .into_styled(PrimitiveStyle::with_fill(theme::BACKGROUND))
    .draw(canvas)
}

/// One line of text, padded to the content width so it covers whatever the
/// previous message left on that row.
fn draw_line(
    text: &str,
    x: i32,
    baseline: i32,
    style: crate::font::AaTextStyle,
    canvas: &mut Canvas<'_>,
) -> Result<(), CanvasError> {
    draw_mono_text_visible(text, Point::new(x, baseline), style, canvas)?;
    let cell = style.font.cell.width.cast_signed();
    let drawn = i32::try_from(text.len()).unwrap_or(0) * cell;
    let tail = CONTENT_WIDTH - drawn;
    if tail <= 0 {
        return Ok(());
    }
    let top = baseline - style.font.baseline.cast_signed();
    Rectangle::new(
        Point::new(x + drawn, top),
        Size::new(u32::try_from(tail).unwrap_or(0), style.font.cell.height),
    )
    .into_styled(PrimitiveStyle::with_fill(style.background_color))
    .draw(canvas)
}

/// The first `columns` characters, cut on a character boundary.
fn truncate(text: &str, columns: usize) -> &str {
    match text.char_indices().nth(columns) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The card is the whole reason the layout changed, so its geometry has to
    /// stay inside the panel and clear of both the status corner and the hint.
    #[test]
    fn the_card_clears_the_corner_above_it_and_the_hint_below() {
        assert!(CARD_TOP >= STATUS_HEIGHT, "the card runs under the corner");
        assert!(
            CARD_BOTTOM < HINT_BASELINE_Y - JETBRAINS_MONO_10X22.baseline.cast_signed(),
            "the card runs into the hint"
        );
        assert!(
            CARD_X + CARD_WIDTH <= PANEL.size.width.cast_signed(),
            "the card runs off the panel"
        );
    }

    /// The rail lives in the margin beside the card, not over it: inside, it
    /// would take its width from every line of text for a mark that is absent
    /// whenever there is only one message.
    #[test]
    fn the_page_rail_sits_beside_the_card() {
        assert!(
            RAIL_CENTER.x > CARD_X + CARD_WIDTH,
            "the rail overlaps the card"
        );
        assert!(
            RAIL_CENTER.x < PANEL.size.width.cast_signed(),
            "the rail is off the panel"
        );
    }

    /// Every body line has to fit between the rule and the foot of the card.
    ///
    /// Measured at the glyph's own edges rather than its baseline, because the
    /// baseline sits well inside the cell - a line whose baseline clears the
    /// card can still have its descenders drawn over the edge.
    #[test]
    fn the_body_fits_between_the_rule_and_the_cards_foot() {
        let baseline = JETBRAINS_MONO_8X18.baseline.cast_signed();
        let height = JETBRAINS_MONO_8X18.cell.height.cast_signed();
        let last = BODY_FIRST_BASELINE_Y + BODY_LINE_STEP * (BODY_LINES as i32 - 1);
        let first_top = BODY_FIRST_BASELINE_Y - baseline;
        let last_bottom = last - baseline + height;
        assert!(first_top > RULE_Y, "the body starts over its own rule");
        assert!(
            last_bottom <= CARD_BOTTOM,
            "the last body line overruns the card"
        );
    }

    /// The leading is what makes a wrapped paragraph read as lines rather than
    /// as one block of texture, so it is a property and not an accident of the
    /// step happening to exceed the cell.
    #[test]
    fn body_lines_are_set_with_leading() {
        assert!(
            BODY_LINE_STEP > JETBRAINS_MONO_8X18.cell.height.cast_signed(),
            "body lines are set solid or overlapping"
        );
    }
}
