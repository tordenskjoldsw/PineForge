//! What a companion says the weather is.
//!
//! `InfiniTime`'s Simple Weather service takes two writes on one characteristic,
//! and this is the half that decides what they mean. No BLE and no drawing: the
//! packets are bytes a phone wrote, every rule about reading them can be wrong
//! without being visibly wrong, and all of it is checked on the host.
//!
//! # The wire format
//!
//! Both packets are little-endian and begin the same way: a message type, a
//! version, and a local timestamp.
//!
//! | Offset | Current weather        | Forecast                     |
//! | ------ | ---------------------- | ---------------------------- |
//! | 0      | type `0`               | type `1`                     |
//! | 1      | version `0` or `1`     | version `0`                  |
//! | 2..10  | timestamp, `u64`       | timestamp, `u64`             |
//! | 10..12 | temperature, `i16`     | day count, `u8` at 10        |
//! | 12..16 | today's min and max    | five bytes per day from 11   |
//! | 16..48 | location, 32 bytes     |                              |
//! | 48     | icon                   |                              |
//! | 49..53 | sunrise, sunset (v1)   |                              |
//!
//! Temperatures are hundredths of a degree Celsius, which is why they are kept
//! as they arrive rather than converted here: a screen that wants whole degrees
//! can round, and one that wants a decimal place still has it.
//!
//! # What is rejected
//!
//! The version rules are `InfiniTime`'s, and they are not symmetrical: current
//! weather is accepted at version 0 or 1, a forecast only at version 0. A
//! version this firmware does not know is not guessed at - a packet whose layout
//! is uncertain is worse than no weather, because the watch would show a number
//! with confidence.

use heapless::{String, Vec};

use crate::notification::sanitize;

/// Bytes the location field occupies. `InfiniTime` keeps 33 for a trailing NUL;
/// the text here is bounded rather than terminated, so the NUL is not stored.
pub const WEATHER_LOCATION_MAX: usize = 32;

/// Days a forecast may carry, as the companion caps it.
pub const FORECAST_DAYS_MAX: usize = 5;

const TYPE_CURRENT: u8 = 0;
const TYPE_FORECAST: u8 = 1;

const HEADER_LEN: usize = 10;
const CURRENT_LEN_V0: usize = 49;
const CURRENT_LEN_V1: usize = 53;
const FORECAST_HEADER_LEN: usize = 11;
const FORECAST_DAY_LEN: usize = 5;

/// The conditions `InfiniTime` draws, by the ids the companion maps onto.
///
/// The names are the upstream ones so the two can be compared without a
/// translation table in between; what each looks like is the drawing layer's
/// problem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeatherIcon {
    Sun,
    CloudsSun,
    Clouds,
    BrokenClouds,
    CloudShowerHeavy,
    CloudSunRain,
    Thunderstorm,
    Snow,
    Smog,
    /// Anything the companion could not map, and anything this firmware does not
    /// know. Upstream sends 255 for the first; the second is why this is a
    /// fallback rather than a rejection - an unfamiliar id is still a forecast
    /// with real temperatures in it.
    #[default]
    Unknown,
}

impl WeatherIcon {
    #[must_use]
    pub const fn from_id(id: u8) -> Self {
        match id {
            0 => Self::Sun,
            1 => Self::CloudsSun,
            2 => Self::Clouds,
            3 => Self::BrokenClouds,
            4 => Self::CloudShowerHeavy,
            5 => Self::CloudSunRain,
            6 => Self::Thunderstorm,
            7 => Self::Snow,
            8 => Self::Smog,
            _ => Self::Unknown,
        }
    }
}

/// A temperature as it arrives: hundredths of a degree Celsius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Temperature(i16);

impl Temperature {
    #[must_use]
    pub const fn from_hundredths(raw: i16) -> Self {
        Self(raw)
    }

    /// The value as sent, in hundredths of a degree.
    #[must_use]
    pub const fn hundredths(self) -> i16 {
        self.0
    }

    /// Whole degrees, rounded to nearest rather than truncated.
    ///
    /// Truncation is wrong in a way people notice: it turns -0.4 into 0 and
    /// 20.6 into 20, so a watch reading one degree cooler than the phone would
    /// be the normal case rather than the edge.
    #[must_use]
    pub const fn celsius(self) -> i16 {
        let raw = self.0;
        if raw >= 0 {
            (raw + 50) / 100
        } else {
            (raw - 50) / 100
        }
    }
}

