//! Project -> `.feedpak` (feedpak format v1): a zip of a YAML manifest, a JSON chart,
//! a beat/section timeline and Ogg Vorbis audio. The format is open and is
//! specified at <https://github.com/got-feedback/feedpak-spec>.

use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::build::{BuildProgress, default_preview_start, fresh};
use crate::{Error, Note, Project, Result};

/// The format version these packages conform to.
const FEEDPAK_VERSION: &str = "1.19.0";
const ARRANGEMENT_FILE: &str = "arrangements/bass.json";
const TIMELINE_FILE: &str = "song_timeline.json";
const STEM_FILE: &str = "stems/full.ogg";
const PREVIEW_FILE: &str = "preview.ogg";
/// A navigation section every this many bars, when the user marked no sections.
const BARS_PER_SECTION: usize = 8;

/// Times are stored to the millisecond, like the rest of the Rocksmith-style chart.
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_minus_one(v: &i32) -> bool {
    *v == -1
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// One note in the wire format (spec section 6.2). Fields at their default are left out.
#[derive(Serialize)]
struct PakNote {
    t: f64,
    s: i32,
    f: i32,
    #[serde(skip_serializing_if = "is_zero")]
    sus: f64,
    #[serde(skip_serializing_if = "is_minus_one")]
    sl: i32,
    #[serde(skip_serializing_if = "is_minus_one")]
    slu: i32,
    #[serde(skip_serializing_if = "is_zero")]
    bn: f64,
    #[serde(skip_serializing_if = "is_false")]
    ho: bool,
    #[serde(skip_serializing_if = "is_false")]
    po: bool,
    #[serde(skip_serializing_if = "is_false")]
    hm: bool,
    #[serde(skip_serializing_if = "is_false")]
    pm: bool,
    #[serde(skip_serializing_if = "is_false")]
    vb: bool,
    #[serde(skip_serializing_if = "is_false")]
    tr: bool,
    #[serde(skip_serializing_if = "is_false")]
    ac: bool,
    #[serde(skip_serializing_if = "is_false")]
    tp: bool,
    #[serde(skip_serializing_if = "is_false")]
    ln: bool,
    #[serde(skip_serializing_if = "is_false")]
    fhm: bool,
    #[serde(skip_serializing_if = "is_false")]
    plk: bool,
    #[serde(skip_serializing_if = "is_false")]
    slp: bool,
}

impl From<&Note> for PakNote {
    fn from(n: &Note) -> PakNote {
        PakNote {
            t: round3(n.start),
            s: n.string,
            f: n.fret,
            sus: round3(n.chart_sustain()),
            sl: n.slide_to,
            slu: n.slide_unpitch_to,
            // the project stores whole steps, the format semitones
            bn: f64::from(n.bend) * 2.0,
            ho: n.hammer_on,
            po: n.pull_off,
            hm: n.harmonic,
            pm: n.palm_mute,
            vb: n.vibrato,
            tr: n.tremolo,
            ac: n.accent,
            tp: n.tap,
            ln: n.link_next,
            fhm: n.mute,
            plk: n.pop,
            slp: n.slap,
        }
    }
}

#[derive(Serialize)]
struct PakAnchor {
    time: f64,
    fret: i32,
    width: i32,
}

#[derive(Serialize)]
struct Arrangement {
    name: &'static str,
    tuning: [i32; 4],
    capo: i32,
    #[serde(rename = "centOffset", skip_serializing_if = "is_zero")]
    cent_offset: f64,
    notes: Vec<PakNote>,
    chords: [(); 0],
    anchors: Vec<PakAnchor>,
    handshapes: [(); 0],
    templates: [(); 0],
}

#[derive(Serialize)]
struct Beat {
    time: f64,
    measure: i32,
}

#[derive(Serialize)]
struct Section {
    name: String,
    number: i32,
    time: f64,
}

#[derive(Serialize)]
struct Tempo {
    time: f64,
    bpm: f64,
}

#[derive(Serialize)]
struct TimeSignature {
    time: f64,
    ts: [usize; 2],
}

#[derive(Serialize)]
struct Timeline {
    version: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tempos: Vec<Tempo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    time_signatures: Vec<TimeSignature>,
    beats: Vec<Beat>,
    // Always written: feedBack ignores a timeline whose `sections` is not a list.
    sections: Vec<Section>,
}

/// The tuning pitch as a cent offset from A440.
fn cent_offset(pitch: f64) -> f64 {
    if pitch > 0.0 { ((1200.0 * (pitch / 440.0).log2()) * 100.0).round() / 100.0 } else { 0.0 }
}

fn arrangement(p: &Project) -> Arrangement {
    let mut notes: Vec<&Note> = p.notes.iter().collect();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.string.cmp(&b.string)));
    let anchors = crate::arrangement::anchors(&notes)
        .into_iter()
        .map(|a| PakAnchor { time: f64::from(a.time) / 1000.0, fret: i32::from(a.fret), width: i32::from(a.width) })
        .collect();
    Arrangement {
        name: "Bass",
        tuning: p.tuning,
        capo: 0,
        cent_offset: cent_offset(p.tuning_pitch),
        notes: notes.into_iter().map(PakNote::from).collect(),
        chords: [],
        anchors,
        handshapes: [],
        templates: [],
    }
}

