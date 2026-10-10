//! Playback of the song, the bass stem and a synthesised version of the chart.
//!
//! Sources are decoded once in the background at the device's sample rate.
//! When playback starts, the chosen sources are mixed (and time-stretched for
//! slowed-down practice) into one buffer, also off the UI thread, which the
//! cpal callback then streams.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use fw_project::Project;

use crate::audio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Mix,
    MixSynth,
    Stem,
    StemSynth,
    Synth,
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Mix, Mode::MixSynth, Mode::Stem, Mode::StemSynth, Mode::Synth];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Mix => "Full mix",
            Mode::MixSynth => "Full mix + chart synth",
            Mode::Stem => "Bass stem",
            Mode::StemSynth => "Bass stem + chart synth",
            Mode::Synth => "Chart synth only",
        }
    }

    fn mix(self) -> bool {
        matches!(self, Mode::Mix | Mode::MixSynth)
    }

    fn stem(self) -> bool {
        matches!(self, Mode::Stem | Mode::StemSynth)
    }

    fn synth(self) -> bool {
        matches!(self, Mode::MixSynth | Mode::StemSynth | Mode::Synth)
    }
}

/// What the audio callback reads.
struct Shared {
    buf: Arc<Vec<f32>>,
    pos: usize,
    playing: bool,
    volume: f32,
}

/// Decoded sources at the device rate.
#[derive(Default, Clone)]
struct Sources {
    mix: Option<Arc<Vec<f32>>>,
    stem: Option<Arc<Vec<f32>>>,
}

pub struct Loaded {
    sources: Sources,
    /// Waveform peaks of the stem (or the mix) and their rate.
    pub peaks: Option<(Vec<f32>, f64)>,
}

pub struct Player {
    _stream: Option<cpal::Stream>,
    shared: Arc<Mutex<Shared>>,
    rate: u32,
    pub error: Option<String>,
    sources: Sources,
    loading: Option<Receiver<Loaded>>,
    rendering: Option<Receiver<Arc<Vec<f32>>>>,
    /// play from here once the current load/render finishes
    pending_play: Option<f64>,
    mode: Mode,
    speed: f64,
    /// speed the current buffer was rendered at
    buf_speed: f64,
    dirty: bool,
    duration: f64,
}

pub const PEAK_RATE: f64 = 200.0;

impl Player {
    pub fn new() -> Player {
        let shared = Arc::new(Mutex::new(Shared { buf: Arc::new(Vec::new()), pos: 0, playing: false, volume: 1.0 }));
        let (stream, rate, error) = match open_stream(shared.clone()) {
            Ok((s, r)) => (Some(s), r, None),
            Err(e) => (None, 48_000, Some(e)),
        };
        Player {
            _stream: stream,
            shared,
            rate,
            error,
            sources: Sources::default(),
            loading: None,
            rendering: None,
            pending_play: None,
            mode: Mode::MixSynth,
            speed: 1.0,
            buf_speed: 1.0,
            dirty: true,
            duration: 0.0,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts decoding a project's audio and stem in the background.
    pub fn set_sources(&mut self, audio_path: Option<PathBuf>, stem: Option<PathBuf>) {
        self.stop();
        self.sources = Sources::default();
        self.dirty = true;
        let rate = self.rate;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let load = |p: &PathBuf| -> Option<Arc<Vec<f32>>> {
                let a = fw_audio::decode(p).ok()?;
                Some(Arc::new(fw_audio::resample::resample(&a.mono(), a.sample_rate, rate)))
            };
            let mix = audio_path.as_ref().and_then(load);
            let stem = stem.as_ref().and_then(load);
            let peaks = stem.as_ref().or(mix.as_ref()).map(|x| (audio::peaks(x, rate, PEAK_RATE), PEAK_RATE));
            let _ = tx.send(Loaded { sources: Sources { mix, stem }, peaks });
        });
        self.loading = Some(rx);
    }

    /// Call every frame: picks up finished background work. Returns new
    /// waveform peaks when sources finished loading.
    pub fn poll(&mut self, project: Option<&Project>) -> Option<Option<(Vec<f32>, f64)>> {
        let mut peaks = None;
        if let Some(rx) = &self.loading
            && let Ok(l) = rx.try_recv()
        {
            self.sources = l.sources;
            self.loading = None;
            self.dirty = true;
            peaks = Some(l.peaks);
        }
        if let Some(rx) = &self.rendering
            && let Ok(buf) = rx.try_recv()
        {
            self.rendering = None;
            let mut s = self.lock();
            s.buf = buf;
            drop(s);
            self.buf_speed = self.speed;
            self.dirty = false;
        }
        if let Some(at) = self.pending_play
            && self.loading.is_none()
            && self.rendering.is_none()
        {
            if self.dirty {
                if let Some(p) = project {
                    self.render(p);
                }
            } else {
                self.pending_play = None;
                self.start(at);
            }
        }
        peaks
    }

