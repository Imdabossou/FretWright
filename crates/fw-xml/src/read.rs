use std::str::FromStr;

use quick_xml::events::Event as XmlEvent;
use quick_xml::reader::Reader;

use crate::*;

/// Minimal element tree; arrangement files are at most a few MB.
#[derive(Debug, Default)]
struct El {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<El>,
    text: String,
    /// Comments that appear before the first child element.
    leading_comments: Vec<String>,
}

impl El {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn req(&self, name: &str) -> Result<&str> {
        self.attr(name).ok_or_else(|| Error::Invalid {
            what: format!("<{}> attribute {name}", self.name),
            value: "(missing)".into(),
        })
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    fn child(&self, name: &str) -> Option<&El> {
        self.children.iter().find(|c| c.name == name)
    }
}

fn parse_tree(xml: &str) -> Result<El> {
    let xml = xml.trim_start_matches('\u{feff}');
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<El> = Vec::new();
    let syntax = |e: quick_xml::Error| Error::Syntax(e.to_string());

    loop {
        match reader.read_event().map_err(syntax)? {
            XmlEvent::Start(e) => stack.push(element(&e)?),
            XmlEvent::Empty(e) => {
                let el = element(&e)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => return Ok(el),
                }
            }
            XmlEvent::End(_) => {
                let el = stack.pop().ok_or_else(|| Error::Syntax("unbalanced end tag".into()))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => return Ok(el),
                }
            }
            XmlEvent::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    // entities arrive as separate GeneralRef events, so this is plain text
                    top.text.push_str(&t.decode().map_err(|e| Error::Syntax(e.to_string()))?);
                }
            }
            XmlEvent::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            XmlEvent::GeneralRef(r) => {
                if let Some(top) = stack.last_mut() {
                    let name = r.decode().map_err(|e| Error::Syntax(e.to_string()))?;
                    let s = format!("&{name};");
                    top.text.push_str(&quick_xml::escape::unescape(&s).map_err(|e| Error::Syntax(e.to_string()))?);
                }
            }
            XmlEvent::Comment(c) => {
                if let Some(top) = stack.last_mut()
                    && top.children.is_empty()
                {
                    top.leading_comments.push(String::from_utf8_lossy(&c).into_owned());
                }
            }
            XmlEvent::Eof => return Err(Error::Syntax("unexpected end of file".into())),
            _ => {}
        }
    }
}

fn element(e: &quick_xml::events::BytesStart) -> Result<El> {
    let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| Error::Syntax(e.to_string()))?;
        let key = String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned();
        let value = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| Error::Syntax(e.to_string()))?
            .into_owned();
        attrs.push((key, value));
    }
    Ok(El { name, attrs, ..Default::default() })
}

fn num<T: FromStr>(what: &str, s: &str) -> Result<T> {
    s.trim().parse().map_err(|_| Error::Invalid { what: what.into(), value: s.into() })
}

fn time(what: &str, s: &str) -> Result<i32> {
    time::parse(s).ok_or_else(|| Error::Invalid { what: what.into(), value: s.into() })
}

/// Rocksmith2014.NET's ParseBinary: true unless the first character is '0'.
fn binary(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c != '0')
}

fn opt_flag(e: &El, attr: &str) -> bool {
    e.attr(attr).is_some_and(binary)
}

/// Attributes like slap/pluck/rightHand are -1 when absent.
fn minus_one_flag(e: &El, attr: &str) -> Result<bool> {
    match e.attr(attr) {
        Some(v) => Ok(num::<i8>(attr, v)? != -1),
        None => Ok(false),
    }
}

fn list<T>(e: &El, item: impl Fn(&El) -> Result<T>) -> Result<Vec<T>> {
    e.children.iter().map(item).collect()
}

