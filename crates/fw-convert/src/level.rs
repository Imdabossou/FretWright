//! Conversion of one difficulty level.

use rayon::prelude::*;
use fw_sng::{self as sng, NoteMask as N};
use fw_xml::{self as xml, NoteMask as X};

use crate::util::*;
use crate::{Error, NoteCounts, Result};

/// A level converted in isolation; merged into the SNG afterwards.
pub(crate) struct LevelResult {
    pub level: sng::Level,
    /// Chord notes referenced by this level's notes (IDs are local).
    pub chord_notes: Vec<sng::ChordNotes>,
    /// String mask contribution per section for this difficulty.
    pub section_string_masks: Vec<i8>,
    pub counts: NoteCounts,
}

#[derive(Clone, Copy)]
enum Entity<'a> {
    Note(&'a xml::Note),
    Chord(&'a xml::Chord),
}

impl Entity<'_> {
    fn time(&self) -> i32 {
        match self {
            Entity::Note(n) => n.time,
            Entity::Chord(c) => c.time,
        }
    }

    fn sustain(&self) -> i32 {
        match self {
            Entity::Note(n) => n.sustain,
            Entity::Chord(c) => c.chord_notes.as_ref().and_then(|cn| cn.first()).map_or(0, |n| n.sustain),
        }
    }
}

// Values found in anchors that contain no notes.
const UNINIT_FIRST_NOTE: f32 = f32::MAX;
const UNINIT_LAST_NOTE: f32 = f32::MIN_POSITIVE;

pub(crate) fn convert_all(arr: &xml::InstrumentalArrangement, pi_times: &[i32]) -> Result<Vec<LevelResult>> {
    arr.levels.par_iter().enumerate().map(|(d, l)| convert_level(arr, pi_times, d, l)).collect()
}

fn convert_level(
    arr: &xml::InstrumentalArrangement,
    pi_times: &[i32],
    difficulty: usize,
    level: &xml::Level,
) -> Result<LevelResult> {
    let mut entities: Vec<Entity> =
        level.notes.iter().map(Entity::Note).chain(level.chords.iter().map(Entity::Chord)).collect();
    entities.sort_by_key(Entity::time);
    let note_times: Vec<i32> = entities.iter().map(Entity::time).collect();

    // hand shapes, split into regular ones and arpeggios
    let mut hand_shapes = Vec::new();
    let mut arpeggios = Vec::new();
    for hs in &level.hand_shapes {
        let template = arr
            .chord_templates
            .get(hs.chord_id as usize)
            .ok_or(Error::BadChordId { time: ms_to_sec(hs.start_time), id: hs.chord_id })?;
        let (first, last) = match first_and_last(&note_times, hs.start_time, hs.end_time) {
            None => (-1.0, -1.0),
            Some((f, l)) => {
                let end = note_times[l] + entities[l].sustain();
                (ms_to_sec(note_times[f]), if end >= hs.end_time { -1.0 } else { ms_to_sec(end) })
            }
        };
        let fp = sng::FingerPrint {
            chord_id: hs.chord_id as i32,
            start_time: ms_to_sec(hs.start_time),
            end_time: ms_to_sec(hs.end_time),
            first_note_time: first,
            last_note_time: last,
        };
        if template.is_arpeggio() { arpeggios.push(fp) } else { hand_shapes.push(fp) }
    }

    let mut conv = NoteConverter {
        arr,
        level,
        difficulty: difficulty as u8,
        note_times: &note_times,
        pi_times,
        finger_prints: [&hand_shapes, &arpeggios],
        pending_link_next: [None; 6],
        previous: None,
        chord_notes: Vec::new(),
        section_string_masks: vec![0; arr.sections.len()],
        anchor_extensions: Vec::new(),
        notes_in_pi_all: vec![0; arr.phrase_iterations.len()],
        notes_in_pi_excl_ignored: vec![0; arr.phrase_iterations.len()],
        counts: NoteCounts::default(),
    };
    let notes = entities
        .iter()
        .enumerate()
        .map(|(i, e)| conv.convert(i, *e))
        .collect::<Result<Vec<_>>>()?;

    let NoteConverter {
        chord_notes,
        section_string_masks,
        anchor_extensions,
        notes_in_pi_all,
        notes_in_pi_excl_ignored,
        counts,
        ..
    } = conv;

    let anchors = level
        .anchors
        .iter()
        .enumerate()
        .map(|(i, a)| convert_anchor(arr, level, &notes, &note_times, i, a))
        .collect();

    let average_notes_per_iteration = (0..arr.phrases.len())
        .map(|phrase_id| {
            let counts: Vec<f32> = notes_in_pi_all
                .iter()
                .enumerate()
                .filter(|(pi, _)| arr.phrase_iterations[*pi].phrase_id == phrase_id as i32)
                .map(|(_, &n)| n as f32)
                .collect();
            if counts.is_empty() {
                0.0
            } else {
                // single-precision running sum, like the original
                counts.iter().fold(0.0f32, |a, &b| a + b) / counts.len() as f32
            }
        })
        .collect();

    Ok(LevelResult {
        level: sng::Level {
            difficulty: difficulty as i32,
            anchors,
            anchor_extensions,
            hand_shapes,
            arpeggios,
            notes,
            average_notes_per_iteration,
            notes_in_phrase_iterations_excl_ignored: notes_in_pi_excl_ignored,
            notes_in_phrase_iterations_all: notes_in_pi_all,
        },
        chord_notes,
        section_string_masks,
        counts,
    })
}

