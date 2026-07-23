//! Wall-clock derivation from a BLE-synchronized reference time.
//!
//! The Current Time Service write anchors a calendar time to the uptime at
//! which it arrived; the display derives the time of day from that anchor
//! without any mutable global clock state.

const SECONDS_PER_DAY: u64 = 86_400;
const SECONDS_PER_HOUR: u64 = 3_600;

/// Minimum Current Time characteristic length: year through second.
const CTS_MIN_LENGTH: usize = 7;

/// A calendar time pinned to the uptime second at which it was received.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallClockReference {
    pub reference_uptime_seconds: u64,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Time of day derived for display purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl WallTime {
    #[must_use]
    pub const fn total_seconds(self) -> u64 {
        self.hour as u64 * SECONDS_PER_HOUR + self.minute as u64 * 60 + self.second as u64
    }
}

impl WallClockReference {
    /// Returns the time of day at the given uptime, rolling over midnight.
    ///
    /// The date fields are retained for future consumers but deliberately do
    /// not advance; only the time of day is derived.
    #[must_use]
    // The modulo arithmetic keeps every component well below its type limit.
    #[allow(clippy::cast_possible_truncation)]
    pub const fn wall_time_at(&self, uptime_seconds: u64) -> WallTime {
        let elapsed = uptime_seconds.saturating_sub(self.reference_uptime_seconds);
        let reference = WallTime {
            hour: self.hour,
            minute: self.minute,
            second: self.second,
        };
        let of_day = (reference.total_seconds() + elapsed) % SECONDS_PER_DAY;
        WallTime {
            hour: (of_day / SECONDS_PER_HOUR) as u8,
            minute: ((of_day / 60) % 60) as u8,
            second: (of_day % 60) as u8,
        }
    }
}

/// Parses a GATT Current Time characteristic write (year through second).
///
/// Trailing day-of-week, fractions, and adjust-reason bytes are accepted and
/// ignored. Returns `None` for short writes or out-of-range fields.
#[must_use]
pub fn parse_cts(data: &[u8], reference_uptime_seconds: u64) -> Option<WallClockReference> {
    if data.len() < CTS_MIN_LENGTH {
        return None;
    }
    let reference = WallClockReference {
        reference_uptime_seconds,
        year: u16::from_le_bytes([data[0], data[1]]),
        month: data[2],
        day: data[3],
        hour: data[4],
        minute: data[5],
        second: data[6],
    };
    let valid = (1..=12).contains(&reference.month)
        && (1..=31).contains(&reference.day)
        && reference.hour < 24
        && reference.minute < 60
        && reference.second < 60;
    valid.then_some(reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_time_advances_with_uptime() {
        let reference = parse_cts(
            &[0xea, 0x07, 7, 23, 18, 30, 15, 4, 0, 0], // 2026-07-23 18:30:15
            1_000,
        )
        .unwrap();
        assert_eq!(reference.year, 2026);
        assert_eq!(
            reference.wall_time_at(1_000),
            WallTime {
                hour: 18,
                minute: 30,
                second: 15
            }
        );
        assert_eq!(
            reference.wall_time_at(1_000 + 3_661),
            WallTime {
                hour: 19,
                minute: 31,
                second: 16
            }
        );
    }

    #[test]
    fn wall_time_rolls_over_midnight() {
        let reference = parse_cts(&[0xea, 0x07, 1, 1, 23, 59, 50, 4, 0, 0], 0).unwrap();
        assert_eq!(
            reference.wall_time_at(10),
            WallTime {
                hour: 0,
                minute: 0,
                second: 0
            }
        );
        assert_eq!(
            reference.wall_time_at(10 + 2 * 86_400),
            WallTime {
                hour: 0,
                minute: 0,
                second: 0
            }
        );
    }

    #[test]
    fn a_new_reference_resynchronizes_the_derived_time() {
        let stale = parse_cts(&[0xea, 0x07, 1, 1, 10, 0, 0, 4, 0, 0], 100).unwrap();
        let fresh = parse_cts(&[0xea, 0x07, 1, 1, 12, 0, 0, 4, 0, 0], 200).unwrap();
        assert_eq!(stale.wall_time_at(200).hour, 10);
        assert_eq!(fresh.wall_time_at(200).hour, 12);
    }

    #[test]
    fn parse_rejects_short_and_invalid_writes() {
        assert_eq!(parse_cts(&[0xea, 0x07, 1, 1, 10, 0], 0), None);
        assert_eq!(parse_cts(&[0xea, 0x07, 0, 1, 10, 0, 0, 4, 0, 0], 0), None);
        assert_eq!(parse_cts(&[0xea, 0x07, 1, 32, 10, 0, 0, 4, 0, 0], 0), None);
        assert_eq!(parse_cts(&[0xea, 0x07, 1, 1, 24, 0, 0, 4, 0, 0], 0), None);
        assert_eq!(parse_cts(&[0xea, 0x07, 1, 1, 10, 60, 0, 4, 0, 0], 0), None);
        assert_eq!(parse_cts(&[0xea, 0x07, 1, 1, 10, 0, 60, 4, 0, 0], 0), None);
    }

    #[test]
    fn seven_byte_writes_without_trailer_are_accepted() {
        assert!(parse_cts(&[0xea, 0x07, 6, 15, 8, 5, 30], 0).is_some());
    }
}
