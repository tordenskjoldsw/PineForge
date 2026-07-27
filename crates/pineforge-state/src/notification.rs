//! Alert Notification Service parsing, as Gadgetbridge speaks it to `InfiniTime`.
//!
//! Gadgetbridge writes phone notifications to the standard New Alert
//! characteristic (`0x2A46`) of the Alert Notification Service (`0x1811`). The
//! payload is a three-byte header (category, count, reserved) followed by the
//! text; `InfiniTime` splits that text into a title and a body on the first NUL,
//! which this parser mirrors. This is pure protocol logic: the BLE task feeds
//! it a raw write and surfaces the result to the UI and the vibration motor.
//!
//! Beside the parser sits everything the notification screen needs that is not
//! drawing: which notifications are pending ([`NotificationInbox`]), which one
//! is showing, what browsing and dismissing do to that, and how a body is
//! broken into lines ([`wrap`]). None of it needs a display to be wrong, so
//! none of it needs a display to be tested.

use heapless::{String, Vec};

/// Fixed New Alert header: category id, alert count, and a reserved byte. The
/// text begins immediately after it, matching `InfiniTime`'s `headerSize = 3`.
const HEADER_LEN: usize = 3;

/// Longest title kept (sender or app name); Gadgetbridge titles are short.
pub const NOTIFICATION_TITLE_MAX: usize = 40;
/// Longest body kept, matching `InfiniTime`'s 100-character message buffer.
pub const NOTIFICATION_BODY_MAX: usize = 100;

/// New Alert category ids from the BLE Alert Notification Service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationCategory {
    SimpleAlert,
    Email,
    News,
    Call,
    MissedCall,
    Sms,
    VoiceMail,
    Schedule,
    HighPriority,
    InstantMessage,
    /// Any id outside the standard range, kept so nothing is silently dropped.
    Other(u8),
}

impl NotificationCategory {
    #[must_use]
    pub const fn from_id(id: u8) -> Self {
        match id {
            0x00 => Self::SimpleAlert,
            0x01 => Self::Email,
            0x02 => Self::News,
            0x03 => Self::Call,
            0x04 => Self::MissedCall,
            0x05 => Self::Sms,
            0x06 => Self::VoiceMail,
            0x07 => Self::Schedule,
            0x08 => Self::HighPriority,
            0x09 => Self::InstantMessage,
            other => Self::Other(other),
        }
    }

    /// Whether this alert is a call, which the UI may want to treat specially.
    #[must_use]
    pub const fn is_call(self) -> bool {
        matches!(self, Self::Call | Self::MissedCall)
    }

    /// The short name a screen puts on this category.
    ///
    /// Here rather than beside the one screen that draws it, because naming a
    /// category is not a drawing decision: the same word has to be right on a
    /// notification screen, in a log line, and on any face that ever shows one.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Call => "CALL",
            Self::MissedCall => "MISSED",
            Self::Sms => "SMS",
            Self::Email => "EMAIL",
            Self::InstantMessage => "IM",
            Self::News => "NEWS",
            Self::VoiceMail => "VMAIL",
            Self::Schedule => "CAL",
            Self::HighPriority => "ALERT",
            Self::SimpleAlert | Self::Other(_) => "MSG",
        }
    }
}

/// A parsed phone notification: a category plus text split into an optional
/// title and a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub category: NotificationCategory,
    pub title: String<NOTIFICATION_TITLE_MAX>,
    pub body: String<NOTIFICATION_BODY_MAX>,
}

/// How many notifications the watch keeps pending at once.
///
/// Four, because each one costs its two text buffers in static RAM and the
/// nRF52832 has 64 KiB of it for the whole firmware, BLE stack included. A
/// watch is for what just arrived; the phone keeps the archive.
pub const NOTIFICATION_CAPACITY: usize = 4;

/// What the rest of the firmware needs to know about the inbox without reading
/// it: how many are pending and what the newest one is.
///
/// A watchface shows exactly this, which is why it travels as one value rather
/// than as a borrow of the inbox - the face never holds the notifications, and
/// so can never disagree with them about how many there are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotificationSummary {
    pub count: u32,
    pub latest: Option<NotificationCategory>,
}