fn convert_anchor(
    arr: &xml::InstrumentalArrangement,
    level: &xml::Level,
    notes: &[sng::Note],
    note_times: &[i32],
    index: usize,
    a: &xml::Anchor,
) -> sng::Anchor {
    let start = ms_to_sec(a.time);
    let end_ms = match level.anchors.get(index + 1) {
        Some(next) => next.time,
        // last anchor ends at the last phrase iteration (normally END)
        None => arr.phrase_iterations.last().map_or(arr.metadata.song_length, |p| p.time),
    };
    let (first_note_time, last_note_time) = match first_and_last(note_times, a.time, end_ms) {
        None => (UNINIT_FIRST_NOTE, UNINIT_LAST_NOTE),
        Some((fi, li)) => {
            let first = &notes[fi];
            let last = &notes[li];
            let first_time = if fi > 0 {
                let prev = &notes[fi - 1];
                // an anchor at the end of a slide starts where the target note would be
                if prev.mask.contains(N::SLIDE) && prev.time + prev.sustain - start < 0.001 {
                    start
                } else {
                    first.time
                }
            } else {
                first.time
            };
            let last_time = if last.mask.intersects(N::SLIDE | N::PARENT) {
                last.time
            } else {
                last.time + last.sustain
            };
            (first_time, last_time)
        }
    };
    sng::Anchor {
        start_time: start,
        end_time: ms_to_sec(end_ms),
        first_note_time,
        last_note_time,
        fret_id: a.fret,
        width: a.width as i32,
        phrase_iteration_id: find_pi(a.time, &arr.phrase_iterations),
        ..Default::default()
    }
}

