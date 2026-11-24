"""CI-only qualification fixture; never part of the installed user runtime.

Fetch official CPU ONNX Runtime and the PP-OCRv6 Tiny models at resolved
immutable upstream revisions, verifying published release/LFS/blob identities.
This produces candidate test provenance, NOT a signed production installer
catalog. Missing metadata fails closed. No downloaded Python or executable
script is run. The CPU library is loaded later by the actual Rust program.
"""
from __future__ import annotations

from hashlib import sha1, sha256
from io import BytesIO
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import zipfile
from urllib.parse import urlparse
from urllib.request import Request, urlopen

ROOT = Path(os.environ["Z2O_OCR_FIXTURE_DIR"]).resolve()
ORT_VERSION = "1.32.0"
# Exact published PyPI file digests for the supported architecture candidates.
ASSET_CATALOG = Path(__file__).resolve().parents[1].joinpath("native", "ocr-cpu-assets.json")
TARGET = os.environ.get("Z2O_OCR_TARGET", "x86_64-unknown-linux-gnu")
RUNTIME_SPEC = json.loads(ASSET_CATALOG.read_text(encoding="utf-8"))["targets"].get(TARGET)
if RUNTIME_SPEC is None:
    raise ValueError(f"Unsupported native CPU OCR target: {TARGET}")
if RUNTIME_SPEC.get("status") == "blocked":
    raise RuntimeError(RUNTIME_SPEC["reason"])
ORT_WHEEL_NAME = RUNTIME_SPEC["wheel"]
ORT_WHEEL_SHA256 = RUNTIME_SPEC["sha256"]
HF_ORG = "PaddlePaddle"
MAX_DOWNLOAD = 256 * 1024 * 1024
HEX40 = re.compile(r"[0-9a-f]{40}")
HEX64 = re.compile(r"[0-9a-f]{64}")


def download(url: str, *, api: bool = False, limit: int = MAX_DOWNLOAD) -> bytes:
    headers = {"User-Agent": "z2o-native-ci-qualification"}
    if api and url.startswith("https://api.github.com/"):
        headers["Accept"] = "application/vnd.github+json"
        token = os.environ.get("GITHUB_TOKEN")
        if token:
            headers["Authorization"] = f"Bearer {token}"
    with urlopen(Request(url, headers=headers), timeout=120) as response:
        data = response.read(limit + 1)
    if not data or len(data) > limit:
        raise ValueError(f"Missing or oversized artifact: {url}")
    return data


def metadata(url: str, *, github: bool = False) -> dict:
    value = json.loads(download(url, api=github, limit=16 * 1024 * 1024))
    if not isinstance(value, dict):
        raise ValueError(f"Expected actual metadata object: {url}")
    return value


def write(relative: str, content: bytes) -> dict:
    target = ROOT.joinpath(relative)
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("xb") as stream:
        stream.write(content)
    return {"path": relative, "sha256": sha256(content).hexdigest()}


