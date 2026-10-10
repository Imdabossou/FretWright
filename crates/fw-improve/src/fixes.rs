//! Arrangement fixes, ported from Rocksmith2014.NET's XML.Processing
//! improvers (MIT, Tapio Malmberg). Order of application lives in lib.rs.
//! Behaviour is kept identical, quirks included; those are commented.

use std::collections::{BTreeSet, HashSet};

use fw_xml::{
    Anchor, BendValue, ChordMask, ChordTemplate, Event, HandShape, InstrumentalArrangement as Arr, NoteMask,
};

use crate::util::*;
use crate::{Error, Result};

// ------------------------------------------------------------------ basic fixes

/// Only characters used in official files; double quotes can corrupt saves.
pub fn validate_phrase_names(arr: &mut Arr) {
    for p in &mut arr.phrases {
        p.name.retain(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '#');
    }
}

/// Ignore 23rd/24th fret notes (and chords using them) and sustained 7th fret harmonics.
pub fn add_ignores(arr: &mut Arr) {
    let ignore_harmonic = |n: &fw_xml::Note| n.fret == 7 && n.sustain > 0 && n.mask.contains(NoteMask::HARMONIC);
    for level in &mut arr.levels {
        for n in &mut level.notes {
            if n.fret >= 23 || ignore_harmonic(n) {
                n.mask.insert(NoteMask::IGNORE);
            }
        }
        for c in &mut level.chords {
            if c.chord_notes.as_ref().is_some_and(|cns| cns.iter().any(ignore_harmonic)) {
                c.mask.insert(ChordMask::IGNORE);
            }
            if arr.chord_templates.get(c.chord_id as usize).is_some_and(|t| t.frets.iter().any(|&f| f >= 23)) {
                c.mask.insert(ChordMask::IGNORE);
            }
        }
    }
}

