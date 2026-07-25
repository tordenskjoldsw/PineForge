//! Versioned, checksummed persistence for user-facing display settings.
//!
//! The on-flash record is fixed-size and written alternately into two 4 KiB
//! sectors. Boot decodes both slots and keeps the record with the newer
//! wrapping sequence number, so an interrupted write never loses the previous
//! configuration.

pub const SETTINGS_RECORD_LEN: usize = 32;

const SETTINGS_MAGIC: [u8; 4] = *b"PFST";
const SETTINGS_VERSION: u16 = 2;
const CRC_OFFSET: usize = 28;

/// The backlight FETs only produce two distinguishable active levels; the
/// dimmed level doubles as the idle-dimming stage.
pub const BRIGHTNESS_LEVELS: [u8; 2] = [1, 7];
pub const DIM_TIMEOUTS_MILLIS: [u32; 4] = [5_000, 10_000, 20_000, 30_000];
pub const OFF_TIMEOUTS_MILLIS: [u32; 4] = [10_000, 20_000, 30_000, 60_000];
pub const HEART_RATE_INTERVALS_SECONDS: [u32; 4] = [60, 300, 900, 1_800];

const MIN_BRIGHTNESS: u8 = 1;
const MAX_BRIGHTNESS: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsError {
    BrightnessOutOfRange,
    ZeroDimTimeout,
    OffNotAfterDim,
    InvalidHeartRateSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    BadChecksum,
    UnsupportedVersion(u16),
    InvalidContent(SettingsError),
}

/// User-adjustable display behavior, validated on construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    brightness: u8,
    dim_after_millis: u32,
    off_after_millis: u32,
    heart_rate_enabled: bool,
    heart_rate_interval_seconds: u32,
}

impl DisplaySettings {
    pub const DEFAULT: Self = Self {
        brightness: 7,
        dim_after_millis: 10_000,
        off_after_millis: 20_000,
        heart_rate_enabled: false,
        heart_rate_interval_seconds: 300,
    };

    pub const fn new(
        brightness: u8,
        dim_after_millis: u32,
        off_after_millis: u32,
    ) -> Result<Self, SettingsError> {
        if brightness < MIN_BRIGHTNESS || brightness > MAX_BRIGHTNESS {
            return Err(SettingsError::BrightnessOutOfRange);
        }
        if dim_after_millis == 0 {
            return Err(SettingsError::ZeroDimTimeout);
        }
        if off_after_millis <= dim_after_millis {
            return Err(SettingsError::OffNotAfterDim);
        }
        Ok(Self {
            brightness,
            dim_after_millis,
            off_after_millis,
            heart_rate_enabled: false,
            heart_rate_interval_seconds: 300,
        })
    }

    #[must_use]
    pub const fn brightness(self) -> u8 {
        self.brightness
    }

    #[must_use]
    pub const fn dim_after_millis(self) -> u32 {
        self.dim_after_millis
    }

    #[must_use]
    pub const fn off_after_millis(self) -> u32 {
        self.off_after_millis
    }

    #[must_use]
    pub const fn heart_rate_enabled(self) -> bool {
        self.heart_rate_enabled
    }

    #[must_use]
    pub const fn heart_rate_interval_seconds(self) -> u32 {
        self.heart_rate_interval_seconds
    }

    #[must_use]
    pub const fn toggle_heart_rate(self) -> Self {
        Self {
            heart_rate_enabled: !self.heart_rate_enabled,
            ..self
        }
    }

    #[must_use]
    pub fn cycle_heart_rate_interval(self) -> Self {
        Self {
            heart_rate_interval_seconds: next_preset(
                &HEART_RATE_INTERVALS_SECONDS,
                self.heart_rate_interval_seconds,
            ),
            ..self
        }
    }

    /// Converts to the inactivity policy configuration. The constructor
    /// invariants match `PowerConfig`'s, so the conversion is total.
    #[must_use]
    pub const fn power_config(self) -> crate::PowerConfig {
        match crate::PowerConfig::new(self.dim_after_millis as u64, self.off_after_millis as u64) {
            Ok(config) => config,
            Err(_) => crate::PowerConfig::DEFAULT,
        }
    }

