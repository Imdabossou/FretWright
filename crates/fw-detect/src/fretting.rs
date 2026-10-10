//! String/fret choice minimising fretting-hand movement (Viterbi).

use std::collections::HashSet;

use fw_project::{MAX_FRET, Note};

fn options(midi: i32, open: &[i32; 4]) -> Vec<(i32, i32)> {
    (0..4).filter(|&s| (0..=MAX_FRET).contains(&(midi - open[s]))).map(|s| (s as i32, midi - open[s])).collect()
}

fn local_cost(_string: i32, fret: i32) -> f64 {
    0.12 * fret as f64 + if fret == 0 { 0.0 } else { 0.1 }
}

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
    if notes.is_empty() {
        return;
    }
    let opts: Vec<Vec<(i32, i32)>> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let o = if locked.contains(&i) { vec![(n.string, n.fret)] } else { options(n.midi, open) };
            if o.is_empty() { vec![(0, (n.midi - open[0]).max(0))] } else { o }
        })
        .collect();
    let mut cost: Vec<Vec<f64>> = vec![opts[0].iter().map(|&(s, f)| local_cost(s, f)).collect()];
    let mut back: Vec<Vec<usize>> = vec![vec![0; opts[0].len()]];
    for i in 1..notes.len() {
        // a long rest lets the hand move freely
        let w = if notes[i].start - notes[i - 1].end > 1.0 { 0.3 } else { 1.0 };
        let mut row = Vec::with_capacity(opts[i].len());
        let mut brow = Vec::with_capacity(opts[i].len());
        for &o in &opts[i] {
            let (mut best, mut arg) = (f64::INFINITY, 0);
            for (j, &p) in opts[i - 1].iter().enumerate() {
                let c = cost[i - 1][j] + w * move_cost(p, o);
                if c < best {
                    best = c;
                    arg = j;
                }
            }
            row.push(best + local_cost(o.0, o.1));
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