/// Fixes link-next errors: no target, target too far, wrong target fret, bends.
pub fn fix_link_nexts(arr: &mut Arr) {
    for level in &mut arr.levels {
        let notes = &mut level.notes;
        for i in 0..notes.len() {
            if !notes[i].mask.contains(NoteMask::LINK_NEXT) {
                continue;
            }
            let note = notes[i].clone();
            match next_note_on_same_string(notes, Some(i), &note) {
                None => notes[i].mask.remove(NoteMask::LINK_NEXT),
                Some(j) if notes[j].time - (note.time + note.sustain) > 50 => notes[i].mask.remove(NoteMask::LINK_NEXT),
                Some(j) => {
                    let fret = if note.slide_to > 0 {
                        note.slide_to
                    } else if note.slide_unpitch_to > 0 {
                        note.slide_unpitch_to
                    } else {
                        note.fret
                    };
                    let next = &mut notes[j];
                    next.fret = fret;
                    if let Some(last) = note.bend_values.as_ref().and_then(|b| b.last()).filter(|_| note.is_bend()) {
                        let step = last.step;
                        if step > 0.0 {
                            if !next.is_bend() {
                                next.max_bend = step;
                                next.bend_values = Some(vec![BendValue { time: next.time, step }]);
                            } else if let Some(bends) = next.bend_values.as_mut()
                                && bends[0].time != next.time
                            {
                                if next.max_bend < step {
                                    next.max_bend = step;
                                }
                                bends.insert(0, BendValue { time: next.time, step });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Keeps only the first bend value at each time.
pub fn remove_overlapping_bend_values(arr: &mut Arr) {
    let dedup = |n: &mut fw_xml::Note| {
        if let Some(b) = n.bend_values.as_mut().filter(|b| !b.is_empty()) {
            let mut seen = HashSet::new();
            b.retain(|v| seen.insert(v.time));
        }
    };
    for level in &mut arr.levels {
        level.notes.iter_mut().for_each(dedup);
        for c in &mut level.chords {
            c.chord_notes.iter_mut().flatten().for_each(dedup);
        }
    }
}

/// Removes fret-hand-muted notes from chords that also contain normal notes.
pub fn remove_muted_notes_from_chords(arr: &mut Arr) {
    // quirk: once a template is fixed, later chords using it are left alone
    let mut fixed: HashSet<i16> = HashSet::new();
    for level in &mut arr.levels {
        for chord in &mut level.chords {
            if fixed.contains(&chord.chord_id) || chord.mask.contains(ChordMask::FRET_HAND_MUTE) {
                continue;
            }
            let Some(cns) = chord.chord_notes.as_mut().filter(|c| !c.is_empty()) else { continue };
            let muted: Vec<i8> =
                cns.iter().filter(|n| n.mask.contains(NoteMask::FRET_HAND_MUTE)).map(|n| n.string).collect();
            if !muted.is_empty() && muted.len() != cns.len() {
                cns.retain(|n| !n.mask.contains(NoteMask::FRET_HAND_MUTE));
                if let Some(t) = arr.chord_templates.get_mut(chord.chord_id as usize) {
                    for s in muted {
                        t.frets[s as usize] = -1;
                        t.fingers[s as usize] = -1;
                    }
                }
                fixed.insert(chord.chord_id);
            }
        }
    }
}

/// Removes anchors identical to the previous one (kept at phrase starts).
pub fn remove_redundant_anchors(arr: &mut Arr) {
    let phrase_times: HashSet<i32> = arr.phrase_iterations.iter().map(|p| p.time).collect();
    for level in &mut arr.levels {
        let anchors = &level.anchors;
        if anchors.is_empty() {
            continue;
        }
        let mut out = vec![anchors[0]];
        for i in 1..anchors.len() {
            // quirk: compares with the original previous anchor, not the last one kept
            let (p, c) = (anchors[i - 1], anchors[i]);
            if !(p.fret == c.fret && p.width == c.width) || phrase_times.contains(&c.time) {
                out.push(c);
            }
        }
        level.anchors = out;
    }
}

/// Moves anchors that are within 5 ms of a note (but not on it) onto the note.
pub fn move_anchors(arr: &mut Arr) {
    for level in &mut arr.levels {
        let times: Vec<i32> = level
            .notes
            .iter()
            .map(|n| n.time)
            .chain(level.chords.iter().map(|c| c.time))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let slide_ends: HashSet<i32> = level
            .chords
            .iter()
            .filter_map(|c| c.chord_notes.as_ref()?.iter().find(|n| n.is_slide()).map(|n| n.time + n.sustain))
            .chain(level.notes.iter().filter(|n| n.is_slide()).map(|n| n.time + n.sustain))
            .collect();
        for a in &mut level.anchors {
            if times.binary_search(&a.time).is_ok() || slide_ends.contains(&a.time) {
                continue;
            }
            if let Some(&t) = times.iter().find(|&&t| {
                let d = a.time - t;
                d != 0 && d.abs() <= 5
            }) {
                a.time = t;
            }
        }
    }
}

/// Moves phrases named "moverN" right by N notes.
pub fn move_phrases(arr: &mut Arr) -> Result<()> {
    for phrase_id in 0..arr.phrases.len() {
        let phrase = arr.phrases[phrase_id].clone();
        if !starts_with_ci(&phrase.name, "mover") {
            continue;
        }
        let number = &phrase.name["mover".len()..];
        for pi_index in 0..arr.phrase_iterations.len() {
            if arr.phrase_iterations[pi_index].phrase_id != phrase_id as i32 {
                continue;
            }
            let phrase_time = arr.phrase_iterations[pi_index].time;
            let bad = || Error::PhraseMover(time_to_string(phrase_time));
            let move_by: usize = number.trim().parse().map_err(|_| bad())?;
            let level = arr.levels.get(phrase.max_difficulty as usize).ok_or_else(bad)?;
            let move_to = time_of_nth_note_from(level, phrase_time, move_by).ok_or_else(bad)?;

            for level in arr.levels.iter_mut().take_while(|l| l.difficulty <= phrase.max_difficulty as i8) {
                let orig = find_index_by_time(&level.anchors, phrase_time, |a| a.time);
                let target = find_index_by_time(&level.anchors, move_to, |a| a.time);
                if let (Some(orig), None) = (orig, target) {
                    let a = level.anchors[orig];
                    insert_by_time(&mut level.anchors, Anchor { time: move_to, ..a }, |a| a.time);
                    let has_content = find_index_by_time(&level.notes, phrase_time, |n| n.time).is_some()
                        || find_index_by_time(&level.chords, phrase_time, |c| c.time).is_some();
                    if !has_content {
                        level.anchors.remove(orig);
                    }
                }
            }
            arr.phrase_iterations[pi_index].time = move_to;
            if let Some(i) = find_index_by_time(&arr.sections, phrase_time, |s| s.time) {
                arr.sections[i].time = move_to;
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ EOF fixes

pub fn fix_crowd_events(arr: &mut Arr) {
    for e in &mut arr.events {
        if matches!(e.code.as_str(), "E0" | "E1" | "E2") {
            e.code = e.code.to_lowercase();
        }
    }
}

/// All chord notes share the longest sustain; a chord with link-next notes is link-next.
pub fn fix_chord_notes(arr: &mut Arr) {
    for c in arr.levels.iter_mut().flat_map(|l| l.chords.iter_mut()) {
        if let Some(cns) = c.chord_notes.as_mut().filter(|c| !c.is_empty()) {
            let sustain = cns.iter().map(|n| n.sustain).max().unwrap_or(0);
            cns.iter_mut().for_each(|n| n.sustain = sustain);
            if !c.mask.contains(ChordMask::LINK_NEXT) && cns.iter().any(|n| n.mask.contains(NoteMask::LINK_NEXT)) {
                c.mask.insert(ChordMask::LINK_NEXT);
            }
        }
    }
}

/// Link-next chord notes need a note on the same string right after them.
pub fn remove_invalid_chord_note_link_nexts(arr: &mut Arr) {
    for level in &mut arr.levels {
        let notes = &level.notes;
        for c in &mut level.chords {
            if !c.mask.contains(ChordMask::LINK_NEXT) {
                continue;
            }
            for cn in c.chord_notes.iter_mut().flatten().filter(|n| n.mask.contains(NoteMask::LINK_NEXT)) {
                match notes.iter().find(|n| n.time > cn.time && n.string == cn.string) {
                    None => cn.mask.remove(NoteMask::LINK_NEXT),
                    Some(n) if n.time - cn.time - cn.sustain > 2 => cn.mask.remove(NoteMask::LINK_NEXT),
                    _ => {}
                }
            }
        }
    }
}

/// Hand shapes of sliding link-next chords end where the slide ends.
pub fn fix_chord_slide_hand_shapes(arr: &mut Arr) {
    for level in &mut arr.levels {
        for c in &level.chords {
            let Some(cns) = c.chord_notes.as_ref().filter(|c| !c.is_empty()) else { continue };
            if !c.mask.contains(ChordMask::LINK_NEXT) || !cns.iter().any(|n| n.is_slide()) {
                continue;
            }
            if let Some(hs) = level.hand_shapes.iter_mut().find(|h| h.start_time == c.time)
                && hs.end_time > hs.start_time + cns[0].sustain
            {
                hs.end_time = hs.start_time + cns[0].sustain;
            }
        }
    }
}

/// Every phrase (except COUNT and END) starts with an anchor.
pub fn fix_phrase_start_anchors(arr: &mut Arr) {
    let pi_times: Vec<i32> = arr.phrase_iterations.iter().map(|p| p.time).collect();
    for level in &mut arr.levels {
        for i in 1..pi_times.len().saturating_sub(1) {
            let pi_time = pi_times[i];
            let anchors = &mut level.anchors;
            match anchors.iter().rev().find(|a| a.time <= pi_time).copied() {
                Some(active) if active.time != pi_time => {
                    insert_by_time(anchors, Anchor { time: pi_time, ..active }, |a| a.time);
                }
                Some(_) => {}
                None => {
                    if let Some(j) = anchors.iter().position(|a| a.time > pi_time) {
                        let next = anchors[j];
                        if pi_times.contains(&next.time) {
                            insert_by_time(anchors, Anchor { time: pi_time, ..next }, |a| a.time);
                        } else {
                            anchors[j].time = pi_time;
                        }
                    }
                }
            }
        }
    }
}

pub fn eof_fixes(arr: &mut Arr) {
    fix_crowd_events(arr);
    fix_chord_notes(arr);
    remove_invalid_chord_note_link_nexts(arr);
    fix_chord_slide_hand_shapes(arr);
    fix_phrase_start_anchors(arr);
}

// ------------------------------------------------------------------ other improvers

/// Removes notes after a link-next that have no sustain (and the link-next).
pub fn remove_unnecessary_notes(arr: &mut Arr) {
    for level in &mut arr.levels {
        let mut remove: HashSet<usize> = HashSet::new();
        for i in 0..level.notes.len() {
            if !level.notes[i].mask.contains(NoteMask::LINK_NEXT) {
                continue;
            }
            let note = level.notes[i].clone();
            if let Some(j) = next_note_on_same_string(&level.notes, Some(i), &note)
                && level.notes[j].sustain == 0
            {
                remove.insert(j);
                level.notes[i].mask.remove(NoteMask::LINK_NEXT);
            }
        }
        for c in &mut level.chords {
            if !c.mask.contains(ChordMask::LINK_NEXT) {
                continue;
            }
            let Some(cns) = c.chord_notes.as_mut() else { continue };
            let Some(note_index) = level.notes.iter().position(|n| n.time > c.time) else { continue };
            let current = note_index.checked_sub(1);
            for cn in cns.iter_mut().filter(|n| n.mask.contains(NoteMask::LINK_NEXT)) {
                if let Some(j) = next_note_on_same_string(&level.notes, current, cn)
                    && level.notes[j].sustain == 0
                {
                    cn.mask.remove(NoteMask::LINK_NEXT);
                    remove.insert(j);
                }
            }
            let any = cns.iter().any(|n| n.mask.contains(NoteMask::LINK_NEXT));
            c.mask.set(ChordMask::LINK_NEXT, any);
        }
        if !remove.is_empty() {
            let mut i = 0;
            level.notes.retain(|_| {
                let keep = !remove.contains(&i);
                i += 1;
                keep
            });
        }
    }
}

/// Adds crowd tempo and applause events when missing.
pub fn add_crowd_events(arr: &mut Arr) {
    let ends_with_crowd_speed = |c: &str| {
        let b = c.as_bytes();
        b.len() >= 2 && b[b.len() - 2] == b'e' && (b'0'..=b'2').contains(&b[b.len() - 1])
    };
    let insert = |arr: &mut Arr, code: &str, time: i32| {
        insert_by_time(&mut arr.events, Event { time, code: code.into() }, |e| e.time);
    };
    if !arr.events.iter().any(|e| ends_with_crowd_speed(&e.code)) {
        let tempo = arr.metadata.average_tempo;
        let code = if tempo < 90.0 {
            "e0"
        } else if tempo < 170.0 {
            "e1"
        } else {
            "e2"
        };
        let start = arr.start_beat();
        insert(arr, code, start);
    }
    if !arr.events.iter().any(|e| e.code == "E3") {
        let start = first_note_time(arr).map_or(arr.start_beat(), |t| t + 600);
        insert(arr, "E3", start);
        insert(arr, "E13", start + 2_500);
    }
    if !arr.events.iter().any(|e| e.code == "D3") {
        let end = arr.metadata.song_length;
        insert(arr, "D3", end - 5_000 - 4_000);
        insert(arr, "E13", end);
    }
}

pub fn process_chord_names(arr: &mut Arr) {
    for t in &mut arr.chord_templates {
        if t.name.trim().is_empty() {
            t.name.clear();
        } else {
            t.name = t.name.replace("min", "m").replace("CONV", "").replace("-nop", "").replace("-arp", "");
        }
        if t.display_name.trim().is_empty() {
            t.display_name.clear();
        } else {
            t.display_name = t.display_name.replace("min", "m").replace("CONV", "-arp");
        }
    }
}

/// Handles the custom events "w3[-fret]", "removebeats" and "so[N]" (slide out).
pub fn process_custom_events(arr: &mut Arr) -> Result<()> {
    // anchor width 3 (arrangements without DD only)
    let w3: Vec<Event> = arr.events.iter().filter(|e| starts_with_ci(&e.code, "w3")).cloned().collect();
    for e in w3 {
        let fret: Option<i32> = e.code.split('-').nth(1).and_then(|f| f.parse().ok());
        if let Some(a) = arr.levels.first_mut().and_then(|l| l.anchors.iter_mut().find(|a| a.time >= e.time)) {
            a.width = 3;
            if let Some(f) = fret.filter(|f| (1..=22).contains(f)) {
                a.fret = f as i8;
            }
        }
        if let Some(i) = arr.events.iter().position(|x| *x == e) {
            arr.events.remove(i);
        }
    }

    if let Some(i) = arr.events.iter().position(|e| e.code.eq_ignore_ascii_case("removebeats")) {
        let t = arr.events[i].time;
        arr.ebeats.retain(|b| b.time < t);
        arr.events.remove(i);
    }

    let slide_outs: Vec<Event> = arr.events.iter().filter(|e| starts_with_ci(&e.code, "so")).cloned().collect();
    for ev in slide_outs {
        let at = time_to_string(ev.time);
        let err = |m: &str| Error::CustomEvent(format!("{m} at {at}"));
        let pi = arr
            .phrase_iterations
            .iter()
            .rev()
            .find(|p| p.time <= ev.time)
            .ok_or_else(|| err("slide-out event before the first phrase"))?;
        let diff = arr.phrases[pi.phrase_id as usize].max_difficulty as usize;
        let slide_time = if ev.code.len() > 2 {
            match ev.code[2..].parse::<usize>() {
                Ok(right_by) => time_of_nth_note_from(&arr.levels[diff], ev.time, right_by),
                Err(_) => continue, // some other event that happens to start with "so"
            }
        } else {
            Some(ev.time)
        };
        let Some(slide_time) = slide_time else { continue };

        let templates_len = arr.chord_templates.len();
        let level = &mut arr.levels[diff];
        let note_index = find_index_by_time(&level.notes, slide_time, |n| n.time);
        let chord_index = find_index_by_time(&level.chords, slide_time, |c| c.time);
        if note_index.is_none() && chord_index.is_none() {
            return Err(err("could not find the notes or chord for slide-out event"));
        }
        let (notes, template_id) = match chord_index {
            None => {
                // notes that follow a link-next chord
                let chord = level
                    .chords
                    .iter()
                    .rev()
                    .find(|c| c.time < slide_time)
                    .ok_or_else(|| err("slide-out notes without a chord"))?;
                let sustain = chord
                    .chord_notes
                    .as_ref()
                    .and_then(|c| c.first())
                    .ok_or_else(|| err("chord missing chord notes for slide-out event"))?
                    .sustain;
                let chord_time = chord.time;
                let chord_id = chord.chord_id;
                if let Some(hs) = level.hand_shapes.iter_mut().find(|h| h.start_time == chord_time)
                    && hs.end_time > chord_time + sustain
                {
                    hs.end_time = chord_time + sustain;
                }
                let notes: Vec<fw_xml::Note> =
                    level.notes.iter().filter(|n| n.time == slide_time && n.is_unpitched_slide()).cloned().collect();
                (notes, chord_id)
            }
            Some(ci) => {
                let chord = &level.chords[ci];
                let cns = chord.chord_notes.as_ref().ok_or_else(|| err("chord missing chord notes for slide-out event"))?;
                let notes: Vec<fw_xml::Note> = cns.iter().filter(|n| n.is_unpitched_slide()).cloned().collect();
                let (chord_time, chord_id) = (chord.time, chord.chord_id);
                if let Some(hs) = level.hand_shapes.iter_mut().find(|h| h.start_time == chord_time) {
                    hs.end_time = hs.start_time + (hs.end_time - hs.start_time) / 3;
                }
                (notes, chord_id)
            }
        };
        if notes.is_empty() {
            return Err(err("invalid slide-out event"));
        }
        // a new hand shape at the slide end (+1 ms to include the notes' sustain)
        let end = notes[0].time + notes[0].sustain + 1;
        let start = end - notes[0].sustain / 3;
        insert_by_time(
            &mut level.hand_shapes,
            HandShape { chord_id: templates_len as i16, start_time: start, end_time: end },
            |h| h.start_time,
        );
        let original = arr.chord_templates.get(template_id as usize).cloned().unwrap_or_default();
        let mut t = ChordTemplate::default();
        for n in &notes {
            let s = n.string as usize;
            t.frets[s] = n.slide_unpitch_to;
            t.fingers[s] = original.fingers[s];
        }
        arr.chord_templates.push(t);
        if let Some(i) = arr.events.iter().position(|x| *x == ev) {
            arr.events.remove(i);
        }
    }
    Ok(())
}

/// Drops beats after the audio ends and moves the last beat onto the end.
pub fn remove_extra_beats(arr: &mut Arr) {
    let beats = &mut arr.ebeats;
    let end = arr.metadata.song_length;
    if beats.len() < 2 {
        return;
    }
    while beats.len() > 2 && beats[beats.len() - 2].time > end {
        beats.pop();
    }
    let n = beats.len();
    if end - beats[n - 2].time <= beats[n - 1].time - end {
        beats.pop();
    }
    if let Some(last) = beats.last_mut() {
        last.time = end;
    }
}

fn beats_around(beats: &[fw_xml::Ebeat], time: i32) -> (i32, i32) {
    // hand shapes after the last beat (bad XML) use the last beat
    let next = beats.iter().position(|b| b.time > time).unwrap_or(beats.len() - 1).max(1);
    (beats[next - 1].time, beats[next].time)
}

/// Lengthens hand shapes that end on a chord.
pub fn lengthen_hand_shapes(arr: &mut Arr) {
    let beats = arr.ebeats.clone();
    let pi_times: Vec<i32> = arr.phrase_iterations.iter().map(|p| p.time).collect();
    if beats.len() < 2 {
        return;
    }
    for level in &mut arr.levels {
        let (mut chord_index, mut hs_index) = (0usize, 0usize);
        while hs_index < level.hand_shapes.len() && chord_index < level.chords.len() {
            let hs = level.hand_shapes[hs_index];
            // last chord at or before the hand shape's end
            let mut i = chord_index;
            while i < level.chords.len() && level.chords[i].time <= hs.end_time {
                i += 1;
            }
            // like the original, stop at the first hand shape with no chord before its end
            let Some(last) = i.checked_sub(1) else { break };
            let time = level.chords[last].time;
            if time >= hs.start_time && time <= hs.end_time && hs.end_time - time <= 10 {
                let (b1, b2) = beats_around(&beats, time);
                let note16th = (b2 - b1) / 4;
                let next_anchor = level.anchors.iter().find(|a| a.time > time).map(|a| a.time);
                let next_phrase = pi_times.iter().copied().find(|&t| t > time);
                let next_content = next_content_time(level, time + 1);
                let new_end = hs.end_time + note16th;
                level.hand_shapes[hs_index].end_time = match min_of(&[next_anchor, next_phrase, next_content]) {
                    Some(next) if new_end >= next => hs.end_time + (next - hs.end_time) / 2,
                    _ => new_end,
                };
            }
            chord_index = last;
            hs_index += 1;
        }
    }
}

fn is_chord_slide_at(level: &fw_xml::Level, time: i32) -> bool {
    match find_index_by_time(&level.chords, time, |c| c.time).map(|i| &level.chords[i]) {
        Some(c) if c.mask.contains(ChordMask::LINK_NEXT) => {
            c.chord_notes.as_ref().is_some_and(|cns| !cns.is_empty() && cns.iter().any(|n| n.is_slide()))
        }
        _ => false,
    }
}

/// Shortens hand shapes that end too close to the next one.
pub fn shorten_hand_shapes(arr: &mut Arr) {
    let beats = arr.ebeats.clone();
    if beats.len() < 2 {
        return;
    }
    for level in &mut arr.levels {
        for i in 1..level.hand_shapes.len() {
            let following = level.hand_shapes[i];
            let preceding = level.hand_shapes[i - 1];
            if preceding.end_time >= following.end_time {
                continue; // nested
            }
            let (b1, b2) = beats_around(&beats, preceding.end_time);
            let note32nd = (b2 - b1) / 8;
            let length = preceding.end_time - preceding.start_time;
            let shorten_by_16th = is_chord_slide_at(level, preceding.start_time) && length > note32nd * 4;
            let min_distance = if length <= note32nd {
                (b2 - b1) / 12
            } else if shorten_by_16th {
                note32nd * 2
            } else {
                note32nd
            };
            if following.start_time - preceding.end_time < min_distance {
                let t = following.start_time - min_distance;
                level.hand_shapes[i - 1].end_time =
                    if t <= preceding.start_time { preceding.start_time + length / 2 } else { t };
            }
        }
    }
}

/// Older (version 7) files mark repeated strums with "high density"; official
/// files instead omit the chord notes. Converts to the official convention and
/// sets the version to 8. Port of `InstrumentalArrangement.FixHighDensity`.
pub fn fix_high_density(arr: &mut Arr) {
    fn remove_high_density(chord: &mut fw_xml::Chord, remove_chord_notes: bool) {
        if chord.mask.contains(ChordMask::HIGH_DENSITY) {
            chord.mask.remove(ChordMask::HIGH_DENSITY);
            if remove_chord_notes {
                // ignore chords with harmonics in them
                if chord.chord_notes.as_ref().is_some_and(|c| c.iter().any(|n| n.mask.contains(NoteMask::HARMONIC))) {
                    chord.mask.insert(ChordMask::IGNORE);
                }
                chord.chord_notes = None;
            }
        }
    }
    let all_sustainless = |c: &fw_xml::Chord| c.chord_notes.as_ref().is_some_and(|cn| !cn.is_empty() && cn.iter().all(|n| n.sustain == 0));

    arr.version = 8;
    for level in &mut arr.levels {
        for hs in level.hand_shapes.clone() {
            let mut starts_with_mute = false;
            let mut chord_num = 0;
            let mut prev_chord_id = -1i16;
            for chord in level.chords.iter_mut().filter(|c| c.time >= hs.start_time && c.time < hs.end_time) {
                chord_num += 1;
                if chord_num == 1 {
                    if chord.mask.contains(ChordMask::FRET_HAND_MUTE) {
                        starts_with_mute = true;
                        // muted chords without techniques have no chord notes
                        if all_sustainless(chord) {
                            chord.chord_notes = None;
                        }
                    } else {
                        remove_high_density(chord, false);
                        prev_chord_id = chord.chord_id;
                        continue;
                    }
                }
                if starts_with_mute && !chord.mask.contains(ChordMask::FRET_HAND_MUTE) {
                    // keep chord notes on the first unmuted chord after muted ones
                    remove_high_density(chord, false);
                    starts_with_mute = false;
                } else {
                    remove_high_density(chord, true);
                    // Toolkit-like behaviour: repeated chords inside a hand shape become strums
                    if chord.chord_id == prev_chord_id && all_sustainless(chord) {
                        chord.chord_notes = None;
                    }
                }
                prev_chord_id = chord.chord_id;
            }
        }
    }
}
