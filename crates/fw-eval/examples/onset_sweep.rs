//! Onset picking variants over all songs: `onset_sweep <work folder>`.
use fw_detect::onset;
use fw_detect::pipeline::{self, SR, load_mono};

const HOP: usize = 128;

struct Song {
    name: String,
    truth: Vec<f64>,
    env: Vec<f64>,
}

fn score(truth: &[f64], found: &[f64], tol: f64) -> (usize, usize, usize) {
    let mut used = vec![false; found.len()];
    let mut hits = 0;
    for &t in truth {
        if let Some((i, _)) = found.iter().enumerate().filter(|(i, f)| !used[*i] && (**f - t).abs() <= tol).min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs())) {
            used[i] = true;
            hits += 1;
        }
    }
    (hits, truth.len(), found.len())
}

/// norm: 0 = global max, 1 = 99th percentile, 2 = local max over `window` seconds, 3 = local mean x k
fn pick(env: &[f64], norm: u8, window: f64, delta: f64, wait_s: f64) -> Vec<f64> {
    let frame_s = HOP as f64 / SR;
    let x: Vec<f64> = match norm {
        0 => {
            let m = env.iter().copied().fold(1e-12, f64::max);
            env.iter().map(|v| v / m).collect()
        }
        1 => {
            let mut s = env.to_vec();
            s.sort_by(|a, b| a.total_cmp(b));
            let m = s[(s.len() as f64 * 0.99) as usize].max(1e-12);
            env.iter().map(|v| v / m).collect()
        }
        _ => {
            let w = (window / frame_s) as usize;
            // local scale: a high percentile of the neighbourhood, so a few loud hits do not hide the rest
            (0..env.len())
                .map(|i| {
                    let (a, b) = (i.saturating_sub(w), (i + w).min(env.len()));
                    let mut loc: Vec<f64> = env[a..b].iter().step_by(4).copied().collect();
                    loc.sort_by(|p, q| p.total_cmp(q));
                    let m = if norm == 2 { loc[(loc.len() as f64 * 0.95) as usize] } else { loc.iter().sum::<f64>() / loc.len() as f64 * 3.0 };
                    env[i] / m.max(1e-9)
                })
                .collect()
        }
    };
    let fr = |s: f64| ((s / frame_s).round() as usize).max(1);
    let (win, avg, wait) = (fr(0.03), fr(0.10), fr(wait_s));
    let mut out = Vec::new();
    let mut i = 0;
    while i < x.len() {
        let (a, b) = (i.saturating_sub(win), (i + win + 1).min(x.len()));
        let (a2, b2) = (i.saturating_sub(avg), (i + avg + 1).min(x.len()));
        let mean = x[a2..b2].iter().sum::<f64>() / (b2 - a2) as f64;
        if x[a..b].iter().all(|&v| v <= x[i]) && x[i] > mean + delta {
            out.push(i as f64 * frame_s);
            i += wait;
        } else {
            i += 1;
        }
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    let mut songs = Vec::new();
    for lag in [4usize] {
        for d in &dirs {
            let p = fw_project::Project::load(d).expect("project");
            let stem = load_mono(&pipeline::stem_path(d, "htdemucs_ft")).expect("stem");
            let mut truth: Vec<f64> = p.notes.iter().map(|n| n.start).collect();
            truth.dedup_by(|a, b| (*a - *b).abs() < 0.02);
            songs.push(Song { name: d.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), truth, env: onset::superflux_strength(&stem, SR, 1024, HOP, lag) });
        }
    }
    let report = |label: &str, norm: u8, window: f64, delta: f64, wait: f64| {
        let (mut f1s, mut th, mut tn, mut tf) = (Vec::new(), 0, 0, 0);
        for s in &songs {
            let found = pick(&s.env, norm, window, delta, wait);
            let (h, n, f) = score(&s.truth, &found, 0.05);
            let (r, p) = (h as f64 / n as f64, h as f64 / f.max(1) as f64);
            f1s.push(if r + p > 0.0 { 2.0 * r * p / (r + p) } else { 0.0 });
            th += h;
            tn += n;
            tf += f;
        }
        let (r, p) = (th as f64 / tn as f64, th as f64 / tf as f64);
        println!("{label:34} per-song F1 {:5.1}%   all: recall {:5.1}% precision {:5.1}%  worst {:5.1}% ({})", 100.0 * f1s.iter().sum::<f64>() / f1s.len() as f64, 100.0 * r, 100.0 * p, 100.0 * f1s.iter().copied().fold(1.0, f64::min), songs[f1s.iter().position(|v| *v == f1s.iter().copied().fold(1.0, f64::min)).unwrap_or(0)].name);
    };
    println!("{} songs", songs.len());
    for d in [0.02, 0.03, 0.04, 0.06] {
        report(&format!("global max, delta {d}"), 0, 0.0, d, 0.06);
    }
    for d in [0.05, 0.08, 0.12, 0.16] {
        report(&format!("99th percentile, delta {d}"), 1, 0.0, d, 0.06);
    }
    for w in [1.0, 2.0, 4.0] {
        for d in [0.08, 0.12, 0.16, 0.22] {
            report(&format!("local 95th pct ±{w}s, delta {d}"), 2, w, d, 0.06);
        }
    }
    for w in [1.0, 2.0] {
        for d in [0.1, 0.15, 0.2, 0.3] {
            report(&format!("local mean x3 ±{w}s, delta {d}"), 3, w, d, 0.06);
        }
    }
}
