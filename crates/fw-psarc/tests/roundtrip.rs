#![allow(clippy::unwrap_used)]
use std::io::Cursor;

use fw_psarc::{Error, Psarc, write};

fn sample_entries() -> Vec<(String, Vec<u8>)> {
    let text: Vec<u8> = b"manifest-ish json { \"a\": 1 } ".repeat(9000); // compressible, > 1 block
    let mut noise = vec![0u8; 200_000]; // incompressible, stored raw in full blocks
    let mut x = 0x1234_5678u32;
    for b in &mut noise {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *b = x as u8;
    }
    vec![
        ("songs/arr/test_showlights.xml".into(), text),
        ("audio/windows/123.wem".into(), noise.clone()), // stored plain by name
        ("gfxassets/noise.dds".into(), noise),
        ("appid.appid".into(), b"248750".to_vec()),
        ("empty.txt".into(), Vec::new()),
        ("exact.bin".into(), vec![7u8; 65536]),
    ]
}

fn roundtrip(encrypt: bool) {
    let entries = sample_entries();
    let mut buf = Cursor::new(Vec::new());
    write(&mut buf, &entries, encrypt).unwrap();
    let mut p = Psarc::from_reader(Cursor::new(buf.into_inner())).unwrap();
    assert_eq!(p.names(), entries.iter().map(|e| e.0.clone()).collect::<Vec<_>>());
    for (name, data) in &entries {
        assert_eq!(&p.read(name).unwrap(), data, "{name}");
    }
}

#[test]
fn roundtrip_encrypted() {
    roundtrip(true);
}

#[test]
fn roundtrip_plain_toc() {
    roundtrip(false);
}

#[test]
fn output_is_deterministic() {
    let entries = sample_entries();
    let mut a = Cursor::new(Vec::new());
    let mut b = Cursor::new(Vec::new());
    write(&mut a, &entries, true).unwrap();
    write(&mut b, &entries, true).unwrap();
    assert_eq!(a.into_inner(), b.into_inner());
}

#[test]
fn corrupt_inputs_are_errors_not_panics() {
    let mut buf = Cursor::new(Vec::new());
    write(&mut buf, &sample_entries(), true).unwrap();
    let good = buf.into_inner();

    assert!(Psarc::from_reader(Cursor::new(b"nope".to_vec())).is_err());
    let mut bad = good.clone();
    bad[0] = b'X';
    assert!(matches!(Psarc::from_reader(Cursor::new(bad)), Err(Error::BadMagic)));

    // truncations and single-byte corruptions anywhere in the header/TOC
    for cut in [10, 40, 200, good.len() / 2] {
        let _ = Psarc::from_reader(Cursor::new(good[..cut].to_vec())).map(|mut p| {
            for i in 0..p.names().len() {
                let _ = p.read_index(i);
            }
        });
    }
    for pos in 0..400.min(good.len()) {
        let mut bad = good.clone();
        bad[pos] ^= 0xA5;
        if let Ok(mut p) = Psarc::from_reader(Cursor::new(bad)) {
            for i in 0..p.names().len() {
                let _ = p.read_index(i);
            }
        }
    }
}

#[test]
fn extract_refuses_escaping_names() {
    let mut buf = Cursor::new(Vec::new());
    write(&mut buf, &[("../evil.txt".into(), b"x".to_vec())], true).unwrap();
    let mut p = Psarc::from_reader(Cursor::new(buf.into_inner())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(p.extract_all(dir.path()), Err(Error::UnsafeName(_))));
}
