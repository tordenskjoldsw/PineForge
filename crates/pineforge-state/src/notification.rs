//! Alert Notification Service parsing, as Gadgetbridge speaks it to `InfiniTime`.
//!
//! Gadgetbridge writes phone notifications to the standard New Alert
//! characteristic (`0x2A46`) of the Alert Notification Service (`0x1811`). The
//! payload is a three-byte header (category, count, reserved) followed by the
//! text; `InfiniTime` splits that text into a title and a body on the first NUL,
//! which this parser mirrors. This is pure protocol logic: the BLE task feeds
//! it a raw write and surfaces the result to the UI and the vibration motor.

use heapless::String;

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
}

/// A parsed phone notification: a category plus text split into an optional
/// title and a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub category: NotificationCategory,
    pub title: String<NOTIFICATION_TITLE_MAX>,
    pub body: String<NOTIFICATION_BODY_MAX>,
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
}
