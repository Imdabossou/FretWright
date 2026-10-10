//! XML arrangement -> SNG conversion.
//!
//! A port of Rocksmith2014.NET's `Rocksmith2014.Conversion` (MIT, Tapio
//! Malmberg). Differences, all deliberate:
//!
//! * Output is deterministic. Levels are converted in parallel, but chord-note
//!   IDs are assigned in level order afterwards (the original assigns them in
//!   whatever order its threads finish).
//! * Notes and chords at the same time keep XML order (stable sort).
//! * Note hashes are FNV-1a of the note's fields. The original uses .NET's
//!   internal structural hash, which cannot be reproduced and is not checked
//!   by the game.
//! * Malformed input (note before the first anchor, bad chord id, ...) is an
//!   error rather than an exception.

mod level;
mod util;

use fw_sng as sng;
use fw_xml as xml;
use util::{find_beat_pi, find_pi, ms_to_sec};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("note at {0:.3}s has no anchor before it")]
    NoAnchor(f32),
    #[error("chord at {time:.3}s refers to chord template {id}, which does not exist")]
    BadChordId { time: f32, id: i16 },
    #[error("the arrangement needs at least two beats")]
    TooFewBeats,
    #[error("the arrangement needs at least one phrase iteration")]
    NoPhraseIterations,
    #[error("phrase iteration refers to phrase {0}, which does not exist")]
    BadPhraseId(i32),
    #[error("too many difficulty levels ({0}, maximum 36)")]
    TooManyLevels(usize),
    #[error("note on string {0}, which does not exist")]
    BadString(i8),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Note counts used for scoring.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoteCounts {
    pub easy: i32,
    pub medium: i32,
    pub hard: i32,
    /// Ignored notes at the hard level.
    pub ignored: i32,
}

impl std::ops::AddAssign for NoteCounts {
    fn add_assign(&mut self, o: Self) {
        self.easy += o.easy;
        self.medium += o.medium;
        self.hard += o.hard;
        self.ignored += o.ignored;
    }
}

/// Converts an instrumental arrangement into an SNG chart.
pub fn xml_to_sng(arr: &xml::InstrumentalArrangement) -> Result<sng::Sng> {
    convert(arr).map(|(s, _)| s)
}

