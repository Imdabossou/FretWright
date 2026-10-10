//! DSP building blocks with librosa/scipy semantics (framing, windows,
//! spectrograms, mel filters, autocorrelation, filtering).

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

/// numpy's tiny for float64 (smallest positive normal number).
pub const TINY: f64 = f64::MIN_POSITIVE;

/// Pads both ends with `n` zeros (numpy pad mode "constant").
pub fn pad_zeros(y: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![0.0; y.len() + 2 * n];
    out[n..n + y.len()].copy_from_slice(y);
    out
}

/// Number of frames librosa's `util.frame` produces.
pub fn frame_count(len: usize, frame_length: usize, hop: usize) -> usize {
    if len < frame_length { 0 } else { 1 + (len - frame_length) / hop }
}

/// Periodic Hann window (scipy get_window("hann", n, fftbins=True)).
pub fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect()
}

/// Symmetric triangular window (scipy.signal.windows.triang).
pub fn triang(m: usize) -> Vec<f64> {
    let half = m.div_ceil(2);
    let w: Vec<f64> = (1..=half)
        .map(|n| if m.is_multiple_of(2) { (2.0 * n as f64 - 1.0) / m as f64 } else { 2.0 * n as f64 / (m as f64 + 1.0) })
        .collect();
    let mut out = w.clone();
    if m.is_multiple_of(2) {
        out.extend(w.iter().rev());
    } else {
        out.extend(w.iter().rev().skip(1));
    }
    out
}

pub struct Fft {
    n: usize,
    plan: Arc<dyn RealToComplex<f64>>,
}

impl Fft {
    pub fn new(n: usize) -> Fft {
        Fft { n, plan: RealFftPlanner::<f64>::new().plan_fft_forward(n) }
    }

    /// |rfft(x zero-padded to n)|^2
    pub fn power(&self, x: &[f64]) -> Vec<f64> {
        let mut input = vec![0.0; self.n];
        input[..x.len().min(self.n)].copy_from_slice(&x[..x.len().min(self.n)]);
        let mut spec = self.plan.make_output_vec();
        // the input length always matches the plan
        self.plan.process(&mut input, &mut spec).expect("fft length matches plan");
        spec.iter().map(|c| c.norm_sqr()).collect()
    }
}

/// Autocorrelation of each frame up to lag `max_size` (librosa.autocorrelate).
pub fn autocorrelate(frame: &[f64], max_size: usize, fwd: &Fft, inv: &Arc<dyn realfft::ComplexToReal<f64>>) -> Vec<f64> {
    let mut input = vec![0.0; fwd.n];
    input[..frame.len()].copy_from_slice(frame);
    let mut spec = fwd.plan.make_output_vec();
    fwd.plan.process(&mut input, &mut spec).expect("fft length matches plan");
    for c in spec.iter_mut() {
        *c = realfft::num_complex::Complex::new(c.norm_sqr(), 0.0);
    }
    let mut out = vec![0.0; fwd.n];
    inv.process(&mut spec, &mut out).expect("ifft length matches plan");
    let scale = 1.0 / fwd.n as f64;
    out.truncate(max_size.min(frame.len()));
    out.iter_mut().for_each(|v| *v *= scale);
    out
}

pub fn inverse_plan(n: usize) -> Arc<dyn realfft::ComplexToReal<f64>> {
    RealFftPlanner::<f64>::new().plan_fft_inverse(n)
}

/// Power spectrogram |STFT|^2 with a centred, zero-padded Hann window.
/// Returns frames x bins.
pub fn power_spectrogram(y: &[f64], n_fft: usize, hop: usize) -> Vec<Vec<f64>> {
    let padded = pad_zeros(y, n_fft / 2);
    let window = hann(n_fft);
    let fft = Fft::new(n_fft);
    let n = frame_count(padded.len(), n_fft, hop);
    (0..n)
        .map(|f| {
            let frame: Vec<f64> = padded[f * hop..f * hop + n_fft].iter().zip(&window).map(|(x, w)| x * w).collect();
            fft.power(&frame)
        })
        .collect()
}

fn hz_to_mel(f: f64) -> f64 {
    // Slaney scale: linear below 1 kHz, logarithmic above
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp }
}

fn mel_to_hz(m: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { f_sp * m }
}

/// librosa.filters.mel with Slaney scale and normalisation (n_mels x bins).
pub fn mel_filters(sr: f64, n_fft: usize, n_mels: usize, fmin: f64, fmax: f64) -> Vec<Vec<f64>> {
    let bins = n_fft / 2 + 1;
    let fftfreqs: Vec<f64> = (0..bins).map(|i| i as f64 * sr / n_fft as f64).collect();
    let (lo, hi) = (hz_to_mel(fmin), hz_to_mel(fmax));
    let mel_f: Vec<f64> = (0..n_mels + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64)).collect();
    let fdiff: Vec<f64> = mel_f.windows(2).map(|w| w[1] - w[0]).collect();
    (0..n_mels)
        .map(|m| {
            let enorm = 2.0 / (mel_f[m + 2] - mel_f[m]);
            fftfreqs
                .iter()
                .map(|&f| {
                    let lower = (f - mel_f[m]) / fdiff[m];
                    let upper = (mel_f[m + 2] - f) / fdiff[m + 1];
                    lower.min(upper).max(0.0) * enorm
                })
                .collect()
        })
        .collect()
}

