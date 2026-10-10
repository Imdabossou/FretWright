#![allow(clippy::unwrap_used)]
//! Every sample arrangement in tests/data parses and survives a save/load
//! round trip, and broken input is an error, never a panic.

use std::path::Path;

use fw_xml::InstrumentalArrangement;

fn data_files() -> Vec<std::path::PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut v: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

#[test]
fn every_sample_parses_and_round_trips() {
    for f in data_files() {
        let arr = InstrumentalArrangement::load(&f).unwrap();
        for abridged in [true, false] {
            let again = InstrumentalArrangement::parse(&arr.to_xml(abridged)).unwrap();
            assert_eq!(again, arr, "{} (abridged={abridged})", f.display());
        }
    }
}

#[test]
fn malformed_input_is_an_error_not_a_panic() {
    let text = std::fs::read_to_string(data_files()[0].clone()).unwrap();
    for cut in (0..text.len()).step_by(997) {
        if text.is_char_boundary(cut) {
            let _ = InstrumentalArrangement::parse(&text[..cut]);
        }
    }
    assert!(InstrumentalArrangement::parse("<vocals count=\"0\"/>").is_err());
    assert!(InstrumentalArrangement::parse("<song version=\"4\"></song>").is_err());
    assert!(InstrumentalArrangement::parse("<song version=\"8\"><levels><level><notes><note time=\"x\"/></notes></level></levels></song>").is_err());
}
