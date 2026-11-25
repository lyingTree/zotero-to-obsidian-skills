"""Build and launch the pinned CPU llama-server on Ubuntu 22.04 ARM64 (CI only)."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess


SOURCE_COMMIT = "5320c87ff0d994845b44ce7976f10055ec697447"
RELEASE = "b11901"
TARGET = "aarch64-unknown-linux-gnu"
OPTIONS = (
    "-DCMAKE_BUILD_TYPE=Release",
    "-DCMAKE_C_COMPILER=gcc-11", "-DCMAKE_CXX_COMPILER=g++-11",
    "-DBUILD_SHARED_LIBS=OFF", "-DGGML_BACKEND_DL=OFF",
    "-DGGML_NATIVE=OFF", "-DGGML_CPU_ARM_ARCH=armv8-a",
    "-DGGML_CPU_KLEIDIAI=OFF", "-DGGML_OPENMP=OFF",
    "-DGGML_CUDA=OFF", "-DGGML_VULKAN=OFF", "-DGGML_RPC=OFF",
    "-DLLAMA_BUILD_COMMON=ON", "-DLLAMA_BUILD_TOOLS=ON",
    "-DLLAMA_BUILD_TESTS=OFF", "-DLLAMA_BUILD_EXAMPLES=OFF",
    "-DLLAMA_BUILD_APP=OFF", "-DLLAMA_BUILD_SERVER=ON",
    "-DLLAMA_BUILD_UI=OFF", "-DLLAMA_USE_PREBUILT_UI=OFF",
    "-DLLAMA_OPENSSL=OFF",
    "-DLLAMA_BUILD_NUMBER=11901", f"-DLLAMA_BUILD_COMMIT={SOURCE_COMMIT[:8]}",
)


def output(args, **kwargs):
    process = subprocess.run(args, check=False, capture_output=True, text=True,
                             encoding="utf-8", errors="replace", **kwargs)
    if process.returncode:
        raise RuntimeError(f"{args[0]} exit {process.returncode}: {process.stderr or process.stdout}")
    return process.stdout, process.stderr


def require_baseline(os_release, machine, glibc):
    if (os_release.get("ID"), os_release.get("VERSION_ID"), machine, glibc) != (
            "ubuntu", "22.04", "aarch64", "glibc 2.35"):
        raise RuntimeError("Expected Ubuntu 22.04 native ARM64 with glibc 2.35; no fallback.")


def inspect_elf(header, versions):
    if re.search(r"(?m)^\s*Machine:\s+AArch64\s*$", header) is None:
        raise RuntimeError("Expected an AArch64 ELF llama-server.")
    required_glibc = sorted(
        {tuple(map(int, value.split("."))) for value in re.findall(r"\bGLIBC_([0-9]+(?:\.[0-9]+)+)\b", versions)}
    )
    if not required_glibc or required_glibc[-1] > (2, 35) or "GLIBC_PRIVATE" in versions:
        raise RuntimeError("AArch64 executable did not meet the glibc 2.35 ABI check.")
    return {
        "glibc": [".".join(map(str, version)) for version in required_glibc],
        "glibcxx": sorted(set(re.findall(r"\bGLIBCXX_([0-9.]+)\b", versions))),
        "cxxabi": sorted(set(re.findall(r"\bCXXABI_([0-9.]+)\b", versions))),
    }


def qualify(source, destination):
    glibc = os.confstr("CS_GNU_LIBC_VERSION")
    require_baseline(platform.freedesktop_os_release(), platform.machine(), glibc)
    revision, _ = output(["git", "-C", str(source), "rev-parse", "HEAD"], timeout=10)
    status, _ = output(["git", "-C", str(source), "status", "--porcelain"], timeout=20)
    if revision.strip() != SOURCE_COMMIT or status.strip():
        raise RuntimeError("Expected clean llama.cpp b11901 source at its pinned commit.")
    destination.mkdir(parents=True, exist_ok=False)
    build = destination / "build"
    subprocess.run(["cmake", "-S", str(source), "-B", str(build), *OPTIONS], check=True)
    subprocess.run(["cmake", "--build", str(build), "--config", "Release", "--target", "llama-server",
                    "--parallel", str(min(os.cpu_count() or 1, 4))], check=True)
    engine = build / "bin" / "llama-server"
    header, _ = output(["readelf", "--file-header", str(engine)], timeout=10)
    versions, _ = output(["readelf", "--wide", "--version-info", str(engine)], timeout=10)
    abi = inspect_elf(header, versions)
    # Must load on the actual baseline with system libraries, not a bundled or
    # newer host libstdc++ override. Only this binary is ever selected here.
    stdout, stderr = output([str(engine), "--version"], cwd=engine.parent,
                            env={"PATH": "", "CUDA_VISIBLE_DEVICES": "", "HIP_VISIBLE_DEVICES": ""},
                            timeout=60)
    if RELEASE[1:] not in stdout + stderr or SOURCE_COMMIT[:8] not in stdout + stderr:
        raise RuntimeError("Engine version output did not identify the pinned source build.")
    cc, _ = output(["gcc-11", "--version"], timeout=10)
    cxx, _ = output(["g++-11", "--version"], timeout=10)
    cmake, _ = output(["cmake", "--version"], timeout=10)
    digest = hashlib.sha256()
    with engine.open("rb") as binary:
        for chunk in iter(lambda: binary.read(1024 * 1024), b""):
            digest.update(chunk)
    evidence = {
        "schema_version": 1, "target": TARGET,
        "origin": "pinned_source_build_not_official_archive",
        "source": {"repository": "ggml-org/llama.cpp", "release": RELEASE, "commit": SOURCE_COMMIT},
        "host": {"distribution": "ubuntu", "version": "22.04", "glibc": glibc},
        "build": {"options": OPTIONS, "gcc": cc.splitlines()[0], "g++": cxx.splitlines()[0],
                  "cmake": cmake.splitlines()[0]},
        "engine": {"variant": "cpu", "executable": "llama-server", "sha256": digest.hexdigest(),
                   "size_bytes": engine.stat().st_size, "required_symbols": abi},
        "version_stdout": stdout, "version_stderr": stderr, "launch_smoke_passed": True,
        "official_archive_qualified_on_this_host": False,
        "inference_qualified": False, "redistribution_qualified": False,
    }
    (destination / "runtime-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print("Verified native Ubuntu 22.04 ARM64 pinned-source launch:", SOURCE_COMMIT, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    qualify(args.source.resolve(), args.destination.resolve())


if __name__ == "__main__":
    main()
