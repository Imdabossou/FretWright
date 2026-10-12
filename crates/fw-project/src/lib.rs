//! A FretWright project: a folder with `project.fretwright.json` (song metadata,
//! the bass chart, the beat grid, detection settings and stable IDs) plus
//! cached intermediate files.

mod arrangement;
mod build;
pub mod import;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use arrangement::to_arrangement;
pub use build::{BuildProgress, build, package_spec};

pub const PROJECT_FILE: &str = "project.fretwright.json";
pub const FORMAT_VERSION: u32 = 1;

/// Open-string MIDI notes of a 4-string bass in E standard (E1 A1 D2 G2).
pub const STANDARD_BASS: [i32; 4] = [28, 33, 38, 43];
pub const MAX_FRET: i32 = 24;

/// Named tunings, as offsets from E standard (low string first).
pub const TUNINGS: &[(&str, [i32; 4])] = &[
    ("E Standard", [0, 0, 0, 0]),
    ("Drop D", [-2, 0, 0, 0]),
    ("Eb Standard", [-1, -1, -1, -1]),
    ("Drop C#", [-3, -1, -1, -1]),
    ("D Standard", [-2, -2, -2, -2]),
    ("Drop C", [-4, -2, -2, -2]),
    ("C# Standard", [-3, -3, -3, -3]),
    ("C Standard", [-4, -4, -4, -4]),
];

/// "Artist - Title [junk].mp3" -> (artist, title)
pub fn guess_metadata(path: &Path) -> (String, String) {
    let base = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut clean = String::new();
    let mut depth = 0i32;
    for c in base.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => clean.push(c),
            _ => {}
        }
    }
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    match clean.split_once(" - ") {
        Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
        None => (String::new(), clean),
    }
}

