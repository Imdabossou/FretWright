# Works FretWright is derived from

Parts of FretWright are Rust ports of code from these projects. None of their
source files are included, but the ports are derivative works, so their
licences apply to those parts and are reproduced here.

| Project | Licence | Ported into |
|---------|---------|-------------|
| [Rocksmith2014.NET](https://github.com/iminashi/Rocksmith2014.NET) (Tapio Malmberg) | MIT, [Rocksmith2014.NET.txt](Rocksmith2014.NET.txt) | `fw-psarc`, `fw-sng`, `fw-xml`, `fw-convert`, `fw-improve`, `fw-dd`, `fw-package`, and the volume formula and preview in `fw-audio` |
| [librosa](https://github.com/librosa/librosa) | ISC, [librosa.txt](librosa.txt) | pYIN, onset strength, peak picking, RMS and the classic beat tracker in `fw-detect` |
| [SciPy](https://github.com/scipy/scipy) | BSD-3-Clause, [scipy.txt](scipy.txt) | the Butterworth design and zero-phase filtering in `fw-detect` |
| [Beat This!](https://github.com/CPJKU/beat_this) (JKU Linz) | MIT, [beat_this.txt](beat_this.txt) | log-mel front end, chunked inference and post-processing in `fw-models`; the model weights |
| [beat-this-rs](https://github.com/danigb/beat-this-rs) | MIT, [beat-this-rs.txt](beat-this-rs.txt) | the chunking scheme and ONNX export settings in `fw-models` |
| [Demucs](https://github.com/facebookresearch/demucs) (Meta) | MIT, [demucs.txt](demucs.txt) | the model weights, run by the vendored demucs-rs |

Vendored code, compiled into FretWright, keeps its licence next to it in
`third_party/` (wem: MIT or Apache-2.0; demucs-rs: Apache-2.0), and release
bundles carry `THIRD_PARTY_LICENSES.txt` with the licence of every crate.

The loudness meter in `fw-audio` is written from ITU-R BS.1770-4. The
resampler, the model download and the app are original.