/// RMS energy per frame (centred, zero padded), librosa.feature.rms.
pub fn rms(y: &[f64], frame_length: usize, hop: usize) -> Vec<f64> {
    let padded = pad_zeros(y, frame_length / 2);
    let n = frame_count(padded.len(), frame_length, hop);
    (0..n)
        .map(|f| {
            let frame = &padded[f * hop..f * hop + frame_length];
            (frame.iter().map(|x| x * x).sum::<f64>() / frame_length as f64).sqrt()
        })
        .collect()
}

/// scipy.ndimage.median_filter with mode="nearest" on integers.
pub fn median_filter_nearest(x: &[i32], size: usize) -> Vec<i32> {
    let half = size / 2;
    let n = x.len() as isize;
    (0..x.len())
        .map(|i| {
            let mut w: Vec<i32> = (0..size)
                .map(|k| x[(i as isize + k as isize - half as isize).clamp(0, n - 1) as usize])
                .collect();
            w.sort_unstable();
            w[size / 2]
        })
        .collect()
}

// ------------------------------------------------------------------ Butterworth low-pass

/// Second-order sections [b0 b1 b2 a0 a1 a2] of a digital Butterworth low-pass
/// (scipy.signal.butter(order, cutoff, fs=fs, output="sos"), even orders).
pub fn butter_lowpass_sos(order: usize, cutoff: f64, fs: f64) -> Vec<[f64; 6]> {
    use std::f64::consts::PI;
    // pre-warp, analog prototype poles, bilinear transform (fs2 = 2*fs, scipy's convention)
    let fs2 = 2.0;
    let warped = 2.0 * fs2 * (PI * (cutoff / (fs / 2.0)) / fs2).tan();
    let mut sections = Vec::new();
    // poles come in conjugate pairs; scipy orders sections from the pole closest
    // to the unit circle last, so build pairs from the outermost angle inwards
    for k in (0..order / 2).rev() {
        let theta = PI * (2.0 * k as f64 + 1.0 + order as f64) / (2.0 * order as f64);
        let (pr, pi) = (warped * theta.cos(), warped * theta.sin());
        // bilinear: z = (fs2*2 + p) / (fs2*2 - p)
        let denom_re = 2.0 * fs2 - pr;
        let denom_im = -pi;
        let num_re = 2.0 * fs2 + pr;
        let num_im = pi;
        let d2 = denom_re * denom_re + denom_im * denom_im;
        let zr = (num_re * denom_re + num_im * denom_im) / d2;
        let zi = (num_im * denom_re - num_re * denom_im) / d2;
        // section with two zeros at z=-1 and poles z, conj(z)
        let a1 = -2.0 * zr;
        let a2 = zr * zr + zi * zi;
        sections.push([1.0, 2.0, 1.0, 1.0, a1, a2]);
    }
    // overall gain so the DC response is 1, put in the first section (like scipy)
    let mut dc = 1.0;
    for s in &sections {
        dc *= (s[0] + s[1] + s[2]) / (s[3] + s[4] + s[5]);
    }
    let k = 1.0 / dc;
    for v in &mut sections[0][..3] {
        *v *= k;
    }
    sections
}

fn sosfilt(sos: &[[f64; 6]], x: &[f64], zi: &mut [[f64; 2]]) -> Vec<f64> {
    let mut y = x.to_vec();
    for (s, z) in sos.iter().zip(zi.iter_mut()) {
        let [b0, b1, b2, _, a1, a2] = *s;
        for v in y.iter_mut() {
            let xn = *v;
            let out = b0 * xn + z[0];
            z[0] = b1 * xn - a1 * out + z[1];
            z[1] = b2 * xn - a2 * out;
            *v = out;
        }
    }
    y
}

/// Steady-state initial conditions for each section (scipy.signal.sosfilt_zi):
/// the transposed direct-form II state a section settles in for a constant
/// input, scaled by the DC gain of the sections before it.
fn sosfilt_zi(sos: &[[f64; 6]]) -> Vec<[f64; 2]> {
    let mut scale = 1.0;
    sos.iter()
        .map(|s| {
            let [b0, b1, b2, _, a1, a2] = *s;
            let y = (b0 + b1 + b2) / (1.0 + a1 + a2);
            let z1 = b2 - a2 * y;
            let z0 = b1 - a1 * y + z1;
            let zi = [scale * z0, scale * z1];
            scale *= y;
            zi
        })
        .collect()
}

/// Zero-phase filtering with odd-extension padding (scipy.signal.sosfiltfilt).
pub fn sosfiltfilt(sos: &[[f64; 6]], x: &[f64]) -> Vec<f64> {
    let ntaps = 2 * sos.len() + 1;
    let padlen = (3 * ntaps).min(x.len().saturating_sub(1));
    if x.is_empty() {
        return Vec::new();
    }
    let n = x.len();
    // odd extension: 2*x[0] - x[padlen..1], x, 2*x[-1] - x[-2..-padlen-1]
    let mut ext = Vec::with_capacity(n + 2 * padlen);
    for i in (1..=padlen).rev() {
        ext.push(2.0 * x[0] - x[i]);
    }
    ext.extend_from_slice(x);
    for i in 1..=padlen {
        ext.push(2.0 * x[n - 1] - x[n - 1 - i]);
    }
    let zi = sosfilt_zi(sos);
    let mut z: Vec<[f64; 2]> = zi.iter().map(|z| [z[0] * ext[0], z[1] * ext[0]]).collect();
    let mut y = sosfilt(sos, &ext, &mut z);
    y.reverse();
    let mut z: Vec<[f64; 2]> = zi.iter().map(|z| [z[0] * y[0], z[1] * y[0]]).collect();
    let mut y = sosfilt(sos, &y, &mut z);
    y.reverse();
    y[padlen..padlen + n].to_vec()
}
