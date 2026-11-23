"""Build and check source/si/link handoff records against explicit file inputs.

Call the functions from a Python script; there is no command-line interface.
These checks concern bindings and recorded relationships, not visual reading,
translation, source completeness, real-library identity, or semantic acceptance.
"""

from __future__ import annotations

from bisect import bisect_left
from collections import Counter, defaultdict
from collections.abc import Callable, Mapping, Sequence
from dataclasses import asdict, dataclass, field
from hashlib import sha256
import importlib.util
import json
import math
from pathlib import Path
import re
import sys
from typing import Any
from urllib.parse import unquote

import yaml


Progress = Callable[[int, int, str], None]
_HASH = re.compile(r"[0-9a-f]{64}\Z")
_ANCHOR_LINE = re.compile(r"(?<!\S)\^([A-Za-z0-9-]+)[ \t]*(?=\r?$)", re.MULTILINE)
_MAP_KEYS = {"source_map", "si_map", "link_map"}
_OBJECT_KINDS = {"figure", "table", "formula", "equation", "video"}
_CALL_KINDS = {"citation", "cross_reference"}
_KINDS = {"metadata", "paragraph", "section", "reference"} | _OBJECT_KINDS | _CALL_KINDS
_LINK_COLLECTIONS = ("internal_links", "library_matches", "external_links", "resource_links")
_BOUNDARY = (
    "Bindings and recorded relationships are checked against the supplied inventory and candidate. "
    "Source PDFs are not reinterpreted here. Generated views are extracted, pending records; "
    "they do not certify observation, semantic correctness, inventory completeness, or real-library matching."
)


def progress_bar(done: int, total: int, label: str) -> None:
    width = 20
    filled = width * done // max(total, 1)
    print(f"\r[{'#' * filled}{'.' * (width - filled)}] {done}/{total} {label}",
          end="\n" if done >= total else "", flush=True)


