# Building FretWright

## Requirements

- [Rust](https://rustup.rs) (stable)
- Linux: the ALSA headers (`libasound2-dev` on Debian/Ubuntu, `alsa-lib` on Arch)
- For the neural network tests: the model files. Run `cargo run --release -p fw-cli -- models download --all` once, or set `FRETWRIGHT_MODELS`.

## Commands

```bash
./run.sh                                                 # build and start the app (run.bat on Windows)
cargo test --workspace --release --locked                # tests
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask package                                      # a release bundle in dist/
```

CI runs clippy and the tests, then builds bundles for Linux, Windows and macOS. Pushing a `v*` tag publishes a release; see [docs/RELEASING.md](docs/RELEASING.md). Model files and their checksums are described in [docs/MODELS.md](docs/MODELS.md).

The networks are trained in Python; the recipe is in [tools/ml/README.md](tools/ml/README.md).

## Layout

```
crates/fw-app       the desktop app (egui)
crates/fw-cli       the fretwright-cli command line tool
crates/fw-detect    signal processing and the note networks: onsets, pitch, beats, notes, fretting
crates/fw-eval      scores the detector against official charts (bench, score)
tools/ml            Python training of the networks
crates/fw-models    neural networks: Beat This!, Demucs, model downloads
crates/fw-project   project files, chart conversion, build pipeline
crates/fw-package   manifests, sound banks, album art, packaging
crates/fw-{psarc,sng,xml,convert,improve,dd,audio}   formats, charting and audio
crates/xtask         release packaging (cargo xtask package)
third_party/         vendored wem and demucs-core (see PATCHES.md)
LICENSES/            licences of the works FretWright is ported from
feedback-plugin/     the feedBack plugin (Python, not in release bundles)
```

## How it works

| Stage | How |
|-------|-----|
| Bass separation | Meta's Demucs v4 (fine-tuned, standard or 6-stem) via [demucs-rs](https://github.com/nikhilunni/demucs-rs) on any Vulkan, Metal or DX12 GPU (or the CPU); or a plain low-pass filter |
| Beats and bars | [Beat This!](https://github.com/CPJKU/beat_this) (ISMIR 2024) on [rten](https://github.com/robertknight/rten), or a classic tracker |
| Notes | two small neural networks trained on official charts (note starts, pitch), fused with spectral-flux onsets; harmonic and pYIN pitch remain as alternatives; a string model picks strings by sound, then a string/fret optimiser |
| Charting | phrases, sections and dynamic difficulty, ported from [Rocksmith2014.NET](https://github.com/iminashi/Rocksmith2014.NET) |
| Audio | Wwise 2013 Vorbis `.wem` via [rabbit-smith/wem](https://github.com/rabbit-smith/wem), so Wwise is not needed |
| Packaging | SNG, manifests, sound banks and encrypted PSARC, written from scratch in Rust; `.feedpak` (Ogg Vorbis audio through libvorbis/aoTuV) |

Parts of FretWright are Rust ports of these projects, each checked against
the original (PyTorch, librosa, Rocksmith2014.NET); the credits are in
[LICENSES/](LICENSES/README.md).
