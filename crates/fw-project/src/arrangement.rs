//! Project chart -> Rocksmith arrangement (single level; phrases, fixes and
//! dynamic difficulty are added during the build).

use fw_xml::{self as xml, InstrumentalArrangement};

use crate::Project;

/// Seconds -> milliseconds exactly as a 3-decimal XML round trip would give.
fn ms(t: f64) -> i32 {
    xml::time::parse(&format!("{t:.3}")).unwrap_or(0)
}

/// Anchors (hand positions) following the fretted notes. The hand stays put
/// while the notes fit in its four frets; when one falls outside, it moves by
/// the smallest shift that covers the whole run of notes coming up (looking
/// ahead, so a riff over frets 1-3 never flips between windows). A run that
/// needs a five-fret stretch gets a five-fret window rather than a jump.
pub(crate) fn anchors(notes: &[&crate::Note]) -> Vec<xml::Anchor> {
    const W: i32 = 4;
    const STRETCH: i32 = 5;
    // (start, lowest fret, highest fret) of every fretted note, slides included.
    let ev: Vec<(f64, i32, i32)> = notes
        .iter()
        .filter(|n| n.fret > 0)
        .map(|n| {
            let end = if n.slide_to > 0 { n.slide_to } else { n.fret };
            (n.start, n.fret.min(end), n.fret.max(end))
        })
        .collect();
    let mut out = Vec::new();
    let mut cur: Option<(i32, i32)> = None; // (first fret, width)
    for (i, &(t, lo, hi)) in ev.iter().enumerate() {
        if let Some((f, w)) = cur
            && lo >= f
            && hi < f + w
        {
            continue;
        }
        let limit = STRETCH.max(hi - lo + 1);
        let (mut mn, mut mx) = (lo, hi);
        for &(_, l, h) in &ev[i + 1..] {
            if h.max(mx) - l.min(mn) + 1 > limit {
                break;
            }
            mn = mn.min(l);
            mx = mx.max(h);
        }
        let width = (mx - mn + 1).max(W);
        let fret = match cur {
            Some((f, _)) if mx > f => mx - width + 1,
            _ => mn,
        }
        .max(1);
        cur = Some((fret, width));
        out.push(xml::Anchor {
            time: ms(t),
            fret: fret as i8,
            width: width.min(24) as i8,
        });
    }
    if out.is_empty() {
        out.push(xml::Anchor {
            time: 0,
            fret: 1,
            width: 4,
        });
    }
    out
}