def _module(name: str):
    path = Path(__file__).resolve().parent.joinpath(name + ".py")
    key = "handoff_" + name + "_" + sha256(str(path).encode()).hexdigest()[:12]
    if key in sys.modules:
        return sys.modules[key]
    spec = importlib.util.spec_from_file_location(key, path)
    if spec is None or spec.loader is None:
        raise ImportError(path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[key] = module
    spec.loader.exec_module(module)
    return module


def _digest(path: Path) -> str:
    digest = sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _text_hash(text: str) -> str:
    return sha256(text.encode("utf-8")).hexdigest()


def _copy(value: Any) -> Any:
    return json.loads(json.dumps(value, ensure_ascii=False, allow_nan=False))


def _nonempty(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


@dataclass(frozen=True)
class HandoffFinding:
    code: str
    object_id: str
    message: str
    level: str = "error"


@dataclass
class HandoffReport:
    findings: list[HandoffFinding] = field(default_factory=list)
    checked_files: dict[str, dict] = field(default_factory=dict)
    written_paths: dict[str, str] = field(default_factory=dict)
    source_fidelity: str = "not_assessed"
    boundary: str = _BOUNDARY

    @property
    def records_ready(self) -> bool:
        return not any(row.level == "error" for row in self.findings)

    def as_dict(self) -> dict:
        return {**asdict(self), "records_ready": self.records_ready}

    def to_dict(self) -> dict:
        return self.as_dict()


@dataclass(frozen=True)
class _Occurrence:
    locator: dict
    href: str
    label: str
    image: bool
    syntax: str
    start: int
    end: int

    @property
    def key(self) -> str:
        return json.dumps(self.locator, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


class _Context:
    def __init__(self, inventory_path: Path, candidate_path: Path, progress: Progress | None):
        self.inventory_path = Path(inventory_path).resolve()
        self.candidate_path = Path(candidate_path).resolve()
        self.progress = progress
        self.report = HandoffReport()
        self.hashes: dict[Path, str] = {}
        self.inventory: dict = {}
        self.sources: dict[str, dict] = {}
        self.source_bindings: dict[str, dict] = {}
        self.units: dict[str, dict] = {}
        self.blocks: dict[str, tuple[str, int, dict, int]] = {}
        self.unit_blocks: dict[str, set[str]] = defaultdict(set)
        self.block_units: dict[str, set[str]] = defaultdict(set)
        self.excluded: dict[str, str] = {}
        self.text = ""
        self.lf_crlf_offsets: list[int] = []
        self.anchor_matches: dict[str, Any] = {}
        self.headings: dict[str, list[tuple[int, int]]] = defaultdict(list)
        self.links: list[_Occurrence] = []
        self.delivery_tools = _module("validate_delivery")
        self.note_tools = self.delivery_tools.note_checks
        self.resolver = self.delivery_tools._Audit(self.candidate_path, None)
        self.target_selectors: dict[str, dict] = {}
        self.targets: dict[str, tuple[int, int, str]] = {}

    def fail(self, code: str, identity: str, message: str, level: str = "error") -> None:
        self.report.findings.append(HandoffFinding(code, str(identity), message, level))

    def notify(self, done: int, total: int, label: str) -> None:
        if self.progress:
            self.progress(done, total, label)

    def actual_binding(self, path: Path, identity: str) -> dict | None:
        try:
            path = path.resolve()
            if path not in self.hashes:
                self.hashes[path] = _digest(path)
            return {"path": str(path), "sha256": self.hashes[path]}
        except (OSError, ValueError) as error:
            self.fail("MAP_FILE_READ", identity, str(error))
            return None

    def binding(self, record: Any, base: Path, identity: str, expected: dict | None = None) -> dict | None:
        if not isinstance(record, Mapping) or not _nonempty(record.get("path")) or not _HASH.fullmatch(str(record.get("sha256", ""))):
            self.fail("MAP_FILE_BINDING", identity, "A real path and lowercase SHA-256 binding are required.")
            return None
        path = Path(record["path"])
        if not path.is_absolute():
            path = base.joinpath(path)
        actual = self.actual_binding(path, identity)
        if actual is None:
            return None
        if actual["sha256"] != record["sha256"]:
            self.fail("MAP_BINDING_HASH", identity, "The bound file bytes changed after recording.")
        if expected is not None and actual != expected:
            self.fail("MAP_BINDING_TARGET", identity, "Bind the exact expected file path and bytes, not another file with similar contents.")
        return actual

    def read(self, path: Path, identity: str) -> dict | None:
        try:
            value = json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=_pairs,
                               parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"Invalid JSON constant: {value}")))
        except (OSError, UnicodeError, ValueError, RecursionError) as error:
            self.fail("MAP_JSON_READ", identity, str(error))
            return None
        if not isinstance(value, dict) or not value:
            self.fail("MAP_JSON_OBJECT", identity, "A nonempty JSON object is required.")
            return None
        if type(value.get("schema_version")) is not int or value["schema_version"] != 1:
            self.fail("MAP_SCHEMA_VERSION", identity, "Use schema_version=1.")
        return value

    def rows(self, value: Any, identity: str, key: str = "id") -> list[dict]:
        if isinstance(value, dict):
            result = []
            for name, row in value.items():
                if not isinstance(row, dict):
                    self.fail("MAP_COLLECTION_ELEMENT", identity + ":" + str(name), "Collection entries must be objects.")
                    continue
                if row.get(key) != name:
                    self.fail("MAP_COLLECTION_KEY", identity + ":" + str(name), f"The mapping key must match the explicit {key}.")
                result.append(row)
            return result
        if not isinstance(value, list):
            self.fail("MAP_COLLECTION", identity, "Use a list of objects or an ID-keyed mapping of objects.")
            return []
        result = []
        for index, row in enumerate(value):
            if not isinstance(row, dict):
                self.fail("MAP_COLLECTION_ELEMENT", f"{identity}[{index}]", "Collection entries must be objects.")
                continue
            result.append(row)
        return result

    def index(self, rows: list[dict], identity: str, key: str = "id") -> dict[str, dict]:
        result = {}
        for row in rows:
            value = row.get(key)
            if not _nonempty(value):
                self.fail("MAP_ID_REQUIRED", identity, f"Each entry needs an explicit nonempty {key}.")
                continue
            if value in result:
                self.fail("MAP_DUPLICATE_ID", value, f"Duplicate {key} in {identity}.")
                continue
            result[value] = row
        return result

    def expected_sources(self, supplied: Any) -> None:
        if supplied is None:
            return
        if isinstance(supplied, Mapping):
            expected = dict(supplied)
        elif isinstance(supplied, list):
            expected = self.index(self.rows(supplied, "expected_source_bindings"), "expected_source_bindings")
        else:
            self.fail("MAP_EXPECTED_SOURCES", "expected_source_bindings", "Use an ID-to-binding mapping or records with explicit id values.")
            return
        if set(expected) != set(self.sources):
            self.fail("MAP_SOURCE_SET", "expected_source_bindings", "The inventory source set differs from the original supplied scope.")
        for identity, binding in expected.items():
            self.binding(binding, self.inventory_path.parent, "expected_source:" + str(identity), self.source_bindings.get(identity))
            if isinstance(binding, Mapping) and "role" in binding and identity in self.sources:
                if binding["role"] != self.sources[identity].get("role"):
                    self.fail("MAP_SOURCE_ROLE", identity, "The source role differs from the original supplied scope.")

    def load_inputs(self, expected_source_bindings: Any) -> None:
        for name, path in (("source_inventory", self.inventory_path), ("candidate", self.candidate_path)):
            binding = self.actual_binding(path, name)
            if binding is not None:
                self.report.checked_files[name] = binding
        inventory = self.read(self.inventory_path, "source_inventory")
        if inventory is None:
            return
        self.inventory = inventory
        self.sources = self.index(self.rows(inventory.get("sources"), "inventory.sources"), "inventory.sources")
        if not self.sources:
            self.fail("MAP_SOURCES_EMPTY", "inventory.sources", "At least one explicitly identified source is required.")
        for index, (identity, source) in enumerate(self.sources.items(), 1):
            if not _nonempty(source.get("role")) or source["role"] not in {"main", "supplement"}:
                self.fail("MAP_SOURCE_ROLE", identity, "Record main or supplement explicitly.")
            binding = self.binding(source, self.inventory_path.parent, identity)
            if binding is not None:
                self.source_bindings[identity] = binding
            pages = self.rows(source.get("pages"), identity + ".pages", "page")
            seen_pages = set()
            order = 0
            for page in pages:
                number = page.get("page")
                if type(number) is not int or number < 1 or number in seen_pages:
                    self.fail("MAP_PAGE_ID", identity, "Source pages need unique positive integer page numbers.")
                    continue
                seen_pages.add(number)
                for block in self.rows(page.get("blocks"), f"{identity}:page-{number}:blocks"):
                    block_id = block.get("id")
                    if not _nonempty(block_id) or block_id in self.blocks:
                        self.fail("MAP_BLOCK_ID", str(block_id), "Source block IDs must be nonempty and globally unique.")
                        continue
                    if not isinstance(block.get("text"), str) or not _bbox(block.get("bbox")):
                        self.fail("MAP_BLOCK_RECORD", block_id, "Source blocks need original text and a finite nonempty bbox.")
                        continue
                    self.blocks[block_id] = (identity, number, block, order)
                    order += 1
            self.notify(index, len(self.sources), "source bindings and block inventory")
        self.expected_sources(expected_source_bindings)
        registered_units = self.index(self.rows(inventory.get("units"), "inventory.units"), "inventory.units")
        for identity, unit in registered_units.items():
            source, kind = unit.get("source"), unit.get("kind")
            if not _nonempty(source) or source not in self.sources:
                self.fail("MAP_UNIT_SOURCE", identity, "The unit points to an unknown source ID.")
                continue
            if not _nonempty(kind) or kind not in _KINDS:
                self.fail("MAP_UNIT_KIND", identity, "The unit needs a supported explicit content kind.")
                continue
            self.units[identity] = unit
        pages_by_source = {identity: {row.get("page") for row in source.get("pages", []) if isinstance(row, dict)}
                           for identity, source in self.sources.items() if isinstance(source.get("pages"), list)}
        for index, (identity, unit) in enumerate(self.units.items(), 1):
            source = unit.get("source")
            evidence = self.rows(unit.get("evidence"), identity + ".evidence")
            if not evidence:
                self.fail("MAP_UNIT_EVIDENCE", identity, "A registered unit needs actual source locations.")
            for location in evidence:
                page = location.get("page")
                if type(page) is not int or page not in pages_by_source.get(source, set()):
                    self.fail("MAP_UNIT_PAGE", identity, "The unit evidence points outside its source pages.")
                if not _bbox(location.get("bbox")) and not _nonempty(location.get("quote")):
                    self.fail("MAP_UNIT_LOCATOR", identity, "A source bbox or explicit source quote is required.")
                references = location.get("blocks", [])
                if not isinstance(references, list) or any(not _nonempty(value) for value in references):
                    self.fail("MAP_BLOCK_REFERENCE", identity, "evidence.blocks must contain explicit block IDs.")
                    continue
                for block_id in references:
                    actual = self.blocks.get(block_id)
                    if actual is None or actual[0] != source or actual[1] != page:
                        self.fail("MAP_BLOCK_REFERENCE", identity, f"Block {block_id!r} does not belong to this source page.")
                        continue
                    self.unit_blocks[identity].add(block_id)
                    self.block_units[block_id].add(identity)
            self.notify(index, len(self.units), "registered source units")
        self.check_unit_references()
        for row in self.rows(inventory.get("excluded_blocks", []), "inventory.excluded_blocks", "block"):
            block_id = row.get("block")
            if not _nonempty(block_id) or block_id not in self.blocks or not _nonempty(row.get("reason")):
                self.fail("MAP_EXCLUSION", str(block_id), "Exclusions need a known block and an explicit reason.")
                continue
            if block_id in self.excluded or self.block_units.get(block_id):
                self.fail("MAP_EXCLUSION_CONFLICT", block_id, "A block is duplicated in exclusions or also used by a source unit.")
            self.excluded[block_id] = row["reason"]
        for row in self.rows(inventory.get("excluded_pages", []), "inventory.excluded_pages", "page"):
            source, page = row.get("source"), row.get("page")
            if not _nonempty(source) or type(page) is not int or page not in pages_by_source.get(source, set()) or not _nonempty(row.get("reason")):
                self.fail("MAP_EXCLUSION", str(source), "Page exclusions need a known source page and an explicit reason.")
                continue
            for block_id, (block_source, number, _block, _order) in self.blocks.items():
                if (source, page) != (block_source, number):
                    continue
                if self.block_units.get(block_id):
                    self.fail("MAP_EXCLUSION_CONFLICT", block_id, "An excluded page also contains a kept registered unit.")
                self.excluded.setdefault(block_id, row["reason"])
        for block_id in self.blocks.keys() - self.block_units.keys() - self.excluded.keys():
            self.fail("MAP_BLOCK_UNACCOUNTED", block_id, "Register this source block in unit evidence or an explicit exclusion.")
        try:
            self.text = self.candidate_path.read_bytes().decode("utf-8-sig")
        except (OSError, UnicodeError) as error:
            self.fail("MAP_CANDIDATE_READ", "candidate", str(error))
            return
        _scan_candidate(self)
        # Delivery selectors share validate_delivery's LF text convention.
        # Keep only the removed CRLF boundaries: mapping a selected boundary
        # back to raw text costs O(log(number of CRLFs)), not a per-character map.
        self.lf_crlf_offsets = [match.start() - index
                                for index, match in enumerate(re.finditer("\r\n", self.text))]
        self.resolver.text = self.text.replace("\r\n", "\n").replace("\r", "\n")
        self.resolver.units = self.units
        self.resolver.visible = self.delivery_tools._visible(self.resolver.text)
        selector_prose = self.delivery_tools._visible(self.resolver.text, keep_tables=False)
        selector_prose = self.note_tools._mask_delimited(selector_prose, "$")
        self.resolver.anchor_matches = {match[1]: match for match in self.delivery_tools._ANCHOR.finditer(selector_prose)}

    def check_unit_references(self) -> None:
        for identity, unit in self.units.items():
            kind = unit.get("kind")
            if kind == "citation":
                refs = unit.get("references")
                if not isinstance(refs, list) or not refs or any(not _nonempty(value) for value in refs):
                    self.fail("MAP_CITATION_REFERENCES", identity, "Each citation needs explicit reference-unit IDs.")
                    continue
                if len(set(refs)) != len(refs):
                    self.fail("MAP_DUPLICATE_REFERENCE", identity, "A citation reference list contains duplicate IDs.")
                for reference in refs:
                    target = self.units.get(reference)
                    if target is None or target.get("kind") != "reference":
                        self.fail("MAP_REFERENCE_UNKNOWN", identity, f"Unknown reference unit: {reference}")
                    elif target.get("source") != unit.get("source") and not _nonempty(unit.get("cross_source_reason")):
                        self.fail("MAP_CROSS_SOURCE_REASON", identity, "A cross-source citation needs its recorded source reason.")
            if kind == "cross_reference":
                key = unit.get("target_unit")
                target = self.units.get(key) if _nonempty(key) else None
                if target is None:
                    self.fail("MAP_TARGET_UNIT_UNKNOWN", identity, "The cross-reference target_unit is not registered.")
            if kind == "reference":
                for name in ("bibliography_id", "original_text"):
                    if not _nonempty(unit.get(name)):
                        self.fail("MAP_REFERENCE_FIELD", identity, f"Retain the source reference {name}.")
            if unit.get("parent_id") is not None:
                parent = self.units.get(unit["parent_id"]) if _nonempty(unit["parent_id"]) else None
                if parent is None or parent.get("source") != unit.get("source"):
                    self.fail("MAP_PARENT_UNIT", identity, "parent_id must identify an object in the same source.")
            if kind == "metadata" and unit.get("field") == "author" and "affiliation_units" in unit:
                affiliations, numbers = unit["affiliation_units"], unit.get("affiliation_numbers")
                if not isinstance(affiliations, list) or any(not _nonempty(value) for value in affiliations):
                    self.fail("MAP_AFFILIATION_REFERENCE", identity, "affiliation_units must list explicit unit IDs.")
                    continue
                if not isinstance(numbers, list) or len(numbers) != len(affiliations):
                    self.fail("MAP_AFFILIATION_REFERENCE", identity, "Affiliation numbers and units must retain their one-to-one correspondence.")
                for affiliation in affiliations:
                    target = self.units.get(affiliation, {})
                    if target.get("source") != unit.get("source") or target.get("kind") != "metadata" or target.get("field") != "affiliation":
                        self.fail("MAP_AFFILIATION_REFERENCE", identity, "The author points to an unknown or different-source affiliation unit.")
        fields = {"title": "title", "authors": "author", "affiliations": "affiliation", "publication_date": "publication_date"}
        for source_id, source in self.sources.items():
            header = source.get("header_fields", {})
            if not isinstance(header, dict):
                self.fail("MAP_HEADER_REFERENCE", source_id, "header_fields must be an object when supplied.")
                continue
            for name, decision in header.items():
                if not isinstance(decision, dict) or "units" not in decision:
                    continue
                ids = decision["units"]
                if not isinstance(ids, list) or any(not _nonempty(value) for value in ids):
                    self.fail("MAP_HEADER_REFERENCE", source_id + ":" + name, "Header decisions must refer to explicit unit IDs.")
                    continue
                for unit_id in ids:
                    target = self.units.get(unit_id, {})
                    if target.get("source") != source_id or target.get("kind") != "metadata" or target.get("field") != fields.get(name):
                        self.fail("MAP_HEADER_REFERENCE", source_id + ":" + name, "The header refers to an unknown or mismatched source metadata unit.")

    def target(self, unit: str, selector: Any) -> tuple[int, int, str] | None:
        if unit not in self.units:
            self.fail("MAP_TARGET_UNIT_UNKNOWN", unit, "A candidate target must identify a registered unit.")
            return None
        if unit in self.target_selectors:
            if self.target_selectors[unit] != selector:
                self.fail("MAP_TARGET_CONFLICT", unit, "The same unit has contradictory candidate selectors.")
            return self.targets.get(unit)
        old_count = len(self.resolver.report.findings)
        span = self.resolver.resolve_target(selector, unit)
        if span is not None:
            start, end, _value = span
            start += bisect_left(self.lf_crlf_offsets, start)
            end += bisect_left(self.lf_crlf_offsets, end)
            span = start, end, self.text[start:end]
        for finding in self.resolver.report.findings[old_count:]:
            self.fail("MAP_" + finding.code, unit, finding.message)
        if isinstance(selector, dict):
            self.target_selectors[unit] = _copy(selector)
        if span is not None:
            self.targets[unit] = span
        return span

    def bindings(self) -> dict:
        return {"source_inventory": self.report.checked_files.get("source_inventory"),
                "candidate": self.report.checked_files.get("candidate"), "sources": _copy(self.source_bindings)}


