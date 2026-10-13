# FretWright user guide

FretWright turns a song into a Rocksmith 2014 bass arrangement in three steps:
**detect** the bass line, **fix** it in the editor, **build** the `.psarc`.

## First start

FretWright needs two neural networks: Demucs separates the bass from the mix and
Beat This! finds the beats and bar lines. On first start it offers to download
them (about 420 MB). They are checked against published SHA-256 sums and stored
in your user folder:

| System | Models folder |
|--------|---------------|
| Linux | `~/.local/share/fretwright/models` |
| Windows | `%APPDATA%\fretwright\models` |
| macOS | `~/Library/Application Support/fretwright/models` |

A `models` folder next to the program also works (for portable or offline
installs), as does `FRETWRIGHT_MODELS=/some/folder`. **Tools → Download models…**
fetches the optional ones (Demucs standard and 6-stem). **Tools → System check**
shows which GPU and models were found.

Without the models, detection still works with the low-pass filter and the
classic beat tracker, but noticeably worse.

## Projects

**File → New Project** asks for:

- **Song audio**: mp3, flac, ogg, wav, m4a, aac or opus.
- **Project folder**: defaults to `<song name> (FretWright)` next to the song. The
  project (`project.fretwright.json`), cached stems and build files live here.
- **Artist, title, album, year**: guessed from `Artist - Title.mp3`.
- **Album art**: an image with the same name as the song is picked up
  automatically. Without art, a cover with the artist and title is made.
- **Bass tuning**: E Standard, Drop D, Eb Standard, Drop C#, D Standard,
  Drop C, C# Standard or C Standard.

**File → Open Official Chart (.psarc / .xml)** opens a Rocksmith package or an arrangement
`.xml` as a project: its bass arrangement and song audio (decoded from the package) come
with every technique, so you can see, hear and edit an official chart. Packages with
stereo 44.1 kHz audio need `vgmstream-cli` on the PATH to decode it.

**File → Open Project** opens a `project.fretwright.json`. **Open Recent** remembers
the last ten. You can also pass a project folder or a song to `fretwright` on the
command line.

## Detection

The **Detection** tab:

| Setting | Choices |
|---------|---------|
| Bass separation | *Demucs v4 fine-tuned* (best, default), *Demucs v4* (faster), *Demucs v4 6-stem*, *Low-pass filter* (no AI) |
| Note starts | *Neural network* (default; trained on official charts, backed by spectral flux) or *Spectral flux* |
| Pitch tracking | *Neural network* (default), *Harmonic match* (works on distorted bass) or *pYIN* |
| Beat tracking | *Beat This!* (finds bar lines too) or the *classic tracker* |
| Device | *Auto* uses the GPU when one works (Vulkan, Metal or DX12), else the CPU |
| Shortest note | notes shorter than this are dropped (default 60 ms) |
| Snap notes to | the grid notes are moved onto (default 1/16) |

**Detect bass line** separates the bass (cached in the project, so it runs once
per model), tracks beats and pitch, makes notes and picks strings and frets.
On a modern GPU a 5-minute song takes about 40 s the first time in a session
(GPU kernels are compiled once per session) and about 20 s after that. On the
CPU, separation takes several minutes.

**Compare models…** runs the chosen separators and scores each result:
*agreement* is the share of notes whose pitch is the loudest one in the bass
stem while they sound, *coverage* the share of audible bass that has a note.
They are sanity checks, not ground truth: listen too. Double-click a row to
load it.

**Re-fret all notes** re-runs the string and fret optimiser. **Beat grid
−10 ms / +10 ms** shifts every beat if the grid sits slightly off the music.

## The editor

Four lanes, G on top and E at the bottom, like tab. The strip above them is the
bass stem's waveform, and the ruler numbers the bars.

**Mouse**

| Action | Does |
|--------|------|
| click a note | select it (Shift/Ctrl adds to the selection) |
| drag empty space | rubber-band select |
| drag a note | move it in time (snaps to the grid; hold Alt to move freely) and between strings, keeping the pitch when the new string can play it |
| drag a note's right edge | change its length |
| double-click a lane | add a note (it takes the fret of the previous note on that string) |
| click the ruler or waveform | move the playhead |
| right-click | note menu (delete, re-fret, techniques) and beat menu |
| wheel / Ctrl+wheel | scroll / zoom |

**Keys**