def official_cpu_runtime() -> tuple[dict, dict]:
    # The v1.28.3 GitHub release API returned 404 in CI. Use Microsoft's
    # stable CPU wheel as a native-library distribution container, not as a
    # Python runtime dependency. ort rc.13 requests the backward-compatible
    # C API 28; the real Rust smoke test must verify this combination.
    metadata_url = f"https://pypi.org/pypi/onnxruntime/{ORT_VERSION}/json"
    release = metadata(metadata_url)
    info = release.get("info", {})
    if info.get("name", "").lower() != "onnxruntime" or info.get("version") != ORT_VERSION:
        raise ValueError("Unexpected official runtime package metadata.")
    matches = [row for row in release.get("urls", []) if row.get("filename") == ORT_WHEEL_NAME]
    if len(matches) != 1:
        raise ValueError("Pinned CPU wheel is unavailable.")
    asset = matches[0]
    url = asset["url"]
    parsed = urlparse(url)
    if (asset.get("yanked") or parsed.scheme != "https" or parsed.netloc != "files.pythonhosted.org"
            or PurePosixPath(parsed.path).name != ORT_WHEEL_NAME
            or asset.get("digests", {}).get("sha256") != ORT_WHEEL_SHA256):
        raise ValueError("Unexpected or modified pinned CPU wheel identity.")
    archive = download(url)
    if len(archive) != asset.get("size") or sha256(archive).hexdigest() != ORT_WHEEL_SHA256:
        raise ValueError("Pinned CPU wheel size or SHA-256 mismatch.")
    records = {}
    runtime_names = set(RUNTIME_SPEC["runtime_names"])
    permitted = runtime_names | set(RUNTIME_SPEC["dependencies"])
    with zipfile.ZipFile(BytesIO(archive)) as wheel:
        # Only these native regular shared libraries are materialized. Never
        # install/import a wheel, follow archive symlinks, or extract scripts.
        for member in wheel.infolist():
            path = PurePosixPath(member.filename)
            if path.parts[:2] != ("onnxruntime", "capi") or len(path.parts) != 3 or path.name not in permitted:
                continue
            mode = member.external_attr >> 16
            if member.is_dir() or stat.S_ISLNK(mode) or (stat.S_IFMT(mode) not in (0, stat.S_IFREG)):
                raise ValueError("CPU wheel library is not a regular file.")
            if path.name in records or not 0 < member.file_size <= MAX_DOWNLOAD:
                raise ValueError("Duplicate or oversized CPU library.")
            with wheel.open(member) as stream:
                data = stream.read(member.file_size + 1)
            if len(data) != member.file_size:
                raise ValueError("Truncated or oversized library.")
            records[path.name] = write(f"runtime/{path.name}", data)
    found = runtime_names & records.keys()
    if len(found) != 1:
        raise ValueError("CPU wheel does not contain exactly one expected standalone runtime library.")
    runtime_name = next(iter(found))
    return (
        {"platform": RUNTIME_SPEC["platform"], "architecture": RUNTIME_SPEC["architecture"], "version": ORT_VERSION,
         "file": records[runtime_name],
         "dependencies": [b for n, b in records.items() if n != runtime_name]},
        {"distribution": "Microsoft onnxruntime CPU wheel (native libraries only)",
         "metadata": metadata_url, "wheel": ORT_WHEEL_NAME, "archive": url,
         "archive_sha256": ORT_WHEEL_SHA256, "libraries": records,
         "qualification": "candidate API-28 compatibility; real Rust inference required"},
    )


def model(repo: str, filenames: tuple[str, ...], pinned: dict | None = None) -> tuple[dict[str, bytes], dict]:
    identity = f"{HF_ORG}/{repo}"
    if pinned is not None:
        revision = pinned.get("revision", "")
        if pinned.get("repository") != identity or not HEX40.fullmatch(revision):
            raise ValueError("Shared model provenance does not bind the expected repository/revision.")
        entry = metadata(f"https://huggingface.co/api/models/{identity}/revision/{revision}?blobs=true")
    else:
        entry = metadata(f"https://huggingface.co/api/models/{identity}?blobs=true")
    revision = entry.get("sha", "")
    if entry.get("id") != identity or not HEX40.fullmatch(revision):
        raise ValueError("Cannot establish an immutable official model revision.")
    files = {row["rfilename"]: row for row in entry.get("siblings", [])}
    result, provenance = {}, {}
    for name in filenames:
        row = files[name]
        url = f"https://huggingface.co/{identity}/resolve/{revision}/{name}"
        data = download(url)
        if "size" in row and len(data) != row["size"]:
            raise ValueError(f"Model file size mismatch: {identity}/{name}")
        lfs = row.get("lfs") or {}
        digest = sha256(data).hexdigest()
        if lfs:
            expected = lfs.get("sha256")
            if not isinstance(expected, str) or not HEX64.fullmatch(expected) or digest != expected:
                raise ValueError(f"Model LFS SHA-256 mismatch: {identity}/{name}")
            if "size" in lfs and len(data) != lfs["size"]:
                raise ValueError("LFS size mismatch.")
        else:
            expected = row.get("blobId", "")
            # Ordinary Git blobs use SHA-1 over the canonical blob envelope.
            actual = sha1(f"blob {len(data)}\0".encode("ascii") + data).hexdigest()
            if not HEX40.fullmatch(expected) or actual != expected:
                raise ValueError(f"Model Git blob identity mismatch: {identity}/{name}")
        if name.endswith(".onnx") and not lfs:
            raise ValueError("The expected standalone ONNX weight lacks LFS provenance.")
        result[name] = data
        provenance[name] = {"url": url, "sha256": digest, "upstream_identity": expected}
    record = {"repository": identity, "revision": revision, "files": provenance}
    if pinned is not None and record != pinned:
        raise ValueError("Shared CPU qualification must use the same exact model artifacts.")
    return result, record