/// The notifications that have arrived and not yet been dismissed, newest
/// first, together with the one currently being read.
///
/// The cursor lives here rather than in the screen for the same reason
/// [`crate::PagedList`] does: every rule about it - what a new arrival does to
/// it, where it lands after a dismissal, that it never points past the end - is
/// a rule that can be got wrong silently, and none of them need a display.
pub struct NotificationInbox {
    /// Newest at index 0. A full inbox drops the oldest, which is the one the
    /// user is least likely to still care about.
    entries: Vec<Notification, NOTIFICATION_CAPACITY>,
    /// Index of the notification being read. Always in range while the inbox
    /// is non-empty; meaningless and ignored while it is empty.
    showing: usize,
}

impl Default for NotificationInbox {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationInbox {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            showing: 0,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The count and newest category, for whoever shows a tally rather than the
    /// messages themselves.
    #[must_use]
    pub fn summary(&self) -> NotificationSummary {
        NotificationSummary {
            count: u32::try_from(self.entries.len()).unwrap_or(u32::MAX),
            latest: self.entries.first().map(|entry| entry.category),
        }
    }

    /// The notification being read, or `None` when nothing is pending.
    #[must_use]
    pub fn showing(&self) -> Option<&Notification> {
        self.entries.get(self.showing)
    }

    /// Which one is showing, counted from one, for a `2/4` style position.
    #[must_use]
    pub fn position(&self) -> usize {
        if self.entries.is_empty() {
            0
        } else {
            self.showing + 1
        }
    }

    /// Files a new notification at the front and makes it the one showing.
    ///
    /// Showing it is the point: an alert the user did not ask to see is worth
    /// nothing if reaching it takes a gesture. A full inbox loses its oldest
    /// entry rather than refusing the new one, so the pending list can never
    /// stall on something read weeks ago.
    pub fn file(&mut self, notification: Notification) -> NotificationSummary {
        if self.entries.is_full() {
            self.entries.pop();
        }
        // The only error is a full vector, and it cannot be full here.
        let _ = self.entries.insert(0, notification);
        self.showing = 0;
        self.summary()
    }

    /// Moves to the next notification, older first, wrapping at the end.
    ///
    /// Wrapping rather than stopping because this is the only gesture that
    /// browses: the way out of the screen is spoken for by the navigation
    /// contract, so a cursor that stopped at the oldest would make the newer
    /// ones reachable only by leaving and coming back.
    ///
    /// Returns whether the showing notification changed, which is false for an
    /// inbox holding fewer than two.
    pub fn show_next(&mut self) -> bool {
        if self.entries.len() < 2 {
            return false;
        }
        self.showing = (self.showing + 1) % self.entries.len();
        true
    }

    /// Removes the notification being read, reporting whether there was one.
    ///
    /// The next one down takes its place, so repeated dismissals clear the
    /// inbox from where the user is standing. Removing the last entry leaves
    /// the cursor on the new last one rather than past the end.
    pub fn dismiss_showing(&mut self) -> bool {
        if self.showing >= self.entries.len() {
            return false;
        }
        self.entries.remove(self.showing);
        if self.showing >= self.entries.len() {
            self.showing = self.entries.len().saturating_sub(1);
        }
        true
    }
}

/// Breaks text into lines of at most `columns` characters.
///
/// Written here rather than pulled in with a text-layout crate because the font
/// is monospace: a line's width is its character count, so wrapping is this
/// iterator and nothing else. Breaks at a space when there is one within the
/// width, splits mid-word only when a single word is wider than the line, and
/// ends a line at an embedded newline - Gadgetbridge sends those.
#[must_use]
pub const fn wrap(text: &str, columns: usize) -> Wrapped<'_> {
    Wrapped {
        rest: text,
        // A zero-column line could never make progress, so it is refused the
        // same way a zero-slot page is.
        columns: if columns == 0 { 1 } else { columns },
    }
}

/// The lines of a wrapped text; see [`wrap`].
pub struct Wrapped<'a> {
    rest: &'a str,
    columns: usize,
}

