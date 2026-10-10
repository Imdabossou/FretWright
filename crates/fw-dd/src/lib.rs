//! Dynamic difficulty: generates the easier versions of every phrase that let
//! Rocksmith level the player up gradually.
//!
//! Port of Rocksmith2014.NET's `Rocksmith2014.DD` (MIT, Tapio Malmberg). Each
//! note gets a score from its rhythmic position (strong beats first); lower
//! levels keep only the lower-scoring notes, drop techniques, thin chords and
//! remove the hand shapes and anchors they no longer need. Identical phrases
//! are then combined.
//!
//! Phrases are generated in parallel; the new chord templates for thinned
//! chords are then applied in phrase order, so output is deterministic (the
//! original adds them in thread completion order).

mod choose;
mod combine;
mod data;

use std::collections::HashMap;

use rayon::prelude::*;
use fw_xml::{ChordMask, ChordTemplate, InstrumentalArrangement, Level, NoteMask};

use choose::{Ent, TemplateRequest};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the arrangement has no phrase iterations")]
    NoPhrases,
    #[error("the arrangement already has difficulty levels")]
    AlreadyHasLevels,
    #[error("a chord or hand shape refers to chord template {0}, which does not exist")]
    BadChordId(i16),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelCount {
    /// One level per distinct note score (what DLC Builder uses by default).
    Simple,
    /// The same number of levels for every phrase (for testing).
    Constant(usize),
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Similarity (percent) above which phrases are combined; None combines
    /// only empty phrases.
    pub phrase_search_threshold: Option<i32>,
    pub level_count: LevelCount,
}

impl Default for Config {
    fn default() -> Self {
        Config { phrase_search_threshold: Some(85), level_count: LevelCount::Simple }
    }
}

const MIN_LEVELS: usize = 2;
const MAX_LEVELS: usize = 30;

#[derive(Clone, Copy)]
enum Target {
    Chord(usize),
    HandShape(usize),
}

struct GenLevel {
    level: Level,
    requests: Vec<(Target, TemplateRequest)>,
}

fn simple_level_count(p: &data::PhraseData, distinct_scores: usize) -> usize {
    let repeated = (p.repeated_notes + p.repeated_chords) as f64 / (p.note_count() + p.chord_count()) as f64;
    let count = if distinct_scores > 15 && repeated > 0.8 { distinct_scores / 2 } else { distinct_scores };
    count.clamp(MIN_LEVELS.max(p.max_chord_strings), MAX_LEVELS)
}

fn generate_levels(config: &Config, templates: &[ChordTemplate], p: &data::PhraseData) -> Vec<GenLevel> {
    if p.is_empty() {
        let level = Level { difficulty: 0, anchors: p.anchors.clone(), ..Default::default() };
        return vec![GenLevel { level, requests: Vec::new() }];
    }
    let entities = choose::entities(&p.notes, &p.chords);
    let scores: Vec<(i32, i32)> = entities.iter().map(|e| (e.time(), choose::score(p, e.time(), e))).collect();
    let mut notes_with_score: HashMap<i32, usize> = HashMap::new();
    let mut time_to_score: HashMap<i32, i32> = HashMap::new();
    for &(t, s) in &scores {
        *notes_with_score.entry(s).or_default() += 1;
        time_to_score.insert(t, s); // duplicates: last one wins, like F#'s dict
    }
    let score_map = choose::score_map(&scores, entities.len());
    let level_count = match config.level_count {
        LevelCount::Simple => simple_level_count(p, score_map.len()),
        LevelCount::Constant(n) => n.max(1),
    };

    (0..level_count)
        .map(|diff| {
            if diff == level_count - 1 {
                // the hardest level is the full phrase
                let level = Level {
                    difficulty: diff as i8,
                    notes: p.notes.clone(),
                    chords: p.chords.clone(),
                    anchors: p.anchors.clone(),
                    hand_shapes: p.hand_shapes.clone(),
                };
                return GenLevel { level, requests: Vec::new() };
            }
            let diff_percent = (diff + 1) as f64 / level_count as f64;
            let chosen = choose::choose_entities(&choose::ChooseInput {
                diff_percent,
                score_map: &score_map,
                time_to_score: &time_to_score,
                notes_with_score: &notes_with_score,
                templates,
                hand_shapes: &p.hand_shapes,
                max_chord_notes: p.max_chord_strings,
                entities: &entities,
            });

            let mut level = Level { difficulty: diff as i8, ..Default::default() };
            let mut entity_requests = Vec::new();
            let mut level_entities = Vec::with_capacity(chosen.len());
            for (e, req) in chosen {
                if let Ent::Chord(c) = &e {
                    let mut c = c.clone();
                    // a link-next chord without link-next notes crashes the game
                    if c.mask.contains(ChordMask::LINK_NEXT)
                        && c.chord_notes.as_ref().is_none_or(|cn| cn.iter().all(|n| !n.mask.contains(NoteMask::LINK_NEXT)))
                    {
                        c.mask.remove(ChordMask::LINK_NEXT);
                    }
                    if let Some(r) = req {
                        entity_requests.push((Target::Chord(level.chords.len()), r));
                    }
                    level.chords.push(c);
                } else if let Ent::Note(n) = &e {
                    level.notes.push(n.clone());
                }
                level_entities.push(e);
            }
            let mut requests = Vec::new();
            for (hs, req) in choose::choose_hand_shapes(
                diff_percent,
                &level_entities,
                &entities,
                p.max_chord_strings,
                templates,
                &p.hand_shapes,
            ) {
                if let Some(r) = req {
                    requests.push((Target::HandShape(level.hand_shapes.len()), r));
                }
                level.hand_shapes.push(hs);
            }
            level.anchors = choose::choose_anchors(&level_entities, &p.anchors, p.start_time, p.end_time);
            // hand shape requests first, then chord requests (upstream order)
            requests.extend(entity_requests);
            GenLevel { level, requests }
        })
        .collect()
}

