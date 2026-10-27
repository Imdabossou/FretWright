# Releasing

## App releases

1. Set the version in the workspace `Cargo.toml` (`[workspace.package] version`)
   and run `cargo build` so `Cargo.lock` follows.
2. Commit, merge into `main`, then tag `main` and push:
   `git tag -a v0.2.0 -m "FretWright v0.2.0" && git push origin v0.2.0`.
   The build fails if the tag doesn't match the `Cargo.toml` version.
3. The `build` workflow tests everything, builds the Linux (`.tar.gz` and
   `.AppImage`), Windows (`.zip`) and macOS (`.app` in a `.zip`) bundles, and
   publishes them as a GitHub release titled `FretWright vX.Y.Z`.

Versions follow [Semantic Versioning](https://semver.org). `0.x` releases are
early access; `1.0.0` will be the first release that is ready for everyday use.

Locally, `cargo xtask package` makes the bundle for the current system in
`dist/`. Add `--with-models` for an offline bundle with `models/` inside, or
`--appimage` on Linux (needs `appimagetool`).

The macOS app is not signed or notarised. Users have to right-click → Open the
first time, or run `xattr -dr com.apple.quarantine FretWright.app`.

## Model files

The app downloads its models from the GitHub release tagged `models-1` of
`Imdabossou/FretWright` (`fw_models::download::DEFAULT_BASE_URL`). The
repository must be public for that to work without signing in. The assets are
the files in `models/` with `/` replaced by `-`.

How the files were made, and their licences, is in
[MODELS.md](MODELS.md). If a file ever changes, update its size and SHA-256
in `crates/fw-models/src/download.rs` and publish a new `models-N` release,
so older app versions keep working.

Publishing (with the GitHub CLI):

```bash
mkdir -p dist/models
for f in beat_this/final0.onnx demucs/htdemucs_ft.safetensors demucs/htdemucs.safetensors demucs/htdemucs_6s.safetensors; do
  cp "models/$f" "dist/models/${f//\//-}"
done
gh release create models-1 dist/models/* --title "Model files" \
  --notes "Weights used by FretWright. Demucs: Meta, MIT. Beat This!: JKU Linz, MIT."
```

## In-game check

Before tagging, build a song with the release bundle and play it in Rocksmith
2014. The game is the only real test of the SNG, audio and manifests.
