//! Model-free sanity scores for comparing detector setups against a bass stem.

use fw_project::Note;

/// Energy of `x` at frequency `f` (Goertzel over a Hann-windowed block).
fn goertzel(x: &[f64], f: f64, sr: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * f / sr;
    let coeff = 2.0 * w.cos();
    let n = x.len().max(1) as f64;
    let (mut s1, mut s2) = (0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n).cos();
        let s0 = v * win + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Share of notes whose pitch class is the strongest of the 60 semitones from
/// C1 in the stem while they sound.
pub fn stem_agreement(notes: &[Note], bass: &[f64], sr: f64) -> f64 {
    if notes.is_empty() {
        return 0.0;
    }
    let hits = notes
        .iter()
        .filter(|n| {
            let a = ((n.start * sr) as usize).min(bass.len());
            let b = ((n.end * sr) as usize).clamp(a, bass.len()).max((a + 1024).min(bass.len()));
            let seg = &bass[a..b];
            let best = (24..84)
                .map(|m| (m, goertzel(seg, 440.0 * 2f64.powf((m as f64 - 69.0) / 12.0), sr)))
                .max_by(|x, y| x.1.total_cmp(&y.1))
                .map_or(0, |(m, _)| m);
            (best - n.midi).rem_euclid(12) == 0
        })
        .count();
    hits as f64 / notes.len() as f64
}

/// Share of the time the stem is clearly sounding (within 24 dB of its
/// loudest moment) that a note covers. Complements [`stem_agreement`], which
/// cannot see missed notes.
pub fn stem_coverage(notes: &[Note], bass: &[f64], sr: f64) -> f64 {
    const HOP: usize = 512;
    let frames = bass.len() / HOP + 1;
    let rms: Vec<f64> = (0..frames)
        .map(|i| {
            let c = i * HOP;
            let seg = &bass[c.saturating_sub(1024)..(c + 1024).min(bass.len())];
            (seg.iter().map(|v| v * v).sum::<f64>() / seg.len().max(1) as f64).sqrt()
        })
        .collect();
    let max = rms.iter().copied().fold(0.0, f64::max);
    let active: Vec<bool> = rms.iter().map(|&r| r > max * 10f64.powf(-24.0 / 20.0)).collect();
    let mut covered = vec![false; frames];
    for n in notes {
        let a = ((n.start * sr) as usize / HOP).min(frames);
        let b = ((n.end * sr) as usize / HOP + 1).min(frames);
        covered[a..b.max(a)].iter_mut().for_each(|c| *c = true);
    }
    let on = active.iter().filter(|&&a| a).count();
    active.iter().zip(&covered).filter(|(a, c)| **a && **c).count() as f64 / on.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_a_perfect_and_a_wrong_chart() {
        let sr = 22050.0;
        // A1 (55 Hz) for 1 s, silence, then D2 (73.4 Hz) for 1 s
        let tone = |f: f64, n: usize| (0..n).map(move |i| (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin());
        let mut y: Vec<f64> = tone(55.0, 22050).collect();
        y.extend(std::iter::repeat_n(0.0, 22050));
        y.extend(tone(73.416, 22050));
        let right = vec![Note::new(0.0, 1.0, 33), Note::new(2.0, 3.0, 38)];
        let wrong = vec![Note::new(0.0, 1.0, 35)];
        assert_eq!(stem_agreement(&right, &y, sr), 1.0);
        assert_eq!(stem_agreement(&wrong, &y, sr), 0.0);
        assert!(stem_coverage(&right, &y, sr) > 0.95);
        assert!(stem_coverage(&wrong, &y, sr) < 0.6);
    }
}
