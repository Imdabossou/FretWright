//! Rocksmith 2014 SNG files: the binary chart format the game reads.
//!
//! An SNG on disk is *packed*: a small header, an AES-CTR encrypted payload
//! holding the zlib-compressed record data, and 56 unused trailing bytes.
//! [`Sng::from_packed`] / [`Sng::to_packed`] handle that; [`Sng::read`] /
//! [`Sng::write`] work on the plain record data.
//!
//! Format knowledge comes from Rocksmith2014.NET (MIT, Tapio Malmberg).

mod bin;
mod crypto;
pub mod types;

use std::io::{Read, Write};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};

use bin::{Bin, FixedStr, Reader};
pub use bin::{Fixed, FixedStr as Str, Pad};
pub use types::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SNG data ends early")]
    Truncated,
    #[error("corrupt SNG: {0}")]
    Corrupt(&'static str),
    #[error("not a packed SNG file")]
    BadMagic,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Target platform; decides the encryption key (and file paths in a package).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Pc,
    Mac,
}

/// Largest plain SNG we accept when unpacking (protects against zip bombs).
const MAX_PLAIN_LEN: usize = 256 << 20;
const MAGIC: u32 = 0x4A;
const FLAGS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetaData {
    pub max_score: f64,
    pub max_notes_and_chords: f64,
    pub max_notes_and_chords_real: f64,
    pub points_per_note: f64,
    pub first_beat_length: f32,
    pub start_time: f32,
    pub capo_fret_id: i8,
    pub last_conversion_date_time: String,
    pub part: i16,
    pub song_length: f32,
    pub tuning: Vec<i16>,
    /// Stored twice in the file; both copies are always equal.
    pub first_note_time: f32,
    pub max_difficulty: i32,
}

impl Bin for MetaData {
    fn read(r: &mut Reader) -> Result<Self> {
        let mut m = MetaData {
            max_score: Bin::read(r)?,
            max_notes_and_chords: Bin::read(r)?,
            max_notes_and_chords_real: Bin::read(r)?,
            points_per_note: Bin::read(r)?,
            first_beat_length: Bin::read(r)?,
            start_time: Bin::read(r)?,
            capo_fret_id: Bin::read(r)?,
            last_conversion_date_time: FixedStr::<32>::read(r)?.0,
            part: Bin::read(r)?,
            song_length: Bin::read(r)?,
            tuning: Bin::read(r)?,
            ..Default::default()
        };
        let _first_copy = f32::read(r)?;
        m.first_note_time = Bin::read(r)?;
        m.max_difficulty = Bin::read(r)?;
        Ok(m)
    }

    fn write(&self, w: &mut Vec<u8>) {
        self.max_score.write(w);
        self.max_notes_and_chords.write(w);
        self.max_notes_and_chords_real.write(w);
        self.points_per_note.write(w);
        self.first_beat_length.write(w);
        self.start_time.write(w);
        self.capo_fret_id.write(w);
        FixedStr::<32>(self.last_conversion_date_time.clone()).write(w);
        self.part.write(w);
        self.song_length.write(w);
        self.tuning.write(w);
        self.first_note_time.write(w);
        self.first_note_time.write(w);
        self.max_difficulty.write(w);
    }
}

/// A complete SNG chart.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sng {
    pub beats: Vec<Beat>,
    pub phrases: Vec<Phrase>,
    pub chords: Vec<Chord>,
    pub chord_notes: Vec<ChordNotes>,
    pub vocals: Vec<Vocal>,
    /// Only stored when there are vocals.
    pub symbols_headers: Vec<SymbolsHeader>,
    /// Only stored when there are vocals.
    pub symbols_textures: Vec<SymbolsTexture>,
    /// Only stored when there are vocals.
    pub symbol_definitions: Vec<SymbolDefinition>,
    pub phrase_iterations: Vec<PhraseIteration>,
    pub phrase_extra_info: Vec<PhraseExtraInfo>,
    pub new_linked_difficulties: Vec<NewLinkedDifficulty>,
    pub actions: Vec<Action>,
    pub events: Vec<Event>,
    pub tones: Vec<Tone>,
    pub dnas: Vec<Dna>,
    pub sections: Vec<Section>,
    pub levels: Vec<Level>,
    pub metadata: MetaData,
}

