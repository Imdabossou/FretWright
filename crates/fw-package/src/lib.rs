//! Assembles a complete Rocksmith 2014 CDLC package (.psarc).
//!
//! Given the arrangements, encoded audio and metadata ([`PackageSpec`]), this
//! runs the pre-build processing (phrases, fixes, dynamic difficulty),
//! converts to SNG and writes every file the game expects: manifests and the
//! manifest header, the xblock and aggregate graph, Wwise sound banks, album
//! art, show lights and the flat models.
//!
//! Ports Rocksmith2014.NET's `PackageBuilder` and helpers (MIT, Tapio
//! Malmberg). Unlike it, builds are reproducible: IDs it draws at random
//! (sound banks, graph UUIDs, show light colours) come from a generator
//! seeded with the arrangement's persistent ID.

mod bnk;
pub mod dds;
mod json;
mod manifest;
mod showlights;
pub mod spec;
pub mod strings;
mod techniques;
mod xblock;

use rayon::prelude::*;

pub use spec::{ArrangementName, ArrangementSpec, PackageSpec, Priority};
pub use strings::Sortable;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the package needs at least one arrangement")]
    NoArrangements,
    #[error("arrangement processing failed: {0}")]
    Improve(#[from] fw_improve::Error),
    #[error("dynamic difficulty generation failed: {0}")]
    Dd(#[from] fw_dd::Error),
    #[error("conversion to SNG failed: {0}")]
    Convert(#[from] fw_convert::Error),
    #[error("unknown section name {0:?}")]
    UnknownSection(String),
    #[error("album art: {0}")]
    Image(String),
    #[error("writing the package failed: {0}")]
    Psarc(#[from] fw_psarc::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Pc,
    Mac,
}

impl Platform {
    /// Package file name suffix.
    pub fn suffix(self) -> &'static str {
        match self {
            Platform::Pc => "_p",
            Platform::Mac => "_m",
        }
    }
    fn sng(self) -> fw_sng::Platform {
        match self {
            Platform::Pc => fw_sng::Platform::Pc,
            Platform::Mac => fw_sng::Platform::Mac,
        }
    }
}

/// Deterministic pseudo-random numbers (SplitMix64).
pub(crate) struct Rng(u64);

impl Rng {
    fn new(seed: &str) -> Rng {
        Rng(seed.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// A positive i32, like .NET's Random.Next().
    pub(crate) fn next_positive(&mut self) -> i32 {
        (self.next() >> 33) as i32
    }
    pub(crate) fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }
    /// A version-4 style UUID string.
    pub(crate) fn uuid(&mut self) -> String {
        let a = self.next();
        let b = self.next();
        format!(
            "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
            a >> 32,
            (a >> 16) & 0xffff,
            a & 0x0fff,
            ((b >> 48) & 0x3fff) | 0x8000,
            b & 0xffff_ffff_ffff
        )
    }
}

/// An arrangement after pre-build processing and conversion.
pub struct Prepared {
    pub xml: fw_xml::InstrumentalArrangement,
    pub sng: fw_sng::Sng,
    pub counts: fw_convert::NoteCounts,
}

/// Phrases, fixes, dynamic difficulty and SNG conversion, as a DLC Builder build does.
pub fn prepare(arr: &ArrangementSpec, part: i16) -> Result<Prepared> {
    let mut xml = arr.xml.clone();
    xml.metadata.part = part;
    xml.metadata.tuning = arr.tuning;
    // tone change IDs follow the order of the tone names
    let names = xml.tones.names.clone();
    for t in &mut xml.tones.changes {
        t.id = names.iter().position(|n| n.as_deref() == Some(t.name.as_str())).map_or(u8::MAX, |i| i as u8);
    }
    if xml.version < 8 {
        fw_improve::fixes::fix_high_density(&mut xml);
    }
    fw_improve::generate_phrases_if_needed(&mut xml)?;
    fw_improve::apply_all(&mut xml)?;
    if xml.levels.len() == 1 {
        fw_dd::generate(&fw_dd::Config::default(), &mut xml)?;
    }
    let (sng, counts) = fw_convert::convert(&xml)?;
    Ok(Prepared { xml, sng, counts })
}

fn part_names(spec: &PackageSpec) -> Vec<(i32, String)> {
    spec.arrangements
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let part = 1 + spec.arrangements[..i].iter().filter(|x| x.name == a.name).count() as i32;
            let base = a.name.as_str().to_lowercase();
            (part, if part == 1 { base } else { format!("{base}{part}") })
        })
        .collect()
}

/// All package entries (path, contents) for one platform, in DLC Builder's order.
pub fn entries(spec: &PackageSpec, platform: Platform) -> Result<Vec<(String, Vec<u8>)>> {
    if spec.arrangements.is_empty() {
        return Err(Error::NoArrangements);
    }
    let key = spec.dlc_key.to_lowercase();
    let parts = part_names(spec);
    let prepared: Vec<Prepared> = spec
        .arrangements
        .par_iter()
        .zip(parts.par_iter())
        .map(|(a, (part, _))| prepare(a, *part as i16))
        .collect::<Result<_>>()?;
    let mut rng = Rng::new(&spec.arrangements[0].persistent_id);

    let tone: serde_json::Value =
        serde_json::from_str(include_str!("../res/default_bass_tone.json")).expect("embedded tone JSON is valid");
    let tone = json::J::from_serde(&tone);

    let built: Vec<manifest::Built> = spec
        .arrangements
        .iter()
        .zip(&prepared)
        .zip(&parts)
        .map(|((a, p), (part, name))| manifest::Built {
            spec,
            arr_spec: a,
            source: &a.xml,
            sng: &p.sng,
            counts: p.counts,
            part_name: name.clone(),
            part: *part,
        })
        .collect();

    let (sng_dir, audio_dir) = match platform {
        Platform::Pc => ("generic", "windows"),
        Platform::Mac => ("macos", "mac"),
    };
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();

    for (b, p) in built.iter().zip(&prepared) {
        out.push((format!("songs/bin/{sng_dir}/{key}_{}.sng", b.part_name), p.sng.to_packed(platform.sng())));
    }
    // show lights from the first lead arrangement, else the first one
    let sl_source = spec.arrangements.iter().position(|a| a.route_mask() == 1).unwrap_or(0);
    let lights = showlights::generate(&prepared[sl_source].sng, &mut rng);
    out.push((format!("songs/arr/{key}_showlights.xml"), showlights::to_xml(&lights)));

    let headers: Vec<(String, json::J)> =
        built.iter().map(|b| (manifest::persistent_id_key(b.arr_spec), manifest::header_attributes(b))).collect();
    out.push((
        format!("manifests/songs_dlc_{key}/songs_dlc_{key}.hsan"),
        manifest::manifest(headers, true).to_string_pretty().into_bytes(),
    ));
    for b in &built {
        let attrs = manifest::attributes(b, &tone)?;
        let m = manifest::manifest(vec![(manifest::persistent_id_key(b.arr_spec), attrs)], false);
        out.push((format!("manifests/songs_dlc_{key}/{key}_{}.json", b.part_name), m.to_string_pretty().into_bytes()));
    }
    for (size, dds) in dds::album_art(spec.album_art.as_deref(), &spec.artist.value, &spec.title.value)? {
        out.push((format!("gfxassets/album_art/album_{key}_{size}.dds"), dds));
    }
    let xblock_parts: Vec<(String, &ArrangementSpec)> =
        parts.iter().zip(&spec.arrangements).map(|((_, n), a)| (n.clone(), a)).collect();
    out.push((format!("gamexblocks/nsongs/{key}.xblock"), xblock::xblock(spec, &xblock_parts)));
    out.push(("flatmodels/rs/rsenumerable_song.flat".into(), include_bytes!("../res/rsenumerable_song.flat").to_vec()));
    out.push(("flatmodels/rs/rsenumerable_root.flat".into(), include_bytes!("../res/rsenumerable_root.flat").to_vec()));
    let names: Vec<String> = parts.iter().map(|(_, n)| n.clone()).collect();
    out.push((format!("{key}_aggregategraph.nt"), xblock::aggregate_graph(spec, &names, platform, &mut rng)));

    for (bank, wem, volume) in [
        (format!("{}_Preview", spec.dlc_key), &spec.preview_wem, spec.preview_volume),
        (spec.dlc_key.clone(), &spec.audio_wem, spec.volume),
    ] {
        let (data, file_id) = bnk::generate(&bank, wem, volume, platform, &mut rng);
        out.push((format!("audio/{audio_dir}/song_{}.bnk", bank.to_lowercase()), data));
        out.push((format!("audio/{audio_dir}/{file_id}.wem"), wem.clone()));
    }

    let toolkit = format!(
        "Toolkit version: {}\nPackage Author: {}\nPackage Version: {}\nPackage Comment: Remastered",
        spec.builder_name, spec.author, spec.version
    );
    out.push(("toolkit.version".into(), toolkit.into_bytes()));
    out.push(("appid.appid".into(), spec.app_id.to_string().into_bytes()));
    Ok(out)
}

/// The package file name (without directory) DLC Builder would use.
pub fn file_name(spec: &PackageSpec, platform: Platform) -> String {
    format!(
        "{}_{}_v{}{}.psarc",
        strings::file_name(&spec.artist.sort),
        strings::file_name(&spec.title.sort),
        spec.version.replace('.', "_"),
        platform.suffix()
    )
}

/// Builds and writes the package; returns the path written.
pub fn build(spec: &PackageSpec, platform: Platform, out_dir: &std::path::Path) -> Result<std::path::PathBuf> {
    let entries = entries(spec, platform)?;
    let path = out_dir.join(file_name(spec, platform));
    fw_psarc::write_file(&path, &entries, true)?;
    Ok(path)
}
