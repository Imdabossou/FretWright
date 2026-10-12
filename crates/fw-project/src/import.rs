//! Official Rocksmith charts -> FretWright projects.
//!
//! Reads a bass arrangement (the `.xml` from a Toolkit project, or the one
//! inside a `.psarc`) with every technique it carries, so a finished chart can
//! be viewed, heard and edited, and used as the answer key for detection.

use std::path::Path;

use fw_xml::{self as xml, InstrumentalArrangement};

use crate::{Error, Metadata, Note, Project, Result, BeatGrid, STANDARD_BASS};

/// The notes a player sees: for every phrase iteration the level of its
/// phrase's maximum difficulty, with chords split into their notes.
pub fn chart_notes(arr: &InstrumentalArrangement) -> Vec<Note> {
    let tuning = arr.metadata.tuning;
    let pitch = |string: i32, fret: i32| STANDARD_BASS[(string as usize).min(3)] + tuning[(string as usize).min(5)] as i32 + fret;
    let mk = |x: &xml::Note, mask_extra: xml::NoteMask| {
        let secs = |ms: i32| ms as f64 / 1000.0;
        let mut n = Note::new(secs(x.time), secs(x.time + x.sustain.max(0)), pitch(x.string as i32, x.fret as i32));
        n.string = x.string as i32;
        n.fret = x.fret as i32;
        let m = x.mask | mask_extra;
        n.hammer_on = m.contains(xml::NoteMask::HAMMER_ON);
        n.pull_off = m.contains(xml::NoteMask::PULL_OFF);
        n.accent = m.contains(xml::NoteMask::ACCENT);
        n.mute = m.contains(xml::NoteMask::FRET_HAND_MUTE);
        n.palm_mute = m.contains(xml::NoteMask::PALM_MUTE);
        n.harmonic = m.intersects(xml::NoteMask::HARMONIC | xml::NoteMask::PINCH_HARMONIC);
        n.tremolo = m.contains(xml::NoteMask::TREMOLO);
        n.slap = m.contains(xml::NoteMask::SLAP);
        n.pop = m.contains(xml::NoteMask::PLUCK);
        n.link_next = m.contains(xml::NoteMask::LINK_NEXT);
        n.tap = x.tap > 0;
        n.vibrato = x.vibrato > 0;
        n.slide_to = if x.slide_to >= 0 { x.slide_to as i32 } else { -1 };
        n.slide_unpitch_to = if x.slide_unpitch_to >= 0 { x.slide_unpitch_to as i32 } else { -1 };
        n.bend = if x.max_bend > 0.0 {
            x.max_bend
        } else {
            x.bend_values.iter().flatten().map(|b| b.step).fold(0.0, f32::max)
        };
        n
    };

    let mut out: Vec<Note> = Vec::new();
    for (i, it) in arr.phrase_iterations.iter().enumerate() {
        let from = it.time;
        let to = arr.phrase_iterations.get(i + 1).map_or(i32::MAX, |n| n.time);
        let max = arr.phrases.get(it.phrase_id as usize).map_or(0, |p| p.max_difficulty) as i8;
        let Some(level) = arr.levels.iter().find(|l| l.difficulty == max).or_else(|| arr.levels.last()) else { continue };
        out.extend(level.notes.iter().filter(|n| n.time >= from && n.time < to).map(|n| mk(n, xml::NoteMask::empty())));
        for c in level.chords.iter().filter(|c| c.time >= from && c.time < to) {
            let mut extra = xml::NoteMask::empty();
            extra.set(xml::NoteMask::ACCENT, c.mask.contains(xml::ChordMask::ACCENT));
            extra.set(xml::NoteMask::FRET_HAND_MUTE, c.mask.contains(xml::ChordMask::FRET_HAND_MUTE));
            extra.set(xml::NoteMask::PALM_MUTE, c.mask.contains(xml::ChordMask::PALM_MUTE));
            extra.set(xml::NoteMask::LINK_NEXT, c.mask.contains(xml::ChordMask::LINK_NEXT));
            if let Some(cn) = c.chord_notes.as_ref().filter(|cn| !cn.is_empty()) {
                out.extend(cn.iter().map(|n| mk(&xml::Note { time: c.time, ..n.clone() }, extra)));
            } else if let Some(t) = arr.chord_templates.get(c.chord_id.max(0) as usize) {
                let notes = (0..4).filter(|&s| t.frets[s] >= 0).map(|s| xml::Note { time: c.time, string: s as i8, fret: t.frets[s], ..Default::default() });
                out.extend(notes.map(|n| mk(&n, extra)));
            }
        }
    }
    finish_notes(&mut out);
    out
}

