//! A small neural network that says which notes are sounding in every 11.6 ms
//! of the bass stem. Trained on official Rocksmith charts (see `tools/ml`), run
//! here in plain Rust.
//!
//! Input: for 138 candidate notes (3 per semitone, MIDI 22.7-68.7) the strength
//! of their first 5 harmonics in a 186 ms spectrogram, so each candidate sees
//! its own harmonic series. Output: a probability for each of the 46 semitones
//! MIDI 23-68, per frame.

use rayon::prelude::*;

use crate::dsp;

const SR: f64 = 22050.0;
const N_FFT: usize = 4096;
const HOP: usize = 256;
const N_BINS: usize = 138;
const HARMONICS: usize = 5;
pub const LO_MIDI: i32 = 23;
pub const N_SEMI: usize = 46;
const MAGIC: &[u8; 4] = b"FWPN";
/// Frames processed together; the layers see 7 frames either side.
const CHUNK: usize = 512;
const HALO: usize = 8;

#[derive(Debug, Clone)]
struct Conv2 {
    /// `[out][kh][kw][in]`
    w: Vec<f32>,
    b: Vec<f32>,
    c_in: usize,
    c_out: usize,
    kh: usize,
    kw: usize,
    dh: usize,
    dw: usize,
    ph: usize,
    pw: usize,
    sh: usize,
}

#[derive(Debug, Clone)]
pub struct PitchNet {
    layers: [Conv2; 7],
}

/// Probability of each semitone (MIDI 23-68) sounding in every frame.
#[derive(Debug, Clone)]
pub struct PitchRoll {
    pub frames: usize,
    prob: Vec<f32>,
}

impl PitchRoll {
    pub const FRAME_S: f64 = HOP as f64 / SR;

    /// Adds another roll of the same song, frame by frame.
    pub fn add(&mut self, other: &PitchRoll) {
        self.prob.iter_mut().zip(&other.prob).for_each(|(a, b)| *a += b);
    }

    pub fn scale(&mut self, k: f32) {
        self.prob.iter_mut().for_each(|v| *v *= k);
    }

