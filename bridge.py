"""FretWright <-> feedpak: find the CLI, run detection, and turn its notes into a
feedpak bass arrangement (feedpak spec §6).

Everything here is plain functions so it can be tested without the host.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import time
from collections.abc import Callable
from pathlib import Path

ARR_ID = "bass_fretwright"
ARR_NAME = "Bass (FretWright)"
ARR_FILE = f"arrangements/{ARR_ID}.json"

# E1 A1 D2 G2: the strings a feedpak bass tuning of [0, 0, 0, 0] means.
STANDARD_BASS = (28, 33, 38, 43)

# Name FretWright files the stem under in its cache (stems/<name>/bass.wav).
# Detection only separates when that file is missing, so a stem put there is used as is.
FEEDPAK_STEM = "feedpak"

_PROGRESS = re.compile(r"^\[\s*([\d.]+)%\]\s*(.*)$")


class Cancelled(Exception):
    pass


# ── the CLI ──────────────────────────────────────────────────────────────────

def find_cli(configured: str = "") -> str | None:
    """The fretwright-cli to run: the configured path, $FRETWRIGHT_CLI, or PATH."""
    for cand in (configured, os.environ.get("FRETWRIGHT_CLI", "")):
        if cand and Path(cand).is_file() and os.access(cand, os.X_OK):
            return str(cand)
    return shutil.which("fretwright-cli")


def cli_info(cli: str) -> dict:
    """Version and downloaded models, for the status chip."""
    info: dict = {"path": cli}
    try:
        out = subprocess.run([cli, "--version"], check=False, capture_output=True, text=True, timeout=20)
        info["version"] = (out.stdout or out.stderr).strip()
    except (OSError, subprocess.SubprocessError) as e:
        info["error"] = str(e)
        return info
    try:
        out = subprocess.run([cli, "models", "--json"], check=False, capture_output=True, text=True, timeout=20)
        info["models"] = sorted(json.loads(out.stdout or "{}"))
    except (OSError, subprocess.SubprocessError, ValueError):
        info["models"] = []
    info["beat_model"] = any(m.startswith("beat_this/") for m in info["models"])
    info["demucs_model"] = any(m.startswith("demucs/htdemucs_ft") for m in info["models"])
    return info


def run_detection(cli: str, mix: Path, cache_dir: Path, *, open_strings: list[int],
                  separator: str, device: str = "auto",
                  progress: Callable[[float, str], None] = lambda f, m: None,
                  cancelled: Callable[[], bool] = lambda: False) -> dict:
    """Run ``fretwright-cli detect-audio`` and return its JSON (notes, grid, warnings)."""
    cmd = [cli, "detect-audio", str(mix), "--cache-dir", str(cache_dir),
           "--separator", separator, "--device", device,
           "--open", ",".join(str(m) for m in open_strings)]
    # The JSON goes to a file: it is far bigger than a pipe buffer, and reading
    # stderr for progress while stdout fills up would deadlock.
    out_path = Path(cache_dir) / "detection.json"
    tail: list[str] = []
    with open(out_path, "w", encoding="utf-8") as out:
        proc = subprocess.Popen(cmd, stdout=out, stderr=subprocess.PIPE, text=True)
        assert proc.stderr is not None
        for line in proc.stderr:
            if cancelled():
                proc.kill()
                proc.wait()
                raise Cancelled()
            line = line.strip()
            m = _PROGRESS.match(line)
            if m:
                progress(float(m.group(1)) / 100.0, m.group(2))
            elif line:
                tail = (tail + [line])[-10:]
    if proc.wait() != 0:
        raise RuntimeError("fretwright-cli failed: " + (" | ".join(tail) or f"exit {proc.returncode}"))
    return json.loads(out_path.read_text(encoding="utf-8"))


# ── tuning ───────────────────────────────────────────────────────────────────

def reference_bass(manifest: dict) -> dict | None:
    """An existing (official or hand-made) bass arrangement entry, to copy its tuning."""
    for a in manifest.get("arrangements") or []:
        if not isinstance(a, dict) or a.get("id") == ARR_ID:
            continue
        label = f"{a.get('id', '')} {a.get('name', '')}".lower()
        if str(a.get("type", "")).lower() == "bass" or "bass" in label:
            return a
    return None


def bass_tuning(manifest: dict) -> list[int]:
    """Semitone offsets for a 4-string bass; standard when no bass part says otherwise."""
    ref = reference_bass(manifest)
    tuning = ref.get("tuning") if ref else None
    if isinstance(tuning, list) and len(tuning) >= 4 and all(isinstance(t, int) for t in tuning):
        return list(tuning[:4])
    return [0, 0, 0, 0]


def open_strings(tuning: list[int]) -> list[int]:
    return [base + off for base, off in zip(STANDARD_BASS, tuning)]


# ── detection -> arrangement ────────────────────────────────────────────────

def _anchors(notes: list[dict]) -> list[dict]:
    """Hand positions following the fretted notes (FretWright's own rule)."""
    out: list[dict] = []
    cur: int | None = None
    for n in notes:
        fret = n["f"]
        if fret == 0 or (cur is not None and cur <= fret <= cur + 3):
            continue
        cur = max(1, fret - 1) if cur is not None and fret > cur + 3 else max(1, fret)
        out.append({"time": n["t"], "fret": cur, "width": 4})
    return out or [{"time": 0.0, "fret": 1, "width": 4}]


def _beats(grid: dict) -> list[dict]:
    beats = grid.get("beats") or []
    downbeats = set(grid.get("downbeats") or [])
    out, bar = [], 0
    for i, t in enumerate(beats):
        if i in downbeats:
            bar += 1
        out.append({"time": round(float(t), 3), "measure": bar if i in downbeats else -1})
    return out


def _note(n: dict) -> dict:
    start, end = float(n["start"]), float(n["end"])
    dur = max(0.0, end - start)
    slide, slide_unpitched = int(n.get("slide_to", -1)), int(n.get("slide_unpitch_to", -1))
    bend = float(n.get("bend", 0.0))
    # Same rule as FretWright's .psarc build: short notes are plain hits.
    sustained = dur >= 0.2 or slide >= 0 or slide_unpitched >= 0 or bend > 0 or n.get("link_next")
    out = {"t": round(start, 3), "s": int(n["string"]), "f": int(n["fret"]),
           "sus": round(dur, 3) if sustained else 0.0}
    optional = {
        "sl": (slide, -1), "slu": (slide_unpitched, -1), "bn": (round(bend, 1), 0.0),
        "ho": (bool(n.get("hammer_on")), False), "po": (bool(n.get("pull_off")), False),
        "fhm": (bool(n.get("mute")), False), "pm": (bool(n.get("palm_mute")), False),
        "ac": (bool(n.get("accent")), False), "hm": (bool(n.get("harmonic")), False),
        "vb": (bool(n.get("vibrato")), False), "tr": (bool(n.get("tremolo")), False),
        "tp": (bool(n.get("tap")), False), "slp": (bool(n.get("slap")), False),
        "plk": (bool(n.get("pop")), False), "ln": (bool(n.get("link_next")), False),
    }
    out.update({k: v for k, (v, default) in optional.items() if v != default})
    return out


def to_arrangement(detection: dict, tuning: list[int]) -> dict:
    notes = sorted((_note(n) for n in detection.get("notes") or []), key=lambda n: (n["t"], n["s"]))
    return {
        "name": ARR_NAME,
        "tuning": tuning,
        "capo": 0,
        "notes": notes,
        "chords": [],
        "anchors": _anchors(notes),
        "handshapes": [],
        "templates": [],
        "beats": _beats(detection.get("grid") or {}),
        "sections": [],
    }


def with_arrangement(manifest: dict, tuning: list[int], provenance: dict) -> tuple[dict, int]:
    """The manifest with our arrangement added (or replaced in place) and its index.

    Existing arrangements are never touched; a re-run only replaces our own entry.
    """
    manifest = dict(manifest)
    arrs = [a for a in manifest.get("arrangements") or []]
    entry = {"id": ARR_ID, "name": ARR_NAME, "file": ARR_FILE, "type": "bass",
             "tuning": tuning, "capo": 0}
    idx = next((i for i, a in enumerate(arrs) if isinstance(a, dict) and a.get("id") == ARR_ID), None)
    if idx is None:
        arrs.append(entry)
        idx = len(arrs) - 1
    else:
        arrs[idx] = entry
    manifest["arrangements"] = arrs
    # Extension key (feedpak spec §9): readers that don't know it ignore it.
    manifest["bass_detection"] = provenance
    return manifest, idx


def provenance(cli_version: str, stem_source: str) -> dict:
    return {
        "engine": "fretwright",
        "version": cli_version,
        "arrangement": ARR_ID,
        "stem": stem_source,
        "created": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
