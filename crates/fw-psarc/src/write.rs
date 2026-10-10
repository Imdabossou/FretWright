use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::Path;

use byteorder::{BigEndian, WriteBytesExt};
use flate2::{Compression, write::ZlibEncoder};
use rayon::prelude::*;

use crate::{
    BLOCK_SIZE, Error, FLAG_ENCRYPTED_TOC, HEADER_LEN, Result, TOC_ENTRY_LEN, crypto, stored_plain,
};

/// One block as stored in the file.
struct Block<'a> {
    data: std::borrow::Cow<'a, [u8]>,
}

/// Splits an entry into blocks; compressed unless the name is stored plain or
/// compression does not make a block smaller than the block size.
fn blocks_for<'a>(name: &str, data: &'a [u8]) -> Vec<Block<'a>> {
    if data.is_empty() {
        return Vec::new();
    }
    let chunks: Vec<&[u8]> = data.chunks(BLOCK_SIZE).collect();
    if stored_plain(name) {
        return chunks.into_iter().map(|c| Block { data: c.into() }).collect();
    }
    chunks
        .into_par_iter()
        .map(|chunk| {
            let mut enc = ZlibEncoder::new(Vec::with_capacity(chunk.len()), Compression::best());
            let packed = enc.write_all(chunk).and_then(|_| enc.finish()).expect("in-memory zlib");
            if packed.len() < BLOCK_SIZE {
                Block { data: packed.into() }
            } else {
                Block { data: chunk.into() }
            }
        })
        .collect()
}

/// Writes a PSARC containing `entries` (name, data) in the given order.
///
/// `encrypt_toc` should be true for archives Rocksmith loads (all official and
/// custom DLC encrypt the table of contents).
pub fn write<W: Write + Seek>(out: W, entries: &[(String, Vec<u8>)], encrypt_toc: bool) -> Result<()> {
    let manifest = entries.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join("\n");
    let mut all: Vec<(&str, &[u8])> = vec![("", manifest.as_bytes())];
    all.extend(entries.iter().map(|(n, d)| (n.as_str(), d.as_slice())));

    for (name, data) in &all {
        if data.len() as u64 >= 1 << 40 {
            return Err(Error::TooLarge((*name).into()));
        }
    }

    // compress every entry; entries are processed in parallel as well as blocks
    let packed: Vec<Vec<Block>> = all.par_iter().map(|(n, d)| blocks_for(n, d)).collect();
    let block_count: usize = packed.iter().map(Vec::len).sum();

    let toc_len = HEADER_LEN + all.len() * TOC_ENTRY_LEN + block_count * 2;
    let mut w = BufWriter::with_capacity(BLOCK_SIZE, out);

    // header
    w.write_all(b"PSAR")?;
    w.write_u16::<BigEndian>(1)?;
    w.write_u16::<BigEndian>(4)?;
    w.write_all(b"zlib")?;
    w.write_u32::<BigEndian>(toc_len as u32)?;
    w.write_u32::<BigEndian>(TOC_ENTRY_LEN as u32)?;
    w.write_u32::<BigEndian>(all.len() as u32)?;
    w.write_u32::<BigEndian>(BLOCK_SIZE as u32)?;
    w.write_u32::<BigEndian>(if encrypt_toc { FLAG_ENCRYPTED_TOC } else { 0 })?;

    // table of contents + block size table
    let mut toc = Vec::with_capacity(toc_len - HEADER_LEN);
    let mut offset = toc_len as u64;
    let mut first_block = 0u32;
    for ((name, data), blocks) in all.iter().zip(&packed) {
        toc.extend_from_slice(&crypto::name_digest(name));
        toc.write_u32::<BigEndian>(first_block)?;
        toc.write_uint::<BigEndian>(data.len() as u64, 5)?;
        toc.write_uint::<BigEndian>(offset, 5)?;
        first_block += blocks.len() as u32;
        offset += blocks.iter().map(|b| b.data.len() as u64).sum::<u64>();
    }
    for b in packed.iter().flatten() {
        // a full-size block is recorded as 0
        let len = if b.data.len() == BLOCK_SIZE { 0 } else { b.data.len() as u16 };
        toc.write_u16::<BigEndian>(len)?;
    }
    if encrypt_toc {
        crypto::encrypt_toc(&mut toc);
    }
    w.write_all(&toc)?;

    for b in packed.iter().flatten() {
        w.write_all(&b.data)?;
    }
    w.flush()?;
    Ok(())
}

/// Writes a PSARC file to `path` (see [`write`]).
pub fn write_file(path: impl AsRef<Path>, entries: &[(String, Vec<u8>)], encrypt_toc: bool) -> Result<()> {
    let path = path.as_ref();
    // write to a temporary name first so a failed build never leaves a broken package
    let tmp = path.with_extension("psarc.part");
    let result = File::create(&tmp)
        .map_err(Error::from)
        .and_then(|f| write(f, entries, encrypt_toc))
        .and_then(|_| std::fs::rename(&tmp, path).map_err(Error::from));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
