# FretWright Bass Detect (feedBack plugin)

Proof of concept: one click turns a song in your feedBack library into a playable
**bass arrangement**, written by [FretWright](https://github.com/Imdabossou/FretWright)'s
note networks. The result lands in the pak as a new arrangement,
**Bass (FretWright)**, so you can play it straight away or polish it in the Song Editor.

```
feedpak ─► bass stem (already made by Stem Splitter) ─► fretwright-cli ─► arrangements/bass_fretwright.json
```

## Why it is small

The plugin ships **no separator and no Python ML stack**. It reuses what feedBack
already has:

| Need | Comes from |
|---|---|
| Bass stem | `stems/bass.*` that **Stem Splitter** already wrote into the pak (BS-Roformer or Demucs, whichever you split with) |
| Writing the pak | the same backup + atomic-replace repack that Stem Splitter uses |
| Song menu entry | core's `libraryCardActions` registry (next to the editor's *Open in editor*) |
| Editing the result | the **Song Editor**, opened on the new arrangement through `window.editSong` |
| Detection | `fretwright-cli`, one native binary (Rust): Beat This! beats, note-start and pitch networks, a string-from-sound model, and a fret optimiser. Runs on the CPU in about 15 s a song |

Existing arrangements are never changed. Running it again replaces only its own
arrangement. The manifest records what made it under an extension key:

```yaml
bass_detection:
  engine: fretwright
  version: 1.0.0
  arrangement: bass_fretwright
  stem: feedpak:bs_roformer_sw
```

If a song has no bass stem, the plugin asks you to split it first. There is an
opt-in setting to let FretWright run its own Demucs (htdemucs_ft through wgpu, so
on any Vulkan, Metal or DX12 GPU) for those songs.

## Use

1. Install: copy (or symlink) this `feedback-plugin` folder into feedBack's plugins
   folder as `fretwright` and restart.
   ```bash
   git clone https://github.com/Imdabossou/FretWright
   cp -r FretWright/feedback-plugin <feedBack>/plugins/fretwright
   ```
2. Get `fretwright-cli` from a FretWright release (or build it with
   `cargo build --release -p fw-cli`), run `fretwright-cli models download` once, and
   point the plugin at it (**Settings → FretWright Bass Detect**), or put it on `PATH`,
   or set `FRETWRIGHT_CLI`.
3. In the library, open a song's **⋯** menu → **Detect bass line (FretWright)**, or use
   the **Bass Detect** screen. When it finishes, click **Open in editor** or **Play**.

Bass tuning is copied from the song's existing bass arrangement if it has one
(4-string), otherwise E standard.

## Routes

All under `/api/plugins/fretwright/`: `status`, `config` (POST), `songs`, `song`,
`detect` (POST `{filename}`), `jobs`, `jobs/{id}/cancel` (POST), `clear_cache` (POST).

## Develop

```bash
cd feedback-plugin
pip install pytest pyyaml
python -m pytest -q
```

## Limits (proof of concept)

- Single notes only: chords, hammer-ons, pull-offs, slides and bends are not detected
  yet (FretWright needs more training data for them).
- The networks were trained on `htdemucs_ft` stems; Stem Splitter defaults to
  `bs_roformer_sw`. Not benchmarked on Roformer stems yet.
- One job at a time; jobs are not kept across restarts.

## License

This folder is AGPL-3.0-only, like feedBack (the repack code follows Stem Splitter's);
see [LICENSE](LICENSE). The rest of the FretWright repository is MIT, and FretWright
runs as a separate program.
