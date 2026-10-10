#![allow(clippy::unwrap_used)]
use fw_sng::*;

fn sample() -> Sng {
    let note = Note {
        mask: NoteMask::SINGLE | NoteMask::SUSTAIN,
        time: 1.5,
        string_index: 1,
        fret: 3,
        chord_id: -1,
        bend_data: vec![BendValue { time: 1.6, step: 0.5, ..Default::default() }],
        ..Default::default()
    };
    Sng {
        beats: vec![Beat { time: 0.0, measure: 1, mask: BeatMask::FIRST_BEAT_OF_MEASURE, ..Default::default() }],
        phrases: vec![Phrase { name: "riff".into(), max_difficulty: 3, ..Default::default() }],
        vocals: vec![Vocal { time: 1.0, lyric: "héllo".into(), ..Default::default() }],
        symbols_headers: vec![SymbolsHeader { unk8: 2, ..Default::default() }],
        levels: vec![Level { difficulty: 0, notes: vec![note], ..Default::default() }],
        metadata: MetaData { tuning: vec![0; 6], first_note_time: 1.5, song_length: 10.0, ..Default::default() },
        ..Default::default()
    }
}

#[test]
fn roundtrips_plain_and_packed() {
    let sng = sample();
    assert_eq!(Sng::read(&sng.write()).unwrap(), sng);
    for p in [Platform::Pc, Platform::Mac] {
        assert_eq!(Sng::from_packed(&sng.to_packed(p), p).unwrap(), sng);
    }
}

#[test]
fn wrong_key_fails_cleanly() {
    let packed = sample().to_packed(Platform::Pc);
    assert!(Sng::from_packed(&packed, Platform::Mac).is_err());
}

#[test]
fn long_strings_are_truncated_on_char_boundary() {
    let mut sng = sample();
    sng.phrases[0].name = Str("ä".repeat(40)); // 80 bytes into a 32-byte field
    let back = Sng::read(&sng.write()).unwrap();
    assert_eq!(back.phrases[0].name.0, "ä".repeat(15));
}

#[test]
fn corrupt_data_is_an_error_not_a_panic() {
    let plain = sample().write();
    for cut in 0..plain.len() {
        let _ = Sng::read(&plain[..cut]);
    }
    let mut huge = plain.clone();
    huge[..4].copy_from_slice(&i32::MAX.to_le_bytes()); // absurd beat count
    assert!(Sng::read(&huge).is_err());
    let packed = sample().to_packed(Platform::Pc);
    for pos in (0..packed.len()).step_by(3) {
        let mut bad = packed.clone();
        bad[pos] ^= 0x5A;
        let _ = Sng::from_packed(&bad, Platform::Pc);
    }
}
