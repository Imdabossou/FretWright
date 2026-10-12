//! One row per onset candidate (features, and whether a chart note starts there)
//! for training a candidate classifier: `onset_features <work folder> <out.csv>`.
use std::io::Write;

use fw_detect::onset::bass_onset_candidates;
use fw_detect::onset_model::{FEATURE_NAMES, Measurer};
use fw_detect::pipeline::{self, SR, load_mono};
use fw_detect::pitch::Analyzer;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&a[2]).expect("csv"));
    writeln!(out, "song,time,label,{}", FEATURE_NAMES.join(",")).expect("write");
    for d in dirs {
        let name = d.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let p = fw_project::Project::load(&d).expect("project");
        let stem = load_mono(&pipeline::stem_path(&d, "htdemucs_ft")).expect("stem");
        let open = p.open_strings();
        let analyzer = Analyzer::new(&stem, SR, open[0] - 1, open[3] + 25);
        let measurer = Measurer::new(&stem, SR, &analyzer);
        let cands = bass_onset_candidates(&stem, SR, 0.03);
        let mut truth: Vec<f64> = p.notes.iter().map(|n| n.start).collect();
        truth.dedup_by(|a, b| (*a - *b).abs() < 0.02);
        // strongest candidates claim the nearest free chart onset within 50 ms
        let mut order: Vec<usize> = (0..cands.len()).collect();
        order.sort_by(|&i, &j| cands[j].1.total_cmp(&cands[i].1));
        let mut taken = vec![false; truth.len()];
        let mut label = vec![0u8; cands.len()];
        for i in order {
            let best = truth.iter().enumerate().filter(|(k, t)| !taken[*k] && (**t - cands[i].0).abs() <= 0.05).min_by(|x, y| (x.1 - cands[i].0).abs().total_cmp(&(y.1 - cands[i].0).abs()));
            if let Some((k, _)) = best {
                taken[k] = true;
                label[i] = 1;
            }
        }
        for (i, c) in cands.iter().enumerate() {
            let gap_prev = if i > 0 { c.0 - cands[i - 1].0 } else { 5.0 };
            let gap_next = cands.get(i + 1).map_or(5.0, |n| n.0 - c.0);
            let f = measurer.features(c.0, c.1, gap_prev, gap_next);
            writeln!(out, "{name},{:.4},{},{}", c.0, label[i], f.iter().map(|v| format!("{v:.4}")).collect::<Vec<_>>().join(",")).expect("write");
        }
        eprintln!("{name}: {} candidates, {} real", cands.len(), label.iter().map(|&l| l as usize).sum::<usize>());
    }
}
