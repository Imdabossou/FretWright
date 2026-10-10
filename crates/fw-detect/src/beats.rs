//! Classic beat tracking (port of librosa.beat.beat_track, Ellis 2007:
//! tempo from an autocorrelation tempogram with a log-normal prior, then
//! dynamic programming over the onset envelope) and beat-grid repair.

use fw_project::BeatGrid;

use crate::dsp::{self, TINY};
use crate::onset;

const START_BPM: f64 = 120.0;
const TIGHTNESS: f64 = 100.0;

/// Tempo (BPM) estimate from an onset envelope (librosa.feature.tempo).
pub fn tempo(env: &[f64], sr: f64, hop: usize) -> f64 {
    let win = ((8.0 * sr).floor() as usize) / hop; // time_to_frames(ac_size = 8 s)
    let n = env.len();
    // tempogram: Hann-windowed autocorrelation of each window, centred with a
    // linear ramp to zero at both ends, normalised by its maximum
    let half = win / 2;
    let mut padded = Vec::with_capacity(n + 2 * half);
    let first = env.first().copied().unwrap_or(0.0);
    let last = env.last().copied().unwrap_or(0.0);
    for i in 0..half {
        padded.push(first * i as f64 / half as f64);
    }
    padded.extend_from_slice(env);
    for i in 0..half {
        padded.push(last * (half - i) as f64 / half as f64);
    }
    let window = dsp::hann(win);
    let n_pad = (2 * win - 1).next_power_of_two();
    let fwd = dsp::Fft::new(n_pad);
    let inv = dsp::inverse_plan(n_pad);
    let mut mean = vec![0.0; win];
    let frames = dsp::frame_count(padded.len(), win, 1).min(n);
    for t in 0..frames {
        let frame: Vec<f64> = padded[t..t + win].iter().zip(&window).map(|(a, w)| a * w).collect();
        let ac = dsp::autocorrelate(&frame, win, &fwd, &inv);
        let max = ac.iter().map(|v| v.abs()).fold(0.0, f64::max);
        let scale = if max < TINY { 1.0 } else { 1.0 / max };
        for (m, v) in mean.iter_mut().zip(&ac) {
            *m += v * scale;
        }
    }
    mean.iter_mut().for_each(|m| *m /= frames.max(1) as f64);

    let bpms: Vec<f64> = (0..win).map(|i| if i == 0 { f64::INFINITY } else { 60.0 * sr / (hop as f64 * i as f64) }).collect();
    let max_idx = bpms.iter().position(|&b| b < 320.0).unwrap_or(0);
    let mut best = (f64::NEG_INFINITY, 0usize);
    for (i, (&tg, &bpm)) in mean.iter().zip(&bpms).enumerate() {
        let logprior = if i < max_idx { f64::NEG_INFINITY } else { -0.5 * (bpm.log2() - START_BPM.log2()).powi(2) };
        let score = (1e6 * tg).ln_1p() + logprior;
        if score > best.0 {
            best = (score, i);
        }
    }
    bpms[best.1]
}

