"""Harmonic-stacked spectrogram features and pitch-roll labels for every song in the work folder.

x: (T, 5, 138) float16 - for each frame, the strength of harmonics 1..5 of 138 candidate notes
   (3 bins per semitone from MIDI 22.67 to 68.67); y: (T, 46) uint8 - which semitones (MIDI 23..68)
   the chart has sounding in that frame.
"""
import json, sys, glob, os
import numpy as np
from scipy.io import wavfile
from scipy.signal import resample_poly
from math import gcd

SR, N_FFT, HOP = 22050, 4096, 256
LO_MIDI, N_SEMI, BINS_PER = 23, 46, 3
N_BINS = N_SEMI * BINS_PER
HARMONICS = 5

def load_stem(path):
    sr, x = wavfile.read(path)
    x = x.astype(np.float32) / 32768.0
    if x.ndim == 2:
        x = x.mean(axis=1)
    if sr != SR:
        g = gcd(sr, SR)
        x = resample_poly(x, SR // g, sr // g).astype(np.float32)
    return x

def stft_mag(y):
    pad = N_FFT // 2
    y = np.pad(y, (pad, pad))
    n = 1 + (len(y) - N_FFT) // HOP
    win = (0.5 - 0.5 * np.cos(2 * np.pi * np.arange(N_FFT) / N_FFT)).astype(np.float32)
    out = np.empty((n, N_FFT // 2 + 1), np.float32)
    for s in range(0, n, 2048):
        e = min(n, s + 2048)
        idx = np.arange(N_FFT)[None, :] + HOP * np.arange(s, e)[:, None]
        out[s:e] = np.abs(np.fft.rfft(y[idx] * win, axis=1))
    return out

def stack(mag):
    centers = 22.667 + np.arange(N_BINS) / BINS_PER             # MIDI of each candidate bin
    base = 440.0 * 2 ** ((centers - 69) / 12)
    df = SR / N_FFT
    out = np.empty((len(mag), HARMONICS, N_BINS), np.float32)
    for h in range(HARMONICS):
        pos = base * (h + 1) / df
        lo = np.floor(pos).astype(int)
        fr = (pos - lo).astype(np.float32)
        hi = np.minimum(lo + 1, mag.shape[1] - 1)
        out[:, h, :] = (1 - fr) * mag[:, lo] + fr * mag[:, hi]
    return np.log1p(30.0 * out)

def main(work, out):
    os.makedirs(out, exist_ok=True)
    for d in sorted(glob.glob(os.path.join(work, '*', 'project.fretwright.json'))):
        d = os.path.dirname(d)
        name = os.path.basename(d)
        stem = os.path.join(d, 'stems', 'htdemucs_ft', 'bass.wav')
        if not os.path.isfile(stem):
            continue
        p = json.load(open(os.path.join(d, 'project.fretwright.json')))
        notes = sorted(p['notes'], key=lambda n: n['start'])
        x = stack(stft_mag(load_stem(stem)))
        x = (x - x.mean(axis=(0, 2), keepdims=True)) / (x.std(axis=(0, 2), keepdims=True) + 1e-3)
        y = np.zeros((len(x), N_SEMI), np.uint8)
        ft = HOP / SR
        starts_all = sorted({round(n['start'], 3) for n in notes})
        for n in notes:
            s = n['start']
            later = [t for t in starts_all if t > s + 0.01]
            e = min(n['end'], later[0] if later else s + 0.4, s + 0.5)   # until the next note starts
            e = max(e, s + 0.06)
            m = n['midi'] - LO_MIDI
            if 0 <= m < N_SEMI:
                y[int(s / ft):int(e / ft) + 1, m] = 1
        meta = np.array([(n['start'], n['end'], n['midi']) for n in notes], np.float64)
        np.savez_compressed(os.path.join(out, name + '.npz'), x=x.astype(np.float16), y=y, notes=meta)
        print(name, x.shape, int(y.sum()), flush=True)

if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
