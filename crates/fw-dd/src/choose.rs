//! Note scoring and the choosers that decide what each difficulty level keeps
//! (ports of NoteScorer, EntityChooser, HandShapeChooser and AnchorChooser).

use std::collections::{BTreeMap, HashMap};

use fw_xml::{Anchor, Chord, ChordMask, ChordTemplate, Ebeat, HandShape, Note, NoteMask};

use crate::data::PhraseData;

#[derive(Debug, Clone)]
pub enum Ent {
    Note(Note),
    Chord(Chord),
}

impl Ent {
    pub fn time(&self) -> i32 {
        match self {
            Ent::Note(n) => n.time,
            Ent::Chord(c) => c.time,
        }
    }
    pub fn sustain(&self) -> i32 {
        match self {
            Ent::Note(n) => n.sustain,
            Ent::Chord(c) => c.chord_notes.as_ref().and_then(|cn| cn.first()).map_or(0, |n| n.sustain),
        }
    }
}

/// Notes and chords merged and sorted by time (stable: notes first on ties).
pub fn entities(notes: &[Note], chords: &[Chord]) -> Vec<Ent> {
    let mut v: Vec<Ent> = notes.iter().cloned().map(Ent::Note).chain(chords.iter().cloned().map(Ent::Chord)).collect();
    v.sort_by_key(Ent::time);
    v
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub low: f64,
    pub high: f64,
}

/// A request to replace a chord template with a reduced one.
#[derive(Debug, Clone, Copy)]
pub struct TemplateRequest {
    pub original_id: i16,
    pub note_count: u8,
    pub from_highest_note: bool,
}

// ------------------------------------------------------------------ scoring

fn round_away(v: f64) -> f64 {
    v.round() // Rust rounds halves away from zero
}

fn subdivision(start: i32, end: i32, time: i32) -> i32 {
    let dist = (end - start) as f64;
    let pos = (time - start) as f64;
    let mid = dist / 2.0;
    let pos = if pos - mid > 10.0 { pos - mid } else { pos };
    round_away(dist / pos) as i32
}

fn subdivision_inside_measure(phrase_end: i32, beats: &[Ebeat], time: i32) -> i32 {
    let Some(first) = beats.iter().rev().find(|b| b.time < time && b.measure != -1) else { return 2 };
    let end = beats.iter().find(|b| b.time > time && b.measure != -1).map_or(phrase_end, |b| b.time);
    subdivision(first.time, end, time)
}

const PHRASE_DIVISIONS: [i32; 9] = [0, 3, 2, 3, 1, 3, 2, 3, 4];

fn division_in_phrase(start: i32, end: i32, time: i32) -> i32 {
    let distance = (end - start) as f64;
    let div_len = distance / PHRASE_DIVISIONS.len() as f64;
    let pos = (time - start) as f64;
    let mut i = 0usize;
    while i < PHRASE_DIVISIONS.len() {
        let low = div_len * i as f64;
        let high = div_len * (i + 1) as f64;
        if pos >= low && pos < high {
            return PHRASE_DIVISIONS[i];
        }
        i += 1;
    }
    PHRASE_DIVISIONS[PHRASE_DIVISIONS.len() - 1]
}

fn technique_penalty(e: &Ent) -> i32 {
    let (mute, bend) = match e {
        Ent::Note(n) => (n.mask.contains(NoteMask::FRET_HAND_MUTE), n.max_bend > 0.0),
        Ent::Chord(c) => (c.mask.contains(ChordMask::FRET_HAND_MUTE), false),
    };
    if mute {
        20
    } else if bend {
        10
    } else {
        0
    }
}

/// Lower scores are "easier" (on strong beats); they survive to lower levels.
pub fn score(p: &PhraseData, time: i32, e: &Ent) -> i32 {
    let b1 = p.beats.iter().rev().find(|b| b.time <= time);
    let b2 = p.beats.iter().find(|b| b.time >= time);
    let rhythm = match (b1, b2) {
        (None, _) => 20,
        (Some(b1), Some(b2)) => {
            if time == b1.time {
                if b1.measure >= 0 { 0 } else { subdivision_inside_measure(p.end_time, &p.beats, time) }
            } else {
                10 * subdivision(b1.time, b2.time, time)
            }
        }
        (Some(b1), None) => 10 * subdivision(b1.time, p.end_time, time),
    };
    technique_penalty(e) + division_in_phrase(p.start_time, p.end_time, time) + rhythm
}

