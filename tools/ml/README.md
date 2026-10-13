# Training the note-start network

FretWright finds where bass notes start with a small neural network (`crates/fw-detect/src/onset_net.rs`)
whose predictions are fused with a spectral-flux detector. It is trained on official Rocksmith charts:
the stem is the input, the chart's note starts are the answer. More songs make it better, so retrain it
whenever you have more charted songs.

## 1. Stems and charts (needs the GPU PC for speed)

    fretwright-cli bench "<folder of .psarc files>" --work <work> --device gpu

This imports every package (chart + audio) and separates the bass with Demucs. Each song ends up in
`<work>/<song>/` with `project.fretwright.json` (the chart) and `stems/htdemucs_ft/bass.wav`.
It also prints how well the current detector does on them.

## 2. Features and labels

    pip install numpy scipy torch
    python prep.py <work> <data>

## 3. Check how well it generalises (optional but wise)

    for f in 0 1 2 3 4; do python train_onset.py <data> $f 1000; done

Each fold trains on 4/5 of the songs and prints note-start F1 on the rest. Compare with the flux
detector (about 78.5% on the first 20 songs).

## 4. Train the model to ship

    for s in 0 1 2; do SEED=$s python train_onset.py <data> -1 1000; python export.py onset_final_s$s.pt onset_final_s$s.bin; done

Copy the `.bin` files to `crates/fw-detect/models/` and list them in `DEFAULT_ONSET_MODELS` in
`crates/fw-detect/src/pipeline.rs` (their probabilities are averaged). Rebuild, run `bench` again.

For an honest score on songs the models trained on, put a fold model in a song's work folder as
`onset_net.bin`: the detector prefers `onset_net*.bin` files in the project folder to the built-in ones.
