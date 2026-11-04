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

/// Puts the user's sections over generated phrases: each starts on a phrase (the nearest
/// generated one moves there, or a new one is added), the first no earlier than the
/// first note, and the end of the chart stays a `noguitar` section. Generated sections
/// remain only before the first of the user's.
pub fn set_sections(arr: &mut Arr, sections: &[Section]) {
    let count = arr.phrase_iterations.len();
    if sections.is_empty() || count < 3 {
        return;
    }
    let content = arr.phrase_iterations[1].time;
    let end = arr.phrase_iterations[count - 1].time;

    let mut wanted: Vec<Section> = Vec::new();
    for s in sections {
        let time = s.time.max(content);
        if time >= end {
            continue;
        }
        match wanted.last_mut() {
            Some(last) if last.time == time => last.name = s.name.clone(),
            _ => wanted.push(Section { time, ..s.clone() }),
        }
    }
    let Some(first) = wanted.first().map(|s| s.time) else { return };

    // Phrase starts already spoken for by a section; the first phrase stays on the first
    // note and the last on the end, so neither is moved.
    let mut taken: Vec<i32> = Vec::new();
    for s in &wanted {
        let last = arr.phrase_iterations.len() - 1;
        let nearest = (1..last)
            .filter(|&i| !taken.contains(&arr.phrase_iterations[i].time))
            .filter(|&i| i != 1 || arr.phrase_iterations[i].time == s.time)
            .min_by_key(|&i| (arr.phrase_iterations[i].time - s.time).abs())
            .filter(|&i| (arr.phrase_iterations[i].time - s.time).abs() < MIN_PHRASE_SEPARATION);
        match nearest {
            Some(i) => arr.phrase_iterations[i].time = s.time,
            None => {
                let at = arr.phrase_iterations.partition_point(|p| p.time < s.time);
                let phrase_id = arr.phrases.len() as i32;
                arr.phrases.push(Phrase { name: format!("p{phrase_id}"), max_difficulty: 0, mask: PhraseMask::empty() });
                arr.phrase_iterations.insert(at, PhraseIteration { time: s.time, phrase_id, ..Default::default() });
            }
        }
        taken.push(s.time);
    }

    let mut out: Vec<Section> = arr.sections.iter().filter(|s| s.time < first).cloned().collect();
    out.extend(wanted);
    out.push(Section { name: "noguitar".into(), number: 1, time: end });
    let mut seen = std::collections::HashMap::new();
    for s in &mut out {
        let n = seen.entry(s.name.clone()).or_insert(0i16);
        *n += 1;
        s.number = *n;
    }
    arr.sections = out;
}

#[cfg(test)]
mod section_tests {
    use fw_xml::Note;

    use super::*;

    /// A chart with notes from 4 s to 90 s on a 0.5 s beat, phrases generated.
    fn chart() -> Arr {
        let ebeats = (0..200).map(|i| Ebeat { time: i * 500, measure: if i % 4 == 0 { (i / 4 + 1) as i16 } else { -1 } }).collect();
        let notes = (0..172).map(|i| Note { time: 4000 + i * 500, ..Default::default() }).collect();
        let mut arr = Arr { ebeats, levels: vec![Level { notes, ..Default::default() }], ..Default::default() };
        arr.metadata.song_length = 100_000;
        generate(&mut arr).unwrap();
        arr
    }

    fn section(name: &str, time: i32) -> Section {
        Section { name: name.into(), number: 0, time }
    }

    fn starts_on_a_phrase(arr: &Arr) {
        for s in &arr.sections {
            assert!(arr.phrase_iterations.iter().any(|p| p.time == s.time), "{} at {}", s.name, s.time);
        }
        assert!(arr.phrase_iterations.windows(2).all(|w| w[0].time < w[1].time), "phrases out of order");
    }

    #[test]
    fn sections_replace_generated_ones_and_number_their_repeats() {
        let mut arr = chart();
        let end = arr.phrase_iterations.last().unwrap().time;
        set_sections(&mut arr, &[section("intro", 0), section("chorus", 30_000), section("verse", 50_000), section("chorus", 70_500)]);
        let got: Vec<(&str, i16, i32)> = arr.sections.iter().map(|s| (s.name.as_str(), s.number, s.time)).collect();
        // the intro waits for the first note; the end of the chart stays
        assert_eq!(got, [("intro", 1, 4000), ("chorus", 1, 30_000), ("verse", 1, 50_000), ("chorus", 2, 70_500), ("noguitar", 1, end)]);
        starts_on_a_phrase(&arr);
    }

    #[test]
    fn music_before_the_first_section_keeps_a_generated_one() {
        let mut arr = chart();
        set_sections(&mut arr, &[section("chorus", 40_000)]);
        assert_eq!((arr.sections[0].name.as_str(), arr.sections[0].time), ("riff", 4000));
        let chorus = arr.sections.iter().position(|s| s.name == "chorus").unwrap();
        assert!(arr.sections[..chorus].iter().all(|s| s.time < 40_000));
        assert_eq!(chorus, arr.sections.len() - 2);
        starts_on_a_phrase(&arr);
    }

    #[test]
    fn sections_past_the_end_and_doubles_are_dropped() {
        let mut arr = chart();
        let end = arr.phrase_iterations.last().unwrap().time;
        set_sections(&mut arr, &[section("intro", 0), section("verse", 1000), section("outro", end + 1000)]);
        let names: Vec<&str> = arr.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["verse", "noguitar"]);
        starts_on_a_phrase(&arr);
    }

    #[test]
    fn close_sections_get_their_own_phrases() {
        let mut arr = chart();
        set_sections(&mut arr, &[section("verse", 30_000), section("chorus", 30_500), section("solo", 31_000)]);
        starts_on_a_phrase(&arr);
        assert_eq!(arr.sections.iter().filter(|s| s.time >= 30_000 && s.time <= 31_000).count(), 3);
    }

    #[test]
    fn no_sections_leaves_the_chart_alone() {
        let mut arr = chart();
        let before = arr.sections.clone();
        set_sections(&mut arr, &[]);
        assert_eq!(arr.sections, before);
    }
}