/// Maps each score to the slice of the difficulty range (0..1) it occupies.
pub fn score_map(scores: &[(i32, i32)], total: usize) -> BTreeMap<i32, Range> {
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for (_, s) in scores {
        *counts.entry(*s).or_default() += 1;
    }
    let mut out = BTreeMap::new();
    let mut low = 0.0;
    for (s, n) in counts {
        let high = low + n as f64 / total as f64;
        out.insert(s, Range { low, high });
        low = high;
    }
    out
}

// ------------------------------------------------------------------ helpers

pub fn note_count(t: &ChordTemplate) -> usize {
    t.frets.iter().filter(|&&f| f >= 0).count()
}

pub fn allowed_chord_notes(diff_percent: f64, max_chord_notes: usize) -> usize {
    if max_chord_notes == 0 { 0 } else { (diff_percent * max_chord_notes as f64).ceil() as usize }
}

/// Double and triple stops on the highest strings keep their highest notes.
pub fn start_from_highest(total: usize, t: &ChordTemplate) -> bool {
    let f = &t.frets;
    (total == 3 && f[5] > -1 && f[4] > -1 && f[3] > -1) || (total == 2 && f[5] > -1 && f[4] > -1)
}

fn request(original_id: i16, note_count: usize, total: usize, t: &ChordTemplate) -> TemplateRequest {
    TemplateRequest { original_id, note_count: note_count as u8, from_highest_note: start_from_highest(total, t) }
}

// ------------------------------------------------------------------ entity chooser

const ALWAYS_ENABLED: NoteMask = NoteMask::IGNORE.union(NoteMask::FRET_HAND_MUTE).union(NoteMask::PALM_MUTE).union(NoteMask::HARMONIC);

fn prune_techniques(diff: f64, removed_link_nexts: &mut [bool; 6], n: &mut Note) {
    if diff <= 0.2 {
        if !n.is_bend() {
            n.sustain = 0;
        }
        if n.mask.contains(NoteMask::LINK_NEXT) {
            removed_link_nexts[n.string as usize] = true;
            n.mask.remove(NoteMask::LINK_NEXT);
        }
        n.mask &= ALWAYS_ENABLED;
        n.vibrato = 0;
        n.slide_to = -1;
        n.slide_unpitch_to = -1;
    } else if diff <= 0.35 && n.mask.contains(NoteMask::TREMOLO) && !n.is_tap() {
        n.mask.remove(NoteMask::TREMOLO);
    } else if diff <= 0.45 && n.is_vibrato() && !n.is_tap() {
        n.vibrato = 0;
    }
    if diff <= 0.60 && n.mask.contains(NoteMask::PINCH_HARMONIC) {
        n.mask.remove(NoteMask::PINCH_HARMONIC);
    }
}

/// Where a pending link-next note lives: the chosen entity and, for chords, the chord note.
#[derive(Clone, Copy)]
struct Pending {
    entity: usize,
    chord_note: Option<usize>,
}

fn pending_note(acc: &mut [(Ent, Option<TemplateRequest>)], p: Pending) -> Option<&mut Note> {
    match (&mut acc[p.entity].0, p.chord_note) {
        (Ent::Note(n), None) => Some(n),
        (Ent::Chord(c), Some(i)) => c.chord_notes.as_mut().and_then(|cn| cn.get_mut(i)),
        _ => None,
    }
}

fn should_exclude(
    diff: f64,
    score: i32,
    notes_with_score: &HashMap<i32, usize>,
    current: &HashMap<i32, usize>,
    r: Range,
) -> bool {
    if diff < r.low {
        true
    } else if diff < r.high {
        let notes = notes_with_score[&score];
        let current = current.get(&score).copied().unwrap_or(0);
        let allowed_percent = (diff - r.low) / (r.high - r.low);
        let allowed = round_away(notes as f64 * allowed_percent);
        let min_notes = if r.low == 0.0 { 1 } else { 0 };
        current + 1 > (allowed as usize).max(min_notes)
    } else {
        false
    }
}