    /// Mean probability of each semitone over `from..to` seconds (at least one frame).
    pub fn mean(&self, from: f64, to: f64) -> [f32; N_SEMI] {
        let a = ((from / Self::FRAME_S) as usize).min(self.frames.saturating_sub(1));
        let b = ((to / Self::FRAME_S) as usize).clamp(a + 1, self.frames);
        let mut out = [0f32; N_SEMI];
        for t in a..b {
            for (o, p) in out.iter_mut().zip(&self.prob[t * N_SEMI..(t + 1) * N_SEMI]) {
                *o += p;
            }
        }
        out.iter_mut().for_each(|v| *v /= (b - a) as f32);
        out
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

impl Conv2 {
    /// `[t][f][c_in]` -> `[t][f_out][c_out]`, before any activation.
    fn run(&self, x: &[f32], t_len: usize, f_in: usize) -> (Vec<f32>, usize) {
        let f_out = (f_in + 2 * self.ph - self.dh * (self.kh - 1) - 1) / self.sh + 1;
        let mut out = vec![0f32; t_len * f_out * self.c_out];
        out.par_chunks_mut(f_out * self.c_out).enumerate().for_each(|(t, row)| {
            for fo in 0..f_out {
                for o in 0..self.c_out {
                    let mut acc = self.b[o];
                    for ih in 0..self.kh {
                        let fi = (fo * self.sh + ih * self.dh) as isize - self.ph as isize;
                        if fi < 0 || fi >= f_in as isize {
                            continue;
                        }
                        for iw in 0..self.kw {
                            let ti = t as isize + (iw * self.dw) as isize - self.pw as isize;
                            if ti < 0 || ti >= t_len as isize {
                                continue;
                            }
                            let xs = &x[(ti as usize * f_in + fi as usize) * self.c_in..][..self.c_in];
                            let ws = &self.w[((o * self.kh + ih) * self.kw + iw) * self.c_in..][..self.c_in];
                            acc += xs.iter().zip(ws).map(|(a, b)| a * b).sum::<f32>();
                        }
                    }
                    row[fo * self.c_out + o] = acc;
                }
            }
        });
        (out, f_out)
    }
}

/// Kernel sizes, dilations, padding and stride of one layer.
struct Shape {
    c_in: usize,
    c_out: usize,
    kh: usize,
    kw: usize,
    dh: usize,
    dw: usize,
    ph: usize,
    pw: usize,
    sh: usize,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn f32s(&mut self, n: usize) -> Result<Vec<f32>, String> {
        let b = self.bytes.get(self.at..self.at + 4 * n).ok_or("the pitch model file is truncated")?;
        self.at += 4 * n;
        Ok(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
    }
}

impl PitchNet {
    pub fn from_bytes(bytes: &[u8]) -> Result<PitchNet, String> {
        if bytes.get(..4) != Some(MAGIC) {
            return Err("not a pitch model file".into());
        }
        let version = u32::from_le_bytes(bytes.get(4..8).ok_or("truncated")?.try_into().map_err(|_| "truncated")?);
        if version != 1 {
            return Err(format!("unsupported pitch model version {version}"));
        }
        let mut r = Reader { bytes, at: 8 };
        let spec = [
            Shape { c_in: 5, c_out: 16, kh: 5, kw: 3, dh: 1, dw: 1, ph: 2, pw: 1, sh: 1 },
            Shape { c_in: 16, c_out: 16, kh: 5, kw: 3, dh: 1, dw: 2, ph: 2, pw: 2, sh: 1 },
            Shape { c_in: 16, c_out: 16, kh: 5, kw: 3, dh: 1, dw: 4, ph: 2, pw: 4, sh: 1 },
            Shape { c_in: 16, c_out: 16, kh: 3, kw: 1, dh: 36, dw: 1, ph: 36, pw: 0, sh: 1 },
            Shape { c_in: 16, c_out: 16, kh: 3, kw: 1, dh: 21, dw: 1, ph: 21, pw: 0, sh: 1 },
            Shape { c_in: 16, c_out: 16, kh: 3, kw: 1, dh: 1, dw: 1, ph: 0, pw: 0, sh: 3 },
            Shape { c_in: 16, c_out: 1, kh: 1, kw: 1, dh: 1, dw: 1, ph: 0, pw: 0, sh: 1 },
        ];
        let mut layers = Vec::with_capacity(7);
        for Shape { c_in, c_out, kh, kw, dh, dw, ph, pw, sh } in spec {
            let stored = r.f32s(c_out * c_in * kh * kw)?; // [out][in][kh][kw]
            let mut w = vec![0f32; stored.len()];
            for o in 0..c_out {
                for i in 0..c_in {
                    for h in 0..kh {
                        for k in 0..kw {
                            w[((o * kh + h) * kw + k) * c_in + i] = stored[((o * c_in + i) * kh + h) * kw + k];
                        }
                    }
                }
            }
            layers.push(Conv2 { w, b: r.f32s(c_out)?, c_in, c_out, kh, kw, dh, dw, ph, pw, sh });
        }
        Ok(PitchNet { layers: layers.try_into().map_err(|_| "layer count")? })
    }

    /// Harmonic-stacked features of a 22.05 kHz mono stem: `[t][bin][harmonic]`.
    fn features(y: &[f64]) -> (Vec<f32>, usize) {
        let spec = dsp::power_spectrogram(y, N_FFT, HOP);
        let n = spec.len();
        let df = SR / N_FFT as f64;
        // (bin, harmonic) -> fractional FFT bin
        let pos: Vec<(usize, f32)> = (0..N_BINS * HARMONICS)
            .map(|k| {
                let (b, h) = (k / HARMONICS, k % HARMONICS);
                let centre = 22.667 + b as f64 / 3.0;
                let p = 440.0 * 2f64.powf((centre - 69.0) / 12.0) * (h + 1) as f64 / df;
                (p.floor() as usize, (p - p.floor()) as f32)
            })
            .collect();
        let mut x = vec![0f32; n * N_BINS * HARMONICS];
        x.par_chunks_mut(N_BINS * HARMONICS).zip(spec.par_iter()).for_each(|(row, frame)| {
            let last = frame.len() - 1;
            for (k, v) in row.iter_mut().enumerate() {
                let (lo, fr) = pos[k];
                let (a, b) = (frame[lo.min(last)].sqrt() as f32, frame[(lo + 1).min(last)].sqrt() as f32);
                *v = (1.0 + 30.0 * ((1.0 - fr) * a + fr * b)).ln();
            }
        });
        // each harmonic is normalised over the whole song
        for h in 0..HARMONICS {
            let count = (n * N_BINS) as f64;
            let mean = (0..n * N_BINS).map(|i| x[i * HARMONICS + h] as f64).sum::<f64>() / count.max(1.0);
            let std = ((0..n * N_BINS).map(|i| (x[i * HARMONICS + h] as f64 - mean).powi(2)).sum::<f64>() / count.max(1.0)).sqrt();
            for i in 0..n * N_BINS {
                x[i * HARMONICS + h] = ((x[i * HARMONICS + h] as f64 - mean) / (std + 1e-3)) as f32;
            }
        }
        (x, n)
    }

    /// Runs the layers on frames `a..b`; returns `[t][46]` probabilities.
    fn run_chunk(&self, x: &[f32], a: usize, b: usize) -> Vec<f32> {
        let t_len = b - a;
        let input = &x[a * N_BINS * HARMONICS..b * N_BINS * HARMONICS];
        let relu = |v: &mut Vec<f32>| v.iter_mut().for_each(|e| *e = e.max(0.0));
        let l = &self.layers;
        let (mut h, mut f) = l[0].run(input, t_len, N_BINS);
        relu(&mut h);
        for layer in &l[1..3] {
            let (mut y, _) = layer.run(&h, t_len, f);
            relu(&mut y);
            h.iter_mut().zip(&y).for_each(|(a, b)| *a += b);
        }
        let (mut o, _) = l[3].run(&h, t_len, f);
        let (mut p, _) = l[4].run(&h, t_len, f);
        relu(&mut o);
        relu(&mut p);
        h.iter_mut().zip(o.iter().zip(&p)).for_each(|(a, (b, c))| *a += b + c);
        let (mut pooled, f_out) = l[5].run(&h, t_len, f);
        relu(&mut pooled);
        f = f_out;
        let (logits, _) = l[6].run(&pooled, t_len, f);
        logits.iter().map(|&v| sigmoid(v)).collect()
    }

    pub fn probabilities(&self, y: &[f64]) -> PitchRoll {
        let (x, n) = Self::features(y);
        let mut prob = vec![0f32; n * N_SEMI];
        let mut s = 0;
        while s < n {
            let (a, b) = (s.saturating_sub(HALO), (s + CHUNK + HALO).min(n));
            let out = self.run_chunk(&x, a, b);
            let core = (s + CHUNK).min(n) - s;
            prob[s * N_SEMI..(s + core) * N_SEMI].copy_from_slice(&out[(s - a) * N_SEMI..(s - a + core) * N_SEMI]);
            s += CHUNK;
        }
        PitchRoll { frames: n, prob }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_models_load_and_bad_files_are_refused() {
        for bytes in [&include_bytes!("../models/pitch_net_0.bin")[..], &include_bytes!("../models/pitch_net_1.bin")[..], &include_bytes!("../models/pitch_net_2.bin")[..]] {
            assert!(PitchNet::from_bytes(bytes).is_ok());
        }
        assert!(PitchNet::from_bytes(b"nope").is_err());
        assert!(PitchNet::from_bytes(&include_bytes!("../models/pitch_net_0.bin")[..100]).is_err());
    }

    #[test]
    fn a_silent_stem_gives_a_roll_of_the_right_size() {
        let net = PitchNet::from_bytes(include_bytes!("../models/pitch_net_0.bin")).expect("model");
        let roll = net.probabilities(&vec![0.0; 22050]);
        assert_eq!(roll.frames, 1 + 22050 / 256);
        assert!(roll.mean(0.2, 0.5).iter().all(|p| (0.0..=1.0).contains(p)));
    }

    #[test]
    fn mean_averages_the_frames_of_a_span() {
        let mut roll = PitchRoll { frames: 4, prob: vec![0.0; 4 * N_SEMI] };
        for t in 1..3 {
            roll.prob[t * N_SEMI + 7] = 1.0;
        }
        let m = roll.mean(PitchRoll::FRAME_S, 3.0 * PitchRoll::FRAME_S);
        assert!((m[7] - 1.0).abs() < 1e-6 && m[8] == 0.0);
        let half = roll.mean(0.0, 4.0 * PitchRoll::FRAME_S);
        assert!((half[7] - 0.5).abs() < 1e-6);
    }
}
