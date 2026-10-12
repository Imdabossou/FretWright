//! How well the onset detector alone finds the chart's note onsets, per song:
//! `onset_recall <work folder> [tolerance ms]`.
use fw_detect::onset;
use fw_detect::pipeline::{self, SR, load_mono};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let tol = a.get(2).and_then(|t| t.parse::<f64>().ok()).unwrap_or(50.0) / 1000.0;
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    println!("{:16} {:>6} {:>7} {:>7} {:>7} {:>9}", "song", "onsets", "found", "recall", "prec", "median ms");
    let (mut th, mut tn, mut tf) = (0usize, 0usize, 0usize);
    for d in dirs {
        let p = fw_project::Project::load(&d).expect("project");
        let stem = load_mono(&pipeline::stem_path(&d, "htdemucs_ft")).expect("stem");
        let mut truth: Vec<f64> = Vec::new();
        let mut legato: Vec<bool> = Vec::new();
        for n in &p.notes {
            if truth.last().is_some_and(|t| (n.start - t).abs() < 0.02) {
                continue;
            }
            truth.push(n.start);
            legato.push(n.hammer_on || n.pull_off);
        }
        let found: Vec<f64> = onset::bass_onset_candidates(&stem, SR, 0.04).into_iter().map(|c| c.0).collect();
        let mut used = vec![false; found.len()];
        let (mut hits, mut errs) = (0usize, Vec::new());
        let (mut miss_legato, mut miss_plain, mut all_legato) = (0usize, 0usize, 0usize);
        for (ti, &t) in truth.iter().enumerate() {
            all_legato += usize::from(legato[ti]);
            let best = found.iter().enumerate().filter(|(i, f)| !used[*i] && (**f - t).abs() <= tol).min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()));
            if let Some((i, f)) = best {
                used[i] = true;
                hits += 1;
                errs.push((f - t) * 1000.0);
            } else if legato[ti] {
                miss_legato += 1;
            } else {
                miss_plain += 1;
            }
        }
        errs.sort_by(|a, b| a.total_cmp(b));
        println!("{:16} {:6} {:7} {:6.1}% {:6.1}% {:9.0}", d.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), truth.len(), found.len(), 100.0 * hits as f64 / truth.len() as f64, 100.0 * hits as f64 / found.len().max(1) as f64, errs.get(errs.len() / 2).copied().unwrap_or(0.0));
        println!("{:16} chart onsets that are hammer-ons/pull-offs: {all_legato}; missed: {miss_legato} of those, and {miss_plain} plain notes", "");
        th += hits;
        tn += truth.len();
        tf += found.len();
    }
    println!("{:16} {:6} {:7} {:6.1}% {:6.1}%", "ALL", tn, tf, 100.0 * th as f64 / tn as f64, 100.0 * th as f64 / tf as f64);
}
