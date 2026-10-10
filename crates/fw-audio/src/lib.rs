//! Audio for Rocksmith 2014 packages.
//!
//! * [`decode`] any common format (mp3, flac, ogg/vorbis, wav, m4a/aac, alac)
//! * [`Audio::to_rocksmith`] resample to 48 kHz stereo
//! * [`volume`] the Rocksmith volume from BS.1770 integrated loudness
//! * [`Audio::preview`] the 28 s faded song-select preview
//! * [`encode_wem`] Wwise 2013 Vorbis `.wem`, without Wwise (rabbit-smith/wem)
//!
//! The loudness meter implements ITU-R BS.1770-4. The volume formula and the
//! preview follow Rocksmith2014.NET's `Rocksmith2014.Audio` (MIT, Tapio
//! Malmberg).

mod loudness;
pub mod resample;

use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not decode the audio: {0}")]
    Decode(String),
    #[error("encoding the .wem failed: {0}")]
    Wem(String),
    #[error("the audio is empty")]
    Empty,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Bumped whenever decoding, resampling or encoding output changes, so
/// cached audio from an older version is re-encoded instead of reused.
/// 2: gapless MP3 trimming and the exactly aligned resampler.
pub const REVISION: u32 = 2;

/// Planar float audio (one Vec per channel), samples in -1.0..1.0.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }

    /// Mono mix (for analysis).
    pub fn mono(&self) -> Vec<f32> {
        let n = self.channels.len().max(1) as f32;
        (0..self.frames()).map(|i| self.channels.iter().map(|c| c[i]).sum::<f32>() / n).collect()
    }

    /// 48 kHz stereo, the format Rocksmith's audio is encoded in.
    pub fn to_rocksmith(&self) -> Result<Audio> {
        let stereo = match self.channels.len() {
            0 => return Err(Error::Empty),
            1 => vec![self.channels[0].clone(), self.channels[0].clone()],
            2 => self.channels.clone(),
            // keep front left/right of surround material
            _ => vec![self.channels[0].clone(), self.channels[1].clone()],
        };
        let audio = Audio { sample_rate: self.sample_rate, channels: stereo };
        audio.resample(48_000)
    }

    /// Band-limited resampling with exact timing (see [`resample`](mod@resample)).
    pub fn resample(&self, rate: u32) -> Result<Audio> {
        let channels = self.channels.iter().map(|c| resample::resample(c, self.sample_rate, rate)).collect();
        Ok(Audio { sample_rate: rate, channels })
    }

    /// Signed 16-bit samples (clipped). Exact inverse of [`Audio::from_i16`].
    pub fn to_i16(&self) -> Vec<Vec<i16>> {
        self.channels
            .iter()
            .map(|c| c.iter().map(|&s| (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16).collect())
            .collect()
    }

    /// Back from 16-bit, as audio tools read a 16-bit WAV.
    pub fn from_i16(sample_rate: u32, pcm: &[Vec<i16>]) -> Audio {
        Audio { sample_rate, channels: pcm.iter().map(|c| c.iter().map(|&s| s as f32 / 32768.0).collect()).collect() }
    }

    /// The song-select preview: 28 s from `start` seconds, faded in over
    /// 2.5 s and out over 3 s (shorter fades for very short audio).
    pub fn preview(&self, start: f64) -> Audio {
        const LENGTH_MS: i64 = 28_000;
        let total_ms = (self.duration() * 1000.0) as i64;
        let (fade_in, fade_out) = if total_ms < 5_500 { (total_ms / 2, total_ms / 2) } else { (2_500, 3_000) };
        let length_ms = total_ms.min(LENGTH_MS);
        let sr = self.sample_rate as i64;
        let from = ((start.max(0.0) * sr as f64) as usize).min(self.frames());
        let to = (from + (length_ms * sr / 1000) as usize).min(self.frames());
        let fade_in_n = (fade_in * sr / 1000) as usize;
        let fade_out_n = (fade_out * sr / 1000) as usize;
        let fade_out_start = ((length_ms - fade_out) * sr / 1000) as usize;
        let channels = self
            .channels
            .iter()
            .map(|c| {
                c[from..to]
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| {
                        let mut g = 1.0;
                        if i <= fade_in_n && fade_in_n > 0 {
                            g *= i as f32 / fade_in_n as f32;
                        }
                        if i >= fade_out_start && fade_out_n > 0 {
                            g *= (1.0 - (i - fade_out_start) as f32 / fade_out_n as f32).max(0.0);
                        }
                        s * g
                    })
                    .collect()
            })
            .collect();
        Audio { sample_rate: self.sample_rate, channels }
    }

    /// 16-bit PCM WAV bytes (for previews and debugging).
    pub fn to_wav(&self) -> Vec<u8> {
        let pcm = self.to_i16();
        let ch = pcm.len() as u16;
        let frames = self.frames();
        let data_len = frames * ch as usize * 2;
        let mut w = Vec::with_capacity(44 + data_len);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&ch.to_le_bytes());
        w.extend_from_slice(&self.sample_rate.to_le_bytes());
        w.extend_from_slice(&(self.sample_rate * ch as u32 * 2).to_le_bytes());
        w.extend_from_slice(&(ch * 2).to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data_len as u32).to_le_bytes());
        for i in 0..frames {
            for c in &pcm {
                w.extend_from_slice(&c[i].to_le_bytes());
            }
        }
        w
    }
}

