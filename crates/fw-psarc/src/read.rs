use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use byteorder::{BigEndian, ReadBytesExt};
use flate2::read::ZlibDecoder;

use crate::{BLOCK_SIZE, Entry, Error, FLAG_ENCRYPTED_TOC, HEADER_LEN, Result, TOC_ENTRY_LEN, crypto};

/// Upper bound on a table of contents we are willing to load (64 MiB).
const MAX_TOC_LEN: usize = 64 << 20;
/// Upper bound on the manifest (list of names) size.
const MAX_MANIFEST_LEN: u64 = 16 << 20;

/// An opened PSARC archive.
pub struct Psarc<R> {
    reader: R,
    file_len: u64,
    names: Vec<String>,
    /// Entries in manifest order (the manifest itself is not included).
    entries: Vec<Entry>,
    /// Compressed block sizes; 0 means a full raw block.
    block_sizes: Vec<u32>,
    block_size: usize,
}

impl Psarc<BufReader<File>> {
    /// Opens a PSARC file from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_reader(BufReader::with_capacity(BLOCK_SIZE, File::open(path)?))
    }
}

impl<R: Read + Seek> Psarc<R> {
    /// Reads the header and table of contents from any seekable reader.
    pub fn from_reader(mut reader: R) -> Result<Self> {
        let file_len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != b"PSAR" {
            return Err(Error::BadMagic);
        }
        let _version_major = reader.read_u16::<BigEndian>()?;
        let _version_minor = reader.read_u16::<BigEndian>()?;
        let mut method = [0u8; 4];
        reader.read_exact(&mut method)?;
        if &method != b"zlib" {
            return Err(Error::UnsupportedCompression(String::from_utf8_lossy(&method).into()));
        }
        let toc_len = reader.read_u32::<BigEndian>()? as usize;
        let toc_entry_len = reader.read_u32::<BigEndian>()? as usize;
        let toc_count = reader.read_u32::<BigEndian>()? as usize;
        let block_size = reader.read_u32::<BigEndian>()? as usize;
        let flags = reader.read_u32::<BigEndian>()?;

        if !(HEADER_LEN..=MAX_TOC_LEN).contains(&toc_len) || toc_len as u64 > file_len {
            return Err(Error::Corrupt("table of contents length out of range"));
        }
        if toc_entry_len != TOC_ENTRY_LEN {
            return Err(Error::Corrupt("unexpected table of contents entry size"));
        }
        let size_bytes = match block_size {
            65536 => 2,
            16_777_216 => 3,
            0 => return Err(Error::Corrupt("zero block size")),
            _ if block_size as u64 == 1 << 32 => 4,
            _ => return Err(Error::Corrupt("unsupported block size")),
        };
        let toc_body = toc_len - HEADER_LEN;
        let entries_len = toc_count
            .checked_mul(TOC_ENTRY_LEN)
            .filter(|&n| n <= toc_body)
            .ok_or(Error::Corrupt("table of contents too short for its entries"))?;
        if toc_count == 0 {
            return Err(Error::Corrupt("archive has no manifest"));
        }

        let mut toc = vec![0u8; toc_body];
        reader.read_exact(&mut toc)?;
        if flags == FLAG_ENCRYPTED_TOC {
            crypto::decrypt_toc(&mut toc);
        }

        let mut entries = Vec::with_capacity(toc_count);
        let mut cur = &toc[..entries_len];
        for _ in 0..toc_count {
            let mut name_digest = [0u8; 16];
            cur.read_exact(&mut name_digest)?;
            entries.push(Entry {
                name_digest,
                first_block: cur.read_u32::<BigEndian>()?,
                length: cur.read_uint::<BigEndian>(5)?,
                offset: cur.read_uint::<BigEndian>(5)?,
            });
        }
        let mut cur = &toc[entries_len..];
        let block_sizes = (0..cur.len() / size_bytes)
            .map(|_| cur.read_uint::<BigEndian>(size_bytes).map(|v| v as u32))
            .collect::<std::io::Result<Vec<_>>>()?;

        let mut psarc = Psarc { reader, file_len, names: Vec::new(), entries, block_sizes, block_size };
        for e in &psarc.entries {
            psarc.validate(e)?;
        }

        let manifest = psarc.entries.remove(0);
        if manifest.length > MAX_MANIFEST_LEN {
            return Err(Error::Corrupt("manifest too large"));
        }
        let text = psarc.inflate(&manifest)?;
        psarc.names = String::from_utf8_lossy(&text)
            .split('\n')
            .map(|s| s.trim_end_matches('\r').to_string())
            .collect();
        if psarc.names.len() == 1 && psarc.names[0].is_empty() {
            psarc.names.clear();
        }
        if psarc.names.len() != psarc.entries.len() {
            return Err(Error::Corrupt("manifest and table of contents disagree"));
        }
        Ok(psarc)
    }