impl<'a> Iterator for Wrapped<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        // Blanks a break consumed, that the sender doubled up, or that separate
        // paragraphs. Skipping them here is what keeps them from indenting the
        // line they start or becoming an empty line of their own - eight lines
        // fit on this screen, and none of them can be spent on nothing.
        let rest = self.rest.trim_start_matches([' ', '\n']);
        if rest.is_empty() {
            self.rest = "";
            return None;
        }

        // Byte index one past the last character that fits, the last space
        // within that span, and an explicit newline if one comes first.
        let mut cut = None;
        let mut last_space = None;
        let mut newline = None;
        for (taken, (index, character)) in rest.char_indices().enumerate() {
            if character == '\n' {
                newline = Some(index);
                break;
            }
            if taken == self.columns {
                cut = Some(index);
                break;
            }
            if character == ' ' {
                last_space = Some(index);
            }
        }

        let (line, remainder) = match (newline, cut, last_space) {
            // The newline is the break and belongs to neither line.
            (Some(index), _, _) => (&rest[..index], &rest[index + 1..]),
            // Everything left fits on one line.
            (None, None, _) => (rest, ""),
            // Break at the space, which neither line carries.
            (None, Some(_), Some(space)) => (&rest[..space], &rest[space + 1..]),
            // A word longer than the line: split it rather than loop forever.
            (None, Some(cut), None) => (&rest[..cut], &rest[cut..]),
        };
        self.rest = remainder;
        // A run of spaces ending a line would otherwise be drawn as background
        // cells, which matters for a right-aligned or centred line later on.
        Some(line.trim_end_matches(' '))
    }
}

/// Parses a New Alert characteristic write into a notification.
///
/// Returns `None` only when the write is shorter than the fixed header. The
/// text after the header is split on the first NUL into title and body, as
/// Gadgetbridge frames it; a message without a NUL is taken as body-only so a
/// plain alert still carries its content. Non-UTF-8 tails and text past the
/// buffer bounds are truncated rather than rejected.
#[must_use]
pub fn parse_new_alert(payload: &[u8]) -> Option<Notification> {
    if payload.len() < HEADER_LEN {
        return None;
    }
    let category = NotificationCategory::from_id(payload[0]);
    let text = &payload[HEADER_LEN..];
    let (title, body) = text
        .iter()
        .position(|&byte| byte == 0)
        .map_or_else(|| (&[][..], text), |nul| (&text[..nul], &text[nul + 1..]));
    Some(Notification {
        category,
        title: sanitize(title),
        body: sanitize(body),
    })
}

