//! Automatic phrase and section generation (port of PhraseGenerator.fs).
//!
//! Phrases are placed about every 8 bars, nudged so they never split a
//! sustain, a link-next pair or a hand shape. Long gaps become "noguitar"
//! sections.

use fw_xml::{Anchor, Ebeat, InstrumentalArrangement as Arr, Level, NoteMask, Phrase, PhraseIteration, PhraseMask, Section};

use crate::util::*;
use crate::{Error, Result};

const MIN_PHRASE_SEPARATION: i32 = 2000;
const MIN_NO_GUITAR_LENGTH: i32 = 2500;

/// Combines the highest difficulty of every phrase into one level.
pub fn transcription_track(arr: &Arr) -> Level {
    let mut out = Level::default();
    let pis = &arr.phrase_iterations;
    for i in 0..pis.len().saturating_sub(1) {
        let (start, end) = (pis[i].time, pis[i + 1].time);
        let max = arr.phrases.get(pis[i].phrase_id as usize).map_or(0, |p| p.max_difficulty as usize);
        let Some(level) = arr.levels.get(max) else { continue };
        let inside = |t: i32| t >= start && t < end;
        out.notes.extend(level.notes.iter().filter(|n| inside(n.time)).cloned());
        out.chords.extend(level.chords.iter().filter(|c| inside(c.time)).cloned());
        out.hand_shapes.extend(level.hand_shapes.iter().filter(|h| inside(h.start_time)).copied());
        out.anchors.extend(level.anchors.iter().filter(|a| inside(a.time)).copied());
    }
    out
}

fn content_end(level: &Level) -> i32 {
    let note = level.notes.iter().map(|n| n.time + n.sustain).max();
    let chord = level
        .chords
        .iter()
        .map(|c| c.time + c.chord_notes.as_ref().and_then(|cn| cn.first()).map_or(0, |n| n.sustain))
        .max();
    let hs = level.hand_shapes.last().map(|h| h.end_time);
    [note, chord, hs].into_iter().flatten().max().unwrap_or(0)
}

fn end_phrase_time(level: &Level, arr: &Arr) -> i32 {
    let old_end = arr
        .phrases
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case("END"))
        .and_then(|id| arr.phrase_iterations.iter().find(|pi| pi.phrase_id == id as i32));
    let content_end = content_end(level);
    let t = match old_end {
        // EOF sometimes places END poorly
        Some(pi) if pi.time >= content_end => pi.time,
        _ => content_end,
    };
    if t != content_end {
        return t;
    }
    // the first beat after the content ends
    arr.ebeats
        .iter()
        .rposition(|b| b.time <= t)
        .and_then(|i| arr.ebeats.get(i + 1))
        .map_or_else(|| (t + 100).min(arr.metadata.song_length - 100), |b| b.time)
}

/// Searches earlier and later from `initial` for a time that splits nothing.
fn good_phrase_times(arr: &Arr, level: &Level, initial: i32) -> (i32, i32) {
    let note_end = |n: &fw_xml::Note| if n.sustain > 0 { n.time + n.sustain } else { n.time + 100 };
    let finder = |earlier: bool| {
        let mut time = initial;
        for _ in 0..20 {
            if let Some(n) = level.notes.iter().find(|n| n.time < time && n.time + n.sustain > time) {
                time = if earlier { n.time } else { n.time + n.sustain };
                continue;
            }
            let breaks_link_next = find_index_by_time(&level.notes, time, |n| n.time).and_then(|i| {
                let note = &level.notes[i];
                let prev_link = match previous_note_on_same_string(&level.notes, i) {
                    (Some(p), _) if level.notes[p].mask.contains(NoteMask::LINK_NEXT) => {
                        let p = &level.notes[p];
                        Some(if earlier { p.time } else { p.time + p.sustain })
                    }
                    _ => None,
                };
                match prev_link {
                    Some(_) if !earlier => Some(note_end(note)),
                    Some(t) => Some(t),
                    None => {
                        let chord_link = previous_chord_using_string(&arr.chord_templates, &level.chords, note.string, note.time)
                            .filter(|c| {
                                c.chord_notes.as_ref().is_some_and(|cns| {
                                    cns.iter().any(|cn| cn.string == note.string && cn.mask.contains(NoteMask::LINK_NEXT))
                                })
                            })
                            .map(|c| c.time);
                        match chord_link {
                            Some(_) if !earlier => Some(note_end(note)),
                            other => other,
                        }
                    }
                }
            });
            if let Some(t) = breaks_link_next {
                time = t;
                continue;
            }
            if let Some(hs) = level.hand_shapes.iter().find(|h| h.start_time < time && h.end_time > time) {
                time = if earlier { hs.start_time } else { hs.end_time };
                continue;
            }
            return time;
        }
        time
    };
    (finder(true), finder(false))
}

