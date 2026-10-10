//! Rocksmith 2014 PSARC archives.
//!
//! A PSARC is a 32-byte header, a table of contents (optionally AES-encrypted),
//! a table of compressed block sizes, then the entry data split into 64 KiB
//! blocks, each zlib-compressed unless that would not save space. The first
//! entry is a manifest listing the names of the others, one per line.
//!
//! Layout and behaviour follow Rocksmith2014.NET's `Rocksmith2014.PSARC`
//! (MIT, Tapio Malmberg). Unlike it, reading treats the archive as untrusted:
//! every size and index is validated, decompression cannot exceed the declared
//! entry length, and extraction refuses names that escape the target folder.
//!
//! ```no_run
//! # fn main() -> Result<(), fw_psarc::Error> {
//! let mut psarc = fw_psarc::Psarc::open("song_p.psarc")?;
//! for name in psarc.names().to_vec() {
//!     let data = psarc.read(&name)?;
//!     println!("{name}: {} bytes", data.len());
//! }
//!
//! fw_psarc::write_file("copy_p.psarc", &[("hello.txt".into(), b"hi".to_vec())], true)?;
//! # Ok(()) }
//! ```

mod crypto;
mod read;
mod write;

pub use read::Psarc;
pub use write::{write, write_file};

/// Errors from reading or writing archives.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a PSARC file")]
    BadMagic,
    #[error("unsupported PSARC compression method {0:?}")]
    UnsupportedCompression(String),
    #[error("corrupt PSARC: {0}")]
    Corrupt(&'static str),
    #[error("no entry named {0:?} in the archive")]
    NotFound(String),
    #[error("entry name {0:?} is not a safe relative path")]
    UnsafeName(String),
    #[error("entry {0:?} is too large for a PSARC (max 1 TiB)")]
    TooLarge(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Size of the archive header in bytes.
pub(crate) const HEADER_LEN: usize = 32;
/// Size of one table-of-contents entry in bytes.
pub(crate) const TOC_ENTRY_LEN: usize = 30;
/// Uncompressed size of one data block. Block sizes are stored as u16, where
/// 0 means a full (raw) block.
pub(crate) const BLOCK_SIZE: usize = 65536;
/// Archive flag marking an encrypted table of contents.
pub(crate) const FLAG_ENCRYPTED_TOC: u32 = 4;

/// One table-of-contents record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// MD5 of the entry name (all zeros for the manifest).
    pub name_digest: [u8; 16],
    /// Index of the first block in the block size table.
    pub first_block: u32,
    /// Uncompressed length in bytes.
    pub length: u64,
    /// Offset of the first block from the start of the file.
    pub offset: u64,
}

/// Files Rocksmith stores uncompressed: already-compressed audio and SNG data,
/// and the tiny appid file.
pub(crate) fn stored_plain(name: &str) -> bool {
    [".wem", ".sng", "appid", "7z"].iter().any(|ext| {
        name.len() >= ext.len() && name.get(name.len() - ext.len()..).is_some_and(|x| x.eq_ignore_ascii_case(ext))
    })
}
