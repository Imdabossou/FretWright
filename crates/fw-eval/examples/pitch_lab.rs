//! Pitch estimation on the chart's own note positions: how often does each
//! estimator name the right MIDI note for a note whose start and end are known?
//! `pitch_lab <work folder>` (the folder `bench` fills).
//!
//! Estimators: the current one (pYIN, majority vote after the attack) and a
//! harmonic-salience search over the note's own spectrum, with variations.

use std::path::Path;

use fw_detect::dsp::{self, Fft};
use fw_detect::pipeline::{self, HOP, SR, load_mono};
use fw_detect::notes;

const N_FFT: usize = 32768;

struct Event {
    start: f64,
    end: f64,
    midi: i32,
}

/// Single-note events (chords skipped) with the time to the next event as their length.
fn events(p: &fw_project::Project) -> Vec<Event> {
    let mut by_start: Vec<(f64, Vec<i32>)> = Vec::new();
    for n in &p.notes {
        match by_start.last_mut() {
            Some((t, v)) if (n.start - *t).abs() < 0.015 => v.push(n.midi),
            _ => by_start.push((n.start, vec![n.midi])),
        }
    }
    let mut out = Vec::new();
    for (i, (t, v)) in by_start.iter().enumerate() {
        let next = by_start.get(i + 1).map_or(t + 0.5, |x| x.0);
        let end = next.min(t + 0.8);
        if v.len() == 1 && end - t >= 0.09 {
            out.push(Event { start: *t, end, midi: v[0] });
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
struct Variant {
    harmonics: usize,
    /// harmonic h weighs 1 / h^alpha
    alpha: f64,
    /// magnitude^compress
    compress: f64,
    bandwidth_cents: f64,
    /// take the octave above the winner when it scores at least this share of it (0 = never)
    up_ratio: f64,
    /// switch to the octave above when the winner's odd harmonics are this weak against its even ones (0 = off)
    ghost_tau: f64,
}

/// Magnitude spectrum of the part of the stem inside the event, Hann windowed.
fn spectrum(stem: &[f64], fft: &Fft, from: f64, to: f64) -> Vec<f32> {
    let (a, b) = ((from * SR) as usize, ((to * SR) as usize).min(stem.len()));
    if b <= a + 256 {
        return Vec::new();
    }
    let w = dsp::hann(b - a);
    let x: Vec<f64> = stem[a..b].iter().zip(&w).map(|(s, w)| s * w).collect();
    fft.power(&x).iter().map(|p| p.sqrt() as f32).collect()
}

/// Harmonic salience of every candidate note; `tuning` in semitones.
fn salience(spec: &[f32], lo: i32, hi: i32, tuning: f64, v: &Variant) -> Vec<f64> {
    let hz_per_bin = SR / N_FFT as f64;
    let peak = |f: f64| -> f64 {
        let half = f * (2f64.powf(v.bandwidth_cents / 1200.0) - 1.0);
        let (a, b) = (((f - half) / hz_per_bin).floor().max(1.0) as usize, ((f + half) / hz_per_bin).ceil() as usize);
        if b >= spec.len() {
            return 0.0;
        }
        (a..=b).map(|i| spec[i] as f64).fold(0.0, f64::max).powf(v.compress)
    };
    (lo..=hi)
        .map(|m| {
            let f0 = 440.0 * 2f64.powf((m as f64 + tuning - 69.0) / 12.0);
            (1..=v.harmonics).map(|h| peak(f0 * h as f64) / (h as f64).powf(v.alpha)).sum()
        })
        .collect()
}

/// Magnitude (sqrt-compressed) of the strongest bin near `f`.
fn peak_at(spec: &[f32], f: f64, bandwidth_cents: f64) -> f64 {
    let hz_per_bin = SR / N_FFT as f64;
    let half = f * (2f64.powf(bandwidth_cents / 1200.0) - 1.0);
    let (a, b) = (((f - half) / hz_per_bin).floor().max(1.0) as usize, ((f + half) / hz_per_bin).ceil() as usize);
    if b >= spec.len() {
        return 0.0;
    }
    (a..=b).map(|i| spec[i] as f64).fold(0.0, f64::max).sqrt()
}

/// Odd over even harmonic energy of the note `midi`.
fn odd_even_ratio(spec: &[f32], midi: i32, tuning: f64, bw: f64) -> f64 {
    let f0 = 440.0 * 2f64.powf((midi as f64 + tuning - 69.0) / 12.0);
    let odd: f64 = [1.0, 3.0, 5.0].iter().map(|h| peak_at(spec, f0 * h, bw)).sum();
    let even: f64 = [2.0, 4.0, 6.0].iter().map(|h| peak_at(spec, f0 * h, bw)).sum();
    if even > 0.0 { odd / even } else { 0.0 }
}

/// Normalised autocorrelation of `x` at `lag` (best of +-2 samples).
fn acf(x: &[f64], lag: f64) -> f64 {
    let n = x.len();
    let energy: f64 = x.iter().map(|v| v * v).sum::<f64>().max(1e-12);
    (-2..=2)
        .map(|d| {
            let l = (lag.round() as i64 + d).max(1) as usize;
            if l >= n {
                return 0.0;
            }
            x[..n - l].iter().zip(&x[l..]).map(|(a, b)| a * b).sum::<f64>() / energy
        })
        .fold(f64::MIN, f64::max)
}

/// A note an octave below the real one shares its even harmonics but has none
/// of the odd ones: if the odd harmonics are missing, move up an octave.
fn ghost_fix(spec: &[f32], midi: i32, tuning: f64, v: &Variant, hi: i32) -> i32 {
    if v.ghost_tau <= 0.0 || midi + 12 > hi {
        return midi;
    }
    let f0 = 440.0 * 2f64.powf((midi as f64 + tuning - 69.0) / 12.0);
    let odd: f64 = [1.0, 3.0, 5.0].iter().map(|h| peak_at(spec, f0 * h, v.bandwidth_cents)).sum();
    let even: f64 = [2.0, 4.0, 6.0].iter().map(|h| peak_at(spec, f0 * h, v.bandwidth_cents)).sum();
    if even > 0.0 && odd / even < v.ghost_tau { midi + 12 } else { midi }
}

fn pick(sal: &[f64], lo: i32, v: &Variant) -> i32 {
    let best = sal.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |x| x.0);
    let up = best + 12;
    if v.up_ratio > 0.0 && up < sal.len() && sal[up] >= v.up_ratio * sal[best] {
        return lo + up as i32;
    }
    lo + best as i32
}

fn main() {
    let work = std::env::args().nth(1).expect("work folder");
    let mut songs: Vec<_> = std::fs::read_dir(&work).expect("work").filter_map(|e| e.ok()).filter(|e| e.path().join("project.fretwright.json").is_file()).map(|e| e.path()).collect();
    songs.sort();

    let v = |harmonics, alpha, compress, bandwidth_cents, up_ratio, ghost_tau| Variant { harmonics, alpha, compress, bandwidth_cents, up_ratio, ghost_tau };
    let variants: Vec<(&str, Variant)> = vec![
        ("H5 ghost off", v(5, 0.5, 0.5, 25.0, 0.0, 0.0)),
        ("H5 ghost 0.6", v(5, 0.5, 0.5, 25.0, 0.0, 0.6)),
        ("H5 ghost 0.8", v(5, 0.5, 0.5, 25.0, 0.0, 0.8)),
        ("H5 ghost 1.0", v(5, 0.5, 0.5, 25.0, 0.0, 1.0)),
        ("H5 ghost 1.3", v(5, 0.5, 0.5, 25.0, 0.0, 1.3)),
        ("H5 ghost 1.7", v(5, 0.5, 0.5, 25.0, 0.0, 1.7)),
        ("H5 ghost 2.2", v(5, 0.5, 0.5, 25.0, 0.0, 2.2)),
    ];
    let attacks = [0.0f64, 0.01];

    // per estimator: [exact, octave error, semitone error, other]; and the pYIN-vote reference
    let mut totals = vec![[0usize; 4]; variants.len() * attacks.len()];
    let mut pyin_total = [0usize; 4];
    let mut count = 0usize;
    let classify = |got: i32, truth: i32| -> usize {
        let d = (got - truth).abs();
        if d == 0 { 0 } else if d % 12 == 0 { 1 } else if d == 1 { 2 } else { 3 }
    };
    let fft = Fft::new(N_FFT);
    // per event: (pyin correct, pyin confidence, salience correct)
    let mut combo: Vec<(bool, f64, bool, f64)> = Vec::new();
    let mut ratios: Vec<(String, Vec<f64>, Vec<f64>)> = Vec::new();
    // feature vectors of real winners and ghosts, pooled over songs
    let (mut good_f, mut ghost_f): (Vec<[f64; 6]>, Vec<[f64; 6]>) = (Vec::new(), Vec::new());
    let mut by_interval: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    let mut by_len = [(0usize, 0usize); 4];
    let mut by_pitch = [(0usize, 0usize); 4];

    for dir in &songs {
        let before: Vec<usize> = (0..variants.len()).map(|vi| totals[variants.len() + vi][0]).collect();
        let project = fw_project::Project::load(dir).expect("project");
        let stem = load_mono(&pipeline::stem_path(dir, "htdemucs_ft")).expect("stem");
        let open = project.open_strings();
        let (lo, hi) = (open[0] - 1, open[3] + 25);
        let evs = events(&project);
        if evs.is_empty() {
            continue;
        }
        let fmin = 440.0 * 2f64.powf(((open[0] - 2) as f64 - 69.0) / 12.0);
        let fmax = 440.0 * 2f64.powf(((open[3] + 24 + 2) as f64 - 69.0) / 12.0);
        let p = pipeline::cached_pyin(&stem, fmin, fmax, &pipeline::pitch_cache_path(dir, "htdemucs_ft"));
        let tuning = notes::tuning_offset(&p.f0, &p.voiced_prob);

        let mut pyin_info: Vec<(bool, f64, f64)> = Vec::new();
        // reference: majority of the rounded pYIN frames after the attack
        let ft = HOP as f64 / SR;
        for e in &evs {
            let (a, b) = (((e.start + 0.10) / ft) as usize, (e.end / ft) as usize);
            let mut votes = std::collections::BTreeMap::new();
            for i in a..b.min(p.f0.len()) {
                if !p.f0[i].is_nan() {
                    *votes.entry((notes::hz_to_midi(p.f0[i]) - tuning).round() as i32).or_insert(0) += 1;
                }
            }
            let got = votes.iter().max_by_key(|(_, c)| **c).map_or(-100, |(m, _)| *m);
            pyin_total[classify(got, e.midi)] += 1;
            // vote share and mean voicing over the event
            let frames = (a..b.min(p.f0.len())).count().max(1);
            let share = votes.get(&got).copied().unwrap_or(0) as f64 / frames as f64;
            let conf = (a..b.min(p.f0.len())).map(|i| p.voiced_prob[i]).sum::<f64>() / frames as f64;
            pyin_info.push((got == e.midi, share, conf));
        }

        let (mut good_r, mut ghost_r) = (Vec::new(), Vec::new());
        {
            let specs: Vec<Vec<f32>> = evs.iter().map(|e| spectrum(&stem, &fft, e.start + 0.01, e.end - 0.01)).collect();
            for (e, spec) in evs.iter().zip(&specs) {
                if spec.is_empty() {
                    continue;
                }
                let w = pick(&salience(spec, lo, hi, tuning, &variants[0].1), lo, &variants[0].1);
                let r = odd_even_ratio(spec, w, tuning, 25.0);
                let f0 = 440.0 * 2f64.powf((w as f64 + tuning - 69.0) / 12.0);
                let v = |h: f64| peak_at(spec, f0 * h, 25.0);
                let total: f64 = (1..=6).map(|h| v(h as f64)).sum::<f64>().max(1e-9);
                let (a, b) = (((e.start + 0.01) * SR) as usize, ((e.end - 0.01) * SR) as usize);
                let seg: Vec<f64> = { let m = stem[a..b.min(stem.len())].iter().sum::<f64>() / (b - a).max(1) as f64; stem[a..b.min(stem.len())].iter().map(|v| v - m).collect() };
                let t1 = SR / f0;
                let acf_ratio = acf(&seg, t1 / 2.0) / acf(&seg, t1).abs().max(1e-6);
                let feat = [v(1.0) / v(2.0).max(1e-9), v(3.0) / v(2.0).max(1e-9), r, v(1.0) / total, (v(1.0) + v(3.0)) / total, acf_ratio];
                if w == e.midi {
                    good_r.push(r);
                    good_f.push(feat);
                } else if w + 12 == e.midi {
                    ghost_r.push(r);
                    ghost_f.push(feat);
                }
            }
        }
        ratios.push((Path::new(dir).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), good_r, ghost_r));
        for (ai, &attack) in attacks.iter().enumerate() {
            let specs: Vec<Vec<f32>> = evs.iter().map(|e| spectrum(&stem, &fft, e.start + attack, e.end - 0.01)).collect();
            for (vi, (_, v)) in variants.iter().enumerate() {
                for (e, spec) in evs.iter().zip(&specs) {
                    let got = if spec.is_empty() { -100 } else { ghost_fix(spec, pick(&salience(spec, lo, hi, tuning, v), lo, v), tuning, v, hi) };
                    totals[ai * variants.len() + vi][classify(got, e.midi)] += 1;
                    if ai == 1 && vi == 0 {
                        let (pc, share, conf) = pyin_info[evs.iter().position(|x| std::ptr::eq(x, e)).unwrap_or(0)];
                        combo.push((pc, share, got == e.midi, conf));
                    }
                    if ai == 1 && vi == 0 {
                        *by_interval.entry(got - e.midi).or_insert(0usize) += 1;
                        let len = e.end - e.start;
                        let lb = if len < 0.12 { 0 } else if len < 0.2 { 1 } else if len < 0.35 { 2 } else { 3 };
                        by_len[lb].0 += usize::from(got == e.midi);
                        by_len[lb].1 += 1;
                        let pb = if e.midi < open[0] + 5 { 0 } else if e.midi < open[0] + 12 { 1 } else if e.midi < open[0] + 19 { 2 } else { 3 };
                        by_pitch[pb].0 += usize::from(got == e.midi);
                        by_pitch[pb].1 += 1;
                    }
                }
            }
        }
        count += evs.len();
        let row: Vec<String> = (0..variants.len()).map(|vi| format!("{:4.1}", 100.0 * (totals[variants.len() + vi][0] - before[vi]) as f64 / evs.len() as f64)).collect();
        eprintln!("{}: {} events, tuning {:+.0}c, exact per variant: {}", Path::new(dir).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), evs.len(), tuning * 100.0, row.join("  "));
    }

    let pct = |c: &[usize; 4]| format!("exact {:5.1}%  octave {:4.1}%  semitone {:4.1}%  other {:4.1}%", 100.0 * c[0] as f64 / count as f64, 100.0 * c[1] as f64 / count as f64, 100.0 * c[2] as f64 / count as f64, 100.0 * c[3] as f64 / count as f64);
    println!("{count} single-note events");
    println!("{:34} {}", "pYIN vote (current)", pct(&pyin_total));
    let q = |v: &Vec<f64>, f: f64| { let mut s = v.clone(); s.sort_by(|a, b| a.total_cmp(b)); if s.is_empty() { f64::NAN } else { s[((s.len() - 1) as f64 * f) as usize] } };
    println!("odd/even harmonic ratio of the winner: real notes (10/50/90th pct) vs ghosts one octave below the truth");
    for (name, g, h) in &ratios {
        println!("  {name:14} real {:.2}/{:.2}/{:.2} (n={:4})   ghost {:.2}/{:.2}/{:.2} (n={:4})", q(g, 0.1), q(g, 0.5), q(g, 0.9), g.len(), q(h, 0.1), q(h, 0.5), q(h, 0.9), h.len());
    }
    let names = ["v1/v2", "v3/v2", "odd/even", "v1/total", "(v1+v3)/total", "acf(T/2)/acf(T)"];
    for k in 0..6 {
        // best single threshold, ghost = below it; balanced accuracy
        let mut cands: Vec<f64> = good_f.iter().chain(&ghost_f).map(|f| f[k]).collect();
        cands.sort_by(|a, b| a.total_cmp(b));
        let best = cands.iter().step_by((cands.len() / 400).max(1)).map(|&t| {
            let (lowside_ghost, hi) = if k == 5 { (false, true) } else { (true, false) };
            let _ = hi;
            let is_g = |f: &[f64; 6]| if lowside_ghost { f[k] < t } else { f[k] >= t };
            let tpr = ghost_f.iter().filter(|f| is_g(f)).count() as f64 / ghost_f.len().max(1) as f64;
            let tnr = good_f.iter().filter(|f| !is_g(f)).count() as f64 / good_f.len().max(1) as f64;
            (0.5 * (tpr + tnr), t, tpr, tnr)
        }).fold((0.0, 0.0, 0.0, 0.0), |a, b| if b.0 > a.0 { b } else { a });
        println!("feature {:14} best threshold {:.3}: balanced accuracy {:.1}% (ghosts caught {:.1}%, real kept {:.1}%)", names[k], best.1, best.0 * 100.0, best.2 * 100.0, best.3 * 100.0);
    }
    println!("pooled: {} real winners, {} ghosts", good_f.len(), ghost_f.len());
    let n = combo.len() as f64;
    let both = combo.iter().filter(|c| c.0 && c.2).count() as f64;
    let only_p = combo.iter().filter(|c| c.0 && !c.2).count() as f64;
    let only_s = combo.iter().filter(|c| !c.0 && c.2).count() as f64;
    println!("pYIN right {:.1}%, salience right {:.1}%: both {:.1}%, only pYIN {:.1}%, only salience {:.1}%, neither {:.1}%", 100.0 * (both + only_p) / n, 100.0 * (both + only_s) / n, 100.0 * both / n, 100.0 * only_p / n, 100.0 * only_s / n, 100.0 * (n - both - only_p - only_s) / n);
    for th in [0.3, 0.5, 0.7, 0.8, 0.9, 0.95] {
        let ok = combo.iter().filter(|c| if c.1 >= th { c.0 } else { c.2 }).count() as f64;
        println!("  use pYIN when its vote share >= {th}: {:.1}% exact", 100.0 * ok / n);
    }
    let mut top: Vec<_> = by_interval.iter().collect();
    top.sort_by_key(|x| std::cmp::Reverse(*x.1));
    println!("\nerrors of {} at {}ms (detected - true semitones: count): {:?}", variants[0].0, attacks[1] * 1000.0, top.iter().take(12).collect::<Vec<_>>());
    for (name, b) in [("event length <120ms, 120-200, 200-350, >350ms", &by_len), ("pitch: open..+4, +5..+11, +12..+18, higher", &by_pitch)] {
        println!("exact by {name}: {}", b.iter().map(|(h, n)| format!("{:.1}% of {n}", 100.0 * *h as f64 / (*n).max(1) as f64)).collect::<Vec<_>>().join(" | "));
    }
    for (ai, &attack) in attacks.iter().enumerate() {
        for (vi, (name, _)) in variants.iter().enumerate() {
            println!("{:34} {}", format!("{name} attack {:.0}ms", attack * 1000.0), pct(&totals[ai * variants.len() + vi]));
        }
    }
}

// (appended) error analysis