impl Sng {
    /// Parses plain (unpacked) SNG record data.
    pub fn read(data: &[u8]) -> Result<Self> {
        let r = &mut Reader::new(data);
        let beats = Bin::read(r)?;
        let phrases = Bin::read(r)?;
        let chords = Bin::read(r)?;
        let chord_notes = Bin::read(r)?;
        let vocals: Vec<Vocal> = Bin::read(r)?;
        let has_vocals = !vocals.is_empty();
        let symbols_headers = read_if(has_vocals, r)?;
        let symbols_textures = read_if(has_vocals, r)?;
        let symbol_definitions = read_if(has_vocals, r)?;
        Ok(Sng {
            beats,
            phrases,
            chords,
            chord_notes,
            vocals,
            symbols_headers,
            symbols_textures,
            symbol_definitions,
            phrase_iterations: Bin::read(r)?,
            phrase_extra_info: Bin::read(r)?,
            new_linked_difficulties: Bin::read(r)?,
            actions: Bin::read(r)?,
            events: Bin::read(r)?,
            tones: Bin::read(r)?,
            dnas: Bin::read(r)?,
            sections: Bin::read(r)?,
            levels: Bin::read(r)?,
            metadata: Bin::read(r)?,
        })
    }

    /// Serialises to plain (unpacked) SNG record data.
    pub fn write(&self) -> Vec<u8> {
        let mut w = Vec::with_capacity(1 << 16);
        self.beats.write(&mut w);
        self.phrases.write(&mut w);
        self.chords.write(&mut w);
        self.chord_notes.write(&mut w);
        self.vocals.write(&mut w);
        if !self.vocals.is_empty() {
            self.symbols_headers.write(&mut w);
            self.symbols_textures.write(&mut w);
            self.symbol_definitions.write(&mut w);
        }
        self.phrase_iterations.write(&mut w);
        self.phrase_extra_info.write(&mut w);
        self.new_linked_difficulties.write(&mut w);
        self.actions.write(&mut w);
        self.events.write(&mut w);
        self.tones.write(&mut w);
        self.dnas.write(&mut w);
        self.sections.write(&mut w);
        self.levels.write(&mut w);
        self.metadata.write(&mut w);
        w
    }

    /// Parses a packed (encrypted + compressed) SNG as stored in a PSARC.
    pub fn from_packed(data: &[u8], platform: Platform) -> Result<Self> {
        Self::read(&unpack(data, platform)?)
    }

    /// Serialises to a packed SNG ready to go into a PSARC.
    pub fn to_packed(&self, platform: Platform) -> Vec<u8> {
        pack(&self.write(), platform)
    }
}

fn read_if<T: Bin>(present: bool, r: &mut Reader) -> Result<Vec<T>> {
    if present { Bin::read(r) } else { Ok(Vec::new()) }
}

/// Decrypts and decompresses a packed SNG into plain record data.
pub fn unpack(data: &[u8], platform: Platform) -> Result<Vec<u8>> {
    let r = &mut Reader::new(data);
    if u32::read(r)? != MAGIC {
        return Err(Error::BadMagic);
    }
    let _flags = u32::read(r)?;
    let iv: [u8; 16] = r.array()?;
    let mut payload = r.take(r.remaining())?.to_vec();
    crypto::apply(&mut payload, platform, &iv);

    let p = &mut Reader::new(&payload);
    let plain_len = u32::read(p)? as usize;
    if plain_len > MAX_PLAIN_LEN {
        return Err(Error::Corrupt("declared size too large"));
    }
    let mut out = Vec::with_capacity(plain_len);
    ZlibDecoder::new(p.take(p.remaining())?)
        .take(plain_len as u64)
        .read_to_end(&mut out)?;
    if out.len() != plain_len {
        return Err(Error::Corrupt("decompressed size does not match header"));
    }
    Ok(out)
}

/// Compresses and encrypts plain record data into a packed SNG.
pub fn pack(plain: &[u8], platform: Platform) -> Vec<u8> {
    let mut payload = (plain.len() as u32).to_le_bytes().to_vec();
    let mut z = ZlibEncoder::new(payload, Compression::best());
    z.write_all(plain).expect("in-memory zlib");
    payload = z.finish().expect("in-memory zlib");

    let iv = [0u8; 16];
    crypto::apply(&mut payload, platform, &iv);

    let mut out = Vec::with_capacity(24 + payload.len() + 56);
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&FLAGS.to_le_bytes());
    out.extend_from_slice(&iv);
    out.extend_from_slice(&payload);
    // 56 bytes the game does not use
    out.resize(out.len() + 56, 0);
    out
}
