"""FretWright Bass Detect — backend routes and job worker.

Routes live under ``/api/plugins/fretwright/``. Detection runs on one background
thread (never inside a request handler). The plugin ships no separator and no
Python ML stack: it reads the bass stem feedBack's Stem Splitter already wrote into
the pak and hands it to ``fretwright-cli``, a single native binary.
"""
from __future__ import annotations

import hashlib
import json
import logging
import queue
import shutil
import threading
import time
import uuid
from pathlib import Path

from fastapi import Body, FastAPI

P = "/api/plugins/fretwright"
DEFAULT_CONFIG = {"cli_path": "", "device": "auto", "own_separation": False}


class Worker:
    def __init__(self, context: dict, bridge, pak_io):
        self.bridge, self.pak_io = bridge, pak_io
        self.config_dir = Path(context["config_dir"])
        self.cache_root = self.config_dir / "fretwright_cache"
        self.config_file = self.config_dir / "fretwright.json"
        self.log = context.get("log") or logging.getLogger("feedBack.plugin.fretwright")
        self.get_dlc_dir = context.get("get_dlc_dir")
        self.meta_db = context.get("meta_db")
        self.extract_meta = context.get("extract_meta")
        self.jobs: dict[str, dict] = {}
        self.lock = threading.Lock()
        self.q: queue.Queue[str] = queue.Queue()
        self._cancel: set[str] = set()
        threading.Thread(target=self._loop, name="fretwright-worker", daemon=True).start()

    # ── config ───────────────────────────────────────────────────────────────
    def config(self) -> dict:
        try:
            data = json.loads(self.config_file.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            data = {}
        return {**DEFAULT_CONFIG, **{k: data[k] for k in DEFAULT_CONFIG if k in data}}

    def save_config(self, body: dict) -> dict:
        cfg = self.config()
        if isinstance(body.get("cli_path"), str):
            cfg["cli_path"] = body["cli_path"].strip()
        if body.get("device") in ("auto", "cpu", "gpu"):
            cfg["device"] = body["device"]
        if "own_separation" in body:
            cfg["own_separation"] = bool(body["own_separation"])
        self.config_dir.mkdir(parents=True, exist_ok=True)
        self.config_file.write_text(json.dumps(cfg, indent=2), encoding="utf-8")
        return cfg

    def cli(self) -> str | None:
        return self.bridge.find_cli(self.config()["cli_path"])

    # ── songs ────────────────────────────────────────────────────────────────
    def resolve(self, filename: str) -> Path:
        dlc = self.get_dlc_dir() if self.get_dlc_dir else None
        if not dlc:
            raise RuntimeError("library folder not configured")
        from safepath import safe_join  # core lib, on sys.path in the host
        target = safe_join(Path(dlc).resolve(), filename)
        if target is None:
            raise RuntimeError(f"unsafe song path: {filename!r}")
        if not Path(target).exists():
            raise FileNotFoundError(f"song not found: {filename}")
        return Path(target)

    def song_state(self, filename: str) -> dict:
        manifest = self.pak_io.read_manifest(self.resolve(filename))
        arrs = manifest.get("arrangements") or []
        idx = next((i for i, a in enumerate(arrs)
                    if isinstance(a, dict) and a.get("id") == self.bridge.ARR_ID), None)
        return {
            "filename": filename,
            "has_bass_stem": self.pak_io.stem_file(manifest, "bass") is not None,
            "arrangement_index": idx,
            "detection": manifest.get("bass_detection"),
        }

    def _reindex(self, filename: str, pak: Path) -> None:
        if not (self.meta_db and self.extract_meta):
            return
        try:
            st = pak.stat()
            self.meta_db.put(filename, st.st_mtime, st.st_size, self.extract_meta(pak))
        except Exception as e:  # noqa: BLE001 - a stale row is not worth failing the job
            self.log.warning("fretwright: reindex of %s failed: %s", filename, e)

    # ── jobs ─────────────────────────────────────────────────────────────────
    def enqueue(self, filename: str) -> dict:
        with self.lock:
            for j in self.jobs.values():
                if j["filename"] == filename and j["status"] in ("queued", "running"):
                    return j
            job = {"id": uuid.uuid4().hex[:12], "filename": filename, "status": "queued",
                   "progress": 0.0, "message": "Waiting", "created": time.time()}
            self.jobs[job["id"]] = job
        self.q.put(job["id"])
        return job

    def cancel(self, job_id: str) -> None:
        with self.lock:
            self._cancel.add(job_id)

    def list(self) -> list[dict]:
        with self.lock:
            return sorted((dict(j) for j in self.jobs.values()), key=lambda j: -j["created"])

    def _update(self, job: dict, **fields) -> None:
        with self.lock:
            job.update(fields)

    def _loop(self) -> None:
        while True:
            job = self.jobs.get(self.q.get())
            if not job:
                continue
            if job["id"] in self._cancel:
                self._update(job, status="canceled", message="Canceled")
                continue
            self._update(job, status="running", message="Starting")
            try:
                result = self._detect(job)
                self._update(job, status="done", progress=1.0, **result)
            except self.bridge.Cancelled:
                self._update(job, status="canceled", message="Canceled")
            except Exception as e:
                self.log.exception("fretwright: detection failed for %s", job["filename"])
                self._update(job, status="failed", message=str(e))

    def _detect(self, job: dict) -> dict:
        b, pak_io = self.bridge, self.pak_io
        cfg = self.config()
        cli = self.cli()
        if not cli:
            raise RuntimeError("fretwright-cli not found: set its path in the plugin settings")
        pak = self.resolve(job["filename"])
        manifest = pak_io.read_manifest(pak)

        mix_rel = pak_io.mix_file(manifest)
        mix = pak_io.read_member_bytes(pak, mix_rel) if mix_rel else None
        if mix is None:
            raise RuntimeError("this song has no audio")
        stem_rel = pak_io.stem_file(manifest, "bass")
        stem = pak_io.read_member_bytes(pak, stem_rel) if stem_rel else None
        if stem is None and not cfg["own_separation"]:
            raise RuntimeError("no bass stem yet: split this song with Stem Splitter first "
                               "(or allow FretWright's own Demucs in the settings)")

        # One cache folder per song keeps the beat tracking between runs.
        cache = self.cache_root / hashlib.sha1(job["filename"].encode()).hexdigest()[:16]
        cache.mkdir(parents=True, exist_ok=True)
        mix_path = cache / ("mix" + (Path(mix_rel).suffix or ".ogg"))
        mix_path.write_bytes(mix)
        if stem is not None:
            separator = b.FEEDPAK_STEM
            # FretWright decodes by content, so an Ogg/FLAC/MP3 stem under this name is fine.
            stem_path = cache / "stems" / separator / "bass.wav"
            stem_path.parent.mkdir(parents=True, exist_ok=True)
            stem_path.write_bytes(stem)
            sep = manifest.get("stem_separation") or {}
            stem_source = f"feedpak:{sep.get('model', 'bass stem')}"
        else:
            separator = "htdemucs_ft"
            stem_source = "fretwright:htdemucs_ft"

        tuning = b.bass_tuning(manifest)
        detection = b.run_detection(
            cli, mix_path, cache, open_strings=b.open_strings(tuning), separator=separator,
            device=cfg["device"],
            progress=lambda f, m: self._update(job, progress=round(f * 0.95, 3), message=m),
            cancelled=lambda: job["id"] in self._cancel,
        )
        self._update(job, progress=0.96, message="Writing arrangement")
        arrangement = b.to_arrangement(detection, tuning)
        version = (b.cli_info(cli).get("version") or "").replace("fretwright-cli", "").strip()
        new_manifest, idx = b.with_arrangement(manifest, tuning, b.provenance(version, stem_source))
        pak_io.repack(pak, manifest=new_manifest,
                      add_files={b.ARR_FILE: json.dumps(arrangement, separators=(",", ":")).encode()})
        self._reindex(job["filename"], pak)
        mix_path.unlink(missing_ok=True)
        return {
            "message": f"{len(arrangement['notes'])} notes",
            "notes": len(arrangement["notes"]),
            "arrangement_index": idx,
            "warnings": detection.get("warnings") or [],
            "seconds": round(detection.get("seconds") or 0.0, 1),
        }


def setup(app: FastAPI, context: dict) -> None:
    load = context.get("load_sibling")
    bridge = load("bridge") if load else __import__("bridge")
    pak_io = load("pak_io") if load else __import__("pak_io")
    w = Worker(context, bridge, pak_io)

    @app.get(f"{P}/status")
    def status():
        cli = w.cli()
        return {"config": w.config(), "cli": bridge.cli_info(cli) if cli else None}

    @app.post(f"{P}/config")
    def save_config(body: dict = Body(...)):  # noqa: B008
        return w.save_config(body or {})

    @app.get(f"{P}/songs")
    def songs(q: str = "", page: int = 0, size: int = 50):
        if not w.meta_db:
            return {"songs": [], "total": 0}
        rows, total = w.meta_db.query_page(q=q, page=max(0, page), size=min(max(1, size), 200),
                                           sort="artist")
        return {"total": total, "songs": [{
            "filename": r.get("filename"), "title": r.get("title"), "artist": r.get("artist"),
            "has_bass_stem": "bass" in (r.get("stem_ids") or []),
        } for r in rows]}

    @app.get(f"{P}/song")
    def song(filename: str):
        try:
            return w.song_state(filename)
        except Exception as e:  # noqa: BLE001
            return {"error": str(e)}

    @app.post(f"{P}/detect")
    def detect(body: dict = Body(...)):  # noqa: B008
        filename = (body or {}).get("filename")
        if not isinstance(filename, str) or not filename:
            return {"error": "no filename"}
        try:
            w.resolve(filename)
        except Exception as e:  # noqa: BLE001
            return {"error": str(e)}
        return {"job": w.enqueue(filename)}

    @app.get(f"{P}/jobs")
    def jobs():
        return {"jobs": w.list()}

    @app.post(f"{P}/jobs/{{job_id}}/cancel")
    def cancel(job_id: str):
        w.cancel(job_id)
        return {"ok": True}

    @app.post(f"{P}/clear_cache")
    def clear_cache():
        shutil.rmtree(w.cache_root, ignore_errors=True)
        return {"ok": True}
