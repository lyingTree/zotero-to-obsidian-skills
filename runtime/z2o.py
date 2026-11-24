"""Bundled executable: explicit JSON operations and trusted local task scripts."""
from __future__ import annotations
import argparse
import base64
from contextlib import redirect_stdout
from dataclasses import asdict, is_dataclass
import hashlib
import importlib
import inspect
import json
import os
from pathlib import Path
import runpy
import shutil
import sys

PRIMARY = "literature-zotero2obsidian"
ROOT = (Path(sys._MEIPASS).joinpath("z2o_bundle") if getattr(sys, "frozen", False)
        else Path(__file__).resolve().parents[1])
VERSION = ROOT.joinpath("VERSION").read_text(encoding="utf-8").strip()
SCRIPT_DIRS = (ROOT.joinpath(PRIMARY, "scripts"),
               ROOT.joinpath("literature-translate-zh", "scripts"), ROOT,
               ROOT.joinpath("vendor", "zotero-skills", "scripts"))
GROUPS = {
    "note_pipeline": ("build_contract", "prepare_content_job", "process_markdown", "process_content"),
    "render_note": ("content_template", "render_note"),
    "normalize_note": ("normalize_note", "normalize_file"),
    "accept_note": ("accept_note",),
    "validate_note": ("validate_note", "validate_file"),
    "lint_presentation": ("lint_presentation", "lint_file"),
    "prepare_source_inventory": ("prepare_sources", "prepare_figure_reviews", "record_figure_edges"),
    "complete_workflow": ("initialize_workflow", "resume_workflow", "require_completed_records",
                          "pending_check_batch", "pending_check_rows", "record_decision_rows",
                          "record_check_batch", "bind_candidate", "write_manifest", "write_maps", "inspect_workflow"),
    "check_records": ("prepare_check_records", "validate_check_records", "bind_file", "bind_evidence"),
    "validate_delivery": ("validate_delivery", "write_delivery_manifest"),
    "handoff_maps": ("make_source_unit", "record_text_visual_conflict", "validate_handoff_maps", "write_handoff_maps"),
    "draft_queue": ("prepare_draft_queue",),
    "mapping_rerun": ("record_mapping_rerun",),
    "format_helpers": ("build_naming_plan", "validate_naming_plan", "check_local_name_conflicts"),
    "deterministic_stage_checks": (
        "snapshot", "verify_binding", "split_frontmatter", "build_properties", "repair_properties",
        "author_token", "collection_ancestry", "inspect_relative_files", "audit_properties",
        "audit_unchanged_properties", "audit_write_readback", "audit_translation_binding",
        "audit_rich_tables", "record_file_checks", "coverage_catalog",
    ),
    "prepare_translation_review": ("prepare_translation_review", "bind_review_file", "bind_review_evidence",
                                   "new_occurrence", "new_term_candidate"),
    "validate_translation_review": ("validate_translation_review",),
}
OPERATIONS = frozenset(f"{module}.{name}" for module, names in GROUPS.items() for name in names)


def configure_imports():
    # Existing dynamic sibling loaders use __file__ and hash actual .py bytes.
    # Keep original modules as bundled data, not relocated into the PYZ archive.
    for directory in reversed(SCRIPT_DIRS):
        location = str(directory)
        if location not in sys.path:
            sys.path.insert(0, location)


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def _nonfinite(value):
    raise ValueError(f"nonfinite JSON constant: {value}")


def read_json(path: Path):
    result = json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=_pairs,
                        parse_constant=_nonfinite)
    if not isinstance(result, dict):
        raise ValueError("expected a JSON object")
    return result


def decode(value):
    if isinstance(value, list):
        return [decode(item) for item in value]
    if not isinstance(value, dict):
        return value
    if set(value) == {"$path"}:
        if not isinstance(value["$path"], str) or not value["$path"]:
            raise ValueError("$path requires a nonempty string")
        return Path(value["$path"])
    if set(value) == {"$bytes"}:
        return base64.b64decode(value["$bytes"], validate=True)
    return {key: decode(item) for key, item in value.items()}


def encode(value):
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, bytes):
        return {"$bytes": base64.b64encode(value).decode("ascii")}
    for name in ("to_dict", "as_dict"):
        callback = getattr(value, name, None)
        if callable(callback):
            return callback()
    if is_dataclass(value) and not isinstance(value, type):
        return asdict(value)
    raise TypeError(f"unsupported result: {type(value).__name__}")


def json_text(value):
    return json.dumps(value, ensure_ascii=False, allow_nan=False, default=encode, indent=2)


def resolve_operation(operation):
    if not isinstance(operation, str) or operation not in OPERATIONS:
        raise ValueError(f"unsupported operation: {operation!r}")
    module, function = operation.split(".")
    loaded = importlib.import_module(module)
    if not any(Path(loaded.__file__).resolve() == directory.joinpath(module + ".py").resolve()
               for directory in SCRIPT_DIRS[:2]):
        raise RuntimeError(f"unexpected module origin: {loaded.__file__}")
    callback = getattr(loaded, function)
    if not callable(callback):
        raise TypeError(operation)
    return callback


def invoke(request):
    if set(request) - {"schema_version", "operation", "args", "kwargs"}:
        raise ValueError("unknown request field")
    if type(request.get("schema_version")) is not int or request["schema_version"] != 1:
        raise ValueError("schema_version must be 1")
    args, kwargs = request.get("args", []), request.get("kwargs", {})
    if not isinstance(args, list) or not isinstance(kwargs, dict) or "progress" in kwargs:
        raise ValueError("args must be an array; kwargs must be an object without callbacks")
    with redirect_stdout(sys.stderr):
        return resolve_operation(request.get("operation"))(*decode(args), **decode(kwargs))


