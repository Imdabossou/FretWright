//! Integrated loudness (LUFS) as specified by ITU-R BS.1770-4 / EBU R 128:
//! K-weighting (a high shelf and a high-pass), mean square over 400 ms blocks
//! every 100 ms, then an absolute gate at -70 LUFS and a relative gate 10 LU
//! below the absolute-gated loudness. The filters are designed for the actual
//! sample rate with the formulas libebur128 (MIT) uses, which reproduce the
//! standard's 48 kHz coefficients.

use std::f64::consts::PI;

use crate::Audio;

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    /// Stage 1: high shelf modelling the head (+4 dB above ~1.7 kHz).
    fn shelf(rate: f64) -> Biquad {
        let (f0, gain_db, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        Biquad {
            b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        }
    }

    /// Stage 2: the "RLB" high-pass at ~38 Hz.
    fn high_pass(rate: f64) -> Biquad {
        let (f0, q) = (38.13547087602444, 0.5003270373238773);
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Biquad { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0] }
    }

    /// Filters in place (direct form II transposed).
    fn run(&self, x: &mut [f64]) {
        let (mut z1, mut z2) = (0.0, 0.0);
        for s in x.iter_mut() {
            let y = self.b[0] * *s + z1;
            z1 = self.b[1] * *s - self.a[0] * y + z2;
            z2 = self.b[2] * *s - self.a[1] * y;
            *s = y;
        }
    }
}

fn lufs(mean_square: f64) -> f64 {
    -0.691 + 10.0 * mean_square.log10()
}

/// Channel weights: 1 for left, right and centre, 1.41 for the surrounds
/// (channels 4 and 5 in the usual L R C LFE Ls Rs order; LFE is not
/// special-cased, as in most meters).
fn weight(channel: usize) -> f64 {
    if channel == 3 || channel == 4 { 1.41 } else { 1.0 }
}

/// Integrated loudness of the audio in LUFS (-inf for silence).
pub fn integrated(audio: &Audio) -> f64 {
    let rate = audio.sample_rate as f64;
    let (shelf, hp) = (Biquad::shelf(rate), Biquad::high_pass(rate));
    let filtered: Vec<Vec<f64>> = audio
        .channels
        .iter()
        .map(|c| {
            let mut x: Vec<f64> = c.iter().map(|&s| s as f64).collect();
            shelf.run(&mut x);
            hp.run(&mut x);
            x
        })
        .collect();
    let block = (0.4 * rate).round() as usize;
    let hop = (0.1 * rate).round() as usize;
    let n = audio.frames();
    if n < block || hop == 0 {
        return f64::NEG_INFINITY;
    }
    // weighted mean square of every 400 ms block, from per-channel prefix sums
    let prefix: Vec<Vec<f64>> = filtered
        .iter()
        .map(|x| {
            let mut acc = Vec::with_capacity(x.len() + 1);
            acc.push(0.0);
            let mut sum = 0.0;
            for v in x {
                sum += v * v;
                acc.push(sum);
            }
            acc
        })
        .collect();
    let blocks: Vec<f64> = (0..=(n - block) / hop)
        .map(|i| {
            let (a, b) = (i * hop, i * hop + block);
            prefix.iter().enumerate().map(|(c, p)| weight(c) * (p[b] - p[a]) / block as f64).sum()
        })
        .collect();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let absolute: Vec<f64> = blocks.into_iter().filter(|&ms| lufs(ms) > -70.0).collect();
    if absolute.is_empty() {
        return f64::NEG_INFINITY;
    }
    let gate = lufs(mean(&absolute)) - 10.0;
    let relative: Vec<f64> = absolute.iter().copied().filter(|&ms| lufs(ms) > gate).collect();
    lufs(mean(&relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amp: f32, secs: f64, rate: u32, channels: usize) -> Audio {
        let n = (secs * rate as f64) as usize;
        let x: Vec<f32> = (0..n).map(|i| amp * (2.0 * PI * freq * i as f64 / rate as f64).sin() as f32).collect();
        Audio { sample_rate: rate, channels: vec![x; channels] }
    }

    #[test]
    fn filters_match_the_standards_48k_coefficients() {
        // ITU-R BS.1770-4, table 1 and table 2
        let s = Biquad::shelf(48_000.0);
        let expect_b = [1.53512485958697, -2.69169618940638, 1.19839281085285];
        let expect_a = [-1.69065929318241, 0.73248077421585];
        for (x, y) in s.b.iter().zip(expect_b).chain(s.a.iter().zip(expect_a)) {
            assert!((x - y).abs() < 1e-9, "{x} vs {y}");
        }
        let h = Biquad::high_pass(48_000.0);
        for (x, y) in h.a.iter().zip([-1.99004745483398, 0.99007225036621]) {
            assert!((x - y).abs() < 1e-9, "{x} vs {y}");
        }
    }

    #[test]
    fn reference_tone_reads_as_specified() {
        // EBU Tech 3341 test 1: a 1 kHz sine at -23 dBFS in both channels of
        // a stereo signal reads -23.0 LUFS
        let amp = 10f32.powf(-23.0 / 20.0);
        for rate in [44_100, 48_000] {
            let l = integrated(&sine(1000.0, amp, 20.0, rate, 2));
            assert!((l - -23.0).abs() < 0.05, "{rate} Hz: {l}");
        }
    }

    #[test]
    fn silence_is_minus_infinity() {
        assert_eq!(integrated(&sine(1000.0, 0.0, 5.0, 48_000, 2)), f64::NEG_INFINITY);
    }
}
