//! Audio file -> beat grid + bass notes.

use std::path::{Path, PathBuf};

use fw_project::{BeatGrid, DetectionSettings, MAX_FRET, Note, Project};

use crate::{beats, dsp, fretting, notes, onset, onset_net, pitch_net, pyin, timbre};

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
        let onsets = find_onsets(&bass, &settings.onsets, cache_dir, &mut warnings);
        let roll = if settings.pitch == "net" { find_pitch_roll(&bass, cache_dir, &mut warnings) } else { None };
        let (found, tuning) = notes::segment_by_salience(
            &bass,
            SR,
            &onsets,
            open[0] - 1,
            open[3] + MAX_FRET + 1,
            &notes::SalienceSegmentation { min_note_s: settings.min_note_ms / 1000.0, ..Default::default() },
            roll.as_ref(),
        );
        if tuning.abs() > 0.15 {
            warnings.push(format!("the recording is {:+.0} cents from A=440; notes were corrected for it", tuning * 100.0));
        }
        found
    };
    notes::snap_to_grid(&mut found, &grid, settings.snap_division, 0.04);
    notes::fit_to_range(&mut found, open);

    progress("Choosing strings and frets", 0.95);
    let sound = string_opinions(&bass, &found, open, cache_dir, &mut warnings);
    fretting::assign_with(&mut found, open, &Default::default(), sound.as_deref());
    progress("Done", 1.0);
    Ok(Detection { notes: found, grid, duration, warnings })
}

/// The string model shipped with the program.
const DEFAULT_STRING_MODEL: &[u8] = include_bytes!("../models/string_net.bin");

/// For each note, how likely each string is judging by its sound (`None` for notes too
/// short to judge). A project folder's own `string_net.bin` takes precedence.
fn string_opinions(bass: &[f64], found: &[Note], open: &[i32; 4], cache_dir: &Path, warnings: &mut Vec<String>) -> Option<Vec<Option<[f32; 4]>>> {
    use rayon::prelude::*;
    let custom = std::fs::read(cache_dir.join("string_net.bin")).ok();
    let net = match timbre::StringNet::from_bytes(custom.as_deref().unwrap_or(DEFAULT_STRING_MODEL)) {
        Ok(n) => n,
        Err(e) => {
            warnings.push(format!("string model unavailable ({e}); strings were chosen by hand position alone"));
            return None;
        }
    };
    let analyzer = timbre::TimbreAnalyzer::default();
    Some(
        found
            .par_iter()
            .map(|n| {
                let allowed: [bool; 4] = std::array::from_fn(|s| (0..=MAX_FRET).contains(&(n.midi - open[s])));
                if allowed.iter().filter(|a| **a).count() < 2 {
                    return None;
                }
                let end = n.end.min(n.start + 0.8) - 0.01;
                analyzer.features(bass, SR, n.start + 0.02, end, n.midi).map(|f| net.log_probs(&f, allowed))
            })
            .collect(),
    )
}

/// Which notes sound when, from the pitch networks (a project folder's `pitch_net*.bin`
/// files, or the ones shipped with the program), averaged. `None` if they cannot be loaded.
fn find_pitch_roll(bass: &[f64], cache_dir: &Path, warnings: &mut Vec<String>) -> Option<pitch_net::PitchRoll> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(cache_dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("pitch_net") && n.ends_with(".bin"))).collect())
        .unwrap_or_default();
    files.sort();
    let nets: Result<Vec<pitch_net::PitchNet>, String> = if files.is_empty() {
        DEFAULT_PITCH_MODELS.iter().map(|b| pitch_net::PitchNet::from_bytes(b)).collect()
    } else {
        files.iter().map(|f| pitch_net::PitchNet::from_bytes(&std::fs::read(f).map_err(|e| e.to_string())?)).collect()
    };
    match nets {
        Ok(nets) if !nets.is_empty() => {
            let mut rolls = nets.iter().map(|n| n.probabilities(bass));
            let mut first = rolls.next()?;
            let mut count = 1.0f32;
            for r in rolls {
                first.add(&r);
                count += 1.0;
            }
            first.scale(1.0 / count);
            Some(first)
        }
        Ok(_) => None,
        Err(e) => {
            warnings.push(format!("pitch network unavailable ({e}); used the harmonic search"));
            None
        }
    }
}

