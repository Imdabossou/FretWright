//! Audio file -> beat grid + bass notes.

use std::path::{Path, PathBuf};

use fw_project::{BeatGrid, DetectionSettings, MAX_FRET, Note, Project};

use crate::{beats, dsp, fretting, notes, onset, pyin};

pub const SR: f64 = 22050.0;
pub const HOP: usize = 256;

/// Progress callback: (message, fraction 0..1).
pub type Progress<'a> = &'a (dyn Fn(&str, f64) + Sync);

#[derive(Debug, Clone)]
pub struct Detection {
    pub notes: Vec<Note>,
    pub grid: BeatGrid,
    pub duration: f64,
    /// Things the user should know (e.g. a model that is not available yet).
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Audio(#[from] fw_audio::Error),
    #[error("{0}")]
    Model(#[from] fw_models::Error),
}

fn midi_to_hz(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

/// Mono audio at 22.05 kHz (channels averaged first, like librosa.load).
pub fn load_mono(path: &Path) -> Result<Vec<f64>, Error> {
    let a = fw_audio::decode(path)?;
    let mono = fw_audio::Audio { sample_rate: a.sample_rate, channels: vec![a.mono()] };
    let r = mono.resample(SR as u32)?;
    Ok(r.channels[0].iter().map(|&s| s as f64).collect())
}

/// Beat grid of the full mix with the chosen tracker. "beat_this" (the neural
/// tracker, which also finds bar lines) falls back to the classic tracker
/// with a warning when its model file is missing.
pub fn track_beats(mix: &[f64], tracker: &str, duration: f64, warnings: &mut Vec<String>, progress: Progress) -> BeatGrid {
    if tracker != "librosa" {
        if tracker != "beat_this" {
            warnings.push(format!("unknown beat tracker '{tracker}'; used Beat This!"));
        }
        match fw_models::beat_this::BeatThis::load_default() {
            Ok(model) => {
                let y: Vec<f32> = mix.iter().map(|&v| v as f32).collect();
                match model.track(&y, &mut |f| progress("Tracking beats", 0.10 + 0.12 * f)) {
                    Ok(b) => return beats::regularize(&b.beats, &b.downbeats, duration, 4),
                    Err(e) => warnings.push(format!("Beat This! failed ({e}); used the classic tracker")),
                }
            }
            Err(e) => warnings.push(format!("{e}; used the classic beat tracker")),
        }
    }
    let (_, beat_times) = beats::track_librosa(mix);
    beats::regularize(&beat_times, &[], duration, 4)
}

/// Runs a Demucs model on the song and caches its bass stem in the project
/// (16-bit stereo WAV at 44.1 kHz, see [`stem_path`]).
pub fn separate(audio: &Path, model: &str, device: &str, cache_dir: &Path, warnings: &mut Vec<String>, progress: Progress) -> Result<(), Error> {
    use fw_models::demucs;
    let a = fw_audio::decode(audio)?;
    let ch = |i: usize| fw_audio::resample::resample(&a.channels[i.min(a.channels.len() - 1)], a.sample_rate, demucs::SAMPLE_RATE);
    let (left, right) = (ch(0), ch(1));
    let device = match device {
        "auto" => demucs::Device::Auto,
        "cpu" => demucs::Device::Cpu,
        _ => demucs::Device::Gpu,
    };
    progress(&format!("Separating the bass with {model}"), 0.25);
    let report = |f: f64| progress(&format!("Separating the bass with {model}"), 0.25 + 0.15 * f);
    let sep = demucs::separate(model, &["bass"], device, &left, &right, &report)?;
    if sep.device == "CPU" && device != demucs::Device::Cpu {
        warnings.push("no usable GPU found; Demucs ran on the CPU (slow)".into());
    }
    let (l, r) = sep.stem("bass").ok_or_else(|| fw_models::Error::Run("no bass stem".into()))?;
    let stem = fw_audio::Audio { sample_rate: demucs::SAMPLE_RATE, channels: vec![l.to_vec(), r.to_vec()] };
    let path = stem_path(cache_dir, model);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fw_audio::Error::Io)?;
    }
    std::fs::write(&path, stem.to_wav()).map_err(fw_audio::Error::Io)?;
    Ok(())
}

/// Where a project caches separated stems: `stems/<model>/bass.wav`.
pub fn stem_path(cache_dir: &Path, model: &str) -> PathBuf {
    cache_dir.join("stems").join(model).join("bass.wav")
}

pub fn detect(audio: &Path, settings: &DetectionSettings, open: &[i32; 4], cache_dir: &Path, progress: Progress) -> Result<Detection, Error> {
    let mut warnings = Vec::new();
    progress("Loading audio", 0.02);
    let mix = load_mono(audio)?;
    let duration = mix.len() as f64 / SR;

    progress("Tracking beats", 0.10);
    let grid = track_beats(&mix, &settings.beats, duration, &mut warnings, progress);

    progress("Isolating bass", 0.25);
    let bass = if settings.separator == "lowpass" {
        dsp::sosfiltfilt(&dsp::butter_lowpass_sos(4, 300.0, SR), &mix)
    } else {
        let stem = stem_path(cache_dir, &settings.separator);
        if !stem.is_file()
            && let Err(e) = separate(audio, &settings.separator, &settings.device, cache_dir, &mut warnings, progress)
        {
            warnings.push(format!("separation with {} failed ({e}); used the low-pass filter", settings.separator));
        }
        if stem.is_file() { load_mono(&stem)? } else { dsp::sosfiltfilt(&dsp::butter_lowpass_sos(4, 300.0, SR), &mix) }
    };

    progress("Tracking pitch (pYIN)", 0.40);
    if settings.pitch != "pyin" {
        warnings.push(format!("pitch tracker '{}' needs the neural models (next phase); used pYIN", settings.pitch));
    }
    let fmin = midi_to_hz((open[0] - 2) as f64);
    let fmax = midi_to_hz((open[3] + MAX_FRET + 2) as f64);
    let p = pyin::pyin(&bass, SR, fmin, fmax, 4096, HOP);

    progress("Segmenting notes", 0.85);
    let rms = dsp::rms(&bass, 2048, HOP);
    let onsets = onset::onset_detect(&bass, SR, HOP);
    let mut found = notes::segment(&p.f0, &p.voiced_prob, &rms, &onsets, HOP as f64 / SR, settings.min_note_ms);
    notes::fix_octave_blips(&mut found, 0.15);
    let mut found = notes::clean_blips(found, 0.12, 0.06);
    notes::snap_to_grid(&mut found, &grid, settings.snap_division, 0.04);
    notes::fit_to_range(&mut found, open);

    progress("Choosing strings and frets", 0.95);
    fretting::assign(&mut found, open, &Default::default());
    progress("Done", 1.0);
    Ok(Detection { notes: found, grid, duration, warnings })
}

/// Detects and stores the result in the project.
pub fn detect_project(p: &mut Project, progress: Progress) -> Result<Detection, Error> {
    let det = detect(Path::new(&p.audio_path), &p.detection, &p.open_strings(), &p.folder.clone(), progress)?;
    p.notes = det.notes.clone();
    p.grid = det.grid.clone();
    p.duration = det.duration;
    Ok(det)
}
