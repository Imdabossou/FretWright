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
    if tracker == "none" {
        return BeatGrid::default();
    }
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

/// The beat grid of `mix`, reused from `cache_dir` when it was tracked before
/// (beat tracking is slow and does not change between runs on the same song).
fn cached_beats(mix: &[f64], tracker: &str, duration: f64, cache_dir: &Path, warnings: &mut Vec<String>, progress: Progress) -> BeatGrid {
    if tracker == "none" {
        return BeatGrid::default();
    }
    let path = cache_dir.join(format!("beats-{tracker}.cache"));
    let key = format!("{} {}", mix.len(), tracker);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let mut lines = text.lines();
        if lines.next() == Some(key.as_str()) {
            let tempo: f64 = lines.next().and_then(|l| l.parse().ok()).unwrap_or(120.0);
            let beats: Vec<f64> = lines.next().unwrap_or("").split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let downbeats: Vec<usize> = lines.next().unwrap_or("").split_whitespace().filter_map(|v| v.parse().ok()).collect();
            if !beats.is_empty() {
                return BeatGrid { beats, downbeats, tempo };
            }
        }
    }
    let grid = track_beats(mix, tracker, duration, warnings, progress);
    let join = |v: Vec<String>| v.join(" ");
    let text = format!(
        "{key}\n{}\n{}\n{}\n",
        grid.tempo,
        join(grid.beats.iter().map(|b| format!("{b:.5}")).collect()),
        join(grid.downbeats.iter().map(|d| d.to_string()).collect())
    );
    let _ = std::fs::write(&path, text);
    grid
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
    let grid = cached_beats(&mix, &settings.beats, duration, cache_dir, &mut warnings, progress);

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

    let mut found = if settings.pitch == "pyin" {
        progress("Tracking pitch (pYIN)", 0.40);
        let fmin = midi_to_hz((open[0] - 2) as f64);
        let fmax = midi_to_hz((open[3] + MAX_FRET + 2) as f64);
        let p = cached_pyin(&bass, fmin, fmax, &pitch_cache_path(cache_dir, &settings.separator));

        progress("Segmenting notes", 0.85);
        let rms = dsp::rms(&bass, 2048, HOP);
        let frame_t = HOP as f64 / SR;
        let onsets: Vec<usize> = onset::bass_onsets(&bass, SR).iter().map(|t| (t / frame_t).round() as usize).collect();
        let tuning = notes::tuning_offset(&p.f0, &p.voiced_prob);
        if tuning.abs() > 0.15 {
            warnings.push(format!("the recording is {:+.0} cents from A=440; notes were corrected for it", tuning * 100.0));
        }
        let mut found = notes::segment_at_onsets(
            &p.f0,
            &rms,
            &onsets,
            &notes::OnsetSegmentation { frame_t, tuning, min_note_ms: settings.min_note_ms, attack_ms: 100.0, near_ms: 100.0 },
        );
        notes::fix_octave_blips(&mut found, 0.15);
        found
    } else {
        progress("Finding notes", 0.40);
        let strong = lab("DELTA", 0.11);
        let weak = lab("WEAK", 0.04);
        let candidates = onset::bass_onset_candidates(&bass, SR, weak.min(strong));
        let cfg = notes::SalienceSegmentation {
            strong_margin: strong,
            weak_margin: weak,
            change_window_s: lab("CHANGEWIN", 0.12),
            change_confidence: lab("CHANGECONF", 2.0),
            grid_tol_s: lab("GRIDTOL", 0.03),
            pitch_change: lab("PITCHCHG", 0.0) > 0.0,
            accent_ratio: lab("ACCENT", 0.0),
            min_note_s: settings.min_note_ms / 1000.0,
            attack_s: lab("ATTACK", 0.0),
            tail_s: lab("TAIL", 0.01),
            min_confidence: lab("MINCONF", 2.0),
            gate_db: lab("GATE", -40.0),
        };
        let subs = notes::subdivisions(&grid, 4);
        let (found, tuning) = notes::segment_by_salience(&bass, SR, &candidates, &subs, open[0] - 1, open[3] + MAX_FRET + 1, &cfg);
        if tuning.abs() > 0.15 {
            warnings.push(format!("the recording is {:+.0} cents from A=440; notes were corrected for it", tuning * 100.0));
        }
        found
    };
    notes::snap_to_grid(&mut found, &grid, settings.snap_division, 0.04);
    notes::fit_to_range(&mut found, open);

    progress("Choosing strings and frets", 0.95);
    fretting::assign(&mut found, open, &Default::default());
    progress("Done", 1.0);
    Ok(Detection { notes: found, grid, duration, warnings })
}

/// Experiment knob: `FW_<name>` in the environment overrides `default`.
fn lab(name: &str, default: f64) -> f64 {
    std::env::var(format!("FW_{name}")).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Where the pitch track of a stem is cached next to the stem.
pub fn pitch_cache_path(cache_dir: &Path, separator: &str) -> PathBuf {
    cache_dir.join("stems").join(separator).join("pyin.cache")
}

/// A cheap fingerprint of the signal and the tracker's settings.
fn pyin_key(y: &[f64], fmin: f64, fmax: f64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ y.len() as u64;
    for v in y.iter().step_by(509) {
        h = (h ^ v.to_bits()).wrapping_mul(0x0100_0000_01b3);
    }
    (h ^ fmin.to_bits()).wrapping_mul(0x0100_0000_01b3) ^ fmax.to_bits()
}

/// pYIN of `y`, reused from `cache` when the same signal was tracked before
/// (the pitch track is the slow part of detection and does not depend on the
/// settings that are usually changed).
pub fn cached_pyin(y: &[f64], fmin: f64, fmax: f64, cache: &Path) -> pyin::PyinOutput {
    let key = pyin_key(y, fmin, fmax);
    if let Ok(bytes) = std::fs::read(cache)
        && bytes.len() >= 16
        && bytes[..8] == key.to_le_bytes()
    {
        let n = u64::from_le_bytes(bytes[8..16].try_into().unwrap_or([0; 8])) as usize;
        if bytes.len() == 16 + n * 17 {
            let f64_at = |i: usize| f64::from_le_bytes(bytes[16 + i * 8..24 + i * 8].try_into().unwrap_or([0; 8]));
            return pyin::PyinOutput {
                f0: (0..n).map(f64_at).collect(),
                voiced_prob: (n..2 * n).map(f64_at).collect(),
                voiced: bytes[16 + 16 * n..].iter().map(|&b| b != 0).collect(),
            };
        }
    }
    let p = pyin::pyin(y, SR, fmin, fmax, 4096, HOP);
    let mut out = Vec::with_capacity(16 + p.f0.len() * 17);
    out.extend_from_slice(&key.to_le_bytes());
    out.extend_from_slice(&(p.f0.len() as u64).to_le_bytes());
    p.f0.iter().chain(&p.voiced_prob).for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    out.extend(p.voiced.iter().map(|&b| u8::from(b)));
    let _ = std::fs::write(cache, out);
    p
}

/// Detects and stores the result in the project.
pub fn detect_project(p: &mut Project, progress: Progress) -> Result<Detection, Error> {
    let det = detect(Path::new(&p.audio_path), &p.detection, &p.open_strings(), &p.folder.clone(), progress)?;
    p.notes = det.notes.clone();
    p.grid = det.grid.clone();
    p.duration = det.duration;
    Ok(det)
}
