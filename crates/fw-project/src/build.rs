//! Project -> .psarc, entirely in Rust.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use fw_package::{ArrangementName, ArrangementSpec, PackageSpec, Platform, Priority, Sortable};

use crate::{Error, Project, Result};

/// Progress callback: (message, fraction 0..1).
pub type BuildProgress<'a> = &'a dyn Fn(&str, f64);

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// True if `target` exists and is newer than `source`.
fn fresh(target: &Path, source: &Path) -> bool {
    matches!((modified(target), modified(source)), (Some(t), Some(s)) if t >= s)
}

/// Default preview: a third of the way into the song, on a bar line.
fn default_preview_start(p: &Project) -> f64 {
    let target = p.duration * 0.33;
    let bars: Vec<f64> = p.grid.downbeats.iter().filter_map(|&i| p.grid.beats.get(i).copied()).collect();
    let t = bars.iter().copied().min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs())).unwrap_or(target);
    t.min(p.duration - 30.0).max(0.0)
}

/// Builds the package spec, encoding (and caching) the song and preview audio.
pub fn package_spec(p: &mut Project, progress: BuildProgress) -> Result<PackageSpec> {
    if p.notes.is_empty() {
        return Err(Error::NoNotes);
    }
    let key = p.ensure_dlc_key();
    let work = p.path("build");
    std::fs::create_dir_all(&work)?;
    let audio_path = PathBuf::from(&p.audio_path);

    progress("Decoding audio", 0.05);
    let rev = fw_audio::REVISION;
    let song_wem = work.join(format!("song.r{rev}.wem"));
    let need_audio = !fresh(&song_wem, &audio_path) || p.volume.is_none() || p.duration <= 0.0;
    let audio = if need_audio { Some(fw_audio::decode(&audio_path)?.to_rocksmith()?) } else { None };
    if let Some(a) = &audio {
        p.duration = a.duration();
        if p.volume.is_none() {
            progress("Measuring loudness", 0.15);
            p.volume = Some(fw_audio::volume(a) as f64);
        }
        if !fresh(&song_wem, &audio_path) {
            progress("Encoding song audio (Wwise Vorbis)", 0.25);
            std::fs::write(&song_wem, fw_audio::encode_wem(a)?)?;
        }
    }

    let start = match p.preview_start {
        Some(s) => s,
        None => {
            let s = default_preview_start(p);
            p.preview_start = Some(s);
            s
        }
    };
    let preview_wem = work.join(format!("preview_{start:.2}.r{rev}.wem"));
    if !fresh(&preview_wem, &audio_path) {
        progress("Encoding preview", 0.6);
        let a = match audio {
            Some(a) => a,
            None => fw_audio::decode(&audio_path)?.to_rocksmith()?,
        };
        std::fs::write(&preview_wem, fw_audio::encode_wem(&a.preview(start))?)?;
    }

    let md = &p.metadata;
    let mut tuning = [0i16; 6];
    for (t, o) in tuning.iter_mut().zip(p.tuning) {
        *t = o as i16;
    }
    let art = PathBuf::from(&p.album_art);
    let volume = p.volume.unwrap_or(-7.0) as f32;
    Ok(PackageSpec {
        dlc_key: key,
        version: if md.version.is_empty() { "1".into() } else { md.version.clone() },
        author: if md.author.is_empty() { "FretWright".into() } else { md.author.clone() },
        artist: Sortable::new(if md.artist.is_empty() { "Unknown" } else { &md.artist }, Some(&md.artist_sort)),
        title: Sortable::new(if md.title.is_empty() { "Untitled" } else { &md.title }, Some(&md.title_sort)),
        album: Sortable::new(&md.album, Some(&md.album_sort)),
        year: md.year,
        album_art: art.is_file().then_some(art),
        audio_wem: std::fs::read(&song_wem)?,
        preview_wem: std::fs::read(&preview_wem)?,
        volume,
        preview_volume: volume,
        arrangements: vec![ArrangementSpec {
            xml: crate::to_arrangement(p),
            name: ArrangementName::Bass,
            priority: Priority::Main,
            bass_picked: p.bass_picked,
            scroll_speed: 1.3,
            tuning,
            tuning_pitch: p.tuning_pitch,
            persistent_id: p.identity.persistent_id.clone(),
            master_id: p.identity.master_id,
        }],
        builder_name: format!("FretWright {}", env!("CARGO_PKG_VERSION")),
        app_id: 248_750,
    })
}

/// Builds the project's package(s) into `out_dir` (default `<project>/out`).
pub fn build(p: &mut Project, out_dir: Option<&Path>, platforms: &[Platform], progress: BuildProgress) -> Result<Vec<PathBuf>> {
    let spec = package_spec(p, progress)?;
    let out = out_dir.map(Path::to_path_buf).unwrap_or_else(|| p.path("out"));
    std::fs::create_dir_all(&out)?;
    let mut paths = Vec::new();
    for &platform in platforms {
        progress("Building package (phrases, difficulty levels, SNG, PSARC)", 0.85);
        paths.push(fw_package::build(&spec, platform, &out)?);
    }
    progress("Package ready", 1.0);
    Ok(paths)
}