/// The song's grid and sections; `None` when the project has neither yet.
fn timeline(p: &Project) -> Option<Timeline> {
    let grid = &p.grid;
    if grid.beats.is_empty() && p.sections.is_empty() {
        return None;
    }
    let mut downbeats: Vec<usize> = grid.downbeats.iter().copied().filter(|&i| i < grid.beats.len()).collect();
    downbeats.sort_unstable();
    downbeats.dedup();

    let mut bar = 0;
    let beats = grid
        .beats
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let measure = if downbeats.binary_search(&i).is_ok() {
                bar += 1;
                bar
            } else {
                -1
            };
            Beat { time: round3(t), measure }
        })
        .collect();

    // Tempo and metre follow the bars; a change is recorded when it is more than jitter.
    let mut tempos: Vec<Tempo> = Vec::new();
    let mut time_signatures: Vec<TimeSignature> = Vec::new();
    for pair in downbeats.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let (start, count) = (grid.beats[from], to - from);
        let seconds = grid.beats[to] - start;
        if seconds <= 0.0 {
            continue;
        }
        let bpm = ((count as f64 * 60.0 / seconds) * 1000.0).round() / 1000.0;
        if tempos.last().is_none_or(|t| (t.bpm - bpm).abs() > 0.5) {
            tempos.push(Tempo { time: round3(start), bpm });
        }
        if time_signatures.last().is_none_or(|t| t.ts[0] != count) {
            time_signatures.push(TimeSignature { time: round3(start), ts: [count, 4] });
        }
    }
    if let (true, Some(&first), true) = (tempos.is_empty(), grid.beats.first(), grid.tempo > 0.0) {
        tempos.push(Tempo { time: round3(first), bpm: grid.tempo });
    }

    let mut sections = Vec::new();
    if !p.sections.is_empty() {
        for (s, number) in p.numbered_sections() {
            sections.push(Section { name: s.name.clone(), number, time: round3(s.time) });
        }
    } else if !downbeats.is_empty() {
        let section = |number, time| Section { name: "section".into(), number, time };
        sections.push(section(1, 0.0));
        for (k, &i) in downbeats.iter().skip(BARS_PER_SECTION).step_by(BARS_PER_SECTION).enumerate() {
            sections.push(section(k as i32 + 2, round3(grid.beats[i])));
        }
    }
    Some(Timeline { version: 1, tempos, time_signatures, beats, sections })
}

/// A YAML string. A JSON string is a valid double-quoted YAML scalar, whatever it holds.
fn yaml_str(s: &str) -> String {
    serde_json::Value::from(s).to_string()
}

