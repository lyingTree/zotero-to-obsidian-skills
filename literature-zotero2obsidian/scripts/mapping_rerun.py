"""Regenerate mappings from unchanged inputs and compare their stable identities.

Use record_mapping_rerun() from a task-local script after the first maps exist.
This writes new evidence files only; it never changes the candidate, prior maps,
source inventory or any acceptance status. Semantic correctness stays unassessed.
"""
from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import importlib.util
import json
from pathlib import Path
import sys


FIELDS = {
    "blocks": ("source_id", "source_attachment", "source_locator", "original_sha256", "decision", "target_units", "target_unit"),
    "units": ("source_id", "source_role", "source_attachment", "source_locator", "registered_unit", "target", "candidate_locator", "resources"),
    "anchors": ("target_id", "target_unit", "source_id", "candidate_locator"),
    "links": ("source_id", "origin_unit", "origin_block", "candidate_locator", "href", "display_text",
              "source_display_text", "syntax", "image", "target_id", "target_unit", "target_path"),
}


def _module(name: str):
    path = Path(__file__).resolve().parent.joinpath(name + ".py")
    key = "literature_rerun_" + hashlib.sha256(str(path).encode()).hexdigest()[:16]
    if key not in sys.modules:
        specification = importlib.util.spec_from_file_location(key, path)
        if specification is None or specification.loader is None:
            raise ImportError(path)
        module = importlib.util.module_from_spec(specification)
        sys.modules[key] = module
        specification.loader.exec_module(module)
    return sys.modules[key]


def _read(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("A mapping document must be a JSON object: " + str(path))
    return value


def _bound_document(path: Path) -> tuple[dict, dict]:
    """Hash exactly the bytes used for the mapping comparison."""
    payload = path.read_bytes()
    value = json.loads(payload.decode("utf-8-sig"))
    if not isinstance(value, dict):
        raise ValueError("A mapping document must be a JSON object: " + str(path))
    return value, {"path": str(path.resolve()), "sha256": hashlib.sha256(payload).hexdigest()}


def _projection(document: dict, name: str) -> dict:
    """Select mapping identities, not subsequently added human review outcomes."""
    kinds = {"blocks": ("block_id", "blocks"), "units": ("unit", "units")}
    if name == "link_map":
        kinds = {"anchors": ("id", "anchors"), **{
            kind: ("id", "links") for kind in ("internal_links", "library_matches", "external_links", "resource_links")}}
    result = {}
    for collection, (identity_field, field_group) in kinds.items():
        rows = document.get(collection, [])
        if isinstance(rows, dict):
            rows = [{identity_field: key, **row} for key, row in rows.items()]
        indexed = {}
        for row in rows:
            identity = row.get(identity_field)
            if not isinstance(identity, str) or not identity or identity in indexed:
                raise ValueError("Each stable mapping needs its unique identity: " + name + "." + collection)
            indexed[identity] = {field: row[field] for field in FIELDS[field_group] if field in row}
        result[collection] = indexed
    return result


def record_mapping_rerun(run_path: Path, deliveries: list[dict], destination: Path, *, progress=None) -> Path:
    """Perform an actual new generation and save the bounded equality result.

    The same source inventory, candidate, source PDFs and explicit delivery
    selectors are used. A different destination is required. Existing human
    review statuses are excluded from the identity comparison and never copied.
    """
    run = _read(Path(run_path))
    records, config = run["records"], run["config"]
    inventory = Path(records["source_inventory"]).resolve(strict=True)
    candidate = Path(config["candidate_path"]).resolve(strict=True)
    baseline = {name: Path(records[name]).resolve(strict=True)
                for name in ("source_map", "si_map", "link_map") if name in records}
    if not {"source_map", "link_map"}.issubset(baseline):
        raise ValueError("Keep the original source and link maps before rerunning.")
    destination = Path(destination).resolve()
    if destination.exists():
        raise FileExistsError("Use a new evidence directory: " + str(destination))
    expected = {source["id"]: {"path": source["path"], "sha256": source["sha256"],
                               "role": run["scope"]["source_roles"][source["id"]]}
                for source in run["identity"]["sources"]}
    maps, checks = _module("handoff_maps"), _module("check_records")
    inputs = {"run": Path(run_path).resolve(), "inventory": inventory, "candidate": candidate,
              **{"source:" + identity: Path(binding["path"]) for identity, binding in expected.items()},
              **{"baseline:" + name: path for name, path in baseline.items()}}
    before = {name: checks.bind_file(path) for name, path in inputs.items()}
    original_documents = {}
    for name, path in baseline.items():
        original_documents[name], bound = _bound_document(path)
        if bound != before["baseline:" + name]:
            raise ValueError("A baseline changed before validation; retain the original snapshot.")
    original_report = maps.validate_handoff_maps(inventory, candidate, baseline,
                                                expected_source_bindings=expected, progress=progress)
    if before != {name: checks.bind_file(path) for name, path in inputs.items()}:
        raise ValueError("An input or baseline changed during baseline validation; do not recapture it as the original.")
    if not original_report.records_ready:
        raise ValueError("Resolve the original mapping diagnostics before measuring stable rerun identities.")
    rerun = {name: destination.joinpath(name.replace("_", "-") + ".json") for name in baseline}
    generated = maps.write_handoff_maps(inventory, candidate, deliveries, rerun,
                                        expected_source_bindings=expected, progress=progress)
    if not generated.records_ready:
        raise ValueError("The new mapping run did not complete: " + json.dumps(generated.as_dict(), ensure_ascii=False))
    if before != {name: checks.bind_file(path) for name, path in inputs.items()}:
        raise ValueError("An input or baseline changed during the mapping rerun; preserve both versions.")
    differences = []
    counts = {}
    rerun_bindings = {}
    for name in baseline:
        rerun_document, rerun_bindings[name] = _bound_document(rerun[name])
        if rerun_bindings[name] != generated.checked_files[name]:
            raise ValueError("A generated map changed before comparison; retain the actual generation record.")
        left, right = _projection(original_documents[name], name), _projection(rerun_document, name)
        counts[name] = {collection: len(rows) for collection, rows in left.items()}
        for collection in left:
            for identity in sorted(left[collection].keys() | right[collection].keys()):
                previous, current = left[collection].get(identity), right[collection].get(identity)
                if previous != current:
                    differences.append({"map": name, "collection": collection, "id": identity,
                                        "previous": previous, "rerun": current})
    if (before != {name: checks.bind_file(path) for name, path in inputs.items()}
            or rerun_bindings != {name: checks.bind_file(path) for name, path in rerun.items()}):
        raise ValueError("A comparison input changed before the report was saved.")
    report = {
        "schema_version": 1, "created_at": datetime.now(timezone.utc).isoformat(),
        "inputs": before, "rerun_maps": rerun_bindings,
        "compared_fields": FIELDS, "baseline_counts": counts,
        "mapping_equal": not differences, "differences": differences,
        "source_fidelity": "not_assessed", "acceptance_status_written": False,
        "boundary": "Same-input mapping identity comparison only; no translation, source identity or human observation is certified.",
    }
    path = destination.joinpath("rerun-comparison.json")
    with path.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    return path