/// Today, as the phone last described it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CurrentWeather {
    /// Local seconds since the epoch, as the companion computed them.
    pub timestamp: u64,
    pub temperature: Temperature,
    pub minimum: Temperature,
    pub maximum: Temperature,
    pub icon: WeatherIcon,
    pub location: String<WEATHER_LOCATION_MAX>,
    /// Minutes since midnight. Absent in a version-0 packet, which carries
    /// neither - and absent is not the same as midnight.
    pub sunrise: Option<u16>,
    pub sunset: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForecastDay {
    pub minimum: Temperature,
    pub maximum: Temperature,
    pub icon: WeatherIcon,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Forecast {
    pub timestamp: u64,
    pub days: Vec<ForecastDay, FORECAST_DAYS_MAX>,
}

/// One accepted write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WeatherUpdate {
    Current(CurrentWeather),
    Forecast(Forecast),
}

/// Reads one write to the Simple Weather characteristic.
///
/// `None` for anything not understood, and the caller's only reasonable answer
/// to that is to keep showing what it had. A packet that is truncated, carries
/// an unknown message type, or states a version whose layout is not this one is
/// not partially decoded: the fields would be read from the wrong offsets, and
/// a wrong temperature shown confidently is worse than none.
#[must_use]
pub fn parse_simple_weather(payload: &[u8]) -> Option<WeatherUpdate> {
    if payload.len() < HEADER_LEN {
        return None;
    }
    let version = payload[1];
    let timestamp = u64::from_le_bytes(payload[2..HEADER_LEN].try_into().ok()?);

    match payload[0] {
        TYPE_CURRENT => parse_current(payload, version, timestamp).map(WeatherUpdate::Current),
        TYPE_FORECAST => parse_forecast(payload, version, timestamp).map(WeatherUpdate::Forecast),
        _ => None,
    }
}

fn parse_current(payload: &[u8], version: u8, timestamp: u64) -> Option<CurrentWeather> {
    // Version 1 adds the sun times and nothing else, so a version-0 packet is
    // read exactly as far as it goes. Anything above 1 is a layout this
    // firmware has not seen.
    let with_sun = match version {
        0 => false,
        1 => true,
        _ => return None,
    };
    let required = if with_sun {
        CURRENT_LEN_V1
    } else {
        CURRENT_LEN_V0
    };
    if payload.len() < required {
        return None;
    }

    let read = |at: usize| i16::from_le_bytes([payload[at], payload[at + 1]]);
    let sun = |at: usize| u16::from_le_bytes([payload[at], payload[at + 1]]);

    Some(CurrentWeather {
        timestamp,
        temperature: Temperature(read(10)),
        minimum: Temperature(read(12)),
        maximum: Temperature(read(14)),
        // The field is NUL-padded to its full width, and `sanitize` drops those
        // along with anything that is not well-formed UTF-8 - the same
        // treatment a notification and a track title get, and for the same
        // reason: text a phone wrote into a fixed-width characteristic.
        location: sanitize(&payload[16..48]),
        icon: WeatherIcon::from_id(payload[48]),
        sunrise: with_sun.then(|| sun(49)),
        sunset: with_sun.then(|| sun(51)),
    })
}