pub(crate) fn arrangement(xml: &str) -> Result<InstrumentalArrangement> {
    let root = parse_tree(xml)?;
    if root.name != "song" {
        return Err(Error::NotAnArrangement(root.name));
    }
    let version: u8 = num("song version", root.attr("version").unwrap_or("0"))?;
    if version < 7 {
        return Err(Error::UnsupportedVersion(version));
    }
    let mut arr = InstrumentalArrangement { version, comments: root.leading_comments.clone(), ..Default::default() };
    let md = &mut arr.metadata;

    for c in &root.children {
        let text = c.text.as_str();
        match c.name.as_str() {
            "title" => md.title = text.into(),
            "arrangement" => md.arrangement = text.into(),
            "part" => md.part = num("part", text)?,
            "centOffset" => md.cent_offset = num("centOffset", text)?,
            "songLength" => md.song_length = time("songLength", text)?,
            "songNameSort" => md.title_sort = Some(text.into()),
            "averageTempo" => md.average_tempo = num("averageTempo", text)?,
            "tuning" => {
                for (i, s) in md.tuning.iter_mut().enumerate() {
                    *s = num("tuning", c.req(&format!("string{i}"))?)?;
                }
            }
            "capo" => md.capo = num("capo", text)?,
            "artistName" => md.artist_name = text.into(),
            "artistNameSort" => md.artist_name_sort = Some(text.into()),
            "albumName" => md.album_name = text.into(),
            "albumNameSort" => md.album_name_sort = Some(text.into()),
            "albumYear" if !text.trim().is_empty() => md.album_year = num("albumYear", text)?,
            "albumArt" => md.album_art = Some(text.into()),
            "arrangementProperties" => md.arrangement_properties = arrangement_properties(c),
            "lastConversionDateTime" => md.last_conversion_date_time = text.into(),
            "phrases" => arr.phrases = list(c, phrase)?,
            "phraseIterations" => arr.phrase_iterations = list(c, phrase_iteration)?,
            "newLinkedDiffs" => arr.new_linked_diffs = list(c, new_linked_diff)?,
            "linkedDiffs" => arr.linked_diffs = Some(list(c, linked_diff)?),
            "phraseProperties" => arr.phrase_properties = Some(list(c, phrase_property)?),
            "chordTemplates" => arr.chord_templates = list(c, chord_template)?,
            "ebeats" => arr.ebeats = list(c, ebeat)?,
            "tonebase" => arr.tones.base = Some(text.into()),
            "tonea" => arr.tones.names[0] = Some(text.into()),
            "toneb" => arr.tones.names[1] = Some(text.into()),
            "tonec" => arr.tones.names[2] = Some(text.into()),
            "toned" => arr.tones.names[3] = Some(text.into()),
            "tones" => arr.tones.changes = list(c, tone_change)?,
            "sections" => arr.sections = list(c, section)?,
            "events" => arr.events = list(c, event)?,
            "transcriptionTrack" => arr.transcription_track = Some(level(c)?),
            "levels" => arr.levels = list(c, level)?,
            _ => {}
        }
    }
    Ok(arr)
}

fn arrangement_properties(e: &El) -> ArrangementProperties {
    let f = |n: &str| opt_flag(e, n);
    ArrangementProperties {
        represent: f("represent"),
        bonus_arrangement: f("bonusArr"),
        standard_tuning: f("standardTuning"),
        non_standard_chords: f("nonStandardChords"),
        barre_chords: f("barreChords"),
        power_chords: f("powerChords"),
        drop_d_power: f("dropDPower"),
        open_chords: f("openChords"),
        finger_picking: f("fingerPicking"),
        pick_direction: f("pickDirection"),
        double_stops: f("doubleStops"),
        palm_mutes: f("palmMutes"),
        harmonics: f("harmonics"),
        pinch_harmonics: f("pinchHarmonics"),
        hopo: f("hopo"),
        tremolo: f("tremolo"),
        slides: f("slides"),
        unpitched_slides: f("unpitchedSlides"),
        bends: f("bends"),
        tapping: f("tapping"),
        vibrato: f("vibrato"),
        fret_hand_mutes: f("fretHandMutes"),
        slap_pop: f("slapPop"),
        two_finger_picking: f("twoFingerPicking"),
        fifths_and_octaves: f("fifthsAndOctaves"),
        syncopation: f("syncopation"),
        bass_pick: f("bassPick"),
        sustain: f("sustain"),
        path_lead: f("pathLead"),
        path_rhythm: f("pathRhythm"),
        path_bass: f("pathBass"),
    }
}

