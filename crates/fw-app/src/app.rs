//! Main window: menus, transport, the editor and the Song / Detection / Build panels.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Color32, Key, KeyboardShortcut, Modifiers, RichText, Ui};
use fw_detect::pipeline::Detection;
use fw_project::{DetectionSettings, Project, TUNINGS};

use crate::editor::{Action, Editor, SHORTCUTS, note_name};
use crate::history::History;
use crate::player::{Mode, Player};

const AUDIO_EXT: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac", "opus"];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];

/// (id, label, description)
pub const SEPARATORS: &[(&str, &str, &str)] = &[
    ("htdemucs_ft", "Demucs v4 fine-tuned", "Best bass quality of the Demucs family. Only its bass network runs."),
    ("htdemucs", "Demucs v4", "Fast general 4-stem model (drums, bass, other, vocals)."),
    (
        "htdemucs_6s",
        "Demucs v4 6-stem",
        "Adds guitar and piano stems. Bass is slightly less clean than the fine-tuned model.",
    ),
    ("lowpass", "Low-pass filter (no AI)", "Instant, but kick drum and other instruments leak in. Poor accuracy."),
];
pub const PITCH: &[(&str, &str, &str)] = &[
    ("net", "Neural network", "A small network trained on official Rocksmith charts names the notes. Best overall."),
    ("harmonic", "Harmonic match", "Finds each note's pitch from the harmonics in its own audio. Works on distorted bass."),
    ("pyin", "pYIN", "Classic probabilistic YIN. Fast, reliable on a clean bass stem."),
];
pub const ONSETS: &[(&str, &str, &str)] = &[
    ("net", "Neural network", "A small network trained on official Rocksmith charts finds where notes start, backed by spectral flux. Best overall."),
    ("flux", "Spectral flux", "Peaks of the stem's spectral change. No AI; misses some quiet or repeated notes."),
];
pub const BEATS: &[(&str, &str, &str)] = &[
    ("beat_this", "Beat This! (neural)", "State-of-the-art transformer beat and downbeat tracker (ISMIR 2024). Finds bar lines too."),
    ("librosa", "Classic tracker", "Dynamic-programming beat tracker. No downbeats; weaker on intros."),
];
const DEVICES: &[(&str, &str)] = &[("auto", "Auto (GPU if available)"), ("gpu", "GPU"), ("cpu", "CPU")];
const SNAPS: &[(&str, u32)] = &[("No snapping", 0), ("1/4 notes", 1), ("1/8 notes", 2), ("1/8 triplets", 3), ("1/16 notes", 4), ("1/16 triplets", 6)];
const GRIDS: &[(&str, u32)] = &[("1/4", 1), ("1/8", 2), ("1/8T", 3), ("1/16", 4), ("1/16T", 6)];

fn label_of(items: &[(&str, &str, &str)], id: &str) -> String {
    items.iter().find(|i| i.0 == id).map_or(id.to_string(), |i| i.1.to_string())
}

use fw_project::guess_metadata;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Song,
    Detection,
    Build,
}

struct CompareRow {
    separator: String,
    pitch: String,
    detection: Detection,
    settings: DetectionSettings,
    agreement: f64,
    coverage: f64,
    seconds: f64,
}

enum Output {
    Models(Vec<PathBuf>),
    Detection(Detection, Box<DetectionSettings>),
    Build(Vec<PathBuf>, Box<Project>),
    Compare(Vec<CompareRow>),
}

enum Msg {
    Progress(String, f64),
    Done(Output),
    Failed(String),
}

struct Job {
    label: &'static str,
    rx: Receiver<Msg>,
    msg: String,
    frac: f64,
}

#[derive(Default)]
struct NewProject {
    audio: String,
    folder: String,
    artist: String,
    title: String,
    album: String,
    year: i32,
    art: String,
    tuning: usize,
    error: String,
}

impl NewProject {
    fn audio_chosen(&mut self, f: &Path) {
        self.audio = f.display().to_string();
        let (artist, title) = guess_metadata(f);
        if self.artist.is_empty() {
            self.artist = artist;
        }
        if self.title.is_empty() {
            self.title = title;
        }
        if self.folder.is_empty() {
            self.folder = fw_project::default_folder(f).display().to_string();
        }
        if self.art.is_empty()
            && let Some(a) = ["jpg", "jpeg", "png", "webp"].iter().map(|e| f.with_extension(e)).find(|p| p.is_file())
        {
            self.art = a.display().to_string();
        }
    }
}

struct Compare {
    separators: Vec<bool>,
    rows: Vec<CompareRow>,
    selected: Option<usize>,
}

enum After {
    New(Option<PathBuf>),
    Open(Option<PathBuf>),
    /// An official Rocksmith chart (.psarc or arrangement .xml).
    ImportChart,
    Quit,
}

enum Dialog {
    NewProject(NewProject),
    Compare(Compare),
    Message(String, String),
    ConfirmDiscard(After),
    ConfirmReplace,
    /// Offer to download these model files
    Models(Vec<&'static fw_models::download::Asset>),
}

pub struct App {
    project: Option<Project>,
    dirty: bool,
    history: History,
    editor: Editor,
    player: Player,
    tab: Tab,
    build_pc: bool,
    build_mac: bool,
    out_dir: String,
    log: String,
    status: Option<(String, f64)>,
    job: Option<Job>,
    dialog: Option<Dialog>,
    recent: Vec<String>,
    art: Option<(String, Option<egui::TextureHandle>)>,
    allow_close: bool,
    volume: f32,
    ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> App {
        style(&cc.egui_ctx);
        let recent = cc.storage.and_then(|s| eframe::get_value(s, "recent")).unwrap_or_default();
        let mut app = App {
            project: None,
            dirty: false,
            history: History::default(),
            editor: Editor::default(),
            player: Player::new(),
            tab: Tab::Detection,
            build_pc: true,
            build_mac: false,
            out_dir: String::new(),
            log: String::new(),
            status: None,
            job: None,
            dialog: None,
            recent,
            art: None,
            allow_close: false,
            volume: 1.0,
            ctx: cc.egui_ctx.clone(),
        };
        let missing = fw_models::download::missing_defaults();
        if !missing.is_empty() {
            app.dialog = Some(Dialog::Models(missing));
        }
        if let Some(e) = &app.player.error {
            app.log.push_str(&format!("Audio output unavailable: {e}\n"));
        }
        if let Some(path) = open {
            if path.is_dir() || path.extension().is_some_and(|e| e == "json") {
                app.open(&path);
            } else if path.is_file() {
                let mut np = NewProject { year: 2026, ..Default::default() };
                np.audio_chosen(&path);
                app.dialog = Some(Dialog::NewProject(np));
            }
        }
        app
    }

