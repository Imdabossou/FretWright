"""Trains the note-start network on charted songs and, optionally, tests it on songs it has not seen.

    python train_onset.py DATA FOLD ITERATIONS [NFOLDS]

DATA is the folder written by prep.py. FOLD >= 0 holds out every NFOLDS-th song (by sorted name) and
prints note-start F1 on them; FOLD = -1 trains on everything. SEED (environment) picks the random
start; the weights go to OUT (environment, default onset_<fold or final>_s<seed>.pt).
"""
import glob, os, sys, time
import numpy as np
import torch
import torch.nn as nn

SR, HOP = 22050, 128
torch.set_num_threads(int(os.environ.get('THREADS', '6')))


class Net(nn.Module):
    """Dilated residual convolutions over time, then a bidirectional GRU."""

    def __init__(self, n_in=136, ch=64):
        super().__init__()
        self.inp = nn.Conv1d(n_in, ch, 5, padding=2)
        self.blocks = nn.ModuleList([nn.Conv1d(ch, ch, 5, padding=2 * d, dilation=d) for d in (1, 2, 4, 8)])
        self.gru = nn.GRU(ch, 32, batch_first=True, bidirectional=True)
        self.out = nn.Linear(64, 1)

    def forward(self, x):          # x: (B, T, F)
        h = torch.relu(self.inp(x.transpose(1, 2)))
        for b in self.blocks:
            h = h + torch.relu(b(h))
        h, _ = self.gru(h.transpose(1, 2))
        return self.out(h).squeeze(-1)   # logits (B, T)


def load(data):
    songs = {}
    for f in sorted(glob.glob(os.path.join(data, '*.npz'))):
        z = np.load(f)
        songs[os.path.basename(f)[:-4]] = (z['x'].astype(np.float32), z['y'], z['starts'])
    return songs


def pick(prob, thr, min_gap=10, half=5):
    out = []
    for i in range(len(prob)):
        if prob[i] < thr:
            continue
        a, b = max(0, i - half), min(len(prob), i + half + 1)
        if prob[i] < prob[a:b].max() or (prob[a:b] == prob[i]).argmax() + a != i:
            continue
        if out and i - out[-1] < min_gap:
            if prob[i] > prob[out[-1]]:
                out[-1] = i
            continue
        out.append(i)
    return np.array(out) * HOP / SR


def f1(truth, found, tol=0.05):
    used = np.zeros(len(found), bool)
    hits = 0
    for t in truth:
        d = np.abs(found - t)
        d[used] = 9
        if len(d) and d.min() <= tol:
            used[d.argmin()] = True
            hits += 1
    r, p = hits / max(len(truth), 1), hits / max(len(found), 1)
    return 2 * r * p / (r + p) if r + p else 0.0, r, p


def train(songs, names, iters, seed=0):
    torch.manual_seed(seed)
    np.random.seed(seed)
    net = Net()
    opt = torch.optim.Adam(net.parameters(), 2e-3)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, 4e-3, total_steps=iters)
    L, B = 512, 16
    weights = np.array([len(songs[n][0]) for n in names], float)
    weights /= weights.sum()
    loss_fn = nn.BCEWithLogitsLoss(pos_weight=torch.tensor(6.0))
    t0 = time.time()
    for it in range(iters):
        xs, ys = [], []
        for _ in range(B):
            n = names[np.random.choice(len(names), p=weights)]
            x, y, _ = songs[n]
            s = np.random.randint(0, len(x) - L)
            xs.append(x[s:s + L])
            ys.append(y[s:s + L])
        loss = loss_fn(net(torch.tensor(np.stack(xs))), torch.tensor(np.stack(ys)))
        opt.zero_grad()
        loss.backward()
        opt.step()
        sched.step()
        if it % 200 == 0:
            print(f'  iter {it} loss {loss.item():.4f} ({time.time() - t0:.0f}s)', flush=True)
    return net


def predict(net, x):
    net.eval()
    with torch.no_grad():
        return torch.sigmoid(net(torch.tensor(x)[None])).numpy()[0]


if __name__ == '__main__':
    data, fold, iters = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    nfolds = int(sys.argv[4]) if len(sys.argv) > 4 else 5
    seed = int(os.environ.get('SEED', '0'))
    songs = load(data)
    names = list(songs)
    test = [] if fold < 0 else [n for i, n in enumerate(names) if i % nfolds == fold]
    tr = [n for n in names if n not in test]
    print(f'{len(tr)} training songs; testing on {test}', flush=True)
    net = train(songs, tr, iters, seed)
    for n in test:
        x, y, starts = songs[n]
        prob = predict(net, x)
        res = {thr: f1(starts, pick(prob, thr)) for thr in (0.3, 0.4, 0.5, 0.6, 0.7)}
        print(n, ' '.join(f'{thr}: F1 {r[0] * 100:.1f} (R {r[1] * 100:.0f} P {r[2] * 100:.0f})' for thr, r in res.items()), flush=True)
    out = os.environ.get('OUT', f'onset_{"final" if fold < 0 else "fold" + str(fold)}_s{seed}.pt')
    torch.save(net.state_dict(), out)
    print('saved', out)
