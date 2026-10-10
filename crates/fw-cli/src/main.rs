//! `fretwright-cli`: the FretWright command line.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "fretwright-cli", version, about = "Turn songs into Rocksmith 2014 arrangements")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Work with PSARC archives
    #[command(subcommand)]
    Psarc(PsarcCmd),
    /// Inspect SNG charts
    #[command(subcommand)]
    Sng(SngCmd),
    /// One step from a song to a .psarc: makes a project next to the song (or
    /// reuses it), detects the bass line and builds the package
    Convert {
        song: PathBuf,
        /// Defaults come from the file name ("Artist - Title.mp3")
        #[arg(long)]
        artist: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        album: Option<String>,
        #[arg(long)]
        year: Option<i32>,
        /// Album art image (a cover with the artist and title is made otherwise)
        #[arg(long)]
        art: Option<PathBuf>,
        /// E Standard, Drop D, Eb Standard, Drop C#, D Standard, Drop C, C# Standard, C Standard
        #[arg(long, default_value = "E Standard")]
        tuning: String,
        /// htdemucs_ft (default), htdemucs, htdemucs_6s or lowpass
        #[arg(long)]
        separator: Option<String>,
        /// Project folder (default: "<song name> (FretWright)" next to the song)
        #[arg(long)]
        project: Option<PathBuf>,
        /// Output folder for the .psarc (default: <project>/out)
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long, value_delimiter = ',', default_value = "pc")]
        platform: Vec<String>,
    },
    /// Detect the bass line of a project's song (replaces its notes and beat grid)
    Detect {
        project: PathBuf,
        /// htdemucs_ft, htdemucs, htdemucs_6s or lowpass
        #[arg(long)]
        separator: Option<String>,
        /// Bar subdivisions notes snap to (0 = off)
        #[arg(long)]
        snap: Option<u32>,
    },
    /// Detect the bass line of an audio file with explicit settings and print
    /// the result as JSON (notes, grid, duration, warnings), for scripting
    DetectAudio {
        audio: PathBuf,
        /// Folder holding the stem cache (stems/<model>/bass.wav), usually the project folder
        #[arg(long)]
        cache_dir: PathBuf,
        #[arg(long, default_value = "htdemucs_ft")]
        separator: String,
        #[arg(long, default_value = "pyin")]
        pitch: String,
        #[arg(long, default_value = "beat_this")]
        beats: String,
        /// auto, cpu or gpu
        #[arg(long, default_value = "auto")]
        device: String,
        #[arg(long, default_value_t = 60.0)]
        min_note_ms: f64,
        #[arg(long, default_value_t = 4)]
        snap: u32,
        /// MIDI notes of the open strings, low to high
        #[arg(long, value_delimiter = ',', default_value = "28,33,38,43")]
        open: Vec<i32>,
    },
    /// List the neural network model files and where they were found;
    /// `models download` fetches the missing ones (verified by SHA-256)
    Models {
        /// "download" to fetch missing models
        action: Option<String>,
        /// Print JSON ({"<file>": "<path>" or null}) instead of a table
        #[arg(long)]
        json: bool,
        /// With download: also the optional models, not just the defaults
        #[arg(long)]
        all: bool,
    },
    /// Build a FretWright project into .psarc package(s)
    Build {
        /// Project folder (or its project file)
        project: PathBuf,
        /// Output folder (default: <project>/out)
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Platforms to build for
        #[arg(long, value_delimiter = ',', default_value = "pc")]
        platform: Vec<String>,
        /// Album art to use when the project has none (not saved to the project)
        #[arg(long)]
        art: Option<PathBuf>,
    },
    /// Build .psarc package(s) from a JSON build spec (arrangement XMLs, audio, metadata)
    Package {
        spec: PathBuf,
    },
    /// Audio tools
    #[command(subcommand)]
    Audio(AudioCmd),
}

#[derive(Subcommand)]
enum AudioCmd {
    /// Rocksmith volume (dB) of an audio file
    Volume { audio: PathBuf },
    /// Encode an audio file to a Rocksmith .wem (resampled to 48 kHz stereo)
    Wem { audio: PathBuf, output: PathBuf },
    /// Write the 28 s song-select preview as a WAV
    Preview {
        audio: PathBuf,
        output: PathBuf,
        /// Start time in seconds
        start: f64,
    },
}

