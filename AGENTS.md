# AGENTS.md

Desktop app and CLI that turns a song into a Rocksmith 2014 bass arrangement: Demucs bass separation, beat tracking, neural note detection, a tab editor, and `.psarc` / `.feedpak` building. Rust workspace (`crates/`), egui app, Python training scripts in `tools/ml`, and a Python feedBack plugin in `feedback-plugin/`.

## Commands

- Tests: `cargo test --workspace --release --locked` (neural tests need models: `cargo run --release -p fw-cli -- models download --all` or `FRETWRIGHT_MODELS`)
- Lint: `cargo clippy --workspace --all-targets --locked -- -D warnings`
- Run: `./run.sh`
- Package: `cargo xtask package`
- Score the detector: `fretwright-cli bench <folder of official .psarc> --work <dir>`
- Prefer `-p <crate>` while iterating; whole-workspace release builds are heavy.
- More in [BUILDING.md](BUILDING.md); codebase rules in [CONTRIBUTING.md](CONTRIBUTING.md)

## Conventions

- Branches: work on `development`; `main` only gets tested releases.
- No `unsafe` in our crates, and no `unwrap()` outside tests.
- Some behaviour deliberately matches the ported originals (Rocksmith2014.NET, librosa, SciPy, Beat This!). Those spots are commented; keep them.
- Vendored code goes in `third_party/` with its licence, and every local change is recorded in `third_party/PATCHES.md`.
- Never commit copyrighted songs, stems or official charts. Screenshots use Creative Commons music only.
- Follow the system light/dark theme and accent colour, with a remembered override. The string colours are fixed on purpose.
- The README's early-access note quotes the detector's score. Update it when the models improve.
- Keep README.md short. User how-to goes in `docs/USER_GUIDE.md`, developer detail in BUILDING.md.

## Traps

- Score the detector honestly: when benchmarking on songs the shipped models were trained on, use leave-one-song-out models, or the result is optimistic.
- Song section names are limited to the game's list (`SECTION_NAMES`). Unknown names make the psarc manifest fail.
- `fw_improve::phrases::generate()` wipes `xml.sections`, so sections are applied afterwards with `set_sections`.
- feedpak: feedBack ignores `song_timeline.json` unless both `beats` and `sections` are lists. Project bends are in whole steps; feedpak `bn` is in semitones.
- Charts must be tested in the real game before release. Tests can't prove the SNG, audio or manifests load.
