//! Wall-clock derivation from a BLE-synchronized reference time.
//!
//! The Current Time Service write anchors a calendar time to the uptime at
//! which it arrived; the display derives both the time of day and the date
//! from that anchor without any mutable global clock state.

const SECONDS_PER_DAY: u64 = 86_400;
const SECONDS_PER_HOUR: u64 = 3_600;
pub const CLOCK_RECORD_LEN: usize = 32;
pub const CLOCK_YEAR_MIN: u16 = 2_000;
pub const CLOCK_YEAR_MAX: u16 = 2_099;

const CLOCK_MAGIC: [u8; 4] = *b"PFCK";
const CLOCK_VERSION: u16 = 1;
const CLOCK_CRC_OFFSET: usize = 28;

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
    pub const MIDNIGHT: Self = Self {
        hour: 0,
        minute: 0,
        second: 0,
    };

    #[must_use]
    pub const fn new(hour: u8, minute: u8, second: u8) -> Option<Self> {
        if hour < 24 && minute < 60 && second < 60 {
            Some(Self {
                hour,
                minute,
                second,
            })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn total_seconds(self) -> u64 {
        self.hour as u64 * SECONDS_PER_HOUR + self.minute as u64 * 60 + self.second as u64
    }
}

/// Calendar date derived for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl CalendarDate {
    pub const DEFAULT: Self = Self {
        year: 2_026,
        month: 1,
        day: 1,
    };

    #[must_use]
    pub const fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        if month >= 1 && month <= 12 && day >= 1 && day <= days_in_month(year, month) {
            Some(Self { year, month, day })
        } else {
            None
        }
    }
}

#[must_use]
pub const fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

#[must_use]
pub const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        // Every other month, including a value no validated reference holds.
        _ => 31,
    }
}

/// Advances a date by whole days, rolling months and years.
///
/// Whole months are skipped at once so a watch that ran for a long time
/// without a synchronization does not walk day by day.
// Only a day count within the current month is ever narrowed to `u8`.
#[allow(clippy::cast_possible_truncation)]
const fn advance_days(date: CalendarDate, days: u64) -> CalendarDate {
    let mut date = date;
    let mut days = days;
    loop {
        // A reference may carry a day past the end of its month - the Current
        // Time write is only range-checked - so this cannot underflow.
        let until_month_end = days_in_month(date.year, date.month).saturating_sub(date.day) as u64;
        if days <= until_month_end {
            date.day += days as u8;
            return date;
        }
        days -= until_month_end + 1;
        date.day = 1;
        if date.month == 12 {
            date.month = 1;
            date.year += 1;
        } else {
            date.month += 1;
        }
    }
}

impl WallClockReference {
    /// Seconds from the start of the reference's calendar day to the given
    /// uptime. Both the date and the time of day derive from it.
    const fn seconds_from_day_start(&self, uptime_seconds: u64) -> u64 {
        let elapsed = uptime_seconds.saturating_sub(self.reference_uptime_seconds);
        let reference = WallTime {
            hour: self.hour,
            minute: self.minute,
            second: self.second,
        };
        reference.total_seconds() + elapsed
    }

    /// Returns the time of day at the given uptime, rolling over midnight.
    #[must_use]
    // The modulo arithmetic keeps every component well below its type limit.
    #[allow(clippy::cast_possible_truncation)]
    pub const fn wall_time_at(&self, uptime_seconds: u64) -> WallTime {
        let of_day = self.seconds_from_day_start(uptime_seconds) % SECONDS_PER_DAY;
        WallTime {
            hour: (of_day / SECONDS_PER_HOUR) as u8,
            minute: ((of_day / 60) % 60) as u8,
            second: (of_day % 60) as u8,
        }
    }

    /// Returns the calendar date at the given uptime, advancing the reference
    /// date by every midnight since the synchronization.
    #[must_use]
    pub const fn date_at(&self, uptime_seconds: u64) -> CalendarDate {
        let date = CalendarDate {
            year: self.year,
            month: self.month,
            day: self.day,
        };
        advance_days(
            date,
            self.seconds_from_day_start(uptime_seconds) / SECONDS_PER_DAY,
        )
    }
}