#[derive(Subcommand)]
enum SngCmd {
    /// Summarise the SNG charts in a package (or a single packed .sng file)
    Info {
        file: PathBuf,
        /// Platform of a bare .sng file (packages are detected from their name)
        #[arg(long, default_value = "pc")]
        platform: String,
    },
}

#[derive(Subcommand)]
enum PsarcCmd {
    /// List the entries of an archive
    List { psarc: PathBuf },
    /// Extract every entry into a folder
    Extract { psarc: PathBuf, out_dir: PathBuf },
    /// Pack a folder into an archive (entry names are paths relative to the folder)
    Pack {
        dir: PathBuf,
        psarc: PathBuf,
        /// Leave the table of contents unencrypted (Rocksmith expects it encrypted)
        #[arg(long)]
        no_encrypt: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Psarc(cmd) => psarc(cmd),
        Cmd::Sng(cmd) => sng(cmd),
        Cmd::Package { spec } => package(&spec),
        Cmd::Build { project, out_dir, platform, art } => build(&project, out_dir.as_deref(), &platform, art),
        Cmd::Detect { project, separator, snap } => detect(&project, separator, snap),
        Cmd::Convert { song, artist, title, album, year, art, tuning, separator, project, out_dir, platform } => {
            let folder = project.unwrap_or_else(|| fw_project::default_folder(&song));
            if !folder.join(fw_project::PROJECT_FILE).is_file() {
                let song = std::path::absolute(&song)?;
                let decoded = fw_audio::decode(&song).with_context(|| format!("reading {}", song.display()))?;
                let (g_artist, g_title) = fw_project::guess_metadata(&song);
                let mut p = fw_project::Project::new(&folder, song.display().to_string());
                p.metadata.artist = artist.unwrap_or(g_artist);
                p.metadata.title = title.unwrap_or(g_title);
                p.metadata.album = album.unwrap_or_default();
                if let Some(y) = year {
                    p.metadata.year = y;
                }
                if let Some(a) = art {
                    p.album_art = std::path::absolute(a)?.display().to_string();
                }
                p.tuning = fw_project::TUNINGS
                    .iter()
                    .find(|t| t.0.eq_ignore_ascii_case(&tuning))
                    .map(|t| t.1)
                    .with_context(|| format!("unknown tuning '{tuning}'"))?;
                p.duration = decoded.duration();
                p.save()?;
                eprintln!("created project {}", folder.display());
            } else {
                eprintln!("using existing project {}", folder.display());
            }
            detect(&folder, separator, None)?;
            build(&folder, out_dir.as_deref(), &platform, None)
        }
        Cmd::Models { action, json, all } => match action.as_deref() {
            None => models(json),
            Some("download") => download_models(all),
            Some(other) => anyhow::bail!("unknown models action '{other}' (try: download)"),
        },
        Cmd::DetectAudio { audio, cache_dir, separator, pitch, beats, device, min_note_ms, snap, open } => {
            let settings = fw_project::DetectionSettings { separator, pitch, beats, min_note_ms, snap_division: snap, device };
            detect_audio(&audio, &settings, &open, &cache_dir)
        }
        Cmd::Audio(cmd) => audio(cmd),
    }
}

fn detect(project: &std::path::Path, separator: Option<String>, snap: Option<u32>) -> Result<()> {
    let t = Instant::now();
    let mut p = fw_project::Project::load(project).with_context(|| format!("opening {}", project.display()))?;
    if let Some(s) = separator {
        p.detection.separator = s;
    }
    if let Some(s) = snap {
        p.detection.snap_division = s;
    }
    let progress = |msg: &str, f: f64| eprintln!("[{:5.1}%] {msg}", f * 100.0);
    let det = fw_detect::pipeline::detect_project(&mut p, &progress)?;
    for w in &det.warnings {
        eprintln!("note: {w}");
    }
    p.save()?;
    println!("{} notes, {:.1} BPM, {:.0} s of audio", p.notes.len(), p.grid.tempo, p.duration);
    eprintln!("done in {:.1?}", t.elapsed());
    Ok(())
}

