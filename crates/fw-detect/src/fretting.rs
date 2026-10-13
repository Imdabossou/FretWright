//! String/fret choice minimising fretting-hand movement (Viterbi).

use std::collections::HashSet;

use fw_project::{MAX_FRET, Note};

fn options(midi: i32, open: &[i32; 4]) -> Vec<(i32, i32)> {
    (0..4).filter(|&s| (0..=MAX_FRET).contains(&(midi - open[s]))).map(|s| (s as i32, midi - open[s])).collect()
}

fn local_cost(_string: i32, fret: i32) -> f64 {
    0.12 * fret as f64 + if fret == 0 { 0.0 } else { 0.1 }
}

/// How much a note's sound counts against the cost of moving the hand.
const SOUND_WEIGHT: f64 = 8.0;
/// Hand movement counts for less once the sound says which string it was.
const MOVE_SCALE: f64 = 1.0;

fn move_cost(a: (i32, i32), b: (i32, i32)) -> f64 {
    // open strings do not need the fretting hand
    if a.1 == 0 || b.1 == 0 {
        0.2 * (a.0 - b.0).abs() as f64
    } else {
        0.9 * (a.1 - b.1).abs() as f64 + 0.25 * (a.0 - b.0).abs() as f64
    }
}

/// Assigns string and fret to every note; `locked` indices keep theirs.
pub fn assign(notes: &mut [Note], open: &[i32; 4], locked: &HashSet<usize>) {
    assign_with(notes, open, locked, None);
}

/// Like [`assign`]; `sound[i]` holds, when known, the log-probability of each string for note `i`
/// judging by how it sounds (see `timbre`). Notes without it are placed by hand position alone.
pub fn assign_with(notes: &mut [Note], open: &[i32; 4], locked: &HashSet<usize>, sound: Option<&[Option<[f32; 4]>]>) {
    if notes.is_empty() {
        return;
    }
    let (weight, move_scale) = (SOUND_WEIGHT, MOVE_SCALE);
    let local = |i: usize, s: i32, f: i32| -> f64 {
        match sound.and_then(|x| x.get(i)).copied().flatten() {
            Some(lp) if lp[s as usize].is_finite() => weight * -(lp[s as usize] as f64) + 0.02 * f as f64,
            Some(_) => 5.0,
            None => if sound.is_some() { 0.02 * f as f64 } else { local_cost(s, f) },
        }
    };
    let opts: Vec<Vec<(i32, i32)>> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let o = if locked.contains(&i) { vec![(n.string, n.fret)] } else { options(n.midi, open) };
            if o.is_empty() { vec![(0, (n.midi - open[0]).max(0))] } else { o }
        })
        .collect();
    let mut cost: Vec<Vec<f64>> = vec![opts[0].iter().map(|&(s, f)| local(0, s, f)).collect()];
    let mut back: Vec<Vec<usize>> = vec![vec![0; opts[0].len()]];
    for i in 1..notes.len() {
        // a long rest lets the hand move freely
        let w = if notes[i].start - notes[i - 1].end > 1.0 { 0.3 } else { 1.0 };
        let mut row = Vec::with_capacity(opts[i].len());
        let mut brow = Vec::with_capacity(opts[i].len());
        for &o in &opts[i] {
            let (mut best, mut arg) = (f64::INFINITY, 0);
            for (j, &p) in opts[i - 1].iter().enumerate() {
                let c = cost[i - 1][j] + w * move_scale * move_cost(p, o);
                if c < best {
                    best = c;
                    arg = j;
                }
            }
            row.push(best + local(i, o.0, o.1));
            brow.push(arg);
        }
        cost.push(row);
        back.push(brow);
    }
    let last = cost.len() - 1;
    let mut k = (0..cost[last].len()).fold(0, |b, i| if cost[last][i] < cost[last][b] { i } else { b });
    for i in (0..notes.len()).rev() {
        (notes[i].string, notes[i].fret) = opts[i][k];
        k = back[i][k];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(midi: i32, t: f64) -> Note {
        Note::new(t, t + 0.2, midi)
    }

    #[test]
    fn without_a_sound_opinion_low_frets_win() {
        let open = [28, 33, 38, 43];
        let mut notes = vec![note(38, 0.0)]; // D2: open D string, or fret 5 on A, fret 10 on E
        assign(&mut notes, &open, &HashSet::new());
        assert_eq!((notes[0].string, notes[0].fret), (2, 0));
    }

    #[test]
    fn a_strong_sound_opinion_moves_the_note_to_another_string() {
        let open = [28, 33, 38, 43];
        let mut notes = vec![note(38, 0.0)];
        // the sound says it was played on the A string (fret 5)
        let opinion = [Some([f32::NEG_INFINITY, -0.05, -3.0, f32::NEG_INFINITY])];
        assign_with(&mut notes, &open, &HashSet::new(), Some(&opinion));
        assert_eq!((notes[0].string, notes[0].fret), (1, 5));
    }
}