    fn validate(&self, e: &Entry) -> Result<()> {
        if e.offset > self.file_len {
            return Err(Error::Corrupt("entry offset past end of file"));
        }
        // enough blocks must exist to hold the declared length
        let needed = e.length.div_ceil(self.block_size as u64);
        if e.first_block as u64 + needed > self.block_sizes.len() as u64 {
            return Err(Error::Corrupt("entry refers to missing blocks"));
        }
        Ok(())
    }

    /// Entry names in archive order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Table-of-contents records, parallel to [`names`](Self::names).
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Decompressed contents of the entry with this name.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let i = self
            .names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| Error::NotFound(name.into()))?;
        self.read_index(i)
    }

    /// Decompressed contents of the `i`-th entry.
    pub fn read_index(&mut self, i: usize) -> Result<Vec<u8>> {
        let e = self.entries.get(i).cloned().ok_or(Error::Corrupt("entry index out of range"))?;
        self.inflate(&e)
    }

    /// Extracts every entry below `dir`, refusing names that would escape it.
    /// Returns the written paths.
    pub fn extract_all(&mut self, dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
        let dir = dir.as_ref();
        let mut written = Vec::with_capacity(self.names.len());
        for i in 0..self.names.len() {
            let target = dir.join(safe_relative_path(&self.names[i])?);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let data = self.read_index(i)?;
            File::create(&target)?.write_all(&data)?;
            written.push(target);
        }
        Ok(written)
    }

    fn inflate(&mut self, e: &Entry) -> Result<Vec<u8>> {
        let total = usize::try_from(e.length).map_err(|_| Error::Corrupt("entry too large"))?;
        let mut out = Vec::with_capacity(total.min(256 << 20));
        self.reader.seek(SeekFrom::Start(e.offset))?;
        let mut block = e.first_block as usize;
        let mut buf = vec![0u8; self.block_size];

        while out.len() < total {
            let size = *self.block_sizes.get(block).ok_or(Error::Corrupt("ran out of blocks"))? as usize;
            let remaining = total - out.len();
            if size == 0 {
                // full raw block
                self.reader.read_exact(&mut buf)?;
                out.extend_from_slice(&buf[..self.block_size.min(remaining)]);
            } else {
                if size > self.block_size {
                    return Err(Error::Corrupt("block larger than block size"));
                }
                self.reader.read_exact(&mut buf[..size])?;
                let raw = &buf[..size];
                // Blocks are zlib-compressed (best compression header 78 DA) unless
                // compressing did not help. Official files contain raw wem blocks that
                // happen to start with 78 DA, so fall back to raw if inflating fails.
                let inflated = if raw.starts_with(&[0x78, 0xDA]) {
                    let mut d = ZlibDecoder::new(raw).take(self.block_size as u64 + 1);
                    let mut tmp = Vec::with_capacity(self.block_size);
                    match d.read_to_end(&mut tmp) {
                        Ok(_) if tmp.len() <= self.block_size => Some(tmp),
                        _ => None,
                    }
                } else {
                    None
                };
                match inflated {
                    Some(data) => out.extend_from_slice(&data[..data.len().min(remaining)]),
                    None => out.extend_from_slice(&raw[..size.min(remaining)]),
                }
            }
            block += 1;
        }
        Ok(out)
    }
}

/// Turns an archive entry name into a relative path that stays inside the
/// extraction folder.
fn safe_relative_path(name: &str) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        let comp = Path::new(part).components().next();
        match comp {
            Some(Component::Normal(p)) if Path::new(p) == Path::new(part) => out.push(part),
            _ => return Err(Error::UnsafeName(name.into())),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::UnsafeName(name.into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal() {
        assert!(safe_relative_path("songs/bin/x.sng").is_ok());
        assert!(safe_relative_path("../etc/passwd").is_err());
        assert!(safe_relative_path("songs/../../x").is_err());
        assert!(safe_relative_path("/abs/path").is_ok_and(|p| p == Path::new("abs/path")));
        assert!(safe_relative_path("").is_err());
    }
}