fn manifest(p: &Project, cover: Option<&str>, has_timeline: bool) -> String {
    let md = &p.metadata;
    let tuning = p.tuning.map(|t| t.to_string()).join(", ");
    let mut y = format!("feedpak_version: \"{FEEDPAK_VERSION}\"\n");
    let title = if md.title.is_empty() { "Untitled" } else { &md.title };
    let artist = if md.artist.is_empty() { "Unknown" } else { &md.artist };
    y += &format!("title: {}\nartist: {}\n", yaml_str(title), yaml_str(artist));
    if !md.album.is_empty() {
        y += &format!("album: {}\n", yaml_str(&md.album));
    }
    if md.year > 0 {
        y += &format!("year: {}\n", md.year);
    }
    if !md.author.is_empty() {
        y += &format!("authors:\n  - name: {}\n", yaml_str(&md.author));
    }
    y += &format!("duration: {}\n", round3(p.duration));
    if let Some(cover) = cover {
        y += &format!("cover: {cover}\n");
    }
    y += &format!("preview: {PREVIEW_FILE}\n");
    if has_timeline {
        y += &format!("song_timeline: {TIMELINE_FILE}\n");
    }
    y += &format!(
        "arrangements:\n  - id: bass\n    name: Bass\n    file: {ARRANGEMENT_FILE}\n    type: bass\n    tuning: [{tuning}]\n    capo: 0\n"
    );
    let cents = cent_offset(p.tuning_pitch);
    if cents != 0.0 {
        y += &format!("    centOffset: {cents}\n");
    }
    y += &format!("stems:\n  - id: full\n    file: {STEM_FILE}\n    default: true\n");
    y
}

/// The cover for the package as (file name in the package, bytes). JPEG and PNG go in
/// untouched; other images are converted, as the format only allows those two.
fn cover(p: &Project) -> Result<Option<(&'static str, Vec<u8>)>> {
    let path = Path::new(&p.album_art);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    let image_error = |e: image::ImageError| Error::Other(format!("album art {}: {e}", path.display()));
    Ok(Some(match image::guess_format(&bytes).map_err(image_error)? {
        image::ImageFormat::Jpeg => ("cover.jpg", bytes),
        image::ImageFormat::Png => ("cover.png", bytes),
        _ => {
            let img = image::load_from_memory(&bytes).map_err(image_error)?;
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png).map_err(image_error)?;
            ("cover.png", png.into_inner())
        }
    }))
}

/// The song's decoded audio, read at most once however many steps need it.
fn song<'a>(cache: &'a mut Option<fw_audio::Audio>, path: &Path) -> Result<&'a fw_audio::Audio> {
    if cache.is_none() {
        *cache = Some(fw_audio::decode(path)?.to_rocksmith()?);
    }
    cache.as_ref().ok_or_else(|| Error::Other("audio was not decoded".into()))
}

fn add<W: Write + Seek>(zip: &mut ZipWriter<W>, name: &str, bytes: &[u8], method: CompressionMethod) -> Result<()> {
    let zip_error = |e: zip::result::ZipError| Error::Other(format!("writing the package: {e}"));
    zip.start_file(name, SimpleFileOptions::default().compression_method(method)).map_err(zip_error)?;
    zip.write_all(bytes)?;
    Ok(())
}