def _bbox(value: Any) -> bool:
    if not isinstance(value, (list, tuple)) or len(value) != 4:
        return False
    try:
        return (all(type(number) in (int, float) and math.isfinite(number) for number in value) and
                value[0] < value[2] and value[1] < value[3])
    except OverflowError:
        return False


def _label_key(value: str) -> str:
    return " ".join(value.split()).casefold()


def _link_hits(context: _Context, text: str, definitions: dict[str, str]) -> list[tuple]:
    """Locate inline, wiki, reference, automatic and HTML links without I/O."""
    visible = context.note_tools._mask_delimited(context.note_tools._mask_delimited(text, "`"), "$")
    occupied = bytearray(len(visible))
    result = []

    def add(start, end, href, label, image, syntax):
        if start < 0 or end <= start or any(occupied[start:end]):
            return
        occupied[start:end] = b"\1" * (end - start)
        result.append((start, end, href, label, image, syntax))

    for match in re.finditer(r"(?m)^ {0,3}\[([^\]\n]+)\]:[ \t]*(?:<([^>\n]+)>|([^\s]+))[^\n]*$", visible):
        occupied[match.start():match.end()] = b"\1" * (match.end() - match.start())
    for link in context.delivery_tools.scan_markdown_links(visible):
        add(link.start, link.end, link.href, link.label, link.image, "markdown")
    for match in re.finditer(r"!?\[\[([^\]\n|]+)(?:\|([^\]\n]*))?\]\]", visible):
        if context.note_tools._escaped(visible, match.start()):
            continue
        add(match.start(), match.end(), match[1], match[2] if match[2] is not None else match[1],
            match[0].startswith("!"), "wiki")
    stack, brackets = [], {}
    index = 0
    while index < len(visible):
        if visible[index] == "\\":
            index += 2
            continue
        if visible[index] == "[":
            stack.append(index)
        elif visible[index] == "]" and stack:
            brackets[stack.pop()] = index
        index += 1
    for opening, closing in sorted(brackets.items()):
        if occupied[opening] or visible[closing + 1:closing + 2] == "(":
            continue
        display = visible[opening + 1:closing]
        end = closing + 1
        key = display
        if end in brackets:
            label_end = brackets[end]
            key = visible[end + 1:label_end] or display
            end = label_end + 1
        href = definitions.get(_label_key(key))
        if href is None:
            continue
        image = opening > 0 and visible[opening - 1] == "!" and not context.note_tools._escaped(visible, opening - 1)
        add(opening - int(image), end, href, display, image, "reference_markdown")
    html_link = re.compile(r"<a\b[^>]*?\bhref\s*=\s*(['\"])(.*?)\1[^>]*>(.*?)</a\s*>", re.I | re.S)
    for match in html_link.finditer(visible):
        add(match.start(), match.end(), match[2], match[3], False, "html")
    for match in re.finditer(r"<((?:[A-Za-z][A-Za-z0-9+.-]*:[^<>\s]+)|(?:[^<>\s@]+@[^<>\s@]+\.[^<>\s@]+))>", visible):
        href = match[1] if ":" in match[1] else "mailto:" + match[1]
        add(match.start(), match.end(), href, match[1], False, "autolink")
    for match in re.finditer(r"(?:https?|ftp)://[^\s<>\[\]`]+", visible):
        if occupied[match.start()]:
            continue
        raw = match[0].rstrip(".,;:!?")
        while raw.endswith(")") and raw.count(")") > raw.count("("):
            raw = raw[:-1]
        if raw:
            add(match.start(), match.start() + len(raw), raw, raw, False, "bare_url")
    return sorted(result)


def _pointer(path: tuple) -> str:
    return "".join("/" + str(part).replace("~", "~0").replace("/", "~1") for part in path)


def _table_strings(context: _Context, data: dict, identity: str):
    def strings(value, path, active):
        if isinstance(value, str):
            yield path, value
            return
        if not isinstance(value, (dict, list)):
            return
        if id(value) in active:
            context.fail("MAP_TABLE_CYCLE", identity, "Recursive table aliases cannot be enumerated as finite text fields.")
            return
        active = active | {id(value)}
        items = value.items() if isinstance(value, dict) else enumerate(value)
        for key, child in items:
            yield from strings(child, path + (key,), active)

    def sheet(value, prefix):
        for key in ("title", "footer"):
            yield from strings(value.get(key), prefix + (key,), set())
        columns = value.get("columns", [])
        rows = value.get("rows", [])
        if not isinstance(columns, list) or not isinstance(rows, list):
            context.fail("MAP_TABLE_STRUCTURE", identity, "Table rows and columns must be lists.")
            return
        for index, column in enumerate(columns):
            if not isinstance(column, dict):
                context.fail("MAP_TABLE_STRUCTURE", identity, "Each table column must be an object.")
                continue
            yield from strings(column.get("name"), prefix + ("columns", index, "name"), set())
        for index, row in enumerate(rows):
            if not isinstance(row, dict):
                context.fail("MAP_TABLE_STRUCTURE", identity, "Each table row must be an object.")
                continue
            yield from strings(row.get("cells"), prefix + ("rows", index, "cells"), set())

    yield from sheet(data, ())
    if data.get("version") != 3:
        return
    sheets = data.get("sheets")
    if not isinstance(sheets, list) or not sheets:
        context.fail("MAP_TABLE_STRUCTURE", identity, "A v3 workbook requires its actual worksheet list.")
        return
    for index, value in enumerate(sheets):
        if not isinstance(value, dict):
            context.fail("MAP_TABLE_STRUCTURE", identity, "Each worksheet must be an object.")
            continue
        yield from sheet(value, ("sheets", index))


def _scalar_nodes(node, path=(), active=frozenset()):
    if id(node) in active:
        return
    if isinstance(node, yaml.ScalarNode):
        yield _pointer(path), node
        return
    active = active | {id(node)}
    if isinstance(node, yaml.SequenceNode):
        for index, child in enumerate(node.value):
            yield from _scalar_nodes(child, path + (index,), active)
    elif isinstance(node, yaml.MappingNode):
        for key, child in node.value:
            if isinstance(key, yaml.ScalarNode):
                yield from _scalar_nodes(child, path + (key.value,), active)