| Key | Does |
|-----|------|
| 0-9 | set the fret (type two digits quickly for 10-24) |
| Up / Down | pitch up or down a semitone on the same string |
| Shift+Up / Shift+Down | move to the next string, keeping the pitch |
| Left / Right | nudge by one grid step (Shift: one beat) |
| H P M A S | hammer-on, pull-off, fret-hand mute, accent, slide (to the next note's fret) |
| [ and ] | lower / raise the target fret of a slide, or the height of a bend |
| Delete / Backspace | delete the selection |
| Ctrl+A / Esc | select all / none |
| Ctrl+Z / Ctrl+Shift+Z | undo / redo |
| Space | play / pause |
| Ctrl+S | save |

**Bar lines**: if Beat This! starts the bars on the wrong beat, right-click the
beat that should be "1" and choose *Make this beat a bar line*. Bars after it
are re-counted; earlier ones stay.

**Grid**: the toolbar's *Grid* sets the snapping resolution (1/4 to 1/16
triplets). It does not change the notes.

## Listening

The toolbar's **Listen** menu chooses what plays: the full mix, the bass stem,
the chart synth (a simple plucked tone playing your notes), or a mix and the
synth together. *Full mix + chart synth* is the quickest way to hear wrong
notes. **Speed** 0.5x and 0.75x slow the music down without changing its pitch.
*Follow playhead* scrolls the editor along.

## Song details

The **Song** tab edits artist, title, album, charter, version and year, the
tuning (changing it keeps the sounding pitches and re-frets them), *played with
a pick*, and the album art.

## Building

The **Build** tab:

- **Platforms**: PC and/or Mac packages (`_p.psarc` / `_m.psarc`).
- **Preview start**: where the 28-second song-select preview starts.
  *Automatic* picks a bar line a third of the way in; *Use playhead* takes the
  current position.
- **Output folder**: defaults to `<project>/out`.

**Build .psarc** writes the arrangement, phrases, sections and dynamic
difficulty levels, encodes the audio as Wwise Vorbis `.wem` (without Wwise),
and packs everything. Copy the `.psarc` into Rocksmith 2014's `dlc` folder
(custom songs need a CDLC-enabled game).

Song IDs are stored in the project, so rebuilding an edited chart keeps your
scores in the game.

## Command line

`fretwright-cli` does the same without the app:

```bash
fretwright-cli convert "Artist - Song.mp3"                     # new project, detect, build
fretwright-cli convert song.mp3 --artist X --title Y --art cover.png --tuning "Drop D"
fretwright-cli detect "Song (FretWright)" --separator htdemucs     # re-detect a project
fretwright-cli build "Song (FretWright)" --platform pc,mac         # rebuild after editing
fretwright-cli models [download [--all]]                        # list / fetch models
fretwright-cli audio volume|wem|preview ...                     # audio tools
fretwright-cli psarc list|extract|pack ...                      # PSARC tools
fretwright-cli sng info file.sng                                # inspect a chart
```

## Troubleshooting

- **"Demucs ran on the CPU"**: no usable GPU was found. Check *Tools → System
  check*; on Linux, make sure the Vulkan driver is installed (`vulkaninfo`).
- **No sound**: *Tools → System check* shows the audio output. On Linux,
  FretWright uses ALSA (PipeWire and PulseAudio provide it).
- **Notes are an octave off**: select them and use Up/Down 12 times, or
  re-detect with another separator or pitch method.
- **The grid drifts**: songs with tempo changes are tracked beat by beat, so
  this is rare. Use the ±10 ms buttons for a constant offset.

## Techniques

Right-click a note for its menu: tick the techniques you want (hammer-on, pull-off, fret-hand mute,
accent, slide), or open **More techniques** for slide-out, bend, harmonic, palm mute, tremolo, vibrato,
slap, pop, tap and link-to-next. Each shows on the note: **H** hammer-on, **P** pull-off, **>** accent,
**M** fret-hand mute, **PM** palm mute, **Hm** harmonic, **Tr** tremolo, **~** vibrato, **Sl** slap,
**Po** pop, **Tp** tap, **L** linked, **b1** bend (height in steps). A slanted line with a number is a
slide to that fret; a dashed line is a slide out. The detector does not mark techniques yet: it finds
notes, strings and frets, and the techniques are for you to add.

## How good is the detection?

Measured against 20 official Rocksmith bass charts (each song scored by models that never saw it):
the overall F1 is about 71% (66% of the chart's notes are found with the right pitch and place, and
76% of the notes we make are real), and string and fret agree with the charters on 75% of those. Busy, fast and
chord-heavy basslines are the hard part (two-note chords are not detected yet). `fretwright-cli bench`
reruns the measurement on any folder of official `.psarc` files.
