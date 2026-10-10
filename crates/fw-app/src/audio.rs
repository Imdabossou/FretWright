//! Offline audio helpers for playback: the chart synth, a pitch-preserving
//! time-stretch for slowed-down practice, and waveform peaks.

use fw_project::Project;

fn midi_hz(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

/// A plucked-bass-ish rendering of the chart, for checking notes by ear.
pub fn synth(p: &Project, length: f64, sr: u32) -> Vec<f32> {
    let sr_f = sr as f64;
    let mut out = vec![0f32; (length * sr_f) as usize + sr as usize];
    let mut noise = 0x9E37_79B9u32; // deterministic xorshift for muted notes
    for n in &p.notes {
        let f = midi_hz(n.midi as f64);
        let dur = n.duration().clamp(0.08, 4.0);
        let len = (dur * sr_f) as usize;
        let fade = len.min(200);
        let f_end = if n.slide_to >= 0 { midi_hz((p.open_strings()[n.string as usize % 4] + n.slide_to) as f64) } else { f };
        let a = (n.start.max(0.0) * sr_f) as usize;
        let mut phase = 0.0f64;
        for i in 0..len {
            let t = i as f64 / sr_f;
            let mut env = (-t * 3.0).exp() * (t / 0.004).min(1.0);
            if i >= len - fade {
                env *= (len - i) as f64 / fade as f64;
            }
            let freq = f + (f_end - f) * i as f64 / len as f64;
            phase += 2.0 * std::f64::consts::PI * freq / sr_f;
            let tone = if n.mute {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                (noise as f64 / u32::MAX as f64 * 2.0 - 1.0) * 0.6 * (-t * 40.0).exp()
            } else {
                phase.sin() + 0.45 * (2.0 * phase).sin() + 0.2 * (3.0 * phase).sin()
            };
            if let Some(o) = out.get_mut(a + i) {
                *o += (0.25 * tone * env) as f32;
            }
        }
    }
    out
}

/// Plays `x` at `speed` (e.g. 0.5) without changing its pitch (WSOLA:
/// overlap-added Hann frames, each shifted to best continue the previous one).
pub fn stretch(x: &[f32], speed: f64, sr: u32) -> Vec<f32> {
    if (speed - 1.0).abs() < 1e-6 || x.is_empty() {
        return x.to_vec();
    }
    let n = ((sr as f64 * 0.046) as usize).next_power_of_two(); // ~46 ms frames
    let hs = n / 2; // output hop
    let ha = hs as f64 * speed; // analysis hop
    let tol = n / 4; // search range
    let window: Vec<f32> = (0..n).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32).collect();
    let out_len = (x.len() as f64 / speed) as usize;
    let mut out = vec![0f32; out_len + n];
    let at = |i: isize| -> f32 { if i >= 0 && (i as usize) < x.len() { x[i as usize] } else { 0.0 } };
    let mut prev: isize = 0; // input position of the previous frame
    let mut k = 0usize;
    while k * hs < out_len {
        let nominal = (k as f64 * ha) as isize;
        let pos = if k == 0 {
            0
        } else {
            // the natural continuation of the previous frame, and the candidate
            // around the nominal position that resembles it most
            let target = prev + hs as isize;
            let score = |d: isize, step: usize| -> f32 {
                (0..hs).step_by(step).map(|i| at(target + i as isize) * at(nominal + d + i as isize)).sum()
            };
            let coarse = (-(tol as isize)..=tol as isize).step_by(4).max_by(|&a, &b| score(a, 4).total_cmp(&score(b, 4))).unwrap_or(0);
            let fine = (coarse - 3..=coarse + 3).max_by(|&a, &b| score(a, 1).total_cmp(&score(b, 1))).unwrap_or(coarse);
            nominal + fine
        };
        for i in 0..n {
            out[k * hs + i] += at(pos + i as isize) * window[i];
        }
        prev = pos;
        k += 1;
    }
    out.truncate(out_len);
    out
}

/// Peak envelope (0..1, square-root scaled for visibility) at `rate` values per second.
pub fn peaks(x: &[f32], sr: u32, rate: f64) -> Vec<f32> {
    let block = ((sr as f64 / rate) as usize).max(1);
    let p: Vec<f32> = x.chunks(block).map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs()))).collect();
    let max = p.iter().copied().fold(0f32, f32::max).max(1e-9);
    p.iter().map(|v| (v / max).sqrt()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_keeps_pitch_and_changes_length() {
        let sr = 48_000;
        let x: Vec<f32> = (0..sr).map(|i| (2.0 * std::f64::consts::PI * 220.0 * i as f64 / sr as f64).sin() as f32).collect();
        let y = stretch(&x, 0.5, sr);
        assert_eq!(y.len(), 2 * sr as usize);
        // count zero crossings in the middle second: 220 Hz -> ~440 crossings
        let mid = &y[sr as usize / 2..sr as usize * 3 / 2];
        let crossings = mid.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((430..=450).contains(&crossings), "{crossings} crossings");
    }

    #[test]
    fn synth_places_notes_in_time() {
        let mut p = Project::new("/tmp/x", "a.mp3");
        p.notes.push(fw_project::Note::new(1.0, 1.5, 40));
        let y = synth(&p, 2.0, 8000);
        assert!(y[..7900].iter().all(|v| *v == 0.0));
        assert!(y[8000..8400].iter().any(|v| v.abs() > 0.05));
    }
}
