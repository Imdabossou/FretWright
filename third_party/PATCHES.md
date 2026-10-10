# Vendored code

Code compiled into FretWright, copied from upstream so builds never depend on
anything outside this repository.

## wem (rabbit-smith/wem, MIT OR Apache-2.0)

The Wwise 2013.2 Vorbis encoder that writes Rocksmith's `.wem` audio. See
`wem/VENDORED_FROM` for the upstream revision, and `wem/PROVENANCE.md` and
`wem/THIRD_PARTY_NOTICES.md` for how it was made and what it derives from.

- Only the six crates FretWright compiles are vendored: `wem-core`,
  `wem-scheduling`, `wem-analysis`, `wem-vorbis`, `wem-container` and
  `wem-profiles` (plus the docs PROVENANCE.md links to). Upstream's C,
  Python and WebAssembly shells, fuzzer, Python package and test suite (with its
  `.wem` fixtures) are left out, so a few upstream tests that read those
  fixtures can't run here. `crates/Cargo.toml` lists only the six members.
- No source changes.

## demucs-rs (nikhilunni/demucs-rs, Apache-2.0)

Demucs source separation on burn. See `demucs-rs/VENDORED_FROM`.

- Only `demucs-core` is vendored (no CLI, plugin, wasm or web UI). No source
  changes.
- Its model download (a third-party Hugging Face mirror) is not used: FretWright
  ships its own download of weights converted from Meta's official checkpoints
  (docs/MODELS.md).

## Formerly vendored

Rocksmith2014.NET (MIT) was vendored while the .NET builder served as the test
oracle for the Rust ports. It was removed once the Rust code replaced it; its
licence stays in `LICENSES/` because the ports derive from it.
