//! Do the chart's slides show up as a pitch trail inside the note?
//! `slide_lab <work folder>`.
use fw_detect::pipeline::{self, SR, load_mono};
use fw_detect::pitch::Analyzer;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    // (kind, start midi, change = last window - first window, windows)
    let mut rows: Vec<(&str, i32, i32, i32)> = Vec::new();
    for d in &dirs {
        let p = fw_project::Project::load(d).expect("project");
        let stem = load_mono(&pipeline::stem_path(d, "htdemucs_ft")).expect("stem");
        let open = p.open_strings();
        let analyzer = Analyzer::new(&stem, SR, open[0] - 1, open[3] + 25);
        let starts: Vec<f64> = p.notes.iter().map(|n| n.start).collect();
        for (i, n) in p.notes.iter().enumerate() {
            let next = starts.iter().copied().find(|&t| t > n.start + 0.02).unwrap_or(n.start + 1.0);
            let end = n.end.min(next);
            if end - n.start < 0.2 {
                continue;
            }
            let kind = if n.slide_to >= 0 { "slide" } else if n.slide_unpitch_to >= 0 { "slide-out" } else if n.technique_names().is_empty() { "plain" } else { continue };
            // pitch of the first and the last 90 ms of the note
            let first = analyzer.estimate(n.start + 0.01, n.start + 0.10, 0.0);
            let last = analyzer.estimate(end - 0.10, end - 0.01, 0.0);
            let (Some(f), Some(l)) = (first, last) else { continue };
            let want = if n.slide_to >= 0 { n.slide_to - n.fret } else { 0 };
            let _ = i;
            rows.push((kind, want, l.midi - f.midi, 0));
        }
    }
    for kind in ["plain", "slide", "slide-out"] {
        let r: Vec<_> = rows.iter().filter(|r| r.0 == kind).collect();
        let big = |t: i32| r.iter().filter(|x| x.2.abs() >= t).count() as f64 / r.len().max(1) as f64 * 100.0;
        println!("{kind:10} {:5} notes (>= 0.2 s): pitch moved by >=1 semitone {:4.1}%, >=2: {:4.1}%, >=4: {:4.1}%", r.len(), big(1), big(2), big(4));
    }
    let slides: Vec<_> = rows.iter().filter(|r| r.0 == "slide").collect();
    let right = slides.iter().filter(|r| r.1 != 0 && (r.2 - r.1).abs() <= 1).count();
    println!("slides: detected change within 1 semitone of the charted slide: {right} of {}", slides.len());
}
