//! Can a second simultaneous note be found? Uses the chart's one- and two-note
//! events: `chord_lab <work folder>`.
use fw_detect::dsp::{self, Fft};
use fw_detect::pipeline::{self, SR, load_mono};

const N_FFT: usize = 32768;
const H: usize = 5;

fn peak(spec: &[f32], f: f64, cents: f64) -> f64 {
    let hz = SR / N_FFT as f64;
    let half = f * (2f64.powf(cents / 1200.0) - 1.0);
    let (a, b) = (((f - half) / hz).floor().max(1.0) as usize, ((f + half) / hz).ceil() as usize);
    if b >= spec.len() { 0.0 } else { (a..=b).map(|i| spec[i] as f64).fold(0.0, f64::max).sqrt() }
}

fn hz(m: i32, tuning: f64) -> f64 {
    440.0 * 2f64.powf((m as f64 + tuning - 69.0) / 12.0)
}

/// Normalised salience of `m`, ignoring harmonics that sit on a harmonic of `other` (if given).
fn salience(spec: &[f32], m: i32, tuning: f64, other: Option<i32>, coincide_cents: f64) -> f64 {
    let f0 = hz(m, tuning);
    let (mut sum, mut wsum) = (0.0, 0.0);
    for h in 1..=H {
        let f = f0 * h as f64;
        if let Some(o) = other {
            let fo = hz(o, tuning);
            let k = (f / fo).round();
            if k >= 1.0 && (1200.0 * (f / (fo * k)).log2()).abs() < coincide_cents {
                continue;
            }
        }
        let w = 1.0 / (h as f64).sqrt();
        sum += w * peak(spec, f, 25.0);
        wsum += w;
    }
    if wsum == 0.0 { 0.0 } else { sum / wsum }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    let fft = Fft::new(N_FFT);
    // per event: (is_pair, both found with m1 in truth and m2 in truth, rho, second note in truth)
    let mut rows: Vec<(bool, f64, bool, bool)> = Vec::new();
    for d in &dirs {
        let p = fw_project::Project::load(d).expect("project");
        let stem = load_mono(&pipeline::stem_path(d, "htdemucs_ft")).expect("stem");
        let open = p.open_strings();
        let (lo, hi) = (open[0] - 1, open[3] + 25);
        let mut by_start: Vec<(f64, Vec<i32>)> = Vec::new();
        for n in &p.notes {
            match by_start.last_mut() {
                Some((t, v)) if (n.start - *t).abs() < 0.015 => v.push(n.midi),
                _ => by_start.push((n.start, vec![n.midi])),
            }
        }
        for (i, (t, v)) in by_start.iter().enumerate() {
            let end = by_start.get(i + 1).map_or(t + 0.5, |x| x.0).min(t + 0.8);
            if end - t < 0.09 || v.len() > 2 {
                continue;
            }
            let (a0, b0) = (((t + 0.01) * SR) as usize, ((end - 0.01) * SR) as usize);
            if b0 > stem.len() || b0 <= a0 + 256 {
                continue;
            }
            let w = dsp::hann(b0 - a0);
            let x: Vec<f64> = stem[a0..b0].iter().zip(&w).map(|(s, w)| s * w).collect();
            let spec: Vec<f32> = fft.power(&x).iter().map(|p| p.sqrt() as f32).collect();
            let m1 = (lo..=hi).max_by(|&a, &b| salience(&spec, a, 0.0, None, 0.0).total_cmp(&salience(&spec, b, 0.0, None, 0.0))).unwrap_or(lo);
            let s1 = salience(&spec, m1, 0.0, None, 0.0).max(1e-9);
            let (m2, s2) = (lo..=hi)
                .filter(|&m| m != m1 && (m - m1).abs() != 12 && (m - m1).abs() != 24)
                .map(|m| (m, salience(&spec, m, 0.0, Some(m1), 40.0)))
                .fold((lo, 0.0), |acc, c| if c.1 > acc.1 { c } else { acc });
            let rho = s2 / s1;
            let is_pair = v.len() == 2;
            let second_ok = v.contains(&m2);
            rows.push((is_pair, rho, second_ok, v.contains(&m1)));
        }
    }
    let pairs: Vec<&(bool, f64, bool, bool)> = rows.iter().filter(|r| r.0).collect();
    let singles: Vec<&(bool, f64, bool, bool)> = rows.iter().filter(|r| !r.0).collect();
    println!("{} two-note events, {} single-note events", pairs.len(), singles.len());
    println!("rho threshold   pairs: second note found right   singles: false alarms");
    for th in [0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0] {
        let found = pairs.iter().filter(|r| r.1 >= th && r.2).count() as f64 / pairs.len() as f64;
        let wrong = pairs.iter().filter(|r| r.1 >= th && !r.2).count() as f64 / pairs.len() as f64;
        let fa = singles.iter().filter(|r| r.1 >= th).count() as f64 / singles.len() as f64;
        println!("  {th:.1}            {:5.1}% (wrong note {:4.1}%)             {:5.1}%", found * 100.0, wrong * 100.0, fa * 100.0);
    }
}
