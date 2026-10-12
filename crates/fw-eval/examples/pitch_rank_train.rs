//! Trains a softmax ranker over the best pitch candidates of each charted note and
//! tests it leave-one-song-out: `pitch_rank_train <work folder>`.
use fw_detect::pipeline::{self, SR, load_mono};
use fw_detect::pitch::{Analyzer, Candidate, N_RANK_FEATURES};

const K: usize = 8;

struct Event {
    song: usize,
    truth: usize, // index into cands, or usize::MAX if the truth is not among them
    flags: String,
    cands: Vec<Candidate>,
}

fn events_of(p: &fw_project::Project) -> Vec<(f64, f64, i32, String)> {
    let mut by_start: Vec<(f64, Vec<i32>, String)> = Vec::new();
    for n in &p.notes {
        match by_start.last_mut() {
            Some((t, v, _)) if (n.start - *t).abs() < 0.015 => v.push(n.midi),
            _ => by_start.push((n.start, vec![n.midi], n.technique_names().first().map_or("plain".to_string(), |s| s.to_string()))),
        }
    }
    let mut out = Vec::new();
    for (i, (t, v, f)) in by_start.iter().enumerate() {
        let end = by_start.get(i + 1).map_or(t + 0.5, |x| x.0).min(t + 0.8);
        if v.len() == 1 && end - t >= 0.09 {
            out.push((*t, end, v[0], f.clone()));
        }
    }
    out
}

fn softmax_train(events: &[&Event], l2: f64) -> [f64; N_RANK_FEATURES] {
    let mut w = [0.0f64; N_RANK_FEATURES];
    let (mut m, mut v) = ([0.0f64; N_RANK_FEATURES], [0.0f64; N_RANK_FEATURES]);
    let usable: Vec<&&Event> = events.iter().filter(|e| e.truth != usize::MAX).collect();
    let n = usable.len() as f64;
    for t in 1..=300 {
        let mut g = [0.0f64; N_RANK_FEATURES];
        for e in &usable {
            let scores: Vec<f64> = e.cands.iter().map(|c| c.features.iter().zip(&w).map(|(a, b)| a * b).sum()).collect();
            let mx = scores.iter().copied().fold(f64::MIN, f64::max);
            let exps: Vec<f64> = scores.iter().map(|s| (s - mx).exp()).collect();
            let z: f64 = exps.iter().sum();
            for (j, c) in e.cands.iter().enumerate() {
                let p = exps[j] / z - f64::from(j == e.truth);
                for i in 0..N_RANK_FEATURES {
                    g[i] += p * c.features[i];
                }
            }
        }
        for i in 0..N_RANK_FEATURES {
            let gi = g[i] / n + l2 * w[i];
            m[i] = 0.9 * m[i] + 0.1 * gi;
            v[i] = 0.999 * v[i] + 0.001 * gi * gi;
            w[i] -= 0.05 * (m[i] / (1.0 - 0.9f64.powi(t))) / ((v[i] / (1.0 - 0.999f64.powi(t))).sqrt() + 1e-8);
        }
    }
    w
}

fn accuracy(events: &[&Event], w: &[f64; N_RANK_FEATURES]) -> (usize, usize) {
    let hit = events
        .iter()
        .filter(|e| {
            let best = (0..e.cands.len()).max_by(|&a, &b| {
                let sa: f64 = e.cands[a].features.iter().zip(w).map(|(x, y)| x * y).sum();
                let sb: f64 = e.cands[b].features.iter().zip(w).map(|(x, y)| x * y).sum();
                sa.total_cmp(&sb)
            });
            best == Some(e.truth)
        })
        .count();
    (hit, events.len())
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    let mut all: Vec<Event> = Vec::new();
    let mut names = Vec::new();
    for (si, d) in dirs.iter().enumerate() {
        let p = fw_project::Project::load(d).expect("project");
        let stem = load_mono(&pipeline::stem_path(d, "htdemucs_ft")).expect("stem");
        let open = p.open_strings();
        let (lo, hi) = (open[0] - 1, open[3] + 25);
        let analyzer = Analyzer::new(&stem, SR, lo, hi);
        let evs = events_of(&p);
        let specs: Vec<Option<Vec<f32>>> = evs.iter().map(|e| analyzer.spectrum(e.0 + 0.01, e.1 - 0.01)).collect();
        // first pass without a prior: how often is each note the top candidate?
        let mut count = vec![0.0f64; (hi - lo + 1) as usize];
        for s in specs.iter().flatten() {
            let c = analyzer.rank_candidates(s, 0.0, None, 1);
            count[(c[0].midi - lo) as usize] += 1.0;
        }
        let total: f64 = count.iter().sum::<f64>() + count.len() as f64;
        let log_prior: Vec<f64> = count.iter().map(|c| ((c + 1.0) / total).ln()).collect();
        for (e, s) in evs.iter().zip(&specs) {
            let Some(s) = s else { continue };
            let cands = analyzer.rank_candidates(s, 0.0, Some(&log_prior), K);
            let truth = cands.iter().position(|c| c.midi == e.2).unwrap_or(usize::MAX);
            all.push(Event { song: si, truth, cands, flags: e.3.clone() });
        }
        names.push(d.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()));
    }
    let mut by_flag: std::collections::BTreeMap<String, (usize, usize)> = std::collections::BTreeMap::new();
    for e in &all {
        let ok = e.truth == 0;
        let x = by_flag.entry(e.flags.clone()).or_insert((0, 0));
        x.0 += usize::from(ok);
        x.1 += 1;
    }
    println!("current estimator by the chart note's first technique:");
    for (k, (h, n)) in &by_flag {
        println!("  {k:16} {:5.1}% of {n}", 100.0 * *h as f64 / *n as f64);
    }
    let reach = all.iter().filter(|e| e.truth != usize::MAX).count();
    println!("{} events; the truth is among the top {K} for {:.1}%", all.len(), 100.0 * reach as f64 / all.len() as f64);
    let baseline_w = {
        let mut w = [0.0; N_RANK_FEATURES];
        w[0] = 1.0; // salience relative to the best: argmax = the current estimator
        w
    };
    let (mut hits, mut base_hits, mut total) = (0, 0, 0);
    for si in 0..names.len() {
        let train: Vec<&Event> = all.iter().filter(|e| e.song != si).collect();
        let test: Vec<&Event> = all.iter().filter(|e| e.song == si).collect();
        let w = softmax_train(&train, 1e-3);
        let (h, n) = accuracy(&test, &w);
        let (bh, _) = accuracy(&test, &baseline_w);
        println!("  {:14} baseline {:5.1}%  ranker {:5.1}%  ({n} events)", names[si], 100.0 * bh as f64 / n as f64, 100.0 * h as f64 / n as f64);
        hits += h;
        base_hits += bh;
        total += n;
    }
    println!("leave-one-song-out over all events: baseline {:.1}%, ranker {:.1}%", 100.0 * base_hits as f64 / total as f64, 100.0 * hits as f64 / total as f64);
    let everyone: Vec<&Event> = all.iter().collect();
    let w = softmax_train(&everyone, 1e-3);
    println!("final weights: {:?}", w.iter().map(|x| (x * 1000.0).round() / 1000.0).collect::<Vec<_>>());
}
