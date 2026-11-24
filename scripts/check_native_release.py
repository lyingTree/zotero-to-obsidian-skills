"""CI-only release gate, using the real compiled native registry and reference AST."""
from __future__ import annotations

import argparse
import ast
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT.joinpath("native", "migration-status.json")
TARGETS = frozenset(["x86_64-pc-windows-msvc","aarch64-pc-windows-msvc","x86_64-unknown-linux-gnu","aarch64-unknown-linux-gnu","x86_64-apple-darwin","aarch64-apple-darwin"])
CAPABILITIES = frozenset(["all_original_runtime_operations","pdf_source_geometry_and_image_evidence","docx_source_qualification","source_delivery_mapping_and_review_ledger","translation_review_lifecycle","native_complete_workflow","native_installation_and_external_integration","twelve_skill_production_entrypoints","all_reference_regressions_and_frozen_examples","platform_installation_and_release_artifacts","locked_dependencies_and_license_review"])


def reference_inventory(root):
    result = {}
    for path in sorted(root.glob("literature-*/scripts/*.py")):
        if path.stem in result:
            raise ValueError(f"Duplicate reference module {path.stem}")
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        symbols = set()
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and not node.name.startswith("_"):
                symbols.add(f"{path.stem}.{node.name}")
            elif isinstance(node, ast.ClassDef) and not node.name.startswith("_"):
                for method in node.body:
                    if isinstance(method, (ast.FunctionDef, ast.AsyncFunctionDef)) and not method.name.startswith("_"):
                        symbols.add(f"{path.stem}.{node.name}.{method.name}")
        result[path.stem] = symbols
    return result


def evaluate(manifest, description, reference, version, crate_version, lock_committed, ref=""):
    errors, blockers = [], []
    if manifest.get("schema_version") != 1 or manifest.get("release_target") != "2.0.0":
        errors.append("Invalid release policy schema or target.")
    if (description.get("engine") != "rust-native" or
            description.get("system_python_required") is not False or
            description.get("python_fallback") is not False):
        errors.append("The actual binary does not declare a native no-fallback runtime.")
    if description.get("engine_version") != crate_version or description.get("reference_version") != manifest.get("reference_version"):
        errors.append("Binary version differs from source manifest.")
    registered = description.get("operations")
    if not isinstance(registered, list) or any(not isinstance(n, str) for n in registered):
        raise ValueError("Native registry must be a list of operation names.")
    ops = set(registered)
    if len(ops) != len(registered):
        errors.append("Duplicate operation names.")
    modules = manifest.get("module_status", {})
    if set(modules) != set(reference):
        errors.append("Manifest must enumerate exactly the actual original modules.")
    if any(s not in {"not_started", "partial", "implementation_candidate", "qualified"} for s in modules.values()):
        errors.append("Unknown module status.")
    all_symbols = set().union(*reference.values()) if reference else set()
    excluded = manifest.get("nonruntime_api_exclusions", {})
    replacements = manifest.get("api_replacements", {})
    if any(s not in all_symbols or not isinstance(reason, str) or not reason.strip() for s, reason in excluded.items()):
        errors.append("Nonruntime exclusions require an actual reference symbol and explicit reason.")
    if any(s not in all_symbols or op not in ops for s, op in replacements.items()):
        errors.append("Replacement mappings must bind original symbols to actual native operations.")
    if set(excluded) & set(replacements):
        errors.append("Conflicting API exclusion/replacement.")
    coverage = {}
    for module, symbols in sorted(reference.items()):
        missing = sorted(s for s in symbols if s not in ops and s not in excluded and replacements.get(s) not in ops)
        status = modules.get(module, "untracked")
        coverage[module] = {"status": status, "registered_operations": sorted(op for op in ops if op.startswith(module + ".")),
                            "reference_public_symbols": len(symbols), "unmapped_public_symbols": missing}
        if status != "qualified":
            blockers.append(f"Unqualified module: {module} ({status})")
        if missing:
            blockers.append(f"Unmapped original public API: {module} ({len(missing)})")
    capabilities = manifest.get("capabilities", {})
    if set(capabilities) != CAPABILITIES:
        errors.append("Mandatory release capabilities cannot be omitted.")
    for name in sorted(CAPABILITIES):
        cap = capabilities.get(name, {})
        if cap.get("qualified") is not True:
            blockers.append(cap.get("blocker") or f"Unqualified capability: {name}")
    if set(manifest.get("required_targets", [])) != TARGETS:
        errors.append("All six native targets are mandatory.")
    if not lock_committed:
        blockers.append("Cargo.lock is not committed.")
    if version != "2.0.0" or crate_version != "2.0.0":
        blockers.append("Repository VERSION and Cargo version must both equal 2.0.0.")
    if re.match(r"refs/tags/v?2(?:[.-]|$)", ref) and ref != "refs/tags/v2.0.0":
        blockers.append("Release tag differs from v2.0.0.")
    if description.get("full_workflow_ready") is not True:
        blockers.append("Actual binary explicitly reports full_workflow_ready=false.")
    elif blockers or errors:
        errors.append("Binary claims complete readiness despite unmet release gates.")
    return {"schema_version": 1, "release_target": "2.0.0", "version": version,
            "native_version": crate_version, "registered_operation_count": len(ops),
            "reference_module_count": len(reference),
            "qualified_module_count": sum(s == "qualified" for s in modules.values()),
            "required_targets": sorted(TARGETS), "modules": coverage,
            "ready_for_release": not errors and not blockers, "configuration_errors": errors,
            "release_blockers": blockers,
            "boundary": "Registry coverage is not behavioral parity. Full reference tests, six native runtimes, and clean installations remain mandatory."}


def main():
    # Native qualification CI uses Python 3.12. Import locally so the
    # independent policy tests remain compatible with reference Python 3.10.
    import tomllib

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT.joinpath("native", "target", "debug", "z2o.exe" if os.name == "nt" else "z2o"))
    parser.add_argument("--require-release", action="store_true")
    args = parser.parse_args()
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    version = ROOT.joinpath("VERSION").read_text(encoding="utf-8").strip()
    crate = tomllib.loads(ROOT.joinpath("native", "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    proc = subprocess.run([str(args.binary.resolve()), "describe"], capture_output=True, text=True, encoding="utf-8", check=True)
    tracked = subprocess.run(["git", "ls-files", "--error-unmatch", "native/Cargo.lock"], cwd=ROOT,
                             capture_output=True, check=False).returncode == 0
    ref = os.environ.get("GITHUB_REF", "")
    report = evaluate(manifest, json.loads(proc.stdout), reference_inventory(ROOT), version, crate,
                      ROOT.joinpath("native", "Cargo.lock").is_file() and tracked, ref)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    releasing = args.require_release or version.split(".", 1)[0] == "2" or bool(re.match(r"refs/tags/v?2(?:[.-]|$)", ref))
    return int(bool(report["configuration_errors"]) or releasing and not report["ready_for_release"])


if __name__ == "__main__":
    raise SystemExit(main())
