"""Read and rewrite a feedpak (zip file or directory form).

The repack logic follows the Stem Splitter plugin's ``pak_io.py`` (feedBack has no
library API to write a pak): backup + temp file + atomic replace for the zip form,
keeping each surviving member's compression so Ogg stems are not re-deflated.
"""
from __future__ import annotations

import shutil
import zipfile
from pathlib import Path

import yaml

_MANIFEST_NAMES = ("manifest.yaml", "manifest.yml")


def is_zip_form(path: Path) -> bool:
    return Path(path).is_file()


def _norm(rel: str) -> str:
    return rel.replace("\\", "/").lstrip("/")


def read_member_bytes(path: Path, relpath: str) -> bytes | None:
    """One member's bytes, without unpacking the whole archive."""
    path, relpath = Path(path), _norm(relpath)
    if is_zip_form(path):
        try:
            with zipfile.ZipFile(path) as zf:
                return zf.read(relpath)
        except (KeyError, zipfile.BadZipFile, OSError):
            return None
    member = path / relpath
    if member.is_file() and member.resolve().is_relative_to(path.resolve()):
        return member.read_bytes()
    return None


def read_manifest(path: Path) -> dict:
    for name in _MANIFEST_NAMES:
        data = read_member_bytes(path, name)
        if data is not None:
            return yaml.safe_load(data.decode("utf-8")) or {}
    raise FileNotFoundError(f"{path} has no manifest.yaml")


def stem_file(manifest: dict, stem_id: str) -> str | None:
    for stem in manifest.get("stems") or []:
        if isinstance(stem, dict) and str(stem.get("id", "")).lower() == stem_id:
            f = stem.get("file")
            if isinstance(f, str) and f.strip():
                return f.strip()
    return None


def mix_file(manifest: dict) -> str | None:
    """The pre-separation full mix: ``original_audio``, else the ``full`` stem,
    else the first stem (a pack that was never split has only the mix)."""
    orig = manifest.get("original_audio")
    if isinstance(orig, str) and orig.strip():
        return orig.strip()
    full = stem_file(manifest, "full")
    if full:
        return full
    for stem in manifest.get("stems") or []:
        if isinstance(stem, dict) and isinstance(stem.get("file"), str):
            return stem["file"]
    return None


def dump_manifest(manifest: dict) -> str:
    return yaml.safe_dump(manifest, sort_keys=False, allow_unicode=True)


def repack(path: Path, *, add_files: dict[str, bytes], manifest: dict) -> None:
    """Add or replace members and rewrite the manifest."""
    path = Path(path)
    add_files = {_norm(k): v for k, v in add_files.items()}
    if is_zip_form(path):
        _repack_zip(path, add_files, manifest)
    else:
        _repack_dir(path, add_files, manifest)


def _repack_zip(path: Path, add_files: dict[str, bytes], manifest: dict) -> None:
    backup = path.with_name(path.name + ".bak")
    created_backup = not backup.exists()
    if created_backup:
        shutil.copy2(path, backup)
    tmp = path.with_name(path.name + ".tmp")
    with zipfile.ZipFile(path) as zin:
        names = zin.namelist()
        manifest_name = next((n for n in _MANIFEST_NAMES if n in names), "manifest.yaml")
        with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zout:
            for item in zin.infolist():
                if item.filename in _MANIFEST_NAMES or item.filename in add_files:
                    continue
                zout.writestr(item, zin.read(item.filename))
            for rel, data in add_files.items():
                zout.writestr(rel, data)
            zout.writestr(manifest_name, dump_manifest(manifest))
    tmp.replace(path)
    if created_backup:
        backup.unlink(missing_ok=True)


def _repack_dir(path: Path, add_files: dict[str, bytes], manifest: dict) -> None:
    for rel, data in add_files.items():
        target = path / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    mf = path / "manifest.yaml"
    if not mf.exists() and (path / "manifest.yml").exists():
        mf = path / "manifest.yml"
    if mf.exists():
        bak = mf.with_name(mf.name + ".bak")
        if not bak.exists():
            shutil.copy2(mf, bak)
    mf.write_text(dump_manifest(manifest), encoding="utf-8")
