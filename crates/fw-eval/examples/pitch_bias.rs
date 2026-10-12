//! How far the pitch tracker reads from the chart's pitch, per chart note and per
//! pitch: `pitch_bias <stem.wav> <chart.xml>`.
use std::collections::BTreeMap;

use fw_detect::pipeline::{HOP, SR, load_mono};
use fw_detect::{notes, pyin};

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bass = load_mono(std::path::Path::new(&a[1])).expect("stem");
    let arr = fw_xml::InstrumentalArrangement::load(&a[2]).expect("chart");
    let p = pyin::pyin(&bass, SR, 27.5, 400.0, 4096, HOP);
    let ft = HOP as f64 / SR;
    let mut by_pitch: BTreeMap<i32, Vec<f64>> = BTreeMap::new();
    let mut all = Vec::new();
    for n in fw_eval::reference_notes(&arr) {
        let (a, b) = ((n.start + 0.12) / ft, (n.start + 0.30).min(n.end.max(n.start + 0.2)) / ft);
        let mut dev: Vec<f64> = (a as usize..(b as usize).min(p.f0.len()))
            .filter(|&i| !p.f0[i].is_nan() && p.voiced_prob[i] > 0.3)
            .map(|i| notes::hz_to_midi(p.f0[i]) - n.midi as f64)
            .collect();
        if dev.len() >= 3 {
            let m = median(&mut dev);
            by_pitch.entry(n.midi).or_default().push(m);
            all.push(m);
        }
    }
    println!("{} chart notes with a pitch reading; median deviation {:+.0} cents", all.len(), median(&mut all.clone()) * 100.0);
    for (midi, mut v) in by_pitch {
        if v.len() >= 8 {
            let q = |f: f64, v: &Vec<f64>| v[((v.len() - 1) as f64 * f) as usize] * 100.0;
            let m = median(&mut v);
            println!("MIDI {midi:3}: {:4} notes, median {:+4.0} cents  (25%..75%: {:+.0}..{:+.0})", v.len(), m * 100.0, q(0.25, &v), q(0.75, &v));
        }
    }
}
