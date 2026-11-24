"""Prepare pending source-by-source translation records; never judge meaning.

Import these functions from a task-local Python script. The caller selects source
unit IDs after inspecting the source inventory. No selection, empty occurrence
list, glossary match or generated record is a semantic acceptance result.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from copy import deepcopy
import hashlib
import importlib.util
import json
import math
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
OCCURRENCE_FIELDS = {
    "claims": ("source_claim", "candidate_claim"),
    "directions": ("source_relation", "candidate_relation"),
    "negations": ("source_scope", "candidate_scope"),
    "conditions": ("source_condition", "candidate_condition"),
    "antecedents": ("source_candidates", "candidate_candidates"),
    "modifiers": ("source_candidates", "candidate_candidates"),
    "strength": ("source_strength", "candidate_strength"),
    "numbers": ("source_expression", "candidate_expression", "difference_type"),
    "units": ("source_expression", "candidate_expression", "source_quantity", "candidate_quantity"),
    "symbols": ("source_expression", "candidate_expression", "source_definition", "candidate_definition"),
    "formulas": ("source_expression", "candidate_expression", "source_structure", "candidate_structure"),
    "citation_labels": ("source_label", "candidate_label", "source_target", "candidate_target"),
    "citation_claims": ("source_claim", "candidate_claim", "source_target", "candidate_target"),
    "table_cells": ("source_cell", "candidate_cell"),
}


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


def bind_review_file(path: Path) -> dict:
    """Return the actual path/SHA-256 pair; do not assign a review status."""
    return _file_record(path)


def bind_review_evidence(path: Path, locator: Mapping) -> dict:
    """Bind real file bytes to an explicitly supplied location, without judging it.

    The validator checks that the location resolves. A binding does not establish
    that the content was read, supports a claim, or warrants a passed status.
    """
    if not isinstance(locator, Mapping) or not locator:
        raise ValueError("Supply a structured evidence locator")
    return {**_file_record(path), "locator": deepcopy(dict(locator))}


def new_occurrence(occurrence_id: str, kind: str, *, source_span: Mapping) -> dict:
    """Prepare one explicitly identified occurrence; all judgments stay pending."""
    if not isinstance(occurrence_id, str) or not occurrence_id.strip():
        raise ValueError("Supply a nonempty occurrence ID")
    if kind not in OCCURRENCE_KINDS:
        raise ValueError(f"Unknown occurrence kind: {kind}")
    if not isinstance(source_span, Mapping) or not source_span:
        raise ValueError("Supply the actual source span")
    return {
        "id": occurrence_id,
        "source_span": deepcopy(dict(source_span)), "candidate_span": None,
        **{name: None for name in OCCURRENCE_FIELDS[kind]}, **_pending(),
    }


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


def _valid_box(box: object) -> bool:
    return (isinstance(box, list) and len(box) == 4
            and all(type(value) in (int, float) and math.isfinite(value) for value in box)
            and box[0] < box[2] and box[1] < box[3])


def _native_tools():
    path = Path(__file__).resolve().parents[2].joinpath(
        "literature-zotero2obsidian", "scripts", "docx_source_inventory.py")
    key = "literature_translation_native_" + hashlib.sha256(str(path).encode()).hexdigest()[:12]
    if key in sys.modules:
        return sys.modules[key]
    spec = importlib.util.spec_from_file_location(key, path)
    if spec is None or spec.loader is None:
        raise ImportError(path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[key] = module
    spec.loader.exec_module(module)
    return module


def _expand_native_locations(unit: Mapping, source: Mapping, locations: list) -> list[dict]:
    native = _native_tools()
    blocks = _index(source.get("native_blocks"), f"{unit.get('id')}: native source blocks")
    expanded = []
    for location in locations:
        errors = native.native_evidence_errors(source, location)
        if errors:
            raise ValueError(f"{unit.get('id')}: " + " ".join(errors))
        result = deepcopy(location)
        result["block_locations"] = [
            {"block_id": key, "basis": "native", "native_locator": deepcopy(blocks[key]["locator"]),
             "quote": blocks[key]["text"]}
            for key in location["blocks"]
        ]
        if not result.get("quote"):
            result["quote"] = native.source_native_text(source, location)
        expanded.append(result)
    return expanded


def expand_source_locations(unit: Mapping, source: Mapping) -> list[dict]:
    """Resolve actual inventory block IDs without changing scope or source bytes.

    Preserve page/basis/blocks. For block references, add exact block_locations;
    where a span has no bbox/quote, derive them solely from those named blocks.
    This does not infer reading order beyond the supplied ID order, paragraph
    completeness, applicability or meaning. The caller must bind the inventory
    and source files; prepare_translation_review does so using actual SHA-256.
    """
    if not isinstance(unit, Mapping) or not isinstance(source, Mapping):
        raise ValueError("Supply inventory unit and source objects")
    identity = unit.get("id")
    if not isinstance(unit.get("source"), str) or unit["source"] != source.get("id"):
        raise ValueError(f"{identity}: unit must belong to the supplied source")
    locations = unit.get("evidence")
    if not isinstance(locations, list) or not locations:
        raise ValueError(f"{identity}: source locations are required")
    if source.get("format") == "docx":
        return _expand_native_locations(unit, source, locations)
    pages = None
    expanded = []
    for location in locations:
        if not isinstance(location, dict) or not location:
            raise ValueError(f"{identity}: source locations must be objects")
        page = location.get("page")
        if type(page) is not int or page < 1:
            raise ValueError(f"{identity}: each source location needs a positive PDF page")
        result = deepcopy(location)
        if "blocks" in location:
            ids = location["blocks"]
            if ids == [] and (_valid_box(location.get("bbox"))
                              or isinstance(location.get("quote"), str) and location["quote"].strip()):
                expanded.append(result)
                continue  # A directly located image/visual span need not name text blocks.
            if (not isinstance(ids, list) or not ids
                    or any(not isinstance(key, str) or not key.strip() for key in ids)
                    or len(ids) != len(set(ids))):
                raise ValueError(f"{identity}: block IDs must be a unique nonempty string list")
            if pages is None:
                raw_pages = source.get("pages")
                if not isinstance(raw_pages, list):
                    raise ValueError(f"{identity}: source.pages is required to resolve block IDs")
                pages = {}
                for row in raw_pages:
                    if not isinstance(row, dict) or type(row.get("page")) is not int or row["page"] < 1 or row["page"] in pages:
                        raise ValueError(f"{identity}: source page records require unique positive page numbers")
                    pages[row["page"]] = row
            if page not in pages:
                raise ValueError(f"{identity}: page {page} has no source block index")
            indexed = _index(pages[page].get("blocks"), f"{identity}: page {page} blocks")
            unknown = set(ids) - indexed.keys()
            if unknown:
                raise ValueError(f"{identity}: unknown blocks on page {page}: {sorted(unknown)}")
            block_locations = []
            for block_id in ids:
                block = indexed[block_id]
                if not _valid_box(block.get("bbox")):
                    raise ValueError(f"{identity}: block {block_id} has no valid original bbox")
                item = {"block_id": block_id, "page": page, "bbox": deepcopy(block["bbox"])}
                if isinstance(block.get("text"), str) and block["text"].strip():
                    item["quote"] = block["text"]
                block_locations.append(item)
            result["block_locations"] = block_locations
            if not _valid_box(result.get("bbox")):
                boxes = [row["bbox"] for row in block_locations]
                result["bbox"] = [min(box[0] for box in boxes), min(box[1] for box in boxes),
                                  max(box[2] for box in boxes), max(box[3] for box in boxes)]
            if not isinstance(result.get("quote"), str) or not result["quote"].strip():
                quotes = [row["quote"] for row in block_locations if "quote" in row]
                if quotes:
                    result["quote"] = "\n".join(quotes)
        has_quote = isinstance(result.get("quote"), str) and bool(result["quote"].strip())
        if not _valid_box(result.get("bbox")) and not has_quote:
            raise ValueError(f"{identity}: each source location needs a valid bbox, nonempty quote, or resolvable blocks")
        expanded.append(result)
    return expanded


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
        locations = expand_source_locations(unit, sources[source_id])
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