/// Builds the project's `.feedpak` into `out_dir` (default `<project>/out`).
pub fn build_feedpak(p: &mut Project, out_dir: Option<&Path>, progress: BuildProgress) -> Result<PathBuf> {
    if p.notes.is_empty() {
        return Err(Error::NoNotes);
    }
    let work = p.path("build");
    std::fs::create_dir_all(&work)?;
    let audio_path = PathBuf::from(&p.audio_path);
    let mut audio = None;

    progress("Decoding audio", 0.05);
    if p.duration <= 0.0 {
        p.duration = song(&mut audio, &audio_path)?.duration();
    }
    let rev = fw_audio::REVISION;
    let song_ogg = work.join(format!("song.r{rev}.ogg"));
    if !fresh(&song_ogg, &audio_path) {
        let decoded = song(&mut audio, &audio_path)?;
        progress("Encoding song audio (Ogg Vorbis)", 0.25);
        std::fs::write(&song_ogg, fw_audio::encode_ogg(decoded)?)?;
    }
    let start = match p.preview_start {
        Some(s) => s,
        None => {
            let s = default_preview_start(p);
            p.preview_start = Some(s);
            s
        }
    };
    let preview_ogg = work.join(format!("preview_{start:.2}.r{rev}.ogg"));
    if !fresh(&preview_ogg, &audio_path) {
        let decoded = song(&mut audio, &audio_path)?;
        progress("Encoding preview", 0.7);
        std::fs::write(&preview_ogg, fw_audio::encode_ogg(&decoded.preview(start))?)?;
    }

    progress("Writing package", 0.9);
    let cover = cover(p)?;
    let timeline = timeline(p);
    let manifest = manifest(p, cover.as_ref().map(|c| c.0), timeline.is_some());

    let md = &p.metadata;
    let name = |value: &str, sort: &str, fallback: &str| {
        fw_package::Sortable::new(if value.is_empty() { fallback } else { value }, Some(sort)).sort
    };
    let version = if md.version.is_empty() { "1" } else { &md.version };
    let file = format!(
        "{}_{}_v{}.feedpak",
        fw_package::strings::file_name(&name(&md.artist, &md.artist_sort, "Unknown")),
        fw_package::strings::file_name(&name(&md.title, &md.title_sort, "Untitled")),
        version.replace('.', "_"),
    );
    let out = out_dir.map(Path::to_path_buf).unwrap_or_else(|| p.path("out"));
    std::fs::create_dir_all(&out)?;
    let target = out.join(file);

    // Written next to the target and renamed, so a failed build leaves no broken package.
    let tmp = target.with_extension("feedpak.tmp");
    let mut zip = ZipWriter::new(BufWriter::new(File::create(&tmp)?));
    // Audio and images are compressed already.
    let (text, stored) = (CompressionMethod::Deflated, CompressionMethod::Stored);
    add(&mut zip, "manifest.yaml", manifest.as_bytes(), text)?;
    add(&mut zip, ARRANGEMENT_FILE, &serde_json::to_vec(&arrangement(p))?, text)?;
    if let Some(t) = &timeline {
        add(&mut zip, TIMELINE_FILE, &serde_json::to_vec(t)?, text)?;
    }
    add(&mut zip, STEM_FILE, &std::fs::read(&song_ogg)?, stored)?;
    add(&mut zip, PREVIEW_FILE, &std::fs::read(&preview_ogg)?, stored)?;
    if let Some((name, bytes)) = &cover {
        add(&mut zip, name, bytes, stored)?;
    }
    zip.finish().map_err(|e| Error::Other(format!("writing the package: {e}")))?.flush()?;
    std::fs::rename(&tmp, &target)?;
    progress("Package ready", 1.0);
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn sine_wav(path: &Path, seconds: u32) {
        let rate = 48_000u32;
        let samples: Vec<f32> = (0..rate * seconds).map(|i| (i as f32 * 0.02).sin() * 0.3).collect();
        let audio = fw_audio::Audio { sample_rate: rate, channels: vec![samples.clone(), samples] };
        std::fs::write(path, audio.to_wav()).unwrap();
    }

    fn project(dir: &Path) -> Project {
        let song = dir.join("song.wav");
        sine_wav(&song, 4);
        let mut p = Project::new(dir.join("project"), song.to_string_lossy());
        p.metadata.title = "Say \"Hi\": #1".into();
        p.metadata.artist = "Ünï Cödé".into();
        p.tuning = [-2, 0, 0, 0];
        p.grid.beats = (0..9).map(|i| f64::from(i) * 0.5).collect();
        p.grid.downbeats = vec![0, 4, 8];
        let mut n = Note::new(1.0, 1.5, 33);
        n.string = 1;
        n.bend = 1.0;
        n.hammer_on = true;
        let mut short = Note::new(0.5, 0.6, 28);
        short.fret = 2;
        p.notes = vec![n, short];
        p
    }

    fn member(zip: &mut zip::ZipArchive<File>, name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn writes_a_complete_package() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project(dir.path());
        let path = build_feedpak(&mut p, None, &|_, _| {}).unwrap();
        assert_eq!(path.extension().unwrap(), "feedpak");
        assert!(!path.with_extension("feedpak.tmp").exists());

        let mut zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let manifest = String::from_utf8(member(&mut zip, "manifest.yaml")).unwrap();
        assert!(manifest.contains("title: \"Say \\\"Hi\\\": #1\"\n"), "{manifest}");
        assert!(manifest.contains("tuning: [-2, 0, 0, 0]\n"), "{manifest}");
        assert!(manifest.contains("duration: 4\n"), "{manifest}");
        assert!(!manifest.contains("cover:"), "no album art was set: {manifest}");

        let chart: serde_json::Value = serde_json::from_slice(&member(&mut zip, ARRANGEMENT_FILE)).unwrap();
        let notes = chart["notes"].as_array().unwrap();
        // in time order, a short note without sustain, bends in semitones
        assert_eq!(notes[0], serde_json::json!({"t": 0.5, "s": 0, "f": 2}));
        assert_eq!(notes[1], serde_json::json!({"t": 1.0, "s": 1, "f": 0, "sus": 0.5, "bn": 2.0, "ho": true}));

        let timeline: serde_json::Value = serde_json::from_slice(&member(&mut zip, TIMELINE_FILE)).unwrap();
        assert_eq!(timeline["beats"][4], serde_json::json!({"time": 2.0, "measure": 2}));
        assert_eq!(timeline["beats"][1]["measure"], -1);
        assert_eq!(timeline["tempos"], serde_json::json!([{"time": 0.0, "bpm": 120.0}]));
        assert_eq!(timeline["time_signatures"], serde_json::json!([{"time": 0.0, "ts": [4, 4]}]));

        for audio in [STEM_FILE, PREVIEW_FILE] {
            assert_eq!(&member(&mut zip, audio)[..4], b"OggS", "{audio}");
        }
    }

    #[test]
    fn converts_art_the_format_does_not_allow() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project(dir.path());
        let art = dir.path().join("art.bmp");
        image::RgbImage::new(8, 8).save(&art).unwrap();
        p.album_art = art.to_string_lossy().into_owned();
        let path = build_feedpak(&mut p, None, &|_, _| {}).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        assert!(String::from_utf8(member(&mut zip, "manifest.yaml")).unwrap().contains("cover: cover.png\n"));
        assert_eq!(image::guess_format(&member(&mut zip, "cover.png")).unwrap(), image::ImageFormat::Png);
    }

    #[test]
    fn a_project_without_a_grid_has_no_timeline() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project(dir.path());
        p.grid = crate::BeatGrid::default();
        let path = build_feedpak(&mut p, None, &|_, _| {}).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        assert!(!String::from_utf8(member(&mut zip, "manifest.yaml")).unwrap().contains("song_timeline"));
        assert!(zip.by_name(TIMELINE_FILE).is_err());
    }

    #[test]
    fn a_grid_without_bars_still_lists_its_sections() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project(dir.path());
        p.grid.downbeats.clear();
        let t = serde_json::to_value(timeline(&p).unwrap()).unwrap();
        assert_eq!(t["sections"], serde_json::json!([]));
        assert_eq!(t["tempos"], serde_json::json!([{"time": 0.0, "bpm": 120.0}]));
    }

    #[test]
    fn marked_sections_replace_the_automatic_ones() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project(dir.path());
        let auto = serde_json::to_value(timeline(&p).unwrap()).unwrap();
        assert_eq!(auto["sections"], serde_json::json!([{"name": "section", "number": 1, "time": 0.0}]));
        p.set_section(0.5, "chorus");
        p.set_section(2.0, "verse");
        p.set_section(3.0, "chorus");
        let marked = serde_json::to_value(timeline(&p).unwrap()).unwrap();
        assert_eq!(
            marked["sections"],
            serde_json::json!([
                {"name": "chorus", "number": 1, "time": 0.5},
                {"name": "verse", "number": 1, "time": 2.0},
                {"name": "chorus", "number": 2, "time": 3.0},
            ])
        );
        p.grid = crate::BeatGrid::default();
        let no_grid = serde_json::to_value(timeline(&p).unwrap()).unwrap();
        assert_eq!(no_grid["sections"].as_array().unwrap().len(), 3);
        assert_eq!(no_grid["beats"], serde_json::json!([]));
    }

    #[test]
    fn needs_notes() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project(dir.path());
        p.notes.clear();
        assert!(matches!(build_feedpak(&mut p, None, &|_, _| {}), Err(Error::NoNotes)));
    }
}
