//! Venue light show generated from the chart (port of ShowLightGenerator.fs).

use std::collections::HashMap;

use fw_sng::Sng;

use crate::Rng;

const FOG_MIN: u8 = 24;
const FOG_MAX: u8 = 35;
const BEAM_OFF: u8 = 42;
const BEAM_MIN: u8 = 48;
const BEAM_MAX: u8 = 59;
const LASERS_OFF: u8 = 66;
const LASERS_ON: u8 = 67;

fn to_ms(secs: f32) -> i32 {
    (secs * 1000.0) as i32
}

fn midi_note(sng: &Sng, n: &fw_sng::Note) -> i32 {
    if n.chord_id == -1 {
        let s = n.string_index.clamp(0, 5) as usize;
        let tuning = sng.metadata.tuning.get(s).copied().unwrap_or(0) as i32;
        let capo = sng.metadata.capo_fret_id;
        let fret = if capo > 0 && n.fret == 0 { capo as i32 } else { n.fret as i32 };
        [40, 45, 50, 55, 59, 64][s] + tuning + fret
    } else {
        sng.chords.get(n.chord_id as usize).and_then(|c| c.notes.iter().copied().find(|&x| x > 0)).unwrap_or(0)
    }
}

/// Show light events (time ms, note), sorted by time.
pub fn generate(sng: &Sng, rng: &mut Rng) -> Vec<(i32, u8)> {
    let mut out: Vec<(i32, u8)> = Vec::new();

    // fog: one colour per distinct section name
    let mut fog_by_section: HashMap<&str, u8> = HashMap::new();
    let mut prev_name = "";
    let mut last_fog: Option<u8> = None;
    for s in &sng.sections {
        let name = s.name.0.as_str();
        if name == prev_name {
            continue;
        }
        prev_name = name;
        let note = *fog_by_section.entry(name).or_insert_with(|| {
            let exclude = last_fog.unwrap_or(0);
            loop {
                let f = FOG_MIN + rng.below((FOG_MAX - FOG_MIN + 1) as u32) as u8;
                if f != exclude {
                    break f;
                }
            }
        });
        last_fog = Some(note);
        out.push((to_ms(s.start_time), note));
    }

    // beams follow the highest-level notes' pitch class, at most every 0.35 s
    let mut beams: Vec<(f32, u8)> = Vec::new();
    for pi in &sng.phrase_iterations {
        let max = sng.phrases.get(pi.phrase_id as usize).map_or(0, |p| p.max_difficulty as usize);
        let Some(level) = sng.levels.get(max) else { continue };
        for n in level.notes.iter().filter(|n| n.time >= pi.start_time && n.time < pi.end_time) {
            let beam = BEAM_MIN + (midi_note(sng, n).rem_euclid(256) as u8 % 12);
            match beams.last() {
                None => beams.push((n.time, beam)),
                Some(&(t, b)) if n.time - t >= 0.35 && beam != b => beams.push((n.time, beam)),
                _ => {}
            }
        }
    }
    out.extend(beams.into_iter().map(|(t, b)| (to_ms(t), b)));

    // lasers: on at the first solo section, else 60% into the song; off 5 s before the end
    let len = sng.metadata.song_length;
    let on = sng.sections.iter().find(|s| crate::strings::starts_with_ci(&s.name.0, "solo")).map_or(len * 0.6, |s| s.start_time);
    out.push((to_ms(on), LASERS_ON));
    out.push((to_ms(len - 5.0), LASERS_OFF));

    let is_beam = |n: u8| (BEAM_MIN..=BEAM_MAX).contains(&n) || n == BEAM_OFF;
    let is_fog = |n: u8| (FOG_MIN..=FOG_MAX).contains(&n);
    if !out.iter().any(|&(_, n)| is_beam(n)) {
        out.push((0, BEAM_MIN));
    }
    if !out.iter().any(|&(_, n)| is_fog(n)) {
        out.push((0, FOG_MIN));
    }
    out.push((to_ms(len), FOG_MAX)); // prevents a glitch at the end
    out.sort_by_key(|&(t, _)| t); // stable, like List.sortBy
    out
}

pub fn to_xml(lights: &[(i32, u8)]) -> Vec<u8> {
    let mut s = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<showlights count=\"{}\">", lights.len());
    for (t, n) in lights {
        s.push_str(&format!("\n  <showlight time=\"{}\" note=\"{n}\" />", fw_xml::time::to_string(*t)));
    }
    s.push_str("\n</showlights>");
    s.into_bytes()
}