    fn set_status(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.status = Some((text.into(), ctx.input(|i| i.time) + 8.0));
    }

    fn message(&mut self, title: &str, text: impl Into<String>) {
        self.dialog = Some(Dialog::Message(title.into(), text.into()));
    }

    // ------------------------------------------------------------ project

    fn best_stem(&self) -> Option<PathBuf> {
        let p = self.project.as_ref()?;
        std::iter::once(p.detection.separator.as_str())
            .chain(SEPARATORS.iter().map(|s| s.0))
            .map(|s| fw_detect::pipeline::stem_path(&p.folder, s))
            .find(|f| f.is_file())
    }

    fn set_project(&mut self, p: Option<Project>) {
        self.project = p;
        self.history.clear();
        self.editor.reset();
        self.editor.refresh_grid(self.project.as_ref());
        self.editor.set_peaks(None);
        self.dirty = false;
        self.art = None;
        let stem = self.best_stem();
        let audio = self.project.as_ref().map(|p| PathBuf::from(&p.audio_path));
        self.player.set_sources(audio, stem);
    }

    fn remember(&mut self, folder: &Path) {
        let f = folder.display().to_string();
        self.recent.retain(|r| *r != f);
        self.recent.insert(0, f);
        self.recent.truncate(10);
    }

    fn open(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(p) => {
                self.remember(&p.folder.clone());
                self.set_project(Some(p));
            }
            Err(e) => self.message("Could not open project", e.to_string()),
        }
    }

    /// Opens an official chart as a new project, with its audio and every technique.
    fn import_chart(&mut self, ctx: &egui::Context) {
        let Some(chart) = rfd::FileDialog::new().add_filter("Rocksmith chart", &["psarc", "xml"]).set_title("Open official chart").pick_file() else {
            return;
        };
        let is_xml = chart.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"));
        let audio = if is_xml {
            let Some(a) = rfd::FileDialog::new().add_filter("Audio", &["wav", "mp3", "flac", "ogg", "m4a"]).set_title("Pick the song this chart belongs to").pick_file() else {
                return;
            };
            Some(a)
        } else {
            None
        };
        let Some(parent) = rfd::FileDialog::new().set_title("Where should the project be saved?").pick_folder() else { return };
        let name = fw_project::default_folder(&chart).file_name().map(|n| n.to_os_string()).unwrap_or_default();
        let folder = parent.join(name);
        let result = match &audio {
            Some(a) => fw_project::import::import_xml(&chart, a, &folder),
            None => fw_project::import::import_psarc(&chart, &folder),
        };
        match result.and_then(|p| p.save().map(|_| p)) {
            Ok(p) => {
                let n = p.notes.len();
                self.remember(&p.folder.clone());
                self.set_project(Some(p));
                self.set_status(ctx, format!("Opened official chart ({n} notes)"));
            }
            Err(e) => self.message("Could not open the chart", e.to_string()),
        }
    }

    fn save(&mut self, ctx: &egui::Context) -> bool {
        let Some(p) = &self.project else { return false };
        match p.save() {
            Ok(_) => {
                self.dirty = false;
                self.set_status(ctx, "Saved");
                true
            }
            Err(e) => {
                self.message("Could not save", e.to_string());
                false
            }
        }
    }

    /// Runs `after` now, or asks about unsaved changes first.
    fn guarded(&mut self, ctx: &egui::Context, after: After) {
        if self.dirty {
            self.dialog = Some(Dialog::ConfirmDiscard(after));
        } else {
            self.run_after(ctx, after);
        }
    }

