//! Runs Beat This! on an audio file and dumps the log-mel (raw f32, 128 per
//! frame), the logits (raw f32, beat then downbeat) and the beats (JSON),
//! for comparison with the PyTorch reference.
#![allow(clippy::unwrap_used)]
use fw_models::beat_this::{self, BeatThis};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(out).unwrap();
    let t = std::time::Instant::now();
    let y = if args[1].ends_with(".f32") {
        // raw mono 22.05 kHz samples
        std::fs::read(&args[1]).unwrap().as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
    } else {
        let a = fw_audio::decode(std::path::Path::new(&args[1])).unwrap();
        fw_audio::resample::resample(&a.mono(), a.sample_rate, beat_this::SAMPLE_RATE)
    };
    let mel = beat_this::log_mel(&y);
    eprintln!("load + mel: {:.2?}", t.elapsed());
    let model = BeatThis::load_default().unwrap();
    let t = std::time::Instant::now();
    let (b, d) = model.logits(&mel, &mut |_| {}).unwrap();
    eprintln!("inference: {:.2?}", t.elapsed());
    let r = beat_this::postprocess(&b, &d);
    let raw = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    std::fs::write(out.join("mel.f32"), raw(&mel.concat())).unwrap();
    std::fs::write(out.join("logits.f32"), raw(&[b, d].concat())).unwrap();
    std::fs::write(out.join("beats.json"), serde_json::json!({"beats": r.beats, "downbeats": r.downbeats}).to_string()).unwrap();
    println!("{} beats, {} downbeats", r.beats.len(), r.downbeats.len());
}