def _candidate_visible(context: _Context, *, keep_tables: bool = True, keep_table_fences: bool = False) -> str:
    """Mask syntax using an equal-length view, then restore original CRLFs.

    The shared fence recognizer accepts horizontal whitespace before LF. A
    temporary CR-to-space substitution satisfies that grammar without shifting
    any character offset or line. Only this parsing view is changed; source
    text, file bindings, selected snippets and their hashes use original bytes.
    """
    original = context.text
    visible = context.delivery_tools._visible(
        original.replace("\r\n", " \n"), keep_tables=keep_tables, keep_table_fences=keep_table_fences,
    )
    if "\r\n" not in original:
        return visible
    return "\n".join(
        rendered[:-1] + "\r" if raw.endswith("\r") else rendered
        for raw, rendered in zip(original.split("\n"), visible.split("\n"), strict=True)
    )


def _scan_candidate(context: _Context) -> None:
    prose = _candidate_visible(context, keep_tables=False)
    visible = context.note_tools._mask_delimited(prose, "$")
    for match in _ANCHOR_LINE.finditer(visible):
        identity = match[1]
        if identity in context.anchor_matches:
            context.fail("MAP_CANDIDATE_ANCHOR_DUPLICATE", identity, "The candidate defines this anchor more than once.")
            continue
        context.anchor_matches[identity] = match
    lines = context.text.splitlines(keepends=True)
    offsets, cursor = [], 0
    for line in lines:
        offsets.append(cursor)
        cursor += len(line)
    offsets.append(cursor)
    stack = []
    for index, line in enumerate(visible.splitlines(keepends=True)):
        heading = context.note_tools._HEADING.match(line.rstrip("\r\n"))
        if heading is None:
            continue
        level, title = len(heading[1]), heading[2]
        while stack and stack[-1][0] >= level:
            stack.pop()
        stack.append((level, title))
        location = (offsets[index], offsets[index + 1])
        context.headings[title].append(location)
        qualified = "#".join(value for _level, value in stack)
        if qualified != title:
            context.headings[qualified].append(location)
    definitions = {}
    for match in re.finditer(r"(?m)^ {0,3}\[([^\]\n]+)\]:[ \t]*(?:<([^>\n]+)>|([^\s]+))[^\n]*$", prose):
        key = _label_key(match[1])
        if key in definitions:
            context.fail("MAP_LINK_DEFINITION_DUPLICATE", match[1], "Reference-link labels must be unambiguous.")
            continue
        definitions[key] = match[2] or match[3]
    for start, end, href, label, image, syntax in _link_hits(context, prose, definitions):
        locator = {"kind": "characters", "unit": "unicode_code_points", "start": start,
                   "end": end, "sha256": _text_hash(context.text[start:end])}
        context.links.append(_Occurrence(locator, href, label, image, syntax, start, end))
    table_lines = _candidate_visible(context, keep_table_fences=True).splitlines(keepends=True)
    index = 0
    while index < len(table_lines):
        opening = context.delivery_tools._FENCE.match(table_lines[index].rstrip("\r\n"))
        if opening is None or opening[2].strip() != "rich-table":
            index += 1
            continue
        start = index
        closing = re.compile(r"^ {0,3}" + re.escape(opening[1][0]) + "{" + str(len(opening[1])) + r",}[ \t]*$")
        index += 1
        while index < len(table_lines) and not closing.fullmatch(table_lines[index].rstrip("\r\n")):
            index += 1
        if index >= len(table_lines):
            context.fail("MAP_TABLE_FENCE", f"line:{start + 1}", "The Rich Table fence is not closed.")
            break
        finish = index
        payload = context.note_tools._rich_table_payload([line.rstrip("\r\n") for line in table_lines[start + 1:finish]])
        label = f"table:{start + 1}-{finish + 1}"
        try:
            data = yaml.load(payload, Loader=context.note_tools._UniqueKeyLoader)
            nodes = dict(_scalar_nodes(yaml.compose(payload, Loader=yaml.SafeLoader)))
        except (yaml.YAMLError, ValueError, RecursionError) as error:
            context.fail("MAP_TABLE_PARSE", label, str(error))
            index += 1
            continue
        if not isinstance(data, dict) or type(data.get("version")) is not int or data["version"] not in (2, 3):
            context.fail("MAP_TABLE_STRUCTURE", label, "Only an explicit Rich Table v2 or v3 object can be enumerated reliably.")
            index += 1
            continue
        for path, value in _table_strings(context, data, label):
            pointer = _pointer(path)
            node = nodes.get(pointer)
            node_start, node_end = offsets[start], offsets[finish + 1]
            if node is not None:
                node_start = offsets[start + 1 + node.start_mark.line] + node.start_mark.column
                node_end = offsets[start + 1 + node.end_mark.line] + node.end_mark.column
            raw_value = context.text[node_start:node_end]
            hits = _link_hits(context, value, definitions)
            seen_raw: Counter = Counter()
            for first, last, href, display, image, syntax in hits:
                raw = value[first:last]
                positions = [match.start() for match in re.finditer(re.escape(raw), raw_value)]
                occurrence = seen_raw[raw]
                seen_raw[raw] += 1
                position_start, position_end = node_start, node_end
                if occurrence < len(positions):
                    position_start = node_start + positions[occurrence]
                    position_end = position_start + len(raw)
                locator = {"kind": "rich_table_text", "fence_lines": [start + 1, finish + 1],
                           "json_pointer": pointer, "start": first, "end": last,
                           "sha256": _text_hash(raw), "value_sha256": _text_hash(value)}
                context.links.append(_Occurrence(locator, href, display, image, syntax, position_start, position_end))
        index += 1
    context.links.sort(key=lambda row: (row.start, row.end, row.key))
    context.notify(len(context.links), len(context.links), "candidate link occurrences")


def _link_kind(context: _Context, occurrence: _Occurrence) -> tuple[str, str]:
    if occurrence.image:
        return "resource_links", occurrence.href
    href = unquote(occurrence.href)
    if href.startswith("#"):
        return "internal_links", href[1:]
    if re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", href) or href.startswith("//"):
        return "external_links", href
    path, separator, fragment = href.partition("#")
    if separator:
        target = context.candidate_path.parent.joinpath(path).resolve()
        if target == context.candidate_path:
            return "internal_links", fragment
    return "library_matches", href


def _paths(context: _Context, supplied: Any) -> dict[str, Path]:
    if not isinstance(supplied, Mapping):
        context.fail("MAP_PATHS", "map_paths", "Supply an explicit map-name-to-Path mapping.")
        return {}
    unknown = set(supplied) - _MAP_KEYS
    for key in unknown:
        context.fail("MAP_PATH_UNKNOWN", str(key), "Use source_map, si_map or link_map as the record name.")
    required = {"source_map", "link_map"}
    if any(source.get("role") == "supplement" for source in context.sources.values()):
        required.add("si_map")
    for key in required - set(supplied):
        context.fail("MAP_PATH_MISSING", key, "This handoff map is required by the actual source roles.")
    result = {}
    occupied = set()
    for key, value in supplied.items():
        if key not in _MAP_KEYS:
            continue
        if not isinstance(value, (str, Path)) or not str(value):
            context.fail("MAP_PATHS", key, "Provide an explicit file path.")
            continue
        path = Path(value).resolve()
        if path in occupied:
            context.fail("MAP_PATH_COLLISION", key, "Different handoff records must not share one file path.")
        if path in {context.inventory_path, context.candidate_path} or path in context.hashes:
            context.fail("MAP_PATH_INPUT", key, "An output map path must not replace an input or source file.")
        occupied.add(path)
        result[key] = path
    return result


def _check_bindings(context: _Context, data: dict, name: str, path: Path) -> None:
    bindings = data.get("bindings")
    if not isinstance(bindings, dict):
        context.fail("MAP_BINDINGS_REQUIRED", name, "Bind source_inventory, candidate and the complete source set.")
        return
    for key in ("source_inventory", "candidate"):
        context.binding(bindings.get(key), path.parent, name + ".bindings." + key, context.report.checked_files.get(key))
    sources = bindings.get("sources")
    if not isinstance(sources, dict):
        context.fail("MAP_SOURCE_BINDINGS", name, "bindings.sources must be an explicit ID-to-file mapping.")
        return
    if set(sources) != set(context.sources):
        context.fail("MAP_SOURCE_SET", name, "The map must bind every actual source exactly once.")
    for identity, binding in sources.items():
        context.binding(binding, path.parent, name + ".source:" + str(identity), context.source_bindings.get(identity))


def _record_unit(context: _Context, row: dict, identity: str) -> str | None:
    unit = row.get("unit")
    if "unit" in row and not _nonempty(unit):
        context.fail("MAP_TARGET_UNIT_UNKNOWN", identity, "An explicit unit field must contain a registered nonempty unit ID.")
        return None
    if unit is None and row.get("id") in context.units:
        unit = row["id"]
    if unit is None and (not _nonempty(row.get("kind")) or row["kind"] not in _CALL_KINDS):
        unit = row.get("target_unit")
    if not _nonempty(unit) or unit not in context.units:
        context.fail("MAP_TARGET_UNIT_UNKNOWN", identity, "Identify the registered source unit explicitly.")
        return None
    return unit