pub fn to_arrangement(p: &Project) -> InstrumentalArrangement {
    let md = &p.metadata;
    let mut notes: Vec<&crate::Note> = p.notes.iter().collect();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let beats = if p.grid.beats.is_empty() {
        vec![0.0]
    } else {
        p.grid.beats.clone()
    };
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
        unpitched_slides: any(&|n| n.slide_unpitch_to >= 0),
        bends: any(&|n| n.bend > 0.0),
        harmonics: any(&|n| n.harmonic),
        palm_mutes: any(&|n| n.palm_mute),
        tremolo: any(&|n| n.tremolo),
        tapping: any(&|n| n.tap),
        vibrato: any(&|n| n.vibrato),
        slap_pop: any(&|n| n.slap || n.pop),
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
        title: if md.title.is_empty() {
            "Untitled".into()
        } else {
            md.title.clone()
        },
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
            xml::Ebeat {
                time: ms(t),
                measure,
            }
        })
        .collect();

    let level_notes = notes
        .iter()
        .map(|n| {
            let sustain = n.chart_sustain();
            let mut mask = xml::NoteMask::empty();
            mask.set(xml::NoteMask::ACCENT, n.accent);
            mask.set(xml::NoteMask::HAMMER_ON, n.hammer_on);
            mask.set(xml::NoteMask::PULL_OFF, n.pull_off);
            mask.set(xml::NoteMask::FRET_HAND_MUTE, n.mute);
            mask.set(xml::NoteMask::PALM_MUTE, n.palm_mute);
            mask.set(xml::NoteMask::HARMONIC, n.harmonic);
            mask.set(xml::NoteMask::TREMOLO, n.tremolo);
            mask.set(xml::NoteMask::SLAP, n.slap);
            mask.set(xml::NoteMask::PLUCK, n.pop);
            mask.set(xml::NoteMask::LINK_NEXT, n.link_next);
            let bend_values = (n.bend > 0.0).then(|| {
                // up to the bend over the first part of the note, held after that
                let up = (sustain * 0.25 * 1000.0).round() as i32;
                vec![
                    xml::BendValue {
                        time: ms(n.start) + up.max(1),
                        step: n.bend,
                    },
                    xml::BendValue {
                        time: ms(n.start) + ms(sustain).max(2) - 1,
                        step: n.bend,
                    },
                ]
            });
            xml::Note {
                time: ms(n.start),
                sustain: ms(sustain),
                string: n.string as i8,
                fret: n.fret as i8,
                slide_to: n.slide_to as i8,
                slide_unpitch_to: n.slide_unpitch_to as i8,
                tap: i8::from(n.tap),
                vibrato: if n.vibrato { 80 } else { 0 },
                max_bend: n.bend,
                bend_values,
                mask,
                ..Default::default()
            }
        })
        .collect();

    let phrase = |name: &str| xml::Phrase {
        name: name.into(),
        ..Default::default()
    };
    let pi = |t: f64, id: i32| xml::PhraseIteration {
        time: ms(t),
        phrase_id: id,
        ..Default::default()
    };
    InstrumentalArrangement {
        version: 7,
        metadata,
        phrases: vec![phrase("COUNT"), phrase("riff"), phrase("END")],
        phrase_iterations: vec![pi(first_beat, 0), pi(music_start, 1), pi(end_time, 2)],
        linked_diffs: Some(Vec::new()),
        phrase_properties: Some(Vec::new()),
        ebeats,
        tones: xml::ToneInfo {
            base: Some("Default_Bass".into()),
            ..Default::default()
        },
        sections: vec![
            xml::Section {
                name: "intro".into(),
                number: 1,
                time: ms(first_beat),
            },
            xml::Section {
                name: "noguitar".into(),
                number: 1,
                time: ms(end_time),
            },
        ],
        events: vec![xml::Event {
            time: ms(end_time),
            code: "END".into(),
        }],
        levels: vec![xml::Level {
            difficulty: 0,
            notes: level_notes,
            anchors: anchors(&notes),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod anchor_tests {
    use super::*;
    use crate::Note;

    fn fretted(frets: &[(f64, i32)]) -> Vec<Note> {
        frets
            .iter()
            .map(|&(t, f)| {
                let mut n = Note::new(t, t + 0.1, 40 + f);
                n.fret = f;
                n
            })
            .collect()
    }

    fn frets(notes: &[Note]) -> Vec<(i32, i8)> {
        let refs: Vec<&Note> = notes.iter().collect();
        anchors(&refs).iter().map(|a| (a.time, a.fret)).collect()
    }

    #[test]
    fn riff_within_four_frets_keeps_one_position() {
        // Starts on 3 then dips to 1: the look-ahead sees 1-3 and settles on 1.
        let n = fretted(&[
            (1.0, 3),
            (1.5, 3),
            (2.0, 1),
            (2.5, 3),
            (3.0, 2),
            (3.5, 1),
            (4.0, 3),
        ]);
        assert_eq!(frets(&n), vec![(1000, 1)]);
    }

    #[test]
    fn riff_over_five_frets_does_not_flip() {
        // 1-3-5 repeated: the old rule moved to 4 on every 5 and back to 1;
        // it is one stretched position.
        let n = fretted(&[
            (1.0, 1),
            (1.5, 3),
            (2.0, 5),
            (2.5, 3),
            (3.0, 1),
            (3.5, 5),
            (4.0, 3),
        ]);
        let a = frets(&n);
        assert_eq!(a, vec![(1000, 1)]);
        let refs: Vec<&Note> = n.iter().collect();
        assert_eq!(anchors(&refs)[0].width, 5);
    }

    #[test]
    fn moves_by_the_smallest_shift() {
        let n = fretted(&[
            (1.0, 1),
            (1.5, 4),
            (2.0, 6),
            (2.5, 3),
            (3.0, 12),
            (3.5, 10),
            (4.0, 0),
        ]);
        assert_eq!(frets(&n), vec![(1000, 1), (2000, 3), (3000, 9)]);
    }
}