fn parse_forecast(payload: &[u8], version: u8, timestamp: u64) -> Option<Forecast> {
    // Asymmetric with the above on purpose, and this is upstream's rule rather
    // than a simplification: a forecast has only ever been version 0.
    if version != 0 || payload.len() < FORECAST_HEADER_LEN {
        return None;
    }

    let stated = payload[10] as usize;
    // The companion caps this at five, so a larger count is a packet this
    // firmware does not understand rather than one to read five days out of.
    if stated > FORECAST_DAYS_MAX {
        return None;
    }
    if payload.len() < FORECAST_HEADER_LEN + stated * FORECAST_DAY_LEN {
        return None;
    }

    let mut days = Vec::new();
    for day in 0..stated {
        let at = FORECAST_HEADER_LEN + day * FORECAST_DAY_LEN;
        let minimum = Temperature(i16::from_le_bytes([payload[at], payload[at + 1]]));
        let maximum = Temperature(i16::from_le_bytes([payload[at + 2], payload[at + 3]]));
        // The capacity equals the cap checked above, so this cannot fail.
        let _ = days.push(ForecastDay {
            minimum,
            maximum,
            icon: WeatherIcon::from_id(payload[at + 4]),
        });
    }

    Some(Forecast { timestamp, days })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A current-weather packet the way the companion assembles one.
    fn current(version: u8, temperature: i16, location: &str) -> [u8; CURRENT_LEN_V1] {
        let mut packet = [0; CURRENT_LEN_V1];
        packet[0] = TYPE_CURRENT;
        packet[1] = version;
        packet[2..10].copy_from_slice(&1_700_000_000_u64.to_le_bytes());
        packet[10..12].copy_from_slice(&temperature.to_le_bytes());
        packet[12..14].copy_from_slice(&(-250_i16).to_le_bytes());
        packet[14..16].copy_from_slice(&1_150_i16.to_le_bytes());
        packet[16..16 + location.len()].copy_from_slice(location.as_bytes());
        packet[48] = 6;
        packet[49..51].copy_from_slice(&389_u16.to_le_bytes());
        packet[51..53].copy_from_slice(&1_260_u16.to_le_bytes());
        packet
    }

    fn forecast(days: &[(i16, i16, u8)]) -> Vec<u8, 64> {
        let mut packet: Vec<u8, 64> = Vec::new();
        let _ = packet.resize(FORECAST_HEADER_LEN + days.len() * FORECAST_DAY_LEN, 0);
        packet[0] = TYPE_FORECAST;
        packet[1] = 0;
        packet[2..10].copy_from_slice(&1_700_000_000_u64.to_le_bytes());
        packet[10] = u8::try_from(days.len()).unwrap();
        for (index, (minimum, maximum, icon)) in days.iter().enumerate() {
            let at = FORECAST_HEADER_LEN + index * FORECAST_DAY_LEN;
            packet[at..at + 2].copy_from_slice(&minimum.to_le_bytes());
            packet[at + 2..at + 4].copy_from_slice(&maximum.to_le_bytes());
            packet[at + 4] = *icon;
        }
        packet
    }

    #[test]
    fn a_version_one_packet_carries_everything_including_the_sun() {
        let Some(WeatherUpdate::Current(weather)) =
            parse_simple_weather(&current(1, 2_137, "Kiel"))
        else {
            panic!("a well-formed current-weather packet was rejected");
        };
        assert_eq!(weather.timestamp, 1_700_000_000);
        assert_eq!(weather.temperature.hundredths(), 2_137);
        assert_eq!(weather.minimum.hundredths(), -250);
        assert_eq!(weather.maximum.hundredths(), 1_150);
        assert_eq!(weather.location.as_str(), "Kiel");
        assert_eq!(weather.icon, WeatherIcon::Thunderstorm);
        assert_eq!(weather.sunrise, Some(389));
        assert_eq!(weather.sunset, Some(1_260));
    }

    /// Version 0 has no sun times, and the fields where they would sit are not
    /// part of the packet at all. Reading them anyway would report a sunrise
    /// from whatever followed in the buffer.
    #[test]
    fn a_version_zero_packet_reports_no_sun_times_rather_than_midnight() {
        let packet = current(0, 500, "Kiel");
        let Some(WeatherUpdate::Current(weather)) = parse_simple_weather(&packet[..CURRENT_LEN_V0])
        else {
            panic!("a version-0 packet was rejected");
        };
        assert_eq!(weather.temperature.hundredths(), 500);
        assert_eq!(weather.sunrise, None);
        assert_eq!(weather.sunset, None);
    }

    /// A version whose layout is unknown is refused rather than read at the
    /// offsets of a version that happens to be known. A wrong temperature shown
    /// confidently is worse than no weather.
    #[test]
    fn a_future_version_is_refused_rather_than_guessed_at() {
        assert!(parse_simple_weather(&current(2, 2_000, "Kiel")).is_none());
        assert!(parse_simple_weather(&current(255, 2_000, "Kiel")).is_none());
    }

    #[test]
    fn a_truncated_packet_is_refused_at_every_length() {
        let packet = current(1, 2_000, "Kiel");
        for length in 0..CURRENT_LEN_V1 {
            assert!(
                parse_simple_weather(&packet[..length]).is_none(),
                "a {length}-byte packet was accepted"
            );
        }
    }

    #[test]
    fn an_unknown_message_type_is_refused() {
        let mut packet = current(1, 2_000, "Kiel");
        packet[0] = 9;
        assert!(parse_simple_weather(&packet).is_none());
    }

    /// The location is NUL-padded to its full width, and none of that padding
    /// belongs in the name of a town.
    #[test]
    fn the_location_loses_its_padding() {
        let Some(WeatherUpdate::Current(weather)) =
            parse_simple_weather(&current(1, 0, "Bad Oldesloe"))
        else {
            panic!("rejected");
        };
        assert_eq!(weather.location.as_str(), "Bad Oldesloe");
        assert_eq!(weather.location.len(), "Bad Oldesloe".len());
    }

    #[test]
    fn a_forecast_carries_each_day_in_order() {
        let packet = forecast(&[(-500, 300, 7), (100, 900, 1), (1_200, 2_400, 0)]);
        let Some(WeatherUpdate::Forecast(forecast)) = parse_simple_weather(&packet) else {
            panic!("a well-formed forecast was rejected");
        };
        assert_eq!(forecast.days.len(), 3);
        assert_eq!(forecast.days[0].minimum.hundredths(), -500);
        assert_eq!(forecast.days[0].icon, WeatherIcon::Snow);
        assert_eq!(forecast.days[2].maximum.hundredths(), 2_400);
        assert_eq!(forecast.days[2].icon, WeatherIcon::Sun);
    }

    /// Upstream accepts a forecast at version 0 only, unlike current weather.
    /// The asymmetry is theirs, and copying it is the point.
    #[test]
    fn a_forecast_is_refused_above_version_zero() {
        let mut packet = forecast(&[(0, 100, 0)]);
        packet[1] = 1;
        assert!(parse_simple_weather(&packet).is_none());
    }

    /// A count larger than the packet describes would otherwise read days out of
    /// whatever followed it.
    #[test]
    fn a_day_count_the_packet_does_not_carry_is_refused() {
        let mut packet = forecast(&[(0, 100, 0)]);
        packet[10] = 5;
        assert!(parse_simple_weather(&packet).is_none());
    }

    #[test]
    fn a_day_count_beyond_the_cap_is_refused() {
        let mut packet = forecast(&[(0, 100, 0), (0, 100, 0)]);
        packet[10] = 6;
        assert!(parse_simple_weather(&packet).is_none());
    }

    #[test]
    fn an_empty_forecast_is_a_forecast_with_no_days() {
        let Some(WeatherUpdate::Forecast(forecast)) = parse_simple_weather(&forecast(&[])) else {
            panic!("an empty forecast was rejected");
        };
        assert!(forecast.days.is_empty());
    }

    /// An id this firmware does not know still arrives with real temperatures
    /// in it, so the condition falls back rather than the packet being thrown
    /// away.
    #[test]
    fn an_unfamiliar_icon_falls_back_without_losing_the_packet() {
        let packet = forecast(&[(0, 100, 200)]);
        let Some(WeatherUpdate::Forecast(forecast)) = parse_simple_weather(&packet) else {
            panic!("rejected");
        };
        assert_eq!(forecast.days[0].icon, WeatherIcon::Unknown);
        assert_eq!(forecast.days[0].maximum.hundredths(), 100);
    }

    /// Rounding to nearest, in both directions and across zero. Truncation
    /// would make a watch read a degree cooler than the phone as the normal
    /// case rather than the edge.
    #[test]
    fn whole_degrees_round_rather_than_truncate() {
        for (hundredths, expected) in [
            (2_137, 21),
            (2_150, 22),
            (2_199, 22),
            (0, 0),
            (-40, 0),
            (-50, -1),
            (-250, -3),
            (-1_249, -12),
        ] {
            assert_eq!(
                Temperature::from_hundredths(hundredths).celsius(),
                expected,
                "{hundredths} hundredths"
            );
        }
    }
}
