"""Developer-only build; users execute z2o without installing Python or pip."""
from __future__ import annotations
import ast
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import zipfile
import PyInstaller.__main__

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from install import LITERATURE_SKILLS, tree_fingerprint, load_dependencies, fetch_repository, _copy_source

BUILD = ROOT.joinpath("build", "z2o-binary")
DIST = ROOT.joinpath("dist", "z2o-binary")
PRIMARY = "literature-zotero2obsidian"
LIBRARIES = ("yaml", "fitz", "pymupdf", "PIL", "pypdf", "pyzotero")


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def copy_bundle(destination):
    destination.mkdir(parents=True)
    for name in (*LITERATURE_SKILLS, "template"):
        shutil.copytree(ROOT.joinpath(name), destination.joinpath(name),
                        ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "*.pyo", "bin"))
    for name in ("VERSION", "dependencies.lock.json", "install.py", "README.md"):
        shutil.copy2(ROOT.joinpath(name), destination.joinpath(name))
    for path in (ROOT.joinpath("runtime", "z2o.py"), ROOT.joinpath("scripts", "build_binary.py"),
                 ROOT.joinpath("scripts", "smoke_binary.py"), ROOT.joinpath("requirements-binary.txt")):
        target = destination.joinpath(*path.relative_to(ROOT).parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)


def prepare_vendor(bundle):
    vendor = {}
    for dependency in load_dependencies(bundle.joinpath("dependencies.lock.json"), {}):
        checkout = BUILD.joinpath("upstream", dependency.name)
        checkout.parent.mkdir(exist_ok=True)
        actual = fetch_repository(dependency, checkout, "git")
        target = bundle.joinpath("vendor", dependency.name)
        for relative in (dependency.skill_path, *(source for source, _ in dependency.extra_paths)):
            _copy_source(checkout.joinpath(*relative.parts), target.joinpath(*relative.parts))
        vendor[dependency.name] = {"revision": actual, "fingerprint": tree_fingerprint(target)}
    return vendor


def hidden_imports(bundle):
    files = sorted(path for directory in (*LITERATURE_SKILLS, "vendor")
                   for path in bundle.joinpath(directory).rglob("*.py"))
    files.append(bundle.joinpath("install.py"))
    local = {path.stem for path in files}
    names = {"argparse", "runpy", "urllib.request", "urllib.error", "urllib.parse",
             "ssl", "http.client", "warnings", "stat", "tempfile", "time",
             "base64", "shutil", "subprocess", "importlib.util"}
    for source in files:
        for node in ast.walk(ast.parse(source.read_text(encoding="utf-8-sig"))):
            if isinstance(node, ast.Import):
                names.update(alias.name for alias in node.names)
            elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
                if node.module.split(".")[0] in local:
                    continue
                names.add(node.module)
                for alias in node.names:
                    if alias.name == "*":
                        continue
                    module = node.module + "." + alias.name
                    try:
                        if importlib.util.find_spec(module) is not None:
                            names.add(module)
                    except (ImportError, AttributeError, ValueError):
                        pass
    return sorted(name for name in names if name.split(".")[0] not in local and name != "__future__")


def archive(package, destination):
    if os.name == "nt":
        output = destination.joinpath(package.name + ".zip")
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as stream:
            for item in sorted(package.rglob("*")):
                # Include directories as their presence participates in fingerprints.
                stream.write(item, item.relative_to(package.parent).as_posix())
    else:
        output = destination.joinpath(package.name + ".tar.gz")
        with tarfile.open(output, "w:gz") as stream:
            stream.add(package, arcname=package.name)
    output.with_name(output.name + ".sha256").write_text(
        f"{sha256(output)}  {output.name}\n", encoding="utf-8")
    return output


def main():
    if BUILD.exists() or DIST.exists():
        raise FileExistsError("Rename generated build/z2o-binary and dist/z2o-binary before rebuilding")
    BUILD.mkdir(parents=True)
    DIST.mkdir(parents=True)
    version = ROOT.joinpath("VERSION").read_text(encoding="utf-8").strip()
    machine = platform.machine().lower()
    os_name = {"win32": "windows", "darwin": "macos", "linux": "linux"}.get(sys.platform, sys.platform)
    arch = {"amd64": "x86_64", "x64": "x86_64", "aarch64": "arm64"}.get(machine, machine)
    package = DIST.joinpath(f"z2o-{version}-{os_name}-{arch}")
    bundle = BUILD.joinpath("z2o_bundle")
    copy_bundle(bundle)
    print("[1/4] Fetching pinned external skills and building standalone runtime", flush=True)
    vendor = prepare_vendor(bundle)
    args = [str(ROOT.joinpath("runtime", "z2o.py")), "--name", "z2o", "--onefile",
            "--clean", "--noconfirm", "--noupx", "--distpath", str(BUILD.joinpath("native")),
            "--workpath", str(BUILD.joinpath("work")), "--specpath", str(BUILD),
            "--add-data", f"{bundle}:z2o_bundle"]
    for dependency in LIBRARIES:
        args.extend(("--collect-all", dependency))
    for module in hidden_imports(bundle):
        args.extend(("--hidden-import", module))
    PyInstaller.__main__.run(args)
    print("[2/4] Assembling distribution", flush=True)
    shutil.copytree(bundle, package)
    launcher = package.joinpath(PRIMARY, "bin", "z2o.exe" if os.name == "nt" else "z2o")
    launcher.parent.mkdir()
    shutil.copy2(BUILD.joinpath("native", launcher.name), launcher)
    if os.name != "nt":
        launcher.chmod(0o755)
    resolved = subprocess.check_output([sys.executable, "-m", "pip", "freeze", "--all"],
                                       text=True, encoding="utf-8").splitlines()
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT,
                                     text=True, encoding="utf-8").strip()
    manifest = {
        "schema_version": 1, "version": version, "commit": commit, "platform": os_name,
        "architecture": arch, "python_build_version": sys.version, "resolved_packages": resolved,
        "binary_sha256": sha256(launcher),
        "dependencies_lock_sha256": sha256(package.joinpath("dependencies.lock.json")),
        "skills": {name: tree_fingerprint(package.joinpath(name)) for name in LITERATURE_SKILLS},
        "vendor": vendor,
    }
    package.joinpath("runtime_manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("[3/4] Verifying package with its own executable", flush=True)
    subprocess.run([str(launcher), "verify-package", str(package)], check=True)
    print("[4/4] Creating archive and SHA-256 sidecar", flush=True)
    print(archive(package, DIST))
    print("Local build only; public redistribution requires license review.")


if __name__ == "__main__":
    main()