/// The default project folder for a song: `<song dir>/<song name> (FretWright)`.
pub fn default_folder(song: &Path) -> PathBuf {
    let base = song.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = base.chars().filter(|c| c.is_alphanumeric() || " -_".contains(*c)).collect();
    song.with_file_name(format!("{} (FretWright)", safe.trim()))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid project file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Audio(#[from] fw_audio::Error),
    #[error("{0}")]
    Package(#[from] fw_package::Error),
    #[error("the chart has no notes yet; run detection first")]
    NoNotes,
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn default_true_one() -> f64 {
    1.0
}
fn minus_one() -> i32 {
    -1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub midi: i32,
    #[serde(default)]
    pub string: i32,
    #[serde(default)]
    pub fret: i32,
    /// Detection confidence 0..1 (1 for notes placed by hand).
    #[serde(default = "default_true_one")]
    pub confidence: f64,
    /// Moved by an octave to fit the instrument's range.
    #[serde(default)]
    pub shifted: bool,
    #[serde(default = "minus_one")]
    pub slide_to: i32,
    #[serde(default)]
    pub hammer_on: bool,
    #[serde(default)]
    pub pull_off: bool,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub accent: bool,
    /// Slide out to nowhere in particular (unpitched), ending on this fret; -1 = none.
    #[serde(default = "minus_one")]
    pub slide_unpitch_to: i32,
    /// Bend height in whole steps (0 = no bend).
    #[serde(default)]
    pub bend: f32,
    #[serde(default)]
    pub harmonic: bool,
    #[serde(default)]
    pub palm_mute: bool,
    #[serde(default)]
    pub tremolo: bool,
    #[serde(default)]
    pub slap: bool,
    /// Popped (plucked) string.
    #[serde(default)]
    pub pop: bool,
    #[serde(default)]
    pub tap: bool,
    #[serde(default)]
    pub vibrato: bool,
    /// Tied into the next note (the slide or sustain carries on).
    #[serde(default)]
    pub link_next: bool,
}

impl Note {
    pub fn new(start: f64, end: f64, midi: i32) -> Note {
        Note {
            start,
            end,
            midi,
            string: 0,
            fret: 0,
            confidence: 1.0,
            shifted: false,
            slide_to: -1,
            hammer_on: false,
            pull_off: false,
            mute: false,
            accent: false,
            slide_unpitch_to: -1,
            bend: 0.0,
            harmonic: false,
            palm_mute: false,
            tremolo: false,
            slap: false,
            pop: false,
            tap: false,
            vibrato: false,
            link_next: false,
        }
    }
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }

    /// Names of the techniques the note carries, in a fixed order.
    pub fn technique_names(&self) -> Vec<&'static str> {
        [
            ("hammer-on", self.hammer_on),
            ("pull-off", self.pull_off),
            ("slide", self.slide_to >= 0),
            ("slide-out", self.slide_unpitch_to >= 0),
            ("bend", self.bend > 0.0),
            ("harmonic", self.harmonic),
            ("palm mute", self.palm_mute),
            ("fret-hand mute", self.mute),
            ("accent", self.accent),
            ("tremolo", self.tremolo),
            ("vibrato", self.vibrato),
            ("slap", self.slap),
            ("pop", self.pop),
            ("tap", self.tap),
            ("link next", self.link_next),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect()
    }

    /// Whether the note carries any technique mark.
    pub fn has_technique(&self) -> bool {
        self.hammer_on
            || self.pull_off
            || self.mute
            || self.accent
            || self.slide_to >= 0
            || self.slide_unpitch_to >= 0
            || self.bend > 0.0
            || self.harmonic
            || self.palm_mute
            || self.tremolo
            || self.slap
            || self.pop
            || self.tap
            || self.vibrato
            || self.link_next
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeatGrid {
    #[serde(default)]
    pub beats: Vec<f64>,
    /// Indices into `beats` that start a bar.
    #[serde(default)]
    pub downbeats: Vec<usize>,
    #[serde(default = "default_tempo")]
    pub tempo: f64,
}

fn default_tempo() -> f64 {
    120.0
}

impl Default for BeatGrid {
    fn default() -> Self {
        BeatGrid { beats: Vec::new(), downbeats: Vec::new(), tempo: 120.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: i32,
    pub artist_sort: String,
    pub title_sort: String,
    pub album_sort: String,
    pub author: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectionSettings {
    pub separator: String,
    pub pitch: String,
    pub beats: String,
    pub min_note_ms: f64,
    pub snap_division: u32,
    pub device: String,
}

impl Default for DetectionSettings {
    fn default() -> Self {
        DetectionSettings {
            separator: "htdemucs_ft".into(),
            pitch: "pyin".into(),
            beats: "beat_this".into(),
            min_note_ms: 60.0,
            snap_division: 4,
            device: "auto".into(),
        }
    }
}

/// IDs that must stay stable across rebuilds so the game keeps the player's stats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    #[serde(default)]
    pub dlc_key: String,
    pub persistent_id: String,
    pub master_id: i32,
}

impl Identity {
    /// Fresh random-ish IDs (time and process based; uniqueness is what matters).
    pub fn generate() -> Identity {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let mut x = (nanos as u64) ^ ((std::process::id() as u64) << 32) ^ 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let (a, b) = (next(), next());
        Identity {
            dlc_key: String::new(),
            persistent_id: format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                a >> 32,
                (a >> 16) & 0xffff,
                a & 0xfff,
                ((b >> 48) & 0x3fff) | 0x8000,
                b & 0xffff_ffff_ffff
            ),
            master_id: (100_000_000 + next() % 1_900_000_000) as i32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// The project folder (not stored in the file).
    #[serde(skip)]
    pub folder: PathBuf,
    #[serde(default)]
    pub audio_path: String,
    #[serde(default)]
    pub album_art: String,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default = "standard_tuning")]
    pub tuning: [i32; 4],
    #[serde(default = "default_pitch")]
    pub tuning_pitch: f64,
    #[serde(default)]
    pub bass_picked: bool,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub preview_start: Option<f64>,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub grid: BeatGrid,
    #[serde(default)]
    pub detection: DetectionSettings,
    pub identity: Identity,
}

fn standard_tuning() -> [i32; 4] {
    [0; 4]
}
fn default_pitch() -> f64 {
    440.0
}

#[derive(Serialize)]
struct OnDisk<'a> {
    #[serde(flatten)]
    project: &'a Project,
    format: u32,
}

impl Project {
    pub fn new(folder: impl Into<PathBuf>, audio_path: impl Into<String>) -> Project {
        Project {
            folder: folder.into(),
            audio_path: audio_path.into(),
            album_art: String::new(),
            metadata: Metadata { year: 2026, version: "1".into(), ..Default::default() },
            tuning: [0; 4],
            tuning_pitch: 440.0,
            bass_picked: false,
            duration: 0.0,
            preview_start: None,
            volume: None,
            notes: Vec::new(),
            grid: BeatGrid::default(),
            detection: DetectionSettings::default(),
            identity: Identity::generate(),
        }
    }

    /// Open-string MIDI notes for the project's tuning.
    pub fn open_strings(&self) -> [i32; 4] {
        std::array::from_fn(|i| STANDARD_BASS[i] + self.tuning[i])
    }

    pub fn path(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.folder.join(rel)
    }

    /// The DLC key, derived from artist and title the first time it is needed.
    pub fn ensure_dlc_key(&mut self) -> String {
        if self.identity.dlc_key.is_empty() {
            let part = |s: &str| {
                let clean: String = s.chars().filter(char::is_ascii_alphanumeric).take(8).collect();
                let mut c = clean.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            };
            let key = format!("Rsb{}{}", part(&self.metadata.artist), part(&self.metadata.title));
            self.identity.dlc_key = if key.len() >= 5 { key } else { key + "Song" };
        }
        self.identity.dlc_key.clone()
    }

    /// Loads a project from its folder or its project file.
    pub fn load(path: impl AsRef<Path>) -> Result<Project> {
        let path = path.as_ref();
        let file = if path.is_dir() { path.join(PROJECT_FILE) } else { path.to_path_buf() };
        let mut p: Project = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
        p.folder = file.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(p)
    }

    /// Saves atomically (write a temporary file, then rename).
    pub fn save(&self) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.folder)?;
        let target = self.folder.join(PROJECT_FILE);
        let tmp = target.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&OnDisk { project: self, format: FORMAT_VERSION })?)?;
        std::fs::rename(&tmp, &target)?;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_folder_is_named_after_the_song() {
        assert_eq!(default_folder(Path::new("/m/Artist - Song!.mp3")), Path::new("/m/Artist - Song (FretWright)"));
    }
}
