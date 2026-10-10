//! Manifest (JSON) and manifest header (HSAN) attributes (port of
//! Manifest/AttributesCreation.fs). Field order, number types and rounding
//! follow DLC Builder so the files match its output.

use std::collections::{BTreeSet, HashMap};

use fw_sng::{self as sng, Sng};

use crate::json::J;
use crate::spec::{ArrangementSpec, PackageSpec};
use crate::techniques;
use crate::Error;

/// Declaration order of the attributes; absent ones are skipped.
const ORDER: &[&str] = &[
    "AlbumArt", "AlbumName", "AlbumNameSort", "ArrangementName", "ArrangementProperties", "ArrangementSort",
    "ArrangementType", "ArtistName", "ArtistNameSort", "BassPick", "BlockAsset", "CapoFret", "CentOffset", "Chords",
    "ChordTemplates", "DLC", "DLCKey", "DNA_Chords", "DNA_Riffs", "DNA_Solo", "DynamicVisualDensity", "EasyMastery",
    "FullName", "InputEvent", "JapaneseArtistName", "JapaneseSongName", "JapaneseVocal", "LastConversionDateTime",
    "LeaderboardChallengeRating", "ManifestUrn", "MasterID_PS3", "MasterID_RDV", "MasterID_XBox360",
    "MaxPhraseDifficulty", "MediumMastery", "NotesEasy", "NotesHard", "NotesMedium", "PersistentID",
    "PhraseIterations", "Phrases", "PreviewBankPath", "RelativeDifficulty", "Representative", "RouteMask",
    "Score_MaxNotes", "Score_PNV", "Sections", "Shipping", "ShowlightsXML", "SKU", "SongAsset", "SongAverageTempo",
    "SongBank", "SongDiffEasy", "SongDiffHard", "SongDifficulty", "SongDiffMed", "SongEvent", "SongKey", "SongLength",
    "SongName", "SongNameSort", "SongOffset", "SongPartition", "SongXml", "SongYear", "TargetScore", "Techniques",
    "Tone_A", "Tone_B", "Tone_Base", "Tone_C", "Tone_D", "Tone_Multiplayer", "Tones", "Tuning",
];

