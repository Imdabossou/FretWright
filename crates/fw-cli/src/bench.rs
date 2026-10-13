//! `fretwright-cli bench`: runs the detector over a folder of official
//! `.psarc` charts and scores each against its own chart.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::{Value, json};

struct Song {
    name: String,
    reference: usize,
    detected: usize,
    matched: usize,
    recall: f64,
    precision: f64,
    f1: f64,
    seconds: f64,
    techniques: BTreeMap<&'static str, fw_eval::TechniqueScore>,
    misses: BTreeMap<fw_eval::Miss, usize>,
    extra: usize,
    lag_ms: f64,
    median_err_ms: f64,
}

fn song_name(psarc: &Path) -> String {
    let stem = psarc.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    stem.strip_suffix("_p").unwrap_or(&stem).to_string()
}

fn run_song(psarc: &Path, work: &Path, settings: &fw_project::DetectionSettings, tol: f64) -> Result<Song> {
    let t = Instant::now();
    let name = song_name(psarc);
    let folder = work.join(&name);
    let project = if folder.join(fw_project::PROJECT_FILE).is_file() {
        fw_project::Project::load(&folder)?
    } else {
        let p = fw_project::import::import_psarc(psarc, &folder)?;
        p.save()?;
        p
    };
    let reference = fw_eval::reference_from_notes(&project.notes);
    // a work folder copied from another machine keeps that machine's absolute audio path
    let audio = if Path::new(&project.audio_path).is_file() { PathBuf::from(&project.audio_path) } else { folder.join("song.wav") };
    let silent = |_: &str, _: f64| {};
    let det = fw_detect::pipeline::detect(&audio, settings, &project.open_strings(), &folder, &silent)?;
    let stem_file = fw_detect::pipeline::stem_path(&folder, &settings.separator);
    // detection quietly falls back to a low-pass filter when separation fails; a benchmark must not
    if settings.separator != "lowpass" && !stem_file.is_file() {
        anyhow::bail!("no bass stem was made: {}", det.warnings.join("; "));
    }
    std::fs::write(folder.join("detected.json"), serde_json::to_string(&det.notes)?)?;

    let stem = stem_file.is_file().then(|| fw_detect::pipeline::load_mono(&stem_file)).transpose()?;
    let offset = fw_eval::estimate_offset(&reference, &det.notes, 0.5, tol);
    let s = fw_eval::score(&reference, &det.notes, tol, offset, stem.as_deref().map(|x| (x, fw_detect::pipeline::SR)));
    Ok(Song {
        name,
        reference: s.reference,
        detected: s.detected,
        matched: s.matched,
        recall: s.recall(),
        precision: s.precision(),
        f1: s.f1(),
        seconds: t.elapsed().as_secs_f64(),
        misses: s.miss_counts(),
        extra: s.extra.len(),
        lag_ms: s.offset * 1000.0,
        median_err_ms: s.median_onset_error() * 1000.0,
        techniques: s.techniques,
    })
}