fn models(json: bool) -> Result<()> {
    let mut files = vec![fw_models::beat_this::DEFAULT_MODEL.to_string()];
    files.extend(fw_models::demucs::models().map(fw_models::demucs::model_file));
    let found: Vec<(String, Option<PathBuf>)> = files.into_iter().map(|f| (f.clone(), fw_models::find_model(&f).ok())).collect();
    if json {
        let map: serde_json::Map<String, serde_json::Value> =
            found.iter().map(|(f, p)| (f.clone(), p.as_ref().map(|p| p.display().to_string()).into())).collect();
        println!("{}", serde_json::Value::Object(map));
    } else {
        for (f, p) in &found {
            println!("{f:32} {}", p.as_ref().map_or("missing".to_string(), |p| p.display().to_string()));
        }
        println!("\nsearched: {}", fw_models::model_dirs().iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", "));
    }
    Ok(())
}

fn download_models(all: bool) -> Result<()> {
    use fw_models::download;
    let todo: Vec<_> = download::ASSETS.iter().filter(|a| (all || a.default) && fw_models::find_model(a.file).is_err()).collect();
    if todo.is_empty() {
        println!("all models are installed");
        return Ok(());
    }
    let total: u64 = todo.iter().map(|a| a.size).sum();
    eprintln!("downloading {} file(s), {:.0} MB, into {}", todo.len(), total as f64 / 1e6, download::install_dir()?.display());
    for a in todo {
        let mut last = 0u64;
        let path = download::download(a, &mut |done, size| {
            if done - last >= 8 << 20 || done == size {
                last = done;
                eprint!("\r[{:5.1}%] {}", done as f64 / size as f64 * 100.0, a.what);
            }
        })
        .inspect_err(|_| eprintln!())?;
        eprintln!();
        println!("{}", path.display());
    }
    Ok(())
}

fn detect_audio(audio: &std::path::Path, settings: &fw_project::DetectionSettings, open: &[i32], cache_dir: &std::path::Path) -> Result<()> {
    let open: [i32; 4] = open.try_into().map_err(|_| anyhow::anyhow!("--open needs 4 MIDI notes"))?;
    let t = Instant::now();
    let progress = |msg: &str, f: f64| eprintln!("[{:5.1}%] {msg}", f * 100.0);
    let det = fw_detect::pipeline::detect(audio, settings, &open, cache_dir, &progress)?;
    let out = serde_json::json!({
        "notes": det.notes,
        "grid": det.grid,
        "duration": det.duration,
        "warnings": det.warnings,
        "seconds": t.elapsed().as_secs_f64(),
    });
    println!("{out}");
    Ok(())
}

fn build(project: &std::path::Path, out_dir: Option<&std::path::Path>, platforms: &[String], art: Option<PathBuf>) -> Result<()> {
    let t = Instant::now();
    let mut p = fw_project::Project::load(project).with_context(|| format!("opening {}", project.display()))?;
    let own_art = p.album_art.clone();
    if let Some(a) = art.filter(|_| !std::path::Path::new(&own_art).is_file()) {
        p.album_art = a.to_string_lossy().into_owned();
    }
    let platforms: Vec<fw_package::Platform> = platforms
        .iter()
        .map(|s| if s.eq_ignore_ascii_case("mac") { fw_package::Platform::Mac } else { fw_package::Platform::Pc })
        .collect();
    let progress = |msg: &str, f: f64| eprintln!("[{:5.1}%] {msg}", f * 100.0);
    let paths = fw_project::build(&mut p, out_dir, &platforms, &progress)?;
    p.album_art = own_art;
    p.save()?;
    for path in paths {
        println!("{}", path.display());
    }
    eprintln!("done in {:.1?}", t.elapsed());
    Ok(())
}

fn audio(cmd: AudioCmd) -> Result<()> {
    match cmd {
        AudioCmd::Volume { audio } => {
            let a = fw_audio::decode(&audio)?.to_rocksmith()?;
            println!("{}", fw_audio::volume(&a));
        }
        AudioCmd::Wem { audio, output } => {
            let t = Instant::now();
            let a = fw_audio::decode(&audio)?.to_rocksmith()?;
            std::fs::write(&output, fw_audio::encode_wem(&a)?)?;
            eprintln!("encoded {:.1}s of audio in {:.1?}", a.duration(), t.elapsed());
        }
        AudioCmd::Preview { audio, output, start } => {
            let a = fw_audio::decode(&audio)?.to_rocksmith()?;
            std::fs::write(&output, a.preview(start).to_wav())?;
        }
    }
    Ok(())
}

