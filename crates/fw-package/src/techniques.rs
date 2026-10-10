//! Technique IDs listed in the manifest (port of Manifest/Techniques.fs).

use fw_sng::{Note, NoteMask as N, Sng};

fn has(n: &Note, f: N) -> bool {
    n.mask.contains(f)
}

fn template_frets(sng: &Sng, n: &Note) -> Option<[i8; 6]> {
    sng.chords.get(n.chord_id as usize).map(|c| c.frets.0)
}

fn played_strings(frets: &[i8; 6]) -> Option<(usize, usize, i8, i8)> {
    let s1 = frets.iter().position(|&f| f >= 0)?;
    let s2 = frets.iter().rposition(|&f| f >= 0)?;
    Some((s1, s2, frets[s1], frets[s2]))
}

fn is_power_chord(sng: &Sng, n: &Note) -> bool {
    has(n, N::DOUBLE_STOP)
        && template_frets(sng, n)
            .and_then(|f| played_strings(&f))
            .is_some_and(|(s1, s2, f1, f2)| s1 <= 2 && s1 + 1 == s2 && f1 + 2 == f2)
}

fn is_drop_d_power(sng: &Sng, n: &Note) -> bool {
    let t = &sng.metadata.tuning;
    has(n, N::CHORD)
        && t.len() >= 3
        && t[0] == t[1] - 2
        && t[0] == t[2] - 2
        && template_frets(sng, n).is_some_and(|f| f[0] != -1 && f[0] == f[1] && f[0] == f[2])
}

fn is_chord(n: &Note) -> bool {
    n.mask & (N::CHORD | N::SUSTAIN | N::DOUBLE_STOP) == N::CHORD
}

fn is_barre(sng: &Sng, n: &Note) -> bool {
    n.mask & (N::CHORD | N::DOUBLE_STOP) == N::CHORD
        && sng.chords.get(n.chord_id as usize).is_some_and(|c| {
            c.fingers.iter().any(|&f| f != -1 && c.fingers.iter().filter(|&&x| x == f).count() >= 3)
        })
}

fn chord_notes<'a>(sng: &'a Sng, n: &Note) -> Option<&'a fw_sng::ChordNotes> {
    sng.chord_notes.get(n.chord_notes_id as usize)
}

fn is_oblique_bend(sng: &Sng, n: &Note) -> bool {
    has(n, N::CHORD_NOTES) && chord_notes(sng, n).is_some_and(|c| c.bend_data.iter().filter(|b| b.used_count > 0).count() == 1)
}

fn is_compound_bend(n: &Note) -> bool {
    has(n, N::BEND) && n.bend_data.len() >= 3 && n.bend_data[0].step != n.bend_data[1].step && n.bend_data[1].step != n.bend_data[2].step
}

fn is_pre_bend(n: &Note) -> bool {
    has(n, N::BEND) && n.bend_data.first().is_some_and(|b| b.time == n.time && b.step > 0.0)
}

fn is_chord_hammer_on(sng: &Sng, n: &Note) -> bool {
    has(n, N::CHORD_NOTES) && chord_notes(sng, n).is_some_and(|c| c.mask.iter().any(|m| m.contains(N::HAMMER_ON)))
}

fn double_stop_adjacent(sng: &Sng, n: &Note) -> bool {
    has(n, N::DOUBLE_STOP) && template_frets(sng, n).and_then(|f| played_strings(&f)).is_some_and(|(s1, s2, ..)| s1 + 1 == s2)
}

const CN_DS: N = N::CHORD_NOTES.union(N::DOUBLE_STOP);

fn chord_notes_only(n: &Note) -> bool {
    n.mask & CN_DS == N::CHORD_NOTES
}

fn any_slide(sng: &Sng, n: &Note) -> bool {
    chord_notes(sng, n).is_some_and(|c| c.slide_to.iter().any(|&s| s != -1))
}

fn any_tremolo(sng: &Sng, n: &Note) -> bool {
    chord_notes(sng, n).is_some_and(|c| c.mask.iter().any(|m| m.contains(N::TREMOLO)))
}

/// Technique numbers of a note, in ascending order.
pub fn techniques(sng: &Sng, n: &Note) -> Vec<i32> {
    if n.mask.is_empty() || n.mask == N::SINGLE || n.mask == (N::SINGLE | N::OPEN) {
        return Vec::new();
    }
    let hopo = n.mask.intersects(N::HAMMER_ON | N::PULL_OFF);
    let power = is_power_chord(sng, n);
    let mut t = Vec::new();
    let mut add = |cond: bool, id: i32| {
        if cond {
            t.push(id)
        }
    };
    add(has(n, N::ACCENT), 0);
    add(has(n, N::BEND), 1);
    add(has(n, N::FRET_HAND_MUTE), 2);
    add(has(n, N::HAMMER_ON), 3);
    add(has(n, N::HARMONIC), 4);
    add(has(n, N::PINCH_HARMONIC), 5);
    add(hopo, 6);
    add(has(n, N::PALM_MUTE), 7);
    add(has(n, N::PLUCK), 8);
    add(has(n, N::PULL_OFF), 9);
    add(has(n, N::SLAP), 10);
    add(has(n, N::SLIDE), 11);
    add(has(n, N::UNPITCHED_SLIDE), 12);
    add(has(n, N::SINGLE) && has(n, N::SUSTAIN), 13);
    add(has(n, N::TAP), 14);
    add(has(n, N::TREMOLO), 15);
    add(has(n, N::VIBRATO), 16);
    add(has(n, N::PALM_MUTE | N::ACCENT), 17);
    add(has(n, N::PALM_MUTE | N::HARMONIC), 18);
    add(has(n, N::PALM_MUTE | N::HAMMER_ON), 19);
    add(has(n, N::PALM_MUTE | N::PULL_OFF), 20);
    add(has(n, N::FRET_HAND_MUTE | N::ACCENT), 21);
    add(has(n, N::BEND | N::TREMOLO), 26);
    add(has(n, N::TREMOLO | N::SLIDE), 27);
    add(has(n, N::TREMOLO | N::VIBRATO), 28);
    add(is_pre_bend(n), 29);
    add(is_oblique_bend(sng, n), 30);
    add(is_compound_bend(n), 31);
    add(!power && double_stop_adjacent(sng, n), 33);
    add(!power && has(n, N::DOUBLE_STOP) && !double_stop_adjacent(sng, n), 34);
    add(power, 35);
    add(is_drop_d_power(sng, n), 36);
    add(is_barre(sng, n), 37);
    add(is_chord(n), 38);
    add(hopo && n.finger_print_id[0] != -1, 40);
    add(chord_notes_only(n) && any_slide(sng, n), 41);
    add(chord_notes_only(n) && any_tremolo(sng, n), 42);
    add(is_chord_hammer_on(sng, n), 43);
    add(has(n, CN_DS) && any_slide(sng, n), 44);
    add(has(n, CN_DS) && any_tremolo(sng, n), 45);
    add(has(n, CN_DS) && chord_notes(sng, n).is_some_and(|c| c.bend_data.iter().any(|b| b.used_count > 0)), 46);
    t
}