/// A project holding `arr`'s chart. `audio_path` is the song to play with it.
pub fn project_from_arrangement(arr: &InstrumentalArrangement, folder: impl Into<std::path::PathBuf>, audio_path: impl Into<String>) -> Project {
    let m = &arr.metadata;
    let mut p = Project::new(folder, audio_path);
    p.metadata = Metadata {
        title: m.title.clone(),
        artist: m.artist_name.clone(),
        album: m.album_name.clone(),
        year: m.album_year,
        version: "1".into(),
        ..Default::default()
    };
    p.tuning = std::array::from_fn(|i| m.tuning[i] as i32);
    p.tuning_pitch = 440.0 * 2f64.powf(m.cent_offset as f64 / 1200.0);
    p.bass_picked = m.arrangement_properties.bass_pick;
    p.duration = m.song_length as f64 / 1000.0;
    p.notes = chart_notes(arr);
    let beats: Vec<f64> = arr.ebeats.iter().map(|b| b.time as f64 / 1000.0).collect();
    let downbeats = arr.ebeats.iter().enumerate().filter(|(_, b)| b.measure >= 0).map(|(i, _)| i).collect();
    p.grid = BeatGrid { beats, downbeats, tempo: if m.average_tempo > 0.0 { m.average_tempo as f64 } else { 120.0 } };
    p
}

fn apply_sng_mask(n: &mut Note, m: fw_sng::NoteMask) {
    use fw_sng::NoteMask as M;
    n.hammer_on |= m.contains(M::HAMMER_ON);
    n.pull_off |= m.contains(M::PULL_OFF);
    n.accent |= m.contains(M::ACCENT);
    n.mute |= m.intersects(M::MUTE | M::FRET_HAND_MUTE);
    n.palm_mute |= m.contains(M::PALM_MUTE);
    n.harmonic |= m.intersects(M::HARMONIC | M::PINCH_HARMONIC);
    n.tremolo |= m.contains(M::TREMOLO);
    n.slap |= m.contains(M::SLAP);
    n.pop |= m.contains(M::PLUCK);
    n.tap |= m.contains(M::TAP);
    n.vibrato |= m.contains(M::VIBRATO);
    n.link_next |= m.contains(M::PARENT);
}