fn phrase(e: &El) -> Result<Phrase> {
    let mut mask = PhraseMask::empty();
    mask.set(PhraseMask::DISPARITY, opt_flag(e, "disparity"));
    mask.set(PhraseMask::IGNORE, opt_flag(e, "ignore"));
    mask.set(PhraseMask::SOLO, opt_flag(e, "solo"));
    Ok(Phrase {
        name: e.attr("name").unwrap_or_default().into(),
        max_difficulty: num("maxDifficulty", e.attr("maxDifficulty").unwrap_or("0"))?,
        mask,
    })
}

fn phrase_iteration(e: &El) -> Result<PhraseIteration> {
    let mut hero = HeroLevels::default();
    if let Some(h) = e.child("heroLevels") {
        for hl in h.children_named("heroLevel") {
            let d: u8 = num("heroLevel difficulty", hl.req("difficulty")?)?;
            match num::<i32>("hero", hl.req("hero")?)? {
                1 => hero.easy = d,
                2 => hero.medium = d,
                3 => hero.hard = d,
                _ => {}
            }
        }
        // official files omit the hard level when it equals medium
        if hero.hard == 0 && hero.medium != 0 {
            hero.hard = hero.medium;
        }
    }
    Ok(PhraseIteration {
        time: time("phraseIteration time", e.req("time")?)?,
        phrase_id: num("phraseId", e.req("phraseId")?)?,
        variation: e.attr("variation").unwrap_or_default().into(),
        hero_levels: hero,
    })
}

fn new_linked_diff(e: &El) -> Result<NewLinkedDiff> {
    Ok(NewLinkedDiff {
        level_break: num("levelBreak", e.attr("levelBreak").unwrap_or("-1"))?,
        ratio: e.attr("ratio").map(Into::into),
        phrase_ids: e
            .children_named("nld_phrase")
            .map(|p| num("nld_phrase id", p.req("id")?))
            .collect::<Result<_>>()?,
    })
}

fn linked_diff(e: &El) -> Result<LinkedDiff> {
    Ok(LinkedDiff {
        child_id: num("childId", e.req("childId")?)?,
        parent_id: num("parentId", e.req("parentId")?)?,
    })
}

fn phrase_property(e: &El) -> Result<PhraseProperty> {
    Ok(PhraseProperty {
        phrase_id: num("phraseId", e.req("phraseId")?)?,
        redundant: num("redundant", e.attr("redundant").unwrap_or("0"))?,
        level_jump: num("levelJump", e.attr("levelJump").unwrap_or("0"))?,
        empty: num("empty", e.attr("empty").unwrap_or("0"))?,
        difficulty: num("difficulty", e.attr("difficulty").unwrap_or("0"))?,
    })
}

fn chord_template(e: &El) -> Result<ChordTemplate> {
    let mut t = ChordTemplate {
        name: e.attr("chordName").unwrap_or_default().into(),
        display_name: e.attr("displayName").unwrap_or_default().into(),
        ..Default::default()
    };
    for i in 0..6 {
        if let Some(v) = e.attr(&format!("finger{i}")) {
            t.fingers[i] = num("finger", v)?;
        }
        if let Some(v) = e.attr(&format!("fret{i}")) {
            t.frets[i] = num("fret", v)?;
        }
    }
    Ok(t)
}

fn ebeat(e: &El) -> Result<Ebeat> {
    Ok(Ebeat {
        time: time("ebeat time", e.req("time")?)?,
        measure: num("measure", e.attr("measure").unwrap_or("-1"))?,
    })
}

fn tone_change(e: &El) -> Result<ToneChange> {
    Ok(ToneChange {
        time: time("tone time", e.req("time")?)?,
        id: num("tone id", e.attr("id").unwrap_or("0"))?,
        name: e.attr("name").unwrap_or_default().into(),
    })
}

fn section(e: &El) -> Result<Section> {
    Ok(Section {
        name: e.attr("name").unwrap_or_default().into(),
        number: num("section number", e.attr("number").unwrap_or("0"))?,
        time: time("section startTime", e.req("startTime")?)?,
    })
}

fn event(e: &El) -> Result<Event> {
    Ok(Event { time: time("event time", e.req("time")?)?, code: e.attr("code").unwrap_or_default().into() })
}

fn level(e: &El) -> Result<Level> {
    let mut l = Level { difficulty: num("level difficulty", e.attr("difficulty").unwrap_or("0"))?, ..Default::default() };
    for c in &e.children {
        match c.name.as_str() {
            "notes" => l.notes = list(c, note)?,
            "chords" => l.chords = list(c, chord)?,
            "anchors" => l.anchors = list(c, anchor)?,
            "handShapes" => l.hand_shapes = list(c, hand_shape)?,
            _ => {}
        }
    }
    Ok(l)
}