struct NoteConverter<'a> {
    arr: &'a xml::InstrumentalArrangement,
    level: &'a xml::Level,
    difficulty: u8,
    note_times: &'a [i32],
    pi_times: &'a [i32],
    finger_prints: [&'a [sng::FingerPrint]; 2],
    /// Per string: a link-next note waiting for its child (note, its index).
    pending_link_next: [Option<(&'a xml::Note, i16)>; 6],
    previous: Option<sng::Note>,
    chord_notes: Vec<sng::ChordNotes>,
    section_string_masks: Vec<i8>,
    anchor_extensions: Vec<sng::AnchorExtension>,
    notes_in_pi_all: Vec<i32>,
    notes_in_pi_excl_ignored: Vec<i32>,
    counts: NoteCounts,
}

fn base_note_mask(n: &xml::Note) -> N {
    let mut m = N::empty();
    m.set(N::OPEN, n.fret == 0);
    m.set(N::SUSTAIN, n.sustain > 0);
    m.set(N::SLIDE, n.is_slide());
    m.set(N::UNPITCHED_SLIDE, n.is_unpitched_slide());
    m.set(N::TAP, n.is_tap());
    m.set(N::VIBRATO, n.is_vibrato());
    m.set(N::BEND, n.is_bend());
    m
}

fn technique_mask(x: X, include_ignore_and_right_hand: bool) -> N {
    let mut m = N::empty();
    for (xf, sf) in [
        (X::LINK_NEXT, N::PARENT),
        (X::ACCENT, N::ACCENT),
        (X::TREMOLO, N::TREMOLO),
        (X::FRET_HAND_MUTE, N::MUTE),
        (X::HAMMER_ON, N::HAMMER_ON),
        (X::HARMONIC, N::HARMONIC),
        (X::PALM_MUTE, N::PALM_MUTE),
        (X::PINCH_HARMONIC, N::PINCH_HARMONIC),
        (X::PLUCK, N::PLUCK),
        (X::PULL_OFF, N::PULL_OFF),
        (X::SLAP, N::SLAP),
    ] {
        m.set(sf, x.contains(xf));
    }
    if include_ignore_and_right_hand {
        m.set(N::IGNORE, x.contains(X::IGNORE));
        m.set(N::RIGHT_HAND, x.contains(X::RIGHT_HAND));
    }
    m
}

fn bend_value(b: &xml::BendValue) -> sng::BendValue {
    sng::BendValue { time: ms_to_sec(b.time), step: b.step, ..Default::default() }
}

impl<'a> NoteConverter<'a> {
    fn string_mask(&mut self, section: usize, string: i8) -> Result<()> {
        if !(0..6).contains(&string) {
            return Err(Error::BadString(string));
        }
        if let Some(m) = self.section_string_masks.get_mut(section) {
            *m |= 1i8 << string;
        }
        Ok(())
    }

    fn convert(&mut self, index: usize, ent: Entity<'a>) -> Result<sng::Note> {
        let arr = self.arr;
        let time = ent.time();
        let time_s = ms_to_sec(time);
        let pi_id = find_pi(time, &arr.phrase_iterations);
        let pi = &arr.phrase_iterations[pi_id as usize];
        let anchor = *find_anchor(time, &self.level.anchors).ok_or(Error::NoAnchor(time_s))?;
        let section = find_section(time, &arr.sections);
        let this = index as i16;
        let prev_iter = if index == 0 || self.note_times[index - 1] < pi.time { -1 } else { this - 1 };
        let next_iter = if index + 1 == self.note_times.len()
            || self.note_times[index + 1] >= self.pi_times[pi_id as usize + 1]
        {
            -1
        } else {
            this + 1
        };
        let fp_ids = [
            find_finger_print(time_s, self.finger_prints[0]),
            find_finger_print(time_s, self.finger_prints[1]),
        ];
        let is_arpeggio = fp_ids[1] != -1;

        let mut note = sng::Note {
            time: time_s,
            anchor_fret: anchor.fret,
            anchor_width: anchor.width,
            phrase_id: pi.phrase_id,
            phrase_iteration_id: pi_id,
            finger_print_id: sng::Fixed(fp_ids),
            ..Default::default()
        };
        let parent;

        match ent {
            Entity::Note(n) => {
                if !(0..6).contains(&n.string) {
                    return Err(Error::BadString(n.string));
                }
                let s = n.string as usize;
                parent = match self.pending_link_next[s].take() {
                    // the child must start at the end of the parent's sustain
                    Some((p, id)) if n.time - (p.time + p.sustain) <= 2 => id,
                    _ => -1,
                };
                if n.mask.contains(X::LINK_NEXT) {
                    self.pending_link_next[s] = Some((n, this));
                }
                let mut mask = N::SINGLE | base_note_mask(n) | technique_mask(n.mask, true);
                mask.set(N::LEFT_HAND, n.left_hand != -1);
                mask.set(N::CHILD, parent != -1);
                mask.set(N::ARPEGGIO, is_arpeggio);
                if n.is_slide() {
                    self.anchor_extensions.push(sng::AnchorExtension {
                        beat_time: ms_to_sec(time + n.sustain),
                        fret_id: n.slide_to,
                        ..Default::default()
                    });
                }
                self.string_mask(section, n.string)?;
                note.mask = mask;
                note.string_index = n.string;
                note.fret = n.fret;
                note.chord_id = -1;
                note.chord_notes_id = -1;
                // more than 32 bend values crash the game
                note.bend_data = n.bend_values.iter().flatten().take(32).map(bend_value).collect();
                note.slide_to = n.slide_to;
                note.slide_unpitch_to = n.slide_unpitch_to;
                note.left_hand = n.left_hand;
                note.vibrato = n.vibrato as i16;
                note.sustain = ms_to_sec(n.sustain);
                note.max_bend = n.max_bend;
                note.pick_direction = n.mask.contains(X::PICK_DIRECTION) as i8;
                note.tap = if n.tap > 0 { n.tap } else { -1 };
                note.slap = if n.mask.contains(X::SLAP) { 1 } else { -1 };
                note.pluck = if n.mask.contains(X::PLUCK) { 1 } else { -1 };
            }
            Entity::Chord(c) => {
                parent = -1;
                let template = arr
                    .chord_templates
                    .get(c.chord_id as usize)
                    .ok_or(Error::BadChordId { time: time_s, id: c.chord_id })?;
                let (sustain, chord_notes_id) = match c.chord_notes.as_ref().filter(|cn| !cn.is_empty()) {
                    Some(cns) => {
                        for cn in cns {
                            self.string_mask(section, cn.string)?;
                        }
                        (ms_to_sec(cns[0].sustain), self.chord_notes(this, cns))
                    }
                    None => (0.0, -1),
                };
                let mut mask = N::CHORD;
                mask.set(N::DOUBLE_STOP, is_double_stop(template));
                mask.set(N::CHORD_PANEL, c.has_chord_notes());
                mask.set(N::ARPEGGIO, template.is_arpeggio() || is_arpeggio);
                mask.set(N::SUSTAIN, sustain > 0.0);
                mask.set(N::CHORD_NOTES, chord_notes_id != -1);
                for (xf, sf) in [
                    (xml::ChordMask::ACCENT, N::ACCENT),
                    (xml::ChordMask::FRET_HAND_MUTE, N::FRET_HAND_MUTE),
                    (xml::ChordMask::HIGH_DENSITY, N::HIGH_DENSITY),
                    (xml::ChordMask::IGNORE, N::IGNORE),
                    (xml::ChordMask::LINK_NEXT, N::PARENT),
                    (xml::ChordMask::PALM_MUTE, N::PALM_MUTE),
                ] {
                    mask.set(sf, c.mask.contains(xf));
                }
                note.mask = mask;
                note.chord_id = c.chord_id as i32;
                note.chord_notes_id = chord_notes_id;
                note.sustain = sustain;
                note.string_index = -1;
                note.fret = -1;
                note.slide_to = -1;
                note.slide_unpitch_to = -1;
                note.left_hand = -1;
                note.tap = -1;
                note.pick_direction = -1;
                note.slap = -1;
                note.pluck = -1;
            }
        }

        // flag notes for fret numbering when the anchor changes (never open notes)
        note.flags = if note.fret == 0 {
            0
        } else {
            match &self.previous {
                Some(p) if p.anchor_fret == note.anchor_fret => 0,
                _ => 1,
            }
        };

        let ignored = note.mask.contains(N::IGNORE);
        self.add_note(pi_id as usize, pi.hero_levels, ignored);
        self.previous = Some(note.clone());

        note.next_iter_note = next_iter;
        note.prev_iter_note = prev_iter;
        note.parent_prev_note = parent;
        Ok(note)
    }

    fn add_note(&mut self, pi: usize, hero: xml::HeroLevels, ignored: bool) {
        self.notes_in_pi_all[pi] += 1;
        if !ignored {
            self.notes_in_pi_excl_ignored[pi] += 1;
        }
        let d = self.difficulty;
        if hero.easy == d {
            self.counts.easy += 1;
        }
        if hero.medium == d {
            self.counts.medium += 1;
        }
        if hero.hard == d {
            self.counts.hard += 1;
            if ignored {
                self.counts.ignored += 1;
            }
        }
    }

    /// Creates (or reuses) the chord-notes record for a chord; -1 when none is needed.
    fn chord_notes(&mut self, this: i16, cns: &'a [xml::Note]) -> i32 {
        let mut masks = [N::empty(); 6];
        for n in cns {
            if (0..6).contains(&n.string) {
                masks[n.string as usize] = base_note_mask(n) | technique_mask(n.mask, false);
            }
        }
        if masks.iter().all(|m| m.is_empty()) {
            return -1;
        }
        let mut rec = sng::ChordNotes {
            mask: sng::Fixed(masks),
            slide_to: sng::Fixed([-1; 6]),
            slide_unpitch_to: sng::Fixed([-1; 6]),
            ..Default::default()
        };
        for n in cns.iter().filter(|n| (0..6).contains(&n.string)) {
            let s = n.string as usize;
            rec.slide_to[s] = n.slide_to;
            rec.slide_unpitch_to[s] = n.slide_unpitch_to;
            rec.vibrato[s] = n.vibrato as i16;
            if let Some(bends) = n.bend_values.as_ref().filter(|b| !b.is_empty()) {
                let mut data = sng::BendData32 { used_count: bends.len() as i32, ..Default::default() };
                for (slot, b) in data.bend_values.iter_mut().zip(bends.iter()) {
                    *slot = bend_value(b);
                }
                rec.bend_data[s] = data;
            }
            if n.mask.contains(X::LINK_NEXT) {
                self.pending_link_next[s] = Some((n, this));
            }
        }
        let key = chord_notes_key(&rec);
        match self.chord_notes.iter().position(|c| chord_notes_key(c) == key) {
            Some(i) => i as i32,
            None => {
                self.chord_notes.push(rec);
                self.chord_notes.len() as i32 - 1
            }
        }
    }
}
