use microfft::real::rfft_64;

const WINDOW_LENGTH: usize = 64;
const OVERLAP_LENGTH: usize = 5;
const BPM_PER_BIN: f32 = 9.375;
const FIRST_HR_BIN: usize = 5;
const LAST_HR_BIN: usize = 24;
const MIN_SIGNAL_TO_NOISE_POWER: f32 = 9.0;
const SECONDARY_PEAK_POWER_RATIO: f32 = 0.36;
const REQUIRED_CONSISTENT_WINDOWS: u8 = 3;
const MAX_CANDIDATE_CHANGE_BPM: u16 = 12;
const LOW_PASS_ALPHA: f32 = 0.816;
const HIGH_PASS_ALPHA: f32 = 0.268;
const HANN_COS_STEP: f32 = 0.995_030_76;
const HANN_SIN_STEP: f32 = 0.099_567_85;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PpgAnalysis {
    Collecting { samples: u8 },
    AmbientLight,
    NoSignal,
    HeartRate { bpm: u16 },
}

/// Heapless, hardware-independent spectral PPG analyzer for 10 Hz samples.
pub struct PpgProcessor {
    samples: [u16; WINDOW_LENGTH],
    sample_count: usize,
    ambient_threshold: Option<u16>,
    candidate_bpm: Option<u16>,
    consistent_windows: u8,
}

impl PpgProcessor {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: [0; WINDOW_LENGTH],
            sample_count: 0,
            ambient_threshold: None,
            candidate_bpm: None,
            consistent_windows: 0,
        }
    }

    pub const fn reset(&mut self) {
        self.sample_count = 0;
        self.ambient_threshold = None;
        self.candidate_bpm = None;
        self.consistent_windows = 0;
    }

    pub fn push(&mut self, hrs: u16, als: u16) -> PpgAnalysis {
        if self
            .ambient_threshold
            .is_some_and(|threshold| als > threshold)
        {
            self.reset();
            return PpgAnalysis::AmbientLight;
        }

        self.samples[self.sample_count] = hrs;
        self.sample_count += 1;
        if self.sample_count < WINDOW_LENGTH {
            return PpgAnalysis::Collecting {
                samples: u8::try_from(self.sample_count).unwrap_or(u8::MAX),
            };
        }

        let result = self.validate_candidate(analyze(&self.samples));
        self.samples.copy_within(OVERLAP_LENGTH..WINDOW_LENGTH, 0);
        self.sample_count = WINDOW_LENGTH - OVERLAP_LENGTH;
        self.ambient_threshold = Some(als.saturating_mul(2));
        result
    }

    fn validate_candidate(&mut self, analysis: PpgAnalysis) -> PpgAnalysis {
        let PpgAnalysis::HeartRate { bpm } = analysis else {
            self.candidate_bpm = None;
            self.consistent_windows = 0;
            return analysis;
        };

        if self
            .candidate_bpm
            .is_some_and(|candidate| candidate.abs_diff(bpm) <= MAX_CANDIDATE_CHANGE_BPM)
        {
            self.consistent_windows = self.consistent_windows.saturating_add(1);
        } else {
            self.candidate_bpm = Some(bpm);
            self.consistent_windows = 1;
        }
        if self.consistent_windows >= REQUIRED_CONSISTENT_WINDOWS {
            PpgAnalysis::HeartRate { bpm }
        } else {
            PpgAnalysis::NoSignal
        }
    }
}

impl Default for PpgProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn analyze(raw: &[u16; WINDOW_LENGTH]) -> PpgAnalysis {
    let first = f32::from(raw[0]);
    let slope = (f32::from(raw[WINDOW_LENGTH - 1]) - first) / (WINDOW_LENGTH - 1) as f32;
    let mut signal = [0.0; WINDOW_LENGTH];
    for (index, value) in raw.iter().enumerate() {
        let trend = first + slope * index as f32;
        signal[index] = f32::from(*value) - trend;
    }
    band_pass(&mut signal);

    let mut hann_cos = 1.0;
    let mut hann_sin = 0.0;
    for value in &mut signal {
        *value *= 0.5 * (1.0 - hann_cos);
        let next_cos = hann_cos * HANN_COS_STEP - hann_sin * HANN_SIN_STEP;
        hann_sin = hann_sin * HANN_COS_STEP + hann_cos * HANN_SIN_STEP;
        hann_cos = next_cos;
    }

    let spectrum = rfft_64(&mut signal);
    spectrum[0].im = 0.0;
    let mut peak_bin = FIRST_HR_BIN;
    let mut peak_power = 0.0;
    let mut powers = [0.0; LAST_HR_BIN + 2];
    for bin in FIRST_HR_BIN - 1..=LAST_HR_BIN + 1 {
        let coefficient = spectrum[bin];
        let power = coefficient.re * coefficient.re + coefficient.im * coefficient.im;
        powers[bin] = power;
        if (FIRST_HR_BIN..=LAST_HR_BIN).contains(&bin) && power > peak_power {
            peak_power = power;
            peak_bin = bin;
        }
    }

