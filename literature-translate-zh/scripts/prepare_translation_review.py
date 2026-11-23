"""Prepare pending source-by-source translation records; never judge meaning.

Import these functions from a task-local Python script. The caller selects source
unit IDs after inspecting the source inventory. No selection, empty occurrence
list, glossary match or generated record is a semantic acceptance result.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import sys


Progress = Callable[[int, int, str], None]
CHECKS = (
    "alignment", "ambiguity", "negation", "strength", "number_changes",
    "terminology", "units", "symbols", "formulas", "citation_labels", "completeness",
    "directions", "citation_claims", "table_text", "conditions",
    "antecedents", "modifier_scope",
)
TERM_CATEGORIES = (
    "concept", "method", "estimator", "statistic", "metric", "procedure",
    "model", "parameter", "classification", "abbreviation",
)
OCCURRENCE_KINDS = (
    "claims", "directions", "negations", "conditions", "antecedents",
    "modifiers", "strength", "numbers", "units", "symbols", "formulas",
    "citation_labels", "citation_claims", "table_cells",
)


def progress_bar(done: int, total: int, label: str) -> None:
    width = 24
    filled = width * done // max(total, 1)
    ending = "\n" if done == total else ""
    print(f"\r[{'#' * filled}{'.' * (width - filled)}] {done}/{total} {label}",
          end=ending, file=sys.stderr, flush=True)


def _file_record(path: Path) -> dict:
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError(f"Expected a file: {path}")
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return {"path": str(path), "sha256": digest.hexdigest()}


def _pending() -> dict:
    return {"status": "pending", "reason": None, "evidence": []}


def _index(rows: object, label: str) -> dict:
    if not isinstance(rows, list):
        raise ValueError(f"{label} must be a list")
    result = {}
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError(f"{label} must contain objects")
        identity = row.get("id")
        if not isinstance(identity, str) or not identity.strip() or identity in result:
            raise ValueError(f"{label} requires unique nonempty IDs")
        result[identity] = row
    return result


def new_term_candidate(
    term_id: str,
    original: str,
    category: str,
    *,
    source_excerpt: str,
    source_locator: Mapping,
) -> dict:
    """Create one unresolved lexical candidate from an explicitly read span.

    The caller supplies the phrase and its category. This function does not mine
    terms, search a glossary, choose a translation or assert exhaustive coverage.
    """
    if not all(isinstance(value, str) and value.strip()
               for value in (term_id, original, source_excerpt)):
        raise ValueError("term_id, original and source_excerpt must be nonempty")
    if category not in TERM_CATEGORIES:
        raise ValueError(f"Unknown term category: {category}")
    if not isinstance(source_locator, Mapping) or not source_locator:
        raise ValueError("Supply the actual source occurrence locator")
    return {
        "id": term_id, "original": original, "category": category,
        "source_excerpt": source_excerpt, "source_locator": deepcopy(dict(source_locator)),
        "aliases": [], "translation": None,
        "searches": {
            mode: {**_pending(), "queries": []}
            for mode in ("full_phrase", "aliases", "family")
        },
        "decision": "pending", "decision_reason": None,
        "glossary_matches": [], "terminology_pointer": None,
    }


def prepare_translation_review(
    inventory_path: Path,
    candidate_path: Path,
    output_path: Path,
    *,
    unit_ids: Sequence[str],
    glossary_path: Path | None = None,
    candidate_locations: Mapping[str, list[dict]] | None = None,
    progress: Progress | None = progress_bar,
) -> Path:
    """Create a fresh pending ledger bound to actual source and candidate files.

    Inventory units use id/source/kind/evidence, as in source-inventory.json.
    Explicit unit_ids permit both full and partial tasks; selecting a subset
    never asserts that the remaining source units are outside translation scope.
    Existing output is never overwritten. Revisions use a different output path.
    """
    output_path = Path(output_path).resolve()
    if output_path.exists():
        raise FileExistsError(f"Keep the existing review record: {output_path}")
    if isinstance(unit_ids, (str, bytes)) or not isinstance(unit_ids, Sequence) or not unit_ids:
        raise ValueError("Select an explicit nonempty sequence of source unit IDs")
    if any(not isinstance(identity, str) or not identity.strip() for identity in unit_ids):
        raise ValueError("Every selected unit ID must be nonempty")
    if len(set(unit_ids)) != len(unit_ids):
        raise ValueError("Selected unit IDs must not repeat")
    inventory_path = Path(inventory_path).resolve(strict=True)
    inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
    if not isinstance(inventory, dict):
        raise ValueError("The source inventory must be an object")
    sources = _index(inventory.get("sources"), "sources")
    units = _index(inventory.get("units"), "units")
    unknown = set(unit_ids) - units.keys()
    if unknown:
        raise ValueError(f"Unknown source units: {sorted(unknown)}")
    targets = candidate_locations or {}
    if not isinstance(targets, Mapping) or set(targets) - set(unit_ids):
        raise ValueError("Candidate locations may refer only to selected source units")

    source_bindings = {}
    records = {}
    for index, identity in enumerate(unit_ids, 1):
        unit = units[identity]
        source_id = unit.get("source")
        if not isinstance(source_id, str) or source_id not in sources:
            raise ValueError(f"{identity}: source identity is missing")
        locations = unit.get("evidence")
        if not isinstance(locations, list) or not locations or any(not isinstance(x, dict) or not x for x in locations):
            raise ValueError(f"{identity}: source locations are required")
        for location in locations:
            if type(location.get("page")) is not int or location["page"] < 1:
                raise ValueError(f"{identity}: each source location needs a positive PDF page")
            box = location.get("bbox")
            has_box = (isinstance(box, list) and len(box) == 4
                       and all(type(value) in (int, float) for value in box)
                       and box[0] < box[2] and box[1] < box[3])
            has_quote = isinstance(location.get("quote"), str) and bool(location["quote"].strip())
            if not has_box and not has_quote:
                raise ValueError(f"{identity}: each source location needs a valid bbox or nonempty quote")
        if source_id not in source_bindings:
            source = sources[source_id]
            if not isinstance(source.get("path"), str) or not source.get("sha256"):
                raise ValueError(f"{source_id}: inventory source path/hash is missing")
            source_path = Path(source["path"])
            if not source_path.is_absolute():
                source_path = inventory_path.parent.joinpath(source_path)
            binding = _file_record(source_path)
            if binding["sha256"] != source["sha256"]:
                raise ValueError(f"{source_id}: source bytes differ from the inventory")
            source_bindings[source_id] = binding
        target_locations = targets.get(identity, [])
        if not isinstance(target_locations, list) or any(not isinstance(x, dict) or not x for x in target_locations):
            raise ValueError(f"{identity}: candidate locations must be a list of locator objects")
        records[identity] = {
            "source_id": source_id, "kind": unit.get("kind"),
            "source_locations": deepcopy(locations),
            "candidate_locations": deepcopy(target_locations),
            "checks": {name: _pending() for name in CHECKS},
            "occurrences": {name: [] for name in OCCURRENCE_KINDS},
            "term_scan": {
                category: {**_pending(), "candidate_ids": []}
                for category in TERM_CATEGORIES
            },
            "term_candidates": [],
        }
        if progress is not None:
            progress(index, len(unit_ids), identity)
    payload = {
        "schema_version": 1,
        "evidence_boundary": "Pending scaffold only; neither semantic correctness nor review actions are inferred.",
        "review_status": "pending",
        "bindings": {
            "source_inventory": _file_record(inventory_path),
            "sources": source_bindings,
            "candidate": _file_record(candidate_path),
            "glossary": _file_record(glossary_path) if glossary_path is not None else None,
        },
        "scope": {
            "selected_units": list(unit_ids),
            "unselected_inventory_units": [identity for identity in units if identity not in records],
            "selection_review": _pending(),
            "excluded_units": [],
        },
        "units": records,
        "terminology_record": None,
        "unresolved_terms": [],
        "source_issues": [],
        "source_issues_review": _pending(),
        "events": [],
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation also protects against a competing writer after exists().
    with output_path.open("x", encoding="utf-8") as stream:
        json.dump(payload, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    if json.loads(output_path.read_text(encoding="utf-8")) != payload:
        raise ValueError("Review record did not round-trip")
    return output_path