/// Reads an audio file for a package: .wem files are used as they are, anything
/// else is decoded, resampled and encoded.
fn package_audio(path: &str) -> Result<Vec<u8>> {
    if path.to_lowercase().ends_with(".wem") {
        return std::fs::read(path).with_context(|| format!("reading {path}"));
    }
    let a = fw_audio::decode(path)?.to_rocksmith()?;
    Ok(fw_audio::encode_wem(&a)?)
}

/// `fretwright-cli package spec.json`: builds packages from a JSON spec naming the
/// arrangement XMLs, audio and metadata (the format of the former .NET
/// builder, handy for scripting); prints {"ok": true, "packages": [...]}.
fn package(spec_path: &std::path::Path) -> Result<()> {
    let run = || -> Result<Vec<String>> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(spec_path)?)?;
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let sortable = |k: &str| fw_package::Sortable::new(&s(k), Some(&s(&format!("{k}Sort"))));
        let mut arrangements = Vec::new();
        for a in v["arrangements"].as_array().context("spec has no arrangements")? {
            let mut tuning = [0i16; 6];
            for (i, t) in a["tuning"].as_array().into_iter().flatten().enumerate().take(6) {
                tuning[i] = t.as_i64().unwrap_or(0) as i16;
            }
            let name = match a["name"].as_str().unwrap_or("Bass").to_lowercase().as_str() {
                "lead" => fw_package::ArrangementName::Lead,
                "combo" => fw_package::ArrangementName::Combo,
                "rhythm" => fw_package::ArrangementName::Rhythm,
                _ => fw_package::ArrangementName::Bass,
            };
            let priority = match a["priority"].as_str().unwrap_or("Main").to_lowercase().as_str() {
                "alternative" => fw_package::Priority::Alternative,
                "bonus" => fw_package::Priority::Bonus,
                _ => fw_package::Priority::Main,
            };
            let xml_path = a["xml"].as_str().context("arrangement without xml")?;
            arrangements.push(fw_package::ArrangementSpec {
                xml: fw_xml::InstrumentalArrangement::load(xml_path).with_context(|| format!("loading {xml_path}"))?,
                name,
                priority,
                bass_picked: a["bassPicked"].as_bool().unwrap_or(false),
                scroll_speed: a["scrollSpeed"].as_f64().filter(|&x| x > 0.0).unwrap_or(1.3),
                tuning,
                tuning_pitch: a["tuningPitch"].as_f64().filter(|&x| x > 0.0).unwrap_or(440.0),
                persistent_id: a["persistentId"].as_str().context("arrangement without persistentId")?.to_string(),
                master_id: a["masterId"].as_i64().unwrap_or(0) as i32,
            });
        }
        let art = s("albumArt");
        let spec = fw_package::PackageSpec {
            dlc_key: fw_package::strings::dlc_key(&s("dlcKey")),
            version: if s("version").is_empty() { "1".into() } else { s("version") },
            author: if s("author").is_empty() { "FretWright".into() } else { s("author") },
            artist: sortable("artist"),
            title: sortable("title"),
            album: sortable("album"),
            year: v["year"].as_i64().unwrap_or(2026) as i32,
            album_art: (!art.is_empty() && std::path::Path::new(&art).is_file()).then(|| PathBuf::from(&art)),
            audio_wem: package_audio(v["audio"]["path"].as_str().context("spec has no audio")?)?,
            preview_wem: package_audio(v["preview"]["path"].as_str().context("spec has no preview")?)?,
            volume: v["audio"]["volume"].as_f64().unwrap_or(-7.0) as f32,
            preview_volume: v["preview"]["volume"].as_f64().unwrap_or(-7.0) as f32,
            arrangements,
            builder_name: format!("FretWright {}", env!("CARGO_PKG_VERSION")),
            app_id: 248_750,
        };
        let output = PathBuf::from(s("output"));
        let mut paths = Vec::new();
        for p in v["platforms"].as_array().cloned().unwrap_or_else(|| vec!["pc".into()]) {
            let platform = if p.as_str() == Some("mac") { fw_package::Platform::Mac } else { fw_package::Platform::Pc };
            // the platform suffix is appended to the output path
            let path = PathBuf::from(format!("{}{}.psarc", output.display(), platform.suffix()));
            let entries = fw_package::entries(&spec, platform)?;
            fw_psarc::write_file(&path, &entries, true)?;
            paths.push(path.display().to_string());
        }
        Ok(paths)
    };
    match run() {
        Ok(paths) => println!("{}", serde_json::json!({ "ok": true, "packages": paths })),
        Err(e) => {
            println!("{}", serde_json::json!({ "ok": false, "error": format!("{e:#}") }));
            std::process::exit(1);
        }
    }
    Ok(())
}

