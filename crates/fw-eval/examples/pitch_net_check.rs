//! Compares the Rust pitch network with PyTorch's output:
//! `pitch_net_check <weights.bin> <stem.wav> <reference (raw f32, frames x 46)>`.
use fw_detect::pipeline::load_mono;
use fw_detect::pitch_net::PitchNet;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let net = PitchNet::from_bytes(&std::fs::read(&a[1]).expect("weights")).expect("model");
    let y = load_mono(std::path::Path::new(&a[2])).expect("stem");
    let t = std::time::Instant::now();
    let roll = net.probabilities(&y);
    println!("{} frames in {:.1?}", roll.frames, t.elapsed());
    let reference: Vec<f32> = std::fs::read(&a[3]).expect("reference").as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    let n = (reference.len() / 46).min(roll.frames);
    let (mut sum, mut max, mut bad) = (0f64, 0f32, 0usize);
    for t in 0..n {
        let mine = roll.mean(t as f64 * 256.0 / 22050.0, (t as f64 + 0.5) * 256.0 / 22050.0);
        for k in 0..46 {
            let d = (mine[k] - reference[t * 46 + k]).abs();
            sum += d as f64;
            max = max.max(d);
            bad += usize::from(d > 0.1);
        }
    }
    println!("frames {} (reference {}): mean |diff| {:.5}, max {:.3}, values off by more than 0.1: {}", roll.frames, reference.len() / 46, sum / (n * 46) as f64, max, bad);
}
