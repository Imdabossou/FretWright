# Training the note networks

FretWright uses two small neural networks, both trained on official Rocksmith charts (the stem is the
input, the chart is the answer):

- the **note-start network** (`crates/fw-detect/src/onset_net.rs`), fused with a spectral-flux detector, says
  where notes begin;
- the **pitch network** (`crates/fw-detect/src/pitch_net.rs`) says which semitones sound in every frame.

More songs make both better, so retrain them whenever you have more charted songs. Three seeds of each are
averaged. Without a pitch network (or with `--pitch harmonic`) the detector falls back to a spectral
harmonic search.

## 1. Stems and charts (needs the GPU PC for speed)

    fretwright-cli bench "<folder of .psarc files>" --work <work> --device gpu

This imports every package (chart + audio) and separates the bass with Demucs. Each song ends up in
`<work>/<song>/` with `project.fretwright.json` (the chart) and `stems/htdemucs_ft/bass.wav`.
It also prints how well the current detector does on them.

## 2. Features and labels

    pip install numpy scipy torch
    python prep.py <work> <onset data>
    python prep_pitch.py <work> <pitch data>

## 3. Check how well they generalise (optional but wise)

    for f in 0 1 2 3 4; do python train_onset.py <onset data> $f 1000; python train_pitch.py <pitch data> $f 1500; done

Each fold trains on 4/5 of the songs and tests on the rest: note-start F1 for the first (flux detector:
about 78.5% on the first 20 songs), and the share of single-note events named correctly for the second
(the harmonic search: 79.3%).

## 4. Train the models to ship

    for s in 0 1 2; do
      SEED=$s python train_onset.py <onset data> -1 1000; python export.py onset_final_s$s.pt onset_net_$s.bin
      SEED=$s OUT=pitch_final_s$s.pt python train_pitch.py <pitch data> -1 1500; python export_pitch.py pitch_final_s$s.pt pitch_net_$s.bin
    done

Copy the `.bin` files to `crates/fw-detect/models/` (they are listed in `DEFAULT_ONSET_MODELS` and
`DEFAULT_PITCH_MODELS` in `crates/fw-detect/src/pipeline.rs`; probabilities are averaged). Rebuild, run
`bench` again.

For an honest score on songs a model trained on, train a fold model without that song and put it in the
song's work folder as `onset_net.bin` / `pitch_net.bin`: the detector prefers `onset_net*.bin` and
`pitch_net*.bin` files in the project folder to the built-in ones.