fn sng(cmd: SngCmd) -> Result<()> {
    let SngCmd::Info { file, platform } = cmd;
    let mut charts = Vec::new();
    if file.extension().is_some_and(|e| e == "psarc") {
        let mac = file.file_stem().is_some_and(|s| s.to_string_lossy().ends_with("_m"));
        let plat = if mac { fw_sng::Platform::Mac } else { fw_sng::Platform::Pc };
        let mut p = fw_psarc::Psarc::open(&file)?;
        for name in p.names().to_vec().into_iter().filter(|n| n.ends_with(".sng")) {
            charts.push((name.clone(), fw_sng::Sng::from_packed(&p.read(&name)?, plat)?));
        }
    } else {
        let plat = if platform == "mac" { fw_sng::Platform::Mac } else { fw_sng::Platform::Pc };
        charts.push((file.display().to_string(), fw_sng::Sng::from_packed(&std::fs::read(&file)?, plat)?));
    }
    for (name, s) in charts {
        let m = &s.metadata;
        println!("{name}");
        println!("  length {:.1}s, first note {:.2}s, tuning {:?}, capo {}", m.song_length,
                 m.first_note_time, m.tuning, if m.capo_fret_id < 0 { 0 } else { m.capo_fret_id });
        println!("  {} beats, {} phrases, {} sections, {} chords, {} vocals",
                 s.beats.len(), s.phrases.len(), s.sections.len(), s.chords.len(), s.vocals.len());
        let counts: Vec<String> = s.levels.iter().map(|l| l.notes.len().to_string()).collect();
        println!("  {} difficulty levels, notes per level: {}", s.levels.len(), counts.join(" "));
        let sections: Vec<&str> = s.sections.iter().map(|x| x.name.0.as_str()).collect();
        println!("  sections: {}", sections.join(", "));
    }
    Ok(())
}

fn psarc(cmd: PsarcCmd) -> Result<()> {
    match cmd {
        PsarcCmd::List { psarc } => {
            let p = fw_psarc::Psarc::open(&psarc).with_context(|| format!("opening {}", psarc.display()))?;
            for (name, e) in p.names().iter().zip(p.entries()) {
                println!("{:>10}  {name}", e.length);
            }
        }
        PsarcCmd::Extract { psarc, out_dir } => {
            let t = Instant::now();
            let mut p = fw_psarc::Psarc::open(&psarc).with_context(|| format!("opening {}", psarc.display()))?;
            let files = p.extract_all(&out_dir)?;
            eprintln!("extracted {} files in {:.0?}", files.len(), t.elapsed());
        }
        PsarcCmd::Pack { dir, psarc, no_encrypt } => {
            let t = Instant::now();
            let mut entries = Vec::new();
            for e in walkdir::WalkDir::new(&dir).sort_by_file_name() {
                let e = e?;
                if e.file_type().is_file() {
                    let name = e.path().strip_prefix(&dir)?.to_string_lossy().replace('\\', "/");
                    entries.push((name, std::fs::read(e.path())?));
                }
            }
            fw_psarc::write_file(&psarc, &entries, !no_encrypt)?;
            eprintln!("packed {} files in {:.0?}", entries.len(), t.elapsed());
        }
    }
    Ok(())
}