def _locations_match(value: Any, expected: list[dict]) -> bool:
    if not isinstance(expected, list) or any(not isinstance(row, dict) for row in expected):
        return False
    locations = value if isinstance(value, list) else [value]
    if not locations or any(not isinstance(row, dict) for row in locations):
        return False
    matched = set()
    for location in locations:
        if not any(key in location for key in ("bbox", "quote", "blocks", "image_xref")):
            return False
        choices = [index for index, original in enumerate(expected)
                   if location.get("page") == original.get("page") and
                   all(key in original and location[key] == original[key]
                       for key in ("bbox", "quote", "blocks", "image_xref", "coordinate_system") if key in location)]
        if not choices:
            return False
        matched.update(choices)
    return matched == set(range(len(expected)))


def _unit_fields(context: _Context, unit_id: str) -> tuple[str, ...]:
    unit = context.units[unit_id]
    kind = unit.get("kind")
    if kind == "metadata":
        return ("field",) + (("value",) if "value" in unit else ())
    if kind == "reference":
        return ("bibliography_id", "original_text", "number")
    if kind == "citation":
        return ("references", "display_text")
    if kind == "cross_reference":
        return ("target_unit", "display_text")
    if kind in _OBJECT_KINDS:
        return ("original_number",)
    return ()


def _check_unit_row(context: _Context, row: dict, name: str, collection: str, path: Path) -> str | None:
    label = name + "." + collection + ":" + str(row.get("id"))
    unit_id = _record_unit(context, row, label)
    if unit_id is None:
        return None
    unit = context.units[unit_id]
    source = unit.get("source")
    if row.get("source_id") != source or ("source" in row and row["source"] != source):
        context.fail("MAP_UNIT_SOURCE", label, "The mapped source ID differs from the registered unit.")
    role = context.sources.get(source, {}).get("role")
    if name == "si_map" and role != "supplement":
        context.fail("MAP_SOURCE_ROLE", label, "si_map may describe only actual supplement sources.")
    if "source_role" in row and row["source_role"] != role:
        context.fail("MAP_SOURCE_ROLE", label, "The mapped role differs from the inventory source role.")
    if "source_attachment" in row:
        context.binding(row["source_attachment"], path.parent, label, context.source_bindings.get(source))
    kind = unit.get("kind")
    allowed = {kind, "equation"} if kind == "formula" else {kind}
    if not _nonempty(row.get("kind")) or row["kind"] not in allowed:
        context.fail("MAP_UNIT_KIND", label, "The map kind differs from the registered source unit kind.")
    locations = row.get("source_locator", row.get("evidence"))
    if not _locations_match(locations, unit.get("evidence", [])):
        context.fail("MAP_UNIT_LOCATOR", label, "Retain the unit's complete registered source locations.")
    for key in _unit_fields(context, unit_id):
        if key not in unit:
            context.fail("MAP_SOURCE_FIELD_MISSING", unit_id, f"Register the source field {key!r} explicitly; it cannot be inferred from an anchor or candidate wording.")
        elif key not in row or row[key] != unit[key]:
            context.fail("MAP_UNIT_FIELD", label, f"Retain the registered {key!r} without rewriting it.")
    derived_fields = {"id", "kind", "source", "status", "review_status", "target", "source_attachment", "source_locator", "source_role", "source_id", "unit"}
    for key in set(unit) & set(row) - derived_fields:
        if unit[key] != row[key]:
            context.fail("MAP_UNIT_FIELD", label, f"The recorded inventory field {key!r} changed in the map.")
    if collection == "units":
        for key in set(unit) - derived_fields - set(row):
            context.fail("MAP_UNIT_FIELD", label, f"The complete unit record is missing registered field {key!r}.")
    if "registered_unit" in row and row["registered_unit"] != unit:
        context.fail("MAP_UNIT_SNAPSHOT", label, "The registered-unit snapshot differs from the actual bound inventory.")
    if "original_sha256" in row:
        if not isinstance(unit.get("original_text"), str) or row["original_sha256"] != _text_hash(unit["original_text"]):
            context.fail("MAP_UNIT_ORIGINAL_HASH", label, "The unit text hash needs the exact registered original_text.")
    if "resources" in row:
        for index, resource in enumerate(context.rows(row["resources"], label + ".resources", "path")):
            context.binding(resource, path.parent, f"{label}:resource-{index}")
    if "delivery" in row:
        delivery = row["delivery"]
        if not isinstance(delivery, dict) or delivery.get("unit") != unit_id or delivery.get("target") != row.get("target"):
            context.fail("MAP_DELIVERY_CONFLICT", label, "The retained delivery must identify this same unit and candidate selector.")
    if "target" in row:
        span = context.target(unit_id, row["target"])
        if span is not None and "candidate_locator" in row:
            start, end, value = span
            expected = {"kind": "characters", "unit": "unicode_code_points", "start": start,
                        "end": end, "sha256": _text_hash(value)}
            if row["candidate_locator"] != expected:
                context.fail("MAP_UNIT_CANDIDATE_LOCATOR", label, "The candidate snippet locator/hash differs from the actual explicit target.")
    elif "candidate_locator" in row:
        context.fail("MAP_UNIT_CANDIDATE_LOCATOR", label, "A candidate locator needs its explicit target selector.")
    return unit_id


def _check_source_map(context: _Context, data: dict, name: str, path: Path) -> tuple[dict, set[str]]:
    role = "supplement" if name == "si_map" else "main"
    expected_sources = {key for key, source in context.sources.items() if source.get("role") == role}
    if name == "si_map" or "sources" in data:
        sources = context.index(context.rows(data.get("sources"), name + ".sources"), name + ".sources")
        if set(sources) != expected_sources:
            context.fail("MAP_ROLE_SOURCE_SET", name, f"Record the exact {role} source set in this view.")
        for identity, row in sources.items():
            if row.get("role") != role:
                context.fail("MAP_SOURCE_ROLE", identity, f"This source view requires role={role}.")
            context.binding(row, path.parent, name + ".sources:" + identity, context.source_bindings.get(identity))
    rows = context.rows(data.get("blocks"), name + ".blocks", "block_id")
    records = context.index(rows, name + ".blocks", "block_id")
    covered_blocks, covered_units = set(), set()
    block_targets = {}
    order_keys = set()
    for index, (block_id, row) in enumerate(records.items(), 1):
        source_ids = row.get("source_blocks", [block_id])
        if not isinstance(source_ids, list) or not source_ids or any(not _nonempty(value) or value not in context.blocks for value in source_ids):
            context.fail("MAP_BLOCK_UNKNOWN", block_id, "Use a known original block ID or an explicit source_blocks list.")
            continue
        if len(set(source_ids)) != len(source_ids):
            context.fail("MAP_DUPLICATE_REFERENCE", block_id, "source_blocks contains duplicate IDs.")
        originals = [context.blocks[key] for key in source_ids]
        source = originals[0][0]
        if any(value[0] != source for value in originals) or row.get("source_id") != source:
            context.fail("MAP_BLOCK_SOURCE", block_id, "Original blocks and source_id must identify the same actual source.")
        if context.sources.get(source, {}).get("role") != role:
            context.fail("MAP_SOURCE_ROLE", block_id, f"This block belongs in the {context.sources.get(source, {}).get('role')} source view.")
        context.binding(row.get("source_attachment"), path.parent, block_id, context.source_bindings.get(source))
        locations = [{"page": value[1], "bbox": value[2]["bbox"]} for value in originals]
        if not _locations_match(row.get("source_locators", row.get("source_locator")), locations):
            context.fail("MAP_BLOCK_LOCATOR", block_id, "The map location differs from the original block positions.")
        joiner = row.get("joiner", "")
        if len(source_ids) > 1 and not isinstance(row.get("joiner"), str):
            context.fail("MAP_BLOCK_JOINER", block_id, "Merged blocks need an explicit joiner for their original-text hash.")
            joiner = ""
        if not isinstance(joiner, str):
            context.fail("MAP_BLOCK_JOINER", block_id, "A supplied joiner must be a string.")
            joiner = ""
        original = joiner.join(value[2]["text"] for value in originals)
        if row.get("original_sha256") != _text_hash(original):
            context.fail("MAP_BLOCK_ORIGINAL_HASH", block_id, "The original-text hash differs from the registered unmodified block text.")
        if "original_text" in row and row["original_text"] != original:
            context.fail("MAP_BLOCK_ORIGINAL_TEXT", block_id, "The map changed its original-text snapshot.")
        if type(row.get("order")) is not int or row["order"] < 0:
            context.fail("MAP_BLOCK_ORDER", block_id, "Retain a nonnegative integer source order.")
        else:
            key = (source, row["order"])
            if key in order_keys:
                context.fail("MAP_BLOCK_ORDER", block_id, "Two blocks claim the same order within one source view.")
            order_keys.add(key)
        if not _nonempty(row.get("kind")) or not _nonempty(row.get("reason")):
            context.fail("MAP_BLOCK_DECISION", block_id, "A block needs its kind and a specific keep/exclude reason.")
        units = row.get("target_units", [row["target_unit"]] if "target_unit" in row else [])
        if not isinstance(units, list) or any(not _nonempty(unit) for unit in units):
            context.fail("MAP_TARGET_UNIT_UNKNOWN", block_id, "target_units must list registered unit IDs.")
            units = []
        if len(set(units)) != len(units):
            context.fail("MAP_DUPLICATE_REFERENCE", block_id, "target_units contains duplicate IDs.")
        if "target_unit" in row and (not _nonempty(row["target_unit"]) or row["target_unit"] not in units):
            context.fail("MAP_BLOCK_TARGET_SET", block_id, "target_unit contradicts the explicit target_units mapping.")
        expected_units = set().union(*(context.block_units.get(key, set()) for key in source_ids))
        if row.get("decision") == "keep":
            if set(units) != expected_units or not units:
                context.fail("MAP_BLOCK_TARGET_SET", block_id, "A kept block must map its exact registered source-unit set.")
        elif row.get("decision") == "exclude":
            if units or expected_units or any(key not in context.excluded for key in source_ids):
                context.fail("MAP_EXCLUSION_CONFLICT", block_id, "Only an explicitly excluded, unused source block may be excluded here.")
            if any(row.get("reason") != context.excluded.get(key) for key in source_ids):
                context.fail("MAP_EXCLUSION_REASON", block_id, "Retain the recorded exclusion reason.")
        else:
            context.fail("MAP_BLOCK_DECISION", block_id, "Record keep or exclude explicitly.")
        for unit_id in units:
            if unit_id not in context.units or context.units[unit_id].get("source") != source:
                context.fail("MAP_TARGET_UNIT_SOURCE", block_id, f"Unit {unit_id!r} does not belong to this source block.")
        covered_blocks.update(source_ids)
        covered_units.update(units)
        block_targets[block_id] = {"source_id": source, "source_blocks": source_ids, "units": set(units)}
        context.notify(index, len(records), name + " block records")
    wanted_blocks = {key for key, value in context.blocks.items() if value[0] in expected_sources}
    for block_id in wanted_blocks - covered_blocks:
        context.fail("MAP_BLOCK_MISSING", block_id, f"The known source block is absent from {name}.")
    categories = {"objects": _OBJECT_KINDS, "references": {"reference"}, "citations": _CALL_KINDS,
                  "metadata": {"metadata"}, "units": _KINDS}
    for collection, kinds in categories.items():
        expected = {key for key, unit in context.units.items() if unit.get("source") in expected_sources and unit.get("kind") in kinds}
        required = collection in {"objects", "references", "citations"} and (name == "source_map" or bool(expected))
        if not required and collection not in data:
            continue
        indexed = context.index(context.rows(data.get(collection), name + "." + collection), name + "." + collection)
        found = set()
        for row in indexed.values():
            unit_id = _check_unit_row(context, row, name, collection, path)
            if unit_id is None:
                continue
            if context.units[unit_id].get("kind") not in kinds:
                context.fail("MAP_COLLECTION_KIND", unit_id, f"This unit does not belong in {collection}.")
            if unit_id in found:
                context.fail("MAP_DUPLICATE_UNIT", unit_id, f"Two {collection} records map the same source unit.")
            found.add(unit_id)
        for unit_id in expected - found:
            context.fail("MAP_UNIT_MISSING", unit_id, f"The registered unit is absent from {name}.{collection}.")
        for unit_id in found - expected:
            context.fail("MAP_COLLECTION_SOURCE", unit_id, f"This unit does not belong to {name}.{collection}.")
        covered_units.update(found)
    wanted_units = {key for key, unit in context.units.items() if unit.get("source") in expected_sources}
    for unit_id in wanted_units - covered_units:
        context.fail("MAP_UNIT_MISSING", unit_id, f"No actual block or unit record covers this registered item in {name}.")
    return block_targets, covered_units