fn has_string(e: &Ent, string: i8) -> bool {
    match e {
        Ent::Note(n) => n.string == string,
        Ent::Chord(c) => c.chord_notes.as_ref().is_some_and(|cn| cn.iter().any(|n| n.string == string)),
    }
}

fn chord_notes_from_template(t: &ChordTemplate, c: &Chord) -> Vec<Note> {
    (0..6)
        .filter(|&i| t.frets[i] != -1)
        .map(|i| {
            let mut n = Note { time: c.time, string: i as i8, fret: t.frets[i], left_hand: t.fingers[i], ..Default::default() };
            n.mask.set(NoteMask::FRET_HAND_MUTE, c.mask.contains(ChordMask::FRET_HAND_MUTE));
            n.mask.set(NoteMask::PALM_MUTE, c.mask.contains(ChordMask::PALM_MUTE));
            n.mask.set(NoteMask::ACCENT, c.mask.contains(ChordMask::ACCENT));
            n
        })
        .collect()
}

fn note_from_chord(diff: f64, removed: &mut [bool; 6], total: usize, t: &ChordTemplate, c: &Chord) -> Note {
    let from_highest = start_from_highest(total, t);
    match c.chord_notes.as_ref().filter(|cn| !cn.is_empty()) {
        Some(cns) => {
            let index = if from_highest { cns.len() - 1 } else { 0 };
            for (i, cn) in cns.iter().enumerate() {
                if i != index && cn.mask.contains(NoteMask::LINK_NEXT) {
                    removed[cn.string as usize] = true;
                }
            }
            let mut n = Note { left_hand: -1, ..cns[index].clone() };
            prune_techniques(diff, removed, &mut n);
            n
        }
        None => {
            let pos = |f: &i8| *f != -1;
            let s = if from_highest { t.frets.iter().rposition(pos) } else { t.frets.iter().position(pos) }.unwrap_or(0);
            let mut n = Note { time: c.time, string: s as i8, fret: t.frets[s], ..Default::default() };
            n.mask.set(NoteMask::FRET_HAND_MUTE, c.mask.contains(ChordMask::FRET_HAND_MUTE));
            n.mask.set(NoteMask::PALM_MUTE, c.mask.contains(ChordMask::PALM_MUTE));
            n.mask.set(NoteMask::ACCENT, c.mask.contains(ChordMask::ACCENT));
            n.mask.set(NoteMask::IGNORE, c.mask.contains(ChordMask::IGNORE));
            n
        }
    }
}

pub struct ChooseInput<'a> {
    pub diff_percent: f64,
    pub score_map: &'a BTreeMap<i32, Range>,
    pub time_to_score: &'a HashMap<i32, i32>,
    pub notes_with_score: &'a HashMap<i32, usize>,
    pub templates: &'a [ChordTemplate],
    pub hand_shapes: &'a [HandShape],
    pub max_chord_notes: usize,
    pub entities: &'a [Ent],
}