/// The notes a player sees in a compiled (`.sng`) chart: for every phrase
/// iteration the level of its phrase's maximum difficulty, with chords split
/// into their notes.
pub fn sng_notes(sng: &fw_sng::Sng) -> Vec<Note> {
    use fw_sng::NoteMask as M;
    let tuning = |s: usize| sng.metadata.tuning.get(s).copied().unwrap_or(0) as i32;
    let mut out: Vec<Note> = Vec::new();
    for it in &sng.phrase_iterations {
        let max = sng.phrases.get(it.phrase_id.max(0) as usize).map_or(0, |p| p.max_difficulty);
        let Some(level) = sng.levels.iter().find(|l| l.difficulty == max).or_else(|| sng.levels.last()) else { continue };
        for x in level.notes.iter().filter(|x| x.time >= it.start_time && x.time < it.end_time) {
            let single = |string: i32, fret: i32| {
                let mut n = Note::new(x.time as f64, (x.time + x.sustain.max(0.0)) as f64, STANDARD_BASS[(string as usize).min(3)] + tuning(string as usize) + fret);
                n.string = string;
                n.fret = fret;
                apply_sng_mask(&mut n, x.mask);
                n
            };
            if x.mask.contains(M::CHORD) {
                let Some(chord) = sng.chords.get(x.chord_id.max(0) as usize) else { continue };
                let details = sng.chord_notes.get(x.chord_notes_id.max(0) as usize).filter(|_| x.chord_notes_id >= 0);
                for s in (0..4).filter(|&s| chord.frets[s] >= 0) {
                    let mut n = single(s as i32, chord.frets[s] as i32);
                    if let Some(d) = details {
                        apply_sng_mask(&mut n, d.mask[s]);
                        n.slide_to = if d.slide_to[s] >= 0 { d.slide_to[s] as i32 } else { -1 };
                        n.slide_unpitch_to = if d.slide_unpitch_to[s] >= 0 { d.slide_unpitch_to[s] as i32 } else { -1 };
                        n.vibrato |= d.vibrato[s] > 0;
                        let bend = &d.bend_data[s];
                        n.bend = bend.bend_values.iter().take(bend.used_count.max(0) as usize).map(|b| b.step).fold(0.0, f32::max);
                    }
                    out.push(n);
                }
            } else {
                let mut n = single(x.string_index as i32, x.fret as i32);
                n.slide_to = if x.slide_to >= 0 { x.slide_to as i32 } else { -1 };
                n.slide_unpitch_to = if x.slide_unpitch_to >= 0 { x.slide_unpitch_to as i32 } else { -1 };
                n.vibrato |= x.vibrato > 0;
                n.tap |= x.tap > 0;
                n.bend = if x.max_bend > 0.0 { x.max_bend } else { x.bend_data.iter().map(|b| b.step).fold(0.0, f32::max) };
                out.push(n);
            }
        }
    }
    finish_notes(&mut out);
    out
}

/// Sorts notes and gives those charted without a sustain a short length, so
/// they can be seen and heard.
fn finish_notes(out: &mut [Note]) {
    out.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.midi.cmp(&b.midi)));
    for i in 0..out.len() {
        if out[i].duration() < 0.05 {
            let next = out[i + 1..].iter().map(|n| n.start).find(|&t| t > out[i].start + 1e-3);
            out[i].end = out[i].start + 0.2f64.min(next.map_or(0.2, |t| t - out[i].start));
        }
    }
}

fn read_entry<R: std::io::Read + std::io::Seek>(a: &mut fw_psarc::Psarc<R>, name: &str) -> Result<Vec<u8>> {
    a.read(name).map_err(|e| Error::Other(format!("{name}: {e}")))
}

