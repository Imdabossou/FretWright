//! Rocksmith 2014 instrumental arrangement XML: the editable chart format used
//! by the Custom Song Toolkit, EOF and DLC Builder.
//!
//! All times are integer **milliseconds** (see [`time`]). The model mirrors
//! Rocksmith2014.NET's `Rocksmith2014.XML` (MIT, Tapio Malmberg) so the
//! conversion and dynamic-difficulty ports can follow it closely.
//!
//! ```no_run
//! # fn main() -> Result<(), fw_xml::Error> {
//! let mut arr = fw_xml::InstrumentalArrangement::load("bass.xml")?;
//! println!("{} beats, {} levels", arr.ebeats.len(), arr.levels.len());
//! arr.metadata.title = "New title".into();
//! arr.save("bass_edited.xml")?;
//! # Ok(()) }
//! ```

mod read;
pub mod time;
mod write;

use bitflags::bitflags;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("XML syntax error: {0}")]
    Syntax(String),
    #[error("not an instrumental arrangement (root element is <{0}>)")]
    NotAnArrangement(String),
    #[error("Rocksmith 1 arrangements (version {0}) are not supported")]
    UnsupportedVersion(u8),
    #[error("invalid value {value:?} for {what}")]
    Invalid { what: String, value: String },
}

pub type Result<T> = std::result::Result<T, Error>;

