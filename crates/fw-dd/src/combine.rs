//! Finding phrases with the same content and combining them (ports of
//! Comparers.fs and PhraseCombiner.fs).

use std::collections::{HashMap, HashSet};

use rayon::prelude::*;
use fw_xml::{Chord, HeroLevels, NewLinkedDiff, Note, Phrase, PhraseIteration, PhraseMask};

use crate::data::PhraseData;

pub fn same_note(a: &Note, b: &Note) -> bool {
    a.fret == b.fret
        && a.string == b.string
        && a.mask == b.mask
        && a.slide_to == b.slide_to
        && a.slide_unpitch_to == b.slide_unpitch_to
        && a.vibrato == b.vibrato
        && a.tap == b.tap
        && a.max_bend == b.max_bend
}

pub fn same_chord(a: &Chord, b: &Chord) -> bool {
    a.chord_id == b.chord_id
        && a.mask == b.mask
        && match (&a.chord_notes, &b.chord_notes) {
            (None, None) => true,
            (Some(x), Some(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_note(p, q)),
            _ => false,
        }
}

/// Literal port of Comparers.findNextMatch (F# lists become slices; the
/// "skipped" lists are built by prepending, so index 0 is the most recent).
fn find_next_match<'a, T>(
    eq: &impl Fn(&T, &T) -> bool,
    mut skipped1: Vec<&'a T>,
    mut skipped2: Vec<&'a T>,
    list1: &'a [T],
    list2: &'a [T],
) -> Option<(&'a [T], &'a [T])> {
    let (mut rem1, mut rem2) = (list1, list2);
    loop {
        let ([h1, t1 @ ..], [h2, t2 @ ..]) = (rem1, rem2) else { return None };
        if eq(h1, h2) {
            return Some((t1, t2));
        }
        if let Some(i) = skipped1.iter().position(|x| eq(h2, x)) {
            return Some((list1.get(i..).unwrap_or(&[]), t2));
        }
        if let Some(j) = skipped2.iter().position(|x| eq(h1, x)) {
            return Some((t1, list2.get(j..).unwrap_or(&[])));
        }
        skipped1.insert(0, h1);
        skipped2.insert(0, h2);
        rem1 = t1;
        rem2 = t2;
    }
}

/// Number of equal items in two lists where order matters.
pub fn same_item_count<T>(eq: impl Fn(&T, &T) -> bool, a: &[T], b: &[T]) -> usize {
    let (mut count, mut l1, mut l2) = (0usize, a, b);
    loop {
        let ([h1, t1 @ ..], [h2, t2 @ ..]) = (l1, l2) else { return count };
        if eq(h1, h2) {
            count += 1;
            l1 = t1;
            l2 = t2;
        } else if l1.len() > l2.len() {
            l1 = t1;
        } else if l1.len() < l2.len() {
            l2 = t2;
        } else {
            match find_next_match(&eq, vec![h1], vec![h2], t1, t2) {
                None => return count,
                Some((n1, n2)) => {
                    count += 1;
                    l1 = n1;
                    l2 = n2;
                }
            }
        }
    }
}

fn similarity_percent<T>(eq: impl Fn(&T, &T) -> bool, a: &[T], b: &[T]) -> f64 {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => 100.0,
        (true, false) | (false, true) => 0.0,
        _ => 100.0 * same_item_count(eq, a, b) as f64 / a.len().max(b.len()) as f64,
    }
}

fn max_similarity_fastest<T>(a: &[T], b: &[T]) -> f64 {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => 100.0,
        (true, false) | (false, true) => 0.0,
        _ => 100.0 * a.len().min(b.len()) as f64 / a.len().max(b.len()) as f64,
    }
}

fn max_similarity_fast<T, K: std::hash::Hash + Eq>(proj: impl Fn(&T) -> K, a: &[T], b: &[T]) -> f64 {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => 100.0,
        (true, false) | (false, true) => 0.0,
        _ => {
            let count = |l: &[T]| {
                let mut m: HashMap<K, usize> = HashMap::new();
                for x in l {
                    *m.entry(proj(x)).or_default() += 1;
                }
                m
            };
            let (m1, m2) = (count(a), count(b));
            let same: usize = m1.iter().map(|(k, c1)| m2.get(k).map_or(0, |c2| (*c1).min(*c2))).sum();
            // upstream divides by max(l1.Length, l1.Length), i.e. the first list's length
            100.0 * same as f64 / a.len() as f64
        }
    }
}

fn note_projection(n: &Note) -> i16 {
    (n.string as i16) | ((n.fret as i16) << 8)
}

fn weighted(p1: &PhraseData, p2: &PhraseData, notes: f64, chords: f64) -> i32 {
    let nt = (p1.note_count() + p2.note_count()) as f64;
    let ct = (p1.chord_count() + p2.chord_count()) as f64;
    // F# `round` rounds halves to even
    ((notes * nt + chords * ct) / (nt + ct)).round_ties_even() as i32
}