    fn run_after(&mut self, ctx: &egui::Context, after: After) {
        match after {
            After::New(audio) => {
                let mut np = NewProject { year: 2026, ..Default::default() };
                if let Some(a) = audio {
                    np.audio_chosen(&a);
                }
                self.dialog = Some(Dialog::NewProject(np));
            }
            After::Open(Some(path)) => self.open(&path),
            After::Open(None) => {
                if let Some(f) = rfd::FileDialog::new().add_filter("FretWright project", &["json"]).set_title("Open project").pick_file() {
                    self.open(&f);
                }
            }
            After::ImportChart => self.import_chart(ctx),
            After::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn chart_changed(&mut self) {
        self.dirty = true;
        self.player.chart_changed();
        self.editor.refresh_grid(self.project.as_ref());
    }

    fn undo(&mut self) {
        if let Some(p) = &mut self.project
            && self.history.undo(p)
        {
            self.editor.selected.clear();
            self.chart_changed();
        }
    }

    fn redo(&mut self) {
        if let Some(p) = &mut self.project
            && self.history.redo(p)
        {
            self.editor.selected.clear();
            self.chart_changed();
        }
    }

    fn act(&mut self, a: Action) {
        if let Some(p) = &mut self.project
            && self.editor.apply(a, p, &mut self.history)
        {
            self.chart_changed();
        }
    }

    fn shift_grid(&mut self, d: f64) {
        if let Some(p) = &mut self.project {
            self.history.push(p);
            p.grid.beats.iter_mut().for_each(|b| *b = (*b + d).max(0.0));
            self.chart_changed();
        }
    }

    fn set_tuning(&mut self, idx: usize) {
        let Some(p) = &mut self.project else { return };
        let tuning = TUNINGS[idx].1;
        if p.tuning == tuning {
            return;
        }
        self.history.push(p);
        p.tuning = tuning;
        // keep the sounding pitches, re-place them on the new tuning
        let opens = p.open_strings();
        fw_detect::notes::fit_to_range(&mut p.notes, &opens);
        fw_detect::fretting::assign(&mut p.notes, &opens, &Default::default());
        self.chart_changed();
    }

    fn export_xml(&mut self) {
        let Some(p) = &self.project else { return };
        if let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).set_directory(&p.folder).set_file_name("bass.xml").save_file()
            && let Err(e) = fw_project::to_arrangement(p).save(&f)
        {
            self.message("Export failed", e.to_string());
        }
    }

    fn open_folder(dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        let cmd = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(cmd).arg(dir).spawn();
    }

    // --------------------------------------------------------------- jobs

    fn spawn<F>(&mut self, ctx: &egui::Context, label: &'static str, work: F)
    where
        F: FnOnce(&(dyn Fn(&str, f64) + Sync)) -> Result<Output, String> + Send + 'static,
    {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let progress = |m: &str, f: f64| {
                let _ = tx.send(Msg::Progress(m.to_string(), f));
                ctx2.request_repaint();
            };
            // a panic in a model or the packager must not take the app down
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&progress)))
                .unwrap_or_else(|e| Err(e.downcast_ref::<String>().cloned().or(e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or("crashed".into())));
            let _ = tx.send(match out {
                Ok(o) => Msg::Done(o),
                Err(e) => Msg::Failed(e),
            });
            ctx2.request_repaint();
        });
        self.job = Some(Job { label, rx, msg: "Starting".into(), frac: 0.0 });
    }

    fn download_models(&mut self, ctx: &egui::Context, assets: Vec<&'static fw_models::download::Asset>) {
        self.spawn(ctx, "Model download", move |progress| {
            let total: u64 = assets.iter().map(|a| a.size).sum::<u64>().max(1);
            let mut before = 0u64;
            let mut paths = Vec::new();
            for a in assets {
                let path = fw_models::download::download(a, &mut |done, _| {
                    progress(&format!("Downloading {}", a.what), (before + done) as f64 / total as f64);
                })
                .map_err(|e| e.to_string())?;
                before += a.size;
                paths.push(path);
            }
            Ok(Output::Models(paths))
        });
    }

    fn run_detection(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.project else { return };
        self.player.stop();
        let (audio, settings, open, folder) = (PathBuf::from(&p.audio_path), p.detection.clone(), p.open_strings(), p.folder.clone());
        self.spawn(ctx, "Detection", move |progress| {
            fw_detect::pipeline::detect(&audio, &settings, &open, &folder, progress)
                .map(|d| Output::Detection(d, Box::new(settings)))
                .map_err(|e| e.to_string())
        });
    }

    fn run_build(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.project else { return };
        if p.notes.is_empty() {
            return self.message("Nothing to build", "Detect or add some notes first.");
        }
        let mut platforms = Vec::new();
        if self.build_pc {
            platforms.push(fw_package::Platform::Pc);
        }
        if self.build_mac {
            platforms.push(fw_package::Platform::Mac);
        }
        if platforms.is_empty() {
            return self.message("No platform", "Tick PC and/or Mac.");
        }
        let mut p = p.clone();
        self.save(ctx);
        let out = (!self.out_dir.trim().is_empty()).then(|| PathBuf::from(self.out_dir.trim()));
        self.log.push_str(&format!("Building {} - {} …\n", p.metadata.artist, p.metadata.title));
        self.tab = Tab::Build;
        self.spawn(ctx, "Build", move |progress| {
            let paths = fw_project::build(&mut p, out.as_deref(), &platforms, &|m, f| progress(m, f)).map_err(|e| e.to_string())?;
            Ok(Output::Build(paths, Box::new(p)))
        });
    }

    fn run_compare(&mut self, ctx: &egui::Context, separators: Vec<String>) {
        let Some(p) = &self.project else { return };
        let (audio, base, open, folder) = (PathBuf::from(&p.audio_path), p.detection.clone(), p.open_strings(), p.folder.clone());
        self.spawn(ctx, "Comparison", move |progress| {
            use fw_detect::pipeline;
            let mut rows = Vec::new();
            let n = separators.len() as f64;
            for (k, sep) in separators.iter().enumerate() {
                let mut s = base.clone();
                s.separator = sep.clone();
                let sub = |m: &str, f: f64| progress(&format!("[{}/{}] {sep}: {m}", k + 1, separators.len()), (k as f64 + f) / n);
                let t = std::time::Instant::now();
                let det = pipeline::detect(&audio, &s, &open, &folder, &sub).map_err(|e| e.to_string())?;
                rows.push(CompareRow {
                    separator: sep.clone(),
                    pitch: s.pitch.clone(),
                    detection: det,
                    settings: s,
                    agreement: 0.0,
                    coverage: 0.0,
                    seconds: t.elapsed().as_secs_f64(),
                });
            }
            // score every result against the best stem available
            let reference = pipeline::stem_path(&folder, "htdemucs_ft");
            if !reference.is_file() {
                let mut w = Vec::new();
                pipeline::separate(&audio, "htdemucs_ft", &base.device, &folder, &mut w, &|m, f| progress(m, f)).map_err(|e| e.to_string())?;
            }
            let bass = pipeline::load_mono(&reference).map_err(|e| e.to_string())?;
            for r in &mut rows {
                r.agreement = fw_detect::metrics::stem_agreement(&r.detection.notes, &bass, pipeline::SR);
                r.coverage = fw_detect::metrics::stem_coverage(&r.detection.notes, &bass, pipeline::SR);
            }
            Ok(Output::Compare(rows))
        });
    }

    fn apply_detection(&mut self, det: Detection, settings: DetectionSettings) {
        let Some(p) = &mut self.project else { return };
        self.history.push(p);
        p.notes = det.notes;
        p.grid = det.grid;
        p.duration = det.duration;
        p.detection = settings;
        self.editor.selected.clear();
        self.chart_changed();
        let stem = self.best_stem();
        let audio = self.project.as_ref().map(|p| PathBuf::from(&p.audio_path));
        self.player.set_sources(audio, stem);
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.job else { return };
        let mut finished = None;
        while let Ok(m) = job.rx.try_recv() {
            match m {
                Msg::Progress(msg, f) => {
                    job.msg = msg;
                    job.frac = f;
                }
                other => finished = Some(other),
            }
        }
        let label = job.label;
        match finished {
            Some(Msg::Done(Output::Models(paths))) => {
                self.job = None;
                self.log.push_str("Downloaded models:\n");
                for p in &paths {
                    self.log.push_str(&format!("{}\n", p.display()));
                }
                self.set_status(ctx, format!("Downloaded {} model file(s)", paths.len()));
            }
            Some(Msg::Done(Output::Detection(det, settings))) => {
                let settings = *settings;
                self.job = None;
                let (n, bpm) = (det.notes.len(), det.grid.tempo);
                let warnings = det.warnings.clone();
                self.apply_detection(det, settings);
                self.set_status(ctx, format!("Found {n} notes at {bpm:.1} BPM"));
                if !warnings.is_empty() {
                    self.message("Detection notes", warnings.join("\n\n"));
                }
            }
            Some(Msg::Done(Output::Build(paths, built))) => {
                self.job = None;
                if let Some(p) = &mut self.project {
                    // what the build measured (loudness, duration, preview, ids)
                    p.volume = built.volume;
                    p.duration = built.duration;
                    p.preview_start = built.preview_start;
                    p.identity = built.identity;
                }
                self.save(ctx);
                self.log.push_str("Built:\n");
                for path in &paths {
                    self.log.push_str(&format!("{}\n", path.display()));
                }
                self.log.push('\n');
                self.set_status(ctx, "Package built");
            }
            Some(Msg::Done(Output::Compare(rows))) => {
                self.job = None;
                let best = (0..rows.len()).max_by(|&a, &b| (rows[a].agreement * rows[a].coverage).total_cmp(&(rows[b].agreement * rows[b].coverage)));
                if let Some(Dialog::Compare(c)) = &mut self.dialog {
                    c.rows = rows;
                    c.selected = best;
                }
            }
            Some(Msg::Failed(e)) => {
                self.job = None;
                self.log.push_str(&format!("{label} failed: {e}\n"));
                self.message(&format!("{label} failed"), e);
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------------- ui

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if pressed(sc(Modifiers::COMMAND, Key::N)) {
            self.guarded(ctx, After::New(None));
        }
        if pressed(sc(Modifiers::COMMAND, Key::O)) {
            self.guarded(ctx, After::Open(None));
        }
        if pressed(sc(Modifiers::COMMAND, Key::S)) {
            self.save(ctx);
        }
        if pressed(sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) || pressed(sc(Modifiers::COMMAND, Key::Y)) {
            self.redo();
        }
        if pressed(sc(Modifiers::COMMAND, Key::Z)) {
            self.undo();
        }
        if pressed(sc(Modifiers::COMMAND, Key::Plus)) || pressed(sc(Modifiers::COMMAND, Key::Equals)) {
            self.zoom(1.25);
        }
        if pressed(sc(Modifiers::COMMAND, Key::Minus)) {
            self.zoom(0.8);
        }
        if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)) {
            self.toggle_play();
        }
    }

    fn zoom(&mut self, f: f32) {
        if let Some(p) = &self.project {
            self.editor.zoom(f, None, p);
        }
    }

    fn toggle_play(&mut self) {
        if self.project.is_none() {
            return;
        }
        if self.player.is_playing() || self.player.preparing() {
            self.player.pause();
        } else {
            self.player.play(self.editor.playhead);
        }
    }

    fn menus(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New Project…").clicked() {
                    self.guarded(&ctx, After::New(None));
                }
                if ui.button("Open Project…").clicked() {
                    self.guarded(&ctx, After::Open(None));
                }
                if ui.button("Open Official Chart (.psarc / .xml)…").clicked() {
                    self.guarded(&ctx, After::ImportChart);
                }
                ui.menu_button("Open Recent", |ui| {
                    let recent: Vec<String> = self.recent.iter().filter(|f| Path::new(f).is_dir()).cloned().collect();
                    if recent.is_empty() {
                        ui.label("No recent projects");
                    }
                    for f in recent {
                        if ui.button(&f).clicked() {
                            self.guarded(&ctx, After::Open(Some(PathBuf::from(f))));
                        }
                    }
                });
                if ui.add_enabled(self.project.is_some(), egui::Button::new("Save")).clicked() {
                    self.save(&ctx);
                }
                ui.separator();
                if ui.add_enabled(self.project.is_some(), egui::Button::new("Export arrangement XML…")).clicked() {
                    self.export_xml();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    self.guarded(&ctx, After::Quit);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(self.history.can_undo(), egui::Button::new("Undo")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(self.history.can_redo(), egui::Button::new("Redo")).clicked() {
                    self.redo();
                }
                ui.separator();
                if ui.button("Select All").clicked() {
                    self.act(Action::SelectAll);
                }
                if ui.button("Re-fret selected notes").clicked() {
                    self.act(Action::Refret { only_selected: true });
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Zoom In").clicked() {
                    self.zoom(1.25);
                }
                if ui.button("Zoom Out").clicked() {
                    self.zoom(0.8);
                }
            });
            ui.menu_button("Tools", |ui| {
                if ui.add_enabled(self.project.is_some(), egui::Button::new("Compare detection models…")).clicked() {
                    self.open_compare();
                }
                if ui.add_enabled(self.job.is_none(), egui::Button::new("Download models…")).clicked() {
                    let all: Vec<_> = fw_models::download::ASSETS.iter().filter(|a| fw_models::find_model(a.file).is_err()).collect();
                    if all.is_empty() {
                        self.message("Models", "All models are installed.");
                    } else {
                        self.dialog = Some(Dialog::Models(all));
                    }
                }
                if ui.button("System check…").clicked() {
                    let text = system_check(&self.player);
                    self.message("System check", text);
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Editor shortcuts").clicked() {
                    self.message("Editor shortcuts", SHORTCUTS);
                }
                if ui.button("About").clicked() {
                    self.message(
                        "About FretWright",
                        format!(
                            "FretWright {}\n\nAudio to Rocksmith 2014 bass arrangements, entirely in Rust.\n\n\
                             Uses Demucs (Meta) via demucs-rs, Beat This! (JKU Linz), a port of\n\
                             Rocksmith2014.NET and a Wwise-free WEM encoder (rabbit-smith/wem).",
                            env!("CARGO_PKG_VERSION")
                        ),
                    );
                }
            });
        });
    }

    fn open_compare(&mut self) {
        self.dialog = Some(Dialog::Compare(Compare {
            separators: SEPARATORS.iter().map(|s| s.0 == "htdemucs_ft" || s.0 == "htdemucs_6s").collect(),
            rows: Vec::new(),
            selected: None,
        }));
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let playing = self.player.is_playing();
            let label = if self.player.preparing() {
                "⏳ Preparing"
            } else if playing {
                "⏸ Pause"
            } else {
                "▶ Play"
            };
            if ui.add_enabled(self.project.is_some(), egui::Button::new(label)).on_hover_text("Space").clicked() {
                self.toggle_play();
            }
            if ui.button("⏹ Stop").clicked() {
                self.player.stop();
            }
            ui.separator();
            ui.label("Listen:");
            let mut mode = self.player.mode();
            egui::ComboBox::from_id_salt("mode").selected_text(mode.label()).show_ui(ui, |ui| {
                for m in Mode::ALL {
                    ui.selectable_value(&mut mode, m, m.label());
                }
            });
            self.player.set_mode(mode);
            ui.label("Speed:");
            let mut speed = self.player.speed();
            egui::ComboBox::from_id_salt("speed").selected_text(format!("{speed}x")).width(60.0).show_ui(ui, |ui| {
                for s in [0.5, 0.75, 1.0] {
                    ui.selectable_value(&mut speed, s, format!("{s}x"));
                }
            });
            self.player.set_speed(speed);
            if ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.5).show_value(false).text("Vol")).changed() {
                self.player.set_volume(self.volume);
            }
            ui.checkbox(&mut self.editor.follow, "Follow playhead");
            ui.separator();
            ui.label("Grid:");
            let mut div = self.editor.division();
            let cur = GRIDS.iter().find(|g| g.1 == div).map_or("?", |g| g.0);
            egui::ComboBox::from_id_salt("grid").selected_text(cur).width(60.0).show_ui(ui, |ui| {
                for (l, d) in GRIDS {
                    ui.selectable_value(&mut div, *d, *l);
                }
            });
            if div != self.editor.division() {
                self.editor.set_division(div, self.project.as_ref());
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let text = match &self.project {
                None => String::new(),
                Some(p) => {
                    let sel = self.editor.selected_notes(p);
                    match sel.as_slice() {
                        [] => format!(
                            "{} notes, {:.1} BPM  |  double-click a lane to add a note, type digits to set frets",
                            p.notes.len(),
                            p.grid.tempo
                        ),
                        [s] => format!(
                            "{} notes  |  selected: {} on {} string fret {}, {:.3}s, {:.0} ms, confidence {:.0}%",
                            p.notes.len(),
                            note_name(s.midi, true),
                            ["E", "A", "D", "G"][s.string.rem_euclid(4) as usize],
                            s.fret,
                            s.start,
                            s.duration() * 1000.0,
                            s.confidence * 100.0
                        ),
                        many => format!("{} notes  |  {} selected", p.notes.len(), many.len()),
                    }
                }
            };
            ui.label(text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(job) = &self.job {
                    ui.add(egui::ProgressBar::new(job.frac as f32).desired_width(220.0).show_percentage());
                    ui.label(format!("{}: {}", job.label, job.msg));
                } else if let Some((s, until)) = &self.status
                    && ui.input(|i| i.time) < *until
                {
                    ui.label(RichText::new(s).color(Color32::from_rgb(0x8f, 0xd1, 0xa0)));
                }
            });
        });
    }

    fn song_tab(&mut self, ui: &mut Ui) {
        let Some(p) = &mut self.project else { return };
        let mut changed = false;
        let mut tuning_to = None;
        egui::Grid::new("song").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            let md = &mut p.metadata;
            for (label, value) in [("Artist", &mut md.artist), ("Title", &mut md.title), ("Album", &mut md.album), ("Charter", &mut md.author), ("Version", &mut md.version)] {
                ui.label(label);
                changed |= ui.text_edit_singleline(value).changed();
                ui.end_row();
            }
            ui.label("Year");
            changed |= ui.add(egui::DragValue::new(&mut md.year).range(1900..=2100)).changed();
            ui.end_row();
            ui.label("Tuning");
            let cur = TUNINGS.iter().position(|t| t.1 == p.tuning);
            egui::ComboBox::from_id_salt("tuning").selected_text(cur.map_or("Custom", |i| TUNINGS[i].0)).show_ui(ui, |ui| {
                for (i, (name, _)) in TUNINGS.iter().enumerate() {
                    if ui.selectable_label(cur == Some(i), *name).clicked() {
                        tuning_to = Some(i);
                    }
                }
            });
            ui.end_row();
            ui.label("");
            changed |= ui.checkbox(&mut p.bass_picked, "Played with a pick").changed();
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.label("Album art");
        let art_path = p.album_art.clone();
        if self.art.as_ref().is_none_or(|a| a.0 != art_path) {
            let tex = load_texture(ui.ctx(), &art_path);
            self.art = Some((art_path.clone(), tex));
        }
        match self.art.as_ref().and_then(|a| a.1.as_ref()) {
            Some(t) => {
                ui.add(egui::Image::new(t).max_size(egui::vec2(160.0, 160.0)));
            }
            None => {
                ui.label(RichText::new("No art (a plain placeholder will be used)").weak());
            }
        }
        if ui.button("Choose album art…").clicked()
            && let Some(f) = rfd::FileDialog::new().add_filter("Images", IMAGE_EXT).pick_file()
            && let Some(p) = &mut self.project
        {
            p.album_art = f.display().to_string();
            changed = true;
        }
        if changed {
            self.dirty = true;
        }
        if let Some(i) = tuning_to {
            self.set_tuning(i);
        }
    }

    fn detection_tab(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        let ctx = ui.ctx().clone();
        let Some(p) = &mut self.project else { return };
        let d = &mut p.detection;
        let before = d.clone();
        egui::Grid::new("detect").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            let combo = |ui: &mut Ui, id: &str, items: &[(&str, &str, &str)], value: &mut String| {
                egui::ComboBox::from_id_salt(id).width(200.0).selected_text(label_of(items, value)).show_ui(ui, |ui| {
                    for (k, l, desc) in items {
                        ui.selectable_value(value, k.to_string(), *l).on_hover_text(*desc);
                    }
                });
            };
            ui.label("Bass separation");
            combo(ui, "sep", SEPARATORS, &mut d.separator);
            ui.end_row();
            ui.label("Note starts");
            combo(ui, "onsets", ONSETS, &mut d.onsets);
            ui.end_row();
            ui.label("Pitch tracking");
            combo(ui, "pitch", PITCH, &mut d.pitch);
            ui.end_row();
            ui.label("Beat tracking");
            combo(ui, "beats", BEATS, &mut d.beats);
            ui.end_row();
            ui.label("Device");
            let dev = DEVICES.iter().find(|x| x.0 == d.device).map_or(d.device.clone(), |x| x.1.to_string());
            egui::ComboBox::from_id_salt("device").width(200.0).selected_text(dev).show_ui(ui, |ui| {
                for (k, l) in DEVICES {
                    ui.selectable_value(&mut d.device, k.to_string(), *l);
                }
            });
            ui.end_row();
            ui.label("Shortest note");
            ui.add(egui::DragValue::new(&mut d.min_note_ms).range(20.0..=400.0).suffix(" ms"));
            ui.end_row();
            ui.label("Snap notes to");
            let cur = SNAPS.iter().find(|s| s.1 == d.snap_division).map_or("?", |s| s.0);
            egui::ComboBox::from_id_salt("snap").width(200.0).selected_text(cur).show_ui(ui, |ui| {
                for (l, v) in SNAPS {
                    ui.selectable_value(&mut d.snap_division, *v, *l);
                }
            });
            ui.end_row();
        });
        ui.add_space(6.0);
        for (items, id) in [(SEPARATORS, &d.separator), (ONSETS, &d.onsets), (PITCH, &d.pitch), (BEATS, &d.beats)] {
            if let Some((_, l, desc)) = items.iter().find(|i| i.0 == id) {
                ui.label(RichText::new(format!("{l}: {desc}")).weak());
            }
        }
        if *d != before {
            self.dirty = true;
        }
        let has_notes = !p.notes.is_empty();
        ui.add_space(8.0);
        ui.add_enabled_ui(!busy, |ui| {
            if ui.add_sized([ui.available_width(), 30.0], egui::Button::new("Detect bass line")).clicked() {
                if has_notes {
                    self.dialog = Some(Dialog::ConfirmReplace);
                } else {
                    self.run_detection(&ctx);
                }
            }
            if ui.button("Compare models…").clicked() {
                self.open_compare();
            }
            if ui.button("Re-fret all notes").on_hover_text("Re-run the string/fret optimiser over the whole chart").clicked() {
                self.act(Action::Refret { only_selected: false });
            }
            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label("Beat grid");
                ui.horizontal(|ui| {
                    if ui.button("−10 ms").on_hover_text("Shift the whole beat grid earlier").clicked() {
                        self.shift_grid(-0.01);
                    }
                    if ui.button("+10 ms").on_hover_text("Shift the whole beat grid later").clicked() {
                        self.shift_grid(0.01);
                    }
                });
            });
        });
    }

    fn build_tab(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        let ctx = ui.ctx().clone();
        let playhead = self.editor.playhead;
        let Some(p) = &mut self.project else { return };
        egui::Grid::new("build").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Platforms");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.build_pc, "PC");
                ui.checkbox(&mut self.build_mac, "Mac");
            });
            ui.end_row();
            ui.label("Preview start");
            ui.horizontal(|ui| {
                let mut v = p.preview_start.unwrap_or(0.0);
                if ui
                    .add(egui::DragValue::new(&mut v).range(0.0..=3600.0).speed(0.1).suffix(" s").custom_formatter(|v, _| if v == 0.0 { "Automatic".into() } else { format!("{v:.2} s") }))
                    .on_hover_text("Where the 28 s song-select preview starts (0 = automatic)")
                    .changed()
                {
                    p.preview_start = (v > 0.0).then_some(v);
                    self.dirty = true;
                }
                if ui.button("Use playhead").clicked() {
                    p.preview_start = Some(playhead);
                    self.dirty = true;
                }
            });
            ui.end_row();
            ui.label("Output folder");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.out_dir).hint_text("<project folder>/out").desired_width(180.0));
                if ui.button("…").clicked()
                    && let Some(d) = rfd::FileDialog::new().pick_folder()
                {
                    self.out_dir = d.display().to_string();
                }
            });
            ui.end_row();
        });
        let out = if self.out_dir.trim().is_empty() { p.path("out") } else { PathBuf::from(self.out_dir.trim()) };
        ui.add_space(8.0);
        if ui.add_enabled(!busy, egui::Button::new("Build .psarc").min_size(egui::vec2(ui.available_width(), 30.0))).clicked() {
            self.run_build(&ctx);
        }
        if ui.button("Open output folder").clicked() {
            Self::open_folder(&out);
        }
        ui.add_space(6.0);
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut self.log.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let mut then: Option<Followup> = None;
        let modal = |id: &str| egui::Modal::new(egui::Id::new(id));
        match dialog {
            Dialog::Message(title, text) => {
                let r = modal("msg").show(ctx, |ui| {
                    ui.heading(&title);
                    ui.add_space(6.0);
                    ui.label(RichText::new(&text).monospace());
                    ui.add_space(6.0);
                    ui.button("OK").clicked()
                });
                keep = !(r.inner || r.should_close());
                if keep {
                    self.dialog = Some(Dialog::Message(title, text));
                }
                return;
            }
            Dialog::Models(assets) => {
                let mut start = false;
                modal("models").show(ctx, |ui| {
                    ui.set_width(520.0);
                    ui.heading("Download AI models");
                    ui.label("FretWright uses neural networks to separate the bass and find the beats. These files are needed:");
                    ui.add_space(4.0);
                    for a in &assets {
                        ui.label(format!("• {}  ({:.0} MB)", a.what, a.size as f64 / 1e6));
                    }
                    let total: u64 = assets.iter().map(|a| a.size).sum();
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{:.0} MB in total, verified by checksum, saved to {}.\nWithout them, detection falls back to the low-pass filter and the classic beat tracker.",
                            total as f64 / 1e6,
                            fw_models::download::install_dir().map(|d| d.display().to_string()).unwrap_or_default()
                        ))
                        .weak(),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Download").clicked() {
                            start = true;
                            keep = false;
                        }
                        if ui.button("Not now").clicked() {
                            keep = false;
                        }
                    });
                });
                if start {
                    self.download_models(ctx, assets);
                } else if keep {
                    self.dialog = Some(Dialog::Models(assets));
                }
                return;
            }
            Dialog::ConfirmReplace => {
                modal("replace").show(ctx, |ui| {
                    ui.heading("Replace chart?");
                    ui.label("Detection replaces the current notes (you can undo).");
                    ui.horizontal(|ui| {
                        if ui.button("Detect").clicked() {
                            keep = false;
                            then = Some(Box::new(|a: &mut App| {
                                let ctx = a.ctx_clone();
                                a.run_detection(&ctx);
                            }));
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if keep {
                    self.dialog = Some(Dialog::ConfirmReplace);
                }
            }
            Dialog::ConfirmDiscard(after) => {
                let mut choice = None;
                modal("discard").show(ctx, |ui| {
                    ui.heading("Unsaved changes");
                    ui.label("Save changes to the current project?");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            choice = Some(1);
                        }
                        if ui.button("Discard").clicked() {
                            choice = Some(2);
                        }
                        if ui.button("Cancel").clicked() {
                            choice = Some(0);
                        }
                    });
                });
                match choice {
                    None => self.dialog = Some(Dialog::ConfirmDiscard(after)),
                    Some(0) => {}
                    Some(c) => {
                        if c == 2 || self.save(ctx) {
                            self.dirty = false;
                            self.run_after(ctx, after);
                        }
                    }
                }
                return;
            }
            Dialog::NewProject(mut np) => {
                modal("new").show(ctx, |ui| {
                    ui.set_width(560.0);
                    ui.heading("New project");
                    egui::Grid::new("np").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("Song audio");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut np.audio).desired_width(380.0));
                            if ui.button("Browse…").clicked()
                                && let Some(f) = rfd::FileDialog::new().add_filter("Audio", AUDIO_EXT).pick_file()
                            {
                                np.audio_chosen(&f);
                            }
                        });
                        ui.end_row();
                        ui.label("Project folder");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut np.folder).desired_width(380.0));
                            if ui.button("Browse…").clicked()
                                && let Some(d) = rfd::FileDialog::new().pick_folder()
                            {
                                np.folder = d.display().to_string();
                            }
                        });
                        ui.end_row();
                        for (l, v) in [("Artist", &mut np.artist), ("Title", &mut np.title), ("Album", &mut np.album)] {
                            ui.label(l);
                            ui.text_edit_singleline(v);
                            ui.end_row();
                        }
                        ui.label("Year");
                        ui.add(egui::DragValue::new(&mut np.year).range(1900..=2100));
                        ui.end_row();
                        ui.label("Album art");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut np.art).desired_width(380.0));
                            if ui.button("Browse…").clicked()
                                && let Some(f) = rfd::FileDialog::new().add_filter("Images", IMAGE_EXT).pick_file()
                            {
                                np.art = f.display().to_string();
                            }
                        });
                        ui.end_row();
                        ui.label("Bass tuning");
                        egui::ComboBox::from_id_salt("np_tuning").selected_text(TUNINGS[np.tuning].0).show_ui(ui, |ui| {
                            for (i, (n, _)) in TUNINGS.iter().enumerate() {
                                ui.selectable_value(&mut np.tuning, i, *n);
                            }
                        });
                        ui.end_row();
                    });
                    if !np.error.is_empty() {
                        ui.colored_label(Color32::from_rgb(0xe0, 0x60, 0x60), &np.error);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            match create_project(&np) {
                                Ok(Created::New(p)) => {
                                    keep = false;
                                    then = Some(Box::new(move |a: &mut App| {
                                        a.remember(&p.folder.clone());
                                        a.set_project(Some(*p));
                                        a.tab = Tab::Detection;
                                        let ctx = a.ctx_clone();
                                        a.set_status(&ctx, "Project created. Choose models and press 'Detect bass line'.");
                                    }));
                                }
                                Ok(Created::Existing(folder)) => {
                                    keep = false;
                                    then = Some(Box::new(move |a: &mut App| a.open(&folder)));
                                }
                                Err(e) => np.error = e,
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if keep {
                    self.dialog = Some(Dialog::NewProject(np));
                }
            }
            Dialog::Compare(mut c) => {
                let busy = self.job.is_some();
                let mut run = None;
                let mut load = None;
                modal("compare").show(ctx, |ui| {
                    ui.set_width(720.0);
                    ui.heading("Compare detection models");
                    ui.label(
                        "Pick the separators to try (stems are cached, so each runs once).\n\
                         Agreement = share of notes whose pitch dominates the bass stem at that moment.\n\
                         Coverage = share of the time the bass is audible that has a note.\n\
                         Both are sanity scores, not ground truth: listen to the result too.",
                    );
                    ui.horizontal_wrapped(|ui| {
                        for (on, (_, l, desc)) in c.separators.iter_mut().zip(SEPARATORS) {
                            ui.checkbox(on, *l).on_hover_text(*desc);
                        }
                    });
                    egui::Grid::new("cmp").striped(true).num_columns(6).spacing([16.0, 4.0]).show(ui, |ui| {
                        for h in ["Separator", "Pitch", "Notes", "Agreement", "Coverage", "Time"] {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for (i, r) in c.rows.iter().enumerate() {
                            let sel = c.selected == Some(i);
                            let resp = ui.selectable_label(sel, label_of(SEPARATORS, &r.separator));
                            if resp.clicked() {
                                c.selected = Some(i);
                            }
                            if resp.double_clicked() {
                                load = Some(i);
                            }
                            ui.label(label_of(PITCH, &r.pitch));
                            ui.label(r.detection.notes.len().to_string());
                            ui.label(format!("{:.0}%", r.agreement * 100.0));
                            ui.label(format!("{:.0}%", r.coverage * 100.0));
                            ui.label(format!("{:.1} s", r.seconds));
                            ui.end_row();
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new("Run comparison")).clicked() {
                            run = Some(SEPARATORS.iter().zip(&c.separators).filter(|(_, on)| **on).map(|(s, _)| s.0.to_string()).collect::<Vec<_>>());
                        }
                        if ui.add_enabled(c.selected.is_some(), egui::Button::new("Load selected into editor")).clicked() {
                            load = c.selected;
                        }
                        if ui.button("Close").clicked() {
                            keep = false;
                        }
                    });
                    if busy {
                        ui.label(RichText::new("Running… (progress in the status bar)").weak());
                    }
                });
                if let Some(i) = load
                    && let Some(r) = c.rows.get(i)
                {
                    let (det, s) = (r.detection.clone(), r.settings.clone());
                    then = Some(Box::new(move |a: &mut App| a.apply_detection(det, s)));
                }
                if keep {
                    self.dialog = Some(Dialog::Compare(c));
                }
                if let Some(seps) = run.filter(|s| !s.is_empty()) {
                    self.run_compare(ctx, seps);
                }
            }
        }
        if let Some(f) = then {
            f(self);
        }
    }

    fn ctx_clone(&self) -> egui::Context {
        self.ctx.clone()
    }
}