struct Attrs(HashMap<&'static str, J>);

impl Attrs {
    fn new() -> Attrs {
        let mut a = Attrs(HashMap::new());
        // non-nullable members with constant defaults
        a.set("DLC", true);
        a.set("LeaderboardChallengeRating", 0);
        a.set("Shipping", true);
        a.set("SKU", "RS2");
        a
    }
    fn set(&mut self, k: &'static str, v: impl Into<J>) {
        self.0.insert(k, v.into());
    }
    fn into_json(mut self) -> J {
        let mut out = J::obj();
        for k in ORDER {
            if let Some(v) = self.0.remove(k) {
                out = out.with(k, v);
            }
        }
        out
    }
}

/// Math.Round(v, digits) with .NET's default (round half to even).
fn round_even(v: f64, digits: i32) -> f64 {
    let p = 10f64.powi(digits);
    (v * p).round_ties_even() / p
}

/// Sum of all DNA-id -> DNA-none ranges (iteration version 2 rules), in single precision.
fn dna_time(sng: &Sng, id: i32) -> f64 {
    fn total(sng: &Sng, id: i32, i: usize, time: f32) -> f32 {
        let Some(next) = sng.dnas.iter().skip(i).position(|d| d.dna_id == id).map(|p| p + i) else { return time };
        match sng.dnas.iter().skip(next).position(|d| d.dna_id == sng::dna::NONE).map(|p| p + next) {
            None => time + (sng.metadata.song_length - sng.dnas[next].time),
            Some(none) => time + total(sng, id, none, sng.dnas[none].time - sng.dnas[next].time),
        }
    }
    round_even(total(sng, id, 0, 0.0) as f64, 3)
}

fn difficulties(arr: &fw_xml::InstrumentalArrangement, sng: &Sng, counts: &fw_convert::NoteCounts) -> (f64, f64, f64) {
    let p = &arr.metadata.arrangement_properties;
    let b = |v: bool| v as i32;
    let mut tech = b(p.non_standard_chords)
        + 3 * b(p.barre_chords)
        + (b(p.power_chords) | b(p.double_stops))
        + b(p.drop_d_power)
        + 2 * b(p.open_chords)
        + b(p.finger_picking)
        + b(p.two_finger_picking)
        + b(p.palm_mutes)
        + 2 * b(p.harmonics)
        + 3 * b(p.pinch_harmonics)
        + b(p.hopo)
        + b(p.tremolo)
        + if p.path_bass { 4 } else { 1 } * b(p.slides)
        + b(p.unpitched_slides)
        + 3 * b(p.bends)
        + 4 * b(p.tapping)
        + 2 * b(p.vibrato)
        + b(p.fret_hand_mutes)
        + b(p.slap_pop)
        + b(p.sustain)
        + b(p.fifths_and_octaves)
        + b(p.syncopation);
    // arrangements with few techniques get very low values otherwise
    if tech <= 5 {
        tech += 4;
    }
    let len = sng.metadata.song_length as f64;
    let d = |count: i32, div: f64| round_even((tech * count) as f64 / len / div, 9);
    (d(counts.hard, 100.0), d(counts.medium, 50.0), d(counts.easy, 25.0))
}

fn section_ui_name(name: &str) -> Result<&'static str, Error> {
    let key: String = name.to_lowercase().chars().filter(|c| c.is_alphabetic()).collect();
    Ok(match key.as_str() {
        "fadein" => "$[34276] Fade In [1]",
        "fadeout" => "$[34277] Fade Out [1]",
        "buildup" => "$[34278] Buildup [1]",
        "chorus" => "$[34279] Chorus [1]",
        "hook" => "$[34280] Hook [1]",
        "head" => "$[34281] Head [1]",
        "bridge" => "$[34282] Bridge [1]",
        "ambient" => "$[34283] Ambient [1]",
        "breakdown" => "$[34284] Breakdown [1]",
        "interlude" => "$[34285] Interlude [1]",
        "intro" => "$[34286] Intro [1]",
        "melody" => "$[34287] Melody [1]",
        "modbridge" => "$[34288] Modulated Bridge [1]",
        "modchorus" => "$[34289] Modulated Chorus [1]",
        "modverse" => "$[34290] Modulated Verse [1]",
        "outro" => "$[34291] Outro [1]",
        "postbrdg" => "$[34292] Post Bridge [1]",
        "postchorus" => "$[34293] Post Chorus [1]",
        "postvs" => "$[34294] Post Verse [1]",
        "prebrdg" => "$[34295] Pre Bridge [1]",
        "prechorus" => "$[34296] Pre Chorus [1]",
        "preverse" => "$[34297] Pre Verse [1]",
        "riff" | "rifff" => "$[34298] Riff [1]",
        "silence" => "$[34299] Silence [1]",
        "solo" => "$[34300] Solo [1]",
        "transition" => "$[34301] Transition [1]",
        "vamp" => "$[34302] Vamp [1]",
        "variation" => "$[34303] Variation [1]",
        "verse" => "$[34304] Verse [1]",
        "tapping" => "$[34305] Tapping [1]",
        "noguitar" => "$[6091] No Guitar [1]",
        _ => return Err(Error::UnknownSection(name.to_string())),
    })
}

fn dynamic_visual_density(levels: usize, scroll_speed: f64) -> Vec<f32> {
    let end = scroll_speed.clamp(0.5, 5.0);
    let begin = 5.0;
    let max_level = levels.min(20) as i64 - 1;
    let factor = if max_level > 0 { (end / begin).powf(1.0 / max_level as f64) } else { 1.0 };
    (0..20)
        .map(|i| if i >= max_level { end as f32 } else { round_even(begin * factor.powi(i as i32), 1) as f32 })
        .collect()
}

fn chord_map(sng: &Sng) -> J {
    let mut out = J::obj();
    for (lvl, level) in sng.levels.iter().enumerate() {
        let mut per_pi = J::obj();
        let mut any = false;
        for (i, pi) in sng.phrase_iterations.iter().enumerate() {
            let ids: BTreeSet<i32> = level
                .hand_shapes
                .iter()
                .filter(|hs| {
                    sng.chords.get(hs.chord_id as usize).is_some_and(|c| !c.name.0.is_empty())
                        && hs.start_time >= pi.start_time
                        && hs.start_time < pi.end_time
                })
                .map(|hs| hs.chord_id)
                .collect();
            if !ids.is_empty() {
                per_pi = per_pi.with(&i.to_string(), ids.into_iter().collect::<Vec<_>>());
                any = true;
            }
        }
        if any {
            out = out.with(&lvl.to_string(), per_pi);
        }
    }
    out
}

