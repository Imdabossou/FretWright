//! Prints the notes in a time window after each detection stage:
//! `stages <stem.wav> <from> <to>` (bass tuned Drop C#: open strings 25,30,35,40).
use fw_detect::pipeline::{HOP, SR, load_mono};
use fw_detect::{dsp, notes, onset, pyin};

fn show(label: &str, ns: &[fw_project::Note], from: f64, to: f64) {
    let s: Vec<String> = ns.iter().filter(|n| n.start >= from && n.start <= to).map(|n| format!("{:.3}-{:.3}:{}", n.start, n.end, n.midi)).collect();
    println!("{label:12} {}", s.join("  "));
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bass = load_mono(std::path::Path::new(&a[1])).expect("stem");
    let (from, to): (f64, f64) = (a[2].parse().expect("from"), a[3].parse().expect("to"));
    let open = [25, 30, 35, 40];
    let (fmin, fmax) = (440.0 * 2f64.powf((open[0] as f64 - 2.0 - 69.0) / 12.0), 440.0 * 2f64.powf((open[3] as f64 + 24.0 + 2.0 - 69.0) / 12.0));
    let p = pyin::pyin(&bass, SR, fmin, fmax, 4096, HOP);
    let rms = dsp::rms(&bass, 2048, HOP);
    let ft = HOP as f64 / SR;
    let onsets: Vec<usize> = onset::bass_onsets(&bass, SR).iter().map(|t| (t / ft).round() as usize).collect();
    println!("onsets       {}", onsets.iter().copied().filter(|&o| (o as f64 * ft) >= from && (o as f64 * ft) <= to).map(|o| format!("{:.3}", o as f64 * ft)).collect::<Vec<_>>().join(" "));
    let seg = notes::segment_at_onsets(
        &p.f0,
        &rms,
        &onsets,
        &notes::OnsetSegmentation { frame_t: ft, tuning: notes::tuning_offset(&p.f0, &p.voiced_prob), min_note_ms: 60.0, attack_ms: 100.0, near_ms: 100.0 },
    );
    show("segmented", &seg, from, to);
    let mut a2 = seg.clone();
    notes::fix_octave_blips(&mut a2, 0.15);
    show("octave-fix", &a2, from, to);
    let b = notes::clean_blips(a2, 0.12, 0.06);
    show("clean_blips", &b, from, to);
}
