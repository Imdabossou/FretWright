//! A small neural network that says, for every 5.8 ms of the bass stem, how
//! likely a note starts there. Trained on official Rocksmith charts (see
//! `tools/ml`), run here in plain Rust so it needs no extra runtime.
//!
//! Input: 136 log-spectrogram bands per frame (96 low bands from a 2048-point
//! STFT, 40 higher bands from a 512-point one), each normalised over the song.
//! Layers: a convolution, four dilated residual convolutions, a bidirectional
//! GRU and a linear output.

use rayon::prelude::*;

use crate::dsp;

const HOP: usize = 128;
const MAGIC: &[u8; 4] = b"FWON";

#[derive(Debug, Clone)]
struct Conv {
    /// `[out][tap][in]`
    w: Vec<f32>,
    b: Vec<f32>,
    n_in: usize,
    n_out: usize,
    taps: usize,
    dilation: usize,
}

#[derive(Debug, Clone)]
struct GruDir {
    w_ih: Vec<f32>, // 3H x in, rows r | z | n
    w_hh: Vec<f32>, // 3H x H
    b_ih: Vec<f32>,
    b_hh: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct OnsetNet {
    input: Conv,
    blocks: Vec<Conv>,
    fwd: GruDir,
    bwd: GruDir,
    hidden: usize,
    out_w: Vec<f32>,
    out_b: f32,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.bytes.get(self.at..self.at + 4).ok_or("the onset model file is truncated")?;
        self.at += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32s(&mut self, n: usize) -> Result<Vec<f32>, String> {
        let b = self.bytes.get(self.at..self.at + 4 * n).ok_or("the onset model file is truncated")?;
        self.at += 4 * n;
        Ok(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
    }
}

/// `[out][in][tap]` (as stored) to `[out][tap][in]`.
fn reorder(w: Vec<f32>, n_out: usize, n_in: usize, taps: usize) -> Vec<f32> {
    let mut r = vec![0.0; w.len()];
    for o in 0..n_out {
        for i in 0..n_in {
            for t in 0..taps {
                r[(o * taps + t) * n_in + i] = w[(o * n_in + i) * taps + t];
            }
        }
    }
    r
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

impl OnsetNet {
    pub fn from_bytes(bytes: &[u8]) -> Result<OnsetNet, String> {
        if bytes.get(..4) != Some(MAGIC) {
            return Err("not an onset model file".into());
        }
        let mut r = Reader { bytes, at: 4 };
        let version = r.u32()?;
        if version != 1 {
            return Err(format!("unsupported onset model version {version}"));
        }
        let (n_in, ch, hidden, n_blocks) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        let dilations: Vec<usize> = (0..n_blocks).map(|_| r.u32().map(|d| d as usize)).collect::<Result<_, _>>()?;
        let conv = |r: &mut Reader, n_in: usize, n_out: usize, dilation: usize| -> Result<Conv, String> {
            let w = reorder(r.f32s(n_out * n_in * 5)?, n_out, n_in, 5);
            Ok(Conv { w, b: r.f32s(n_out)?, n_in, n_out, taps: 5, dilation })
        };
        let input = conv(&mut r, n_in, ch, 1)?;
        let blocks = dilations.iter().map(|&d| conv(&mut r, ch, ch, d)).collect::<Result<Vec<_>, _>>()?;
        let gru = |r: &mut Reader| -> Result<GruDir, String> {
            Ok(GruDir { w_ih: r.f32s(3 * hidden * ch)?, w_hh: r.f32s(3 * hidden * hidden)?, b_ih: r.f32s(3 * hidden)?, b_hh: r.f32s(3 * hidden)? })
        };
        let fwd = gru(&mut r)?;
        let bwd = gru(&mut r)?;
        let out_w = r.f32s(2 * hidden)?;
        let out_b = r.f32s(1)?[0];
        Ok(OnsetNet { input, blocks, fwd, bwd, hidden, out_w, out_b })
    }

    /// Spectrogram features of a 22.05 kHz mono stem: `frames x 136`, row-major.
    pub fn features(y: &[f64]) -> (Vec<f32>, usize) {
        const SR: f64 = 22050.0;
        let bands = |n_fft: usize, fmin: f64, fmax: f64, n_bands: usize| -> Vec<Vec<f32>> {
            let spec = dsp::power_spectrogram(y, n_fft, HOP);
            let freqs: Vec<f64> = (0..=n_fft / 2).map(|k| k as f64 * SR / n_fft as f64).collect();
            let edges: Vec<f64> = (0..n_bands + 2).map(|i| (fmin.ln() + (fmax.ln() - fmin.ln()) * i as f64 / (n_bands + 1) as f64).exp()).collect();
            let filters: Vec<Vec<f64>> = (0..n_bands)
                .map(|i| {
                    let (lo, c, hi) = (edges[i], edges[i + 1], edges[i + 2]);
                    let mut f: Vec<f64> = freqs.iter().map(|&x| (((x - lo) / (c - lo)).min((hi - x) / (hi - c))).max(0.0)).collect();
                    let sum = f.iter().sum::<f64>().max(1e-9);
                    f.iter_mut().for_each(|v| *v /= sum);
                    f
                })
                .collect();
            spec.par_iter()
                .map(|frame| filters.iter().map(|f| f.iter().zip(frame).map(|(w, p)| w * p.sqrt()).sum::<f64>() as f32).collect())
                .collect()
        };
        let low = bands(2048, 30.0, 2500.0, 96);
        let high = bands(512, 150.0, 6000.0, 40);
        let n = low.len().min(high.len());
        let mut x = vec![0f32; n * 136];
        for t in 0..n {
            for (i, v) in low[t].iter().chain(&high[t]).enumerate() {
                x[t * 136 + i] = (1.0 + 30.0 * *v).ln();
            }
        }
        // each band is normalised over the song
        for i in 0..136 {
            let mean = (0..n).map(|t| x[t * 136 + i] as f64).sum::<f64>() / n.max(1) as f64;
            let std = ((0..n).map(|t| (x[t * 136 + i] as f64 - mean).powi(2)).sum::<f64>() / n.max(1) as f64).sqrt();
            for t in 0..n {
                x[t * 136 + i] = ((x[t * 136 + i] as f64 - mean) / (std + 1e-3)) as f32;
            }
        }
        (x, n)
    }

    fn conv(c: &Conv, x: &[f32], n: usize, relu_residual: Option<&[f32]>) -> Vec<f32> {
        let pad = (c.dilation * (c.taps - 1) / 2) as isize;
        let mut out = vec![0f32; n * c.n_out];
        out.par_chunks_mut(c.n_out).enumerate().for_each(|(t, row)| {
            for o in 0..c.n_out {
                let mut acc = c.b[o];
                for j in 0..c.taps {
                    let tt = t as isize + (j * c.dilation) as isize - pad;
                    if tt < 0 || tt >= n as isize {
                        continue;
                    }
                    let xr = &x[tt as usize * c.n_in..(tt as usize + 1) * c.n_in];
                    let wr = &c.w[(o * c.taps + j) * c.n_in..(o * c.taps + j + 1) * c.n_in];
                    acc += xr.iter().zip(wr).map(|(a, b)| a * b).sum::<f32>();
                }
                row[o] = match relu_residual {
                    Some(res) => res[t * c.n_out + o] + acc.max(0.0),
                    None => acc.max(0.0),
                };
            }
        });
        out
    }

    /// One direction of the GRU over `x` (`n x in`), returning `n x hidden`.
    fn gru(&self, g: &GruDir, x: &[f32], n: usize, n_in: usize, reverse: bool) -> Vec<f32> {
        let h = self.hidden;
        // input projections for every frame
        let gi: Vec<f32> = (0..n)
            .into_par_iter()
            .flat_map_iter(|t| {
                let xr = &x[t * n_in..(t + 1) * n_in];
                (0..3 * h).map(move |r| g.b_ih[r] + g.w_ih[r * n_in..(r + 1) * n_in].iter().zip(xr).map(|(a, b)| a * b).sum::<f32>())
            })
            .collect();
        let mut out = vec![0f32; n * h];
        let mut state = vec![0f32; h];
        let mut gh = vec![0f32; 3 * h];
        let order: Box<dyn Iterator<Item = usize>> = if reverse { Box::new((0..n).rev()) } else { Box::new(0..n) };
        for t in order {
            for (r, v) in gh.iter_mut().enumerate() {
                *v = g.b_hh[r] + g.w_hh[r * h..(r + 1) * h].iter().zip(&state).map(|(a, b)| a * b).sum::<f32>();
            }
            let gi_t = &gi[t * 3 * h..(t + 1) * 3 * h];
            for k in 0..h {
                let r = sigmoid(gi_t[k] + gh[k]);
                let z = sigmoid(gi_t[h + k] + gh[h + k]);
                let cand = (gi_t[2 * h + k] + r * gh[2 * h + k]).tanh();
                state[k] = (1.0 - z) * cand + z * state[k];
            }
            out[t * h..(t + 1) * h].copy_from_slice(&state);
        }
        out
    }

    /// Probability that a note starts in each frame (frame `i` is at `i * 128 / 22050` s).
    pub fn probabilities(&self, y: &[f64]) -> Vec<f32> {
        let (x, n) = Self::features(y);
        if n == 0 {
            return Vec::new();
        }
        let mut h = Self::conv(&self.input, &x, n, None);
        for b in &self.blocks {
            h = Self::conv(b, &h, n, Some(&h));
        }
        let ch = self.input.n_out;
        let (f, b) = rayon::join(|| self.gru(&self.fwd, &h, n, ch, false), || self.gru(&self.bwd, &h, n, ch, true));
        let hs = self.hidden;
        (0..n)
            .map(|t| {
                let z: f32 = self.out_b + (0..hs).map(|k| self.out_w[k] * f[t * hs + k] + self.out_w[hs + k] * b[t * hs + k]).sum::<f32>();
                sigmoid(z)
            })
            .collect()
    }
}

/// Note starts (seconds) from the frame probabilities: local maxima at least
/// `threshold` high and at least 58 ms apart.
pub fn pick_onsets(prob: &[f32], threshold: f32) -> Vec<f64> {
    const HALF: usize = 5;
    const MIN_GAP: usize = 10;
    let mut out: Vec<usize> = Vec::new();
    for i in 0..prob.len() {
        if prob[i] < threshold {
            continue;
        }
        let (a, b) = (i.saturating_sub(HALF), (i + HALF + 1).min(prob.len()));
        let window = &prob[a..b];
        let first_max = window.iter().enumerate().fold((0, f32::MIN), |m, (k, &v)| if v > m.1 { (k, v) } else { m }).0 + a;
        if first_max != i {
            continue;
        }
        match out.last_mut() {
            Some(last) if i - *last < MIN_GAP => {
                if prob[i] > prob[*last] {
                    *last = i;
                }
            }
            _ => out.push(i),
        }
    }
    out.into_iter().map(|i| i as f64 * HOP as f64 / 22050.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_peaks_above_the_threshold_and_keeps_them_apart() {
        let mut p = vec![0.0f32; 100];
        p[10] = 0.9;
        p[12] = 0.95; // too close to frame 10: only the higher survives
        p[40] = 0.6;
        p[70] = 0.3; // below the threshold
        let hop = 128.0 / 22050.0;
        let got = pick_onsets(&p, 0.5);
        assert_eq!(got.len(), 2);
        assert!((got[0] - 12.0 * hop).abs() < 1e-9 && (got[1] - 40.0 * hop).abs() < 1e-9);
    }

    #[test]
    fn shipped_models_load_and_run_on_silence() {
        for bytes in [&include_bytes!("../models/onset_net_0.bin")[..], &include_bytes!("../models/onset_net_1.bin")[..], &include_bytes!("../models/onset_net_2.bin")[..]] {
            let net = OnsetNet::from_bytes(bytes).expect("model");
            let prob = net.probabilities(&vec![0.0; 22050]);
            assert_eq!(prob.len(), 1 + 22050 / 128);
            assert!(prob.iter().all(|p| (0.0..=1.0).contains(p)));
        }
        assert!(OnsetNet::from_bytes(b"nope").is_err());
    }
}