fn technique_map(sng: &Sng) -> J {
    let mut out = J::obj();
    for (lvl, level) in sng.levels.iter().enumerate() {
        let mut per_pi = J::obj();
        let mut any = false;
        for i in 0..sng.phrase_iterations.len().saturating_sub(1) {
            let pi = &sng.phrase_iterations[i];
            // the odd (start, end] split matches official files
            let ids: BTreeSet<i32> = level
                .notes
                .iter()
                .filter(|n| n.time > pi.start_time && n.time <= pi.end_time)
                .flat_map(|n| techniques::techniques(sng, n))
                .collect();
            if !ids.is_empty() {
                per_pi = per_pi.with(&i.to_string(), ids.into_iter().collect::<Vec<_>>());
                any = true;
            }
        }
        if any {
            out = out.with(&lvl.to_string(), per_pi);
        }
    }
    out
}

pub struct Built<'a> {
    pub spec: &'a PackageSpec,
    pub arr_spec: &'a ArrangementSpec,
    /// The arrangement as given (its metadata is used, like DLC Builder re-reading the XML).
    pub source: &'a fw_xml::InstrumentalArrangement,
    pub sng: &'a Sng,
    pub counts: fw_convert::NoteCounts,
    pub part_name: String,
    pub part: i32,
}

fn persistent_id(arr: &ArrangementSpec) -> String {
    arr.persistent_id.replace('-', "").to_uppercase()
}

fn base(b: &Built, a: &mut Attrs) {
    let key = b.spec.dlc_key.to_lowercase();
    a.set("AlbumArt", format!("urn:image:dds:album_{key}"));
    a.set("ArrangementName", b.arr_spec.name.as_str());
    a.set("DLCKey", b.spec.dlc_key.as_str());
    a.set("ManifestUrn", format!("urn:database:json-db:{key}_{}", b.part_name));
    a.set("MasterID_RDV", b.arr_spec.master_id);
    a.set("PersistentID", persistent_id(b.arr_spec));
    a.set("SongKey", b.spec.dlc_key.as_str());
}

fn song_common(b: &Built, a: &mut Attrs) {
    let s = b.spec;
    let sng = b.sng;
    let c = b.counts;
    let (hard, medium, easy) = difficulties(b.source, sng, &c);
    let hard_notes = (c.hard as f64).max(1.0);
    a.set("AlbumName", s.album.value.as_str());
    a.set("AlbumNameSort", if s.album.sort.is_empty() { s.album.value.as_str() } else { s.album.sort.as_str() });
    a.set("ArtistName", s.artist.value.as_str());
    a.set(
        "ArtistNameSort",
        if s.artist.sort.is_empty() { crate::strings::artist_sort(&s.artist.value) } else { s.artist.sort.clone() },
    );
    if b.source.metadata.capo > 0 {
        a.set("CapoFret", b.source.metadata.capo as f64);
    }
    a.set("CentOffset", (1200.0 * (b.arr_spec.tuning_pitch / 440.0).ln() / 2f64.ln()).round_ties_even());
    a.set("DNA_Chords", dna_time(sng, sng::dna::CHORD));
    a.set("DNA_Riffs", dna_time(sng, sng::dna::RIFF));
    a.set("DNA_Solo", dna_time(sng, sng::dna::SOLO));
    a.set("EasyMastery", round_even(c.easy as f64 / hard_notes, 9));
    a.set("MediumMastery", round_even(c.medium as f64 / hard_notes, 9));
    a.set("NotesEasy", c.easy as f32);
    a.set("NotesHard", c.hard as f32);
    a.set("NotesMedium", c.medium as f32);
    a.set("SongDiffEasy", easy);
    a.set("SongDiffHard", hard);
    a.set("SongDiffMed", medium);
    a.set("SongDifficulty", hard);
    a.set("SongLength", sng.metadata.song_length);
    a.set("SongName", s.title.value.as_str());
    a.set(
        "SongNameSort",
        if s.title.sort.is_empty() { crate::strings::title_sort(&s.title.value) } else { s.title.sort.clone() },
    );
    a.set("SongYear", s.year);
    let t = b.arr_spec.tuning;
    a.set(
        "Tuning",
        J::obj()
            .with("string0", t[0])
            .with("string1", t[1])
            .with("string2", t[2])
            .with("string3", t[3])
            .with("string4", t[4])
            .with("string5", t[5]),
    );
}

