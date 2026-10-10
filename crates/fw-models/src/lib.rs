//! Neural networks for FretWright, run in pure Rust with [rten] (no Python,
//! no system libraries, no GPU drivers needed).
//!
//! * [`beat_this`]: Beat This! beat and downbeat tracking (Foscarin et al.,
//!   ISMIR 2024), matching the PyTorch reference frame for frame
//! * [`demucs`]: Demucs source separation on the GPU (wgpu) or CPU
//!
//! Model weights are not part of the source tree; [`find_model`] looks for
//! them in the usual places.

pub mod beat_this;
pub mod download;
#[cfg(feature = "demucs")]
pub mod demucs;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("model file {name} not found (looked in: {searched})")]
    NotFound { name: String, searched: String },
    #[error("could not load model {path}: {msg}")]
    Load { path: PathBuf, msg: String },
    #[error("inference failed: {0}")]
    Run(String),
    #[error("model download failed: {0}")]
    Download(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Directories searched for model files, in order:
/// `$FRETWRIGHT_MODELS`, `models/` next to the executable (and its parents, for
/// `target/release` builds), then the per-user data
/// directory (`~/.local/share/fretwright/models`, `%APPDATA%\fretwright\models`,
/// `~/Library/Application Support/fretwright/models`).
pub fn model_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os("FRETWRIGHT_MODELS") {
        dirs.push(PathBuf::from(d));
    }
    if let Ok(exe) = std::env::current_exe() {
        for anc in exe.ancestors().skip(1).take(4) {
            dirs.push(anc.join("models"));
        }
    }
    if let Some(d) = user_data_dir() {
        dirs.push(d.join("models"));
    }
    dirs
}

/// The per-user data directory of FretWright.
pub fn user_data_dir() -> Option<PathBuf> {
    let name = "fretwright";
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        env("APPDATA").map(|d| d.join(name))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support").join(name))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share"))).map(|d| d.join(name))
    }
}

/// Finds a model file by its relative name, e.g. `beat_this/final0.onnx`.
pub fn find_model(name: &str) -> Result<PathBuf> {
    let dirs = model_dirs();
    dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()).ok_or_else(|| Error::NotFound {
        name: name.to_string(),
        searched: dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", "),
    })
}
