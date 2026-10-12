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

    let v = |harmonics, alpha, compress, bandwidth_cents, up_ratio| Variant { harmonics, alpha, compress, bandwidth_cents, up_ratio };
    let variants: Vec<(&str, Variant)> = vec![
        ("H5 up0", v(5, 0.5, 0.5, 25.0, 0.0)),
        ("H5 up0.95", v(5, 0.5, 0.5, 25.0, 0.95)),
        ("H5 up0.85", v(5, 0.5, 0.5, 25.0, 0.85)),
        ("H5 up0.75", v(5, 0.5, 0.5, 25.0, 0.75)),
        ("H5 up0.65", v(5, 0.5, 0.5, 25.0, 0.65)),
        ("H5 up0.55", v(5, 0.5, 0.5, 25.0, 0.55)),
        ("H5 up0.45", v(5, 0.5, 0.5, 25.0, 0.45)),
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
    let mut by_interval: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    let mut by_len = [(0usize, 0usize); 4];
    let mut by_pitch = [(0usize, 0usize); 4];

    for dir in &songs {
        let before = totals[variants.len()][0];
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

        for (ai, &attack) in attacks.iter().enumerate() {
            let specs: Vec<Vec<f32>> = evs.iter().map(|e| spectrum(&stem, &fft, e.start + attack, e.end - 0.01)).collect();
            for (vi, (_, v)) in variants.iter().enumerate() {
                for (e, spec) in evs.iter().zip(&specs) {
                    let got = if spec.is_empty() { -100 } else { pick(&salience(spec, lo, hi, tuning, v), lo, v) };
                    totals[ai * variants.len() + vi][classify(got, e.midi)] += 1;
                    if ai == 1 && vi == 0 {
                        let (pc, share, conf) = pyin_info[evs.iter().position(|x| std::ptr::eq(x, e)).unwrap_or(0)];
                        combo.push((pc, share, got == e.midi, conf));
                    }
                    if ai == 1 && vi == 3 {
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
        let exact = 100.0 * (totals[variants.len()][0] - before) as f64 / evs.len() as f64;
        eprintln!("{}: {} single-note events, tuning {:+.0} cents, salience exact {exact:.0}%", Path::new(dir).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), evs.len(), tuning * 100.0);
    }

    let pct = |c: &[usize; 4]| format!("exact {:5.1}%  octave {:4.1}%  semitone {:4.1}%  other {:4.1}%", 100.0 * c[0] as f64 / count as f64, 100.0 * c[1] as f64 / count as f64, 100.0 * c[2] as f64 / count as f64, 100.0 * c[3] as f64 / count as f64);
    println!("{count} single-note events");
    println!("{:34} {}", "pYIN vote (current)", pct(&pyin_total));
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
    println!("\nerrors of {} at {}ms (detected - true semitones: count): {:?}", variants[3].0, attacks[1] * 1000.0, top.iter().take(12).collect::<Vec<_>>());
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