/// Manifest header attributes (one per arrangement, in the .hsan file).
pub fn header_attributes(b: &Built) -> J {
    let mut a = Attrs::new();
    base(b, &mut a);
    song_common(b, &mut a);
    if b.arr_spec.bass_picked {
        a.set("BassPick", 1);
    }
    a.set("Representative", (b.arr_spec.priority == crate::spec::Priority::Main) as i32);
    a.set("RouteMask", b.arr_spec.route_mask());
    a.into_json()
}

/// Full manifest attributes for an arrangement.
pub fn attributes(b: &Built, tone: &J) -> Result<J, Error> {
    let s = b.spec;
    let sng = b.sng;
    let key = s.dlc_key.to_lowercase();
    let name = &b.part_name;
    let mut a = Attrs::new();
    base(b, &mut a);
    song_common(b, &mut a);

    // common to every non-header arrangement
    a.set("ArrangementSort", 0);
    a.set("BlockAsset", format!("urn:emergent-world:{key}"));
    a.set("DynamicVisualDensity", dynamic_visual_density(sng.levels.len(), b.arr_spec.scroll_speed));
    a.set("FullName", format!("{}_{}", s.dlc_key, b.arr_spec.name.as_str()));
    a.set("MasterID_PS3", -1);
    a.set("MasterID_XBox360", -1);
    a.set("PreviewBankPath", format!("song_{key}_preview.bnk"));
    a.set("RelativeDifficulty", 0);
    a.set("ShowlightsXML", format!("urn:application:xml:{key}_showlights"));
    a.set("SongAsset", format!("urn:application:musicgame-song:{key}_{name}"));
    a.set("SongBank", format!("song_{key}.bnk"));
    a.set("SongEvent", format!("Play_{}", s.dlc_key));
    a.set("SongXml", format!("urn:application:xml:{key}_{name}"));

    // instrumental specifics
    let p = &b.source.metadata.arrangement_properties;
    let u = |v: bool| v as u8;
    let arr = b.arr_spec;
    let rm = arr.route_mask();
    a.set(
        "ArrangementProperties",
        J::obj()
            .with("represent", u(arr.priority == crate::spec::Priority::Main))
            .with("bonusArr", u(arr.priority == crate::spec::Priority::Bonus))
            .with("standardTuning", u(p.standard_tuning))
            .with("nonStandardChords", u(p.non_standard_chords))
            .with("barreChords", u(p.barre_chords))
            .with("powerChords", u(p.power_chords))
            .with("dropDPower", u(p.drop_d_power))
            .with("openChords", u(p.open_chords))
            .with("fingerPicking", u(p.finger_picking))
            .with("pickDirection", u(p.pick_direction))
            .with("doubleStops", u(p.double_stops))
            .with("palmMutes", u(p.palm_mutes))
            .with("harmonics", u(p.harmonics))
            .with("pinchHarmonics", u(p.pinch_harmonics))
            .with("hopo", u(p.hopo))
            .with("tremolo", u(p.tremolo))
            .with("slides", u(p.slides))
            .with("unpitchedSlides", u(p.unpitched_slides))
            .with("bends", u(p.bends))
            .with("tapping", u(p.tapping))
            .with("vibrato", u(p.vibrato))
            .with("fretHandMutes", u(p.fret_hand_mutes))
            .with("slapPop", u(p.slap_pop))
            .with("twoFingerPicking", u(p.two_finger_picking))
            .with("fifthsAndOctaves", u(p.fifths_and_octaves))
            .with("syncopation", u(p.syncopation))
            .with("bassPick", u(arr.bass_picked))
            .with("sustain", u(p.sustain))
            .with("pathLead", u(rm == 1))
            .with("pathRhythm", u(rm == 2))
            .with("pathBass", u(rm == 4))
            .with("routeMask", rm),
    );
    a.set("ArrangementType", arr.name.type_id());
    a.set("Chords", chord_map(sng));
    a.set(
        "ChordTemplates",
        J::Arr(
            sng.chords
                .iter()
                .enumerate()
                .filter(|(_, c)| !c.name.0.is_empty() && c.mask != sng::ChordMask::ARPEGGIO)
                .map(|(id, c)| {
                    J::obj()
                        .with("ChordId", id as i16)
                        .with("ChordName", c.name.0.as_str())
                        .with("Fingers", c.fingers.to_vec())
                        .with("Frets", c.frets.to_vec())
                })
                .collect(),
        ),
    );
    a.set("LastConversionDateTime", sng.metadata.last_conversion_date_time.as_str());
    a.set("MaxPhraseDifficulty", sng.levels.len() as i32 - 1);
    a.set(
        "PhraseIterations",
        J::Arr(
            sng.phrase_iterations
                .iter()
                .map(|pi| {
                    let ph = &sng.phrases[pi.phrase_id as usize];
                    J::obj()
                        .with("PhraseIndex", pi.phrase_id)
                        .with("MaxDifficulty", ph.max_difficulty)
                        .with("Name", ph.name.0.as_str())
                        .with("StartTime", pi.start_time)
                        .with("EndTime", pi.end_time)
                })
                .collect(),
        ),
    );
    a.set(
        "Phrases",
        J::Arr(
            sng.phrases
                .iter()
                .map(|p| {
                    J::obj()
                        .with("MaxDifficulty", p.max_difficulty as i8)
                        .with("Name", p.name.0.as_str())
                        .with("IterationCount", p.iteration_count)
                })
                .collect(),
        ),
    );
    a.set("Score_MaxNotes", b.counts.hard as f32);
    a.set("Score_PNV", 100_000f32 / (b.counts.hard as f32).max(1.0));
    let mut sections = Vec::new();
    for sec in &sng.sections {
        sections.push(
            J::obj()
                .with("Name", sec.name.0.as_str())
                .with("UIName", section_ui_name(&sec.name.0)?)
                .with("Number", sec.number)
                .with("StartTime", sec.start_time)
                .with("EndTime", sec.end_time)
                .with("StartPhraseIterationIndex", sec.start_phrase_iteration_id)
                .with("EndPhraseIterationIndex", sec.end_phrase_iteration_id)
                .with("IsSolo", crate::strings::starts_with_ci(&sec.name.0, "solo")),
        );
    }
    a.set("Sections", J::Arr(sections));
    let tempo = b.source.metadata.average_tempo;
    a.set("SongAverageTempo", if tempo <= 0.0 { 120.0 } else { tempo });
    a.set("SongOffset", -sng.metadata.start_time);
    a.set("SongPartition", b.part);
    a.set("TargetScore", 100_000);
    a.set("Techniques", technique_map(sng));
    a.set("Tone_A", "");
    a.set("Tone_B", "");
    a.set("Tone_Base", crate::spec::BASE_TONE_KEY);
    a.set("Tone_C", "");
    a.set("Tone_D", "");
    a.set("Tone_Multiplayer", "");
    a.set("Tones", J::Arr(vec![tone.clone()]));
    Ok(a.into_json())
}

/// Wraps attributes into a manifest file body.
pub fn manifest(entries: Vec<(String, J)>, header: bool) -> J {
    let mut map = J::obj();
    let mut entries = entries;
    entries.sort_by(|a, b| a.0.cmp(&b.0)); // F# Map is sorted by key
    for (id, attrs) in entries {
        map = map.with(&id, J::obj().with("Attributes", attrs));
    }
    let mut m = J::obj().with("Entries", map);
    if !header {
        m = m.with("ModelName", "RSEnumerable_Song").with("IterationVersion", 2);
    }
    m.with("InsertRoot", if header { "Static.Songs.Headers" } else { "Static.Songs.Entries" })
}

pub fn persistent_id_key(arr: &ArrangementSpec) -> String {
    persistent_id(arr)
}
