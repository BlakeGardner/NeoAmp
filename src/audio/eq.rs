//! Winamp's ten-band graphic equalizer: a preamp and one peaking filter per band.
//!
//! Each band is an RBJ "audio EQ cookbook" peaking biquad at Winamp's centre frequency, run in
//! transposed direct form II. A flat band leaves the signal untouched.

/// Number of equalizer bands.
pub const BANDS: usize = 10;
/// Centre frequencies of the bands in Hz, as labelled on the equalizer window.
pub const BAND_HZ: [f32; BANDS] = [
    60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12_000.0, 14_000.0, 16_000.0,
];
/// Each slider reaches this many dB up or down.
pub const RANGE_DB: f32 = 12.0;
/// Bandwidth of each band: Q = 1 is about 1.4 octaves, so neighbouring bands blend into a
/// smooth curve instead of leaving dips between them.
const Q: f32 = 1.0;
/// A constant far below hearing, added at the input so the filters' state never decays into
/// denormal numbers (which are very slow on some CPUs).
const ANTI_DENORMAL: f32 = 1e-20;

/// Equalizer settings as the UI shows them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    /// The equalizer's ON button. When off, the preamp is bypassed too, as in Winamp.
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands_db: [f32; BANDS],
}

impl Default for EqSettings {
    fn default() -> Self {
        EqSettings {
            enabled: true,
            preamp_db: 0.0,
            bands_db: [0.0; BANDS],
        }
    }
}

/// One second-order filter with a separate state for each stereo channel.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    s1: [f32; 2],
    s2: [f32; 2],
}

impl Biquad {
    const IDENTITY: Biquad = Biquad {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
        s1: [0.0; 2],
        s2: [0.0; 2],
    };

    /// Set this filter to a peaking EQ of `gain_db` at `freq`, keeping its state so a change
    /// while playing does not click.
    fn set_peaking(&mut self, rate: f32, freq: f32, gain_db: f32) {
        if gain_db.abs() < 0.01 || freq >= rate * 0.45 {
            let (s1, s2) = (self.s1, self.s2);
            *self = Biquad {
                s1,
                s2,
                ..Self::IDENTITY
            };
            return;
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * freq / rate;
        let alpha = w0.sin() / (2.0 * Q);
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;
        self.b0 = (1.0 + alpha * a) / a0;
        self.b1 = -2.0 * cos / a0;
        self.b2 = (1.0 - alpha * a) / a0;
        self.a1 = self.b1;
        self.a2 = (1.0 - alpha / a) / a0;
    }

    #[inline]
    fn process(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.b0 * x + self.s1[ch];
        self.s1[ch] = self.b1 * x - self.a1 * y + self.s2[ch];
        self.s2[ch] = self.b2 * x - self.a2 * y;
        y
    }
}

/// The filter chain the device callback runs.
#[derive(Clone, Debug)]
pub(super) struct Equalizer {
    enabled: bool,
    /// Linear preamp gain.
    preamp: f32,
    bands: [Biquad; BANDS],
}

impl Default for Equalizer {
    fn default() -> Self {
        Equalizer {
            enabled: false,
            preamp: 1.0,
            bands: [Biquad::IDENTITY; BANDS],
        }
    }
}

impl Equalizer {
    /// Apply new settings for audio at `rate` Hz. Gains are clamped to the sliders' range.
    pub(super) fn configure(
        &mut self,
        rate: u32,
        enabled: bool,
        preamp_db: f32,
        bands_db: &[f32; BANDS],
    ) {
        // Sample rates are far below 2^24, so the conversion is exact.
        let rate = rate as f32;
        self.enabled = enabled;
        self.preamp = 10f32.powf(preamp_db.clamp(-RANGE_DB, RANGE_DB) / 20.0);
        for ((band, freq), db) in self.bands.iter_mut().zip(BAND_HZ).zip(bands_db) {
            band.set_peaking(rate, freq, db.clamp(-RANGE_DB, RANGE_DB));
        }
    }

    /// Run one stereo frame through the preamp and every band.
    #[inline]
    pub(super) fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled {
            return (l, r);
        }
        let mut l = l * self.preamp + ANTI_DENORMAL;
        let mut r = r * self.preamp + ANTI_DENORMAL;
        for band in &mut self.bands {
            l = band.process(0, l);
            r = band.process(1, r);
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady-state gain in dB of a sine at `freq` through `eq` (left channel).
    fn gain_db(eq: &mut Equalizer, freq: f32) -> f32 {
        let rate = 48_000.0;
        let mut peak = 0.0f32;
        for i in 0..48_000 {
            let x = (std::f32::consts::TAU * freq * i as f32 / rate).sin();
            let (y, _) = eq.process(x, 0.0);
            if i > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        20.0 * peak.log10()
    }

    #[test]
    fn a_flat_equalizer_is_transparent() {
        let mut eq = Equalizer::default();
        eq.configure(48_000, true, 0.0, &[0.0; BANDS]);
        for f in [50.0, 1000.0, 15_000.0] {
            assert!(gain_db(&mut eq, f).abs() < 0.01, "{f} Hz");
        }
    }

    #[test]
    fn a_band_boosts_its_own_frequency_most() {
        let mut eq = Equalizer::default();
        let mut bands = [0.0; BANDS];
        bands[4] = 12.0; // 1 kHz
        eq.configure(48_000, true, 0.0, &bands);
        let at = gain_db(&mut eq, 1000.0);
        assert!((at - 12.0).abs() < 0.2, "1 kHz gain {at}");
        assert!(gain_db(&mut eq, 100.0).abs() < 1.0);
        assert!(gain_db(&mut eq, 10_000.0).abs() < 1.0);
    }

    #[test]
    fn preamp_and_bypass() {
        let mut eq = Equalizer::default();
        eq.configure(48_000, true, -6.0, &[0.0; BANDS]);
        assert!((gain_db(&mut eq, 440.0) + 6.0).abs() < 0.05);
        eq.configure(48_000, false, -6.0, &[12.0; BANDS]);
        assert_eq!(eq.process(0.25, -0.5), (0.25, -0.5));
    }

    #[test]
    fn gains_are_clamped_and_bands_above_nyquist_are_left_flat() {
        let mut eq = Equalizer::default();
        let mut bands = [0.0; BANDS];
        bands[9] = 12.0;
        eq.configure(32_000, true, 40.0, &bands);
        assert!((eq.preamp - 10f32.powf(0.6)).abs() < 1e-5);
        // 16 kHz is above the 14.4 kHz limit at 32 kHz.
        assert_eq!(eq.bands[9].b0, 1.0);
    }
}
