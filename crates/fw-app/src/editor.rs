//! Tab-style note editor: four string lanes over a beat grid and a waveform strip.
//!
//! Mouse
//!   click                select note (Shift/Ctrl add to selection), click empty = deselect
//!   drag in empty space  rubber-band select
//!   drag note            move in time (snaps to the grid; hold Alt for free movement)
//!                        and between strings (pitch is kept when the new string allows it)
//!   drag note's end      change length
//!   double-click lane    add a note there
//!   click ruler/waveform move the playhead
//!   right-click          note and beat menu (make bar line, ...)
//!   wheel                scroll, Ctrl+wheel zoom
//! Keys
//!   0-9                  type a fret (two digits within 0.7 s)
//!   Up/Down              pitch +/- 1 semitone on the same string
//!   Shift+Up/Down        move to the next string keeping the pitch
//!   Left/Right           nudge by one grid step (Shift: one beat)
//!   Delete/Backspace     delete selection
//!   H P M A S            toggle hammer-on, pull-off, mute, accent, slide (to next note's fret)
//!   [ ]                  lower / raise the target fret of a slide

use std::collections::{BTreeSet, HashSet};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use fw_project::{MAX_FRET, Note, Project};

use crate::history::History;

pub const SHORTCUTS: &str = "Mouse
  click                select note (Shift/Ctrl add to selection)
  drag in empty space  rubber-band select
  drag note            move in time (snaps; hold Alt for free) and between strings
  drag note's end      change length
  double-click lane    add a note
  click ruler          move the playhead
  right-click          note / beat menu
  wheel                scroll, Ctrl+wheel zoom
Keys
  0-9                  type a fret (two digits within 0.7 s)
  Up/Down              pitch +/- 1 semitone
  Shift+Up/Down        move to the next string keeping the pitch
  Left/Right           nudge by one grid step (Shift: one beat)
  Delete/Backspace     delete selection
  H P M A S            hammer-on, pull-off, mute, accent, slide
  [ and ]              lower / raise the target fret of a slide, or a bend's height
  Marks on notes       H hammer-on, P pull-off, > accent, M fret-hand mute, PM palm mute,
                       Hm harmonic, Tr tremolo, ~ vibrato, Sl slap, Po pop, Tp tap,
                       L linked to the next note, b1 bend (steps); slanted line + number =
                       slide to that fret, dashed line = slide out. More under right-click.
  Ctrl+A / Esc         select all / none
  Space                play / pause";

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub fn note_name(midi: i32, octave: bool) -> String {
    let name = NOTE_NAMES[midi.rem_euclid(12) as usize];
    if octave { format!("{name}{}", midi.div_euclid(12) - 1) } else { name.to_string() }
}

/// Rocksmith's bass string colours, low to high.
pub const STRING_COLORS: [Color32; 4] =
    [Color32::from_rgb(0xe0, 0x47, 0x4c), Color32::from_rgb(0xe8, 0xc5, 0x47), Color32::from_rgb(0x3f, 0x8f, 0xe0), Color32::from_rgb(0xf0, 0x8a, 0x3c)];

const RULER_H: f32 = 22.0;
const WAVE_H: f32 = 56.0;
const LANE_MIN: f32 = 34.0;
const LANE_MAX: f32 = 110.0;
const LEFT: f32 = 34.0;
const MIN_NOTE_PX: f32 = 14.0;
const SCROLL_H: f32 = 12.0;

mod theme {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1d);
    pub const LANE: Color32 = Color32::from_rgb(0x1c, 0x1f, 0x26);
    pub const LANE_ALT: Color32 = Color32::from_rgb(0x1f, 0x23, 0x2b);
    pub const STRING_LINE: Color32 = Color32::from_rgb(0x35, 0x3b, 0x47);
    pub const BEAT: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x39);
    pub const BAR: Color32 = Color32::from_rgb(0x4a, 0x52, 0x63);
    pub const SUB: Color32 = Color32::from_rgb(0x20, 0x24, 0x2c);
    pub const DIM: Color32 = Color32::from_rgb(0x7d, 0x85, 0x94);
    pub const WAVE: Color32 = Color32::from_rgb(0x3c, 0x6e, 0x57);
    pub const WAVE_BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x19);
    pub const RULER: Color32 = Color32::from_rgb(0x10, 0x12, 0x16);
    pub const RUBBER: Color32 = Color32::from_rgba_premultiplied(24, 34, 51, 50);
}

pub mod mark {
    use eframe::egui::Color32;
    pub const HAMMER: Color32 = Color32::from_rgb(0x5c, 0xc2, 0x6a);
    pub const PULL: Color32 = Color32::from_rgb(0xf0, 0x9a, 0x2c);
    pub const SLIDE: Color32 = Color32::from_rgb(0x4f, 0xd0, 0xe8);
    pub const ACCENT: Color32 = Color32::from_rgb(0xee, 0xee, 0xf2);
    pub const MUTE: Color32 = Color32::from_rgb(0xb0, 0x70, 0xe0);
}