    /// Advances brightness to the next preset level.
    #[must_use]
    pub fn cycle_brightness(self) -> Self {
        Self {
            brightness: next_preset(&BRIGHTNESS_LEVELS, self.brightness),
            ..self
        }
    }

    /// Advances the dim timeout to the next preset, raising the off timeout
    /// to the smallest preset that keeps it strictly later than dimming.
    #[must_use]
    pub fn cycle_dim_timeout(self) -> Self {
        let dim_after_millis = next_preset(&DIM_TIMEOUTS_MILLIS, self.dim_after_millis);
        let off_after_millis = if self.off_after_millis > dim_after_millis {
            self.off_after_millis
        } else {
            smallest_preset_above(&OFF_TIMEOUTS_MILLIS, dim_after_millis)
        };
        Self {
            dim_after_millis,
            off_after_millis,
            ..self
        }
    }

    /// Advances the off timeout to the next preset later than the dim timeout.
    #[must_use]
    pub fn cycle_off_timeout(self) -> Self {
        let mut off_after_millis = self.off_after_millis;
        for _ in 0..OFF_TIMEOUTS_MILLIS.len() {
            off_after_millis = next_preset(&OFF_TIMEOUTS_MILLIS, off_after_millis);
            if off_after_millis > self.dim_after_millis {
                break;
            }
        }
        Self {
            off_after_millis,
            ..self
        }
    }

    /// Serializes a version-2 record carrying the given sequence number.
    #[must_use]
    pub fn encode(self, sequence: u32) -> [u8; SETTINGS_RECORD_LEN] {
        let mut record = [0_u8; SETTINGS_RECORD_LEN];
        record[0..4].copy_from_slice(&SETTINGS_MAGIC);
        record[4..6].copy_from_slice(&SETTINGS_VERSION.to_le_bytes());
        record[6] = self.brightness;
        record[7] = u8::from(self.heart_rate_enabled);
        record[8..12].copy_from_slice(&self.dim_after_millis.to_le_bytes());
        record[12..16].copy_from_slice(&self.off_after_millis.to_le_bytes());
        record[16..20].copy_from_slice(&sequence.to_le_bytes());
        record[20..24].copy_from_slice(&self.heart_rate_interval_seconds.to_le_bytes());
        let crc = crc32(&record[..CRC_OFFSET]);
        record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        record
    }

