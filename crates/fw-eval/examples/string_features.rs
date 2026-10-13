//! Timbre measurements of every charted note (with the string it was played on), for
//! learning to tell strings apart by sound: `string_features <work folder> <out.csv>`.
use std::io::Write;

use fw_detect::dsp::{self, Fft};
use fw_detect::pipeline::{self, SR, load_mono};

const N_FFT: usize = 32768;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut dirs: Vec<_> = std::fs::read_dir(&a[1]).expect("work").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("project.fretwright.json").is_file()).collect();
    dirs.sort();
    let fft = Fft::new(N_FFT);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&a[2]).expect("csv"));
    writeln!(out, "song,time,midi,string,fret,h1,h2,h3,h4,h5,h6,centroid,slope,decay,flatness").expect("write");
    let hz_per_bin = SR / N_FFT as f64;
    for d in dirs {
        let name = d.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let p = fw_project::Project::load(&d).expect("project");
        let stem = load_mono(&pipeline::stem_path(&d, "htdemucs_ft")).expect("stem");
        let mut by_start: Vec<(f64, Vec<&fw_project::Note>)> = Vec::new();
        for n in &p.notes {
            match by_start.last_mut() {
                Some((t, v)) if (n.start - *t).abs() < 0.015 => v.push(n),
                _ => by_start.push((n.start, vec![n])),
            }
        }
        for (i, (t, v)) in by_start.iter().enumerate() {
            let end = by_start.get(i + 1).map_or(t + 0.5, |x| x.0).min(t + 0.8);
            if v.len() != 1 || end - t < 0.12 {
                continue;
            }
            let n = v[0];
            let (a0, b0) = (((t + 0.02) * SR) as usize, ((end - 0.01) * SR) as usize);
            if b0 > stem.len() || b0 <= a0 + 512 {
                continue;
            }
            let w = dsp::hann(b0 - a0);
            let x: Vec<f64> = stem[a0..b0].iter().zip(&w).map(|(s, w)| s * w).collect();
            let mag: Vec<f64> = fft.power(&x).iter().map(|p| p.sqrt()).collect();
            let f0 = 440.0 * 2f64.powf((n.midi as f64 - 69.0) / 12.0);
            let peak = |f: f64| -> f64 {
                let half = f * 0.015;
                let (lo, hi) = (((f - half) / hz_per_bin) as usize, ((f + half) / hz_per_bin) as usize + 1);
                if hi >= mag.len() { 0.0 } else { (lo..=hi).map(|k| mag[k]).fold(0.0, f64::max) }
            };
            let hs: Vec<f64> = (1..=6).map(|h| peak(f0 * h as f64)).collect();
            let total: f64 = hs.iter().sum::<f64>().max(1e-9);
            // centroid and slope over 40-3000 Hz
            let (klo, khi) = ((40.0 / hz_per_bin) as usize, (3000.0 / hz_per_bin) as usize);
            let (mut sw, mut swf) = (0.0, 0.0);
            let (mut sx, mut sy, mut sxx, mut sxy, mut cnt) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let mut log_sum = 0.0;
            for (k, &m) in mag.iter().enumerate().take(khi).skip(klo) {
                sw += m;
                swf += m * (k as f64 * hz_per_bin).log2();
                let (lx, ly) = ((k as f64 * hz_per_bin).log2(), (m + 1e-9).ln());
                sx += lx;
                sy += ly;
                sxx += lx * lx;
                sxy += lx * ly;
                cnt += 1.0;
                log_sum += ly;
            }
            let slope = (cnt * sxy - sx * sy) / (cnt * sxx - sx * sx).max(1e-9);
            let flat = (log_sum / cnt).exp() / (sw / cnt).max(1e-9);
            // energy of the last 40% over the first 40%
            let len = b0 - a0;
            let e = |a: usize, b: usize| (stem[a0 + a..a0 + b].iter().map(|v| v * v).sum::<f64>() / (b - a).max(1) as f64).sqrt();
            let decay = 20.0 * (e(len * 6 / 10, len) / e(0, len * 4 / 10).max(1e-9)).max(1e-6).log10();
            writeln!(
                out,
                "{name},{:.3},{},{},{},{}",
                t,
                n.midi,
                n.string,
                n.fret,
                hs.iter().map(|h| format!("{:.4}", h / total)).chain([format!("{:.4}", swf / sw.max(1e-9)), format!("{slope:.4}"), format!("{decay:.3}"), format!("{flat:.4}")]).collect::<Vec<_>>().join(",")
            )
            .expect("write");
        }
    }
}