/// Beat frames (librosa.beat.beat_track with its defaults, trim=True).
pub fn beat_track(env: &[f64], sr: f64, hop: usize) -> (f64, Vec<usize>) {
    if !env.iter().any(|&v| v != 0.0) {
        return (0.0, Vec::new());
    }
    let bpm = tempo(env, sr, hop);
    let frame_rate = sr / hop as f64;
    let fpb = (frame_rate * 60.0 / bpm).round_ties_even();
    let n = env.len();

    // normalise by the standard deviation (ddof = 1)
    let mean = env.iter().sum::<f64>() / n as f64;
    let std = (env.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt();
    let norm: Vec<f64> = env.iter().map(|v| v / (std + TINY)).collect();

    // local score: convolution with a Gaussian of width ~1/32 beat
    let k_half = fpb as i64;
    let window: Vec<f64> = (-k_half..=k_half).map(|i| (-0.5 * (i as f64 * 32.0 / fpb).powi(2)).exp()).collect();
    let k = window.len();
    let local: Vec<f64> = (0..n)
        .map(|i| {
            let lo = (i + k / 2 + 1).saturating_sub(n);
            let hi = (i + k / 2).min(k);
            (lo..hi).map(|j| window[j] * norm[i + k / 2 - j]).sum()
        })
        .collect();

    // dynamic programming
    let score_thresh = 0.01 * local.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut backlink = vec![-1i64; n];
    let mut cum = vec![0.0; n];
    let mut first_beat = true;
    let half_beat = (fpb / 2.0).round_ties_even() as i64;
    for i in 0..n {
        let (mut best, mut loc_best) = (f64::NEG_INFINITY, -1i64);
        let start = i as i64 - half_beat;
        let stop = (i as f64 - 2.0 * fpb - 1.0) as i64;
        let mut loc = start;
        while loc > stop {
            if loc < 0 {
                break;
            }
            let s = cum[loc as usize] - TIGHTNESS * (((i as i64 - loc) as f64).ln() - fpb.ln()).powi(2);
            if s > best {
                best = s;
                loc_best = loc;
            }
            loc -= 1;
        }
        cum[i] = if loc_best >= 0 { local[i] + best } else { local[i] };
        if first_beat && local[i] < score_thresh {
            backlink[i] = -1;
        } else {
            backlink[i] = loc_best;
            first_beat = false;
        }
    }

    // last beat: the latest local maximum of the cumulative score above half
    // the median of all local maxima
    let is_max: Vec<bool> = (0..n)
        .map(|i| {
            if i == 0 {
                false
            } else if i == n - 1 {
                cum[i] > cum[i - 1]
            } else {
                cum[i] > cum[i - 1] && cum[i] >= cum[i + 1]
            }
        })
        .collect();
    let mut maxima: Vec<f64> = (0..n).filter(|&i| is_max[i]).map(|i| cum[i]).collect();
    maxima.sort_by(|a, b| a.total_cmp(b));
    let median = match maxima.len() {
        0 => 0.0,
        m if m % 2 == 1 => maxima[m / 2],
        m => (maxima[m / 2 - 1] + maxima[m / 2]) / 2.0,
    };
    let threshold = 0.5 * median;
    let mut tail = n - 1;
    for i in (0..n).rev() {
        if is_max[i] && cum[i] >= threshold {
            tail = i;
            break;
        }
    }
    let mut beats = vec![false; n];
    let mut b = tail as i64;
    while b >= 0 {
        beats[b as usize] = true;
        b = backlink[b as usize];
    }

    // trim weak beats at both ends
    let beat_scores: Vec<f64> = (0..n).filter(|&i| beats[i]).map(|i| local[i]).collect();
    let w = [0.0, 0.5, 1.0, 0.5, 0.0];
    let full: Vec<f64> = (0..beat_scores.len() + 4)
        .map(|i| (0..5).filter(|&j| i >= j && i - j < beat_scores.len()).map(|j| w[j] * beat_scores[i - j]).sum())
        .collect();
    let smooth = &full[2..(n + 2).min(full.len())];
    let threshold = 0.5 * (smooth.iter().map(|v| v * v).sum::<f64>() / smooth.len().max(1) as f64).sqrt();
    let mut i = 0;
    while i < n && local[i] <= threshold {
        beats[i] = false;
        i += 1;
    }
    let mut i = n as i64 - 1;
    while i >= 0 && local[i as usize] <= threshold {
        beats[i as usize] = false;
        i -= 1;
    }
    (bpm, (0..n).filter(|&i| beats[i]).collect())
}

/// Beat times (s) of a full mix with the classic tracker (22.05 kHz, hop 512).
pub fn track_librosa(y22050: &[f64]) -> (f64, Vec<f64>) {
    let env = onset::onset_strength(y22050, 22050.0, 512, true);
    let (bpm, frames) = beat_track(&env, 22050.0, 512);
    (bpm, frames.into_iter().map(|f| f as f64 * 512.0 / 22050.0).collect())
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    match s.len() {
        0 => 0.0,
        n if n % 2 == 1 => s[n / 2],
        n => (s[n / 2 - 1] + s[n / 2]) / 2.0,
    }
}

/// Turns raw tracker output into a complete, monotonic grid covering the song:
/// fills skipped beats, drops doubled ones, extends to 0 s and past the end
/// with the local beat length, and lays out regular bar lines using the
/// dominant meter and the phase that agrees with most tracked downbeats.
pub fn regularize(beats: &[f64], downbeats: &[f64], duration: f64, default_meter: usize) -> BeatGrid {
    let mut beats = beats.to_vec();
    beats.sort_by(|a, b| a.total_cmp(b));
    if beats.len() < 4 {
        let period = 0.5;
        let grid: Vec<f64> = (0..).map(|i| i as f64 * period).take_while(|&t| t < duration + period).collect();
        let downbeats = (0..grid.len()).step_by(default_meter).collect();
        return BeatGrid { beats: grid, downbeats, tempo: 60.0 / period };
    }
    let diffs: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    let period = median(&diffs);
    let mut fixed = vec![beats[0]];
    for &b in &beats[1..] {
        let last = *fixed.last().unwrap_or(&b);
        let gap = b - last;
        if gap < 0.55 * period {
            continue; // doubled or spurious beat
        }
        let n = (gap / period).round_ties_even() as usize;
        if n >= 2 {
            fixed.extend((1..n).map(|k| last + gap * k as f64 / n as f64));
        }
        fixed.push(b);
    }
    let head = median(&fixed.windows(2).take(8).map(|w| w[1] - w[0]).collect::<Vec<_>>());
    while fixed[0] - head >= 0.0 {
        fixed.insert(0, fixed[0] - head);
    }
    let tail_diffs: Vec<f64> = fixed.windows(2).rev().take(8).map(|w| w[1] - w[0]).collect();
    let tail = median(&tail_diffs);
    while *fixed.last().unwrap_or(&0.0) < duration + tail {
        let l = *fixed.last().unwrap_or(&0.0);
        fixed.push(l + tail);
    }

    let nearest = |d: f64| fixed.iter().enumerate().fold(0, |best, (i, &v)| if (v - d).abs() < (fixed[best] - d).abs() { i } else { best });
    let mut db_idx: Vec<usize> =
        downbeats.iter().map(|&d| (nearest(d), d)).filter(|&(i, d)| (fixed[i] - d).abs() < 0.4 * period).map(|(i, _)| i).collect();
    db_idx.sort_unstable();
    db_idx.dedup();
    let (mut meter, mut phase) = (default_meter, 0);
    if db_idx.len() >= 2 {
        let mut counts = std::collections::BTreeMap::new();
        for w in db_idx.windows(2) {
            *counts.entry(w[1] - w[0]).or_insert(0usize) += 1;
        }
        // most common spacing (smallest on ties, like numpy argmax of bincount)
        let m = counts.iter().fold((0, 0), |best, (&k, &c)| if c > best.1 { (k, c) } else { best }).0;
        meter = if (2..=7).contains(&m) { m } else { default_meter };
        let mut phases = vec![0usize; meter];
        for &i in &db_idx {
            phases[i % meter] += 1;
        }
        phase = (0..meter).fold(0, |best, p| if phases[p] > phases[best] { p } else { best });
    }
    let downbeats = (phase..fixed.len()).step_by(meter).collect();
    let tempo = 60.0 / median(&fixed.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
    let beats = fixed.iter().map(|&b| (b * 1e4).round_ties_even() / 1e4).collect();
    BeatGrid { beats, downbeats, tempo }
}

#[cfg(test)]
mod tests {
    #[test]
    fn regularize_fills_gaps_in_order() {
        let raw = [0.0, 0.5, 1.0, 3.0, 3.5, 4.0, 4.5, 5.0]; // three beats missing
        let g = super::regularize(&raw, &[0.0], 5.0, 4);
        assert!(g.beats.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(&g.beats[..8], &[0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]);
    }
}