fn similarity(threshold: i32, p1: &PhraseData, p2: &PhraseData) -> i32 {
    if p1.is_empty() && p2.is_empty() {
        return 100;
    }
    let fastest = weighted(
        p1,
        p2,
        max_similarity_fastest(&p1.notes, &p2.notes),
        max_similarity_fastest(&p1.chords, &p2.chords),
    );
    let fast = if fastest < threshold {
        0
    } else {
        weighted(
            p1,
            p2,
            max_similarity_fast(note_projection, &p1.notes, &p2.notes),
            max_similarity_fast(|c: &Chord| c.chord_id, &p1.chords, &p2.chords),
        )
    };
    if fast < threshold {
        return 0;
    }
    weighted(p1, p2, similarity_percent(same_note, &p1.notes, &p2.notes), similarity_percent(same_chord, &p1.chords, &p2.chords))
}

struct Combination {
    main: usize,
    ids: Vec<usize>,
    same_difficulty: bool,
}

fn find_same_phrases(threshold: i32, level_counts: &[usize], data: &[PhraseData]) -> Vec<Combination> {
    let last = data.len() - 1;
    let mut matched: HashSet<usize> = [0, last].into_iter().collect();
    let mut out = Vec::new();
    for main in 0..data.len() {
        if matched.contains(&main) {
            continue;
        }
        let ids: Vec<usize> = (main + 1..data.len())
            .into_par_iter()
            .filter(|&j| !matched.contains(&j) && similarity(threshold, &data[main], &data[j]) >= threshold)
            .collect();
        if ids.is_empty() {
            continue;
        }
        matched.extend(ids.iter().copied());
        let same = std::iter::once(main).chain(ids.iter().copied()).all(|i| level_counts[i] == level_counts[main]);
        out.push(Combination { main, ids, same_difficulty: same });
    }
    out
}

fn find_empty_phrases(data: &[PhraseData]) -> Vec<Combination> {
    let empty: Vec<usize> = (1..data.len().saturating_sub(1)).filter(|&i| data[i].is_empty()).collect();
    if empty.len() > 1 {
        vec![Combination { main: empty[0], ids: empty[1..].to_vec(), same_difficulty: true }]
    } else {
        Vec::new()
    }
}

pub fn combine_same_phrases(
    threshold: Option<i32>,
    data: &[PhraseData],
    iterations: &[PhraseIteration],
    level_counts: &[usize],
) -> (Vec<Phrase>, Vec<PhraseIteration>, Vec<NewLinkedDiff>) {
    let (same, different): (Vec<Combination>, Vec<Combination>) = match threshold {
        None => (find_empty_phrases(data), Vec::new()),
        Some(t) => find_same_phrases(t, level_counts, data).into_iter().partition(|c| c.same_difficulty),
    };

    // new phrase ids after combining phrases with the same content
    let mut next_id: i32 = -1;
    let mut by_main: HashMap<usize, i32> = HashMap::new();
    let new_ids: Vec<i32> = (0..iterations.len())
        .map(|id| match same.iter().find(|c| c.main == id || c.ids.contains(&id)) {
            Some(c) => *by_main.entry(c.main).or_insert_with(|| {
                next_id += 1;
                next_id
            }),
            None => {
                next_id += 1;
                next_id
            }
        })
        .collect();

    let new_iterations: Vec<PhraseIteration> = iterations
        .iter()
        .zip(level_counts)
        .enumerate()
        .map(|(i, (old, &count))| {
            let max = (count - 1) as u8;
            let hero_levels = if max > 0 {
                let (easy, medium) = if max == 1 {
                    (max, max) // score attack crashes on easy/medium otherwise
                } else {
                    ((max as f64 / 4.0).round_ties_even() as u8, (max as f64 / 2.0).round_ties_even() as u8)
                };
                HeroLevels { easy, medium, hard: max }
            } else {
                HeroLevels::default()
            };
            PhraseIteration { time: old.time, phrase_id: new_ids[i], variation: String::new(), hero_levels }
        })
        .collect();

    let linked = different
        .iter()
        .map(|c| NewLinkedDiff {
            level_break: -1,
            ratio: None,
            phrase_ids: std::iter::once(new_ids[c.main]).chain(c.ids.iter().map(|&i| new_ids[i])).collect(),
        })
        .collect();

    let first = new_iterations[0].phrase_id;
    let last = new_iterations[new_iterations.len() - 1].phrase_id;
    let mut created = HashSet::new();
    let mut counter = -1;
    let phrases = new_iterations
        .iter()
        .filter(|pi| created.insert(pi.phrase_id))
        .map(|pi| {
            let max = pi.hero_levels.hard;
            let name = if pi.phrase_id == first {
                "COUNT".to_string()
            } else if pi.phrase_id == last {
                "END".to_string()
            } else if max == 0 {
                "NOGUITAR".to_string()
            } else {
                counter += 1;
                format!("p{counter}")
            };
            Phrase { name, max_difficulty: max, mask: PhraseMask::empty() }
        })
        .collect();

    (phrases, new_iterations, linked)
}