/// Copies the valid UTF-8 prefix of `bytes` into a bounded string, dropping
/// embedded NULs and stopping at the buffer's capacity.
fn sanitize<const N: usize>(bytes: &[u8]) -> String<N> {
    let valid = match core::str::from_utf8(bytes) {
        Ok(text) => text,
        // Everything up to `valid_up_to` is guaranteed well-formed UTF-8.
        Err(error) => core::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or(""),
    };
    let mut out = String::new();
    for ch in valid.chars() {
        if ch == '\0' {
            continue;
        }
        if out.push(ch).is_err() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a New Alert payload: category, count, reserved, then the text.
    fn new_alert(category: u8, count: u8, text: &[u8]) -> heapless::Vec<u8, 256> {
        let mut payload = heapless::Vec::new();
        payload.extend_from_slice(&[category, count, 0]).unwrap();
        payload.extend_from_slice(text).unwrap();
        payload
    }

    #[test]
    fn parses_category_title_and_body() {
        let notif = parse_new_alert(&new_alert(0x05, 1, b"Alice\0See you at 8")).unwrap();
        assert_eq!(notif.category, NotificationCategory::Sms);
        assert_eq!(notif.title.as_str(), "Alice");
        assert_eq!(notif.body.as_str(), "See you at 8");
    }

    #[test]
    fn maps_every_standard_category() {
        for (id, expected) in [
            (0x00, NotificationCategory::SimpleAlert),
            (0x01, NotificationCategory::Email),
            (0x02, NotificationCategory::News),
            (0x03, NotificationCategory::Call),
            (0x04, NotificationCategory::MissedCall),
            (0x05, NotificationCategory::Sms),
            (0x06, NotificationCategory::VoiceMail),
            (0x07, NotificationCategory::Schedule),
            (0x08, NotificationCategory::HighPriority),
            (0x09, NotificationCategory::InstantMessage),
        ] {
            assert_eq!(NotificationCategory::from_id(id), expected);
        }
        assert_eq!(
            NotificationCategory::from_id(0x42),
            NotificationCategory::Other(0x42)
        );
        assert!(NotificationCategory::MissedCall.is_call());
        assert!(!NotificationCategory::Email.is_call());
    }

    #[test]
    fn text_without_a_separator_is_body_only() {
        let notif = parse_new_alert(&new_alert(0x00, 1, b"Low battery")).unwrap();
        assert!(notif.title.is_empty());
        assert_eq!(notif.body.as_str(), "Low battery");
    }

    #[test]
    fn a_header_only_write_is_an_empty_notification() {
        let notif = parse_new_alert(&[0x03, 0, 0]).unwrap();
        assert_eq!(notif.category, NotificationCategory::Call);
        assert!(notif.title.is_empty());
        assert!(notif.body.is_empty());
    }

    #[test]
    fn writes_shorter_than_the_header_are_rejected() {
        assert_eq!(parse_new_alert(&[]), None);
        assert_eq!(parse_new_alert(&[0x00, 0x01]), None);
    }

    #[test]
    fn an_overlong_body_is_truncated_to_capacity() {
        let long = [b'x'; 200];
        let notif = parse_new_alert(&new_alert(0x01, 1, &long)).unwrap();
        assert_eq!(notif.body.len(), NOTIFICATION_BODY_MAX);
    }

    #[test]
    fn an_invalid_utf8_tail_is_dropped_without_losing_the_prefix() {
        // Valid "Hi" then a lone continuation byte that starts no code point.
        let notif = parse_new_alert(&new_alert(0x00, 1, b"\0Hi\xff\xfe")).unwrap();
        assert_eq!(notif.body.as_str(), "Hi");
    }

    /// A notification whose title says which one it is.
    fn from(sender: &str) -> Notification {
        Notification {
            category: NotificationCategory::Sms,
            title: String::try_from(sender).unwrap(),
            body: String::new(),
        }
    }

    /// The titles of the inbox in order, newest first.
    fn titles(inbox: &NotificationInbox) -> heapless::Vec<&str, NOTIFICATION_CAPACITY> {
        inbox
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect()
    }

    #[test]
    fn the_newest_arrival_is_the_one_being_read() {
        let mut inbox = NotificationInbox::new();
        assert!(inbox.is_empty());
        assert_eq!(inbox.showing(), None);
        assert_eq!(inbox.position(), 0);

        let summary = inbox.file(from("Alice"));
        assert_eq!(
            summary,
            NotificationSummary {
                count: 1,
                latest: Some(NotificationCategory::Sms),
            }
        );

        // Reading an older one and then getting a new alert must not leave the
        // user staring at the old one: the arrival is the whole point.
        inbox.file(from("Bob"));
        assert_eq!(titles(&inbox).as_slice(), ["Bob", "Alice"]);
        assert_eq!(inbox.showing().unwrap().title.as_str(), "Bob");
        assert_eq!(inbox.position(), 1);
    }

    #[test]
    fn a_full_inbox_loses_its_oldest_rather_than_the_arrival() {
        let mut inbox = NotificationInbox::new();
        for index in 0..NOTIFICATION_CAPACITY {
            let mut title = String::<NOTIFICATION_TITLE_MAX>::new();
            title.push(char::from(b'A' + index as u8)).unwrap();
            inbox.file(Notification {
                category: NotificationCategory::Sms,
                title,
                body: String::new(),
            });
        }
        assert_eq!(titles(&inbox).as_slice(), ["D", "C", "B", "A"]);

        inbox.file(from("E"));
        assert_eq!(inbox.len(), NOTIFICATION_CAPACITY);
        assert_eq!(titles(&inbox).as_slice(), ["E", "D", "C", "B"]);
    }

    #[test]
    fn browsing_wraps_so_every_notification_is_reachable() {
        let mut inbox = NotificationInbox::new();
        // A single notification has nowhere to go, and must not pretend it moved.
        inbox.file(from("Alice"));
        assert!(!inbox.show_next());
        assert_eq!(inbox.position(), 1);

        inbox.file(from("Bob"));
        inbox.file(from("Cleo"));
        for expected in ["Bob", "Alice", "Cleo", "Bob"] {
            assert!(inbox.show_next());
            assert_eq!(inbox.showing().unwrap().title.as_str(), expected);
        }
    }

    #[test]
    fn dismissing_moves_the_next_one_under_the_finger() {
        let mut inbox = NotificationInbox::new();
        inbox.file(from("Alice"));
        inbox.file(from("Bob"));
        inbox.file(from("Cleo"));

        assert!(inbox.dismiss_showing());
        assert_eq!(inbox.showing().unwrap().title.as_str(), "Bob");
        assert_eq!(inbox.position(), 1);
        assert_eq!(inbox.len(), 2);
    }

    /// The cursor pointing past the end is the failure this guards: the entry
    /// it names would be drawn, or dismissed, and neither exists.
    #[test]
    fn dismissing_the_oldest_leaves_the_cursor_on_one_that_exists() {
        let mut inbox = NotificationInbox::new();
        inbox.file(from("Alice"));
        inbox.file(from("Bob"));
        assert!(inbox.show_next());
        assert_eq!(inbox.showing().unwrap().title.as_str(), "Alice");

        assert!(inbox.dismiss_showing());
        assert_eq!(inbox.showing().unwrap().title.as_str(), "Bob");
        assert_eq!(inbox.position(), 1);
    }

    #[test]
    fn dismissing_the_last_one_empties_the_inbox_without_a_dangling_cursor() {
        let mut inbox = NotificationInbox::new();
        inbox.file(from("Alice"));

        assert!(inbox.dismiss_showing());
        assert!(inbox.is_empty());
        assert_eq!(inbox.showing(), None);
        assert_eq!(inbox.position(), 0);
        assert_eq!(inbox.summary(), NotificationSummary::default());
        // Nothing to dismiss reports so rather than panicking on the index.
        assert!(!inbox.dismiss_showing());
        assert!(!inbox.show_next());
    }

    /// Collects the wrapped lines, so the expectations below read as the screen.
    fn lines(text: &str, columns: usize) -> heapless::Vec<&str, 8> {
        wrap(text, columns).collect()
    }

    #[test]
    fn a_line_breaks_at_the_last_space_that_fits() {
        assert_eq!(
            lines("the quick brown fox jumps", 10).as_slice(),
            ["the quick", "brown fox", "jumps"]
        );
        // Exactly the width is not too wide.
        assert_eq!(lines("abcde fghij", 5).as_slice(), ["abcde", "fghij"]);
    }

    #[test]
    fn a_word_wider_than_the_line_is_split_rather_than_looped_on() {
        assert_eq!(
            lines("supercalifragilistic ok", 8).as_slice(),
            ["supercal", "ifragili", "stic ok"]
        );
    }

    #[test]
    fn an_embedded_newline_ends_its_line_where_it_stands() {
        assert_eq!(
            lines("Alice\nSee you at 8", 20).as_slice(),
            ["Alice", "See you at 8"]
        );
        // A blank line carries nothing, so it is not a line at all.
        assert_eq!(lines("a\n\nb", 20).as_slice(), ["a", "b"]);
    }

    #[test]
    fn runs_of_spaces_do_not_indent_or_empty_a_line() {
        assert_eq!(
            lines("  padded   text  ", 10).as_slice(),
            ["padded", "text"]
        );
        assert_eq!(lines("", 10).as_slice(), [] as [&str; 0]);
        assert_eq!(lines("     ", 10).as_slice(), [] as [&str; 0]);
    }

    #[test]
    fn a_zero_width_line_is_refused_rather_than_never_advancing() {
        assert_eq!(lines("ab", 0).as_slice(), ["a", "b"]);
    }
}
