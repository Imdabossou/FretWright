//! String/fret choice that keeps the fretting hand where it is (Viterbi): moves
//! within reach of the fingers are cheap, shifting position is expensive, and
//! open strings leave the hand in place. How a note sounds (see `timbre`) only
//! tips the balance between equally comfortable choices.

use std::collections::HashSet;

use fw_project::{MAX_FRET, Note};

fn options(midi: i32, open: &[i32; 4]) -> Vec<(i32, i32)> {
    (0..4).filter(|&s| (0..=MAX_FRET).contains(&(midi - open[s]))).map(|s| (s as i32, midi - open[s])).collect()
}

/// Frets the fingers reach without moving the hand.
const REACH: f64 = 3.0;
/// Cost per fret of a move within reach.
const PER_FRET: f64 = 0.15;
/// Cost per fret of shifting the hand beyond its reach.
const LEAP: f64 = 2.0;
/// Where the hand is assumed to be before the first fretted note: near the nut.
const HOME: i32 = 1;
/// A slight preference for lower frets, when nothing else decides.
const LOW_FRETS: f64 = 0.03;

/// Cost of playing a note at `fret` with the hand at `hand` (its last fretted position, or
/// near the nut before any),
/// after a note on string `from_string`. Moves within reach of the fingers are cheap,
/// shifting the hand further is not, and crossing strings is easy.
fn move_cost(hand: Option<i32>, from_string: i32, string: i32, fret: i32) -> f64 {
    let strings = 0.15 * (string - from_string).abs() as f64;
    if fret == 0 {
        // an open string does not need the fretting hand
        return strings;
    }
    let d = (fret - hand.unwrap_or(HOME)).abs() as f64;
    strings + if d <= REACH { PER_FRET * d } else { PER_FRET * REACH + LEAP * (d - REACH) }
}

/// How much a note's sound counts: a hint for when hand position does not decide.
const SOUND_WEIGHT: f64 = 1.0;
/// The most one note's sound can count (in log-probability units).
const SOUND_CAP: f64 = 1.0;

/// Assigns string and fret to every note; `locked` indices keep theirs.
pub fn assign(notes: &mut [Note], open: &[i32; 4], locked: &HashSet<usize>) {
    assign_with(notes, open, locked, None);
}

/// Like [`assign`]; `sound[i]` holds, when known, the log-probability of each string for note `i`
/// judging by how it sounds (see `timbre`). It only tips the balance: where the hand is comes first.
pub fn assign_with(notes: &mut [Note], open: &[i32; 4], locked: &HashSet<usize>, sound: Option<&[Option<[f32; 4]>]>) {
    if notes.is_empty() {
        return;
    }
    let local = |i: usize, s: i32, f: i32| -> f64 {
        let low_frets = LOW_FRETS * f as f64;
        match sound.and_then(|x| x.get(i)).copied().flatten() {
            // capped, so that however sure the sound is, it never pays for a leap
            Some(lp) if lp[s as usize].is_finite() => low_frets + SOUND_WEIGHT * (-(lp[s as usize] as f64)).min(SOUND_CAP),
            _ => low_frets,
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
    // Viterbi over each note's options; each path also carries where the hand is
    let mut cost: Vec<Vec<f64>> = vec![opts[0].iter().map(|&(s, f)| local(0, s, f) + move_cost(None, s, s, f)).collect()];
    let mut hand: Vec<Vec<Option<i32>>> = vec![opts[0].iter().map(|&(_, f)| (f > 0).then_some(f)).collect()];
    let mut back: Vec<Vec<usize>> = vec![vec![0; opts[0].len()]];
    for i in 1..notes.len() {
        // a long rest lets the hand move freely
        let w = if notes[i].start - notes[i - 1].end > 1.0 { 0.1 } else { 1.0 };
        let (mut row, mut hrow, mut brow) = (Vec::new(), Vec::new(), Vec::new());
        for &(s, f) in &opts[i] {
            let (mut best, mut arg) = (f64::INFINITY, 0);
            for (j, &(ps, _)) in opts[i - 1].iter().enumerate() {
                let c = cost[i - 1][j] + w * move_cost(hand[i - 1][j], ps, s, f);
                if c < best {
                    best = c;
                    arg = j;
                }
            }
            row.push(best + local(i, s, f));
            hrow.push(if f > 0 { Some(f) } else { hand[i - 1][arg] });
            brow.push(arg);
        }
        cost.push(row);
        hand.push(hrow);
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

    /// The largest move between consecutive fretted notes.
    fn largest_move(notes: &[Note]) -> i32 {
        let frets: Vec<i32> = notes.iter().filter(|n| n.fret > 0).map(|n| n.fret).collect();
        frets.windows(2).map(|w| (w[1] - w[0]).abs()).max().unwrap_or(0)
    }

    #[test]
    fn a_line_across_strings_does_not_leap() {
        let open = [28, 33, 38, 43];
        let mut notes: Vec<Note> = [33, 36, 38, 40, 43, 45, 48, 50].iter().enumerate().map(|(i, &m)| note(m, i as f64 * 0.25)).collect();
        assign(&mut notes, &open, &HashSet::new());
        assert!(largest_move(&notes) <= 4, "{:?}", notes.iter().map(|n| (n.string, n.fret)).collect::<Vec<_>>());
    }

    #[test]
    fn a_misleading_sound_does_not_pull_the_hand_up_the_neck() {
        let open = [28, 33, 38, 43];
        // A1 D2 E2 D2 played around the 5th fret; the sound wrongly prefers E2 on the E string (fret 12)
        let mut notes: Vec<Note> = [33, 38, 40, 38].iter().enumerate().map(|(i, &m)| note(m, i as f64 * 0.25)).collect();
        let mut opinion = vec![None; 4];
        opinion[2] = Some([-0.2, -2.0, -2.5, f32::NEG_INFINITY]);
        assign_with(&mut notes, &open, &HashSet::new(), Some(&opinion));
        assert_ne!((notes[2].string, notes[2].fret), (0, 12));
        assert!(largest_move(&notes) <= 4, "{:?}", notes.iter().map(|n| (n.string, n.fret)).collect::<Vec<_>>());
    }

    #[test]
    fn where_the_hand_does_not_decide_the_sound_does() {
        let open = [28, 33, 38, 43];
        // C2 at the 3rd fret of the A string, then D2: the open D string and the
        // 5th fret of the A string are both in reach; the sound says A string
        let mut notes = vec![note(36, 0.0), note(38, 0.25)];
        notes[0].string = 1;
        notes[0].fret = 3;
        let opinion = [None, Some([f32::NEG_INFINITY, -0.05, -3.0, f32::NEG_INFINITY])];
        assign_with(&mut notes, &open, &[0].into(), Some(&opinion));
        assert_eq!((notes[1].string, notes[1].fret), (1, 5));
    }
}