def _smallest_owner(context: _Context, start: int, end: int) -> str | None:
    choices = [(last - first, identity) for identity, (first, last, _value) in context.targets.items()
               if first <= start and end <= last]
    if not choices:
        return None
    size = min(value[0] for value in choices)
    matches = [identity for length, identity in choices if length == size]
    return matches[0] if len(matches) == 1 else None


def _anchor_owners(context: _Context, data: dict) -> dict[str, str]:
    rows = context.index(context.rows(data.get("anchors"), "link_map.anchors"), "link_map.anchors")
    owners = {}
    for identity, row in rows.items():
        target = row.get("target_id")
        if not _nonempty(target) or target not in context.anchor_matches:
            context.fail("MAP_ANCHOR_UNKNOWN", identity, "target_id must be an actual candidate block anchor.")
            continue
        if target in owners:
            context.fail("MAP_ANCHOR_DUPLICATE", target, "An actual anchor is recorded more than once.")
        unit = row.get("target_unit", row.get("unit"))
        if unit is None and identity in context.units:
            unit = identity
        if unit is None:
            match = context.anchor_matches[target]
            unit = _smallest_owner(context, match.start(), match.end())
        if not _nonempty(unit) or unit not in context.units:
            context.fail("MAP_ANCHOR_UNIT", target, "Provide an unambiguous registered target_unit for this anchor.")
            continue
        if row.get("source_id") != context.units[unit].get("source"):
            context.fail("MAP_ANCHOR_SOURCE", target, "The anchor source differs from its registered unit source.")
        if "source_role" in row and row["source_role"] != context.sources[context.units[unit]["source"]].get("role"):
            context.fail("MAP_SOURCE_ROLE", target, "The anchor role differs from its registered source role.")
        if not _nonempty(row.get("status")):
            context.fail("MAP_RECORD_STATUS", identity, "Retain the record's status; generation does not mean checked.")
        if unit in context.targets:
            first, last, _text = context.targets[unit]
            match = context.anchor_matches[target]
            if not first <= match.start() < match.end() <= last:
                context.fail("MAP_ANCHOR_TARGET", target, "The anchor is outside its unit's explicit candidate target.")
        if "candidate_locator" in row:
            match = context.anchor_matches[target]
            expected = {"kind": "characters", "start": match.start(), "end": match.end(), "unit": "unicode_code_points",
                        "sha256": _text_hash(context.text[match.start():match.end()])}
            if row["candidate_locator"] != expected:
                context.fail("MAP_ANCHOR_LOCATOR", target, "The anchor locator/hash is not its actual current candidate definition.")
        owners[target] = unit
    for target in context.anchor_matches.keys() - owners.keys():
        context.fail("MAP_ANCHOR_MISSING", target, "Record this actual candidate anchor and its registered unit.")
    return owners


def _internal_target(context: _Context, fragment: str, owners: dict[str, str], identity: str) -> tuple[str | None, str]:
    if fragment.startswith("^"):
        target = fragment[1:]
        if target not in owners:
            context.fail("MAP_LINK_TARGET", identity, "The local block-link target is unknown or lacks a registered owner.")
            return None, target
        return owners[target], target
    locations = context.headings.get(fragment, [])
    if len(locations) != 1:
        context.fail("MAP_LINK_TARGET", identity, "The local heading target is absent or ambiguous in the current candidate.")
        return None, fragment
    unit = _smallest_owner(context, *locations[0])
    if unit is None:
        context.fail("MAP_LINK_TARGET_UNIT", identity, "Bind the actual heading to an explicit source-unit delivery target.")
    return unit, fragment


