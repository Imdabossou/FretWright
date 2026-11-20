# Contributing to FretWright

Thanks for helping. The project is a Cargo workspace; [BUILDING.md](BUILDING.md) lists the
crates.

## Before you start

- **Bugs and ideas:** open an issue. For detection problems, say which song (not the audio, unless it's freely licensed), the separator and the detection settings.
- **Branches:** open pull requests against `development`. `main` only gets tested releases.
- **Checks:** `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --release --locked` must pass. CI runs both. Setup is in [BUILDING.md](BUILDING.md).
- **No copyrighted audio or charts:** never commit songs, stems or official charts, including in tests and screenshots.
- **Licence:** contributions are accepted under the project's licence (GPL-3.0-or-later; the feedBack plugin is AGPL-3.0). Third-party code must be compatible.
- **Security issues:** please report them privately, see [SECURITY.md](SECURITY.md).

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