/// Work a dialog hands back to run once it has closed.
type Followup = Box<dyn FnOnce(&mut App)>;

enum Created {
    New(Box<Project>),
    Existing(PathBuf),
}

fn create_project(np: &NewProject) -> Result<Created, String> {
    let audio = PathBuf::from(np.audio.trim());
    if !audio.is_file() {
        return Err("Choose the song's audio file.".into());
    }
    if np.folder.trim().is_empty() {
        return Err("Choose where to keep the project.".into());
    }
    let folder = std::path::absolute(np.folder.trim()).map_err(|e| e.to_string())?;
    if folder.join(fw_project::PROJECT_FILE).is_file() {
        return Ok(Created::Existing(folder));
    }
    let audio = std::path::absolute(&audio).map_err(|e| e.to_string())?;
    let decoded = fw_audio::decode(&audio).map_err(|e| format!("Could not read the audio: {e}"))?;
    let mut p = Project::new(&folder, audio.display().to_string());
    p.metadata.artist = np.artist.trim().into();
    p.metadata.title = np.title.trim().into();
    p.metadata.album = np.album.trim().into();
    p.metadata.year = np.year;
    p.album_art = if np.art.trim().is_empty() { String::new() } else { std::path::absolute(np.art.trim()).map(|p| p.display().to_string()).unwrap_or_default() };
    p.tuning = TUNINGS[np.tuning].1;
    p.duration = decoded.duration();
    p.save().map_err(|e| e.to_string())?;
    Ok(Created::New(Box::new(p)))
}

