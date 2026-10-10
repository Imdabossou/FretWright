//! Project chart -> Rocksmith arrangement (single level; phrases, fixes and
//! dynamic difficulty are added during the build).

use fw_xml::{self as xml, InstrumentalArrangement};

use crate::Project;

/// Seconds -> milliseconds exactly as a 3-decimal XML round trip would give.
fn ms(t: f64) -> i32 {
    xml::time::parse(&format!("{t:.3}")).unwrap_or(0)
}

/// Anchors (hand positions) following the fretted notes.
fn anchors(notes: &[&crate::Note]) -> Vec<xml::Anchor> {
    let mut out = Vec::new();
    let mut cur: Option<i32> = None;
    for n in notes {
        if n.fret == 0 {
            continue;
        }
        match cur {
            Some(c) if n.fret >= c && n.fret <= c + 3 => {}
            _ => {
                let fret = match cur {
                    Some(c) if n.fret > c + 3 => (n.fret - 1).max(1),
                    _ => n.fret.max(1),
                };
                cur = Some(fret);
                out.push(xml::Anchor { time: ms(n.start), fret: fret as i8, width: 4 });
            }
        }
    }
    if out.is_empty() {
        out.push(xml::Anchor { time: 0, fret: 1, width: 4 });
    }
    out
}

pub fn to_arrangement(p: &Project) -> InstrumentalArrangement {
    let md = &p.metadata;
    let mut notes: Vec<&crate::Note> = p.notes.iter().collect();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let beats = if p.grid.beats.is_empty() { vec![0.0] } else { p.grid.beats.clone() };
    let first_beat = beats[0];
    let last_beat = *beats.last().unwrap_or(&0.0);
    let song_len = p.duration.max(last_beat) + 0.5;
    let music_start = (notes.first().map_or(first_beat, |n| n.start) - 0.001).max(first_beat);
    let end_time = last_beat.max(song_len - 0.5);

    let any = |f: &dyn Fn(&crate::Note) -> bool| notes.iter().any(|n| f(n));
    let standard = p.tuning.iter().all(|&t| t == 0);
    let props = xml::ArrangementProperties {
        represent: true,
        standard_tuning: standard,
        finger_picking: !p.bass_picked,
        hopo: any(&|n| n.hammer_on || n.pull_off),
        slides: any(&|n| n.slide_to >= 0),
        fret_hand_mutes: any(&|n| n.mute),
        two_finger_picking: !p.bass_picked,
        bass_pick: p.bass_picked,
        sustain: any(&|n| n.duration() > 0.3),
        path_bass: true,
        ..Default::default()
    };
    let mut tuning = [0i16; 6];
    for (t, o) in tuning.iter_mut().zip(p.tuning) {
        *t = o as i16;
    }
    let metadata = xml::MetaData {
        title: if md.title.is_empty() { "Untitled".into() } else { md.title.clone() },
        arrangement: "Bass".into(),
        part: 1,
        song_length: ms(song_len),
        average_tempo: p.grid.tempo as f32,
        tuning,
        artist_name: md.artist.clone(),
        album_name: md.album.clone(),
        album_year: md.year,
        arrangement_properties: props,
        ..Default::default()
    };

    let downbeats: std::collections::HashSet<usize> = p.grid.downbeats.iter().copied().collect();
    let mut bar = 0i16;
    let ebeats = beats
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let measure = if downbeats.contains(&i) {
                bar += 1;
                bar
            } else {
                -1
            };
            xml::Ebeat { time: ms(t), measure }
        })
        .collect();

    let level_notes = notes
        .iter()
        .map(|n| {
            let sustain = if n.duration() >= 0.2 || n.slide_to >= 0 { n.duration() } else { 0.0 };
            let mut mask = xml::NoteMask::empty();
            mask.set(xml::NoteMask::ACCENT, n.accent);
            mask.set(xml::NoteMask::HAMMER_ON, n.hammer_on);
            mask.set(xml::NoteMask::PULL_OFF, n.pull_off);
            mask.set(xml::NoteMask::FRET_HAND_MUTE, n.mute);
            xml::Note {
                time: ms(n.start),
                sustain: ms(sustain),
                string: n.string as i8,
                fret: n.fret as i8,
                slide_to: n.slide_to as i8,
                mask,
                ..Default::default()
            }
        })
        .collect();

    let phrase = |name: &str| xml::Phrase { name: name.into(), ..Default::default() };
    let pi = |t: f64, id: i32| xml::PhraseIteration { time: ms(t), phrase_id: id, ..Default::default() };
    InstrumentalArrangement {
        version: 7,
        metadata,
        phrases: vec![phrase("COUNT"), phrase("riff"), phrase("END")],
        phrase_iterations: vec![pi(first_beat, 0), pi(music_start, 1), pi(end_time, 2)],
        linked_diffs: Some(Vec::new()),
        phrase_properties: Some(Vec::new()),
        ebeats,
        tones: xml::ToneInfo { base: Some("Default_Bass".into()), ..Default::default() },
        sections: vec![
            xml::Section { name: "intro".into(), number: 1, time: ms(first_beat) },
            xml::Section { name: "noguitar".into(), number: 1, time: ms(end_time) },
        ],
        events: vec![xml::Event { time: ms(end_time), code: "END".into() }],
        levels: vec![xml::Level {
            difficulty: 0,
            notes: level_notes,
            anchors: anchors(&notes),
            ..Default::default()
        }],
        ..Default::default()
    }
}
