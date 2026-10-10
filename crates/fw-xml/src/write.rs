//! XML output in the style of .NET's XmlWriter (2-space indent, `<a b="c" />`
//! for empty elements) so files diff cleanly against DLC Builder's.

use crate::*;

struct Out {
    s: String,
    depth: usize,
    /// The current start tag is still open (`<name attr=".."` without `>`).
    open: bool,
    /// Names of open elements and whether each has child elements.
    stack: Vec<(&'static str, bool)>,
    abridged: bool,
}

fn escape_attr(v: &str) -> String {
    let mut o = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\n' => o.push_str("&#xA;"),
            '\r' => o.push_str("&#xD;"),
            '\t' => o.push_str("&#x9;"),
            c => o.push(c),
        }
    }
    o
}

fn escape_text(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl Out {
    fn close_start(&mut self) {
        if self.open {
            self.s.push('>');
            self.open = false;
        }
    }

    fn indent(&mut self) {
        self.s.push('\n');
        for _ in 0..self.depth {
            self.s.push_str("  ");
        }
    }

    fn start(&mut self, name: &'static str) {
        self.close_start();
        if let Some(top) = self.stack.last_mut() {
            top.1 = true;
        }
        self.indent();
        self.s.push('<');
        self.s.push_str(name);
        self.open = true;
        self.stack.push((name, false));
        self.depth += 1;
    }

    fn attr(&mut self, k: &str, v: impl AsRef<str>) {
        self.s.push(' ');
        self.s.push_str(k);
        self.s.push_str("=\"");
        self.s.push_str(&escape_attr(v.as_ref()));
        self.s.push('"');
    }

    /// Writes `k="1"` when set, otherwise `k="<default>"` unless abridged.
    fn flag(&mut self, k: &str, on: bool, on_value: &str, default: &str) {
        if on {
            self.attr(k, on_value);
        } else if !self.abridged {
            self.attr(k, default);
        }
    }

    fn end(&mut self) {
        let (name, had_children) = self.stack.pop().expect("balanced elements");
        self.depth -= 1;
        if self.open {
            self.s.push_str(" />");
            self.open = false;
        } else {
            if had_children {
                self.indent();
            }
            self.s.push_str("</");
            self.s.push_str(name);
            self.s.push('>');
        }
    }

    /// `<name>text</name>`, or `<name />` when the text is empty.
    fn element(&mut self, name: &'static str, text: &str) {
        self.start(name);
        if !text.is_empty() {
            self.close_start();
            self.s.push_str(&escape_text(text));
        }
        self.end();
    }

    fn list<T>(&mut self, name: &'static str, item: &'static str, xs: &[T], f: impl Fn(&mut Out, &T)) {
        self.start(name);
        self.attr("count", xs.len().to_string());
        for x in xs {
            self.start(item);
            f(self, x);
            self.end();
        }
        self.end();
    }
}

/// .NET's default float formatting is the shortest round-trip string.
fn float(v: f32) -> String {
    format!("{v}")
}

pub(crate) fn arrangement(arr: &InstrumentalArrangement, abridged: bool) -> String {
    let mut o = Out { s: String::with_capacity(1 << 20), depth: 0, open: false, stack: Vec::new(), abridged };
    o.s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>");
    let md = &arr.metadata;

    o.start("song");
    o.attr("version", arr.version.to_string());
    for c in &arr.comments {
        o.close_start();
        o.indent();
        o.s.push_str("<!--");
        o.s.push_str(c);
        o.s.push_str("-->");
        if let Some(top) = o.stack.last_mut() {
            top.1 = true;
        }
    }
    o.element("title", &md.title);
    o.element("arrangement", &md.arrangement);
    o.element("part", &md.part.to_string());
    o.element("offset", &format!("{:.3}", arr.start_beat() as f32 / -1000.0));
    o.element("centOffset", &float(md.cent_offset));
    o.element("songLength", &time::to_string(md.song_length));
    if let Some(s) = &md.title_sort {
        o.element("songNameSort", s);
    }
    o.element("startBeat", &time::to_string(arr.start_beat()));
    o.element("averageTempo", &format!("{:.3}", md.average_tempo));
    o.start("tuning");
    for (i, s) in md.tuning.iter().enumerate() {
        o.attr(&format!("string{i}"), s.to_string());
    }
    o.end();
    o.element("capo", &md.capo.to_string());
    o.element("artistName", &md.artist_name);
    if let Some(s) = &md.artist_name_sort {
        o.element("artistNameSort", s);
    }
    o.element("albumName", &md.album_name);
    if let Some(s) = &md.album_name_sort {
        o.element("albumNameSort", s);
    }
    o.element("albumYear", &md.album_year.to_string());
    if let Some(s) = &md.album_art {
        o.element("albumArt", s);
    }
    o.element("crowdSpeed", "1");
    arrangement_properties(&mut o, &md.arrangement_properties);
    o.element("lastConversionDateTime", &md.last_conversion_date_time);

    o.list("phrases", "phrase", &arr.phrases, |o, p| {
        o.attr("maxDifficulty", p.max_difficulty.to_string());
        o.attr("name", &p.name);
        o.flag("disparity", p.mask.contains(PhraseMask::DISPARITY), "1", "0");
        o.flag("ignore", p.mask.contains(PhraseMask::IGNORE), "1", "0");
        o.flag("solo", p.mask.contains(PhraseMask::SOLO), "1", "0");
    });
    o.list("phraseIterations", "phraseIteration", &arr.phrase_iterations, |o, pi| {
        o.attr("time", time::to_string(pi.time));
        o.attr("phraseId", pi.phrase_id.to_string());
        if !(o.abridged && pi.variation.is_empty()) {
            o.attr("variation", &pi.variation);
        }
        if pi.hero_levels.any() {
            o.start("heroLevels");
            o.attr("count", "3");
            for (hero, d) in [(1, pi.hero_levels.easy), (2, pi.hero_levels.medium), (3, pi.hero_levels.hard)] {
                o.start("heroLevel");
                o.attr("hero", hero.to_string());
                o.attr("difficulty", d.to_string());
                o.end();
            }
            o.end();
        }
    });
    o.list("newLinkedDiffs", "newLinkedDiff", &arr.new_linked_diffs, |o, n| {
        o.attr("levelBreak", n.level_break.to_string());
        o.attr("ratio", n.ratio.as_deref().unwrap_or("1.000"));
        o.attr("phraseCount", n.phrase_ids.len().to_string());
        for id in &n.phrase_ids {
            o.start("nld_phrase");
            o.attr("id", id.to_string());
            o.end();
        }
    });
    if let Some(ld) = &arr.linked_diffs {
        o.list("linkedDiffs", "linkedDiff", ld, |o, l| {
            o.attr("childId", l.child_id.to_string());
            o.attr("parentId", l.parent_id.to_string());
        });
    }
    if let Some(pp) = &arr.phrase_properties {
        o.list("phraseProperties", "phraseProperty", pp, |o, p| {
            o.attr("phraseId", p.phrase_id.to_string());
            o.attr("redundant", p.redundant.to_string());
            o.attr("levelJump", p.level_jump.to_string());
            o.attr("empty", p.empty.to_string());
            o.attr("difficulty", p.difficulty.to_string());
        });
    }
    o.list("chordTemplates", "chordTemplate", &arr.chord_templates, |o, t| {
        o.attr("chordName", &t.name);
        o.attr("displayName", &t.display_name);
        for (i, f) in t.fingers.iter().enumerate() {
            o.flag(&format!("finger{i}"), *f != -1, &f.to_string(), "-1");
        }
        for (i, f) in t.frets.iter().enumerate() {
            o.flag(&format!("fret{i}"), *f != -1, &f.to_string(), "-1");
        }
    });
    o.list("ebeats", "ebeat", &arr.ebeats, |o, b| {
        o.attr("time", time::to_string(b.time));
        o.flag("measure", b.measure != -1, &b.measure.to_string(), "-1");
    });

    if let Some(b) = &arr.tones.base {
        o.element("tonebase", b);
    }
    for (i, n) in arr.tones.names.iter().enumerate() {
        if let Some(n) = n {
            o.element(["tonea", "toneb", "tonec", "toned"][i], n);
        }
    }
    o.list("tones", "tone", &arr.tones.changes, |o, t| {
        o.attr("time", time::to_string(t.time));
        o.attr("id", t.id.to_string());
        o.attr("name", &t.name);
    });

    o.list("sections", "section", &arr.sections, |o, s| {
        o.attr("name", &s.name);
        o.attr("number", s.number.to_string());
        o.attr("startTime", time::to_string(s.time));
    });
    o.list("events", "event", &arr.events, |o, e| {
        o.attr("time", time::to_string(e.time));
        o.attr("code", &e.code);
    });
    if let Some(t) = &arr.transcription_track {
        o.start("transcriptionTrack");
        level_body(&mut o, t);
        o.end();
    }
    o.list("levels", "level", &arr.levels, level_body);
    o.end();
    o.s
}

fn arrangement_properties(o: &mut Out, p: &ArrangementProperties) {
    o.start("arrangementProperties");
    let b = |v: bool| if v { "1" } else { "0" };
    for (k, v) in [
        ("represent", p.represent),
        ("bonusArr", p.bonus_arrangement),
        ("standardTuning", p.standard_tuning),
        ("nonStandardChords", p.non_standard_chords),
        ("barreChords", p.barre_chords),
        ("powerChords", p.power_chords),
        ("dropDPower", p.drop_d_power),
        ("openChords", p.open_chords),
        ("fingerPicking", p.finger_picking),
        ("pickDirection", p.pick_direction),
        ("doubleStops", p.double_stops),
        ("palmMutes", p.palm_mutes),
        ("harmonics", p.harmonics),
        ("pinchHarmonics", p.pinch_harmonics),
        ("hopo", p.hopo),
        ("tremolo", p.tremolo),
        ("slides", p.slides),
        ("unpitchedSlides", p.unpitched_slides),
        ("bends", p.bends),
        ("tapping", p.tapping),
        ("vibrato", p.vibrato),
        ("fretHandMutes", p.fret_hand_mutes),
        ("slapPop", p.slap_pop),
        ("twoFingerPicking", p.two_finger_picking),
        ("fifthsAndOctaves", p.fifths_and_octaves),
        ("syncopation", p.syncopation),
        ("bassPick", p.bass_pick),
        ("sustain", p.sustain),
        ("pathLead", p.path_lead),
        ("pathRhythm", p.path_rhythm),
        ("pathBass", p.path_bass),
    ] {
        o.attr(k, b(v));
    }
    o.end();
}

fn level_body(o: &mut Out, l: &Level) {
    o.attr("difficulty", l.difficulty.to_string());
    o.list("notes", "note", &l.notes, note_attrs);
    o.list("chords", "chord", &l.chords, |o, c| {
        o.attr("time", time::to_string(c.time));
        o.attr("chordId", c.chord_id.to_string());
        let m = c.mask;
        o.flag("linkNext", m.contains(ChordMask::LINK_NEXT), "1", "0");
        o.flag("accent", m.contains(ChordMask::ACCENT), "1", "0");
        o.flag("fretHandMute", m.contains(ChordMask::FRET_HAND_MUTE), "1", "0");
        o.flag("highDensity", m.contains(ChordMask::HIGH_DENSITY), "1", "0");
        o.flag("ignore", m.contains(ChordMask::IGNORE), "1", "0");
        o.flag("palmMute", m.contains(ChordMask::PALM_MUTE), "1", "0");
        o.flag("hopo", m.contains(ChordMask::HOPO), "1", "0");
        if !o.abridged {
            o.attr("strum", "down");
        }
        for cn in c.chord_notes.iter().flatten() {
            o.start("chordNote");
            note_attrs(o, cn);
            o.end();
        }
    });
    o.list("anchors", "anchor", &l.anchors, |o, a| {
        o.attr("time", time::to_string(a.time));
        o.attr("fret", a.fret.to_string());
        o.attr("width", format!("{:.3}", a.width as f32));
    });
    o.list("handShapes", "handShape", &l.hand_shapes, |o, h| {
        o.attr("chordId", h.chord_id.to_string());
        o.attr("startTime", time::to_string(h.start_time));
        o.attr("endTime", time::to_string(h.end_time));
    });
}

fn note_attrs(o: &mut Out, n: &Note) {
    let m = n.mask;
    o.attr("time", time::to_string(n.time));
    o.attr("string", n.string.to_string());
    o.attr("fret", n.fret.to_string());
    o.flag("sustain", n.sustain > 0, &time::to_string(n.sustain), "0.000");
    o.flag("linkNext", m.contains(NoteMask::LINK_NEXT), "1", "0");
    o.flag("accent", m.contains(NoteMask::ACCENT), "1", "0");
    o.flag("bend", n.is_bend(), &float(n.max_bend), "0");
    o.flag("hammerOn", m.contains(NoteMask::HAMMER_ON), "1", "0");
    o.flag("harmonic", m.contains(NoteMask::HARMONIC), "1", "0");
    o.flag("hopo", n.is_hopo(), "1", "0");
    o.flag("ignore", m.contains(NoteMask::IGNORE), "1", "0");
    o.flag("leftHand", n.left_hand != -1, &n.left_hand.to_string(), "-1");
    o.flag("mute", m.contains(NoteMask::FRET_HAND_MUTE), "1", "0");
    o.flag("palmMute", m.contains(NoteMask::PALM_MUTE), "1", "0");
    o.flag("pluck", m.contains(NoteMask::PLUCK), "1", "-1");
    o.flag("pullOff", m.contains(NoteMask::PULL_OFF), "1", "0");
    o.flag("slap", m.contains(NoteMask::SLAP), "1", "-1");
    o.flag("slideTo", n.is_slide(), &n.slide_to.to_string(), "-1");
    o.flag("tremolo", m.contains(NoteMask::TREMOLO), "1", "0");
    o.flag("harmonicPinch", m.contains(NoteMask::PINCH_HARMONIC), "1", "0");
    o.flag("pickDirection", m.contains(NoteMask::PICK_DIRECTION), "1", "0");
    o.flag("rightHand", m.contains(NoteMask::RIGHT_HAND), "1", "-1");
    o.flag("slideUnpitchTo", n.is_unpitched_slide(), &n.slide_unpitch_to.to_string(), "-1");
    o.flag("tap", n.is_tap(), &n.tap.to_string(), "0");
    o.flag("vibrato", n.is_vibrato(), &n.vibrato.to_string(), "0");
    if let Some(bends) = n.bend_values.as_ref().filter(|b| !b.is_empty()) {
        o.list("bendValues", "bendValue", bends, |o, b| {
            o.attr("time", time::to_string(b.time));
            if b.step != 0.0 || !o.abridged {
                o.attr("step", format!("{:.3}", b.step));
            }
        });
    }
}