fn load_texture(ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
    if path.is_empty() {
        return None;
    }
    let img = image::open(path).ok()?.thumbnail(320, 320).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(path, color, egui::TextureOptions::LINEAR))
}

fn system_check(player: &Player) -> String {
    let mut out = format!("FretWright {}\n\nAudio output: ", env!("CARGO_PKG_VERSION"));
    match &player.error {
        Some(e) => out.push_str(&format!("unavailable ({e})\n")),
        None => out.push_str(&format!("{} Hz\n", player.device_rate())),
    }
    out.push_str(&format!(
        "GPU for Demucs: {}\n\nModels:\n",
        fw_models::demucs::gpu_name().unwrap_or_else(|e| format!("none ({e}); Demucs will use the CPU"))
    ));
    let mut files = vec![fw_models::beat_this::DEFAULT_MODEL.to_string()];
    files.extend(fw_models::demucs::models().map(fw_models::demucs::model_file));
    for f in files {
        out.push_str(&format!("  {f:32} {}\n", fw_models::find_model(&f).map_or("missing".into(), |p| p.display().to_string())));
    }
    out
}

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(0x1b, 0x1d, 0x23);
    v.window_fill = Color32::from_rgb(0x1f, 0x22, 0x29);
    v.extreme_bg_color = Color32::from_rgb(0x23, 0x26, 0x2e);
    v.selection.bg_fill = Color32::from_rgb(0x3f, 0x8f, 0x63);
    ctx.set_visuals(v);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_job(&ctx);
        if let Some(peaks) = self.player.poll(self.project.as_ref()) {
            self.editor.set_peaks(peaks);
        }
        if self.player.is_playing() {
            let t = self.player.position();
            if let Some(p) = &self.project {
                self.editor.set_playhead(t, p);
            }
            ctx.request_repaint();
        } else if self.player.preparing() || self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::ConfirmDiscard(After::Quit));
        }
        let title = match &self.project {
            Some(p) => format!("FretWright - {} - {}{}", p.metadata.artist, p.metadata.title, if self.dirty { " *" } else { "" }),
            None => "FretWright".into(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        if self.dialog.is_none() {
            self.shortcuts(&ctx);
        }
        egui::Panel::top("menu").show(ui, |ui| self.menus(ui));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("side").default_size(380.0).min_size(320.0).show(ui, |ui| {
            ui.add_enabled_ui(self.project.is_some(), |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Song, "Song");
                    ui.selectable_value(&mut self.tab, Tab::Detection, "Detection");
                    ui.selectable_value(&mut self.tab, Tab::Build, "Build");
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                    Tab::Song => self.song_tab(ui),
                    Tab::Detection => self.detection_tab(ui),
                    Tab::Build => self.build_tab(ui),
                });
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let ev = self.editor.ui(ui, self.project.as_mut(), &mut self.history);
            if ev.changed {
                self.chart_changed();
            }
            if let Some(t) = ev.seek {
                if let Some(p) = &self.project {
                    self.editor.set_playhead(t, p);
                }
                self.player.seek(t);
            }
        });
        self.dialogs(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "recent", &self.recent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_artist_and_title() {
        assert_eq!(guess_metadata(Path::new("/x/Fluid Love - Dirty Water [Official Video].mp3")), ("Fluid Love".into(), "Dirty Water".into()));
        assert_eq!(guess_metadata(Path::new("song (live).flac")), (String::new(), "song".into()));
    }
}