def _check_link_map(context: _Context, data: dict, blocks: dict) -> None:
    owners = _anchor_owners(context, data)
    actual = {row.key: row for row in context.links}
    accounted = Counter()
    call_targets: dict[str, set[str]] = defaultdict(set)
    link_ids = set()
    for collection in _LINK_COLLECTIONS:
        if collection not in data and collection not in {"internal_links", "library_matches"}:
            continue
        rows = context.index(context.rows(data.get(collection), "link_map." + collection), "link_map." + collection)
        for identity, row in rows.items():
            if identity in link_ids:
                context.fail("MAP_DUPLICATE_ID", identity, "Link occurrence IDs must be unique across link collections.")
            link_ids.add(identity)
            locator = row.get("candidate_locator")
            key = json.dumps(locator, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
            occurrence = actual.get(key)
            if occurrence is None:
                context.fail("MAP_LINK_LOCATOR", identity, "The record does not identify an actual current Markdown/table link occurrence.")
                continue
            accounted[key] += 1
            expected_collection, target = _link_kind(context, occurrence)
            if collection != expected_collection:
                context.fail("MAP_LINK_KIND", identity, f"This occurrence belongs in {expected_collection}.")
            if row.get("href") != occurrence.href or row.get("display_text") != occurrence.label:
                context.fail("MAP_LINK_TEXT", identity, "Retain the actual destination and displayed Markdown label for this occurrence.")
            if "image" in row and row["image"] is not occurrence.image:
                context.fail("MAP_LINK_KIND", identity, "The image/link distinction differs from the actual occurrence.")
            if "syntax" in row and row["syntax"] != occurrence.syntax:
                context.fail("MAP_LINK_KIND", identity, "The recorded syntax differs from the actual occurrence.")
            unit = row.get("origin_unit")
            block_key = row.get("origin_block")
            block = blocks.get(block_key) if _nonempty(block_key) else None
            if not _nonempty(unit) or unit not in context.units:
                context.fail("MAP_LINK_ORIGIN_UNIT", identity, "A link occurrence needs a registered origin_unit.")
                continue
            source = context.units[unit].get("source")
            if row.get("source_id") != source or block is None or block["source_id"] != source or unit not in block["units"]:
                context.fail("MAP_LINK_ORIGIN_BLOCK", identity, "origin_block, origin_unit and source_id must refer to the same registered source content.")
            if not isinstance(row.get("source_display_text"), str):
                context.fail("MAP_LINK_SOURCE_DISPLAY_MISSING", identity, "Retain an explicit source display transcription instead of inferring it from candidate wording.")
            elif "display_text" in context.units[unit] and row["source_display_text"] != context.units[unit]["display_text"]:
                context.fail("MAP_LINK_SOURCE_DISPLAY", identity, "The source display transcription differs from its registered unit.")
            if unit in context.targets:
                first, last, _value = context.targets[unit]
                if not first <= occurrence.start < occurrence.end <= last:
                    context.fail("MAP_LINK_ORIGIN_TARGET", identity, "The link occurrence is outside the origin unit's explicit candidate target.")
            if not _nonempty(row.get("status")):
                context.fail("MAP_RECORD_STATUS", identity, "A link record needs an explicit status.")
            if collection == "internal_links":
                target_unit, target_id = _internal_target(context, target, owners, identity)
                if row.get("target_id") != target_id or row.get("target_unit") != target_unit:
                    context.fail("MAP_LINK_TARGET_UNIT", identity, "The target ID/unit does not match this occurrence's actual local destination.")
                origin = context.units[unit]
                if target_unit is not None and origin.get("kind") in _CALL_KINDS:
                    call_targets[unit].add(target_unit)
                if origin.get("kind") == "citation" and target_unit not in origin.get("references", []):
                    context.fail("MAP_CITATION_TARGET", identity, "This occurrence links outside its explicitly registered source reference set.")
                if origin.get("kind") == "cross_reference" and target_unit != origin.get("target_unit"):
                    context.fail("MAP_CROSS_REFERENCE_TARGET", identity, "This occurrence differs from the source call's explicit target_unit.")
            elif row.get("target_path") != occurrence.href:
                context.fail("MAP_LINK_TARGET_PATH", identity, "Retain the actual candidate path/URL; do not substitute a guessed library match.")
    for key, occurrence in actual.items():
        if accounted[key] != 1:
            context.fail("MAP_LINK_COVERAGE", _text_hash(key)[:20], f"This actual link occurrence has {accounted[key]} mapping records; exactly one is required.")
    for identity, unit in context.units.items():
        kind = unit.get("kind")
        references = unit.get("references")
        if kind == "citation" and isinstance(references, list) and all(_nonempty(value) for value in references):
            if call_targets[identity] != set(references):
                context.fail("MAP_CITATION_COVERAGE", identity, "The mapped link target set does not cover this source citation's complete registered references.")
        target = unit.get("target_unit")
        if kind == "cross_reference" and _nonempty(target) and call_targets[identity] != {target}:
            context.fail("MAP_CROSS_REFERENCE_COVERAGE", identity, "Retain the registered object call at its actual candidate link occurrence.")
    unresolved = context.index(context.rows(data.get("unresolved"), "link_map.unresolved"), "link_map.unresolved")
    for identity, row in unresolved.items():
        unit = row.get("unit", row.get("origin_unit"))
        occurrence = row.get("occurrence_id")
        if (not _nonempty(unit) or unit not in context.units) and (not _nonempty(occurrence) or occurrence not in link_ids):
            context.fail("MAP_UNRESOLVED_IDENTITY", identity, "An unresolved item must identify a known source unit or link occurrence.")
        if not _nonempty(row.get("reason")) or not _nonempty(row.get("status")) or row["status"] not in {"pending", "ambiguous", "unavailable", "absent", "source_error"}:
            context.fail("MAP_UNRESOLVED_REASON", identity, "Record a specific reason and unresolved disposition.")


def _validate_documents(context: _Context, documents: dict[str, dict], paths: dict[str, Path]) -> None:
    blocks = {}
    for name in ("source_map", "si_map"):
        if name not in documents:
            continue
        data = documents[name]
        _check_bindings(context, data, name, paths[name])
        new_blocks, _units = _check_source_map(context, data, name, paths[name])
        for identity in new_blocks.keys() & blocks.keys():
            context.fail("MAP_DUPLICATE_BLOCK", identity, "One block ID appears in both source-role views.")
        blocks.update(new_blocks)
    if "link_map" in documents:
        _check_bindings(context, documents["link_map"], "link_map", paths["link_map"])
        _check_link_map(context, documents["link_map"], blocks)


def validate_handoff_maps(
    inventory_path: Path,
    candidate_path: Path,
    map_paths: Mapping[str, Path],
    *,
    expected_source_bindings=None,
    progress: Progress | None = progress_bar,
) -> HandoffReport:
    """Read-only validation of current bindings, identities and exact coverage.

    ``map_paths`` names source_map/link_map and, when applicable, si_map.
    ``expected_source_bindings`` can retain the initial ID-to-file bindings (or
    a list containing id); an optional role also protects the initial role.
    A ready result certifies neither source fidelity nor any observing event.
    """
    context = _Context(inventory_path, candidate_path, progress)
    context.load_inputs(expected_source_bindings)
    paths = _paths(context, map_paths)
    documents = {}
    for name, path in paths.items():
        data = context.read(path, name)
        binding = context.actual_binding(path, name)
        if binding is not None:
            context.report.checked_files[name] = binding
        if data is not None:
            documents[name] = data
    _validate_documents(context, documents, paths)
    return context.report


def _writer_deliveries(context: _Context, deliveries: Any) -> dict[str, dict]:
    indexed = context.index(context.rows(deliveries, "deliveries", "unit"), "deliveries", "unit")
    for unit in context.units.keys() - indexed.keys():
        context.fail("MAP_DELIVERY_MISSING", unit, "Supply the explicit final-candidate target for this registered source unit.")
    for unit in indexed.keys() - context.units.keys():
        context.fail("MAP_DELIVERY_UNKNOWN", unit, "A delivery cannot introduce an unregistered source identity.")
    for index, (unit, delivery) in enumerate(indexed.items(), 1):
        context.target(unit, delivery.get("target"))
        context.notify(index, len(indexed), "explicit delivery targets")
    return indexed


def _derived_unit(context: _Context, unit_id: str, delivery: dict) -> dict:
    unit = context.units[unit_id]
    row = _copy(unit)
    row.update({
        "unit": unit_id, "source_id": unit["source"],
        "source_role": context.sources[unit["source"]]["role"],
        "source_attachment": context.source_bindings[unit["source"]],
        "source_locator": _copy(unit["evidence"]), "registered_unit": _copy(unit),
        "target": _copy(delivery["target"]), "delivery": _copy(delivery),
        "status": "extracted", "review_status": "pending",
    })
    if unit_id in context.targets:
        start, end, text = context.targets[unit_id]
        row["candidate_locator"] = {"kind": "characters", "unit": "unicode_code_points",
                                    "start": start, "end": end, "sha256": _text_hash(text)}
    if isinstance(unit.get("original_text"), str):
        row["original_sha256"] = _text_hash(unit["original_text"])
    if "assets" in delivery:
        assets = delivery["assets"]
        if not isinstance(assets, list) or any(not isinstance(asset, dict) for asset in assets):
            context.fail("MAP_DELIVERY_RESOURCES", unit_id, "Delivery assets must be explicit file records.")
        else:
            resources = []
            root_value = delivery.get("resource_root", context.candidate_path.parent)
            if not isinstance(root_value, (str, Path)) or not str(root_value):
                context.fail("MAP_RESOURCE_ROOT", unit_id, "An explicitly supplied resource_root must be a directory path.")
                return row
            root = Path(root_value).resolve()
            for index, asset in enumerate(assets):
                if not _nonempty(asset.get("path")):
                    context.fail("MAP_DELIVERY_RESOURCES", unit_id, "Each resource needs its explicit actual path.")
                    continue
                path = Path(asset["path"])
                path = path if path.is_absolute() else root.joinpath(path)
                binding = context.actual_binding(path, f"{unit_id}:asset-{index}")
                if binding is None:
                    continue
                if "sha256" in asset and asset["sha256"] != binding["sha256"]:
                    context.fail("MAP_RESOURCE_HASH", unit_id, "The explicitly supplied resource hash is stale.")
                resources.append(binding)
            row["resources"] = resources
    return row


def _derived_source_map(context: _Context, role: str, deliveries: dict[str, dict]) -> dict:
    sources = {key for key, source in context.sources.items() if source.get("role") == role}
    records = {key: _derived_unit(context, key, deliveries[key]) for key, unit in context.units.items()
               if unit.get("source") in sources and key in deliveries and key in context.targets}
    blocks = []
    for block_id, (source, page, block, order) in context.blocks.items():
        if source not in sources:
            continue
        units = sorted(context.block_units.get(block_id, set()))
        kinds = {context.units[unit].get("kind") for unit in units}
        decision = "exclude" if block_id in context.excluded else "keep"
        row = {
            "block_id": block_id, "source_id": source, "source_blocks": [block_id],
            "source_attachment": context.source_bindings[source],
            "source_locator": {"page": page, "bbox": _copy(block["bbox"])},
            "order": order, "kind": next(iter(kinds)) if len(kinds) == 1 else "mixed" if units else "excluded",
            "original_text": block["text"], "original_sha256": _text_hash(block["text"]),
            "decision": decision,
            "reason": context.excluded.get(block_id, "Accounted for by explicit inventory units and delivery targets; semantic review remains pending."),
            "target_units": units, "status": "extracted", "review_status": "pending",
        }
        if len(units) == 1:
            row["target_unit"] = units[0]
        blocks.append(row)
    return {
        "schema_version": 1, "bindings": context.bindings(), "status": "extracted", "review_status": "pending",
        "source_fidelity": "not_assessed", "boundary": _BOUNDARY,
        "sources": [{"id": identity, "role": role, **context.source_bindings[identity]}
                    for identity in sorted(sources)],
        "blocks": blocks, "units": list(records.values()),
        "metadata": [row for row in records.values() if row["kind"] == "metadata"],
        "objects": [row for row in records.values() if row["kind"] in _OBJECT_KINDS],
        "references": [row for row in records.values() if row["kind"] == "reference"],
        "citations": [row for row in records.values() if row["kind"] in _CALL_KINDS],
    }


def _link_overrides(context: _Context, deliveries: dict[str, dict]) -> dict[str, tuple[str, dict]]:
    result = {}
    actual = {occurrence.key for occurrence in context.links}
    for unit, delivery in deliveries.items():
        for row in context.rows(delivery.get("link_bindings", []), unit + ".link_bindings"):
            key = json.dumps(row.get("candidate_locator"), sort_keys=True, ensure_ascii=False, separators=(",", ":"))
            if key not in actual:
                context.fail("MAP_LINK_OVERRIDE", unit, "A link binding must locate an actual current candidate/table occurrence.")
                continue
            if key in result:
                context.fail("MAP_LINK_OVERRIDE", unit, "An occurrence has more than one explicit origin binding.")
                continue
            result[key] = (unit, row)
    return result


def _derived_link_map(context: _Context, deliveries: dict[str, dict]) -> dict:
    result = {"schema_version": 1, "bindings": context.bindings(), "status": "extracted", "review_status": "pending",
              "source_fidelity": "not_assessed", "boundary": _BOUNDARY,
              "anchors": [], "internal_links": [], "library_matches": [],
              "external_links": [], "resource_links": [], "unresolved": []}
    owners = {}
    for target, match in context.anchor_matches.items():
        unit = _smallest_owner(context, match.start(), match.end())
        if unit is None:
            context.fail("MAP_ANCHOR_DELIVERY_MISSING", target, "Supply a unique delivery selector that covers this actual anchor and its source unit.")
            continue
        owners[target] = unit
        result["anchors"].append({"id": target, "target_id": target, "target_unit": unit,
                                  "source_id": context.units[unit]["source"],
                                  "candidate_locator": {"kind": "characters", "start": match.start(), "end": match.end(),
                                                        "unit": "unicode_code_points", "sha256": _text_hash(context.text[match.start():match.end()])},
                                  "status": "extracted", "review_status": "pending"})
    overrides = _link_overrides(context, deliveries)
    for index, occurrence in enumerate(context.links, 1):
        explicit = overrides.get(occurrence.key)
        unit = explicit[0] if explicit else _smallest_owner(context, occurrence.start, occurrence.end)
        override = explicit[1] if explicit else {}
        identity = "link-" + _text_hash(occurrence.key)
        if unit is None:
            context.fail("MAP_LINK_DELIVERY_MISSING", identity,
                         "Provide an unambiguous unit delivery or link_bindings entry for " + occurrence.key)
            continue
        original = context.units[unit]
        origin_block = override.get("origin_block", deliveries[unit].get("origin_block"))
        blocks = context.unit_blocks.get(unit, set())
        if origin_block is None and len(blocks) == 1:
            origin_block = next(iter(blocks))
        if not _nonempty(origin_block) or origin_block not in blocks:
            context.fail("MAP_LINK_SOURCE_BLOCK_MISSING", identity,
                         "Supply origin_block from this unit's registered source blocks; more than one or none is available.")
            continue
        source_display = override.get("source_display_text", original.get("display_text"))
        if not isinstance(source_display, str):
            context.fail("MAP_LINK_SOURCE_DISPLAY_MISSING", identity,
                         "Record the source display text explicitly in the unit or link_bindings; candidate wording cannot supply it. " + occurrence.key)
            continue
        collection, target = _link_kind(context, occurrence)
        row = {"id": identity, "source_id": original["source"], "origin_unit": unit, "origin_block": origin_block,
               "candidate_locator": _copy(occurrence.locator), "href": occurrence.href,
               "display_text": occurrence.label, "source_display_text": source_display,
               "syntax": occurrence.syntax, "image": occurrence.image,
               "status": "extracted", "review_status": "pending"}
        if collection == "internal_links":
            target_unit, target_id = _internal_target(context, target, owners, identity)
            row.update({"target_id": target_id, "target_unit": target_unit})
            if "target_unit" in override and override["target_unit"] != target_unit:
                context.fail("MAP_LINK_TARGET_UNIT", identity, "The explicit target_unit differs from the actual candidate destination.")
        else:
            row["target_path"] = occurrence.href
        if collection == "library_matches":
            row.update({"status": "unavailable", "match_status": "unavailable",
                        "reason": "No real-library identity lookup was supplied or performed."})
            result["unresolved"].append({"id": "unresolved-" + identity, "occurrence_id": identity,
                                         "unit": unit, "source_id": original["source"],
                                         "status": "unavailable", "reason": row["reason"]})
        result[collection].append(row)
        context.notify(index, len(context.links), "explicit link occurrence mappings")
    return result


def write_handoff_maps(
    inventory_path: Path,
    candidate_path: Path,
    deliveries: Sequence[Mapping[str, Any]],
    map_paths: Mapping[str, Path],
    *,
    expected_source_bindings=None,
    progress: Progress | None = progress_bar,
) -> HandoffReport:
    """Derive new handoff views from registered facts and explicit deliveries.

    Each delivery identifies ``unit`` and one current ``target`` (anchor, unique
    quote, or hashed lines), following the existing delivery contract. No text
    is translated, classified from model memory, or marked visually verified.
    Selectors use the contract's LF parsing view, including UTF-8/LF line-range
    hashes. Generated candidate_locator offsets and snippet hashes instead refer
    to the original candidate text; complete-file bindings hash original bytes.

    Missing source numbers, call display text, ambiguous target ownership or
    ambiguous source blocks produce findings instead of fabricated mappings.
    For a link needing more detail, supply delivery.link_bindings entries with
    candidate_locator (reported in the finding), source_display_text and, when
    needed, origin_block. A single unit-wide source display can be registered
    as inventory.units[].display_text. Optional delivery assets are bound using
    explicit resource_root, otherwise the candidate directory, never a guessed
    real vault. Source files and old maps are never changed.

    Nothing is written if preflight or structural checks fail. Existing output
    paths are refused as a group; select new versioned paths to retain history.
    Exclusive creation also protects against a concurrent writer. An I/O race
    can leave new partial views, which are reported and retained for inspection.
    """
    context = _Context(inventory_path, candidate_path, progress)
    context.load_inputs(expected_source_bindings)
    paths = _paths(context, map_paths)
    for name, path in paths.items():
        supplied_path = Path(map_paths[name])
        if path.exists() or path.is_symlink() or supplied_path.exists() or supplied_path.is_symlink():
            context.fail("MAP_OUTPUT_EXISTS", name, f"Keep the existing record and choose a new versioned path: {path}")
    indexed = _writer_deliveries(context, deliveries)
    if not context.report.records_ready:
        return context.report
    documents = {"source_map": _derived_source_map(context, "main", indexed)}
    if "si_map" in paths:
        documents["si_map"] = _derived_source_map(context, "supplement", indexed)
    documents["link_map"] = _derived_link_map(context, indexed)
    _validate_documents(context, documents, paths)
    if not context.report.records_ready:
        return context.report
    payloads = {name: json.dumps(document, ensure_ascii=False, indent=2, allow_nan=False) + "\n"
                for name, document in documents.items()}
    # Recheck input bytes before creating any map; this is not a concurrency lock.
    for path, expected_hash in context.hashes.items():
        try:
            if _digest(path) != expected_hash:
                context.fail("MAP_INPUT_CHANGED", str(path), "An input or resource changed during map preparation; preserve the old snapshot.")
        except OSError as error:
            context.fail("MAP_INPUT_CHANGED", str(path), str(error))
    if not context.report.records_ready:
        return context.report
    for index, (name, payload) in enumerate(payloads.items(), 1):
        path = paths[name]
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("x", encoding="utf-8", newline="\n") as stream:
                stream.write(payload)
            context.report.written_paths[name] = str(path)
            reread = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_pairs)
            if reread != documents[name]:
                context.fail("MAP_WRITE_READBACK", name, "The actual written map differs from the prepared view.")
            context.report.checked_files[name] = {"path": str(path), "sha256": _digest(path)}
        except (OSError, ValueError) as error:
            context.fail("MAP_WRITE_FAILED", name, str(error))
            break
        context.notify(index, len(payloads), "new handoff views written and read back")
    return context.report