    /// Deserializes a record, returning the settings and their sequence.
    pub fn decode(record: &[u8; SETTINGS_RECORD_LEN]) -> Result<(Self, u32), DecodeError> {
        if record[0..4] != SETTINGS_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        let stored_crc = u32::from_le_bytes([record[28], record[29], record[30], record[31]]);
        if crc32(&record[..CRC_OFFSET]) != stored_crc {
            return Err(DecodeError::BadChecksum);
        }
        let version = u16::from_le_bytes([record[4], record[5]]);
        if version != 1 && version != SETTINGS_VERSION {
            // Newer or unknown formats fall back to defaults at the caller.
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let dim = u32::from_le_bytes([record[8], record[9], record[10], record[11]]);
        let off = u32::from_le_bytes([record[12], record[13], record[14], record[15]]);
        let sequence = u32::from_le_bytes([record[16], record[17], record[18], record[19]]);
        let mut settings = Self::new(record[6], dim, off).map_err(DecodeError::InvalidContent)?;
        if version == SETTINGS_VERSION {
            let interval = u32::from_le_bytes([record[20], record[21], record[22], record[23]]);
            if record[7] > 1 || !HEART_RATE_INTERVALS_SECONDS.contains(&interval) {
                return Err(DecodeError::InvalidContent(
                    SettingsError::InvalidHeartRateSettings,
                ));
            }
            settings.heart_rate_enabled = record[7] != 0;
            settings.heart_rate_interval_seconds = interval;
        }
        Ok((settings, sequence))
    }
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn next_preset<T: Copy + PartialOrd>(presets: &[T], current: T) -> T {
    for preset in presets {
        if *preset > current {
            return *preset;
        }
    }
    presets[0]
}

fn smallest_preset_above(presets: &[u32], threshold: u32) -> u32 {
    for preset in presets {
        if *preset > threshold {
            return *preset;
        }
    }
    presets[presets.len() - 1]
}

/// Bitwise IEEE CRC-32, small enough to avoid a lookup table in flash.
#[must_use]
pub const fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    let mut index = 0;
    while index < data.len() {
        crc ^= data[index] as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            bit += 1;
        }
        index += 1;
    }
    !crc
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSlot {
    A,
    B,
}

impl SettingsSlot {
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// Boot-time decision derived from both decoded slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotDecision {
    /// The valid record with the newest sequence, if any slot decoded.
    pub current: Option<(DisplaySettings, u32)>,
    /// The slot the next write must target.
    pub write_slot: SettingsSlot,
    /// The sequence number the next write must carry.
    pub next_sequence: u32,
}

/// Chooses the authoritative slot using a wrapping sequence comparison, so
/// the scheme survives `u32` rollover after a lifetime of writes.
#[must_use]
pub const fn select_slot(
    slot_a: Option<(DisplaySettings, u32)>,
    slot_b: Option<(DisplaySettings, u32)>,
) -> SlotDecision {
    match (slot_a, slot_b) {
        (None, None) => SlotDecision {
            current: None,
            write_slot: SettingsSlot::A,
            next_sequence: 0,
        },
        (Some(a), None) => SlotDecision {
            current: Some(a),
            write_slot: SettingsSlot::B,
            next_sequence: a.1.wrapping_add(1),
        },
        (None, Some(b)) => SlotDecision {
            current: Some(b),
            write_slot: SettingsSlot::A,
            next_sequence: b.1.wrapping_add(1),
        },
        (Some(a), Some(b)) => {
            let a_is_newer = a.1.wrapping_sub(b.1) < u32::MAX / 2;
            let (newer, write_slot) = if a_is_newer {
                (a, SettingsSlot::B)
            } else {
                (b, SettingsSlot::A)
            };
            SlotDecision {
                current: Some(newer),
                write_slot,
                next_sequence: newer.1.wrapping_add(1),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_previous_hardcoded_behavior() {
        let settings = DisplaySettings::DEFAULT;
        assert_eq!(settings.brightness(), 7);
        assert_eq!(settings.power_config(), crate::PowerConfig::DEFAULT);
    }

    #[test]
    fn construction_rejects_invalid_values() {
        assert_eq!(
            DisplaySettings::new(0, 10_000, 20_000),
            Err(SettingsError::BrightnessOutOfRange)
        );
        assert_eq!(
            DisplaySettings::new(8, 10_000, 20_000),
            Err(SettingsError::BrightnessOutOfRange)
        );
        assert_eq!(
            DisplaySettings::new(4, 0, 20_000),
            Err(SettingsError::ZeroDimTimeout)
        );
        assert_eq!(
            DisplaySettings::new(4, 20_000, 20_000),
            Err(SettingsError::OffNotAfterDim)
        );
    }

    #[test]
    fn encode_decode_round_trip_preserves_settings_and_sequence() {
        let settings = DisplaySettings::new(7, 5_000, 30_000)
            .unwrap()
            .toggle_heart_rate()
            .cycle_heart_rate_interval();
        let record = settings.encode(41);
        assert_eq!(DisplaySettings::decode(&record), Ok((settings, 41)));
    }

    #[test]
    fn version_one_migrates_with_heart_rate_disabled() {
        let mut record = DisplaySettings::DEFAULT.toggle_heart_rate().encode(7);
        record[4..6].copy_from_slice(&1_u16.to_le_bytes());
        let crc = crc32(&record[..CRC_OFFSET]);
        record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        let (settings, sequence) = DisplaySettings::decode(&record).unwrap();
        assert_eq!(sequence, 7);
        assert!(!settings.heart_rate_enabled());
        assert_eq!(settings.heart_rate_interval_seconds(), 300);
    }

    #[test]
    fn decode_rejects_corruption_and_foreign_data() {
        let mut record = DisplaySettings::DEFAULT.encode(1);
        record[10] ^= 0x01;
        assert_eq!(
            DisplaySettings::decode(&record),
            Err(DecodeError::BadChecksum)
        );

        let blank = [0xFF_u8; SETTINGS_RECORD_LEN];
        assert_eq!(DisplaySettings::decode(&blank), Err(DecodeError::BadMagic));

        let zeros = [0_u8; SETTINGS_RECORD_LEN];
        assert_eq!(DisplaySettings::decode(&zeros), Err(DecodeError::BadMagic));
    }

    #[test]
    fn decode_rejects_future_versions_without_misreading_them() {
        let mut record = DisplaySettings::DEFAULT.encode(1);
        record[4..6].copy_from_slice(&3_u16.to_le_bytes());
        let crc = crc32(&record[..CRC_OFFSET]);
        record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(
            DisplaySettings::decode(&record),
            Err(DecodeError::UnsupportedVersion(3))
        );
    }

    #[test]
    fn decode_rejects_valid_checksum_with_invalid_content() {
        let mut record = DisplaySettings::DEFAULT.encode(1);
        record[6] = 9;
        let crc = crc32(&record[..CRC_OFFSET]);
        record[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(
            DisplaySettings::decode(&record),
            Err(DecodeError::InvalidContent(
                SettingsError::BrightnessOutOfRange
            ))
        );
    }

    #[test]
    fn crc32_matches_the_ieee_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn slot_selection_covers_the_full_matrix() {
        let settings = DisplaySettings::DEFAULT;

        let empty = select_slot(None, None);
        assert_eq!(empty.current, None);
        assert_eq!(empty.write_slot, SettingsSlot::A);
        assert_eq!(empty.next_sequence, 0);

        let only_a = select_slot(Some((settings, 7)), None);
        assert_eq!(only_a.current, Some((settings, 7)));
        assert_eq!(only_a.write_slot, SettingsSlot::B);
        assert_eq!(only_a.next_sequence, 8);

        let only_b = select_slot(None, Some((settings, 3)));
        assert_eq!(only_b.current, Some((settings, 3)));
        assert_eq!(only_b.write_slot, SettingsSlot::A);
        assert_eq!(only_b.next_sequence, 4);

        let newer_b = select_slot(Some((settings, 3)), Some((settings, 4)));
        assert_eq!(newer_b.current, Some((settings, 4)));
        assert_eq!(newer_b.write_slot, SettingsSlot::A);
        assert_eq!(newer_b.next_sequence, 5);
    }

    #[test]
    fn slot_selection_survives_sequence_wraparound() {
        let settings = DisplaySettings::DEFAULT;
        let decision = select_slot(Some((settings, u32::MAX)), Some((settings, 0)));
        assert_eq!(decision.current, Some((settings, 0)));
        assert_eq!(decision.write_slot, SettingsSlot::A);
        assert_eq!(decision.next_sequence, 1);
    }

    #[test]
    fn preset_cycles_keep_the_off_after_dim_invariant() {
        let mut settings = DisplaySettings::DEFAULT;
        for _ in 0..(BRIGHTNESS_LEVELS.len() * DIM_TIMEOUTS_MILLIS.len() * 2) {
            settings = settings.cycle_brightness();
            settings = settings.cycle_dim_timeout();
            settings = settings.cycle_off_timeout();
            assert!(
                DisplaySettings::new(
                    settings.brightness(),
                    settings.dim_after_millis(),
                    settings.off_after_millis(),
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn cycling_dim_past_the_off_timeout_raises_the_off_timeout() {
        let settings = DisplaySettings::new(4, 20_000, 30_000).unwrap();
        let cycled = settings.cycle_dim_timeout();
        assert_eq!(cycled.dim_after_millis(), 30_000);
        assert_eq!(cycled.off_after_millis(), 60_000);
    }
}