bitflags! {
    /// Single-note techniques stored as attributes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct NoteMask: u16 {
        const LINK_NEXT = 1 << 0;
        const ACCENT = 1 << 1;
        const HAMMER_ON = 1 << 2;
        const HARMONIC = 1 << 3;
        const IGNORE = 1 << 4;
        const FRET_HAND_MUTE = 1 << 5;
        const PALM_MUTE = 1 << 6;
        const PULL_OFF = 1 << 7;
        const TREMOLO = 1 << 8;
        const PINCH_HARMONIC = 1 << 9;
        const PICK_DIRECTION = 1 << 10;
        const SLAP = 1 << 11;
        const PLUCK = 1 << 12;
        const RIGHT_HAND = 1 << 13;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct ChordMask: u8 {
        const LINK_NEXT = 1 << 0;
        const ACCENT = 1 << 1;
        const FRET_HAND_MUTE = 1 << 2;
        const HIGH_DENSITY = 1 << 3;
        const IGNORE = 1 << 4;
        const PALM_MUTE = 1 << 5;
        const HOPO = 1 << 6;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct PhraseMask: u8 {
        const DISPARITY = 1 << 0;
        const IGNORE = 1 << 1;
        const SOLO = 1 << 2;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BendValue {
    pub time: i32,
    pub step: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub time: i32,
    pub sustain: i32,
    pub string: i8,
    pub fret: i8,
    /// Fret-hand finger (-1 = unset).
    pub left_hand: i8,
    /// -1 = no slide.
    pub slide_to: i8,
    /// -1 = no unpitched slide.
    pub slide_unpitch_to: i8,
    pub tap: i8,
    pub vibrato: u8,
    pub mask: NoteMask,
    pub max_bend: f32,
    pub bend_values: Option<Vec<BendValue>>,
}

impl Default for Note {
    fn default() -> Self {
        Note {
            time: 0,
            sustain: 0,
            string: 0,
            fret: 0,
            left_hand: -1,
            slide_to: -1,
            slide_unpitch_to: -1,
            tap: 0,
            vibrato: 0,
            mask: NoteMask::empty(),
            max_bend: 0.0,
            bend_values: None,
        }
    }
}

impl Note {
    pub fn is_bend(&self) -> bool {
        self.bend_values.as_ref().is_some_and(|b| !b.is_empty())
    }
    pub fn is_slide(&self) -> bool {
        self.slide_to != -1
    }
    pub fn is_unpitched_slide(&self) -> bool {
        self.slide_unpitch_to != -1
    }
    pub fn is_hopo(&self) -> bool {
        self.mask.intersects(NoteMask::HAMMER_ON | NoteMask::PULL_OFF)
    }
    pub fn is_tap(&self) -> bool {
        self.tap != 0
    }
    pub fn is_vibrato(&self) -> bool {
        self.vibrato != 0
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Chord {
    pub time: i32,
    pub chord_id: i16,
    pub mask: ChordMask,
    pub chord_notes: Option<Vec<Note>>,
}

impl Chord {
    pub fn has_chord_notes(&self) -> bool {
        self.chord_notes.as_ref().is_some_and(|c| !c.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChordTemplate {
    pub name: String,
    pub display_name: String,
    /// Fingers per string, low to high (-1 = unused).
    pub fingers: [i8; 6],
    /// Frets per string, low to high (-1 = not played).
    pub frets: [i8; 6],
}

impl Default for ChordTemplate {
    fn default() -> Self {
        ChordTemplate { name: String::new(), display_name: String::new(), fingers: [-1; 6], frets: [-1; 6] }
    }
}

impl ChordTemplate {
    pub fn is_arpeggio(&self) -> bool {
        self.display_name.ends_with("-arp")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ebeat {
    pub time: i32,
    /// Bar number on the first beat of a bar, otherwise -1.
    pub measure: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Phrase {
    pub name: String,
    pub max_difficulty: u8,
    pub mask: PhraseMask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeroLevels {
    pub easy: u8,
    pub medium: u8,
    pub hard: u8,
}

impl HeroLevels {
    pub fn any(&self) -> bool {
        self.easy > 0 || self.medium > 0 || self.hard > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhraseIteration {
    pub time: i32,
    pub phrase_id: i32,
    /// Empty when unset.
    pub variation: String,
    pub hero_levels: HeroLevels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLinkedDiff {
    pub level_break: i8,
    pub ratio: Option<String>,
    pub phrase_ids: Vec<i32>,
}

impl Default for NewLinkedDiff {
    fn default() -> Self {
        NewLinkedDiff { level_break: -1, ratio: None, phrase_ids: Vec::new() }
    }
}

/// Leftover from Rocksmith 1, found in some early 2014 files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkedDiff {
    pub child_id: i32,
    pub parent_id: i32,
}

/// Leftover from Rocksmith 1, found in some early 2014 files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhraseProperty {
    pub phrase_id: i32,
    pub redundant: i16,
    pub level_jump: i8,
    pub empty: i32,
    pub difficulty: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub name: String,
    pub number: i16,
    pub time: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    pub time: i32,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToneChange {
    pub time: i32,
    pub id: u8,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToneInfo {
    pub base: Option<String>,
    /// Tones A-D.
    pub names: [Option<String>; 4],
    pub changes: Vec<ToneChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Anchor {
    pub time: i32,
    pub fret: i8,
    pub width: i8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HandShape {
    pub chord_id: i16,
    pub start_time: i32,
    pub end_time: i32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Level {
    pub difficulty: i8,
    pub notes: Vec<Note>,
    pub chords: Vec<Chord>,
    pub anchors: Vec<Anchor>,
    pub hand_shapes: Vec<HandShape>,
}

/// Arrangement-wide flags shown in the game's arrangement details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArrangementProperties {
    pub represent: bool,
    pub bonus_arrangement: bool,
    pub standard_tuning: bool,
    pub non_standard_chords: bool,
    pub barre_chords: bool,
    pub power_chords: bool,
    pub drop_d_power: bool,
    pub open_chords: bool,
    pub finger_picking: bool,
    pub pick_direction: bool,
    pub double_stops: bool,
    pub palm_mutes: bool,
    pub harmonics: bool,
    pub pinch_harmonics: bool,
    pub hopo: bool,
    pub tremolo: bool,
    pub slides: bool,
    pub unpitched_slides: bool,
    pub bends: bool,
    pub tapping: bool,
    pub vibrato: bool,
    pub fret_hand_mutes: bool,
    pub slap_pop: bool,
    pub two_finger_picking: bool,
    pub fifths_and_octaves: bool,
    pub syncopation: bool,
    pub bass_pick: bool,
    pub sustain: bool,
    pub path_lead: bool,
    pub path_rhythm: bool,
    pub path_bass: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetaData {
    pub title: String,
    pub title_sort: Option<String>,
    pub arrangement: String,
    pub part: i16,
    pub cent_offset: f32,
    pub song_length: i32,
    pub average_tempo: f32,
    /// Semitone offsets from E standard, low string first.
    pub tuning: [i16; 6],
    pub capo: i8,
    pub artist_name: String,
    pub artist_name_sort: Option<String>,
    pub album_name: String,
    pub album_name_sort: Option<String>,
    pub album_year: i32,
    pub album_art: Option<String>,
    pub arrangement_properties: ArrangementProperties,
    pub last_conversion_date_time: String,
}

impl Default for MetaData {
    fn default() -> Self {
        MetaData {
            title: String::new(),
            title_sort: None,
            arrangement: String::new(),
            part: 0,
            cent_offset: 0.0,
            song_length: 0,
            average_tempo: 120.0,
            tuning: [0; 6],
            capo: 0,
            artist_name: String::new(),
            artist_name_sort: None,
            album_name: String::new(),
            album_name_sort: None,
            album_year: 0,
            album_art: None,
            arrangement_properties: ArrangementProperties::default(),
            last_conversion_date_time: String::new(),
        }
    }
}

/// A Rocksmith 2014 instrumental (lead, rhythm, combo or bass) arrangement.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentalArrangement {
    /// File version; 8 for current files (7 is accepted).
    pub version: u8,
    /// Comments directly after the root element (tool signatures etc.).
    pub comments: Vec<String>,
    pub metadata: MetaData,
    pub phrases: Vec<Phrase>,
    pub phrase_iterations: Vec<PhraseIteration>,
    pub new_linked_diffs: Vec<NewLinkedDiff>,
    pub linked_diffs: Option<Vec<LinkedDiff>>,
    pub phrase_properties: Option<Vec<PhraseProperty>>,
    pub chord_templates: Vec<ChordTemplate>,
    pub ebeats: Vec<Ebeat>,
    pub tones: ToneInfo,
    pub sections: Vec<Section>,
    pub events: Vec<Event>,
    pub transcription_track: Option<Level>,
    pub levels: Vec<Level>,
}

impl Default for InstrumentalArrangement {
    fn default() -> Self {
        InstrumentalArrangement {
            version: 8,
            comments: Vec::new(),
            metadata: MetaData::default(),
            phrases: Vec::new(),
            phrase_iterations: Vec::new(),
            new_linked_diffs: Vec::new(),
            linked_diffs: None,
            phrase_properties: None,
            chord_templates: Vec::new(),
            ebeats: Vec::new(),
            tones: ToneInfo::default(),
            sections: Vec::new(),
            events: Vec::new(),
            transcription_track: None,
            levels: Vec::new(),
        }
    }
}

impl InstrumentalArrangement {
    /// Time of the first beat (ms).
    pub fn start_beat(&self) -> i32 {
        self.ebeats.first().map_or(0, |b| b.time)
    }

    /// Parses an arrangement from XML text.
    pub fn parse(xml: &str) -> Result<Self> {
        read::arrangement(xml)
    }

    /// Loads an arrangement file.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    /// Serialises to XML. Abridged output omits attributes at their default
    /// values (what DLC Builder writes); full output includes every attribute.
    pub fn to_xml(&self, abridged: bool) -> String {
        write::arrangement(self, abridged)
    }

    /// Saves as abridged XML.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        std::fs::write(path, self.to_xml(true))?;
        Ok(())
    }
}