    fn render(&mut self, p: &Project) {
        let (sources, mode, speed, rate) = (self.sources.clone(), self.mode, self.speed, self.rate);
        let length = sources.mix.as_ref().map_or(0, |m| m.len()).max((p.duration.max(1.0) * rate as f64) as usize);
        self.duration = length as f64 / rate as f64;
        let synth = mode.synth().then(|| audio::synth(p, self.duration, rate));
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut out = vec![0f32; length];
            let mut add = |x: &[f32], gain: f32| {
                for (o, v) in out.iter_mut().zip(x) {
                    *o += v * gain;
                }
            };
            if mode.mix()
                && let Some(m) = &sources.mix
            {
                add(m, 0.8);
            }
            if mode.stem()
                && let Some(s) = &sources.stem
            {
                add(s, 1.0);
            }
            if let Some(s) = &synth {
                add(s, 1.0);
            }
            let _ = tx.send(Arc::new(audio::stretch(&out, speed, rate)));
        });
        self.rendering = Some(rx);
    }

    fn start(&mut self, at: f64) {
        let pos = (at.max(0.0) / self.buf_speed * self.rate as f64) as usize;
        let mut s = self.lock();
        s.pos = pos.min(s.buf.len());
        s.playing = true;
    }

    /// True while waiting for audio to load or render before playing.
    pub fn preparing(&self) -> bool {
        self.pending_play.is_some()
    }

    pub fn play(&mut self, at: f64) {
        if self.error.is_some() {
            return;
        }
        self.pending_play = Some(at);
    }

    pub fn pause(&mut self) {
        self.pending_play = None;
        self.lock().playing = false;
    }

    pub fn stop(&mut self) {
        self.pause();
    }

    pub fn is_playing(&self) -> bool {
        self.lock().playing
    }

    /// Song time of the playhead.
    pub fn position(&self) -> f64 {
        let s = self.lock();
        s.pos as f64 / self.rate as f64 * self.buf_speed
    }

    pub fn seek(&mut self, t: f64) {
        let pos = (t.max(0.0) / self.buf_speed * self.rate as f64) as usize;
        let mut s = self.lock();
        s.pos = pos.min(s.buf.len());
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            self.mode = mode;
            self.invalidate();
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_speed(&mut self, speed: f64) {
        if (speed - self.speed).abs() > 1e-6 {
            self.speed = speed;
            self.invalidate();
        }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn set_volume(&mut self, v: f32) {
        self.lock().volume = v;
    }

    /// The chart or settings changed: re-render before playing next time
    /// (restarting playback where it is if it is running).
    pub fn invalidate(&mut self) {
        self.dirty = true;
        if self.is_playing() {
            let at = self.position();
            self.lock().playing = false;
            self.pending_play = Some(at);
        }
    }

    /// The chart changed (only matters when the synth is audible).
    pub fn chart_changed(&mut self) {
        if self.mode.synth() {
            self.invalidate();
        }
    }

    pub fn device_rate(&self) -> u32 {
        self.rate
    }
}

fn open_stream(shared: Arc<Mutex<Shared>>) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = config.sample_rate();
    let channels = config.channels() as usize;
    let format = config.sample_format();
    let cfg: cpal::StreamConfig = config.into();
    let err = |e: cpal::Error| eprintln!("audio output: {e}");
    macro_rules! build {
        ($t:ty) => {
            device.build_output_stream(
                cfg,
                move |data: &mut [$t], _: &cpal::OutputCallbackInfo| {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    let (vol, playing) = (s.volume, s.playing);
                    for frame in data.chunks_mut(channels) {
                        let v = if playing { s.buf.get(s.pos).copied() } else { None };
                        let sample = match v {
                            Some(v) => {
                                s.pos += 1;
                                (v * vol).clamp(-1.0, 1.0)
                            }
                            None => 0.0,
                        };
                        for out in frame.iter_mut() {
                            *out = <$t as cpal::FromSample<f32>>::from_sample_(sample);
                        }
                    }
                    if playing && s.pos >= s.buf.len() {
                        s.playing = false;
                    }
                },
                err,
                None,
            )
        };
    }
    let stream = match format {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::U16 => build!(u16),
        other => return Err(format!("unsupported audio sample format {other}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}
