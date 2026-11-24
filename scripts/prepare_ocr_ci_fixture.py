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
import tarfile
from urllib.request import Request, urlopen

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(os.environ["Z2O_OCR_FIXTURE_DIR"]).resolve()
ORT_VERSION = "1.28.3"
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
    url = f"https://api.github.com/repos/microsoft/onnxruntime/releases/tags/v{ORT_VERSION}"
    release = metadata(url, github=True)
    if release.get("tag_name") != f"v{ORT_VERSION}" or release.get("draft") or release.get("prerelease"):
        raise ValueError("Use the actual published stable release.")
    name = f"onnxruntime-linux-x64-{ORT_VERSION}.tgz"
    assets = [row for row in release.get("assets", []) if row.get("name") == name]
    if len(assets) != 1:
        raise ValueError(f"Official Linux x64 CPU runtime is unavailable: {name}")
    asset = assets[0]
    digest = str(asset.get("digest", "")).removeprefix("sha256:")
    if not HEX64.fullmatch(digest):
        raise ValueError("Release does not publish an independently verifiable SHA-256.")
    expected_url = f"https://github.com/microsoft/onnxruntime/releases/download/v{ORT_VERSION}/{name}"
    if asset.get("browser_download_url") != expected_url:
        raise ValueError("Unexpected runtime artifact source.")
    archive = download(expected_url)
    if sha256(archive).hexdigest() != digest or len(archive) != asset.get("size"):
        raise ValueError("Runtime archive hash/size mismatch.")
    records = {}
    with tarfile.open(fileobj=BytesIO(archive), mode="r:gz") as tar:
        # Extract only selected regular library files; never follow archive
        # links, create arbitrary member paths, or execute packaged programs.
        runtime_name = f"libonnxruntime.so.{ORT_VERSION}"
        for member in tar.getmembers():
            path = PurePosixPath(member.name)
            if not member.isfile() or len(path.parts) != 3 or path.parts[1] != "lib":
                continue
            if path.name not in {runtime_name, "libonnxruntime_providers_shared.so"}:
                continue
            if path.name in records or member.size > MAX_DOWNLOAD:
                raise ValueError("Duplicate or oversized runtime library.")
            stream = tar.extractfile(member)
            if stream is None:
                raise ValueError("Missing runtime library.")
            records[path.name] = write(f"runtime/{path.name}", stream.read())
    if runtime_name not in records:
        raise ValueError("Release does not contain the expected regular CPU library.")
    return (
        {"platform": "linux", "architecture": "x86_64", "version": ORT_VERSION,
         "file": records[runtime_name],
         "dependencies": [b for n, b in records.items() if n != runtime_name]},
        {"release": release["tag_name"], "archive": expected_url, "archive_sha256": digest,
         "libraries": records},
    )


def model(repo: str, filenames: tuple[str, ...]) -> tuple[dict[str, bytes], dict]:
    identity = f"{HF_ORG}/{repo}"
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
    return result, {"repository": identity, "revision": revision, "files": provenance}


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
    det, det_source = model("PP-OCRv6_tiny_det_onnx", ("inference.onnx",))
    rec, rec_source = model("PP-OCRv6_tiny_rec_onnx", ("inference.onnx", "inference.yml"))
    detector = write("tiny/det.onnx", det["inference.onnx"])
    recognizer = write("tiny/rec.onnx", rec["inference.onnx"])
    config = write("tiny/rec_inference.yml", rec["inference.yml"])
    dictionary = write("tiny/dict.txt", recognition_dictionary(rec["inference.yml"]))
    print("[3/4] Save exact test-resource manifest", flush=True)
    package = {
        "schema_version": 1,
        "engine": "ppocr-v6",
        "runtime": runtime,
        "profiles": {"tiny": {"detector": detector, "recognizer": recognizer, "dictionary": dictionary}},
        "provenance": {"runtime": runtime_provenance, "detector": det_source,
                       "recognizer": rec_source, "recognition_config": config,
                       "boundary": "CI-resolved test assets only; not a qualified production distribution catalog."},
    }
    raw = (json.dumps(package, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    binding = write("package.json", raw)
    write("package.sha256", (binding["sha256"] + "\n").encode("ascii"))
    print(f"Package SHA-256: {binding['sha256']}", flush=True)
    print("[4/4] Generate invented bilingual image-only scan", flush=True)
    font_path = Path("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc")
    font = ImageFont.truetype(str(font_path), size=76, index=2)
    with Image.new("RGB", (1600, 480), "white") as image:
        draw = ImageDraw.Draw(image)
        draw.text((80, 65), "NATIVE OCR TEST 12345", fill="black", font=font)
        draw.text((80, 235), "海洋重力波 2026", fill="black", font=font)
        image.save(ROOT.joinpath("fixture.png"), format="PNG")
        image.save(ROOT.joinpath("fixture.pdf"), format="PDF", resolution=144.0)


if __name__ == "__main__":
    main()
