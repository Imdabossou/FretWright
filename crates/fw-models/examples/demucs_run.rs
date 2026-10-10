//! Separates one stem of an audio file and writes it as raw interleaved f32
//! stereo at 44.1 kHz, printing the time of each stage.
//! usage: demucs_run <audio> <out.f32> [model] [stem] [auto|gpu|cpu]
#![allow(clippy::unwrap_used)]
use fw_models::demucs::{self, Device};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model = args.get(3).map_or("htdemucs_ft", |s| s.as_str());
    let stem = args.get(4).map_or("bass", |s| s.as_str());
    let device = match args.get(5).map(|s| s.as_str()) {
        Some("gpu") => Device::Gpu,
        Some("cpu") => Device::Cpu,
        _ => Device::Auto,
    };
    let t = std::time::Instant::now();
    let a = fw_audio::decode(std::path::Path::new(&args[1])).unwrap();
    let ch = |i: usize| fw_audio::resample::resample(&a.channels[i.min(a.channels.len() - 1)], a.sample_rate, demucs::SAMPLE_RATE);
    let (l, r) = (ch(0), ch(1));
    eprintln!("decode + resample: {:.2?}", t.elapsed());
    let t = std::time::Instant::now();
    let out = demucs::separate(model, &[stem], device, &l, &r, &|f| eprintln!("{:6.2}s {:5.1}%", t.elapsed().as_secs_f64(), f * 100.0)).unwrap();
    eprintln!("\nseparate on {}: {:.2?}", out.device, t.elapsed());
    let (sl, sr) = out.stem(stem).unwrap();
    let bytes: Vec<u8> = sl.iter().zip(sr).flat_map(|(a, b)| [a.to_le_bytes(), b.to_le_bytes()]).flatten().collect();
    std::fs::write(&args[2], bytes).unwrap();
}
