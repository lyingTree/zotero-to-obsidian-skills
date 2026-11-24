"""CI only: actual native MiniCPM-OCR and Jev inference, not server startup.
Resolve official repositories to immutable revisions; verify LFS digests while
streaming weights. The installed runtime and model processes do not use Python.
"""
from pathlib import Path
from urllib.request import Request, urlopen
import hashlib
import json
import os
import re
import subprocess

ROOT = Path(os.environ["Z2O_LLAMA_FIXTURE_DIR"]).resolve()
SHARED = Path(os.environ["Z2O_LLAMA_SHARED_SCAN"]).resolve()
BINARY = Path(os.environ["Z2O_LLAMA_NATIVE_BINARY"]).resolve()
TARGET = os.environ["Z2O_LLAMA_TARGET"]
UPSTREAM = "c21a3ca524263ef8166cb6d350864b10c3056b61"
EXAMPLES = {
    "01_text_putnam": ["a2d9ef67235ddca2052855fa10260abbb1e55966", "b4d8adb328fe762585982ab6c35747d12371a83a", "e4ac67753d14c797b7515861acce761bb2c77e9d"],
    "02_formula_putnam": ["6f3b4b1881120d161ef40b0901b00a12b2c07b89", "552a8032e63594c9d2ab77ddf2660899a3720ea6", "dd1536bf01b7bca6ed0bfd0e0f7e6fbe7787c689"],
    "03_table_gb_standard": ["6aa872a0062edfeeda48d62fe6b4a8708b62c51b", "185be55d731c4057296025381f217e22f6129f06", "c9874c5e76dafa156136d1bcf5fb36149d31803b"],
}

def require(ok, reason):
    if not ok:
        raise RuntimeError(reason)

def sha(data):
    return hashlib.sha256(data).hexdigest()

def file_sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def get(url, limit):
    with urlopen(Request(url, headers={"User-Agent": "z2o-native-ci"}), timeout=120) as stream:
        result = stream.read(limit + 1)
    require(len(result) <= limit, "Response exceeds budget")
    return result

def weights(repo, names):
    info = json.loads(get(f"https://huggingface.co/api/models/{repo}?blobs=true", 16 * 1024**2))
    revision = info["sha"]
    require(info["id"].casefold() == repo.casefold() and re.fullmatch(r"[0-9a-f]{40}", revision), "Invalid model identity")
    directory = ROOT.joinpath("weights", repo.split("/")[-1])
    directory.mkdir(parents=True, exist_ok=True)
    rows = []
    for name in names:
        entry = next(row for row in info["siblings"] if row["rfilename"] == name)
        lfs = entry["lfs"]
        expected = lfs["sha256"]
        size = lfs["size"]
        require(re.fullmatch(r"[0-9a-f]{64}", expected) and 0 < size <= 8 * 1024**3, "Invalid LFS binding")
        url = f"https://huggingface.co/{repo}/resolve/{revision}/{name}"
        destination = directory.joinpath(name)
        tmp = destination.with_suffix(".part")
        print(f"Downloading {repo}@{revision}/{name} ({size:,} bytes)", flush=True)
        count = 0
        digest = hashlib.sha256()
        with urlopen(Request(url, headers={"User-Agent": "z2o-native-ci"}), timeout=120) as stream, tmp.open("wb") as out:
            while chunk := stream.read(4 * 1024**2):
                count += len(chunk)
                require(count <= size, "Weight exceeds declared length")
                digest.update(chunk)
                out.write(chunk)
        require(count == size and digest.hexdigest() == expected, "Weight hash/size mismatch")
        tmp.replace(destination)
        rows.append({"path": destination.relative_to(ROOT).as_posix(), "sha256": expected,
                     "repository": repo, "revision": revision, "size": size})
    return rows

def make_package(kind, rows, runtime):
    platform = "macos" if "apple" in TARGET else ("windows" if "windows" in TARGET else "linux")
    package = {"schema_version": 1, "engine": "llama.cpp", "release": "b11901", "model_kind": kind,
               "platform": platform, "architecture": TARGET.split("-")[0], "cpu": runtime,
               "gpu": None, "model": rows[0], "projector": rows[1], "draft": None}
    path = ROOT.joinpath(kind + "-package.json")
    path.write_text(json.dumps(package, indent=2) + "\n", encoding="utf-8")
    return path

