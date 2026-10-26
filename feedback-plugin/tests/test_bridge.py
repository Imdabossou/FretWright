import json
import sys
import zipfile
from pathlib import Path

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import bridge
import pak_io


def _note(start, end, string, fret, **kw):
    return {"start": start, "end": end, "midi": 0, "string": string, "fret": fret,
            "slide_to": -1, "slide_unpitch_to": -1, "bend": 0.0, **kw}


DETECTION = {
    "notes": [
        _note(2.0, 2.1, 1, 0),
        _note(1.0, 1.5, 0, 5, hammer_on=True),
        _note(3.0, 3.1, 2, 9, slide_to=11),
        _note(4.0, 4.05, 0, 3, mute=True),
    ],
    "grid": {"beats": [0.5, 1.0, 1.5, 2.0, 2.5], "downbeats": [0, 4], "tempo": 120.0},
    "warnings": [],
}


def test_arrangement_shape():
    arr = bridge.to_arrangement(DETECTION, [0, 0, 0, 0])
    notes = arr["notes"]
    assert [n["t"] for n in notes] == [1.0, 2.0, 3.0, 4.0]
    assert notes[0] == {"t": 1.0, "s": 0, "f": 5, "sus": 0.5, "ho": True}
    assert notes[1] == {"t": 2.0, "s": 1, "f": 0, "sus": 0.0}  # short note: no sustain
    assert notes[2]["sl"] == 11 and notes[2]["sus"] == 0.1      # a slide keeps its length
    assert notes[3]["fhm"] is True
    assert [b["measure"] for b in arr["beats"]] == [1, -1, -1, -1, 2]
    assert "phrases" not in arr  # spec §6.7: omit, never []


def test_anchors_follow_frets():
    notes = [{"t": 0.0, "f": 5}, {"t": 1.0, "f": 7}, {"t": 2.0, "f": 12}, {"t": 3.0, "f": 0}, {"t": 4.0, "f": 3}]
    assert bridge._anchors(notes) == [
        {"time": 0.0, "fret": 5, "width": 4},
        {"time": 2.0, "fret": 11, "width": 4},
        {"time": 4.0, "fret": 3, "width": 4},
    ]


def test_tuning_from_existing_bass():
    m = {"arrangements": [{"id": "lead", "tuning": [-2, 0, 0, 0, 0, 0]},
                          {"id": "bass", "name": "Bass", "tuning": [-2, 0, 0, 0]}]}
    assert bridge.bass_tuning(m) == [-2, 0, 0, 0]
    assert bridge.open_strings([-2, 0, 0, 0]) == [26, 33, 38, 43]
    assert bridge.bass_tuning({"arrangements": [{"id": "lead"}]}) == [0, 0, 0, 0]


def test_manifest_adds_then_replaces():
    m = {"arrangements": [{"id": "bass", "file": "arrangements/bass.json"}]}
    m1, i1 = bridge.with_arrangement(m, [0, 0, 0, 0], {"engine": "fretwright"})
    m2, i2 = bridge.with_arrangement(m1, [-1, -1, -1, -1], {"engine": "fretwright"})
    assert i1 == i2 == 1
    assert len(m2["arrangements"]) == 2
    assert m2["arrangements"][0] == {"id": "bass", "file": "arrangements/bass.json"}
    assert m2["arrangements"][1]["tuning"] == [-1, -1, -1, -1]
    assert m["arrangements"] == [{"id": "bass", "file": "arrangements/bass.json"}]  # input untouched


def _zip_pak(path: Path) -> None:
    manifest = {"title": "t", "artist": "a", "duration": 1.0,
                "arrangements": [{"id": "bass", "file": "arrangements/bass.json"}],
                "stems": [{"id": "bass", "file": "stems/bass.ogg"}, {"id": "full", "file": "stems/full.ogg"}]}
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("manifest.yaml", yaml.safe_dump(manifest))
        z.writestr("arrangements/bass.json", "{}")
        z.writestr("stems/bass.ogg", b"BASS", compress_type=zipfile.ZIP_STORED)
        z.writestr("stems/full.ogg", b"FULL", compress_type=zipfile.ZIP_STORED)


def test_zip_repack(tmp_path):
    pak = tmp_path / "song.feedpak"
    _zip_pak(pak)
    m = pak_io.read_manifest(pak)
    assert pak_io.stem_file(m, "bass") == "stems/bass.ogg"
    assert pak_io.mix_file(m) == "stems/full.ogg"
    new, _ = bridge.with_arrangement(m, [0, 0, 0, 0], {"engine": "fretwright"})
    pak_io.repack(pak, manifest=new, add_files={bridge.ARR_FILE: b'{"notes":[]}'})
    with zipfile.ZipFile(pak) as z:
        assert z.getinfo("stems/bass.ogg").compress_type == zipfile.ZIP_STORED
        assert json.loads(z.read(bridge.ARR_FILE)) == {"notes": []}
        assert z.read("arrangements/bass.json") == b"{}"
    assert [a["id"] for a in pak_io.read_manifest(pak)["arrangements"]] == ["bass", bridge.ARR_ID]
    assert not (tmp_path / "song.feedpak.bak").exists()


def test_dir_repack(tmp_path):
    pak = tmp_path / "song.feedpak"
    pak.mkdir()
    (pak / "manifest.yaml").write_text(yaml.safe_dump({"title": "t", "arrangements": []}))
    pak_io.repack(pak, manifest={"title": "t2"}, add_files={"arrangements/x.json": b"{}"})
    assert pak_io.read_manifest(pak) == {"title": "t2"}
    assert (pak / "arrangements/x.json").read_bytes() == b"{}"
    assert (pak / "manifest.yaml.bak").exists()