struct Counters {
    phrase: i32,
    riff: i16,
    no_guitar: i16,
}

impl Counters {
    fn add_phrase(&mut self, arr: &mut Arr, time: i32) {
        arr.phrase_iterations.push(PhraseIteration { time, phrase_id: arr.phrases.len() as i32, ..Default::default() });
        arr.phrases.push(Phrase { name: format!("p{}", self.phrase), max_difficulty: 0, mask: PhraseMask::empty() });
        self.phrase += 1;
    }

    fn add_section(&mut self, arr: &mut Arr, riff: bool, time: i32) {
        let (name, number) = if riff {
            self.riff += 1;
            ("riff", self.riff - 1)
        } else {
            self.no_guitar += 1;
            ("noguitar", self.no_guitar - 1)
        };
        arr.sections.push(Section { name: name.into(), number, time });
    }
}

/// Generates phrases and sections, replacing existing ones.
pub fn generate(arr: &mut Arr) -> Result<()> {
    let level = if arr.levels.len() == 1 { arr.levels[0].clone() } else { transcription_track(arr) };
    let Some(content_start) = next_content_time(&level, 0) else {
        return Ok(()); // no notes, chords or hand shapes at all
    };
    if arr.ebeats.len() < 2 {
        return Err(Error::PhraseGeneration("the arrangement needs at least two beats".into()));
    }
    let end_time = end_phrase_time(&level, arr);

    // the empty first phrase (COUNT), inserting a beat if content starts on the first one
    let first_beat = arr.ebeats[0].time;
    let count_time = if first_beat == content_start {
        if content_start == 0 {
            return Err(Error::PhraseGeneration(
                "there is no room for an empty phrase before the arrangement content starts".into(),
            ));
        }
        let beat_len = arr.ebeats[1].time - first_beat;
        let t = (first_beat - beat_len).max(0);
        arr.ebeats.insert(0, Ebeat { time: t, measure: 0 });
        t
    } else {
        first_beat
    };

    arr.phrases.clear();
    arr.phrase_iterations.clear();
    arr.sections.clear();
    arr.phrases.push(Phrase { name: "COUNT".into(), ..Default::default() });
    arr.phrase_iterations.push(PhraseIteration { time: count_time, phrase_id: 0, ..Default::default() });

    let mut c = Counters { phrase: 0, riff: 1, no_guitar: 1 };
    c.add_phrase(arr, content_start);
    c.add_section(arr, true, content_start);

    // anchors are added to the working level as phrases are placed; mirror them
    // into the real single level when there is one
    let mut level = level;
    let mut measure_counter = 0;
    let mut next_phrase_time: Option<i32> = None;
    let beats: Vec<Ebeat> =
        arr.ebeats.iter().copied().skip_while(|b| b.time < content_start).take_while(|b| b.time < end_time).collect();

    for beat in beats {
        if beat.measure != -1 && next_phrase_time.is_none() {
            measure_counter += 1;
        }
        if !(measure_counter >= 9 || next_phrase_time.is_some_and(|t| beat.time >= t)) {
            continue;
        }
        measure_counter = 1;
        let prev_phrase_time = arr.sections.last().map_or(0, |s| s.time);
        let (create, time) = match next_phrase_time.take() {
            None => {
                let (earlier, later) = good_phrase_times(arr, &level, beat.time);
                let closest = if beat.time - earlier < later - beat.time { earlier } else { later };
                if closest - prev_phrase_time > MIN_PHRASE_SEPARATION {
                    (true, closest)
                } else if later - prev_phrase_time > MIN_PHRASE_SEPARATION {
                    (true, later)
                } else {
                    (false, later)
                }
            }
            // the phrase after a noguitar section starts with the content
            Some(t) => (true, t),
        };
        if !create {
            continue;
        }
        let riff = match next_content_time(&level, time) {
            None => false,
            Some(next) if next - time >= MIN_NO_GUITAR_LENGTH => {
                next_phrase_time = Some(next);
                false
            }
            Some(_) => true,
        };
        if let Some(active) = active_anchor(&level, time)
            && active.time != time
        {
            let a = Anchor { time, ..active };
            insert_by_time(&mut level.anchors, a, |a| a.time);
            if arr.levels.len() == 1 {
                insert_by_time(&mut arr.levels[0].anchors, a, |a| a.time);
            }
        }
        c.add_phrase(arr, time);
        c.add_section(arr, riff, time);
    }

    arr.phrase_iterations.push(PhraseIteration { time: end_time, phrase_id: arr.phrases.len() as i32, ..Default::default() });
    arr.phrases.push(Phrase { name: "END".into(), ..Default::default() });
    c.add_section(arr, false, end_time);
    Ok(())
}