def invoke(name, image, package, category, **kwargs):
    request = ROOT.joinpath(name + "-request.json")
    operation = "score_jev_region" if "render_image_path" in kwargs else "recognize_minicpm_region"
    request.write_text(json.dumps({"schema_version": 1, "operation": "prepare_source_inventory." + operation,
        "args": [str(image)], "kwargs": {"expected_image_sha256": file_sha(image),
        "package_manifest": str(package), "expected_package_sha256": file_sha(package),
        "category": category, "acceleration": "auto", "timeout_seconds": 600, **kwargs}},
        ensure_ascii=False), encoding="utf-8")
    env = dict(os.environ)
    env.update(PATH="", CUDA_VISIBLE_DEVICES="", HIP_VISIBLE_DEVICES="")
    process = subprocess.run([str(BINARY), "invoke", str(request)], env=env, capture_output=True, timeout=1500)
    ROOT.joinpath(name + "-stderr.txt").write_bytes(process.stderr)
    ROOT.joinpath(name + "-reply.json").write_bytes(process.stdout)
    require(process.returncode == 0, f"{name}: native operation failed: {process.stderr.decode(errors='replace')[-2000:]}")
    result = json.loads(process.stdout)["result"]
    require(result["acceptance_ready"] is False and result["execution_mode"] == "cpu" and result["fallback_used"] is True,
            f"{name}: CPU fallback not demonstrated: {result}")
    return result

def example(name):
    dest = ROOT.joinpath("examples", name)
    dest.mkdir(parents=True, exist_ok=True)
    for filename, expected in zip(("block.png", "case.json", "reference_render.png"), EXAMPLES[name]):
        url = f"https://raw.githubusercontent.com/OpenBMB/MiniCPM-OCR/{UPSTREAM}/scripts/minicpm-ocr-jev/examples/{name}/{filename}"
        data = get(url, 4 * 1024**2)
        require(hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest() == expected, "Example blob mismatch")
        dest.joinpath(filename).write_bytes(data)
    case = json.loads(dest.joinpath("case.json").read_text(encoding="utf-8"))
    require(case["reference_render_of"] == "ocr_output", "Render does not bind expected candidate")
    candidate = next(row for row in case["candidates"] if row["role"] == "ocr_output")
    path = dest.joinpath("candidate.txt")
    path.write_bytes(candidate["text"].encode("utf-8"))
    return case, candidate, path, dest

def main():
    runtime = json.loads(ROOT.joinpath("runtime-evidence.json").read_text(encoding="utf-8"))
    require(runtime["target"] == TARGET, "Runtime target mismatch")
    manifest = SHARED.joinpath("package.json")
    require(file_sha(manifest) == SHARED.joinpath("package.sha256").read_text(encoding="ascii").strip(), "Shared scan manifest mismatch")
    scan = SHARED.joinpath("fixture.png")
    require(file_sha(scan) == json.loads(manifest.read_text(encoding="utf-8"))["provenance"]["fixtures"]["fixture.png"]["sha256"], "Shared scan hash mismatch")
    ocr = make_package("minicpm-ocr", weights("openbmb/MiniCPM-OCR-1.0-gguf",
        ["MiniCPM-OCR-1_0-Q8_0.gguf", "mmproj-model-f16-4x.gguf"]), runtime["engine"])
    result = invoke("minicpm-bilingual", scan, ocr, "text")
    require(result["status"] == "minicpm_candidate_review_required", f"MiniCPM inference blocked: {result}")
    normalized = re.sub(r"\s+", "", result["result"]["candidate_text"]).upper()
    require("NATIVEOCRTEST12345" in normalized and "海洋重力波2026" in normalized, f"Bilingual transcription mismatch: {normalized}")
    jev = make_package("jev", weights("openbmb/MiniCPM-OCR-Jev-gguf",
        ["MiniCPM-OCR-Jev-Q4_K_M.gguf", "mmproj-model-f16.gguf"]), runtime["engine"])
    scores = {}
    for name in EXAMPLES:
        case, candidate, candidate_path, dest = example(name)
        rendered = dest.joinpath("reference_render.png")
        result = invoke("jev-" + name, dest.joinpath("block.png"), jev, case["category"],
            render_image_path=str(rendered), expected_render_sha256=file_sha(rendered),
            candidate_text_path=str(candidate_path), expected_candidate_sha256=file_sha(candidate_path))
        require(result["status"] == "jev_score_review_required" and result["render_semantics_verified"] is False, "Invalid Jev evidence")
        zero, one = result["result"]["prob_0"], result["result"]["prob_1"]
        require(0 <= zero <= 1 and 0 <= one <= 1 and abs(zero + one - 1) < 1e-4, "Invalid probability pair")
        scores[case["category"]] = {"prob_1": one, "label": candidate["label"], "case_id": case["case_id"]}
    # Upstream documents the scores as relative, not a universal threshold.
    require(scores["formula"]["prob_1"] > max(scores["text"]["prob_1"], scores["table"]["prob_1"]),
            f"Jev reference discrimination regression: {scores}")
    ROOT.joinpath("model-smoke-result.json").write_text(json.dumps({"target": TARGET,
        "bilingual_cpu_inference": True, "jev_reference_discrimination": True,
        "scores": scores, "gpu_hardware_verified": False, "source_acceptance": False}, indent=2) + "\n", encoding="utf-8")

if __name__ == "__main__":
    main()
