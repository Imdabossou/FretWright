"""Trains the string-from-sound model (11 timbre measurements -> 4 strings) and writes it for the Rust detector.

    python string_train.py FEATURES.csv WORKDIR final OUT.bin        # trained on every song
    python string_train.py FEATURES.csv WORKDIR loso               # one model per song, trained without it,
                                                                     # written to WORKDIR/<song>/string_net.bin
File layout: "FWST" | u32 version | u32 n_in | u32 hidden | mu | sd | w1 | b1 | w2 | b2 (f32 LE).
"""
import csv, json, struct, sys
import numpy as np
import torch, torch.nn as nn

STD = [28, 33, 38, 43]
cols = ['h1','h2','h3','h4','h5','h6','centroid','slope','decay','flatness']

def load(path, work):
    rows = list(csv.DictReader(open(path)))
    songs = sorted({r['song'] for r in rows})
    tun = {s: json.load(open(f'{work}/{s}/project.fretwright.json'))['tuning'] for s in songs}
    X, Y, M, S = [], [], [], []
    for r in rows:
        op = [STD[i] + tun[r['song']][i] for i in range(4)]
        midi = int(r['midi'])
        X.append([float(r[c]) for c in cols] + [midi / 12.0])
        Y.append(int(r['string'])); M.append([0 <= midi - op[i] <= 24 for i in range(4)]); S.append(r['song'])
    return np.array(X, np.float32), np.array(Y), np.array(M), np.array(S), songs

def fit(X, Y, M, epochs=200, hidden=32, seed=0):
    torch.manual_seed(seed)
    mu, sd = X.mean(0), X.std(0) + 1e-6
    net = nn.Sequential(nn.Linear(X.shape[1], hidden), nn.ReLU(), nn.Linear(hidden, 4))
    opt = torch.optim.Adam(net.parameters(), 3e-3, weight_decay=1e-3)
    xt, yt, mt = torch.tensor((X - mu) / sd), torch.tensor(Y), torch.tensor(M)
    for _ in range(epochs):
        loss = nn.functional.cross_entropy(net(xt).masked_fill(~mt, -1e9), yt)
        opt.zero_grad(); loss.backward(); opt.step()
    return net, mu, sd

def write(model, out):
    net, mu, sd = model
    w1, b1, w2, b2 = [p.detach().numpy().astype('<f4') for p in (net[0].weight, net[0].bias, net[2].weight, net[2].bias)]
    with open(out, 'wb') as f:
        f.write(b'FWST'); f.write(struct.pack('<III', 1, len(mu), w1.shape[0]))
        for a in (mu, sd, w1, b1, w2, b2):
            f.write(np.asarray(a, '<f4').tobytes())

if __name__ == '__main__':
    path, work, mode = sys.argv[1:4]
    X, Y, M, S, songs = load(path, work)
    if mode == 'final':
        write(fit(X, Y, M), sys.argv[4]); print('wrote', sys.argv[4])
    else:
        for s in songs:
            tr = S != s
            write(fit(X[tr], Y[tr], M[tr]), f'{work}/{s}/string_net.bin')
        print('wrote', len(songs), 'leave-one-song-out models')
