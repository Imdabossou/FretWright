"""Writes a trained pitch network as a flat little-endian f32 file for the Rust detector.

Layout: magic "FWPN" | u32 version | then, for each of the seven convolutions in ORDER,
weight (out, in, kh, kw) followed by bias, as f32.
"""
import struct, sys
import torch

ORDER = ['c1', 'c2', 'c3', 'oct', 'fifth', 'pool', 'out']

def export(pt, out):
    sd = torch.load(pt, map_location='cpu')
    with open(out, 'wb') as f:
        f.write(b'FWPN')
        f.write(struct.pack('<I', 1))
        n = 0
        for k in ORDER:
            for part in ('weight', 'bias'):
                t = sd[f'{k}.{part}'].detach().numpy().astype('<f4')
                f.write(t.tobytes())
                n += t.size
    print('wrote', out, n, 'weights')

if __name__ == '__main__':
    export(sys.argv[1], sys.argv[2])
