"""Validate translation-review records, never translation or applicability.

Import validate_translation_review(path, inventory_path=None, candidate_path=None)
from a task-local script. No file is changed. The pending scaffold produced by
prepare_translation_review.py is the schema contract, not a completed review.
All status, scope, term and meaning decisions remain supplied by the recorder.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import asdict, dataclass, field
from datetime import datetime
from itertools import accumulate
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
from typing import Any


def _contract():
    path = Path(__file__).resolve().parent.joinpath("prepare_translation_review.py")
    spec = importlib.util.spec_from_file_location("_translation_review_contract", path)
    if spec is None or spec.loader is None:
        raise ImportError(f"Cannot load translation scaffold contract: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CONTRACT = _contract()
CHECKS = CONTRACT.CHECKS
TERM_CATEGORIES = CONTRACT.TERM_CATEGORIES
OCCURRENCE_KINDS = CONTRACT.OCCURRENCE_KINDS
OCCURRENCE_FIELDS = CONTRACT.OCCURRENCE_FIELDS
SEMANTIC_STATUSES = frozenset({"pending", "passed", "failed", "blocked", "not_applicable", "unresolved"})
CHECK_OCCURRENCES = {
    "completeness": "claims", "directions": "directions", "negation": "negations",
    "conditions": "conditions", "antecedents": "antecedents", "modifier_scope": "modifiers",
    "strength": "strength", "number_changes": "numbers", "units": "units",
    "symbols": "symbols", "formulas": "formulas", "citation_labels": "citation_labels",
    "citation_claims": "citation_claims", "table_text": "table_cells",
}
STRUCTURE_FIELDS = ("denominators", "radicals", "exponents", "brackets")
TEXT_FIELDS = frozenset({
    "original", "translation", "context", "source_excerpt", "entry_id", "excerpt",
    "query", "interpretation", "reason", "resolution_basis", "description", "disposition",
    "performed_at", "action", "check", "table_id", "text", "subject", "reference", "relation",
    "source_expression", "candidate_expression", "source_label", "candidate_label",
})
SHA256 = re.compile(r"[0-9a-f]{64}\Z")


@dataclass(frozen=True)
class TranslationReviewFinding:
    code: str
    object_id: str
    message: str
    level: str = "error"

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass
class TranslationReviewReport:
    findings: list[TranslationReviewFinding] = field(default_factory=list)
    inventory_unit_count: int = 0
    selected_unit_count: int = 0
    recorded_unit_count: int = 0
    excluded_unit_count: int = 0
    status_counts: dict[str, int] = field(default_factory=dict)
    pending_ids: list[str] = field(default_factory=list)
    unresolved_ids: list[str] = field(default_factory=list)
    failed_ids: list[str] = field(default_factory=list)
    review_status: str | None = None
    source_fidelity: str = field(default="not_assessed", init=False)
    execution_truth: str = field(default="not_assessed", init=False)
    independent_review_required: bool = field(default=True, init=False)
    evidence_boundary: str = (
        "Only recorded scope coverage, required fields, current file hashes and resolvable "
        "locations are checked. Source PDF quotes, meaning, applicability, term choice, "
        "exhaustiveness within a paragraph and actual review/search actions require independent "
        "source comparison. records_ready never certifies a translation or grants any atomic-ID pass."
    )

    @property
    def records_ready(self) -> bool:
        return (self.review_status == "reviewed" and self.selected_unit_count > 0
                and not any(item.level == "error" for item in self.findings)
                and not self.pending_ids and not self.unresolved_ids and not self.failed_ids)

    def to_dict(self) -> dict:
        return {**asdict(self), "records_ready": self.records_ready}


def _nonempty(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _filled(value: Any) -> bool:
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, dict):
        return bool(value) and all(_filled(item) for item in value.values())
    if isinstance(value, list):
        return bool(value) and all(_filled(item) for item in value)
    return type(value) in (int, float) and math.isfinite(value)


def _integer(value: Any) -> bool:
    return type(value) is int


def _unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def _reject_constant(value):
    raise ValueError(f"Nonfinite JSON number: {value}")


def _pointer(value: Any, pointer: Any) -> Any:
    if not isinstance(pointer, str) or not pointer.startswith("/"):
        raise ValueError("Use a non-root JSON Pointer starting with /.")
    for part in pointer.split("/")[1:]:
        if re.search(r"~(?![01])", part):
            raise ValueError("Invalid JSON Pointer escape.")
        part = part.replace("~1", "/").replace("~0", "~")
        if isinstance(value, dict) and part in value:
            value = value[part]
            continue
        if isinstance(value, list) and re.fullmatch(r"0|[1-9][0-9]*", part) and int(part) < len(value):
            value = value[int(part)]
            continue
        raise ValueError("JSON Pointer does not identify an existing value.")
    return value


class _Files:
    """Cache shared bytes/derived metadata once per read-only validation call."""

    def __init__(self):
        self.hashes: dict[Path, str] = {}
        self.texts: dict[Path, str] = {}
        self.line_offsets: dict[Path, list[int]] = {}
        self.jsons: dict[Path, Any] = {}
        self.pdf_rects: dict[Path, list[tuple]] = {}

    def digest(self, path: Path) -> str:
        if path not in self.hashes:
            digest = hashlib.sha256()
            with path.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
            self.hashes[path] = digest.hexdigest()
        return self.hashes[path]

    def text(self, path: Path) -> str:
        if path not in self.texts:
            self.texts[path] = path.read_bytes().decode("utf-8")
        return self.texts[path]

    def json(self, path: Path) -> Any:
        if path not in self.jsons:
            self.jsons[path] = json.loads(self.text(path), object_pairs_hook=_unique_pairs,
                                         parse_constant=_reject_constant)
        return self.jsons[path]

    def offsets(self, path: Path) -> list[int]:
        if path not in self.line_offsets:
            self.line_offsets[path] = list(accumulate(map(len, self.text(path).splitlines(keepends=True)), initial=0))
        return self.line_offsets[path]

    def pages(self, path: Path) -> list[tuple]:
        if path not in self.pdf_rects:
            import fitz
            with fitz.open(path) as document:
                if not document.is_pdf or document.needs_pass:
                    raise ValueError("A readable, unencrypted PDF is required for this locator.")
                self.pdf_rects[path] = [tuple(page.rect * page.derotation_matrix) for page in document]
        return self.pdf_rects[path]


class _Validator:
    def __init__(self, path: Path, report: TranslationReviewReport):
        self.path, self.base, self.report = path, path.parent, report
        self.files = _Files()
        self.units: dict[str, dict] = {}
        self.sources: dict[str, dict] = {}
        self.source_paths: dict[str, Path] = {}
        self.candidate: Path | None = None
        self.glossary: Path | None = None
        self.terminology: dict = {}
        self.terminology_path: Path | None = None
        self.selected: set[str] = set()
        self.unresolved_candidates: set[tuple[str, str]] = set()
        self.term_mappings: set[tuple[str, str]] = set()
        self.statuses = Counter()

    def finding(self, code: str, object_id: str, message: str, level: str = "error") -> None:
        self.report.findings.append(TranslationReviewFinding(code, object_id, message, level))

    def mapping(self, value: Any, identity: str) -> dict:
        if isinstance(value, dict):
            return value
        self.finding("TRN_OBJECT_REQUIRED", identity, "Record an object with the documented fields.")
        return {}

    def rows(self, value: Any, identity: str) -> list:
        if isinstance(value, list):
            return value
        self.finding("TRN_LIST_REQUIRED", identity, "Record a list; an absent field is not an empty reviewed set.")
        return []

    def ids(self, value: Any, identity: str, *, nonempty: bool = False, unique: bool = True) -> list[str]:
        rows = self.rows(value, identity)
        result = []
        seen = set()
        for index, item in enumerate(rows):
            if not _nonempty(item):
                self.finding("TRN_ID_INVALID", f"{identity}/{index}", "Use a nonempty string ID.")
                continue
            if unique and item in seen:
                self.finding("TRN_ID_DUPLICATE", identity, f"ID occurs more than once: {item}")
                continue
            seen.add(item)
            result.append(item)
        if nonempty and not result:
            self.finding("TRN_IDS_EMPTY", identity, "List the actual nonempty scope explicitly.")
        return result

    def index(self, value: Any, identity: str) -> dict[str, dict]:
        result = {}
        for index, raw in enumerate(self.rows(value, identity)):
            row = self.mapping(raw, f"{identity}/{index}")
            key = row.get("id")
            if not _nonempty(key):
                self.finding("TRN_ID_INVALID", f"{identity}/{index}", "Use a nonempty string id.")
                continue
            if key in result:
                self.finding("TRN_ID_DUPLICATE", identity, f"Duplicate id: {key}")
                continue
            result[key] = row
        return result

    def read_json(self, path: Path, identity: str) -> Any:
        try:
            return self.files.json(path)
        except (OSError, UnicodeError, ValueError, RecursionError) as error:
            self.finding("TRN_JSON_UNREADABLE", identity, f"Cannot read a unique-key UTF-8 JSON file: {error}")
            return None

    def binding(self, value: Any, identity: str, *, base: Path | None = None,
                expected_path: Path | None = None) -> Path | None:
        record = self.mapping(value, identity)
        raw_path = record.get("path")
        if not _nonempty(raw_path):
            self.finding("TRN_FILE_PATH_MISSING", identity, "Record the actual file path.")
            return None
        try:
            path = Path(raw_path)
            path = (path if path.is_absolute() else (base or self.base).joinpath(path)).resolve()
            if not path.is_file():
                raise ValueError(f"Not an existing file: {path}")
            actual = self.files.digest(path)
        except (OSError, ValueError, RuntimeError) as error:
            self.finding("TRN_FILE_UNREADABLE", identity, str(error))
            return None
        expected = record.get("sha256")
        if not isinstance(expected, str) or SHA256.fullmatch(expected) is None:
            self.finding("TRN_HASH_MISSING", identity, "Record the real lowercase SHA-256 digest.")
        elif expected != actual:
            self.finding("TRN_HASH_CHANGED", identity, f"Bound bytes differ from the current file: {path}")
        if expected_path is not None and path != expected_path:
            self.finding("TRN_BINDING_IDENTITY", identity, f"Bind the requested file {expected_path}, not {path}.")
        return path

    def location(self, path: Path | None, value: Any, identity: str) -> tuple[int, int] | None:
        if path is None:
            return None  # The missing file binding already has a finding.
        locator = self.mapping(value, identity)
        if not locator:
            self.finding("TRN_LOCATION_MISSING", identity, "Supply a concrete source/candidate/evidence location.")
            return None
        try:
            kind = locator.get("kind")
            if kind == "json_pointer":
                _pointer(self.files.json(path), locator.get("pointer"))
                return None
            if kind == "image_region":
                from PIL import Image
                with Image.open(path) as image:
                    self.box(locator.get("bbox"), (0, 0, image.width, image.height))
                return None
            if kind == "pdf_pages":
                start, end = locator.get("start"), locator.get("end")
                if not _integer(start) or not _integer(end) or not 1 <= start <= end <= len(self.files.pages(path)):
                    raise ValueError("PDF page range is outside the actual document.")
                return None
            if "page" in locator:
                page = locator["page"]
                rectangles = self.files.pages(path)
                if not _integer(page) or not 1 <= page <= len(rectangles):
                    raise ValueError("Source page must be a positive page in the actual PDF.")
                has_quote = _nonempty(locator.get("quote"))
                if "bbox" in locator:
                    self.box(locator["bbox"], rectangles[page - 1])
                elif not has_quote:
                    raise ValueError("A PDF source span needs a bbox or a nonempty original quote.")
                # Visual PDF words need not match a damaged/absent text layer.
                return None
            if kind not in (None, "lines", "quote"):
                raise ValueError(f"Unsupported locator kind: {kind!r}")
            text = self.files.text(path)
            if "lines" in locator or kind == "lines":
                lines = locator.get("lines") if "lines" in locator else [locator.get("start"), locator.get("end")]
                if not isinstance(lines, list) or len(lines) != 2 or not all(_integer(n) for n in lines):
                    raise ValueError("Use lines: [start, end], with positive integer line numbers.")
                start, end = lines
                offsets = self.files.offsets(path)
                if not 1 <= start <= end < len(offsets):
                    raise ValueError("Line range is outside the actual UTF-8 file.")
                begin, finish = offsets[start - 1], offsets[end]
                finish -= len(text[begin:finish]) - len(text[begin:finish].rstrip("\r\n"))
                if "quote" in locator and (not _nonempty(locator["quote"]) or locator["quote"] not in text[begin:finish]):
                    raise ValueError("The supplied exact quote is absent from its recorded lines.")
                return begin, finish
            quote = locator.get("quote")
            if not _nonempty(quote):
                raise ValueError("Use a line range or a unique exact quote, not an unlocated claim.")
            if text.count(quote) != 1:
                raise ValueError("Quote-only location must occur exactly once; add explicit lines for repeated text.")
            start = text.index(quote)
            return start, start + len(quote)
        except (OSError, UnicodeError, ValueError, RuntimeError, RecursionError) as error:
            self.finding("TRN_LOCATION_INVALID", identity, str(error))
            return None

    @staticmethod
    def box(value: Any, rectangle: tuple) -> None:
        if (not isinstance(value, list) or len(value) != 4
                or not all(type(n) in (int, float) and math.isfinite(n) for n in value)):
            raise ValueError("Use four finite numeric bbox coordinates.")
        left, top, right, bottom = value
        x0, y0, x1, y1 = rectangle
        if not (x0 - 0.01 <= left < right <= x1 + 0.01 and y0 - 0.01 <= top < bottom <= y1 + 0.01):
            raise ValueError("The bbox is outside the source page/image or has reversed bounds.")

    def evidence(self, value: Any, identity: str, *, required: bool = True) -> None:
        rows = self.rows(value, identity)
        if required and not rows:
            self.finding("TRN_EVIDENCE_MISSING", identity, "Attach actual file/hash/location evidence for this recorded conclusion.")
        for index, raw in enumerate(rows):
            where = f"{identity}/{index}"
            item = self.mapping(raw, where)
            path = self.binding(item, where)
            self.location(path, item.get("locator"), where + "/locator")

    def state(self, value: Any, identity: str, *, allowed=SEMANTIC_STATUSES) -> str | None:
        row = self.mapping(value, identity)
        status = row.get("status")
        if not isinstance(status, str) or status not in allowed:
            self.finding("TRN_STATUS_INVALID", identity, f"Use one of: {', '.join(sorted(allowed))}.")
            return None
        self.statuses[status] += 1
        if status == "pending":
            self.report.pending_ids.append(identity)
            self.finding("TRN_PENDING", identity, "This item still requires an explicit source-based decision.", "warning")
            if row.get("evidence"):
                self.evidence(row["evidence"], identity + "/evidence", required=False)
            return status
        if not _nonempty(row.get("reason")):
            self.finding("TRN_REASON_MISSING", identity, "Explain this single conclusion; for not_applicable name the actual condition and scope.")
        self.evidence(row.get("evidence"), identity + "/evidence")
        if status in {"blocked", "unresolved"}:
            self.report.unresolved_ids.append(identity)
            self.finding("TRN_UNRESOLVED", identity, f"Recorder reports {status}; retain the issue until it is resolved.", "warning")
        if status == "failed":
            self.report.failed_ids.append(identity)
            self.finding("TRN_RECORDED_FAILURE", identity, "Recorder reports a failed comparison; no automatic semantic judgment was made.", "warning")
        return status

    def expected_keys(self, value: Any, expected, identity: str) -> dict:
        rows = self.mapping(value, identity)
        for key in expected:
            if key not in rows:
                self.finding("TRN_TARGET_MISSING", f"{identity}/{key}", "Restore this required individual target from the pending scaffold.")
        for key in rows.keys() - set(expected):
            self.finding("TRN_TARGET_UNKNOWN", f"{identity}/{key}", "Unknown target name; do not replace a required target with a spelling variant.")
        return rows

    def field_values(self, row: dict, fields, identity: str) -> None:
        for name in fields:
            value = row.get(name)
            present = bool(value) if name in {"source_structure", "candidate_structure"} and isinstance(value, dict) else _filled(value)
            if name in TEXT_FIELDS:
                present = _nonempty(value)
            if not present:
                self.finding("TRN_FIELD_MISSING", identity + "/" + name, "Record this occurrence's explicit source/candidate comparison field.")

    def check_scope(self, raw: Any, review_units: dict) -> None:
        scope = self.mapping(raw, "scope")
        selected = self.ids(scope.get("selected_units"), "scope/selected_units", nonempty=True)
        self.selected = set(selected)
        self.report.selected_unit_count = len(selected)
        self.state(scope.get("selection_review"), "scope/selection_review")
        for identity in self.selected - self.units.keys():
            self.finding("TRN_UNIT_UNKNOWN", identity, "Selected ID is absent from the bound source inventory.")
        for identity in self.selected - review_units.keys():
            self.finding("TRN_UNIT_MISSING", identity, "Selected source unit has no independent translation-review record.")
        for identity in review_units.keys() - self.selected:
            self.finding("TRN_UNIT_OUTSIDE_SCOPE", identity, "A review unit must be explicitly selected in scope.")
        expected_unselected = self.units.keys() - self.selected
        unselected = set(self.ids(scope.get("unselected_inventory_units"), "scope/unselected_inventory_units"))
        if unselected != expected_unselected:
            self.finding("TRN_UNSELECTED_INVENTORY_MISMATCH", "scope", "Keep an exact list of inventory IDs outside selected_units; do not silently drop them.")
        excluded = self.index(scope.get("excluded_units"), "scope/excluded_units")
        self.report.excluded_unit_count = len(excluded)
        for identity, row in excluded.items():
            if identity not in expected_unselected:
                self.finding("TRN_EXCLUSION_CONFLICT", identity, "An excluded ID must be a known, unselected inventory unit.")
            for field_name in ("applicability_condition", "reason"):
                if not _nonempty(row.get(field_name)):
                    self.finding("TRN_EXCLUSION_REASON", identity, f"Record {field_name} from actual language/task scope; unselected is not an applicability decision.")
            self.evidence(row.get("evidence"), "scope/excluded_units/" + identity + "/evidence")
        for identity in expected_unselected - excluded.keys():
            self.finding("TRN_SCOPE_UNACCOUNTED", identity, "Inventory unit is neither selected nor explicitly excluded with condition and evidence. Review its applicability; do not infer it from kind or keywords.")

    def source_span(self, source_id: str, value: Any, unit: dict, identity: str) -> None:
        source = self.source_paths.get(source_id)
        self.location(source, value, identity)
        if not isinstance(value, dict):
            return
        locations = unit.get("source_locations")
        if not isinstance(locations, list):
            return
        pages = {row.get("page") for row in locations if isinstance(row, dict) and _integer(row.get("page"))}
        if _integer(value.get("page")) and value["page"] not in pages:
            self.finding("TRN_SOURCE_SPAN_OUTSIDE_UNIT", identity, "This occurrence is not on a page belonging to its source unit.")
            return
        same_page = [row for row in locations if isinstance(row, dict) and row.get("page") == value.get("page")]
        if not same_page:
            return
        box = value.get("bbox")
        if isinstance(box, list) and len(box) == 4 and all(type(n) in (int, float) and math.isfinite(n) for n in box):
            parent_boxes = [row["bbox"] for row in same_page
                            if isinstance(row.get("bbox"), list) and len(row["bbox"]) == 4
                            and all(type(n) in (int, float) and math.isfinite(n) for n in row["bbox"])]
            if parent_boxes and not any(a <= box[0] < box[2] <= c and b <= box[1] < box[3] <= d
                                        for a, b, c, d in parent_boxes):
                self.finding("TRN_SOURCE_SPAN_OUTSIDE_UNIT", identity, "Occurrence bbox is outside this source unit's recorded boxes.")
            return
        # This is a recorded-span containment check, not a PDF text-layer/OCR verdict.
        quote = value.get("quote")
        if any(CONTRACT._valid_box(row.get("bbox")) for row in same_page):
            return  # A visible quote can differ from a damaged text layer at the same real bbox.
        parent_quotes = [row["quote"] for row in same_page if _nonempty(row.get("quote"))]
        if _nonempty(quote) and parent_quotes and not any(" ".join(quote.split()) in " ".join(parent.split()) for parent in parent_quotes):
            self.finding("TRN_SOURCE_SPAN_OUTSIDE_UNIT", identity, "Quote-only occurrence is absent from this unit's recorded original quote; use the actual source span or its visible bbox.")

    def candidate_span(self, value: Any, ranges: list[tuple], identity: str) -> None:
        position = self.location(self.candidate, value, identity)
        if position is not None and ranges and not any(a <= position[0] <= position[1] <= b for a, b in ranges):
            self.finding("TRN_CANDIDATE_SPAN_OUTSIDE_UNIT", identity, "Occurrence location must fall inside this unit's recorded candidate locations.")

    def occurrences(self, raw: Any, unit: dict, source_id: str, ranges: list[tuple], identity: str, checks: dict) -> None:
        groups = self.expected_keys(raw, OCCURRENCE_KINDS, identity + "/occurrences")
        all_ids = set()
        for kind in OCCURRENCE_KINDS:
            if kind not in groups:
                continue
            where = f"{identity}/occurrences/{kind}"
            instances = self.index(groups[kind], where)
            for key, row in instances.items():
                loc = f"{where}/{key}"
                if key in all_ids:
                    self.finding("TRN_OCCURRENCE_DUPLICATE", loc, "Use a distinct occurrence ID within this unit, including across categories.")
                all_ids.add(key)
                self.state(row, loc)
                self.source_span(source_id, row.get("source_span"), unit, loc + "/source_span")
                self.candidate_span(row.get("candidate_span"), ranges, loc + "/candidate_span")
                self.field_values(row, OCCURRENCE_FIELDS[kind], loc)
                if kind == "directions":
                    for side in ("source_relation", "candidate_relation"):
                        relation = self.mapping(row.get(side), loc + "/" + side)
                        self.field_values(relation, ("subject", "reference", "relation"), loc + "/" + side)
                if kind in {"antecedents", "modifiers"}:
                    for side in ("source_candidates", "candidate_candidates"):
                        self.ids(row.get(side), loc + "/" + side, nonempty=True)
                if kind == "numbers" and row.get("difference_type") not in ("unchanged", "representation_only", "value_changed", "structure_changed", "unresolved"):
                    self.finding("TRN_NUMBER_DIFFERENCE_TYPE", loc, "Classify the numeric representation/value/structure difference explicitly.")
                if kind == "formulas":
                    for side in ("source_structure", "candidate_structure"):
                        structure = self.expected_keys(row.get(side), STRUCTURE_FIELDS, loc + "/" + side)
                        for component in STRUCTURE_FIELDS:
                            if component in structure:
                                self.ids(structure[component], f"{loc}/{side}/{component}", unique=False)
                if kind == "table_cells":
                    for side in ("source_cell", "candidate_cell"):
                        cell = self.mapping(row.get(side), loc + "/" + side)
                        self.field_values(cell, ("table_id", "text"), loc + "/" + side)
                        if any(not _integer(cell.get(axis)) or cell[axis] < 1 for axis in ("row", "column")):
                            self.finding("TRN_TABLE_CELL_LOCATION", loc + "/" + side, "Record positive row and column numbers for this cell.")
        for check, kind in CHECK_OCCURRENCES.items():
            check_row = checks.get(check)
            if not isinstance(check_row, dict) or not isinstance(groups.get(kind), list):
                continue
            status, rows = check_row.get("status"), groups[kind]
            if status == "passed" and not rows:
                self.finding("TRN_OCCURRENCES_MISSING", f"{identity}/checks/{check}", f"Passed {check} needs independent occurrences.{kind} records; an empty list does not prove coverage.")
            if status == "not_applicable" and rows:
                self.finding("TRN_APPLICABILITY_CONFLICT", f"{identity}/checks/{check}", f"The record declares no applicable {kind} but also registers instances; resolve this contradiction explicitly.")

    def glossary_hit(self, raw: Any, identity: str) -> None:
        hit = self.mapping(raw, identity)
        self.field_values(hit, ("entry_id", "excerpt"), identity)
        location = hit.get("glossary_locator")
        if isinstance(location, dict) and _nonempty(hit.get("excerpt")):
            location = {**location, "quote": hit["excerpt"]}
        self.location(self.glossary, location, identity + "/glossary_locator")

    def term(self, row: dict, term_id: str, unit_id: str, unit: dict, source_id: str) -> None:
        identity = f"units/{unit_id}/term_candidates/{term_id}"
        self.field_values(row, ("original", "source_excerpt"), identity)
        category = row.get("category")
        if not isinstance(category, str) or category not in TERM_CATEGORIES:
            self.finding("TRN_TERM_CATEGORY", identity, "Use a category from the scaffold's TERM_CATEGORIES.")
        self.source_span(source_id, row.get("source_locator"), unit, identity + "/source_locator")
        aliases = self.ids(row.get("aliases"), identity + "/aliases")
        searches = self.expected_keys(row.get("searches"), ("full_phrase", "aliases", "family"), identity + "/searches")
        for mode in ("full_phrase", "aliases", "family"):
            if mode not in searches:
                continue
            where = identity + "/searches/" + mode
            search = self.mapping(searches[mode], where)
            status = self.state(search, where, allowed={"pending", "searched", "not_applicable", "blocked", "unresolved"})
            queries = self.rows(search.get("queries"), where + "/queries")
            required = mode != "aliases" or bool(aliases)
            if required and status == "not_applicable":
                self.finding("TRN_TERM_SEARCH_REQUIRED", where, "Full phrase and family searches always need actual queries; aliases need them when aliases exist.")
            if status == "searched" and not queries:
                self.finding("TRN_TERM_QUERIES_MISSING", where, "Record actual queries, including queries with zero hits.")
            if status == "not_applicable" and queries:
                self.finding("TRN_APPLICABILITY_CONFLICT", where, "A not-applicable search must not also claim performed queries.")
            for number, raw_query in enumerate(queries):
                query_id = f"{where}/queries/{number}"
                query = self.mapping(raw_query, query_id)
                self.field_values(query, ("query", "interpretation"), query_id)
                for hit_index, hit in enumerate(self.rows(query.get("hits"), query_id + "/hits")):
                    self.glossary_hit(hit, f"{query_id}/hits/{hit_index}")
        decision = row.get("decision")
        if not isinstance(decision, str) or decision not in {"pending", "adopted", "unlisted", "unresolved", "not_a_term"}:
            self.finding("TRN_TERM_DECISION", identity, "Record one explicit term decision from the documented states.")
            return
        if decision == "pending":
            self.report.pending_ids.append(identity)
            self.finding("TRN_PENDING", identity, "Term choice remains pending.", "warning")
            return
        if not _nonempty(row.get("decision_reason")):
            self.finding("TRN_TERM_DECISION_REASON", identity, "Explain this term's contextual choice or exclusion.")
        matches = self.rows(row.get("glossary_matches"), identity + "/glossary_matches")
        for index, hit in enumerate(matches):
            self.glossary_hit(hit, f"{identity}/glossary_matches/{index}")
        if decision == "adopted" and not matches:
            self.finding("TRN_GLOSSARY_MATCH_MISSING", identity, "An adopted same-concept entry needs an actual glossary match and location.")
        if decision == "not_a_term":
            return
        if decision == "unresolved":
            self.unresolved_candidates.add((unit_id, term_id))
            self.report.unresolved_ids.append(identity)
            self.finding("TRN_UNRESOLVED", identity, "Term meaning/choice remains explicitly unresolved.", "warning")
        elif not _nonempty(row.get("translation")):
            self.finding("TRN_TERM_TRANSLATION", identity, "Record the actual chosen translation.")
        pointer = row.get("terminology_pointer")
        try:
            target = _pointer(self.terminology, pointer)
        except ValueError as error:
            self.finding("TRN_TERMINOLOGY_POINTER", identity, str(error))
            return
        section = "adopted" if decision == "adopted" else "unlisted"
        if not isinstance(pointer, str) or not re.fullmatch(rf"/{section}/(?:0|[1-9][0-9]*)", pointer):
            self.finding("TRN_TERMINOLOGY_POINTER", identity, f"Point at one actual /{section}/<index> entry, not the whole ledger.")
        else:
            self.term_mappings.add((unit_id, pointer))
        target = self.mapping(target, identity + "/terminology_pointer")
        if target.get("original") != row.get("original") or target.get("translation") != row.get("translation"):
            self.finding("TRN_TERM_MAPPING", identity, "Original and translation must agree with the pointed terminology entry.")
        source_units = target.get("source_units")
        if not isinstance(source_units, list) or unit_id not in source_units:
            self.finding("TRN_TERM_MAPPING", identity, "The terminology entry must include this source unit ID.")
        if decision == "unresolved" and target.get("status") != "unresolved":
            self.finding("TRN_TERM_MAPPING", identity, "Unresolved term must remain unresolved in the terminology ledger.")
        if decision == "unlisted" and target.get("status") != "resolved":
            self.finding("TRN_TERM_MAPPING", identity, "A decided unlisted term needs a resolved entry with its actual basis.")

    def term_scan(self, raw: Any, candidates: dict, identity: str) -> None:
        scans = self.expected_keys(raw, TERM_CATEGORIES, identity + "/term_scan")
        referenced = set()
        for category in TERM_CATEGORIES:
            if category not in scans:
                continue
            where = f"{identity}/term_scan/{category}"
            scan = self.mapping(scans[category], where)
            status = self.state(scan, where, allowed={"pending", "scanned", "not_applicable", "blocked", "unresolved"})
            ids = self.ids(scan.get("candidate_ids"), where + "/candidate_ids")
            referenced.update(ids)
            for term_id in set(ids) - candidates.keys():
                self.finding("TRN_TERM_CANDIDATE_UNKNOWN", where, f"Unknown term candidate ID: {term_id}")
            if status == "scanned" and not ids:
                self.finding("TRN_TERM_SCAN_EMPTY", where, "A scanned category needs its candidates; if the complete block has none, record an evidenced not_applicable condition.")
            if status == "not_applicable" and ids:
                self.finding("TRN_APPLICABILITY_CONFLICT", where, "Category has candidate IDs while claiming no applicable terms.")
        for term_id in candidates.keys() - referenced:
            self.finding("TRN_TERM_CANDIDATE_UNSCANNED", identity + "/term_candidates/" + term_id, "Reference every registered candidate from its actual category scans.")
        for term_id, term in candidates.items():
            category = term.get("category")
            scan = scans.get(category) if isinstance(category, str) else None
            if isinstance(scan, dict) and isinstance(scan.get("candidate_ids"), list) and term_id not in scan["candidate_ids"]:
                self.finding("TRN_TERM_CATEGORY_MAPPING", identity + "/term_candidates/" + term_id, "Candidate is missing from its declared primary category scan.")

    def unit(self, unit_id: str, raw: Any) -> None:
        identity = "units/" + unit_id
        unit = self.mapping(raw, identity)
        original = self.units.get(unit_id)
        if original is None:
            self.finding("TRN_UNIT_UNKNOWN", identity, "Unit ID is absent from the bound inventory.")
            return
        source_id = unit.get("source_id")
        if not isinstance(source_id, str) or source_id != original.get("source"):
            self.finding("TRN_UNIT_SOURCE", identity, "Use the source identity assigned to this inventory unit.")
            source_id = original.get("source") if isinstance(original.get("source"), str) else ""
        if unit.get("kind") != original.get("kind"):
            self.finding("TRN_UNIT_KIND", identity, "Preserve the inventory unit kind.")
        locations = self.rows(unit.get("source_locations"), identity + "/source_locations")
        try:
            expected_locations = CONTRACT.expand_source_locations(original, self.sources.get(source_id, {}))
        except ValueError as error:
            expected_locations = None
            self.finding("TRN_INVENTORY_SOURCE_LOCATION", identity, str(error))
        if not locations or locations != expected_locations:
            self.finding("TRN_SOURCE_LOCATIONS_CHANGED", identity, "Retain all original inventory locations in order, including cross-page spans.")
        for index, location in enumerate(locations):
            self.location(self.source_paths.get(source_id), location, f"{identity}/source_locations/{index}")
        targets = self.rows(unit.get("candidate_locations"), identity + "/candidate_locations")
        if not targets:
            self.finding("TRN_CANDIDATE_LOCATION_MISSING", identity, "Locate this unit's actual complete candidate text; an untranslated reason does not establish a finished translation.")
        ranges = [position for index, location in enumerate(targets)
                  if (position := self.location(self.candidate, location, f"{identity}/candidate_locations/{index}")) is not None]
        checks = self.expected_keys(unit.get("checks"), CHECKS, identity + "/checks")
        for name in CHECKS:
            if name in checks:
                self.state(checks[name], identity + "/checks/" + name)
        self.occurrences(unit.get("occurrences"), unit, source_id, ranges, identity, checks)
        candidates = self.index(unit.get("term_candidates"), identity + "/term_candidates")
        self.term_scan(unit.get("term_scan"), candidates, identity)
        for term_id, term in candidates.items():
            self.term(term, term_id, unit_id, unit, source_id)

    def terminology_record(self, raw: Any) -> None:
        self.terminology_path = self.binding(raw, "terminology_record")
        if self.terminology_path is None:
            return
        self.terminology = self.mapping(self.read_json(self.terminology_path, "terminology_record"), "terminology_record")
        ledger = self.terminology
        if type(ledger.get("schema_version")) is not int or ledger["schema_version"] != 1:
            self.finding("TRN_TERMINOLOGY_VERSION", "terminology_record", "Use terminology schema_version 1.")
        self.binding(ledger.get("glossary"), "terminology_record/glossary", base=self.terminology_path.parent, expected_path=self.glossary)
        if ledger.get("review_status") != "reviewed":
            self.report.pending_ids.append("terminology_record/review_status")
            self.finding("TRN_TERMINOLOGY_PENDING", "terminology_record", "Finish the actual terminology review before declaring its records reviewed.", "warning")
        covered = set(self.ids(ledger.get("covered_units"), "terminology_record/covered_units"))
        if not self.selected <= covered or not covered <= self.units.keys():
            self.finding("TRN_TERMINOLOGY_COVERAGE", "terminology_record/covered_units", "List each selected unit and only actual inventory IDs; a count cannot replace coverage IDs.")
        for category in ("adopted", "unlisted"):
            entries = self.rows(ledger.get(category), "terminology_record/" + category)
            if category == "unlisted" and not entries and not _nonempty(ledger.get("no_unlisted_reason")):
                self.finding("TRN_UNLISTED_EMPTY_REASON", "terminology_record", "Explain the reviewed empty unlisted-term set from the actual searches.")
            for index, raw_entry in enumerate(entries):
                identity = f"terminology_record/{category}/{index}"
                entry = self.mapping(raw_entry, identity)
                self.field_values(entry, ("original", "translation", "context"), identity)
                ids = set(self.ids(entry.get("source_units"), identity + "/source_units", nonempty=True))
                if not ids <= self.units.keys():
                    self.finding("TRN_TERM_MAPPING", identity, "Terminology source_units must exist in the inventory.")
                if category == "adopted":
                    if not _nonempty(entry.get("glossary_locator")) and not isinstance(entry.get("glossary_locator"), dict):
                        self.finding("TRN_GLOSSARY_MATCH_MISSING", identity, "Record the adopted glossary entry's locator.")
                    continue
                self.field_values(entry, ("reason",), identity)
                if entry.get("status") == "resolved":
                    self.field_values(entry, ("resolution_basis",), identity)
                elif entry.get("status") == "unresolved":
                    self.report.unresolved_ids.append(identity)
                    self.finding("TRN_UNRESOLVED", identity, "Unlisted terminology remains unresolved.", "warning")
                else:
                    self.finding("TRN_TERM_DECISION", identity, "Use resolved or unresolved for an unlisted term.")

    def issue_records(self, ledger: dict, review_units: dict) -> None:
        for category in ("adopted", "unlisted"):
            entries = self.terminology.get(category)
            if not isinstance(entries, list):
                continue
            for index, entry in enumerate(entries):
                if not isinstance(entry, dict) or not isinstance(entry.get("source_units"), list):
                    continue
                pointer = f"/{category}/{index}"
                for unit_id in entry["source_units"]:
                    if not isinstance(unit_id, str) or unit_id not in self.selected:
                        continue
                    if (unit_id, pointer) not in self.term_mappings:
                        self.finding("TRN_TERMINOLOGY_CANDIDATE_MISSING", f"{unit_id}:{pointer}", "The terminology ledger claims this selected-unit term, but no per-unit candidate maps back to it.")
        self.state(ledger.get("source_issues_review"), "source_issues_review")
        issues = self.index(ledger.get("source_issues"), "source_issues")
        for issue_id, issue in issues.items():
            identity = "source_issues/" + issue_id
            unit_id = issue.get("unit_id")
            if not isinstance(unit_id, str) or unit_id not in review_units:
                self.finding("TRN_ISSUE_UNIT", identity, "Tie the source issue to an actual selected review unit.")
                continue
            unit = self.mapping(review_units[unit_id], "units/" + unit_id)
            source_id = unit.get("source_id")
            self.source_span(source_id if isinstance(source_id, str) else "", issue.get("source_locator"), unit, identity + "/source_locator")
            self.field_values(issue, ("description", "disposition"), identity)
            self.ids(issue.get("candidate_readings"), identity + "/candidate_readings", nonempty=True)
            self.state(issue, identity)
        registered = set()
        for index, raw in enumerate(self.rows(ledger.get("unresolved_terms"), "unresolved_terms")):
            identity = f"unresolved_terms/{index}"
            row = self.mapping(raw, identity)
            key = (row.get("unit_id"), row.get("candidate_id"))
            if not all(_nonempty(part) for part in key):
                self.finding("TRN_UNRESOLVED_TERM_MAPPING", identity, "Supply unit_id and candidate_id.")
                continue
            if key in registered or key not in self.unresolved_candidates:
                self.finding("TRN_UNRESOLVED_TERM_MAPPING", identity, "Use each actual unresolved term candidate once.")
            registered.add(key)
            if not _nonempty(row.get("reason")):
                self.finding("TRN_REASON_MISSING", identity, "Explain the actual unresolved term question.")
        for key in self.unresolved_candidates - registered:
            self.finding("TRN_UNRESOLVED_TERM_MISSING", "/".join(key), "Register this unresolved candidate in unresolved_terms.")
        for index, raw in enumerate(self.rows(ledger.get("events"), "events")):
            identity = f"events/{index}"
            event = self.mapping(raw, identity)
            ids = set(self.ids(event.get("unit_ids"), identity + "/unit_ids", nonempty=True))
            if not ids <= self.selected:
                self.finding("TRN_EVENT_SCOPE", identity, "Event must name actual selected units.")
            self.field_values(event, ("check", "performed_at", "action"), identity)
            try:
                datetime.fromisoformat(event.get("performed_at", ""))
            except (TypeError, ValueError):
                self.finding("TRN_EVENT_TIME", identity, "Use the actual action's ISO timestamp; a timestamp alone is not process evidence.")
            self.evidence(event.get("trace_evidence", []), identity + "/trace_evidence", required=False)

    def run(self, ledger: dict, inventory_path: Path | None, candidate_path: Path | None) -> None:
        if type(ledger.get("schema_version")) is not int or ledger["schema_version"] != 1:
            self.finding("TRN_SCHEMA_VERSION", "review", "Use the schema_version 1 scaffold contract.")
        status = ledger.get("review_status")
        self.report.review_status = status if isinstance(status, str) else None
        if status == "pending":
            self.report.pending_ids.append("review_status")
            self.finding("TRN_PENDING", "review_status", "The overall review is still pending.", "warning")
        elif status != "reviewed":
            self.finding("TRN_REVIEW_STATUS", "review_status", "Use pending or reviewed; reviewed does not mean all comparisons passed.")
        bindings = self.mapping(ledger.get("bindings"), "bindings")
        bound_inventory = self.binding(bindings.get("source_inventory"), "bindings/source_inventory", expected_path=inventory_path)
        inventory_file = inventory_path or bound_inventory
        inventory = self.mapping(self.read_json(inventory_file, "inventory") if inventory_file else None, "inventory")
        self.units = self.index(inventory.get("units"), "inventory/units")
        self.sources = self.index(inventory.get("sources"), "inventory/sources")
        self.report.inventory_unit_count = len(self.units)
        inventory_base = inventory_file.parent if inventory_file else self.base
        for source_id, source in self.sources.items():
            path = self.binding(source, "inventory/sources/" + source_id, base=inventory_base)
            if path is not None:
                self.source_paths[source_id] = path
        for unit_id, unit in self.units.items():
            source_id = unit.get("source")
            if not isinstance(source_id, str) or source_id not in self.sources:
                self.finding("TRN_UNIT_SOURCE", "inventory/units/" + unit_id, "Inventory unit has no known source identity.")
        self.candidate = self.binding(bindings.get("candidate"), "bindings/candidate", expected_path=candidate_path)
        if candidate_path is not None:
            self.candidate = candidate_path
        self.glossary = self.binding(bindings.get("glossary"), "bindings/glossary")
        if self.glossary is not None:
            try:
                if not self.files.text(self.glossary).strip():
                    raise ValueError("The bound glossary is empty.")
            except (OSError, UnicodeError, ValueError) as error:
                self.finding("TRN_GLOSSARY_UNREADABLE", "bindings/glossary", str(error))
        review_units = self.mapping(ledger.get("units"), "units")
        self.report.recorded_unit_count = len(review_units)
        self.check_scope(ledger.get("scope"), review_units)
        source_bindings = self.mapping(bindings.get("sources"), "bindings/sources")
        expected_sources = {self.units[key].get("source") for key in self.selected & self.units.keys()
                            if isinstance(self.units[key].get("source"), str)}
        for source_id in expected_sources - source_bindings.keys():
            self.finding("TRN_SOURCE_BINDING_MISSING", source_id, "Bind every source used by selected units.")
        for source_id, binding in source_bindings.items():
            if source_id not in self.sources:
                self.finding("TRN_SOURCE_BINDING_UNKNOWN", source_id, "Source binding has no inventory source identity.")
            self.binding(binding, "bindings/sources/" + source_id, expected_path=self.source_paths.get(source_id))
        self.terminology_record(ledger.get("terminology_record"))
        for unit_id, unit in review_units.items():
            self.unit(unit_id, unit)
        self.issue_records(ledger, review_units)
        self.report.status_counts = dict(self.statuses)


def validate_translation_review(path, inventory_path=None, candidate_path=None) -> TranslationReviewReport:
    """Return diagnostics for a real ledger, including missing/malformed inputs.

    Optional paths pin the expected inventory/candidate identity; they do not
    replace stale hashes. records_ready is false for errors or recorder-declared
    pending/failed/blocked/unresolved items. A clean report never certifies
    meaning, applicability, completeness within a source paragraph, or actions.
    """
    report = TranslationReviewReport()
    try:
        path = Path(path).resolve()
        inventory_path = Path(inventory_path).resolve() if inventory_path is not None else None
        candidate_path = Path(candidate_path).resolve() if candidate_path is not None else None
    except (TypeError, ValueError, OSError, RuntimeError) as error:
        report.findings.append(TranslationReviewFinding("TRN_PATH_INVALID", "review", str(error)))
        return report
    validator = _Validator(path, report)
    ledger = validator.read_json(path, "review")
    if not isinstance(ledger, dict):
        validator.finding("TRN_REVIEW_SHAPE", "review", "The translation-review root must be an object.")
        return report
    try:
        validator.run(ledger, inventory_path, candidate_path)
    except RecursionError:
        validator.finding("TRN_STRUCTURE_DEPTH", "review", "Record nesting is too deep to validate; use the documented flat per-unit structure.")
    return report