/// A complete calendar checkpoint independent of any particular boot's uptime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSnapshot {
    pub date: CalendarDate,
    pub time: WallTime,
}

impl ClockSnapshot {
    pub const DEFAULT: Self = Self {
        date: CalendarDate::DEFAULT,
        time: WallTime::MIDNIGHT,
    };

    #[must_use]
    pub const fn new(date: CalendarDate, time: WallTime) -> Self {
        Self { date, time }
    }

    #[must_use]
    pub const fn from_reference(reference: WallClockReference, uptime_seconds: u64) -> Self {
        Self {
            date: reference.date_at(uptime_seconds),
            time: reference.wall_time_at(uptime_seconds),
        }
    }

    #[must_use]
    pub const fn reference_at(self, uptime_seconds: u64) -> WallClockReference {
        WallClockReference {
            reference_uptime_seconds: uptime_seconds,
            year: self.date.year,
            month: self.date.month,
            day: self.date.day,
            hour: self.time.hour,
            minute: self.time.minute,
            second: self.time.second,
        }
    }

    #[must_use]
    pub fn encode(self, sequence: u32) -> [u8; CLOCK_RECORD_LEN] {
        let mut record = [0xff; CLOCK_RECORD_LEN];
        record[0..4].copy_from_slice(&CLOCK_MAGIC);
        record[4..6].copy_from_slice(&CLOCK_VERSION.to_le_bytes());
        record[8..12].copy_from_slice(&sequence.to_le_bytes());
        record[12..14].copy_from_slice(&self.date.year.to_le_bytes());
        record[14] = self.date.month;
        record[15] = self.date.day;
        record[16] = self.time.hour;
        record[17] = self.time.minute;
        record[18] = self.time.second;
        let crc = crate::crc32(&record[..CLOCK_CRC_OFFSET]);
        record[CLOCK_CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        record
    }

    #[must_use]
    pub fn decode(record: &[u8; CLOCK_RECORD_LEN]) -> Option<(Self, u32)> {
        if record[0..4] != CLOCK_MAGIC
            || u16::from_le_bytes([record[4], record[5]]) != CLOCK_VERSION
            || u32::from_le_bytes(record[CLOCK_CRC_OFFSET..].try_into().ok()?)
                != crate::crc32(&record[..CLOCK_CRC_OFFSET])
        {
            return None;
        }
        let date = CalendarDate::new(
            u16::from_le_bytes([record[12], record[13]]),
            record[14],
            record[15],
        )?;
        let time = WallTime::new(record[16], record[17], record[18])?;
        let sequence = u32::from_le_bytes(record[8..12].try_into().ok()?);
        Some((Self { date, time }, sequence))
    }
}

#[must_use]
pub const fn clock_sequence_is_newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeField {
    Hour,
    Minute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeEditor {
    value: WallTime,
    field: TimeField,
}

impl TimeEditor {
    #[must_use]
    pub const fn new(value: WallTime) -> Self {
        Self {
            value,
            field: TimeField::Hour,
        }
    }

    #[must_use]
    pub const fn value(self) -> WallTime {
        self.value
    }

    #[must_use]
    pub const fn field(self) -> TimeField {
        self.field
    }

    pub const fn next(&mut self) {
        self.field = match self.field {
            TimeField::Hour => TimeField::Minute,
            TimeField::Minute => TimeField::Hour,
        };
    }

    pub const fn adjust(&mut self, increase: bool) {
        match self.field {
            TimeField::Hour => {
                self.value.hour = step_wrapped(self.value.hour, 23, increase);
            }
            TimeField::Minute => {
                self.value.minute = step_wrapped(self.value.minute, 59, increase);
            }
        }
        self.value.second = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateField {
    Year,
    Month,
    Day,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateEditor {
    value: CalendarDate,
    field: DateField,
}

impl DateEditor {
    #[must_use]
    pub const fn new(value: CalendarDate) -> Self {
        let year = if value.year < CLOCK_YEAR_MIN || value.year > CLOCK_YEAR_MAX {
            CalendarDate::DEFAULT.year
        } else {
            value.year
        };
        let month = if value.month >= 1 && value.month <= 12 {
            value.month
        } else {
            1
        };
        let last_day = days_in_month(year, month);
        let day = if value.day < 1 {
            1
        } else if value.day > last_day {
            last_day
        } else {
            value.day
        };
        Self {
            value: CalendarDate { year, month, day },
            field: DateField::Year,
        }
    }

    #[must_use]
    pub const fn value(self) -> CalendarDate {
        self.value
    }

    #[must_use]
    pub const fn field(self) -> DateField {
        self.field
    }

    pub const fn next(&mut self) {
        self.field = match self.field {
            DateField::Year => DateField::Month,
            DateField::Month => DateField::Day,
            DateField::Day => DateField::Year,
        };
    }

    pub const fn adjust(&mut self, increase: bool) {
        match self.field {
            DateField::Year => {
                self.value.year = if increase {
                    if self.value.year == CLOCK_YEAR_MAX {
                        CLOCK_YEAR_MIN
                    } else {
                        self.value.year + 1
                    }
                } else if self.value.year == CLOCK_YEAR_MIN {
                    CLOCK_YEAR_MAX
                } else {
                    self.value.year - 1
                };
            }
            DateField::Month => {
                self.value.month = step_one_based(self.value.month, 12, increase);
            }
            DateField::Day => {
                self.value.day = step_one_based(
                    self.value.day,
                    days_in_month(self.value.year, self.value.month),
                    increase,
                );
                return;
            }
        }
        let last_day = days_in_month(self.value.year, self.value.month);
        if self.value.day > last_day {
            self.value.day = last_day;
        }
    }
}

const fn step_wrapped(value: u8, maximum: u8, increase: bool) -> u8 {
    if increase {
        if value == maximum { 0 } else { value + 1 }
    } else if value == 0 {
        maximum
    } else {
        value - 1
    }
}

const fn step_one_based(value: u8, maximum: u8, increase: bool) -> u8 {
    if increase {
        if value == maximum { 1 } else { value + 1 }
    } else if value == 1 {
        maximum
    } else {
        value - 1
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
    let valid = CalendarDate::new(reference.year, reference.month, reference.day).is_some()
        && WallTime::new(reference.hour, reference.minute, reference.second).is_some();
    valid.then_some(reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn date(year: u16, month: u8, day: u8) -> CalendarDate {
        CalendarDate { year, month, day }
    }

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
    fn the_date_advances_with_every_midnight() {
        // 2026-07-23 18:30:15
        let reference = parse_cts(&[0xea, 0x07, 7, 23, 18, 30, 15, 4, 0, 0], 1_000).unwrap();
        assert_eq!(reference.date_at(1_000), date(2026, 7, 23));
        // Still the same day one second before midnight.
        assert_eq!(reference.date_at(1_000 + 19_784), date(2026, 7, 23));
        assert_eq!(reference.date_at(1_000 + 19_785), date(2026, 7, 24));
        assert_eq!(
            reference.date_at(1_000 + 19_785 + 86_400),
            date(2026, 7, 25)
        );
    }

    #[test]
    fn the_date_rolls_over_months_and_years() {
        // 2026-12-31 23:00:00, one hour before the new year.
        let reference = parse_cts(&[0xea, 0x07, 12, 31, 23, 0, 0, 4, 0, 0], 0).unwrap();
        assert_eq!(reference.date_at(3_599), date(2026, 12, 31));
        assert_eq!(reference.date_at(3_600), date(2027, 1, 1));
        assert_eq!(
            reference.date_at(3_600 + 31 * SECONDS_PER_DAY),
            date(2027, 2, 1)
        );
    }

    #[test]
    fn february_lengths_follow_the_leap_year_rule() {
        // 2028-02-28, a leap year: the next day is the 29th.
        let leap = parse_cts(&[0xec, 0x07, 2, 28, 12, 0, 0, 4, 0, 0], 0).unwrap();
        assert_eq!(leap.date_at(SECONDS_PER_DAY), date(2028, 2, 29));
        assert_eq!(leap.date_at(2 * SECONDS_PER_DAY), date(2028, 3, 1));

        // 2026-02-28, a common year: the next day is March.
        let common = parse_cts(&[0xea, 0x07, 2, 28, 12, 0, 0, 4, 0, 0], 0).unwrap();
        assert_eq!(common.date_at(SECONDS_PER_DAY), date(2026, 3, 1));

        // 2100 is divisible by 4 but not a leap year.
        let century = parse_cts(&[0x34, 0x08, 2, 28, 12, 0, 0, 4, 0, 0], 0).unwrap();
        assert_eq!(century.date_at(SECONDS_PER_DAY), date(2100, 3, 1));

        // 2000 was, being divisible by 400.
        let quad_century = parse_cts(&[0xd0, 0x07, 2, 28, 12, 0, 0, 4, 0, 0], 0).unwrap();
        assert_eq!(quad_century.date_at(SECONDS_PER_DAY), date(2000, 2, 29));
    }

    #[test]
    fn a_long_run_without_synchronization_still_lands_on_the_right_date() {
        let reference = parse_cts(&[0xea, 0x07, 1, 1, 0, 0, 0, 4, 0, 0], 0).unwrap();
        // 2026 is a common year, so a full year lands back on January 1st.
        assert_eq!(reference.date_at(365 * SECONDS_PER_DAY), date(2027, 1, 1));
        // Across 2028's leap day, four years take 1_461 days.
        assert_eq!(reference.date_at(1_461 * SECONDS_PER_DAY), date(2030, 1, 1));
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

    #[test]
    fn clock_records_round_trip_and_reject_corruption() {
        let snapshot = ClockSnapshot::new(
            CalendarDate::new(2028, 2, 29).unwrap(),
            WallTime::new(23, 58, 59).unwrap(),
        );
        let mut record = snapshot.encode(42);
        assert_eq!(ClockSnapshot::decode(&record), Some((snapshot, 42)));
        record[17] ^= 1;
        assert_eq!(ClockSnapshot::decode(&record), None);
    }

    #[test]
    fn clock_record_sequences_wrap_without_mistaking_old_for_new() {
        assert!(clock_sequence_is_newer(8, 7));
        assert!(clock_sequence_is_newer(0, u32::MAX));
        assert!(!clock_sequence_is_newer(7, 8));
        assert!(!clock_sequence_is_newer(7, 7));
    }

    #[test]
    fn time_editor_wraps_and_clears_seconds() {
        let mut editor = TimeEditor::new(WallTime::new(23, 59, 45).unwrap());
        editor.adjust(true);
        assert_eq!(editor.value(), WallTime::new(0, 59, 0).unwrap());
        editor.next();
        editor.adjust(true);
        assert_eq!(editor.value(), WallTime::MIDNIGHT);
        editor.adjust(false);
        assert_eq!(editor.value(), WallTime::new(0, 59, 0).unwrap());
    }

    #[test]
    fn date_editor_clamps_short_months_and_wraps_each_field() {
        let mut editor = DateEditor::new(CalendarDate::new(2028, 1, 31).unwrap());
        editor.next();
        editor.adjust(true);
        assert_eq!(editor.value(), CalendarDate::new(2028, 2, 29).unwrap());
        editor.next();
        editor.adjust(true);
        assert_eq!(editor.value(), CalendarDate::new(2028, 2, 1).unwrap());

        let mut year = DateEditor::new(CalendarDate::new(CLOCK_YEAR_MAX, 12, 1).unwrap());
        year.adjust(true);
        assert_eq!(year.value().year, CLOCK_YEAR_MIN);
    }

    #[test]
    fn invalid_calendar_dates_are_rejected_at_the_boundary() {
        assert_eq!(CalendarDate::new(2026, 2, 29), None);
        assert!(CalendarDate::new(2028, 2, 29).is_some());
        assert_eq!(CalendarDate::new(2028, 4, 31), None);
    }
}