/// Decodes an audio file (format detected from its contents and extension).
pub fn decode(path: impl AsRef<Path>) -> Result<Audio> {
    let path = path.as_ref();
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        // gapless: trim MP3/AAC encoder delay and padding (LAME/iTunes info), as
        // libsndfile/mpg123 (and so librosa) do; otherwise audio starts ~25 ms late
        .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
        .map_err(|e| Error::Decode(e.to_string()))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| Error::Decode("no audio track".into()))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| Error::Decode(e.to_string()))?;
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut channels: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(Error::Decode(e.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue, // skip a corrupt frame
            Err(e) => return Err(Error::Decode(e.to_string())),
        };
        let spec = *decoded.spec();
        sample_rate = spec.rate;
        let n = spec.channels.count();
        if channels.is_empty() {
            channels = vec![Vec::new(); n];
        }
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks_exact(n) {
            for (c, s) in channels.iter_mut().zip(frame) {
                c.push(*s);
            }
        }
    }
    if channels.is_empty() || channels[0].is_empty() {
        return Err(Error::Empty);
    }
    Ok(Audio { sample_rate, channels })
}

/// Rocksmith volume (dB) for the audio: -16 LUFS reference minus its
/// integrated loudness, rounded to 0.1 dB. Analyses the audio as it would be
/// stored (16-bit), like DLC Builder does.
pub fn volume(audio: &Audio) -> f32 {
    let pcm = audio.to_i16();
    let as_read = Audio::from_i16(audio.sample_rate, &pcm);
    let l = loudness::integrated(&as_read);
    if l.is_infinite() || l.is_nan() {
        0.0
    } else {
        // MidpointRounding.AwayFromZero, which Rust's round() does
        ((-16.0 - l) * 10.0).round() as f32 / 10.0
    }
}

/// Encodes 48 kHz stereo audio as a Rocksmith-compatible Wwise Vorbis `.wem`.
pub fn encode_wem(audio: &Audio) -> Result<Vec<u8>> {
    if audio.sample_rate != 48_000 || audio.channels.len() != 2 {
        return Err(Error::Wem("audio must be 48 kHz stereo; call to_rocksmith() first".into()));
    }
    let profile = wem_core::WwiseProfile::new(wem_core::WwiseVersion::Wwise2013, 2, 48_000)
        .map_err(|e| Error::Wem(e.to_string()))?;
    let encoder = wem_core::Encoder::new(profile).map_err(|e| Error::Wem(e.to_string()))?;
    let mut pcm = audio.to_i16();
    // the encoder needs a minimum number of frames
    let min = wem_core::MIN_PCM_FRAMES as usize;
    for c in &mut pcm {
        if c.len() < min {
            c.resize(min, 0);
        }
    }
    let pcm = wem_core::Pcm16::new(48_000, pcm).map_err(|e| Error::Wem(e.to_string()))?;
    Ok(encoder.encode_pcm(&pcm).map_err(|e| Error::Wem(e.to_string()))?.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(a: &Audio) -> usize {
        a.channels[0].iter().enumerate().fold(0, |b, (i, v)| if v.abs() > a.channels[0][b].abs() { i } else { b })
    }

    #[test]
    fn resampling_keeps_timing() {
        for (from, to) in [(44_100u32, 22_050u32), (44_100, 48_000), (48_000, 22_050), (22_050, 48_000), (44_056, 48_000)] {
            let mut x = vec![0.0f32; from as usize * 3];
            let at = from as usize; // impulse at 1.0 s
            x[at] = 1.0;
            let a = Audio { sample_rate: from, channels: vec![x] }.resample(to).unwrap();
            let expected = to as usize;
            let got = peak(&a);
            assert!(got.abs_diff(expected) <= 1, "{from}->{to}: impulse at {got}, expected {expected}");
            assert_eq!(a.frames(), to as usize * 3);
        }
    }

    #[test]
    fn resampling_is_exact_to_a_fraction_of_a_sample() {
        // a passband tone must come out as the same tone sampled at the new
        // times; a 1/100-sample timing error alone would leave ~-30 dB error
        for (from, to) in [(48_000u32, 22_050u32), (44_100, 48_000), (22_050, 48_000), (44_056, 48_000)] {
            let tone = |rate: u32, n: usize| (2.0 * std::f64::consts::PI * 3_000.0 * n as f64 / rate as f64 + 0.3).sin();
            let x: Vec<f32> = (0..from as usize).map(|n| tone(from, n) as f32).collect();
            let y = resample::resample(&x, from, to);
            let mid = to as usize / 4..3 * to as usize / 4;
            let err = mid.clone().map(|n| (y[n] as f64 - tone(to, n)).abs()).fold(0.0, f64::max);
            assert!(err < 1e-5, "{from}->{to}: max error {err}");
        }
    }

    #[test]
    fn resampling_rejects_aliases() {
        // 12 kHz cannot be represented at 22.05 kHz and must vanish
        let x: Vec<f32> = (0..48_000).map(|n| (2.0 * std::f64::consts::PI * 12_000.0 * n as f64 / 48_000.0).sin() as f32).collect();
        let y = resample::resample(&x, 48_000, 22_050);
        let peak = y[5_000..17_000].iter().map(|v| v.abs()).fold(0.0, f32::max);
        assert!(peak < 1e-4, "alias peak {peak}");
    }
}

