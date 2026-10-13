//! How often does the string/fret assignment agree with the charters when the pitches are right?
//! `fret_lab <work folder>`.
use std::collections::HashSet;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    let (mut ok_all, mut n_all) = (0usize, 0usize);
    for d in dirs {
        let p = fw_project::Project::load(&d).expect("project");
        // one note per time (chords skipped), as the detector produces them
        let mut notes: Vec<fw_project::Note> = Vec::new();
        for n in &p.notes {
            if notes.last().is_none_or(|l| (n.start - l.start).abs() > 0.015) {
                notes.push(n.clone());
            }
        }
        let truth: Vec<(i32, i32)> = notes.iter().map(|n| (n.string, n.fret)).collect();
        for n in notes.iter_mut() {
            n.string = 0;
            n.fret = 0;
        }
        fw_detect::fretting::assign(&mut notes, &p.open_strings(), &HashSet::new());
        let ok = notes.iter().zip(&truth).filter(|(n, t)| (n.string, n.fret) == **t).count();
        println!("{:14} {:5.1}% of {}", d.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), 100.0 * ok as f64 / notes.len() as f64, notes.len());
        ok_all += ok;
        n_all += notes.len();
    }
    println!("ALL            {:5.1}% of {n_all}", 100.0 * ok_all as f64 / n_all as f64);
}