def describe():
    return {"schema_version": 1, "version": VERSION, "operations": {
        name: str(inspect.signature(resolve_operation(name))) for name in sorted(OPERATIONS)}}


def run_script(path: Path, arguments):
    """Explicit compatibility mode, not a sandbox; only run trusted local scripts."""
    path = path.resolve(strict=True)
    if not path.is_file() or path.suffix.lower() != ".py":
        raise ValueError("run-script needs a local trusted .py file")
    previous_argv, previous_path = sys.argv[:], sys.path[:]
    try:
        sys.argv = [str(path), *arguments]
        sys.path.insert(0, str(path.parent))
        runpy.run_path(str(path), run_name="__main__")
    finally:
        sys.argv = previous_argv
        sys.path[:] = previous_path
    return 0


def sha256(path: Path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def verify_package(source_root: Path):
    installer = importlib.import_module("install")
    source_root = source_root.resolve(strict=True)
    manifest = read_json(source_root.joinpath("runtime_manifest.json"))
    if manifest.get("schema_version") != 1 or manifest.get("version") != VERSION:
        raise ValueError("package version differs from binary")
    if source_root.joinpath("VERSION").read_text(encoding="utf-8").strip() != VERSION:
        raise ValueError("package VERSION differs")
    binary = source_root.joinpath(PRIMARY, "bin", "z2o.exe" if os.name == "nt" else "z2o")
    if getattr(sys, "frozen", False) and not binary.samefile(Path(sys.executable)):
        raise ValueError("run the binary from the unpacked package")
    if sha256(binary) != manifest.get("binary_sha256"):
        raise ValueError("package binary digest mismatch")
    fingerprints = manifest.get("skills")
    if not isinstance(fingerprints, dict) or set(fingerprints) != set(installer.LITERATURE_SKILLS):
        raise ValueError("package must contain all 12 skills")
    for name in installer.LITERATURE_SKILLS:
        if installer.tree_fingerprint(source_root.joinpath(name)) != fingerprints[name]:
            raise ValueError(f"skill digest mismatch: {name}")
    if sha256(source_root.joinpath("dependencies.lock.json")) != manifest.get("dependencies_lock_sha256"):
        raise ValueError("dependency lock digest mismatch")
    vendor = manifest.get("vendor")
    dependencies = installer.load_dependencies(source_root.joinpath("dependencies.lock.json"), {})
    if not isinstance(vendor, dict) or set(vendor) != {item.name for item in dependencies}:
        raise ValueError("package lacks pinned external dependencies")
    for dependency in dependencies:
        record = vendor[dependency.name]
        if (record.get("revision") != dependency.revision or
                installer.tree_fingerprint(source_root.joinpath("vendor", dependency.name)) != record.get("fingerprint")):
            raise ValueError(f"external dependency digest or revision mismatch: {dependency.name}")
    return manifest


def install_package(source_root, skills_root):
    installer = importlib.import_module("install")
    if source_root is None:
        if not getattr(sys, "frozen", False):
            raise ValueError("install needs a built distribution")
        source_root = Path(sys.executable).resolve().parents[2]
    manifest = verify_package(source_root)
    if skills_root is None:
        setting = os.environ.get("Z2O_SKILLS_ROOT")
        if not setting:
            raise ValueError("provide --skills-root or Z2O_SKILLS_ROOT explicitly")
        skills_root = Path(setting)
    def fetch_local(dependency, destination, _git_executable):
        record = manifest["vendor"][dependency.name]
        if dependency.requested_ref != dependency.revision or record["revision"] != dependency.revision:
            raise ValueError("binary package supports only the original pinned dependency")
        source = source_root.joinpath("vendor", dependency.name)
        if installer.tree_fingerprint(source) != record["fingerprint"]:
            raise ValueError("pinned dependency changed before copy")
        shutil.copytree(source, destination)
        if installer.tree_fingerprint(destination) != record["fingerprint"]:
            raise ValueError("pinned dependency copy differs")
        return dependency.revision

    # Use pinned build-time snapshots; no Git, pip, or Python discovery.
    result = installer.install(installer.InstallConfig(
        source_root=source_root, skills_root=skills_root, install_python_dependencies=False),
        fetcher=fetch_local)
    print(json_text({"installed": list(result.installed), "skipped": list(result.skipped),
                     "backup_root": result.backup_root, "dependency_revisions": result.dependency_revisions,
                     "runtime": "bundled", "system_python_required": False}))
    return 0


def main(argv=None):
    configure_imports()
    # Machine-readable output is UTF-8 even on a Windows console/pipe.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="backslashreplace")
    parser = argparse.ArgumentParser(prog="z2o")
    parser.add_argument("--version", action="version", version=VERSION)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("describe")
    request = commands.add_parser("invoke")
    request.add_argument("request", type=Path)
    script = commands.add_parser("run-script")
    script.add_argument("script", type=Path)
    script.add_argument("arguments", nargs=argparse.REMAINDER)
    install = commands.add_parser("install")
    install.add_argument("--source-root", type=Path)
    install.add_argument("--skills-root", type=Path)
    verify = commands.add_parser("verify-package")
    verify.add_argument("source_root", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "describe":
            print(json_text(describe()))
        elif args.command == "invoke":
            print(json_text({"schema_version": 1, "result": invoke(read_json(args.request))}))
        elif args.command == "run-script":
            return run_script(args.script, args.arguments)
        elif args.command == "install":
            return install_package(args.source_root, args.skills_root)
        elif args.command == "verify-package":
            print(json_text(verify_package(args.source_root)))
        return 0
    except Exception as error:
        print(json_text({"schema_version": 1, "error": type(error).__name__, "message": str(error)}),
              file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
