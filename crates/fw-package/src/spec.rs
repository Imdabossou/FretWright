//! What a package is built from.

use std::path::PathBuf;

use crate::strings::Sortable;

pub const BASE_TONE_KEY: &str = "Default_Bass";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangementName {
    Lead,
    Combo,
    Rhythm,
    Bass,
}

impl ArrangementName {
    pub fn as_str(self) -> &'static str {
        match self {
            ArrangementName::Lead => "Lead",
            ArrangementName::Combo => "Combo",
            ArrangementName::Rhythm => "Rhythm",
            ArrangementName::Bass => "Bass",
        }
    }
    pub fn type_id(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Main,
    Alternative,
    Bonus,
}

#[derive(Debug, Clone)]
pub struct ArrangementSpec {
    /// The arrangement as charted (single level is fine; phrases, fixes and
    /// dynamic difficulty are generated during the build).
    pub xml: fw_xml::InstrumentalArrangement,
    pub name: ArrangementName,
    pub priority: Priority,
    pub bass_picked: bool,
    pub scroll_speed: f64,
    /// Semitone offsets from E standard, low string first.
    pub tuning: [i16; 6],
    pub tuning_pitch: f64,
    /// Stable across rebuilds so the game keeps the player's stats ("8-4-4-4-12" UUID).
    pub persistent_id: String,
    pub master_id: i32,
}

impl ArrangementSpec {
    pub fn route_mask(&self) -> i32 {
        match self.name {
            ArrangementName::Bass => 4,
            ArrangementName::Rhythm => 2,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PackageSpec {
    /// Alphanumeric key that identifies the song in the game.
    pub dlc_key: String,
    pub version: String,
    pub author: String,
    pub artist: Sortable,
    pub title: Sortable,
    pub album: Sortable,
    pub year: i32,
    /// Album art image (jpg, png, webp, bmp, gif); None = a plain placeholder.
    pub album_art: Option<PathBuf>,
    /// Encoded .wem files (see fw-audio).
    pub audio_wem: Vec<u8>,
    pub preview_wem: Vec<u8>,
    /// Rocksmith volume (dB) for the song and the preview.
    pub volume: f32,
    pub preview_volume: f32,
    pub arrangements: Vec<ArrangementSpec>,
    /// "Toolkit version" line written into the package.
    pub builder_name: String,
    pub app_id: u64,
}