/// Draws the technique marks of a note: a badge per technique along its top
/// edge (always visible, however short the note) and a slanted line to the
/// target fret for slides.
fn draw_techniques(painter: &egui::Painter, n: &Note, r: Rect, head: Rect) {
    let mut x = r.left();
    let mut badge = |text: &str, fill: Color32| {
        let w = if text.chars().count() > 1 { 20.0 } else { 14.0 };
        let b = Rect::from_min_size(pos2(x, r.top() - 6.0), vec2(w, 11.0));
        painter.rect_filled(b, CornerRadius::same(3), fill);
        painter.text(b.center(), Align2::CENTER_CENTER, text, FontId::proportional(10.0), Color32::from_rgb(0x11, 0x11, 0x11));
        x += w + 1.0;
    };
    if n.hammer_on {
        badge("H", mark::HAMMER);
    }
    if n.pull_off {
        badge("P", mark::PULL);
    }
    if n.accent {
        badge(">", mark::ACCENT);
    }
    if n.mute {
        badge("M", mark::MUTE);
    }
    if n.palm_mute {
        badge("PM", mark::MUTE);
    }
    if n.harmonic {
        badge("Hm", mark::SLIDE);
    }
    if n.tremolo {
        badge("Tr", mark::PULL);
    }
    if n.vibrato {
        badge("~", mark::PULL);
    }
    if n.slap {
        badge("Sl", mark::HAMMER);
    }
    if n.pop {
        badge("Po", mark::HAMMER);
    }
    if n.tap {
        badge("Tp", mark::ACCENT);
    }
    if n.link_next {
        badge("L", mark::ACCENT);
    }
    if n.bend > 0.0 {
        badge(&format!("b{}", if n.bend.fract() == 0.0 { format!("{}", n.bend as i32) } else { format!("{:.1}", n.bend) }), mark::PULL);
    }
    if n.slide_unpitch_to >= 0 {
        // slide off the string: a dashed line towards the fret it ends near
        let up = n.slide_unpitch_to > n.fret;
        let x0 = head.right() + 3.0;
        let x1 = (r.right() - 4.0).max(x0 + 14.0);
        let (top, bottom) = (r.top() + 5.0, r.bottom() - 5.0);
        let (y0, y1) = if up { (bottom, top) } else { (top, bottom) };
        let segs = 5;
        for k in 0..segs {
            let (a, b) = (k as f32 / segs as f32, (k as f32 + 0.6) / segs as f32);
            let at = |f: f32| pos2(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
            painter.line_segment([at(a), at(b)], Stroke::new(2.5, mark::SLIDE));
        }
        painter.text(pos2(x1 + 4.0, r.center().y), Align2::LEFT_CENTER, format!("out {}", n.slide_unpitch_to), FontId::proportional(10.0), mark::SLIDE);
    }
    if n.slide_to >= 0 {
        let up = n.slide_to > n.fret;
        let x0 = head.right() + 3.0;
        let x1 = (r.right() - 4.0).max(x0 + 14.0);
        let (top, bottom) = (r.top() + 5.0, r.bottom() - 5.0);
        let (y0, y1) = if up { (bottom, top) } else { (top, bottom) };
        painter.line_segment([pos2(x0, y0), pos2(x1, y1)], Stroke::new(2.5, mark::SLIDE));
        painter.circle_filled(pos2(x1, y1), 3.0, mark::SLIDE);
        let label = n.slide_to.to_string();
        let galley = painter.layout_no_wrap(label, FontId::proportional(11.0), Color32::from_rgb(0x11, 0x11, 0x11));
        let b = Rect::from_min_size(pos2(x1 + 4.0, r.center().y - 7.0), vec2(galley.size().x + 6.0, 14.0));
        painter.rect_filled(b, CornerRadius::same(3), mark::SLIDE);
        painter.galley(pos2(b.left() + 3.0, b.center().y - galley.size().y / 2.0), galley, Color32::BLACK);
    }
}

/// A chart edit requested from a menu or key, applied with history.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Delete,
    SetFret(i32),
    Transpose(i32),
    MoveString(i32),
    Nudge(i32, bool),
    Toggle(Technique),
    /// Moves the target fret of the selected slides and the height of bends.
    SlideTarget(i32),
    Refret { only_selected: bool },
    SetDownbeat(usize),
    SelectAll,
    SelectNone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Technique {
    HammerOn,
    PullOff,
    Mute,
    Accent,
    Slide,
    SlideOut,
    Bend,
    Harmonic,
    PalmMute,
    Tremolo,
    Vibrato,
    Slap,
    Pop,
    Tap,
    LinkNext,
}

impl Technique {
    /// Whether `n` has this technique on.
    pub fn is_on(self, n: &Note) -> bool {
        match self {
            Technique::HammerOn => n.hammer_on,
            Technique::PullOff => n.pull_off,
            Technique::Mute => n.mute,
            Technique::Accent => n.accent,
            Technique::Slide => n.slide_to >= 0,
            Technique::SlideOut => n.slide_unpitch_to >= 0,
            Technique::Bend => n.bend > 0.0,
            Technique::Harmonic => n.harmonic,
            Technique::PalmMute => n.palm_mute,
            Technique::Tremolo => n.tremolo,
            Technique::Vibrato => n.vibrato,
            Technique::Slap => n.slap,
            Technique::Pop => n.pop,
            Technique::Tap => n.tap,
            Technique::LinkNext => n.link_next,
        }
    }

    /// The techniques of the right-click menu's main section.
    pub const COMMON: [(Technique, &'static str); 5] = [
        (Technique::HammerOn, "Hammer-on"),
        (Technique::PullOff, "Pull-off"),
        (Technique::Mute, "Fret-hand mute"),
        (Technique::Accent, "Accent"),
        (Technique::Slide, "Slide"),
    ];
    /// The rest, in the menu's "More techniques" submenu.
    pub const MORE: [(Technique, &'static str); 10] = [
        (Technique::SlideOut, "Slide out (unpitched)"),
        (Technique::Bend, "Bend"),
        (Technique::Harmonic, "Harmonic"),
        (Technique::PalmMute, "Palm mute"),
        (Technique::Tremolo, "Tremolo"),
        (Technique::Vibrato, "Vibrato"),
        (Technique::Slap, "Slap"),
        (Technique::Pop, "Pop"),
        (Technique::Tap, "Tap"),
        (Technique::LinkNext, "Link to next note"),
    ];
}

#[derive(Clone)]
struct Orig {
    idx: usize,
    start: f64,
    end: f64,
    string: i32,
    fret: i32,
    midi: i32,
}

enum Drag {
    Seek,
    Rubber { origin: Pos2, pos: Pos2, base: BTreeSet<usize> },
    Note { resize: bool, origin: Pos2, anchor: usize, started: bool, orig: Vec<Orig> },
    Scrollbar { grab: f64 },
}

/// What happened in the editor this frame.
#[derive(Default)]
pub struct Events {
    pub changed: bool,
    pub seek: Option<f64>,
}

pub struct Editor {
    pub selected: BTreeSet<usize>,
    pub pps: f32,
    pub scroll_t: f64,
    pub playhead: f64,
    pub follow: bool,
    division: u32,
    peaks: Option<(Vec<f32>, f64)>,
    subs: Vec<f64>,
    drag: Option<Drag>,
    digits: (String, f64),
    width: f32,
    menu_beat: Option<usize>,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            selected: BTreeSet::new(),
            pps: 120.0,
            scroll_t: 0.0,
            playhead: 0.0,
            follow: true,
            division: 4,
            peaks: None,
            subs: Vec::new(),
            drag: None,
            digits: (String::new(), 0.0),
            width: 1000.0,
            menu_beat: None,
        }
    }
}

fn duration(p: &Project) -> f64 {
    p.notes.iter().map(|n| n.end).fold(p.duration.max(1.0), f64::max)
}

/// Sorts notes by start time, keeping the selection on the same notes.
fn sort_keep(p: &mut Project, selected: &mut BTreeSet<usize>) {
    let mut tagged: Vec<(Note, bool)> = p.notes.drain(..).enumerate().map(|(i, n)| (n, selected.contains(&i))).collect();
    tagged.sort_by(|a, b| a.0.start.total_cmp(&b.0.start));
    selected.clear();
    for (i, (n, sel)) in tagged.into_iter().enumerate() {
        if sel {
            selected.insert(i);
        }
        p.notes.push(n);
    }
}

impl Editor {
    pub fn reset(&mut self) {
        self.selected.clear();
        self.drag = None;
        self.scroll_t = 0.0;
        self.playhead = 0.0;
    }

    pub fn set_peaks(&mut self, peaks: Option<(Vec<f32>, f64)>) {
        self.peaks = peaks;
    }

    pub fn set_division(&mut self, division: u32, p: Option<&Project>) {
        self.division = division;
        self.refresh_grid(p);
    }

    pub fn division(&self) -> u32 {
        self.division
    }

    pub fn refresh_grid(&mut self, p: Option<&Project>) {
        self.subs = match p {
            Some(p) if !p.grid.beats.is_empty() => fw_detect::notes::subdivisions(&p.grid, self.division),
            _ => Vec::new(),
        };
    }

    pub fn selected_notes<'a>(&self, p: &'a Project) -> Vec<&'a Note> {
        self.selected.iter().filter_map(|&i| p.notes.get(i)).collect()
    }

    fn span(&self) -> f64 {
        ((self.width - LEFT) / self.pps) as f64
    }

    pub fn set_scroll(&mut self, t: f64, p: &Project) {
        let max = (duration(p) - self.span() * 0.5).max(0.0);
        self.scroll_t = t.clamp(0.0, max);
    }

    pub fn zoom(&mut self, factor: f32, anchor_x: Option<f32>, p: &Project) {
        let ax = anchor_x.unwrap_or(self.width / 2.0);
        let t = self.scroll_t + ((ax - LEFT) / self.pps) as f64;
        self.pps = (self.pps * factor).clamp(15.0, 1500.0);
        self.set_scroll(t - ((ax - LEFT) / self.pps) as f64, p);
    }

    pub fn set_playhead(&mut self, t: f64, p: &Project) {
        self.playhead = t;
        if self.follow && self.drag.is_none() {
            let (a, span) = (self.scroll_t, self.span());
            if t > a + span * 0.8 || t < a {
                self.set_scroll(t - span * 0.2, p);
            }
        }
    }

    fn grid_step(&self, t: f64) -> f64 {
        if self.subs.len() < 2 {
            return 0.05;
        }
        let k = self.subs.partition_point(|&g| g < t).clamp(1, self.subs.len() - 1);
        self.subs[k] - self.subs[k - 1]
    }

    fn snap(&self, t: f64, free: bool) -> f64 {
        if free || self.subs.is_empty() {
            return t;
        }
        let k = self.subs.partition_point(|&g| g < t);
        let best = [k.wrapping_sub(1), k]
            .iter()
            .filter_map(|&i| self.subs.get(i).copied())
            .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
            .unwrap_or(t);
        if ((best - t).abs() as f32) * self.pps < 14.0 { best } else { t }
    }

    // ------------------------------------------------------------------ edits

    /// Applies an edit, recording history first. Returns true if the chart changed.
    pub fn apply(&mut self, action: Action, p: &mut Project, hist: &mut History) -> bool {
        let opens = p.open_strings();
        let sel: Vec<usize> = self.selected.iter().copied().filter(|&i| i < p.notes.len()).collect();
        let needs_sel = !matches!(action, Action::Refret { only_selected: false } | Action::SetDownbeat(_) | Action::SelectAll | Action::SelectNone);
        if needs_sel && sel.is_empty() {
            return false;
        }
        match action {
            Action::SelectAll => {
                self.selected = (0..p.notes.len()).collect();
                return false;
            }
            Action::SelectNone => {
                self.selected.clear();
                return false;
            }
            Action::Refret { .. } if p.notes.is_empty() => return false,
            Action::SetDownbeat(b) if b >= p.grid.beats.len() => return false,
            _ => {}
        }
        hist.push(p);
        match action {
            Action::Delete => {
                let keep: Vec<Note> = p.notes.drain(..).enumerate().filter(|(i, _)| !self.selected.contains(i)).map(|(_, n)| n).collect();
                p.notes = keep;
                self.selected.clear();
            }
            Action::SetFret(f) => {
                for &i in &sel {
                    let n = &mut p.notes[i];
                    n.fret = f.clamp(0, MAX_FRET);
                    n.midi = opens[n.string as usize % 4] + n.fret;
                    n.shifted = false;
                    n.confidence = 1.0;
                }
            }
            Action::Transpose(d) => {
                for &i in &sel {
                    let n = &mut p.notes[i];
                    if (0..=MAX_FRET).contains(&(n.fret + d)) {
                        n.fret += d;
                        n.midi += d;
                        n.confidence = 1.0;
                    }
                }
            }
            Action::MoveString(d) => {
                for &i in &sel {
                    let n = &mut p.notes[i];
                    let s = n.string + d;
                    if (0..4).contains(&s) && (0..=MAX_FRET).contains(&(n.midi - opens[s as usize])) {
                        n.string = s;
                        n.fret = n.midi - opens[s as usize];
                    }
                }
            }
            Action::Nudge(dir, beat) => {
                let tempo = if p.grid.tempo > 0.0 { p.grid.tempo } else { 120.0 };
                for &i in &sel {
                    let step = if beat { 60.0 / tempo } else { self.grid_step(p.notes[i].start) };
                    let n = &mut p.notes[i];
                    let d = dir as f64 * step;
                    n.start = (n.start + d).max(0.0);
                    n.end = (n.end + d).max(0.01);
                }
                sort_keep(p, &mut self.selected);
            }
            Action::Toggle(t) => {
                // one state for the whole selection: off if every note has it, otherwise on
                let all = sel.iter().all(|&i| t.is_on(&p.notes[i]));
                for &i in &sel {
                    let next = p.notes.get(i + 1).cloned();
                    let n = &mut p.notes[i];
                    match t {
                        Technique::Slide => {
                            n.slide_to = if all {
                                -1
                            } else if n.slide_to >= 0 {
                                n.slide_to
                            } else {
                                match next {
                                    Some(m) if m.string == n.string && m.fret != n.fret => m.fret,
                                    _ => (n.fret + 2).min(MAX_FRET),
                                }
                            }
                        }
                        Technique::HammerOn => {
                            n.hammer_on = !all;
                            if n.hammer_on {
                                n.pull_off = false;
                            }
                        }
                        Technique::PullOff => {
                            n.pull_off = !all;
                            if n.pull_off {
                                n.hammer_on = false;
                            }
                        }
                        Technique::Mute => n.mute = !all,
                        Technique::Accent => n.accent = !all,
                        Technique::SlideOut => n.slide_unpitch_to = if all { -1 } else { (n.fret + 5).min(MAX_FRET) },
                        Technique::Bend => n.bend = if all { 0.0 } else { 1.0 },
                        Technique::Harmonic => n.harmonic = !all,
                        Technique::PalmMute => n.palm_mute = !all,
                        Technique::Tremolo => n.tremolo = !all,
                        Technique::Vibrato => n.vibrato = !all,
                        Technique::Slap => {
                            n.slap = !all;
                            n.pop &= all;
                        }
                        Technique::Pop => {
                            n.pop = !all;
                            n.slap &= all;
                        }
                        Technique::Tap => n.tap = !all,
                        Technique::LinkNext => n.link_next = !all,
                    }
                }
            }
            Action::SlideTarget(d) => {
                for &i in &sel {
                    let n = &mut p.notes[i];
                    if n.slide_to >= 0 {
                        n.slide_to = (n.slide_to + d).clamp(1, MAX_FRET);
                    }
                    if n.slide_unpitch_to >= 0 {
                        n.slide_unpitch_to = (n.slide_unpitch_to + d).clamp(0, MAX_FRET);
                    }
                    if n.bend > 0.0 {
                        n.bend = (n.bend + 0.5 * d as f32).clamp(0.5, 3.0);
                    }
                }
            }
            Action::Refret { only_selected } => {
                let locked: HashSet<usize> =
                    if only_selected { (0..p.notes.len()).filter(|i| !self.selected.contains(i)).collect() } else { HashSet::new() };
                fw_detect::fretting::assign(&mut p.notes, &opens, &locked);
            }
            Action::SetDownbeat(beat) => {
                let g = &mut p.grid;
                // dominant bar length so far
                let mut counts = std::collections::BTreeMap::new();
                for w in g.downbeats.windows(2) {
                    *counts.entry(w[1] - w[0]).or_insert(0usize) += 1;
                }
                let meter = counts.iter().fold((4usize, 0usize), |best, (&k, &c)| if c > best.1 && k > 0 { (k, c) } else { best }).0;
                // earlier bar lines stay; from here on bars are re-counted. Before the
                // first bar line (count-in) the meter continues back to the start.
                let mut set: BTreeSet<usize> = g.downbeats.iter().copied().filter(|&d| d + meter <= beat).collect();
                if set.is_empty() {
                    set.extend((beat % meter..beat).step_by(meter));
                }
                set.extend((beat..g.beats.len()).step_by(meter));
                g.downbeats = set.into_iter().collect();
            }
            Action::SelectAll | Action::SelectNone => {}
        }
        p.notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.refresh_grid(Some(p));
        true
    }

    fn add_note(&mut self, t: f64, string: i32, p: &mut Project, hist: &mut History) {
        let opens = p.open_strings();
        let fret = p.notes.iter().rfind(|n| n.string == string && n.start <= t).map_or(0, |n| n.fret);
        let len = (self.grid_step(t) * 2.0).max(0.12);
        hist.push(p);
        let mut n = Note::new(t, t + len, opens[string as usize] + fret);
        n.string = string;
        n.fret = fret;
        n.confidence = 1.0;
        p.notes.push(n);
        self.selected = [p.notes.len() - 1].into();
        sort_keep(p, &mut self.selected);
    }

    // ------------------------------------------------------------------- ui

    pub fn ui(&mut self, ui: &mut Ui, project: Option<&mut Project>, hist: &mut History) -> Events {
        let mut ev = Events::default();
        let full = ui.available_rect_before_wrap();
        let (rect, scroll_rect) = full.split_top_bottom_at_y(full.bottom() - SCROLL_H);
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(full);
        painter.rect_filled(full, 0.0, theme::BG);
        self.width = rect.width();
        let Some(p) = project else {
            painter.text(rect.center(), Align2::CENTER_CENTER, "Create or open a project to start  (File > New Project)", FontId::proportional(15.0), theme::DIM);
            return ev;
        };
        if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
            resp.request_focus();
        }
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });

        let lane_h = ((rect.height() - RULER_H - WAVE_H - 6.0) / 4.0).clamp(LANE_MIN, LANE_MAX);
        let lanes_top = rect.top() + RULER_H + WAVE_H;
        let t2x = |t: f64, s: &Editor| rect.left() + LEFT + ((t - s.scroll_t) as f32) * s.pps;
        let x2t = |x: f32, s: &Editor| s.scroll_t + ((x - rect.left() - LEFT) / s.pps) as f64;
        let lane_y = |string: i32| lanes_top + (3 - string) as f32 * lane_h; // G on top
        let y2string = |y: f32| -> Option<i32> {
            let k = ((y - lanes_top) / lane_h).floor();
            (0.0..4.0).contains(&k).then(|| 3 - k as i32)
        };
        let note_rect = |n: &Note, s: &Editor| {
            let x0 = t2x(n.start, s);
            let x1 = t2x(n.end, s).max(x0 + MIN_NOTE_PX);
            let y = lane_y(n.string);
            Rect::from_min_max(pos2(x0, y + 5.0), pos2(x1, y + lane_h - 5.0))
        };
        let note_at = |pos: Pos2, s: &Editor, p: &Project| (0..p.notes.len()).rev().find(|&i| note_rect(&p.notes[i], s).contains(pos));
        let beat_at = |x: f32, s: &Editor, p: &Project| -> Option<usize> {
            let t = x2t(x, s);
            let k = p.grid.beats.iter().enumerate().min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()))?.0;
            ((t2x(p.grid.beats[k], s) - x).abs() < 5.0).then_some(k)
        };

        // ---------------------------------------------------------- input
        let (pressed, down, released, mods, hover, scroll, zoom) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.modifiers,
                i.pointer.interact_pos(),
                i.smooth_scroll_delta,
                i.zoom_delta(),
            )
        });
        // a menu or popup over the editor owns the pointer: its clicks must not also edit the chart
        let covered = hover.is_some_and(|h| ui.ctx().layer_id_at(h).is_some_and(|l| l != ui.layer_id()));
        let pressed = pressed && !covered;
        let in_editor = hover.is_some_and(|h| rect.contains(h)) && !covered;
        let in_scrollbar = hover.is_some_and(|h| scroll_rect.contains(h)) && !covered;

        if in_editor && (zoom - 1.0).abs() > 1e-3 {
            self.zoom(zoom, hover.map(|h| h.x - rect.left()), p);
        } else if (in_editor || in_scrollbar) && (scroll.x != 0.0 || scroll.y != 0.0) {
            let d = if scroll.x.abs() > scroll.y.abs() { scroll.x } else { scroll.y };
            let t = self.scroll_t - (d / self.pps) as f64;
            self.set_scroll(t, p);
        }

        // scrollbar
        let dur = duration(p);
        let thumb = |s: &Editor| {
            let total = dur.max(s.span());
            let w = (s.span() / total) as f32 * scroll_rect.width();
            let x = scroll_rect.left() + (s.scroll_t / total) as f32 * scroll_rect.width();
            Rect::from_min_size(pos2(x, scroll_rect.top() + 2.0), vec2(w.max(20.0), SCROLL_H - 4.0))
        };

        if pressed && let Some(pos) = hover {
            if scroll_rect.contains(pos) {
                let th = thumb(self);
                let total = dur.max(self.span());
                if !th.contains(pos) {
                    let t = (pos.x - scroll_rect.left()) as f64 / scroll_rect.width() as f64 * total - self.span() / 2.0;
                    self.set_scroll(t, p);
                }
                let grab = (pos.x - thumb(self).left()) as f64 / scroll_rect.width() as f64 * total;
                self.drag = Some(Drag::Scrollbar { grab });
            } else if rect.contains(pos) {
                if pos.y < lanes_top {
                    ev.seek = Some(x2t(pos.x, self).max(0.0));
                    self.drag = Some(Drag::Seek);
                } else {
                    let additive = mods.shift || mods.command;
                    match note_at(pos, self, p) {
                        None => {
                            if !additive {
                                self.selected.clear();
                            }
                            self.drag = Some(Drag::Rubber { origin: pos, pos, base: self.selected.clone() });
                        }
                        Some(idx) => {
                            if additive {
                                if !self.selected.remove(&idx) {
                                    self.selected.insert(idx);
                                }
                            } else if !self.selected.contains(&idx) {
                                self.selected = [idx].into();
                            }
                            let r = note_rect(&p.notes[idx], self);
                            let resize = pos.x > r.right() - 6.0 && r.width() > 20.0;
                            let orig: Vec<Orig> = self
                                .selected
                                .iter()
                                .filter_map(|&i| {
                                    p.notes.get(i).map(|n| Orig { idx: i, start: n.start, end: n.end, string: n.string, fret: n.fret, midi: n.midi })
                                })
                                .collect();
                            let anchor = orig.iter().position(|o| o.idx == idx).unwrap_or(0);
                            self.drag = Some(Drag::Note { resize, origin: pos, anchor, started: false, orig });
                        }
                    }
                }
            }
        }

        if down && let Some(pos) = hover {
            let free = mods.alt;
            let opens = p.open_strings();
            let pps = self.pps;
            let snapped = |s: &Editor, t: f64| s.snap(t, free);
            let mut drag = self.drag.take();
            match &mut drag {
                Some(Drag::Seek) => ev.seek = Some(x2t(pos.x, self).max(0.0)),
                Some(Drag::Scrollbar { grab }) => {
                    let total = dur.max(self.span());
                    let t = (pos.x - scroll_rect.left()) as f64 / scroll_rect.width() as f64 * total - *grab;
                    self.set_scroll(t, p);
                }
                Some(Drag::Rubber { origin, pos: cur, base }) => {
                    *cur = pos;
                    let r = Rect::from_two_pos(*origin, pos);
                    let base = base.clone();
                    let hit: BTreeSet<usize> = (0..p.notes.len()).filter(|&i| r.intersects(note_rect(&p.notes[i], self))).collect();
                    self.selected = base.union(&hit).copied().collect();
                }
                Some(Drag::Note { resize, origin, anchor, started, orig }) => {
                    if !*started && (pos - *origin).length() >= 4.0 {
                        hist.push(p);
                        *started = true;
                    }
                    if *started {
                        let (resize, origin, a) = (*resize, *origin, orig[*anchor].clone());
                        let orig = orig.clone();
                        let dt = ((pos.x - origin.x) / pps) as f64;
                        if !resize {
                            let shift = snapped(self, a.start + dt) - a.start;
                            let ds = y2string(pos.y).map_or(0, |s| s - a.string);
                            for o in &orig {
                                let n = &mut p.notes[o.idx];
                                n.start = (o.start + shift).max(0.0);
                                n.end = (o.end + shift).max(0.01);
                                let s2 = (o.string + ds).clamp(0, 3);
                                if (0..=MAX_FRET).contains(&(o.midi - opens[s2 as usize])) {
                                    n.string = s2;
                                    n.fret = o.midi - opens[s2 as usize];
                                } else {
                                    n.string = o.string;
                                    n.fret = o.fret;
                                }
                            }
                        } else {
                            let end = snapped(self, a.end + dt);
                            for o in &orig {
                                let n = &mut p.notes[o.idx];
                                let e = if o.idx == a.idx { end } else { o.end + (end - a.end) };
                                n.end = e.max(n.start + 0.03);
                            }
                        }
                    }
                }
                None => {}
            }
            self.drag = drag;
        }

        if released {
            if let Some(Drag::Note { started: true, .. }) = self.drag {
                sort_keep(p, &mut self.selected);
                ev.changed = true;
            }
            self.drag = None;
        }

        if resp.double_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(s) = y2string(pos.y)
            && note_at(pos, self, p).is_none()
        {
            let t = self.snap(x2t(pos.x, self), false);
            self.add_note(t, s, p, hist);
            self.drag = None;
            ev.changed = true;
        }

        // cursor
        if self.drag.is_none()
            && let Some(pos) = hover
            && let Some(i) = note_at(pos, self, p)
            && pos.x > note_rect(&p.notes[i], self).right() - 6.0
        {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        // context menu
        if resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            if let Some(i) = note_at(pos, self, p)
                && !self.selected.contains(&i)
            {
                self.selected = [i].into();
            }
            self.menu_beat = beat_at(pos.x, self, p);
        }
        let mut menu_action = None;
        let has_sel = !self.selected.is_empty();
        let menu_beat = self.menu_beat;
        let all_on = |t: &Technique| has_sel && self.selected.iter().filter_map(|&i| p.notes.get(i)).all(|n| t.is_on(n));
        let common_on: Vec<bool> = Technique::COMMON.iter().map(|(t, _)| all_on(t)).collect();
        let more_on: Vec<bool> = Technique::MORE.iter().map(|(t, _)| all_on(t)).collect();
        let has_slide = self.selected.iter().filter_map(|&i| p.notes.get(i)).any(|n| n.slide_to >= 0 || n.slide_unpitch_to >= 0 || n.bend > 0.0);
        resp.context_menu(|ui| {
            if has_sel {
                if ui.button("Delete notes").clicked() {
                    menu_action = Some(Action::Delete);
                }
                if ui.button("Re-fret selected notes").clicked() {
                    menu_action = Some(Action::Refret { only_selected: true });
                }
                ui.separator();
                for ((t, label), mut on) in Technique::COMMON.into_iter().zip(common_on) {
                    if ui.checkbox(&mut on, label).clicked() {
                        menu_action = Some(Action::Toggle(t));
                    }
                }
                ui.menu_button("More techniques", |ui| {
                    for ((t, label), mut on) in Technique::MORE.into_iter().zip(more_on) {
                        if ui.checkbox(&mut on, label).clicked() {
                            menu_action = Some(Action::Toggle(t));
                            ui.close();
                        }
                    }
                });
                if has_slide {
                    if ui.button("Slide target / bend height +  ( ] )").clicked() {
                        menu_action = Some(Action::SlideTarget(1));
                    }
                    if ui.button("Slide target / bend height -  ( [ )").clicked() {
                        menu_action = Some(Action::SlideTarget(-1));
                    }
                }
            }
            if let Some(b) = menu_beat {
                ui.separator();
                if ui.button("Make this beat a bar line (re-count bars from here)").clicked() {
                    menu_action = Some(Action::SetDownbeat(b));
                }
            }
            if !has_sel && menu_beat.is_none() {
                ui.label("Right-click a note or a beat line");
            }
            if menu_action.is_some() {
                ui.close();
            }
        });
        if let Some(a) = menu_action {
            ev.changed |= self.apply(a, p, hist);
        }

        // keys
        if resp.has_focus() {
            let now = ui.input(|i| i.time);
            let events = ui.input(|i| i.events.clone());
            for e in events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = e
                    && let Some(a) = self.key_action(key, modifiers, now)
                {
                    ev.changed |= self.apply(a, p, hist);
                }
            }
        }

        // ---------------------------------------------------------- paint
        let (t0, t1) = (self.scroll_t, self.scroll_t + self.span());
        for s in 0..4 {
            let y = lane_y(s);
            let lane = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), lane_h));
            painter.rect_filled(lane, 0.0, if s % 2 == 1 { theme::LANE } else { theme::LANE_ALT });
            painter.line_segment([pos2(rect.left() + LEFT, y + lane_h / 2.0), pos2(rect.right(), y + lane_h / 2.0)], Stroke::new(1.0, theme::STRING_LINE));
        }

        // waveform
        let wave = Rect::from_min_size(pos2(rect.left(), rect.top() + RULER_H), vec2(rect.width(), WAVE_H));
        painter.rect_filled(wave, 0.0, theme::WAVE_BG);
        if let Some((peaks, prate)) = &self.peaks {
            let mid = wave.center().y;
            let mut x = rect.left() + LEFT;
            while x < rect.right() {
                let ta = x2t(x, self);
                let tb = x2t(x + 2.0, self);
                let (a, b) = ((ta * prate).max(0.0) as usize, ((tb * prate).max(0.0) as usize).max((ta * prate).max(0.0) as usize + 1));
                let v = peaks.get(a.min(peaks.len())..b.min(peaks.len())).map_or(0.0, |w| w.iter().copied().fold(0.0, f32::max));
                if v > 0.0 {
                    let h = v * (WAVE_H / 2.0 - 3.0);
                    painter.line_segment([pos2(x, mid - h), pos2(x, mid + h)], Stroke::new(2.0, theme::WAVE));
                }
                x += 2.0;
            }
        }

        // grid
        let bottom = lanes_top + 4.0 * lane_h;
        if !self.subs.is_empty() && self.pps * self.grid_step(t0) as f32 > 10.0 {
            let a = self.subs.partition_point(|&g| g < t0);
            let b = self.subs.partition_point(|&g| g <= t1);
            for &t in &self.subs[a..b] {
                let x = t2x(t, self);
                painter.line_segment([pos2(x, lanes_top), pos2(x, bottom)], Stroke::new(1.0, theme::SUB));
            }
        }
        let ruler = Rect::from_min_size(rect.min, vec2(rect.width(), RULER_H));
        painter.rect_filled(ruler, 0.0, theme::RULER);
        let beats = &p.grid.beats;
        let downs: HashSet<usize> = p.grid.downbeats.iter().copied().collect();
        let bar_no: std::collections::HashMap<usize, usize> = p.grid.downbeats.iter().enumerate().map(|(k, &i)| (i, k + 1)).collect();
        let a = beats.partition_point(|&b| b < t0).saturating_sub(1);
        let b = (beats.partition_point(|&b| b <= t1) + 1).min(beats.len());
        for (i, &beat) in beats.iter().enumerate().take(b).skip(a) {
            let x = t2x(beat, self);
            if x < rect.left() + LEFT {
                continue;
            }
            let is_bar = downs.contains(&i);
            let top = if is_bar { rect.top() + RULER_H } else { lanes_top };
            painter.line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(if is_bar { 1.4 } else { 1.0 }, if is_bar { theme::BAR } else { theme::BEAT }));
            if let Some(n) = bar_no.get(&i) {
                painter.text(pos2(x + 3.0, rect.top() + 4.0), Align2::LEFT_TOP, n.to_string(), FontId::proportional(12.0), theme::DIM);
            }
        }
        if beats.is_empty() {
            for s in t0 as i64..=t1 as i64 {
                painter.text(pos2(t2x(s as f64, self) + 2.0, rect.top() + 4.0), Align2::LEFT_TOP, format!("{s}s"), FontId::proportional(12.0), theme::DIM);
            }
        }

        // notes
        let start_from = p.notes.partition_point(|n| n.start < t0 - 10.0);
        for i in start_from..p.notes.len() {
            let n = &p.notes[i];
            if n.start > t1 {
                break;
            }
            if n.end < t0 {
                continue;
            }
            let r = note_rect(n, self);
            let mut col = STRING_COLORS[n.string.rem_euclid(4) as usize];
            if n.confidence < 0.3 {
                col = col.gamma_multiply(0.6);
            }
            let dark = Color32::from_rgb(col.r() / 2 + 8, col.g() / 2 + 8, col.b() / 2 + 8);
            painter.rect_filled(r, CornerRadius::same(4), dark);
            let head = Rect::from_min_size(r.min, vec2(r.width().min(22.0), r.height()));
            painter.rect_filled(head, CornerRadius::same(4), col);
            if self.selected.contains(&i) {
                painter.rect_stroke(r.expand(1.0), CornerRadius::same(5), Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
            }
            let label = if n.mute { "X".to_string() } else { n.fret.to_string() };
            painter.text(head.center(), Align2::CENTER_CENTER, label, FontId::proportional(13.0), Color32::from_rgb(0x11, 0x11, 0x11));
            draw_techniques(&painter, n, r, head);
        }

        // string names
        painter.rect_filled(Rect::from_min_size(pos2(rect.left(), lanes_top), vec2(LEFT - 2.0, 4.0 * lane_h)), 0.0, theme::BG);
        let opens = p.open_strings();
        for s in 0..4 {
            painter.text(
                pos2(rect.left() + (LEFT - 4.0) / 2.0, lane_y(s) + lane_h / 2.0),
                Align2::CENTER_CENTER,
                note_name(opens[s as usize], false),
                FontId::proportional(14.0),
                STRING_COLORS[s as usize],
            );
        }

        // playhead
        let x = t2x(self.playhead, self);
        if x >= rect.left() + LEFT && x <= rect.right() {
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, Color32::WHITE));
        }

        if let Some(Drag::Rubber { origin, pos, .. }) = &self.drag {
            let r = Rect::from_two_pos(*origin, *pos);
            painter.rect_filled(r, 0.0, theme::RUBBER);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(120, 170, 255)), StrokeKind::Inside);
        }

        // scrollbar
        painter.rect_filled(scroll_rect, 0.0, theme::RULER);
        painter.rect_filled(thumb(self), CornerRadius::same(4), Color32::from_rgb(0x3c, 0x42, 0x50));

        if self.drag.is_some() {
            ui.ctx().request_repaint();
        }
        ev
    }

    fn key_action(&mut self, key: Key, m: Modifiers, now: f64) -> Option<Action> {
        let digit = match key {
            Key::Num0 => Some(0),
            Key::Num1 => Some(1),
            Key::Num2 => Some(2),
            Key::Num3 => Some(3),
            Key::Num4 => Some(4),
            Key::Num5 => Some(5),
            Key::Num6 => Some(6),
            Key::Num7 => Some(7),
            Key::Num8 => Some(8),
            Key::Num9 => Some(9),
            _ => None,
        };
        if let Some(d) = digit
            && !m.command
        {
            let (prev, t) = &self.digits;
            let combined = format!("{prev}{d}");
            let text = if !prev.is_empty() && now - t < 0.7 && combined.parse::<i32>().is_ok_and(|v| v <= MAX_FRET) { combined } else { d.to_string() };
            let fret = text.parse().unwrap_or(0);
            self.digits = (text, now);
            return Some(Action::SetFret(fret));
        }
        Some(match key {
            Key::Delete | Key::Backspace => Action::Delete,
            Key::ArrowUp if m.shift => Action::MoveString(1),
            Key::ArrowUp => Action::Transpose(1),
            Key::ArrowDown if m.shift => Action::MoveString(-1),
            Key::ArrowDown => Action::Transpose(-1),
            Key::ArrowLeft => Action::Nudge(-1, m.shift),
            Key::ArrowRight => Action::Nudge(1, m.shift),
            Key::H => Action::Toggle(Technique::HammerOn),
            Key::P => Action::Toggle(Technique::PullOff),
            Key::M => Action::Toggle(Technique::Mute),
            Key::A if m.command => Action::SelectAll,
            Key::A => Action::Toggle(Technique::Accent),
            Key::S if !m.command => Action::Toggle(Technique::Slide),
            Key::OpenBracket => Action::SlideTarget(-1),
            Key::CloseBracket => Action::SlideTarget(1),
            Key::Escape => Action::SelectNone,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let mut p = Project::new("/tmp/x", "a.mp3");
        for (i, (s, f)) in [(1, 0), (1, 2), (2, 3)].iter().enumerate() {
            let mut n = Note::new(i as f64, i as f64 + 0.5, p.open_strings()[*s as usize] + f);
            n.string = *s;
            n.fret = *f;
            p.notes.push(n);
        }
        p
    }

    #[test]
    fn edits_apply_and_undo() {
        let (mut e, mut p, mut h) = (Editor::default(), project(), History::default());
        e.selected = [0].into();
        assert!(e.apply(Action::SetFret(5), &mut p, &mut h));
        assert_eq!((p.notes[0].fret, p.notes[0].midi), (5, 38));
        assert!(e.apply(Action::MoveString(1), &mut p, &mut h));
        assert_eq!((p.notes[0].string, p.notes[0].fret), (2, 0));
        assert!(e.apply(Action::Toggle(Technique::Slide), &mut p, &mut h));
        assert_eq!(p.notes[0].slide_to, 2);
        assert!(e.apply(Action::Delete, &mut p, &mut h));
        assert_eq!(p.notes.len(), 2);
        assert!(h.undo(&mut p) && h.undo(&mut p) && h.undo(&mut p) && h.undo(&mut p));
        assert_eq!((p.notes.len(), p.notes[0].fret), (3, 0));
    }

    #[test]
    fn techniques_toggle_as_a_group_and_slides_retarget() {
        let (mut e, mut p, mut h) = (Editor::default(), project(), History::default());
        p.notes[0].hammer_on = true;
        e.selected = [0, 1].into();
        e.apply(Action::Toggle(Technique::HammerOn), &mut p, &mut h); // mixed -> all on
        assert!(p.notes[0].hammer_on && p.notes[1].hammer_on);
        e.apply(Action::Toggle(Technique::HammerOn), &mut p, &mut h); // all on -> all off
        assert!(!p.notes[0].hammer_on && !p.notes[1].hammer_on);

        e.selected = [1].into();
        e.apply(Action::Toggle(Technique::Slide), &mut p, &mut h);
        assert!(p.notes[1].slide_to >= 0);
        e.apply(Action::SlideTarget(-100), &mut p, &mut h);
        assert_eq!(p.notes[1].slide_to, 1);
        e.apply(Action::SlideTarget(100), &mut p, &mut h);
        assert_eq!(p.notes[1].slide_to, MAX_FRET);
        e.selected = [0].into();
        e.apply(Action::SlideTarget(3), &mut p, &mut h); // not a slide: untouched
        assert_eq!(p.notes[0].slide_to, -1);
    }

    #[test]
    fn the_other_techniques_toggle_and_bends_resize() {
        let (mut e, mut p, mut h) = (Editor::default(), project(), History::default());
        e.selected = [0].into();
        for (t, _) in Technique::COMMON.iter().chain(Technique::MORE.iter()) {
            e.apply(Action::Toggle(*t), &mut p, &mut h);
            assert!(t.is_on(&p.notes[0]), "{t:?} should turn on");
        }
        // slap and pop exclude each other: the later toggle won
        assert!(p.notes[0].pop && !p.notes[0].slap);
        e.apply(Action::SlideTarget(2), &mut p, &mut h);
        assert_eq!(p.notes[0].bend, 2.0);
        e.apply(Action::SlideTarget(-100), &mut p, &mut h);
        assert_eq!(p.notes[0].bend, 0.5);
        for (t, _) in Technique::COMMON.iter().chain(Technique::MORE.iter()) {
            e.apply(Action::Toggle(*t), &mut p, &mut h);
        }
    }

    #[test]
    fn nudging_keeps_selection_on_the_moved_note() {
        let (mut e, mut p, mut h) = (Editor::default(), project(), History::default());
        e.selected = [0].into();
        e.apply(Action::Nudge(1, true), &mut p, &mut h); // +0.5 s at 120 BPM
        e.apply(Action::Nudge(1, true), &mut p, &mut h);
        e.apply(Action::Nudge(1, true), &mut p, &mut h); // now at 1.5 s, after note 1
        assert_eq!(e.selected, [1].into());
        assert_eq!(p.notes[1].start, 1.5);
    }

    #[test]
    fn digits_combine_into_two_digit_frets() {
        let mut e = Editor::default();
        assert_eq!(e.key_action(Key::Num1, Modifiers::NONE, 0.0), Some(Action::SetFret(1)));
        assert_eq!(e.key_action(Key::Num2, Modifiers::NONE, 0.3), Some(Action::SetFret(12)));
        assert_eq!(e.key_action(Key::Num3, Modifiers::NONE, 2.0), Some(Action::SetFret(3)));
        assert_eq!(e.key_action(Key::Num9, Modifiers::NONE, 2.2), Some(Action::SetFret(9))); // 39 > 24
    }

    #[test]
    fn note_names() {
        assert_eq!(note_name(28, false), "E");
        assert_eq!(note_name(33, true), "A1");
    }
}