/// Like [`xml_to_sng`], also returning the scoring note counts (needed for the
/// package manifest).
pub fn convert(arr: &xml::InstrumentalArrangement) -> Result<(sng::Sng, NoteCounts)> {
    if arr.ebeats.len() < 2 {
        return Err(Error::TooFewBeats);
    }
    if arr.phrase_iterations.is_empty() {
        return Err(Error::NoPhraseIterations);
    }
    if arr.levels.len() > 36 {
        return Err(Error::TooManyLevels(arr.levels.len()));
    }
    for pi in &arr.phrase_iterations {
        if pi.phrase_id < 0 || pi.phrase_id as usize >= arr.phrases.len() {
            return Err(Error::BadPhraseId(pi.phrase_id));
        }
    }

    // phrase iteration start times, with the song length as a sentinel
    let mut pi_times: Vec<i32> = arr.phrase_iterations.iter().map(|p| p.time).collect();
    pi_times.push(arr.metadata.song_length);

    let levels = level::convert_all(arr, &pi_times)?;

    // merge per-level results, assigning chord-note IDs in level order
    let mut chord_notes: Vec<sng::ChordNotes> = Vec::new();
    let mut chord_note_ids: std::collections::HashMap<Vec<u8>, i32> = Default::default();
    let mut string_masks = vec![[0i8; 36]; arr.sections.len()];
    let mut counts = NoteCounts::default();
    let mut sng_levels = Vec::with_capacity(levels.len());

    for (d, mut lvl) in levels.into_iter().enumerate() {
        let remap: Vec<i32> = lvl
            .chord_notes
            .iter()
            .map(|cn| {
                let key = util::chord_notes_key(cn);
                *chord_note_ids.entry(key).or_insert_with(|| {
                    chord_notes.push(cn.clone());
                    chord_notes.len() as i32 - 1
                })
            })
            .collect();
        for n in &mut lvl.level.notes {
            if n.chord_notes_id >= 0 {
                n.chord_notes_id = remap[n.chord_notes_id as usize];
            }
            n.hash = util::note_hash(n);
        }
        for (s, mask) in lvl.section_string_masks.iter().enumerate() {
            string_masks[s][d] = *mask;
        }
        counts += lvl.counts;
        sng_levels.push(lvl.level);
    }

    // the string masks of a section must also include those of the next one
    for s in 0..string_masks.len().saturating_sub(1) {
        let next = string_masks[s + 1];
        for (mask, n) in string_masks[s].iter_mut().zip(next).take(arr.levels.len()) {
            *mask |= n;
        }
    }

    let first_note_time = sng_levels
        .iter()
        .filter_map(|l| l.notes.first().map(|n| n.time))
        .fold(f32::MAX, f32::min);

    let mut beat_counter: i16 = 0;
    let mut current_measure: i16 = -1;
    let beats = arr
        .ebeats
        .iter()
        .map(|b| {
            if b.measure >= 0 {
                beat_counter = 0;
                current_measure = b.measure;
            } else {
                beat_counter += 1;
            }
            let mask = if beat_counter == 0 {
                if current_measure % 2 == 0 {
                    sng::BeatMask::FIRST_BEAT_OF_MEASURE | sng::BeatMask::EVEN_MEASURE
                } else {
                    sng::BeatMask::FIRST_BEAT_OF_MEASURE
                }
            } else {
                sng::BeatMask::empty()
            };
            sng::Beat {
                time: ms_to_sec(b.time),
                measure: current_measure,
                beat: beat_counter,
                phrase_iteration: find_beat_pi(b.time, &arr.phrase_iterations),
                mask,
            }
        })
        .collect();

    let phrases = arr
        .phrases
        .iter()
        .enumerate()
        .map(|(id, p)| sng::Phrase {
            solo: p.mask.contains(xml::PhraseMask::SOLO) as i8,
            disparity: p.mask.contains(xml::PhraseMask::DISPARITY) as i8,
            ignore: p.mask.contains(xml::PhraseMask::IGNORE) as i8,
            max_difficulty: p.max_difficulty as i32,
            iteration_count: arr.phrase_iterations.iter().filter(|pi| pi.phrase_id == id as i32).count() as i32,
            name: p.name.as_str().into(),
            ..Default::default()
        })
        .collect();

    let chords = arr.chord_templates.iter().map(|t| util::convert_chord_template(arr, t)).collect();

    let phrase_iterations = arr
        .phrase_iterations
        .iter()
        .enumerate()
        .map(|(i, pi)| sng::PhraseIteration {
            phrase_id: pi.phrase_id,
            start_time: ms_to_sec(pi.time),
            end_time: ms_to_sec(pi_times[i + 1]),
            difficulty: sng::Fixed([
                pi.hero_levels.easy as i32,
                pi.hero_levels.medium as i32,
                pi.hero_levels.hard as i32,
            ]),
        })
        .collect();

    let phrase_extra_info = arr
        .phrase_properties
        .iter()
        .flatten()
        .map(|p| sng::PhraseExtraInfo {
            phrase_id: p.phrase_id,
            difficulty: p.difficulty,
            empty: p.empty,
            level_jump: p.level_jump,
            redundant: p.redundant,
            ..Default::default()
        })
        .collect();

    let new_linked_difficulties = arr
        .new_linked_diffs
        .iter()
        .map(|n| sng::NewLinkedDifficulty { level_break: n.level_break as i32, nld_phrases: n.phrase_ids.clone() })
        .collect();

    let events = arr
        .events
        .iter()
        .map(|e| sng::Event { time: ms_to_sec(e.time), name: e.code.as_str().into() })
        .collect();

    let tones = arr
        .tones
        .changes
        .iter()
        .map(|t| sng::Tone { time: ms_to_sec(t.time), tone_id: t.id as i32 })
        .collect();

    let dnas = arr
        .events
        .iter()
        .filter_map(|e| {
            let id = match e.code.as_str() {
                "dna_none" => sng::dna::NONE,
                "dna_solo" => sng::dna::SOLO,
                "dna_riff" => sng::dna::RIFF,
                "dna_chord" => sng::dna::CHORD,
                _ => return None,
            };
            Some(sng::Dna { time: ms_to_sec(e.time), dna_id: id })
        })
        .collect();

    let sections = arr
        .sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let end_time = arr.sections.get(i + 1).map_or(arr.metadata.song_length, |n| n.time);
            let start_pi = find_pi(s.time, &arr.phrase_iterations);
            let mut end_pi = start_pi as usize + 1;
            while end_pi < arr.phrase_iterations.len() && arr.phrase_iterations[end_pi].time < end_time {
                end_pi += 1;
            }
            sng::Section {
                name: s.name.as_str().into(),
                number: s.number as i32,
                start_time: ms_to_sec(s.time),
                end_time: ms_to_sec(end_time),
                start_phrase_iteration_id: start_pi,
                end_phrase_iteration_id: end_pi as i32 - 1,
                string_mask: sng::Fixed(string_masks[i]),
            }
        })
        .collect();

    let md = &arr.metadata;
    let max_score = 100_000.0;
    let metadata = sng::MetaData {
        max_score,
        max_notes_and_chords: counts.hard as f64,
        max_notes_and_chords_real: (counts.hard - counts.ignored) as f64,
        points_per_note: max_score / (counts.hard as f64).max(1.0),
        first_beat_length: ms_to_sec(arr.ebeats[1].time - arr.ebeats[0].time),
        start_time: ms_to_sec(arr.start_beat()),
        capo_fret_id: if md.capo <= 0 { -1 } else { md.capo },
        last_conversion_date_time: util::conversion_date(),
        part: md.part,
        song_length: ms_to_sec(md.song_length),
        tuning: md.tuning.to_vec(),
        first_note_time,
        max_difficulty: arr.levels.len() as i32 - 1,
    };

    let sng = sng::Sng {
        beats,
        phrases,
        chords,
        chord_notes,
        phrase_iterations,
        phrase_extra_info,
        new_linked_difficulties,
        events,
        tones,
        dnas,
        sections,
        levels: sng_levels,
        metadata,
        ..Default::default()
    };
    Ok((sng, counts))
}