pub fn choose_entities(inp: &ChooseInput) -> Vec<(Ent, Option<TemplateRequest>)> {
    let diff = inp.diff_percent;
    let mut removed = [false; 6];
    let mut pending: [Option<Pending>; 6] = [None; 6];
    let mut current: HashMap<i32, usize> = HashMap::new();
    let allowed_chord_notes = allowed_chord_notes(diff, inp.max_chord_notes);
    let mut acc: Vec<(Ent, Option<TemplateRequest>)> = Vec::new();

    for e in inp.entities {
        let score = inp.time_to_score[&e.time()];
        let range = inp.score_map[&score];
        let include_always = match e {
            Ent::Chord(_) => false,
            Ent::Note(n) => {
                (n.mask - (NoteMask::IGNORE | NoteMask::LINK_NEXT)).is_empty()
                    && pending[n.string as usize].is_some()
                    && !(n.is_slide() || n.is_unpitched_slide() || n.is_bend() || n.is_vibrato())
            }
        };

        if !include_always && should_exclude(diff, score, inp.notes_with_score, &current, range) {
            // drop the link-next pointing at this note
            if let Ent::Note(n) = e {
                let s = n.string as usize;
                if let Some(p) = pending[s].take()
                    && let Some(ln) = pending_note(&mut acc, p)
                {
                    if ln.is_slide() {
                        ln.slide_to = -1;
                        ln.sustain = 0;
                    }
                    ln.mask.remove(NoteMask::LINK_NEXT);
                }
            }
            match e {
                Ent::Note(n) => removed[n.string as usize] = n.mask.contains(NoteMask::LINK_NEXT) && !n.is_slide(),
                Ent::Chord(c) if c.mask.contains(ChordMask::LINK_NEXT) => {
                    for cn in c.chord_notes.iter().flatten() {
                        if cn.mask.contains(NoteMask::LINK_NEXT) && !cn.is_slide() {
                            removed[cn.string as usize] = true;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        match e {
            Ent::Note(original) => {
                let s = original.string as usize;
                if removed[s] {
                    // the previous linked note on this string was removed: remove this one too
                    if !original.mask.contains(NoteMask::LINK_NEXT) {
                        removed[s] = false;
                    }
                    continue;
                }
                *current.entry(score).or_default() += 1;
                let mut note = original.clone();
                prune_techniques(diff, &mut removed, &mut note);
                let index = acc.len();
                pending[s] = note.mask.contains(NoteMask::LINK_NEXT).then_some(Pending { entity: index, chord_note: None });

                if note.is_hopo() {
                    let prev_level = acc.iter().rposition(|(x, _)| has_string(x, note.string));
                    let prev_all = inp.entities.iter().rev().find(|x| match x {
                        Ent::Note(n) => n.string == note.string && n.time < note.time,
                        Ent::Chord(c) => {
                            c.time < note.time
                                && c.chord_notes.as_ref().is_some_and(|cn| cn.iter().any(|n| n.string == note.string))
                        }
                    });
                    let keep = match (prev_level.map(|i| (i, &acc[i].0)), prev_all) {
                        (_, None) => true, // hammer-on from nowhere
                        (Some((_, Ent::Note(n))), _) if n.is_tap() => true,
                        (Some((_, Ent::Note(n))), Some(Ent::Note(nn))) if n.time == nn.time => true,
                        (Some((_, Ent::Chord(c))), Some(Ent::Chord(cc))) if c.time == cc.time => true,
                        (Some((i, Ent::Note(n))), _)
                            if i == acc.len() - 1
                                && !(n.mask.contains(NoteMask::FRET_HAND_MUTE) || n.mask.contains(NoteMask::HARMONIC))
                                && ((note.mask.contains(NoteMask::PULL_OFF) && n.fret > note.fret)
                                    || (note.mask.contains(NoteMask::HAMMER_ON) && n.fret < note.fret)) =>
                        {
                            true
                        }
                        _ => false,
                    };
                    if !keep {
                        note.mask.remove(NoteMask::HAMMER_ON | NoteMask::PULL_OFF);
                    }
                }
                acc.push((Ent::Note(note), None));
            }
            Ent::Chord(chord) => {
                *current.entry(score).or_default() += 1;
                let template = &inp.templates[chord.chord_id as usize];
                let total = note_count(template);
                let index = acc.len();
                if allowed_chord_notes <= 1 {
                    let note = note_from_chord(diff, &mut removed, total, template, chord);
                    if note.mask.contains(NoteMask::LINK_NEXT) {
                        pending[note.string as usize] = Some(Pending { entity: index, chord_note: None });
                    }
                    acc.push((Ent::Note(note), None));
                    continue;
                }
                let mut copy = chord.clone();
                if copy.chord_notes.is_none() && is_first_chord_in_hs(&acc, inp.hand_shapes, &copy) {
                    copy.chord_notes = Some(chord_notes_from_template(template, &copy));
                }
                if allowed_chord_notes >= total {
                    if let Some(cns) = copy.chord_notes.as_mut().filter(|c| !c.is_empty()) {
                        for (i, cn) in cns.iter_mut().enumerate() {
                            prune_techniques(diff, &mut removed, cn);
                            if cn.mask.contains(NoteMask::LINK_NEXT) {
                                pending[cn.string as usize] = Some(Pending { entity: index, chord_note: Some(i) });
                            }
                        }
                    }
                    acc.push((Ent::Chord(copy), None));
                } else {
                    if let Some(cns) = copy.chord_notes.as_mut().filter(|c| !c.is_empty()) {
                        let remove = cns.len().saturating_sub(allowed_chord_notes);
                        for cn in &cns[cns.len() - remove..] {
                            if cn.mask.contains(NoteMask::LINK_NEXT) {
                                removed[cn.string as usize] = true;
                            }
                        }
                        cns.truncate(cns.len() - remove);
                        for (i, cn) in cns.iter_mut().enumerate() {
                            prune_techniques(diff, &mut removed, cn);
                            if cn.mask.contains(NoteMask::LINK_NEXT) {
                                pending[cn.string as usize] = Some(Pending { entity: index, chord_note: Some(i) });
                            }
                        }
                    }
                    let req = request(chord.chord_id, allowed_chord_notes, total, template);
                    acc.push((Ent::Chord(copy), Some(req)));
                }
            }
        }
    }
    acc
}

fn is_first_chord_in_hs(acc: &[(Ent, Option<TemplateRequest>)], hand_shapes: &[HandShape], chord: &Chord) -> bool {
    let Some(hs) = hand_shapes.iter().find(|h| chord.time >= h.start_time && chord.time < h.end_time) else {
        return true; // phrase start placed poorly
    };
    !acc.iter().any(|(e, _)| match e {
        Ent::Chord(c) => c.has_chord_notes() && c.chord_id == chord.chord_id && c.time >= hs.start_time && c.time < hs.end_time,
        Ent::Note(_) => false,
    })
}

// ------------------------------------------------------------------ hand shapes and anchors

fn inside(hs: &HandShape, t: i32) -> bool {
    t >= hs.start_time && t < hs.end_time
}

pub fn choose_hand_shapes(
    diff: f64,
    level: &[Ent],
    all: &[Ent],
    max_chord_notes: usize,
    templates: &[ChordTemplate],
    hand_shapes: &[HandShape],
) -> Vec<(HandShape, Option<TemplateRequest>)> {
    let allowed = allowed_chord_notes(diff, max_chord_notes);
    hand_shapes
        .iter()
        .filter_map(|hs| {
            let template = &templates[hs.chord_id as usize];
            let total = note_count(template);
            let no_notes = !level.iter().any(|e| inside(hs, e.time()) || inside(hs, e.time() + e.sustain()));
            let notes: Vec<&Note> =
                all.iter().filter_map(|e| if let Ent::Note(n) = e { inside(hs, n.time).then_some(n) } else { None }).collect();
            let arpeggio = !notes.is_empty() && !notes.iter().all(|n| n.string == notes[0].string || n.time == notes[0].time);
            if no_notes {
                None
            } else if arpeggio {
                Some((*hs, None)) // arpeggios always keep the full hand shape
            } else if allowed <= 1 {
                None
            } else if allowed >= total {
                Some((*hs, None))
            } else {
                Some((*hs, Some(request(hs.chord_id, allowed, total, template))))
            }
        })
        .collect()
}

/// Anchors needed by a level: the one at the phrase start and any with content.
pub fn choose_anchors(level: &[Ent], anchors: &[Anchor], start: i32, end: i32) -> Vec<Anchor> {
    const MARGIN: i32 = 3; // notes up to 3 ms early were meant to be on the anchor
    let include = |a_start: i32, a_end: i32| {
        level.iter().any(|e| {
            let (t, s) = (e.time(), e.sustain());
            (t + MARGIN >= a_start && t < a_end) || (t + s + MARGIN >= a_start && t + s < a_end)
        })
    };
    anchors
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            let a_end = anchors.get(i + 1).map_or(end, |n| n.time);
            a.time == start || include(a.time, a_end)
        })
        .map(|(_, a)| *a)
        .collect()
}
