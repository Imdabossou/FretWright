use fw_sng as sng;
use fw_xml as xml;

/// Milliseconds -> seconds, in single precision like the original.
pub fn ms_to_sec(ms: i32) -> f32 {
    ms as f32 / 1000.0
}

/// Index of the phrase iteration containing `time`. `inclusive`: an iteration
/// starting exactly at `time` contains it.
fn find_pi_impl(inclusive: bool, time: i32, its: &[xml::PhraseIteration]) -> i32 {
    let mut id = its.len() as i32 - 1;
    while id > 0 {
        let t = its[id as usize].time;
        if (inclusive && t == time) || t < time {
            break;
        }
        id -= 1;
    }
    id
}

pub fn find_pi(time: i32, its: &[xml::PhraseIteration]) -> i32 {
    find_pi_impl(true, time, its)
}

/// Beats exactly on a phrase iteration belong to the previous one.
pub fn find_beat_pi(time: i32, its: &[xml::PhraseIteration]) -> i32 {
    find_pi_impl(false, time, its)
}

pub fn find_section(time: i32, sections: &[xml::Section]) -> usize {
    let mut id = sections.len().saturating_sub(1);
    while id > 0 && sections[id].time > time {
        id -= 1;
    }
    id
}

/// The last anchor at or before `time`.
pub fn find_anchor(time: i32, anchors: &[xml::Anchor]) -> Option<&xml::Anchor> {
    anchors.iter().rev().find(|a| a.time <= time)
}

pub fn find_finger_print(time: f32, fps: &[sng::FingerPrint]) -> i16 {
    for (i, fp) in fps.iter().enumerate() {
        if fp.start_time > time {
            break;
        }
        if time >= fp.start_time && time < fp.end_time {
            return i as i16;
        }
    }
    -1
}

/// Indexes of the first and last note times within `[start, end)`.
pub fn first_and_last(times: &[i32], start: i32, end: i32) -> Option<(usize, usize)> {
    let first = times.partition_point(|&t| t < start);
    if first == times.len() || times[first] >= end {
        return None;
    }
    let after = times.partition_point(|&t| t < end);
    Some((first, after - 1))
}

const STANDARD_MIDI: [i32; 6] = [40, 45, 50, 55, 59, 64];

pub fn midi_note(string: usize, fret: i8, tuning: &[i16; 6], capo: i8, bass: bool) -> i32 {
    let fret = if capo > 0 && fret == 0 { capo as i32 } else { fret as i32 };
    STANDARD_MIDI[string] + tuning[string] as i32 + fret + if bass { -12 } else { 0 }
}

pub fn convert_chord_template(arr: &xml::InstrumentalArrangement, t: &xml::ChordTemplate) -> sng::Chord {
    let md = &arr.metadata;
    let mask = if t.is_arpeggio() {
        sng::ChordMask::ARPEGGIO
    } else if t.display_name.ends_with("-nop") {
        sng::ChordMask::NOP
    } else {
        sng::ChordMask::empty()
    };
    let mut notes = [-1i32; 6];
    for (s, n) in notes.iter_mut().enumerate() {
        if t.frets[s] != -1 {
            *n = midi_note(s, t.frets[s], &md.tuning, md.capo, md.arrangement_properties.path_bass);
        }
    }
    sng::Chord {
        mask,
        frets: sng::Fixed(t.frets),
        fingers: sng::Fixed(t.fingers),
        notes: sng::Fixed(notes),
        name: t.name.as_str().into(),
    }
}

pub fn is_double_stop(t: &xml::ChordTemplate) -> bool {
    t.frets.iter().filter(|&&f| f != -1).count() == 2
}

/// Identity of a chord-notes record for de-duplication.
pub fn chord_notes_key(cn: &sng::ChordNotes) -> Vec<u8> {
    let tmp = sng::Sng { chord_notes: vec![cn.clone()], ..Default::default() };
    tmp.write()
}

/// Stable 32-bit FNV-1a hash of a note's fields (excluding the hash itself).
pub fn note_hash(n: &sng::Note) -> u32 {
    let mut probe = n.clone();
    probe.hash = 0;
    let level = sng::Level { notes: vec![probe], ..Default::default() };
    let bytes = sng::Sng { levels: vec![level], ..Default::default() }.write();
    let mut h: u32 = 0x811c_9dc5;
    for b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// "MM-d-yy HH:mm" in UTC, the format the game's tools write.
pub fn conversion_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!("{:02}-{}-{:02} {:02}:{:02}", month, day, year % 100, tod / 3600, tod % 3600 / 60)
}
