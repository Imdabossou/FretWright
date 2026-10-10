# Contributing to FretWright

Thanks for helping. The project is a Cargo workspace; `README.md` lists the
crates.

## Building and testing

```bash
cargo build --release
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
```

On Linux you need the ALSA headers (`libasound2-dev` or `alsa-lib`). Neural
network tests need the model files: run `cargo run --release -p fw-cli --
models download --all` once, or set `FRETWRIGHT_MODELS`.

## Rules of the codebase

- **No `unsafe`** in our crates (`unsafe_code = "forbid"`), and avoid
  `unwrap()` outside tests (`unwrap_used = "warn"`).
- **The Rust code is the reference.** FretWright was ported from
  Rocksmith2014.NET, librosa, SciPy and Beat This! (credited in `LICENSES/`),
  and some behaviour deliberately matches those originals, for example
  case-insensitive prefix checks and round-half-to-even where .NET did that.
  Those spots are commented; keep them unless the game proves otherwise.
- **Vendored code** goes in `third_party/` with its licence, and every local
  change is recorded in `third_party/PATCHES.md`.
- **Comments** explain why, not what. Match the surrounding style.
- **Charts are tested in the game.** Anything that changes the SNG, the audio
  or the manifests needs a real Rocksmith run before release.

## Layout of a change

1. Write or extend a test.
2. Make the change and keep `cargo clippy` clean.
3. Describe what you verified (tests, in-game) in the pull request.
