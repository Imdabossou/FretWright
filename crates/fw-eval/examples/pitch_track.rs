//! Prints the raw pitch track (fractional MIDI) of a window: `pitch_track <stem.wav> <from> <to>`.
use fw_detect::pipeline::{HOP, SR, load_mono};
use fw_detect::{notes, pyin};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bass = load_mono(std::path::Path::new(&a[1])).expect("stem");
    let (from, to): (f64, f64) = (a[2].parse().expect("from"), a[3].parse().expect("to"));
    let p = pyin::pyin(&bass, SR, 27.5, 400.0, 4096, HOP);
    let ft = HOP as f64 / SR;
    for (i, (f, c)) in p.f0.iter().zip(&p.voiced_prob).enumerate() {
        let t = i as f64 * ft;
        if t >= from && t <= to {
            println!("{t:.3}  {:7.3}  conf {c:.2}", if f.is_nan() { f64::NAN } else { notes::hz_to_midi(*f) });
        }
    }
}
