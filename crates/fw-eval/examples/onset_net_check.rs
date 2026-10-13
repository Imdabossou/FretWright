//! Compares the Rust onset network with PyTorch's output:
//! `onset_net_check <weights.bin> <stem.wav> <reference probabilities (raw f32)>`.
use fw_detect::onset_net::{OnsetNet, pick_onsets};
use fw_detect::pipeline::load_mono;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let net = OnsetNet::from_bytes(&std::fs::read(&a[1]).expect("weights")).expect("model");
    let y = load_mono(std::path::Path::new(&a[2])).expect("stem");
    let t = std::time::Instant::now();
    let prob = net.probabilities(&y);
    println!("{} frames in {:.1?}", prob.len(), t.elapsed());
    let reference: Vec<f32> = std::fs::read(&a[3]).expect("reference").as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    let n = prob.len().min(reference.len());
    let diffs: Vec<f32> = (0..n).map(|i| (prob[i] - reference[i]).abs()).collect();
    println!("frames {} (reference {}): mean |diff| {:.4}, max {:.3}, frames off by more than 0.1: {}", prob.len(), reference.len(), diffs.iter().sum::<f32>() / n as f32, diffs.iter().copied().fold(0.0, f32::max), diffs.iter().filter(|d| **d > 0.1).count());
    println!("onsets at 0.5: rust {}, reference {}", pick_onsets(&prob, 0.5).len(), pick_onsets(&reference, 0.5).len());
}