pub fn bench(songs: &Path, work: &Path, only: Option<&str>, tol: f64, settings: &fw_project::DetectionSettings) -> Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(songs)
        .with_context(|| format!("reading {}", songs.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("psarc")))
        .filter(|p| only.is_none_or(|o| song_name(p).contains(o)))
        .collect();
    files.sort();
    std::fs::create_dir_all(work)?;

    let results_file = work.join("results.json");
    let prev_file = work.join("results.prev.json");
    let previous: Option<Value> = std::fs::read_to_string(&results_file).ok().and_then(|t| serde_json::from_str(&t).ok());

    let mut done: Vec<Song> = Vec::new();
    for (i, f) in files.iter().enumerate() {
        eprint!("[{}/{}] {} ... ", i + 1, files.len(), song_name(f));
        match run_song(f, work, settings, tol) {
            Ok(s) => {
                eprintln!("F1 {:.1}% ({:.0} s)", s.f1 * 100.0, s.seconds);
                done.push(s);
            }
            Err(e) => eprintln!("failed: {e:#}"),
        }
    }
    if done.is_empty() {
        anyhow::bail!("no songs were scored");
    }

    let prev_f1 = |name: &str| previous.as_ref().and_then(|p| p["songs"][name]["f1"].as_f64());
    println!("{:18} {:>6} {:>6} {:>7} {:>9} {:>7} {:>8} {:>8} {:>8} {:>7} {:>7}", "song", "notes", "found", "recall", "precision", "F1", "change", "lag ms", "err ms", "silent", "merged");
    for s in &done {
        let delta = prev_f1(&s.name).map_or(String::new(), |p| format!("{:+.1}", (s.f1 - p) * 100.0));
        println!("{:18} {:6} {:6} {:6.1}% {:8.1}% {:6.1}% {:>8} {:8.0} {:8.0} {:7} {:7}", s.name, s.reference, s.detected, s.recall * 100.0, s.precision * 100.0, s.f1 * 100.0, delta, s.lag_ms, s.median_err_ms, s.misses.get(&fw_eval::Miss::Silent).copied().unwrap_or(0), s.misses.get(&fw_eval::Miss::Merged).copied().unwrap_or(0));
    }
    let (r, d, m): (usize, usize, usize) = done.iter().fold((0, 0, 0), |a, s| (a.0 + s.reference, a.1 + s.detected, a.2 + s.matched));
    let (rec, prec) = (m as f64 / r.max(1) as f64, m as f64 / d.max(1) as f64);
    let micro = if rec + prec > 0.0 { 2.0 * rec * prec / (rec + prec) } else { 0.0 };
    let macro_f1 = done.iter().map(|s| s.f1).sum::<f64>() / done.len() as f64;
    let prev_macro = previous.as_ref().and_then(|p| p["macro_f1"].as_f64());
    println!(
        "\nall songs: {r} chart notes, recall {:.1}%, precision {:.1}%, F1 {:.1}% (average per song {:.1}%{})",
        rec * 100.0,
        prec * 100.0,
        micro * 100.0,
        macro_f1 * 100.0,
        prev_macro.map_or(String::new(), |p| format!(", {:+.1} since the last run", (macro_f1 - p) * 100.0))
    );

    let mut causes: BTreeMap<fw_eval::Miss, usize> = BTreeMap::new();
    for s in &done {
        for (k, v) in &s.misses {
            *causes.entry(*k).or_insert(0) += v;
        }
    }
    println!("\nwhy chart notes were missed, over all songs ({} of {r}):", r - m);
    for (cause, n) in &causes {
        println!("  {n:6}  {}", cause.label());
    }
    println!("  {:6}  extra notes we made that the chart does not have (of {d})", done.iter().map(|s| s.extra).sum::<usize>());

    let mut tech: BTreeMap<&str, fw_eval::TechniqueScore> = BTreeMap::new();
    for s in &done {
        for (k, v) in &s.techniques {
            let e = tech.entry(k).or_default();
            e.hit += v.hit;
            e.missed += v.missed;
            e.false_alarm += v.false_alarm;
        }
    }
    if !tech.is_empty() {
        println!("\ntechniques over all songs (notes found with the right pitch):");
        println!("  {:16} {:>6} {:>6} {:>6}   {:>6} {:>9}", "", "chart", "found", "false", "recall", "precision");
        for (name, t) in &tech {
            let pct = |a: usize, b: usize| if b == 0 { "   -".to_string() } else { format!("{:4.0}%", a as f64 / b as f64 * 100.0) };
            println!("  {name:16} {:6} {:6} {:6}   {:>6} {:>9}", t.chart(), t.hit, t.false_alarm, pct(t.hit, t.chart()), pct(t.hit, t.hit + t.false_alarm));
        }
    }

    let json = json!({
        "macro_f1": macro_f1, "micro_f1": micro, "recall": rec, "precision": prec,
        "songs": done.iter().map(|s| (s.name.clone(), json!({"f1": s.f1, "recall": s.recall, "precision": s.precision, "notes": s.reference}))).collect::<serde_json::Map<_, _>>(),
    });
    if results_file.is_file() {
        let _ = std::fs::rename(&results_file, &prev_file);
    }
    std::fs::write(&results_file, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}
