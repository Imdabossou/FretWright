//! The game xblock (entity list) and aggregate graph (asset index).

use crate::spec::PackageSpec;
use crate::{Platform, Rng};

pub fn xblock(spec: &PackageSpec, parts: &[(String, &crate::spec::ArrangementSpec)]) -> Vec<u8> {
    let key = spec.dlc_key.to_lowercase();
    let mut s = String::from("\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<game>\n  <entitySet>\n");
    for (part, arr) in parts {
        s.push_str(&format!(
            "    <entity id=\"{}\" modelName=\"RSEnumerable_Song\" name=\"{}_{}\" iterations=\"0\">\n      <properties>\n",
            arr.persistent_id.replace('-', "").to_lowercase(),
            spec.dlc_key,
            arr.name.as_str()
        ));
        let props = [
            ("Header", format!("urn:database:hsan-db:songs_dlc_{key}")),
            ("Manifest", format!("urn:database:json-db:{key}_{part}")),
            ("SngAsset", format!("urn:application:musicgame-song:{key}_{part}")),
            ("AlbumArtSmall", format!("urn:image:dds:album_{key}_64")),
            ("AlbumArtMedium", format!("urn:image:dds:album_{key}_128")),
            ("AlbumArtLarge", format!("urn:image:dds:album_{key}_256")),
            ("LyricArt", String::new()),
            ("ShowLightsXMLAsset", format!("urn:application:xml:{key}_showlights")),
            ("SoundBank", format!("urn:audio:wwise-sound-bank:song_{key}")),
            ("PreviewSoundBank", format!("urn:audio:wwise-sound-bank:song_{key}_preview")),
        ];
        for (name, value) in props {
            s.push_str(&format!(
                "        <property name=\"{name}\">\n          <set value=\"{value}\" />\n        </property>\n"
            ));
        }
        s.push_str("      </properties>\n    </entity>\n");
    }
    s.push_str("  </entitySet>\n</game>");
    s.into_bytes()
}

struct Item {
    uuid: String,
    llid: Option<String>,
    tags: Vec<&'static str>,
    canonical: String,
    name: String,
    rel_path: String,
    log_path: Option<String>,
}

pub fn aggregate_graph(spec: &PackageSpec, parts: &[String], platform: Platform, rng: &mut Rng) -> Vec<u8> {
    let key = spec.dlc_key.to_lowercase();
    let (sng_dir, audio_dir, platform_tag) = match platform {
        Platform::Pc => ("generic", "windows", "dx9"),
        Platform::Mac => ("macos", "mac", "macos"),
    };
    let mut items: Vec<Item> = Vec::new();
    let mut make = |rng: &mut Rng, name: String, canonical: String, tags: Vec<&'static str>, rel: String, log: Option<String>| {
        items.push(Item {
            uuid: rng.uuid(),
            llid: log.as_ref().map(|_| format!("{:08x}-0000-0000-0000-000000000000", rng.next_positive())),
            tags,
            canonical,
            name,
            rel_path: rel,
            log_path: log,
        });
    };
    let normal = |name: &str, canonical: &str, ext: &str| format!("{canonical}/{name}.{ext}");

    make(rng, key.clone(), "/gamexblocks/nsongs".into(), vec!["emergent-world", "x-world"], normal(&key, "/gamexblocks/nsongs", "xblock"), None);
    // the show lights arrangement comes first (DLC Builder prepends it)
    let sl = format!("{key}_showlights");
    let path = normal(&sl, "/songs/arr", "xml");
    make(rng, sl, "/songs/arr".into(), vec!["application", "xml"], path.clone(), Some(path));
    for part in parts {
        let name = format!("{key}_{part}");
        let canonical = format!("/manifests/songs_dlc_{key}");
        make(rng, name.clone(), canonical.clone(), vec!["database", "json-db"], normal(&name, &canonical, "json"), None);
        let sng_canonical = format!("/songs/bin/{sng_dir}");
        let mut tags = vec!["application", "musicgame-song"];
        if platform == Platform::Mac {
            tags.push("macos");
        }
        make(rng, name.clone(), sng_canonical.clone(), tags, format!("{sng_canonical}/{name}.sng"), Some(format!("/songs/bin/{name}.sng")));
    }
    let hsan = format!("songs_dlc_{key}");
    let canonical = format!("/manifests/songs_dlc_{key}");
    make(rng, hsan.clone(), canonical.clone(), vec!["database", "hsan-db"], normal(&hsan, &canonical, "hsan"), None);
    for size in [64, 128, 256] {
        let name = format!("album_{key}_{size}");
        let path = normal(&name, "/gfxassets/album_art", "dds");
        make(rng, name, "/gfxassets/album_art".into(), vec!["dds", "image"], path.clone(), Some(path));
    }
    for bank in [format!("song_{key}"), format!("song_{key}_preview")] {
        let canonical = format!("/audio/{audio_dir}");
        make(rng, bank.clone(), canonical.clone(), vec!["audio", "wwise-sound-bank", platform_tag], format!("{canonical}/{bank}.bnk"), Some(format!("/audio/{bank}.bnk")));
    }

    let line = |uuid: &str, kind: &str, value: &str| format!("<urn:uuid:{uuid}> <http://emergent.net/aweb/1.0/{kind}> \"{value}\".");
    let mut out = String::from("\u{feff}");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for t in &it.tags {
            out.push_str(&line(&it.uuid, "tag", t));
            out.push('\n');
        }
        out.push_str(&line(&it.uuid, "canonical", &it.canonical));
        out.push('\n');
        out.push_str(&line(&it.uuid, "name", &it.name));
        out.push('\n');
        if let (Some(llid), Some(log)) = (&it.llid, &it.log_path) {
            out.push_str(&line(&it.uuid, "llid", llid));
            out.push('\n');
            out.push_str(&line(&it.uuid, "logpath", log));
            out.push('\n');
        }
        out.push_str(&line(&it.uuid, "relpath", &it.rel_path));
    }
    out.into_bytes()
}
