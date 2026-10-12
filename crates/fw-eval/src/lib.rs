//! Scores a detected bass line against an official Rocksmith chart.
//!
//! The chart is the answer key: every detected note is matched one-to-one to a
//! chart note by onset time and pitch, and every miss is given a cause so the
//! detector can be tuned against evidence rather than by ear.

use std::collections::BTreeMap;

use fw_project::Note;
use fw_xml::InstrumentalArrangement;

#[derive(Debug, Clone, PartialEq)]
pub struct RefNote {
    pub start: f64,
    pub end: f64,
    pub midi: i32,
    pub string: i32,
    pub fret: i32,
    pub techniques: Vec<&'static str>,
}

/// Why a chart note was not found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Miss {
    /// A detected note of the right pitch is already sounding over the onset
    /// (a repeated or re-picked note was merged into a longer one).
    Merged,
    /// A note was detected at the right time with the same pitch class but the
    /// wrong octave.
    Octave,
    /// A note was detected at the right time with a different pitch.
    WrongPitch,
    /// Nothing detected near the onset.
    Silent,
}

impl Miss {
    pub fn label(self) -> &'static str {
        match self {
            Miss::Merged => "merged into a held note",
            Miss::Octave => "right note, wrong octave",
            Miss::WrongPitch => "wrong pitch",
            Miss::Silent => "nothing detected",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MissedNote {
    pub note: RefNote,
    pub cause: Miss,
    /// Stem level at the onset in dB below the stem's loudest moment.
    pub level_db: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Score {
    pub reference: usize,
    pub detected: usize,
    pub matched: usize,
    /// Matched pairs whose string and fret agree too.
    pub fretting_agrees: usize,
    /// Onset error (detected - chart) of the matched pairs, in seconds, sorted.
    pub onset_errors: Vec<f64>,
    pub missed: Vec<MissedNote>,
    /// Detected notes that matched nothing.
    pub extra: Vec<Note>,
    /// Constant lag (seconds) removed from the detection before matching.
    pub offset: f64,
    /// Per chart note: whether it was found.
    pub found: Vec<bool>,
    /// Per technique, over the notes found with the right pitch.
    pub techniques: BTreeMap<&'static str, TechniqueScore>,
}

/// How well one technique was detected, judged on notes that matched by pitch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TechniqueScore {
    /// Matched notes the chart marks with it and we marked too.
    pub hit: usize,
    /// Matched notes the chart marks with it and we did not.
    pub missed: usize,
    /// Matched notes we marked and the chart does not.
    pub false_alarm: usize,
}

impl TechniqueScore {
    pub fn chart(&self) -> usize {
        self.hit + self.missed
    }
}

impl Score {
    pub fn recall(&self) -> f64 {
        self.matched as f64 / self.reference.max(1) as f64
    }
    pub fn precision(&self) -> f64 {
        self.matched as f64 / self.detected.max(1) as f64
    }
    pub fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p + r == 0.0 { 0.0 } else { 2.0 * p * r / (p + r) }
    }
    pub fn median_onset_error(&self) -> f64 {
        self.onset_errors.get(self.onset_errors.len() / 2).copied().unwrap_or(0.0)
    }
    pub fn miss_counts(&self) -> BTreeMap<Miss, usize> {
        let mut m = BTreeMap::new();
        for x in &self.missed {
            *m.entry(x.cause).or_insert(0) += 1;
        }
        m
    }
}

/// The notes a player sees (see [`fw_project::import::chart_notes`]).
pub fn reference_notes(arr: &InstrumentalArrangement) -> Vec<RefNote> {
    fw_project::import::chart_notes(arr)
        .into_iter()
        .map(|n| RefNote { start: n.start, end: n.end, midi: n.midi, string: n.string, fret: n.fret, techniques: n.technique_names() })
        .collect()
}

/// Chart notes of a project (as imported from an official chart).
pub fn reference_from_notes(notes: &[Note]) -> Vec<RefNote> {
    notes.iter().map(|n| RefNote { start: n.start, end: n.end, midi: n.midi, string: n.string, fret: n.fret, techniques: n.technique_names() }).collect()
}

/// Greedy one-to-one matching of chart notes to detected notes, closest first.
/// `ok(r, d)` decides which pairs may match.
fn pair(reference: &[RefNote], detected: &[Note], tol: f64, ref_free: &mut [bool], det_free: &mut [bool], ok: impl Fn(&RefNote, &Note) -> bool) -> Vec<(usize, usize)> {
    let mut cand: Vec<(f64, usize, usize)> = Vec::new();
    let mut lo = 0;
    for (ri, r) in reference.iter().enumerate() {
        while lo < detected.len() && detected[lo].start < r.start - tol {
            lo += 1;
        }
        for (di, d) in detected.iter().enumerate().skip(lo) {
            if d.start > r.start + tol {
                break;
            }
            if ref_free[ri] && det_free[di] && ok(r, d) {
                cand.push(((d.start - r.start).abs(), ri, di));
            }
        }
    }
    cand.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut pairs = Vec::new();
    for (_, ri, di) in cand {
        if ref_free[ri] && det_free[di] {
            ref_free[ri] = false;
            det_free[di] = false;
            pairs.push((ri, di));
        }
    }
    pairs
}

/// Number of chart notes matched by pitch within `tol` after shifting the
/// detection by `shift` seconds.
fn matches_at(reference: &[RefNote], detected: &[Note], shift: f64, tol: f64) -> usize {
    let shifted: Vec<Note> = detected.iter().map(|n| Note { start: n.start - shift, end: n.end - shift, ..n.clone() }).collect();
    let (mut rf, mut df) = (vec![true; reference.len()], vec![true; shifted.len()]);
    pair(reference, &shifted, tol, &mut rf, &mut df, |r, d| r.midi == d.midi).len()
}

/// The constant lag of the detection against the chart (seconds), searched
/// within +-`range` in 5 ms steps.
pub fn estimate_offset(reference: &[RefNote], detected: &[Note], range: f64, tol: f64) -> f64 {
    let steps = (range / 0.005).round() as i32;
    let counts: Vec<(f64, usize)> = (-steps..=steps).map(|i| i as f64 * 0.005).map(|s| (s, matches_at(reference, detected, s, tol))).collect();
    let best = counts.iter().map(|c| c.1).max().unwrap_or(0);
    // every shift on the best plateau matches equally: take its middle
    let plateau: Vec<f64> = counts.iter().filter(|c| c.1 == best).map(|c| c.0).collect();
    plateau.get(plateau.len() / 2).copied().unwrap_or(0.0)
}

/// Scores `detected` against `reference`. `stem` (mono samples at `sr`) is
/// only used to report how loud missed notes were.
pub fn score(reference: &[RefNote], detected: &[Note], tol: f64, offset: f64, stem: Option<(&[f64], f64)>) -> Score {
    let det: Vec<Note> = detected.iter().map(|n| Note { start: n.start - offset, end: n.end - offset, ..n.clone() }).collect();
    let (mut rf, mut df) = (vec![true; reference.len()], vec![true; det.len()]);

    let exact = pair(reference, &det, tol, &mut rf, &mut df, |r, d| r.midi == d.midi);
    let octave = pair(reference, &det, tol, &mut rf, &mut df, |r, d| (r.midi - d.midi).rem_euclid(12) == 0);
    let wrong = pair(reference, &det, tol, &mut rf, &mut df, |_, _| true);

    let mut onset_errors: Vec<f64> = exact.iter().map(|&(r, d)| det[d].start - reference[r].start).collect();
    onset_errors.sort_by(|a, b| a.abs().total_cmp(&b.abs()));
    let fretting_agrees = exact.iter().filter(|&&(r, d)| det[d].string == reference[r].string && det[d].fret == reference[r].fret).count();

    let peak = stem.map_or(0.0, |(s, _)| s.iter().fold(0.0f64, |m, v| m.max(v.abs())));
    let level = |t: f64| {
        let (s, sr) = stem?;
        let a = ((t * sr) as usize).min(s.len());
        let b = ((t + 0.05) * sr).min(s.len() as f64) as usize;
        if b <= a || peak <= 0.0 {
            return None;
        }
        let rms = (s[a..b].iter().map(|v| v * v).sum::<f64>() / (b - a) as f64).sqrt();
        Some(20.0 * (rms / peak).max(1e-9).log10())
    };

    let wrong_ref: std::collections::HashMap<usize, usize> = wrong.iter().chain(octave.iter()).copied().collect();
    let octave_ref: std::collections::HashSet<usize> = octave.iter().map(|p| p.0).collect();
    let mut missed = Vec::new();
    for (ri, r) in reference.iter().enumerate() {
        if !rf[ri] && !wrong_ref.contains_key(&ri) {
            continue;
        }
        let cause = if octave_ref.contains(&ri) {
            Miss::Octave
        } else if wrong_ref.contains_key(&ri) {
            Miss::WrongPitch
        } else {
            Miss::Silent
        };
        missed.push(MissedNote { note: r.clone(), cause, level_db: level(r.start) });
    }
    // chart notes with no pairing at all: a detected note of the same pitch
    // sounding over the onset means the repeat was merged into it
    for m in missed.iter_mut().filter(|m| m.cause == Miss::Silent) {
        let t = m.note.start;
        if det.iter().any(|d| d.midi == m.note.midi && d.start < t - tol && d.end > t) {
            m.cause = Miss::Merged;
        }
    }
    let mut techniques: BTreeMap<&'static str, TechniqueScore> = BTreeMap::new();
    for &(r, d) in &exact {
        let (truth, found) = (&reference[r].techniques, det[d].technique_names());
        for t in truth.iter().chain(found.iter()).collect::<std::collections::BTreeSet<_>>() {
            let e = techniques.entry(t).or_default();
            match (truth.contains(t), found.contains(t)) {
                (true, true) => e.hit += 1,
                (true, false) => e.missed += 1,
                _ => e.false_alarm += 1,
            }
        }
    }
    let extra = det.iter().zip(&df).filter(|(_, free)| **free).map(|(d, _)| d.clone()).collect();

    Score {
        reference: reference.len(),
        detected: det.len(),
        matched: exact.len(),
        fretting_agrees,
        onset_errors,
        missed,
        extra,
        offset,
        techniques,
        found: {
            let mut f = vec![false; reference.len()];
            exact.iter().for_each(|&(r, _)| f[r] = true);
            f
        },
    }
}

/// Recall per group of chart notes, `key` choosing each note's group.
pub fn recall_by<K: Ord>(reference: &[RefNote], found: &[bool], key: impl Fn(usize, &RefNote) -> K) -> BTreeMap<K, (usize, usize)> {
    let mut m = BTreeMap::new();
    for (i, r) in reference.iter().enumerate() {
        let e = m.entry(key(i, r)).or_insert((0, 0));
        e.0 += usize::from(found[i]);
        e.1 += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(t: f64, midi: i32) -> RefNote {
        RefNote { start: t, end: t + 0.2, midi, string: 0, fret: 0, techniques: Vec::new() }
    }

    #[test]
    fn scores_hits_octaves_merges_and_silence() {
        let reference = vec![r(1.0, 40), r(2.0, 40), r(3.0, 45), r(5.0, 47), r(6.0, 33)];
        let detected = vec![
            Note::new(1.01, 1.4, 40),  // hit
            Note::new(2.0, 2.2, 52),   // octave up
            Note::new(3.0, 3.2, 41),   // wrong pitch
            Note::new(4.5, 6.5, 47),   // sounds over the 5.0 onset
            Note::new(9.0, 9.2, 40),   // extra
        ];
        let s = score(&reference, &detected, 0.07, 0.0, None);
        assert_eq!(s.matched, 1);
        let c = s.miss_counts();
        assert_eq!(c[&Miss::Octave], 1);
        assert_eq!(c[&Miss::WrongPitch], 1);
        assert_eq!(c[&Miss::Merged], 1);
        assert_eq!(c[&Miss::Silent], 1);
        assert_eq!(s.extra.len(), 2);
    }

    #[test]
    fn finds_a_constant_lag() {
        let reference: Vec<RefNote> = (0..20).map(|i| r(i as f64 * 0.5, 40 + i % 3)).collect();
        let detected: Vec<Note> = reference.iter().map(|n| Note::new(n.start + 0.12, n.start + 0.3, n.midi)).collect();
        let off = estimate_offset(&reference, &detected, 0.3, 0.05);
        assert!((off - 0.12).abs() < 0.006, "{off}");
    }
}
