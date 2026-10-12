//! Trains a logistic-regression onset classifier on `onset_features` output and
//! tests it leave-one-song-out: `onset_train <features.csv>`.
use std::collections::BTreeMap;

struct Row {
    song: String,
    label: f64,
    x: Vec<f64>,
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

struct Model {
    mean: Vec<f64>,
    std: Vec<f64>,
    w: Vec<f64>,
    bias: f64,
}

impl Model {
    fn prob(&self, x: &[f64]) -> f64 {
        sigmoid(self.bias + x.iter().enumerate().map(|(i, v)| self.w[i] * (v - self.mean[i]) / self.std[i]).sum::<f64>())
    }
}

fn train(rows: &[&Row]) -> Model {
    let d = rows[0].x.len();
    let n = rows.len() as f64;
    let mean: Vec<f64> = (0..d).map(|i| rows.iter().map(|r| r.x[i]).sum::<f64>() / n).collect();
    let std: Vec<f64> = (0..d).map(|i| (rows.iter().map(|r| (r.x[i] - mean[i]).powi(2)).sum::<f64>() / n).sqrt().max(1e-6)).collect();
    let z: Vec<Vec<f64>> = rows.iter().map(|r| (0..d).map(|i| (r.x[i] - mean[i]) / std[i]).collect()).collect();
    let (mut w, mut b) = (vec![0.0; d], 0.0);
    let (mut mw, mut vw, mut mb, mut vb) = (vec![0.0; d], vec![0.0; d], 0.0, 0.0);
    for t in 1..=500 {
        let (mut gw, mut gb) = (vec![0.0; d], 0.0);
        for (zi, r) in z.iter().zip(rows) {
            let p = sigmoid(b + zi.iter().zip(&w).map(|(a, c)| a * c).sum::<f64>());
            let e = p - r.label;
            for i in 0..d {
                gw[i] += e * zi[i];
            }
            gb += e;
        }
        let lr = 0.05;
        for i in 0..d {
            let g = gw[i] / n + 1e-3 * w[i];
            mw[i] = 0.9 * mw[i] + 0.1 * g;
            vw[i] = 0.999 * vw[i] + 0.001 * g * g;
            w[i] -= lr * (mw[i] / (1.0 - 0.9f64.powi(t))) / ((vw[i] / (1.0 - 0.999f64.powi(t))).sqrt() + 1e-8);
        }
        let g = gb / n;
        mb = 0.9 * mb + 0.1 * g;
        vb = 0.999 * vb + 0.001 * g * g;
        b -= lr * (mb / (1.0 - 0.9f64.powi(t))) / ((vb / (1.0 - 0.999f64.powi(t))).sqrt() + 1e-8);
    }
    Model { mean, std, w, bias: b }
}

/// F1 over the reachable onsets (recall counts only chart onsets that have a candidate).
fn f1(rows: &[&Row], accept: impl Fn(&Row) -> bool) -> f64 {
    let real: f64 = rows.iter().map(|r| r.label).sum();
    let (mut tp, mut acc) = (0.0f64, 0.0f64);
    for r in rows {
        if accept(r) {
            acc += 1.0;
            tp += r.label;
        }
    }
    let (p, rc) = (tp / acc.max(1.0), tp / real.max(1.0));
    if p + rc > 0.0 { 2.0 * p * rc / (p + rc) } else { 0.0 }
}

fn main() {
    let text = std::fs::read_to_string(std::env::args().nth(1).expect("csv")).expect("read");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let rows: Vec<Row> = lines
        .map(|l| {
            let c: Vec<&str> = l.split(',').collect();
            Row { song: c[0].to_string(), label: c[2].parse().expect("label"), x: c[3..].iter().map(|v| v.parse().expect("num")).collect() }
        })
        .collect();
    let mut songs: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in &rows {
        songs.entry(r.song.clone()).or_default().push(r);
    }
    println!("{} candidates, {} real, {} songs", rows.len(), rows.iter().filter(|r| r.label > 0.5).count(), songs.len());

    // baseline: accept margin > 0.11 (the current detector)
    let base: Vec<f64> = songs.values().map(|s| f1(s, |r| r.x[0] > 0.11)).collect();
    println!("baseline (margin > 0.11): mean per-song reachable F1 {:.1}%", 100.0 * base.iter().sum::<f64>() / base.len() as f64);

    // leave one song out: the threshold is the best one on the other songs
    let names: Vec<&String> = songs.keys().collect();
    let mut held: Vec<f64> = Vec::new();
    for held_out in &names {
        let train_rows: Vec<&Row> = songs.iter().filter(|(k, _)| k != held_out).flat_map(|(_, v)| v.iter().copied()).collect();
        let model = train(&train_rows);
        let mut best = (0.0, 0.5);
        for k in 10..90 {
            let th = k as f64 / 100.0;
            let score: f64 = songs.iter().filter(|(k, _)| k != held_out).map(|(_, v)| f1(v, |r| model.prob(&r.x) > th)).sum::<f64>() / (names.len() - 1) as f64;
            if score > best.0 {
                best = (score, th);
            }
        }
        let s = f1(&songs[*held_out], |r| model.prob(&r.x) > best.1);
        held.push(s);
        println!("  {:14} baseline {:5.1}%  model {:5.1}%  (threshold {:.2})", held_out, 100.0 * base[held.len() - 1], 100.0 * s, best.1);
    }
    println!("leave-one-song-out mean per-song reachable F1: model {:.1}% vs baseline {:.1}%", 100.0 * held.iter().sum::<f64>() / held.len() as f64, 100.0 * base.iter().sum::<f64>() / base.len() as f64);

    let all: Vec<&Row> = rows.iter().collect();
    let model = train(&all);
    let mut best = (0.0, 0.5);
    for k in 10..90 {
        let th = k as f64 / 100.0;
        let score: f64 = songs.values().map(|v| f1(v, |r| model.prob(&r.x) > th)).sum::<f64>() / names.len() as f64;
        if score > best.0 {
            best = (score, th);
        }
    }
    println!("\nfinal model (all songs), threshold {:.2}:", best.1);
    for (i, name) in header[3..].iter().enumerate() {
        println!("  {name:18} mean {:9.4} std {:9.4} weight {:9.4}", model.mean[i], model.std[i], model.w[i]);
    }
    println!("  bias {:.4}", model.bias);
}
