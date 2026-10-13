"""Writes a trained onset network as a flat little-endian f32 file the Rust detector reads.

Layout: magic "FWON" | u32 version | u32 n_in | u32 ch | u32 gru_hidden | u32 n_blocks |
        n_blocks x u32 dilation | then f32 tensors in the order listed in ORDER.
"""
import struct, sys
import numpy as np
import torch

ORDER = ['inp.weight', 'inp.bias'] + \
        [f'blocks.{i}.{k}' for i in range(4) for k in ('weight', 'bias')] + \
        ['gru.weight_ih_l0', 'gru.weight_hh_l0', 'gru.bias_ih_l0', 'gru.bias_hh_l0',
         'gru.weight_ih_l0_reverse', 'gru.weight_hh_l0_reverse', 'gru.bias_ih_l0_reverse', 'gru.bias_hh_l0_reverse',
         'out.weight', 'out.bias']

def export(pt, out):
    sd = torch.load(pt, map_location='cpu')
    n_in = sd['inp.weight'].shape[1]
    ch = sd['inp.weight'].shape[0]
    hidden = sd['gru.weight_hh_l0'].shape[1]
    dil = [1, 2, 4, 8]
    with open(out, 'wb') as f:
        f.write(b'FWON')
        f.write(struct.pack('<IIIII', 1, n_in, ch, hidden, len(dil)))
        f.write(struct.pack('<' + 'I' * len(dil), *dil))
        for k in ORDER:
            f.write(sd[k].detach().numpy().astype('<f4').tobytes())
    print('wrote', out, sum(sd[k].numel() for k in ORDER), 'weights')

if __name__ == '__main__':
    export(sys.argv[1], sys.argv[2])
