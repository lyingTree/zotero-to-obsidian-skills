"""CI-only retrieval of exact official native llama-server distributions.
No user Python dependency. Archives are hashed against the committed release
catalog before any file is materialized or executed. No arbitrary extraction.
"""
from __future__ import annotations
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tarfile
import zipfile
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
TARGET = os.environ["Z2O_LLAMA_TARGET"]
DEST = Path(os.environ["Z2O_LLAMA_FIXTURE_DIR"]).resolve()
CATALOG = json.loads(ROOT.joinpath("native", "minicpm-runtime-assets.json").read_text(encoding="utf-8"))

def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def download(spec: dict, release: str) -> bytes:
    url = spec["url"]
    if release not in {"b11901", "b11928"} or url != (
        f"https://github.com/ggml-org/llama.cpp/releases/download/{release}/{spec['name']}"
    ):
        raise ValueError("Unexpected release location.")
    if not 0 < spec["size"] <= 512 * 1024 * 1024:
        raise ValueError("Invalid archive size.")
    with urlopen(Request(url, headers={"User-Agent": "z2o-native-ci"}), timeout=120) as stream:
        raw = stream.read(spec["size"] + 1)
    if len(raw) != spec["size"] or digest(raw) != spec["sha256"]:
        raise ValueError("Official archive identity differs.")
    return raw

def safe(name: str) -> PurePosixPath:
    p = PurePosixPath(name)
    if p.is_absolute() or not p.parts or any(s in ("", ".", "..") or re.search(r'[\\/:<>|"*?\x00-\x1f]', s) or s.endswith((".", " ")) for s in p.parts):
        raise ValueError(f"Unsafe archive path: {name}")
    return p

def extract(spec: dict, dest: Path, release: str) -> dict:
    raw = download(spec, release)
    dest.mkdir(parents=True, exist_ok=False)
    files: dict[str, tuple[bytes, int]] = {}
    links: dict[str, str] = {}
    if spec["name"].endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(raw)) as z:
            for m in z.infolist():
                p = safe(m.filename)
                if m.is_dir():
                    continue
                mode = m.external_attr >> 16
                if stat.S_ISLNK(mode) or not 0 <= m.file_size <= 512 * 1024 * 1024:
                    raise ValueError("Unsupported ZIP entry.")
                if str(p) in files:
                    raise ValueError("Duplicate archive entry.")
                data = z.read(m)
                files[str(p)] = (data, mode)
    else:
        with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as t:
            for m in t:
                p = safe(m.name)
                if m.isdir():
                    continue
                if str(p) in files or str(p) in links:
                    raise ValueError("Duplicate archive entry.")
                if m.issym():
                    # Shared library aliases are copied as actual bytes, never
                    # symlinks. Only a relative same-directory file is allowed.
                    target = safe(m.linkname)
                    links[str(p)] = str(p.parent.joinpath(target))
                    continue
                if m.islnk():
                    links[str(p)] = str(safe(m.linkname))
                    continue
                if not m.isfile() or not 0 <= m.size <= 512 * 1024 * 1024:
                    raise ValueError("Unsupported tar entry.")
                files[str(p)] = (t.extractfile(m).read(), m.mode)
        while links:
            ready = [k for k, v in links.items() if v in files]
            if not ready:
                raise ValueError("Missing or cyclic library alias.")
            for k in ready:
                files[k] = files[links.pop(k)]
    if not files or len(files) > 512 or sum(len(v[0]) for v in files.values()) > 2 * 1024**3:
        raise ValueError("Engine package exceeds extraction budget.")
    folded = set()
    rows = []
    for name, (data, mode) in sorted(files.items()):
        if name.casefold() in folded:
            raise ValueError("Ambiguous archive filename.")
        folded.add(name.casefold())
        target = dest.joinpath(*safe(name).parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        if os.name != "nt":
            target.chmod(0o755 if mode & 0o111 else 0o644)
        rows.append({"path": name, "sha256": digest(data)})
    exe = "llama-server.exe" if os.name == "nt" else "llama-server"
    matches = [name for name in files if PurePosixPath(name).name == exe]
    if len(matches) != 1:
        raise ValueError("No unique native llama-server in official archive.")
    return {"directory": dest.name, "executable": matches[0], "variant": "cpu", "files": rows}

def prepare(catalog: dict, kind: str, folder: str, report: str) -> None:
    spec = catalog["targets"][TARGET]["cpu"]
    release = catalog["release"]
    record = extract(spec, DEST.joinpath(folder), release)
    exe = DEST.joinpath(folder, *PurePosixPath(record["executable"]).parts)
    env = dict(os.environ)
    env.update(PATH=str(exe.parent), CUDA_VISIBLE_DEVICES="", HIP_VISIBLE_DEVICES="")
    if os.name != "nt":
        env.update(LD_LIBRARY_PATH=str(exe.parent), DYLD_LIBRARY_PATH=str(exe.parent))
    result = subprocess.run([str(exe), "--version"], cwd=exe.parent, env=env, capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    stdout = result.stdout.decode(errors="replace")
    stderr = result.stderr.decode(errors="replace")
    if kind == "jev":
        identity = re.search(r"build\s+11928,\s+commit\s+([0-9a-f]{7,40})\b", stdout + stderr)
        if identity is None or not catalog["upstream_commit"].startswith(identity[1]):
            raise RuntimeError("Jev engine version does not identify the pinned upstream commit.")
    evidence = {"schema_version": 1, "target": TARGET, "model_kind": kind, "release": release,
                "archive": spec, "engine": record, "version_stdout": stdout,
                "version_stderr": stderr, "inference_qualified": False}
    DEST.joinpath(report).write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print("Verified native engine launch:", TARGET, kind, release, flush=True)

def main() -> None:
    if TARGET not in CATALOG["targets"]:
        raise ValueError(f"Unsupported native MiniCPM target: {TARGET}")
    jev = None
    if os.environ.get("Z2O_LLAMA_PREPARE_JEV") == "1":
        jev = json.loads(ROOT.joinpath("native", "jev-runtime-assets.json").read_text(encoding="utf-8"))
        if TARGET not in jev["targets"]:
            raise ValueError(f"Unsupported native Jev target: {TARGET}")
    DEST.mkdir(parents=True, exist_ok=False)
    prepare(CATALOG, "minicpm-ocr", "cpu", "runtime-evidence.json")
    if jev is not None:
        prepare(jev, "jev", "jev-cpu", "jev-runtime-evidence.json")

if __name__ == "__main__":
    main()