def recognition_dictionary(config: bytes) -> bytes:
    # Match the pinned ppocr-rs ModelHub extractor; use the model's own
    # ordered vocabulary, including empty slots and YAML single-quote escapes.
    chars = []
    active = False
    for line in config.decode("utf-8").splitlines():
        trimmed = line.lstrip()
        if not active:
            active = trimmed.startswith("character_dict:")
            continue
        if trimmed.startswith("- "):
            token = trimmed[2:]
            if len(token) >= 2 and token.startswith("'") and token.endswith("'"):
                token = token[1:-1].replace("''", "'")
            chars.append(token)
        elif trimmed and not trimmed.startswith("-"):
            break
    if len(chars) < 1000:
        raise ValueError("Missing or incomplete recognition vocabulary.")
    return ("\n".join(chars) + "\n").encode("utf-8")


def main() -> None:
    ROOT.mkdir(parents=True, exist_ok=False)
    print("[1/4] Verify official CPU runtime", flush=True)
    runtime, runtime_provenance = official_cpu_runtime()
    print("[2/4] Resolve and verify official Tiny detection and recognition weights", flush=True)
    shared_path = os.environ.get("Z2O_OCR_SHARED_FIXTURE_DIR")
    shared = None
    if shared_path:
        shared_root = Path(shared_path).resolve()
        raw_manifest = shared_root.joinpath("package.json").read_bytes()
        expected = shared_root.joinpath("package.sha256").read_text(encoding="ascii").strip()
        if sha256(raw_manifest).hexdigest() != expected:
            raise ValueError("Shared qualification manifest changed.")
        shared = json.loads(raw_manifest)
    det, det_source = model("PP-OCRv6_tiny_det_onnx", ("inference.onnx",),
                            shared["provenance"]["detector"] if shared else None)
    rec, rec_source = model("PP-OCRv6_tiny_rec_onnx", ("inference.onnx", "inference.yml"),
                            shared["provenance"]["recognizer"] if shared else None)
    detector = write("tiny/det.onnx", det["inference.onnx"])
    recognizer = write("tiny/rec.onnx", rec["inference.onnx"])
    config = write("tiny/rec_inference.yml", rec["inference.yml"])
    dictionary = write("tiny/dict.txt", recognition_dictionary(rec["inference.yml"]))
    print("[3/4] Prepare the exact bilingual image-only scan", flush=True)
    if shared:
        for name in ("fixture.pdf", "fixture.png"):
            data = shared_root.joinpath(name).read_bytes()
            expected = shared["provenance"]["fixtures"][name]
            if expected["sha256"] != sha256(data).hexdigest() or expected["size"] != len(data):
                raise ValueError("Shared test fixture changed.")
            write(name, data)
        if shared["profiles"]["tiny"] != {"detector": detector, "recognizer": recognizer, "dictionary": dictionary}:
            raise ValueError("Test model bindings differ between CPU platforms.")
    else:
        # Pillow and Noto CJK are development-only tools on the generating
        # Linux runner. Other platforms consume the exact original bytes.
        from PIL import Image, ImageDraw, ImageFont
        font_path = Path("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc")
        font = ImageFont.truetype(str(font_path), size=76, index=2)
        with Image.new("RGB", (1600, 480), "white") as image:
            draw = ImageDraw.Draw(image)
            draw.text((80, 65), "NATIVE OCR TEST 12345", fill="black", font=font)
            draw.text((80, 235), "海洋重力波 2026", fill="black", font=font)
            image.save(ROOT.joinpath("fixture.png"), format="PNG")
            image.save(ROOT.joinpath("fixture.pdf"), format="PDF", resolution=144.0)
    fixtures = {}
    for name in ("fixture.pdf", "fixture.png"):
        raw = ROOT.joinpath(name).read_bytes()
        fixtures[name] = {"sha256": sha256(raw).hexdigest(), "size": len(raw)}
    print("[4/4] Save exact test-resource manifest", flush=True)
    package = {
        "schema_version": 1,
        "engine": "ppocr-v6",
        "runtime": runtime,
        "profiles": {"tiny": {"detector": detector, "recognizer": recognizer, "dictionary": dictionary}},
        "provenance": {"runtime": runtime_provenance, "detector": det_source,
                       "recognizer": rec_source, "recognition_config": config, "fixtures": fixtures,
                       "boundary": "CI-resolved test assets only; not a qualified production distribution catalog."},
    }
    raw = (json.dumps(package, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    binding = write("package.json", raw)
    write("package.sha256", (binding["sha256"] + "\n").encode("ascii"))
    print(f"Package SHA-256: {binding['sha256']}", flush=True)


if __name__ == "__main__":
    main()