    let mut noise_power = 0.0;
    let mut noise_bins = 0;
    for (bin, power) in powers
        .iter()
        .enumerate()
        .take(LAST_HR_BIN + 1)
        .skip(FIRST_HR_BIN)
    {
        if bin.abs_diff(peak_bin) > 1 {
            noise_power += power;
            noise_bins += 1;
        }
    }
    let noise_mean = noise_power / noise_bins as f32;
    if peak_power <= f32::EPSILON || peak_power < noise_mean * MIN_SIGNAL_TO_NOISE_POWER {
        return PpgAnalysis::NoSignal;
    }

    let secondary_threshold = peak_power * SECONDARY_PEAK_POWER_RATIO;
    let peak_count = (FIRST_HR_BIN..=LAST_HR_BIN)
        .filter(|&bin| {
            powers[bin] >= secondary_threshold
                && powers[bin] > powers[bin - 1]
                && powers[bin] >= powers[bin + 1]
        })
        .count();
    if peak_count != 1 {
        return PpgAnalysis::NoSignal;
    }

    let left = powers[peak_bin - 1];
    let center = powers[peak_bin];
    let right = powers[peak_bin + 1];
    let denominator = left - 2.0 * center + right;
    let offset = if denominator.abs() > f32::EPSILON {
        (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let bpm = ((peak_bin as f32 + offset) * BPM_PER_BIN + 0.5) as u16;
    if (40..=230).contains(&bpm) {
        PpgAnalysis::HeartRate { bpm }
    } else {
        PpgAnalysis::NoSignal
    }
}

fn band_pass(signal: &mut [f32; WINDOW_LENGTH]) {
    for _ in 0..4 {
        let mut average = signal[0];
        for value in &mut *signal {
            average = LOW_PASS_ALPHA * *value + (1.0 - LOW_PASS_ALPHA) * average;
            *value = average;
        }
    }
    for _ in 0..4 {
        let mut average = signal[0];
        for value in &mut *signal {
            average = HIGH_PASS_ALPHA * *value + (1.0 - HIGH_PASS_ALPHA) * average;
            *value -= average;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_periodic(processor: &mut PpgProcessor, pattern: &[i16]) -> PpgAnalysis {
        let mut result = PpgAnalysis::NoSignal;
        for index in 0..WINDOW_LENGTH + 2 * OVERLAP_LENGTH {
            let value = 10_000_i32 + i32::from(pattern[index % pattern.len()]);
            result = processor.push(u16::try_from(value).unwrap(), 100);
        }
        result
    }

    #[test]
    fn waits_for_a_complete_window() {
        let mut processor = PpgProcessor::new();
        for samples in 1..WINDOW_LENGTH {
            assert_eq!(
                processor.push(10_000, 100),
                PpgAnalysis::Collecting {
                    samples: u8::try_from(samples).unwrap()
                }
            );
        }
    }

    #[test]
    fn detects_sixty_beats_per_minute() {
        let pattern = [0, 588, 951, 951, 588, 0, -588, -951, -951, -588];
        let result = feed_periodic(&mut PpgProcessor::new(), &pattern);
        let PpgAnalysis::HeartRate { bpm } = result else {
            panic!("expected heart rate, got {result:?}");
        };
        assert!((55..=65).contains(&bpm));
    }

    #[test]
    fn detects_one_hundred_twenty_beats_per_minute() {
        let pattern = [0, 951, 588, -588, -951];
        let result = feed_periodic(&mut PpgProcessor::new(), &pattern);
        let PpgAnalysis::HeartRate { bpm } = result else {
            panic!("expected heart rate, got {result:?}");
        };
        assert!((115..=125).contains(&bpm));
    }

    #[test]
    fn rejects_a_constant_signal() {
        let result = feed_periodic(&mut PpgProcessor::new(), &[0]);
        assert_eq!(result, PpgAnalysis::NoSignal);
    }

    #[test]
    fn rejects_two_competing_periodic_signals() {
        let sixty = [0, 588, 951, 951, 588, 0, -588, -951, -951, -588];
        let one_twenty = [0, 951, 588, -588, -951];
        let mut processor = PpgProcessor::new();
        let mut result = PpgAnalysis::NoSignal;
        for index in 0..WINDOW_LENGTH + 2 * OVERLAP_LENGTH {
            let mixed = i32::from(sixty[index % sixty.len()])
                + i32::from(one_twenty[index % one_twenty.len()]);
            result = processor.push(u16::try_from(10_000 + mixed).unwrap(), 100);
        }
        assert_eq!(result, PpgAnalysis::NoSignal);
    }

    #[test]
    fn rejects_ambient_light_spikes_after_calibration() {
        let pattern = [0, 588, 951, 951, 588, 0, -588, -951, -951, -588];
        let mut processor = PpgProcessor::new();
        let _ = feed_periodic(&mut processor, &pattern);
        assert_eq!(processor.push(10_000, 201), PpgAnalysis::AmbientLight);
    }
}
