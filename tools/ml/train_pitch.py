"""Pitch-roll network on harmonic-stacked spectrograms, tested on songs it has not seen.

    python train_pitch.py DATA FOLD ITERS [NFOLDS]     (FOLD -1: train on everything)
"""
import glob, os, sys, time
import numpy as np
import torch
import torch.nn as nn

SR, HOP = 22050, 256
LO_MIDI, N_SEMI = 23, 46
torch.set_num_threads(int(os.environ.get('THREADS', '8')))

class PNet(nn.Module):
    def __init__(self, ch=16):
        super().__init__()
        self.c1 = nn.Conv2d(5, ch, (5, 3), padding=(2, 1))
        self.c2 = nn.Conv2d(ch, ch, (5, 3), padding=(2, 2), dilation=(1, 2))
        self.c3 = nn.Conv2d(ch, ch, (5, 3), padding=(2, 4), dilation=(1, 4))
        self.oct = nn.Conv2d(ch, ch, (3, 1), padding=(36, 0), dilation=(36, 1))     # an octave away
        self.fifth = nn.Conv2d(ch, ch, (3, 1), padding=(21, 0), dilation=(21, 1))   # a fifth away
        self.pool = nn.Conv2d(ch, ch, (3, 1), stride=(3, 1))                          # 138 bins -> 46 semitones
        self.out = nn.Conv2d(ch, 1, 1)

    def forward(self, x):          # x: (B, 5, F, T)
        h = torch.relu(self.c1(x))
        h = h + torch.relu(self.c2(h))
        h = h + torch.relu(self.c3(h))
        h = h + torch.relu(self.oct(h)) + torch.relu(self.fifth(h))
        h = torch.relu(self.pool(h))
        return self.out(h).squeeze(1)    # (B, 46, T)

def load(data):
    songs = {}
    for f in sorted(glob.glob(os.path.join(data, '*.npz'))):
        z = np.load(f)
        songs[os.path.basename(f)[:-4]] = (z['x'], z['y'], z['notes'])
    return songs

def predict(net, x, chunk=1024, ctx=32):
    """x: (T, 5, F) -> (T, 46) probabilities."""
    net.eval()
    T = len(x)
    out = np.zeros((T, N_SEMI), np.float32)
    with torch.no_grad():
        for s in range(0, T, chunk):
            a, b = max(0, s - ctx), min(T, s + chunk + ctx)
            xb = torch.tensor(x[a:b].astype(np.float32)).permute(1, 2, 0)[None]   # (1,5,F,t)
            p = torch.sigmoid(net(xb))[0].permute(1, 0).numpy()
            out[s:min(T, s + chunk)] = p[s - a:s - a + min(chunk, T - s)]
    return out

def events(notes):
    """Single- and two-note events with their time spans, like the Rust lab."""
    by = []
    for s, e, m in notes:
        if by and abs(s - by[-1][0]) < 0.015:
            by[-1][2].append(int(m))
        else:
            by.append([s, e, [int(m)]])
    out = []
    for i, (s, e, ms) in enumerate(by):
        end = min(by[i + 1][0] if i + 1 < len(by) else s + 0.5, s + 0.8)
        if end - s >= 0.09 and len(ms) <= 2:
            out.append((s, end, ms))
    return out

def evaluate(net, x, notes):
    prob = predict(net, x)
    ft = HOP / SR
    single = [0, 0]
    pair = [0, 0, 0]   # both in top-2, either in top-1, total
    for s, e, ms in events(notes):
        a, b = int((s + 0.02) / ft), max(int((e - 0.01) / ft), int((s + 0.02) / ft) + 1)
        if b > len(prob):
            continue
        mean = prob[a:b].mean(axis=0)
        order = np.argsort(-mean) + LO_MIDI
        if len(ms) == 1:
            single[0] += int(order[0] == ms[0]); single[1] += 1
        else:
            pair[0] += int(set(order[:2]) == set(ms)); pair[1] += int(order[0] in ms); pair[2] += 1
    return single, pair

def train(songs, names, iters, seed=0):
    torch.manual_seed(seed); np.random.seed(seed)
    net = PNet()
    opt = torch.optim.Adam(net.parameters(), 2e-3)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, 4e-3, total_steps=iters)
    L, B = 160, 12
    weights = np.array([len(songs[n][0]) for n in names], float); weights /= weights.sum()
    loss_fn = nn.BCEWithLogitsLoss(pos_weight=torch.tensor(8.0))
    t0 = time.time()
    for it in range(iters):
        xs, ys = [], []
        for _ in range(B):
            n = names[np.random.choice(len(names), p=weights)]
            x, y, _ = songs[n]
            s = np.random.randint(0, len(x) - L)
            xs.append(x[s:s + L].astype(np.float32)); ys.append(y[s:s + L].astype(np.float32))
        xb = torch.tensor(np.stack(xs)).permute(0, 2, 3, 1)      # (B,5,F,T)
        yb = torch.tensor(np.stack(ys)).permute(0, 2, 1)         # (B,46,T)
        loss = loss_fn(net(xb), yb)
        opt.zero_grad(); loss.backward(); opt.step(); sched.step()
        if it % 100 == 0:
            print(f'  iter {it} loss {loss.item():.4f} ({time.time() - t0:.0f}s)', flush=True)
    return net

if __name__ == '__main__':
    data, fold, iters = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    nfolds = int(sys.argv[4]) if len(sys.argv) > 4 else 5
    songs = load(data)
    names = list(songs)
    test = [] if fold < 0 else [n for i, n in enumerate(names) if i % nfolds == fold]
    tr = [n for n in names if n not in test]
    print('train', len(tr), 'songs; test', test, flush=True)
    net = train(songs, tr, iters, int(os.environ.get('SEED', '0')))
    for n in test:
        x, y, notes = songs[n]
        single, pair = evaluate(net, x, notes)
        print(f'{n}: single-note events {100 * single[0] / max(single[1], 1):.1f}% of {single[1]};'
              f' two-note events: both found {100 * pair[0] / max(pair[2], 1):.1f}%, one right {100 * pair[1] / max(pair[2], 1):.1f}% of {pair[2]}', flush=True)
    torch.save(net.state_dict(), os.environ.get('OUT', f'pitch_fold{fold}.pt'))