/// Imports the bass arrangement and song audio of a `.psarc` into `folder`
/// (the audio is written there as `song.wav`).
pub fn import_psarc(psarc: &Path, folder: &Path) -> Result<Project> {
    let mut archive = fw_psarc::Psarc::open(psarc).map_err(|e| Error::Other(format!("{}: {e}", psarc.display())))?;
    let names: Vec<String> = archive.names().to_vec();

    // charts shipped as .xml (custom packages) are read as they are
    let mut chart: Option<Chart> = None;
    for name in names.iter().filter(|n| n.starts_with("songs/arr/") && n.ends_with(".xml") && !n.contains("showlights") && !n.contains("vocals")) {
        let text = String::from_utf8_lossy(&read_entry(&mut archive, name)?).into_owned();
        if let Ok(arr) = InstrumentalArrangement::parse(&text)
            && arr.metadata.arrangement.eq_ignore_ascii_case("bass")
        {
            chart = Some(Chart::Xml(Box::new(arr)));
            break;
        }
    }
    // official packages only carry the compiled charts the game plays
    if chart.is_none()
        && let Some(name) = names.iter().find(|n| n.starts_with("songs/bin/") && n.ends_with("_bass.sng"))
    {
        let platform = if name.contains("/macos/") || name.contains("/mac/") { fw_sng::Platform::Mac } else { fw_sng::Platform::Pc };
        let sng = fw_sng::Sng::from_packed(&read_entry(&mut archive, name)?, platform).map_err(|e| Error::Other(format!("{name}: {e}")))?;
        let prefix = name.rsplit('/').next().unwrap_or("").trim_end_matches("_bass.sng").to_string();
        let manifest = names
            .iter()
            .find(|n| n.starts_with("manifests/") && n.ends_with(&format!("{prefix}_bass.json")))
            .and_then(|n| read_entry(&mut archive, n).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        chart = Some(Chart::Sng(Box::new(sng), manifest));
    }
    let chart = chart.ok_or_else(|| Error::Other("this package has no bass arrangement".into()))?;

    // the song is the biggest .wem (the other is the 28 s preview)
    let song = archive
        .entries()
        .iter()
        .zip(&names)
        .filter(|(_, n)| n.starts_with("audio/") && n.ends_with(".wem"))
        .max_by_key(|(e, _)| e.length)
        .map(|(_, n)| n.clone())
        .ok_or_else(|| Error::Other("this package has no audio".into()))?;
    let audio = fw_audio::decode_wem(&read_entry(&mut archive, &song)?)?;
    std::fs::create_dir_all(folder)?;
    let wav = folder.join("song.wav");
    std::fs::write(&wav, audio.to_wav())?;

    let wav = wav.to_string_lossy().into_owned();
    let mut p = match chart {
        Chart::Xml(arr) => project_from_arrangement(&arr, folder, wav),
        Chart::Sng(sng, manifest) => project_from_sng(&sng, manifest.as_ref(), folder, wav),
    };
    if p.duration <= 0.0 {
        p.duration = audio.duration();
    }
    Ok(p)
}

enum Chart {
    Xml(Box<InstrumentalArrangement>),
    Sng(Box<fw_sng::Sng>, Option<serde_json::Value>),
}

/// A project holding a compiled chart. Titles come from the package's
/// manifest, which the chart itself does not carry.
pub fn project_from_sng(sng: &fw_sng::Sng, manifest: Option<&serde_json::Value>, folder: impl Into<std::path::PathBuf>, audio_path: impl Into<String>) -> Project {
    let mut p = Project::new(folder, audio_path);
    let attrs = manifest.and_then(|m| m["Entries"].as_object()).and_then(|e| e.values().next()).map(|e| &e["Attributes"]);
    let text = |k: &str| attrs.and_then(|a| a[k].as_str()).unwrap_or("").to_string();
    p.metadata = Metadata {
        title: text("SongName"),
        artist: text("ArtistName"),
        album: text("AlbumName"),
        year: attrs.and_then(|a| a["SongYear"].as_i64()).unwrap_or(0) as i32,
        version: "1".into(),
        ..Default::default()
    };
    p.tuning = std::array::from_fn(|i| sng.metadata.tuning.get(i).copied().unwrap_or(0) as i32);
    p.tuning_pitch = 440.0 * 2f64.powf(attrs.and_then(|a| a["CentOffset"].as_f64()).unwrap_or(0.0) / 1200.0);
    p.duration = sng.metadata.song_length as f64;
    p.notes = sng_notes(sng);
    p.grid = BeatGrid {
        beats: sng.beats.iter().map(|b| b.time as f64).collect(),
        downbeats: sng.beats.iter().enumerate().filter(|(_, b)| b.mask.contains(fw_sng::BeatMask::FIRST_BEAT_OF_MEASURE)).map(|(i, _)| i).collect(),
        tempo: attrs.and_then(|a| a["SongAverageTempo"].as_f64()).filter(|t| *t > 0.0).unwrap_or(120.0),
    };
    p
}

/// Imports a bare arrangement `.xml`, to be played with `audio`.
pub fn import_xml(xml_path: &Path, audio: &Path, folder: &Path) -> Result<Project> {
    let arr = InstrumentalArrangement::load(xml_path).map_err(|e| Error::Other(format!("{}: {e}", xml_path.display())))?;
    let mut p = project_from_arrangement(&arr, folder, audio.to_string_lossy().into_owned());
    if p.duration <= 0.0 {
        p.duration = fw_audio::decode(audio)?.duration();
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrangement() -> InstrumentalArrangement {
        let note = |time, string, fret, f: &dyn Fn(&mut xml::Note)| {
            let mut n = xml::Note { time, sustain: 500, string, fret, ..Default::default() };
            f(&mut n);
            n
        };
        let notes = vec![
            note(1000, 1, 3, &|n| n.mask = xml::NoteMask::HAMMER_ON | xml::NoteMask::ACCENT),
            note(2000, 2, 5, &|n| n.slide_to = 9),
            note(3000, 0, 2, &|n| n.slide_unpitch_to = 12),
            note(4000, 3, 7, &|n| n.max_bend = 1.0),
            note(5000, 1, 0, &|n| n.mask = xml::NoteMask::HARMONIC | xml::NoteMask::PALM_MUTE | xml::NoteMask::LINK_NEXT),
            note(6000, 1, 4, &|n| n.mask = xml::NoteMask::SLAP | xml::NoteMask::TREMOLO),
            note(7000, 1, 4, &|n| n.mask = xml::NoteMask::PLUCK | xml::NoteMask::PULL_OFF),
        ];
        let mut arr = InstrumentalArrangement::default();
        arr.metadata.tuning = [-2, 0, 0, 0, 0, 0];
        arr.phrases = vec![xml::Phrase { name: "riff".into(), max_difficulty: 0, ..Default::default() }];
        arr.phrase_iterations = vec![xml::PhraseIteration { time: 0, phrase_id: 0, ..Default::default() }];
        arr.levels = vec![xml::Level { difficulty: 0, notes, ..Default::default() }];
        arr
    }

    #[test]
    fn techniques_survive_import_and_export() {
        let arr = arrangement();
        let p = project_from_arrangement(&arr, "/tmp/x", "a.wav");
        assert_eq!(p.tuning, [-2, 0, 0, 0]);
        let n = &p.notes;
        assert_eq!(n.len(), 7);
        assert_eq!(n[0].midi, 33 + 3);
        assert!(n[0].hammer_on && n[0].accent);
        assert_eq!(n[1].slide_to, 9);
        assert_eq!(n[2].slide_unpitch_to, 12);
        assert_eq!(n[3].bend, 1.0);
        assert!(n[4].harmonic && n[4].palm_mute && n[4].link_next);
        assert!(n[5].slap && n[5].tremolo);
        assert!(n[6].pop && n[6].pull_off);

        let out = crate::to_arrangement(&p);
        let x = &out.levels[0].notes;
        assert_eq!(x.len(), 7);
        assert!(x[0].mask.contains(xml::NoteMask::HAMMER_ON | xml::NoteMask::ACCENT));
        assert_eq!((x[1].slide_to, x[2].slide_unpitch_to), (9, 12));
        assert!(x[3].is_bend() && x[3].max_bend == 1.0);
        assert!(x[4].mask.contains(xml::NoteMask::HARMONIC | xml::NoteMask::PALM_MUTE | xml::NoteMask::LINK_NEXT));
        assert!(x[5].mask.contains(xml::NoteMask::SLAP | xml::NoteMask::TREMOLO));
        assert!(x[6].mask.contains(xml::NoteMask::PLUCK | xml::NoteMask::PULL_OFF));
        let props = out.metadata.arrangement_properties;
        assert!(props.hopo && props.slides && props.unpitched_slides && props.bends && props.harmonics && props.slap_pop && props.tremolo);
    }
}