fn note(e: &El) -> Result<Note> {
    let mut n = Note::default();
    let mut mask = NoteMask::empty();
    for (k, v) in &e.attrs {
        match k.as_str() {
            "time" => n.time = time("note time", v)?,
            "sustain" => n.sustain = time("note sustain", v)?,
            "fret" => n.fret = num("fret", v)?,
            "leftHand" => n.left_hand = num("leftHand", v)?,
            "slideTo" => n.slide_to = num("slideTo", v)?,
            "slideUnpitchTo" => n.slide_unpitch_to = num("slideUnpitchTo", v)?,
            "string" => n.string = num("string", v)?,
            "vibrato" => n.vibrato = num("vibrato", v)?,
            "tap" => n.tap = num("tap", v)?,
            "bend" => n.max_bend = num("bend", v)?,
            "linkNext" => mask.set(NoteMask::LINK_NEXT, binary(v)),
            "accent" => mask.set(NoteMask::ACCENT, binary(v)),
            "hammerOn" => mask.set(NoteMask::HAMMER_ON, binary(v)),
            "harmonic" => mask.set(NoteMask::HARMONIC, binary(v)),
            "ignore" => mask.set(NoteMask::IGNORE, binary(v)),
            "mute" => mask.set(NoteMask::FRET_HAND_MUTE, binary(v)),
            "palmMute" => mask.set(NoteMask::PALM_MUTE, binary(v)),
            "pullOff" => mask.set(NoteMask::PULL_OFF, binary(v)),
            "tremolo" => mask.set(NoteMask::TREMOLO, binary(v)),
            "harmonicPinch" => mask.set(NoteMask::PINCH_HARMONIC, binary(v)),
            "pickDirection" => mask.set(NoteMask::PICK_DIRECTION, binary(v)),
            _ => {}
        }
    }
    mask.set(NoteMask::SLAP, minus_one_flag(e, "slap")?);
    mask.set(NoteMask::PLUCK, minus_one_flag(e, "pluck")?);
    mask.set(NoteMask::RIGHT_HAND, minus_one_flag(e, "rightHand")?);
    n.mask = mask;
    if let Some(b) = e.child("bendValues") {
        n.bend_values = Some(list(b, |bv| {
            Ok(BendValue {
                time: time("bendValue time", bv.req("time")?)?,
                step: num("bendValue step", bv.attr("step").unwrap_or("0"))?,
            })
        })?);
    }
    Ok(n)
}

fn chord(e: &El) -> Result<Chord> {
    let mut mask = ChordMask::empty();
    for (attr, bit) in [
        ("linkNext", ChordMask::LINK_NEXT),
        ("accent", ChordMask::ACCENT),
        ("fretHandMute", ChordMask::FRET_HAND_MUTE),
        ("highDensity", ChordMask::HIGH_DENSITY),
        ("ignore", ChordMask::IGNORE),
        ("palmMute", ChordMask::PALM_MUTE),
        ("hopo", ChordMask::HOPO),
    ] {
        mask.set(bit, opt_flag(e, attr));
    }
    let notes: Vec<Note> = e.children_named("chordNote").map(note).collect::<Result<_>>()?;
    Ok(Chord {
        time: time("chord time", e.req("time")?)?,
        chord_id: num("chordId", e.req("chordId")?)?,
        mask,
        chord_notes: if notes.is_empty() { None } else { Some(notes) },
    })
}

fn anchor(e: &El) -> Result<Anchor> {
    Ok(Anchor {
        time: time("anchor time", e.req("time")?)?,
        fret: num("anchor fret", e.req("fret")?)?,
        // stored as "4.000"; truncated like the original
        width: num::<f32>("anchor width", e.attr("width").unwrap_or("4"))? as i8,
    })
}

fn hand_shape(e: &El) -> Result<HandShape> {
    Ok(HandShape {
        chord_id: num("handShape chordId", e.req("chordId")?)?,
        start_time: time("handShape startTime", e.req("startTime")?)?,
        end_time: time("handShape endTime", e.req("endTime")?)?,
    })
}