/// Returns the frets/fingers with `count` notes removed, from the lowest
/// string up when keeping the highest notes, otherwise from the highest down.
fn remove_notes(count: usize, from_highest: bool, t: &ChordTemplate) -> ([i8; 6], [i8; 6]) {
    let (mut fingers, mut frets) = (t.fingers, t.frets);
    let order: Vec<usize> = if from_highest { (0..6).collect() } else { (0..6).rev().collect() };
    let mut left = count;
    for i in order {
        if left == 0 {
            break;
        }
        if frets[i] != -1 {
            frets[i] = -1;
            fingers[i] = -1;
            left -= 1;
        }
    }
    (fingers, frets)
}

fn apply_requests(templates: &mut Vec<ChordTemplate>, phrase: &mut [GenLevel]) -> Result<()> {
    let mut cache: HashMap<(i16, u8), i16> = HashMap::new();
    for gl in phrase {
        for (target, req) in &gl.requests {
            let id = match cache.get(&(req.original_id, req.note_count)) {
                Some(&id) => id,
                None => {
                    let t = templates.get(req.original_id as usize).cloned().ok_or(Error::BadChordId(req.original_id))?;
                    let remove = choose::note_count(&t).saturating_sub(req.note_count as usize);
                    let (fingers, frets) = remove_notes(remove, req.from_highest_note, &t);
                    // upstream compares name with display name (swapped); kept for identical output
                    let existing = templates.iter().position(|x| {
                        x.display_name == t.name && x.name == t.display_name && x.frets == frets && x.fingers == fingers
                    });
                    let id = match existing {
                        Some(i) => i as i16,
                        None => {
                            templates.push(ChordTemplate { name: t.name.clone(), display_name: t.display_name.clone(), fingers, frets });
                            templates.len() as i16 - 1
                        }
                    };
                    cache.insert((req.original_id, req.note_count), id);
                    id
                }
            };
            match *target {
                Target::Chord(i) => gl.level.chords[i].chord_id = id,
                Target::HandShape(i) => gl.level.hand_shapes[i].chord_id = id,
            }
        }
    }
    Ok(())
}

/// Generates difficulty levels for an arrangement that has a single level.
pub fn generate(config: &Config, arr: &mut InstrumentalArrangement) -> Result<()> {
    if arr.phrase_iterations.is_empty() {
        return Err(Error::NoPhrases);
    }
    if arr.levels.len() != 1 {
        return Err(Error::AlreadyHasLevels);
    }
    for c in &arr.levels[0].chords {
        if c.chord_id < 0 || c.chord_id as usize >= arr.chord_templates.len() {
            return Err(Error::BadChordId(c.chord_id));
        }
    }
    for h in &arr.levels[0].hand_shapes {
        if h.chord_id < 0 || h.chord_id as usize >= arr.chord_templates.len() {
            return Err(Error::BadChordId(h.chord_id));
        }
    }

    let phrase_data: Vec<data::PhraseData> =
        (0..arr.phrase_iterations.len()).into_par_iter().map(|i| data::phrase_iteration_data(arr, i)).collect();
    let mut per_phrase: Vec<Vec<GenLevel>> =
        phrase_data.par_iter().map(|p| generate_levels(config, &arr.chord_templates, p)).collect();
    for phrase in &mut per_phrase {
        apply_requests(&mut arr.chord_templates, phrase)?;
    }

    let level_counts: Vec<usize> = per_phrase.iter().map(Vec::len).collect();
    let max_levels = level_counts.iter().copied().max().unwrap_or(1);
    let mut levels: Vec<Level> = (0..max_levels).map(|d| Level { difficulty: d as i8, ..Default::default() }).collect();
    for phrase in per_phrase {
        for (d, gl) in phrase.into_iter().enumerate() {
            let l = &mut levels[d];
            l.anchors.extend(gl.level.anchors);
            l.notes.extend(gl.level.notes);
            l.hand_shapes.extend(gl.level.hand_shapes);
            l.chords.extend(gl.level.chords);
        }
    }

    let (phrases, iterations, linked) =
        combine::combine_same_phrases(config.phrase_search_threshold, &phrase_data, &arr.phrase_iterations, &level_counts);
    arr.levels = levels;
    arr.phrases = phrases;
    arr.phrase_iterations = iterations;
    arr.new_linked_diffs = linked;
    Ok(())
}
