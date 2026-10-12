//! Compares onset-strength functions on a bass stem against the onsets of an
//! official chart: `onset_lab <stem.wav> <chart.xml>`.

use fw_detect::dsp;
use fw_detect::pipeline::{SR, load_mono};

struct Variant {
    name: &'static str,
    n_fft: usize,
    hop: usize,
    fmin: f64,
    fmax: f64,
    /// log(1 + gamma * magnitude) compression; 0 = plain log power in dB
    gamma: f64,
    /// SuperFlux maximum filter width over bins (1 = off)
    max_filter: usize,
    /// subtract a short moving average of the whole function (adaptive threshold)
    lag: usize,
}

fn strength(y: &[f64], v: &Variant) -> Vec<f64> {
    let spec = dsp::power_spectrogram(y, v.n_fft, v.hop);
    let bin = |f: f64| ((f * v.n_fft as f64 / SR).round() as usize).clamp(1, v.n_fft / 2);
    let (lo, hi) = (bin(v.fmin), bin(v.fmax));
    let comp: Vec<Vec<f64>> = spec
        .iter()
        .map(|fr| {
            fr[lo..=hi]
                .iter()
                .map(|p| if v.gamma > 0.0 { (1.0 + v.gamma * p.sqrt()).ln() } else { 10.0 * p.max(1e-10).log10() })
                .collect()
        })
        .collect();
    let w = v.max_filter / 2;
    let mut env = vec![0.0; comp.len()];
    for t in v.lag..comp.len() {
        let prev = &comp[t - v.lag];
        env[t] = comp[t]
            .iter()
            .enumerate()
            .map(|(k, &x)| {
                let m = prev[k.saturating_sub(w)..(k + w + 1).min(prev.len())].iter().copied().fold(f64::NEG_INFINITY, f64::max);
                (x - m).max(0.0)
            })
            .sum::<f64>();
    }
    env
}

fn pick(env: &[f64], frame_s: f64, delta: f64) -> Vec<f64> {
    let max = env.iter().copied().fold(0.0, f64::max).max(1e-12);
    let x: Vec<f64> = env.iter().map(|v| v / max).collect();
    let fr = |s: f64| ((s / frame_s).round() as usize).max(1);
    let (pre, post, avg, wait) = (fr(0.03), fr(0.03), fr(0.10), fr(0.06));
    let mut out = Vec::new();
    let mut i = 0;
    while i < x.len() {
        let a = i.saturating_sub(pre);
        let b = (i + post + 1).min(x.len());
        let is_max = x[a..b].iter().all(|&v| v <= x[i]);
        let a2 = i.saturating_sub(avg);
        let b2 = (i + avg + 1).min(x.len());
        let mean = x[a2..b2].iter().sum::<f64>() / (b2 - a2) as f64;
        if is_max && x[i] > mean + delta {
            out.push(i as f64 * frame_s);
            i += wait;
        } else {
            i += 1;
        }
    }
    out
}

fn f1(truth: &[f64], found: &[f64], tol: f64) -> (f64, f64, f64, f64) {
    let mut used = vec![false; found.len()];
    let (mut hits, mut errs) = (0, Vec::new());
    for &t in truth {
        let best = found.iter().enumerate().filter(|(i, f)| !used[*i] && (**f - t).abs() <= tol).min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()));
        if let Some((i, f)) = best {
            used[i] = true;
            hits += 1;
            errs.push(f - t);
        }
    }
    errs.sort_by(|a, b| a.total_cmp(b));
    let (r, p) = (hits as f64 / truth.len() as f64, hits as f64 / found.len().max(1) as f64);
    (r, p, if r + p > 0.0 { 2.0 * r * p / (r + p) } else { 0.0 }, errs.get(errs.len() / 2).copied().unwrap_or(0.0))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let y = load_mono(std::path::Path::new(&args[1])).expect("stem");
    let arr = fw_xml::InstrumentalArrangement::load(&args[2]).expect("chart");
    let mut truth: Vec<f64> = fw_eval::reference_notes(&arr).iter().map(|n| n.start).collect();
    truth.dedup_by(|a, b| (*a - *b).abs() < 0.02);

    let v = |name, n_fft, hop, fmin, fmax, gamma, max_filter, lag| Variant { name, n_fft, hop, fmin, fmax, gamma, max_filter, lag };
    let variants = [
        v("superflux 60-4000 1024/128", 1024, 128, 60.0, 4000.0, 10.0, 3, 2),
        v("sf 60-4000 g=3", 1024, 128, 60.0, 4000.0, 3.0, 3, 2),
        v("sf 60-4000 g=30", 1024, 128, 60.0, 4000.0, 30.0, 3, 2),
        v("sf 60-4000 g=100", 1024, 128, 60.0, 4000.0, 100.0, 3, 2),
        v("sf 60-4000 mf5", 1024, 128, 60.0, 4000.0, 10.0, 5, 2),
        v("sf 60-4000 lag3", 1024, 128, 60.0, 4000.0, 10.0, 3, 3),
        v("sf 60-4000 lag4", 1024, 128, 60.0, 4000.0, 10.0, 3, 4),
        v("sf 60-4000 hop64", 1024, 64, 60.0, 4000.0, 10.0, 3, 4),
        v("sf 60-6000 2048/128", 2048, 128, 60.0, 6000.0, 10.0, 3, 2),
        v("sf 80-4000 512/128", 512, 128, 80.0, 4000.0, 10.0, 3, 2),
        v("sf 60-2000 1024/128", 1024, 128, 60.0, 2000.0, 10.0, 3, 2),
        v("sf 60-8000 1024/128", 1024, 128, 60.0, 8000.0, 10.0, 3, 2),
    ];
    println!("{} chart onsets", truth.len());
    for var in &variants {
        let env = strength(&y, var);
        let frame_s = var.hop as f64 / SR;
        let best = (1..=30)
            .map(|d| d as f64 * 0.01)
            .map(|d| (d, f1(&truth, &pick(&env, frame_s, d), 0.05)))
            .max_by(|a, b| a.1.2.total_cmp(&b.1.2))
            .expect("sweep");
        let (d, (r, p, f, e)) = best;
        println!("{:36} best delta {d:.2}: recall {:5.1}% precision {:5.1}% F1 {:5.1}% median err {:+.0} ms", var.name, r * 100.0, p * 100.0, f * 100.0, e * 1000.0);
    }
}
