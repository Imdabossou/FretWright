//! Frame-level pitch track -> clean, playable notes.

use std::collections::HashSet;

use fw_project::{BeatGrid, MAX_FRET, Note};

use crate::dsp;
use crate::pitch::{self, Analyzer};

/// MIDI note of a frequency, computed exactly as librosa does. pYIN's pitch
/// bins sit on half-semitones, so values are often exact rounding ties and the
/// last bit of the formula decides which way they round.
pub fn hz_to_midi(f: f64) -> f64 {
    12.0 * (f.log2() - 440f64.log2()) + 69.0
}

/// Splits the pitch track into notes at pitch changes and, for repeated
/// notes, at onsets.
pub fn segment(f0: &[f64], conf: &[f64], rms: &[f64], onsets: &[usize], frame_t: f64, min_note_ms: f64) -> Vec<Note> {
    let n = f0.len().min(rms.len()).min(conf.len());
    let max_rms = rms[..n].iter().copied().fold(0.0, f64::max);
    let gate = max_rms * 10f64.powf(-35.0 / 20.0);
    let raw: Vec<i32> = (0..n)
        .map(|i| if max_rms > 0.0 && rms[i] > gate && !f0[i].is_nan() { hz_to_midi(f0[i]).round_ties_even() as i32 } else { 0 })
        .collect();
    let midi = dsp::median_filter_nearest(&raw, 5);

    let onset_set: HashSet<usize> = onsets.iter().copied().collect();
    let min_frames = ((min_note_ms / 1000.0 / frame_t).round_ties_even() as usize).max(2);
    // pitch-change onsets come a few frames off the pitch boundary: keep
    // repeated-note cuts away from run edges to avoid phantom notes
    let margin = min_frames.max((0.09 / frame_t).round_ties_even() as usize);

    let mut runs: Vec<(usize, usize, i32)> = Vec::new();
    let mut i = 0;
    while i < n {
        let m = midi[i];
        if m == 0 {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && midi[j] == m {
            j += 1;
        }
        let mut cuts = vec![i];
        if j > i + 2 * margin {
            cuts.extend((i + margin..j - margin).filter(|o| onset_set.contains(o)));
        }
        cuts.push(j);
        runs.extend(cuts.windows(2).map(|w| (w[0], w[1], m)));
        i = j;
    }

    let mut merged: Vec<(usize, usize, i32)> = Vec::new();
    for r in runs {
        match merged.last_mut() {
            Some(last) if last.2 == r.2 && r.0 >= last.1 && r.0 - last.1 <= 2 && !onset_set.contains(&r.0) => last.1 = r.1,
            _ => merged.push(r),
        }
    }

    merged
        .into_iter()
        .filter(|(a, b, _)| b - a >= min_frames)
        .map(|(a, b, m)| {
            let mut note = Note::new(a as f64 * frame_t, b as f64 * frame_t, m);
            let mean = conf[a..b].iter().sum::<f64>() / (b - a) as f64;
            note.confidence = (mean * 1000.0).round_ties_even() / 1000.0;
            note
        })
        .collect()
}

/// How far the recording sits from A=440 tuning, in semitones (-0.5..0.5):
/// the confidence-weighted circular mean of the fractional part of the pitch
/// track. Subtracting it before rounding stops notes of a detuned song from
/// landing a semitone off.
pub fn tuning_offset(f0: &[f64], conf: &[f64]) -> f64 {
    let (mut sin, mut cos) = (0.0, 0.0);
    for (f, c) in f0.iter().zip(conf) {
        if f.is_nan() || *c < 0.3 {
            continue;
        }
        let angle = 2.0 * std::f64::consts::PI * hz_to_midi(*f);
        sin += c * angle.sin();
        cos += c * angle.cos();
    }
    if sin == 0.0 && cos == 0.0 { 0.0 } else { sin.atan2(cos) / (2.0 * std::f64::consts::PI) }
}

/// Settings for [`segment_at_onsets`].
#[derive(Debug, Clone, Copy)]
pub struct OnsetSegmentation {
    /// Seconds per pitch-track frame.
    pub frame_t: f64,
    /// Semitones the recording is off A=440 (see [`tuning_offset`]).
    pub tuning: f64,
    pub min_note_ms: f64,
    /// Start of a note whose pitch reading is not trusted (the tracker's long
    /// window smears the attack).
    pub attack_ms: f64,
    /// Pitch changes this close to an onset are ignored.
    pub near_ms: f64,
}

/// Frame-level pitch track -> notes, cut at the given onsets.
///
/// A note starts at every onset. Where the pitch changes with no onset close
/// by (legato playing: hammer-ons, pull-offs, slides) the note is cut at the
/// pitch change as well. Pitch changes right next to an onset are the pitch
/// tracker wobbling on the attack and are ignored. Each note's pitch is the
/// most common one after its attack, not whatever the tracker said first.
pub fn segment_at_onsets(f0: &[f64], rms: &[f64], onsets: &[usize], cfg: &OnsetSegmentation) -> Vec<Note> {
    let OnsetSegmentation { frame_t, tuning, min_note_ms, attack_ms, near_ms } = *cfg;
    let n = f0.len().min(rms.len());
    let max_rms = rms[..n].iter().copied().fold(0.0, f64::max);
    let gate = max_rms * 10f64.powf(-35.0 / 20.0);
    let raw: Vec<i32> = (0..n)
        .map(|i| if max_rms > 0.0 && rms[i] > gate && !f0[i].is_nan() { (hz_to_midi(f0[i]) - tuning).round_ties_even() as i32 } else { 0 })
        .collect();
    let midi = dsp::median_filter_nearest(&raw, 5);
    let frames = |ms: f64| ((ms / 1000.0 / frame_t).round() as usize).max(1);
    let (attack, near, min_frames) = (frames(attack_ms), frames(near_ms), frames(min_note_ms).max(2));

    let near_onset = |i: usize| onsets.iter().any(|&o| o.abs_diff(i) <= near);
    let mut cuts: Vec<usize> = onsets.iter().copied().filter(|&o| o < n).collect();
    // a new pitch only counts if it holds for a while
    let hold = min_frames + 2;
    for i in 1..n {
        if midi[i] != midi[i - 1] && midi[i] != 0 && midi[i - 1] != 0 && !near_onset(i) && i + hold <= n && midi[i..i + hold].iter().all(|&m| m == midi[i]) {
            cuts.push(i);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    if cuts.first() != Some(&0) {
        cuts.insert(0, 0);
    }
    cuts.push(n);

    let onset_set: std::collections::HashSet<usize> = onsets.iter().copied().collect();
    let mut out: Vec<(Note, bool)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        // silent tail: the note ends where the sound does
        let mut end = b;
        while end > a && midi[end - 1] == 0 {
            end -= 1;
        }
        let mut start = a;
        while start < end && midi[start] == 0 {
            start += 1;
        }
        if end <= start || end - start < min_frames {
            continue;
        }
        let len = end - start;
        let from = if len > attack + 4 { start + attack } else { start + len / 3 };
        let mut votes: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
        midi[from..end].iter().filter(|&&m| m != 0).for_each(|m| *votes.entry(*m).or_insert(0) += 1);
        let Some((&m, _)) = votes.iter().max_by_key(|(_, c)| **c) else { continue };
        out.push((Note::new(start as f64 * frame_t, end as f64 * frame_t, m), onset_set.contains(&a)));
    }
    // neighbouring segments of one pitch that no onset separates are one note
    let mut merged: Vec<Note> = Vec::new();
    for (note, at_onset) in out {
        match merged.last_mut() {
            Some(last) if last.midi == note.midi && note.start - last.end <= 2.0 * frame_t && !at_onset => last.end = note.end,
            _ => merged.push(note),
        }
    }
    merged
}

/// Short notes exactly an octave from both (equal) neighbours are the pitch
/// tracker locking onto the 2nd harmonic: move them to the neighbours' octave.
pub fn fix_octave_blips(notes: &mut [Note], max_len: f64) {
    for i in 1..notes.len().saturating_sub(1) {
        let (a, b) = (notes[i - 1].midi, notes[i + 1].midi);
        let n = &mut notes[i];
        if n.duration() <= max_len && a == b && (n.midi - a).abs() == 12 {
            n.midi = a;
        }
    }
}

/// Folds very short notes a semitone or two from a touching neighbour into it
/// (slides, wobble, attack transients).
pub fn clean_blips(notes: Vec<Note>, blip_s: f64, gap_s: f64) -> Vec<Note> {
    let mut out: Vec<Note> = Vec::new();
    let mut notes = notes;
    let len = notes.len();
    for i in 0..len {
        let n = notes[i].clone();
        if n.duration() < blip_s {
            if let Some(prev) = out.last_mut()
                && (prev.midi - n.midi).abs() <= 2
                && n.start - prev.end < gap_s
            {
                prev.end = n.end;
                continue;
            }
            if i + 1 < len && (notes[i + 1].midi - n.midi).abs() <= 2 && notes[i + 1].start - n.end < gap_s {
                notes[i + 1].start = n.start;
                continue;
            }
        }
        out.push(n);
    }
    out
}

/// Grid positions at `division` steps per beat.
pub fn subdivisions(grid: &BeatGrid, division: u32) -> Vec<f64> {
    let b = &grid.beats;
    if b.len() < 2 || division < 1 {
        return b.clone();
    }
    let mut out = Vec::with_capacity(b.len() * division as usize);
    for k in 0..b.len() - 1 {
        for d in 0..division {
            out.push(b[k] + (b[k + 1] - b[k]) * d as f64 / division as f64);
        }
    }
    out.push(b[b.len() - 1]);
    out
}

/// Snaps note starts to the nearest grid position when within `tolerance`.
pub fn snap_to_grid(notes: &mut [Note], grid: &BeatGrid, division: u32, tolerance: f64) {
    let g = subdivisions(grid, division);
    if g.len() < 2 {
        return;
    }
    for n in notes.iter_mut() {
        let k = g
            .iter()
            .enumerate()
            .fold(0, |best, (i, &v)| if (v - n.start).abs() < (g[best] - n.start).abs() { i } else { best });
        if (g[k] - n.start).abs() <= tolerance && g[k] < n.end {
            n.start = g[k];
        }
    }
    for i in 0..notes.len().saturating_sub(1) {
        let next = notes[i + 1].start;
        notes[i].end = notes[i].end.min(next);
    }
}

/// Moves notes by octaves into the instrument's range.
pub fn fit_to_range(notes: &mut [Note], open_strings: &[i32; 4]) {
    let (lo, hi) = (open_strings[0], open_strings[3] + MAX_FRET);
    for n in notes.iter_mut() {
        while n.midi < lo {
            n.midi += 12;
            n.shifted = true;
        }
        while n.midi > hi {
            n.midi -= 12;
            n.shifted = true;
        }
    }
}

/// Settings for [`segment_by_salience`].
#[derive(Debug, Clone, Copy)]
pub struct SalienceSegmentation {
    /// Notes shorter than this are dropped (false onsets, clicks).
    pub min_note_s: f64,
    /// Skipped at the start of a note before its pitch is read.
    pub attack_s: f64,
    /// Skipped at the end of a note before its pitch is read.
    pub tail_s: f64,
    /// Pitch estimates less clear than this (see [`pitch::Estimate::confidence`]) are dropped.
    pub min_confidence: f64,
    /// Segments quieter than this many dB below the loudest are silence.
    pub gate_db: f64,
}

/// Notes from onset times: a note starts at every onset and lasts until the
/// next onset or until its sound dies away; its pitch is the best match of
/// harmonics in its own spectrum. Returns the notes and the recording's
/// tuning offset in semitones.
pub fn segment_by_salience(bass: &[f64], sr: f64, onsets: &[f64], lo: i32, hi: i32, cfg: &SalienceSegmentation) -> (Vec<Note>, f64) {
    use rayon::prelude::*;
    const ENV_WIN: usize = 512;
    const ENV_HOP: usize = 128;
    let env = dsp::rms(bass, ENV_WIN, ENV_HOP);
    let hop_t = ENV_HOP as f64 / sr;
    let loudest = env.iter().copied().fold(0.0, f64::max);
    if loudest <= 0.0 {
        return (Vec::new(), 0.0);
    }
    let gate = loudest * 10f64.powf(cfg.gate_db / 20.0);
    let duration = bass.len() as f64 / sr;

    // [start, end) of every sounding stretch
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for (i, &start) in onsets.iter().enumerate() {
        let limit = onsets.get(i + 1).copied().unwrap_or((start + 1.5).min(duration));
        let (a, b) = (((start / hop_t) as usize).min(env.len()), ((limit / hop_t) as usize).min(env.len()));
        if b <= a {
            continue;
        }
        let peak = env[a..b].iter().copied().fold(0.0, f64::max);
        if peak < gate {
            continue;
        }
        let floor = peak * 10f64.powf(-30.0 / 20.0);
        let last = (a..b).rev().find(|&f| env[f] >= floor).unwrap_or(a);
        let end = (((last + 1) as f64 * hop_t).min(limit)).max(start);
        if end - start >= cfg.min_note_s {
            spans.push((start, end));
        }
    }

    let analyzer = Analyzer::new(bass, sr, lo, hi);
    let window = |&(s, e): &(f64, f64)| {
        let (from, to) = (s + cfg.attack_s, e - cfg.tail_s);
        if to - from >= 0.04 { (from, to) } else { (s, e) }
    };
    // first pass: where does the recording sit against A=440?
    let rough: Vec<pitch::Estimate> = spans
        .par_iter()
        .filter_map(|sp| {
            let (from, to) = window(sp);
            analyzer.estimate(from, to, 0.0)
        })
        .collect();
    let tuning = pitch::tuning_from(&rough);

    let notes: Vec<Option<Note>> = spans
        .par_iter()
        .map(|sp| {
            let (from, to) = window(sp);
            let e = analyzer.estimate(from, to, tuning)?;
            (e.confidence >= cfg.min_confidence).then(|| {
                let mut n = Note::new(sp.0, sp.1, e.midi);
                n.confidence = ((e.confidence - 1.0) / 9.0).clamp(0.0, 1.0);
                n
            })
        })
        .collect();
    (notes.into_iter().flatten().collect(), tuning)
}
