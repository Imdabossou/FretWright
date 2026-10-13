"""Spectrogram features and onset labels for every song in the work folder."""
import json, sys, glob, os
import numpy as np
from scipy.io import wavfile
from scipy.signal import resample_poly

SR = 22050
HOP = 128

def hann(n):
    return (0.5 - 0.5 * np.cos(2 * np.pi * np.arange(n) / n)).astype(np.float32)

def stft_mag(y, n_fft, hop):
    pad = n_fft // 2
    y = np.pad(y, (pad, pad))
    n = 1 + (len(y) - n_fft) // hop
    idx = np.arange(n_fft)[None, :] + hop * np.arange(n)[:, None]
    frames = y[idx] * hann(n_fft)[None, :]
    return np.abs(np.fft.rfft(frames, axis=1)).astype(np.float32)

def log_bands(mag, n_fft, fmin, fmax, n_bands):
    """Triangular filters spaced evenly in log frequency."""
    freqs = np.fft.rfftfreq(n_fft, 1 / SR)
    edges = np.geomspace(fmin, fmax, n_bands + 2)
    fb = np.zeros((n_bands, len(freqs)), dtype=np.float32)
    for i in range(n_bands):
        lo, c, hi = edges[i], edges[i + 1], edges[i + 2]
        up = (freqs - lo) / (c - lo)
        down = (hi - freqs) / (hi - c)
        fb[i] = np.clip(np.minimum(up, down), 0, None)
    fb /= np.maximum(fb.sum(axis=1, keepdims=True), 1e-9)
    return mag @ fb.T

def features(y):
    low = log_bands(stft_mag(y, 2048, HOP), 2048, 30.0, 2500.0, 96)
    high = log_bands(stft_mag(y, 512, HOP), 512, 150.0, 6000.0, 40)
    n = min(len(low), len(high))
    f = np.concatenate([low[:n], high[:n]], axis=1)
    f = np.log1p(30.0 * f)
    return f

def load_stem(path):
    sr, x = wavfile.read(path)
    x = x.astype(np.float32) / 32768.0
    if x.ndim == 2:
        x = x.mean(axis=1)
    if sr != SR:
        from math import gcd
        g = gcd(sr, SR)
        x = resample_poly(x, SR // g, sr // g).astype(np.float32)
    return x

def main(work, out):
    os.makedirs(out, exist_ok=True)
    for d in sorted(glob.glob(os.path.join(work, '*', 'project.fretwright.json'))):
        d = os.path.dirname(d)
        name = os.path.basename(d)
        stem = os.path.join(d, 'stems', 'htdemucs_ft', 'bass.wav')
        if not os.path.isfile(stem):
            continue
        p = json.load(open(os.path.join(d, 'project.fretwright.json')))
        starts = []
        for n in p['notes']:
            if not starts or abs(n['start'] - starts[-1]) > 0.02:
                starts.append(n['start'])
        f = features(load_stem(stem))
        f = (f - f.mean(axis=0, keepdims=True)) / (f.std(axis=0, keepdims=True) + 1e-3)
        y = np.zeros(len(f), dtype=np.float32)
        for t in starts:
            i = int(round(t * SR / HOP))
            for j in (i - 1, i, i + 1):
                if 0 <= j < len(y):
                    y[j] = max(y[j], 1.0 if j == i else 0.5)
        np.savez_compressed(os.path.join(out, name + '.npz'), x=f.astype(np.float16), y=y, starts=np.array(starts))
        print(name, f.shape, len(starts), flush=True)

if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
