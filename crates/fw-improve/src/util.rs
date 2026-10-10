//! Search helpers shared by the improvers (ports of XML.Processing.Utils and
//! the Rocksmith2014.XML list extensions).

use fw_xml::{Anchor, ChordTemplate, Chord, InstrumentalArrangement, Level, Note};

/// Index of the element at exactly `time`, scanning a time-sorted list and
/// stopping early once past it.
pub fn find_index_by_time<T>(items: &[T], time: i32, t: impl Fn(&T) -> i32) -> Option<usize> {
    for (i, x) in items.iter().enumerate() {
        match t(x).cmp(&time) {
            std::cmp::Ordering::Equal => return Some(i),
            std::cmp::Ordering::Greater => return None,
            std::cmp::Ordering::Less => {}
        }
    }
    None
}

/// Inserts before the first element with a later time (after equal times).
pub fn insert_by_time<T>(items: &mut Vec<T>, item: T, t: impl Fn(&T) -> i32) {
    let time = t(&item);
    match items.iter().position(|x| t(x) > time) {
        Some(i) => items.insert(i, item),
        None => items.push(item),
    }
}

pub fn min_of(options: &[Option<i32>]) -> Option<i32> {
    options.iter().flatten().copied().min()
}

/// "mm:ss.fff" for messages.
pub fn time_to_string(ms: i32) -> String {
    let minutes = ms / 1000 / 60;
    let seconds = ms / 1000 - minutes * 60;
    let millis = ms - minutes * 60_000 - seconds * 1000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}

/// Time of the n-th distinct note/chord time at or after `start`.
pub fn time_of_nth_note_from(level: &Level, start: i32, nth: usize) -> Option<i32> {
    let distinct = |times: &mut dyn Iterator<Item = i32>| -> Vec<i32> {
        let mut out: Vec<i32> = Vec::new();
        for t in times {
            if t >= start && !out.contains(&t) {
                out.push(t);
                if out.len() == nth {
                    break;
                }
            }
        }
        out
    };
    let mut all = distinct(&mut level.chords.iter().map(|c| c.time));
    all.extend(distinct(&mut level.notes.iter().map(|n| n.time)));
    all.sort_unstable();
    all.get(nth.checked_sub(1)?).copied()
}

pub fn first_level_with_content(arr: &InstrumentalArrangement) -> Option<&Level> {
    if arr.levels.len() == 1 {
        return arr.levels.first();
    }
    let first_pi = arr
        .phrase_iterations
        .iter()
        .find(|pi| arr.phrases.get(pi.phrase_id as usize).is_some_and(|p| p.max_difficulty > 0));
    match first_pi {
        Some(pi) => arr.levels.get(arr.phrases[pi.phrase_id as usize].max_difficulty as usize),
        None => arr.levels.iter().find(|l| !l.notes.is_empty() || !l.chords.is_empty()),
    }
}

pub fn first_note_time(arr: &InstrumentalArrangement) -> Option<i32> {
    let level = first_level_with_content(arr)?;
    min_of(&[level.notes.first().map(|n| n.time), level.chords.first().map(|c| c.time)])
}

/// The previous note on the same string (skipping notes at the same time), and
/// whether notes on other strings were passed on the way.
pub fn previous_note_on_same_string(notes: &[Note], start: usize) -> (Option<usize>, bool) {
    let s = &notes[start];
    let mut other_strings = false;
    for i in (0..start).rev() {
        let n = &notes[i];
        if n.time == s.time {
            continue;
        }
        if n.string != s.string {
            other_strings = true;
            continue;
        }
        return (Some(i), other_strings);
    }
    (None, other_strings)
}

/// The last chord before `time` whose template uses `string`.
pub fn previous_chord_using_string<'a>(
    templates: &[ChordTemplate],
    chords: &'a [Chord],
    string: i8,
    time: i32,
) -> Option<&'a Chord> {
    chords.iter().rev().find(|c| {
        c.time < time
            && templates
                .get(c.chord_id as usize)
                .is_some_and(|t| t.frets.get(string as usize).is_some_and(|&f| f > -1))
    })
}

/// The anchor in effect at `time` (the first anchor if all are later).
pub fn active_anchor(level: &Level, time: i32) -> Option<Anchor> {
    if level.anchors.is_empty() {
        return None;
    }
    let idx = level.anchors.iter().position(|a| a.time > time).unwrap_or(level.anchors.len());
    Some(level.anchors[idx.saturating_sub(1)])
}

/// Index of the next note on the same string after `current` (or, when
/// `current` is None, the first later note on that string).
pub fn next_note_on_same_string(notes: &[Note], current: Option<usize>, note: &Note) -> Option<usize> {
    match current {
        None => notes.iter().position(|n| n.time > note.time && n.string == note.string),
        Some(i) => notes.iter().skip(i + 1).position(|n| n.string == note.string).map(|p| p + i + 1),
    }
}

/// Earliest note, chord or hand shape at or after `time`.
pub fn next_content_time(level: &Level, time: i32) -> Option<i32> {
    min_of(&[
        level.notes.iter().find(|n| n.time >= time).map(|n| n.time),
        level.chords.iter().find(|c| c.time >= time).map(|c| c.time),
        level.hand_shapes.iter().find(|h| h.start_time >= time).map(|h| h.start_time),
    ])
}

/// Case-insensitive ASCII prefix test (F#'s `String.startsWith` uses OrdinalIgnoreCase).
pub(crate) fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len()).is_some_and(|x| x.eq_ignore_ascii_case(prefix))
}