/// The network trained on official charts, shipped with the program.
const DEFAULT_ONSET_MODELS: &[&[u8]] = &[
    include_bytes!("../models/onset_net_0.bin"),
    include_bytes!("../models/onset_net_1.bin"),
    include_bytes!("../models/onset_net_2.bin"),
];
const DEFAULT_PITCH_MODELS: &[&[u8]] = &[
    include_bytes!("../models/pitch_net_0.bin"),
    include_bytes!("../models/pitch_net_1.bin"),
    include_bytes!("../models/pitch_net_2.bin"),
];
/// A frame whose note-start probability is at least this starts a note.
const ONSET_THRESHOLD: f32 = 0.65;
/// How much the spectral flux counts next to the network (the network alone is 1.0).
const FLUX_WEIGHT: f64 = 0.5;
/// Network peaks below this probability are not even considered.
const NET_LOW: f64 = 0.25;

/// Note starts of a bass stem: from the onset network (or, with `method` "flux", from
/// spectral-flux peaks). A project folder may hold its own `onset_net.bin`.
fn find_onsets(bass: &[f64], method: &str, cache_dir: &Path, warnings: &mut Vec<String>) -> Vec<f64> {
    if method != "flux" {
        match load_onset_nets(cache_dir) {
            Ok(nets) => {
                let prob = average_probabilities(&nets, bass);
                return fuse_onsets(bass, &prob, ONSET_THRESHOLD as f64, FLUX_WEIGHT, NET_LOW);
            }
            Err(e) => warnings.push(format!("onset network unavailable ({e}); used spectral flux")),
        }
    }
    onset::bass_onsets(bass, SR)
}

/// The onset networks to use: the `onset_net*.bin` files in the project folder if there
/// are any, otherwise the ones shipped with the program.
fn load_onset_nets(cache_dir: &Path) -> Result<Vec<onset_net::OnsetNet>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(cache_dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("onset_net") && n.ends_with(".bin"))).collect())
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        return DEFAULT_ONSET_MODELS.iter().map(|b| onset_net::OnsetNet::from_bytes(b)).collect();
    }
    files.iter().map(|f| onset_net::OnsetNet::from_bytes(&std::fs::read(f).map_err(|e| e.to_string())?)).collect()
}

/// The mean of the networks' note-start probabilities.
fn average_probabilities(nets: &[onset_net::OnsetNet], bass: &[f64]) -> Vec<f32> {
    let mut sum: Vec<f32> = Vec::new();
    for net in nets {
        let p = net.probabilities(bass);
        if sum.is_empty() {
            sum = p;
        } else {
            sum.iter_mut().zip(&p).for_each(|(a, b)| *a += b);
        }
    }
    sum.iter_mut().for_each(|v| *v /= nets.len().max(1) as f32);
    sum
}

/// Candidates from both detectors, each scored by the network's probability plus
/// `flux_weight` times how clearly the spectral flux rises there.
fn fuse_onsets(bass: &[f64], prob: &[f32], threshold: f64, flux_weight: f64, net_low: f64) -> Vec<f64> {
    let frame_t = 128.0 / SR;
    let flux = onset::bass_onset_candidates(bass, SR, 0.03);
    let net_peaks = onset_net::pick_onsets(prob, net_low as f32);
    let mut cands: Vec<f64> = net_peaks.clone();
    cands.extend(flux.iter().map(|c| c.0));
    cands.sort_by(|a, b| a.total_cmp(b));
    let near = |t: f64| -> (f64, f64) {
        let (a, b) = (((t - 0.025) / frame_t).max(0.0) as usize, (((t + 0.025) / frame_t) as usize + 1).min(prob.len()));
        let p = prob[a.min(b)..b].iter().copied().fold(0.0f32, f32::max) as f64;
        let m = flux.iter().filter(|c| (c.0 - t).abs() <= 0.025).map(|c| c.1).fold(0.0, f64::max);
        (p, m)
    };
    let mut scored: Vec<(f64, f64)> = cands
        .iter()
        .map(|&t| {
            let (p, m) = near(t);
            (t, p + flux_weight * (m / 0.3).min(1.0))
        })
        .filter(|c| c.1 >= threshold)
        .collect();
    // keep the best of any candidates closer than 60 ms
    let mut out: Vec<(f64, f64)> = Vec::new();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    for c in scored {
        match out.last_mut() {
            Some(last) if c.0 - last.0 < 0.06 => {
                if c.1 > last.1 {
                    *last = c;
                }
            }
            _ => out.push(c),
        }
    }
    out.into_iter().map(|c| c.0).collect()
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
