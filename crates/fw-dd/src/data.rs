//! Per-phrase-iteration data used by the generator (port of DataExtractor.fs,
//! limited to the fields the simple level counter and the choosers use).

use fw_xml::{Anchor, Chord, Ebeat, HandShape, InstrumentalArrangement, Note};

#[derive(Debug, Clone)]
pub struct PhraseData {
    pub start_time: i32,
    pub end_time: i32,
    pub beats: Vec<Ebeat>,
    pub notes: Vec<Note>,
    pub chords: Vec<Chord>,
    pub hand_shapes: Vec<HandShape>,
    pub anchors: Vec<Anchor>,
    pub repeated_notes: usize,
    pub repeated_chords: usize,
    pub max_chord_strings: usize,
}

impl PhraseData {
    pub fn note_count(&self) -> usize {
        self.notes.len()
    }
    pub fn chord_count(&self) -> usize {
        self.chords.len()
    }
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.chords.is_empty()
    }
}

fn in_range(t: i32, start: i32, end: i32) -> bool {
    t >= start && t < end
}

pub fn phrase_iteration_data(arr: &InstrumentalArrangement, index: usize) -> PhraseData {
    let pi = &arr.phrase_iterations[index];
    let max_d = arr.phrases.get(pi.phrase_id as usize).map_or(0, |p| p.max_difficulty as usize);
    let start = pi.time;
    let end = arr.phrase_iterations.get(index + 1).map_or(arr.metadata.song_length, |p| p.time);
    let level = &arr.levels[max_d.min(arr.levels.len() - 1)];

    let notes: Vec<Note> = level.notes.iter().filter(|n| in_range(n.time, start, end)).cloned().collect();
    let chords: Vec<Chord> = level.chords.iter().filter(|c| in_range(c.time, start, end)).cloned().collect();
    let repeated_notes = notes
        .windows(2)
        .filter(|w| w[0].string == w[1].string && w[0].fret == w[1].fret && w[0].mask == w[1].mask)
        .count();
    let repeated_chords = chords.windows(2).filter(|w| w[0].chord_id == w[1].chord_id).count();
    let mut seen = Vec::new();
    let max_chord_strings = chords
        .iter()
        .filter(|c| {
            if seen.contains(&c.chord_id) {
                false
            } else {
                seen.push(c.chord_id);
                true
            }
        })
        .map(|c| {
            arr.chord_templates.get(c.chord_id as usize).map_or(0, |t| t.frets.iter().filter(|&&f| f != -1).count())
        })
        .max()
        .unwrap_or(0);

    PhraseData {
        start_time: start,
        end_time: end,
        beats: arr.ebeats.iter().filter(|b| in_range(b.time, start, end)).copied().collect(),
        notes,
        chords,
        hand_shapes: level.hand_shapes.iter().filter(|h| in_range(h.start_time, start, end)).copied().collect(),
        anchors: level.anchors.iter().filter(|a| in_range(a.time, start, end)).copied().collect(),
        repeated_notes,
        repeated_chords,
        max_chord_strings,
    }
}
