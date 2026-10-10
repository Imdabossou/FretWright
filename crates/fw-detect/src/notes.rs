//! Frame-level pitch track -> clean, playable notes.

use std::collections::HashSet;

use fw_project::{BeatGrid, MAX_FRET, Note};

use crate::dsp;

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
