//! Measurements that say whether an onset candidate is a real note start,
//! for a classifier trained on charted songs.

use crate::dsp;
use crate::pitch::Analyzer;

pub const N_FEATURES: usize = 10;
pub const FEATURE_NAMES: [&str; N_FEATURES] =
    ["margin", "energy_change_db", "level_db", "pitch_changed", "min_conf", "spectral_change", "gap_prev", "gap_next", "conf_after", "attack_db"];

const ENV_WIN: usize = 512;
const ENV_HOP: usize = 128;
/// Length of the stretches compared before and after a candidate.
const SPAN: f64 = 0.08;

pub struct Measurer<'a> {
    env: Vec<f64>,
    hop_t: f64,
    loudest: f64,
    analyzer: &'a Analyzer<'a>,
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.max(1e-6).log10()
}

impl<'a> Measurer<'a> {
    pub fn new(bass: &[f64], sr: f64, analyzer: &'a Analyzer<'a>) -> Measurer<'a> {
        let env = dsp::rms(bass, ENV_WIN, ENV_HOP);
        let loudest = env.iter().copied().fold(1e-9, f64::max);
        Measurer { env, hop_t: ENV_HOP as f64 / sr, loudest, analyzer }
    }

    fn mean_env(&self, from: f64, to: f64) -> f64 {
        let (a, b) = (((from / self.hop_t).max(0.0)) as usize, ((to / self.hop_t) as usize).min(self.env.len()));
        if b <= a { 0.0 } else { self.env[a..b].iter().sum::<f64>() / (b - a) as f64 }
    }

    fn max_env(&self, from: f64, to: f64) -> f64 {
        let (a, b) = (((from / self.hop_t).max(0.0)) as usize, ((to / self.hop_t) as usize).min(self.env.len()));
        if b <= a { 0.0 } else { self.env[a..b].iter().copied().fold(0.0, f64::max) }
    }

    /// Features of the candidate at `t` that rose `margin`, with the gaps (s) to the neighbouring candidates.
    pub fn features(&self, t: f64, margin: f64, gap_prev: f64, gap_next: f64) -> [f64; N_FEATURES] {
        let (before, after) = (self.mean_env(t - SPAN, t - 0.01), self.mean_env(t + 0.01, t + SPAN));
        let peak_after = self.max_env(t, t + 0.06);
        let peak_before = self.max_env(t - 0.06, t);
        let a = self.analyzer.salience_vector(t - SPAN, t - 0.01, 0.0);
        let b = self.analyzer.salience_vector(t + 0.01, t + SPAN, 0.0);
        let argmax = |v: &Vec<f64>| v.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).map_or(0, |x| x.0);
        let conf = |v: &Vec<f64>| {
            let mean = v.iter().sum::<f64>() / v.len().max(1) as f64;
            if mean > 0.0 { (v[argmax(v)] / mean).clamp(1.0, 10.0) } else { 1.0 }
        };
        let (mut changed, mut min_conf, mut spectral, mut conf_after) = (0.0, 1.0, 0.0, 1.0);
        if let (Some(a), Some(b)) = (&a, &b) {
            changed = f64::from(argmax(a) != argmax(b));
            min_conf = conf(a).min(conf(b));
            conf_after = conf(b);
            let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
            let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(b) {
                sab += (x - ma) * (y - mb);
                saa += (x - ma) * (x - ma);
                sbb += (y - mb) * (y - mb);
            }
            spectral = 1.0 - sab / (saa * sbb).sqrt().max(1e-12);
        } else if let Some(b) = &b {
            conf_after = conf(b);
            min_conf = 1.0;
            changed = 1.0;
            spectral = 1.0;
        }
        [
            margin,
            db(after / before.max(1e-9)).clamp(-24.0, 24.0),
            db(after / self.loudest).clamp(-60.0, 0.0),
            changed,
            min_conf,
            spectral.clamp(0.0, 1.5),
            gap_prev.max(0.01).log10().clamp(-2.5, 0.5),
            gap_next.max(0.01).log10().clamp(-2.5, 0.5),
            conf_after,
            db(peak_after / peak_before.max(1e-9)).clamp(-24.0, 24.0),
        ]
    }
}
