"""Check the evidence chain of a PDF-to-Markdown delivery, without editing it.

Set MANIFEST_PATH below and run this file, or call validate_delivery(path).
See ../references/delivery-checks.md for the incremental inventory contract.
Zero findings mean recorded evidence is structurally consistent. They never
mean that translation, visual interpretation, or source fidelity has passed.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
from collections import Counter
from html import unescape
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
import sys
from typing import Any, Callable
from urllib.parse import unquote

import fitz
import yaml


MANIFEST_PATH: Path | None = None
REPORT_PATH: Path | None = None
KINDS = {"metadata", "paragraph", "section", "figure", "table", "formula",
         "reference", "citation", "cross_reference"}
FIELDS = {"title": "title", "authors": "author", "affiliations": "affiliation",
          "publication_date": "publication_date"}
_ANCHOR = re.compile(r"(?<!\S)\^([A-Za-z0-9-]+)[ \t]*$", re.MULTILINE)
_FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
_IMAGE = re.compile(r"!\[\[([^\]\n|]+)(?:\|[^\]\n]*)?\]\]|!\[[^\]\n]*\]\(<?([^\s)>]+)>?\)")


def _load_sibling(name: str):
    path = Path(__file__).resolve().parent.joinpath(name + ".py")
    path_key = hashlib.sha256(str(path).encode("utf-8")).hexdigest()[:16]
    module_name = f"literature_delivery_{name}_{path_key}"
    if module_name in sys.modules:
        return sys.modules[module_name]
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise ImportError(path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


note_checks = _load_sibling("validate_note")
source_tools = _load_sibling("prepare_source_inventory")


@dataclass(frozen=True)
class MarkdownLink:
    href: str
    start: int
    end: int
    image: bool
    label: str


def scan_markdown_links(text: str) -> list[MarkdownLink]:
    """Parse inline links, including angle destinations and optional titles.

    Offsets refer to the supplied text. Callers control masking of code,
    mathematics and comments. Reference-style links are outside this helper.
    """
    stack, pairs = [], {}
    index = 0
    while index < len(text):
        if text[index] == "\\":
            index += 2
            continue
        if text[index] == "[":
            stack.append(index)
        elif text[index] == "]" and stack:
            pairs[stack.pop()] = index
        index += 1
    result, consumed = [], -1
    for opening, closing in sorted(pairs.items()):
        if opening < consumed or text[closing + 1:closing + 2] != "(":
            continue
        cursor = closing + 2
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1
        if cursor >= len(text):
            continue
        angle = text[cursor] == "<"
        destination_start = cursor + int(angle)
        cursor = destination_start
        depth = 0
        while cursor < len(text):
            character = text[cursor]
            if character == "\\" and cursor + 1 < len(text):
                cursor += 2
                continue
            if angle:
                if character in {">", "\n", "\r"}:
                    break
            else:
                if character.isspace() or character == ")" and depth == 0:
                    break
                if character == "(":
                    depth += 1
                elif character == ")":
                    depth -= 1
            cursor += 1
        if cursor >= len(text) or angle and text[cursor] != ">" or depth:
            continue
        href = text[destination_start:cursor]
        cursor += int(angle)
        whitespace_start = cursor
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1
        if cursor < len(text) and text[cursor] != ")":
            if cursor == whitespace_start or text[cursor] not in {"'", '"', "("}:
                continue
            ending = ")" if text[cursor] == "(" else text[cursor]
            cursor += 1
            while cursor < len(text) and (text[cursor] != ending or note_checks._escaped(text, cursor)):
                cursor += 1
            if cursor >= len(text):
                continue
            cursor += 1
            while cursor < len(text) and text[cursor].isspace():
                cursor += 1
        if cursor >= len(text) or text[cursor] != ")":
            continue
        image = opening > 0 and text[opening - 1] == "!" and not note_checks._escaped(text, opening - 1)
        start = opening - int(image)
        href = unescape(re.sub(r"\\([!\"#$%&'()*+,\-./:;<=>?@\[\\\]^_`{|}~])", r"\1", href))
        result.append(MarkdownLink(href, start, cursor + 1, image, text[opening + 1:closing]))
        consumed = cursor + 1
    return result


def _reference_links(text: str) -> list[tuple[str, int]]:
    visible = note_checks._mask_delimited(note_checks._mask_delimited(text, "`"), "$")
    links = [(unquote(link.href)[2:], link.start) for link in scan_markdown_links(visible)
             if re.fullmatch(r"#\^[A-Za-z0-9-]+", unquote(link.href))]
    links.extend((match["id"], match.start()) for match in note_checks._LOCAL_WIKI.finditer(visible)
                 if not note_checks._escaped(visible, match.start()))
    return links


def _image_destinations(text: str) -> list[str]:
    values = [link.href for link in scan_markdown_links(text) if link.image]
    values.extend(match[1] for match in re.finditer(r"!\[\[([^\]\n|]+)(?:\|[^\]\n]*)?\]\]", text)
                  if not note_checks._escaped(text, match.start()))
    return values


@dataclass(frozen=True)
class DeliveryFinding:
    code: str
    object_id: str
    message: str
    level: str = "error"
    line: int | None = None


@dataclass
class DeliveryReport:
    findings: list[DeliveryFinding] = field(default_factory=list)
    source_fidelity: str = "not_assessed"
    independent_review_required: bool = True
    evidence_boundary: str = (
        "File provenance, inventory accounting and candidate targets are checked. "
        "The completeness of human classifications, visual reading, translation, "
        "scientific meaning and live Obsidian behavior require independent review."
    )

    @property
    def structurally_consistent(self) -> bool:
        return not any(item.level == "error" for item in self.findings)

    def to_dict(self) -> dict:
        return {**asdict(self), "structurally_consistent": self.structurally_consistent}


def _unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def _normal(text: str) -> str:
    return " ".join(text.split())


def _visible(text: str, *, keep_tables: bool = True, keep_table_fences: bool = False) -> str:
    """Mask examples/comments while retaining offsets and actual Rich Table data."""
    lines = text.splitlines(keepends=True)
    output = []
    fenced = None
    comment = None
    frontmatter = bool(lines and lines[0].strip() == "---")
    for index, line in enumerate(lines):
        if frontmatter:
            output.append(note_checks._blank(line))
            if index and line.strip() == "---":
                frontmatter = False
            continue
        if fenced:
            marker, language = fenced
            closing = bool(re.fullmatch(r" {0,3}" + re.escape(marker[0]) + "{" + str(len(marker)) + r",}[ \t]*\n?", line))
            output.append(line if language == "rich-table" and keep_tables and (not closing or keep_table_fences) else note_checks._blank(line))
            if closing:
                fenced = None
            continue
        safe, comment = note_checks._mask_comments(line, comment)
        opening = _FENCE.match(safe)
        if opening:
            fenced = (opening[1], opening[2].strip())
            output.append(safe if keep_tables and keep_table_fences and opening[2].strip() == "rich-table" else note_checks._blank(safe))
            continue
        if len(safe.expandtabs(4)) - len(safe.expandtabs(4).lstrip()) >= 4:
            output.append(note_checks._blank(safe))
            continue
        output.append(note_checks._mask_delimited(safe, "`"))
    return "".join(output)


def write_delivery_manifest(
    inventory_path: Path,
    candidate_path: Path,
    deliveries: list[dict],
    manifest_path: Path,
    *,
    resource_root: Path | None = None,
) -> Path:
    """Bind completed files to a mapping. This does not invent missing targets."""
    if not inventory_path.is_file() or not candidate_path.is_file():
        raise FileNotFoundError("Create the inventory and final candidate before binding their hashes")
    root = (resource_root or candidate_path.parent).resolve()
    if not root.is_dir():
        raise NotADirectoryError(root)
    copied = json.loads(json.dumps(deliveries, ensure_ascii=False))
    for delivery in copied:
        for asset in delivery.get("assets", []):
            path = Path(asset["path"])
            if not path.is_absolute():
                path = root.joinpath(path)
            asset.update(source_tools.file_record(path))
    manifest = {
        "schema_version": 1,
        "inventory": source_tools.file_record(inventory_path),
        "candidate": source_tools.file_record(candidate_path),
        "resource_root": str(root),
        "deliveries": copied,
    }
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return manifest_path


class _Audit:
    def __init__(self, manifest_path: Path, progress: Callable | None):
        self.path = manifest_path
        self.progress = progress
        self.report = DeliveryReport()
        self.hashes: dict[Path, str] = {}
        self.sources: dict[str, dict] = {}
        self.pages: dict[tuple[str, int], dict] = {}
        self.rects: dict[tuple[str, int], fitz.Rect] = {}
        self.blocks: dict[str, tuple[str, int, dict]] = {}
        self.used_blocks: set[str] = set()
        self.used_pages: set[tuple[str, int]] = set()
        self.units: dict[str, dict] = {}
        self.targets: dict[str, tuple[int, int, str]] = {}
        self.deliveries: dict[str, dict] = {}
        self.text = ""
        self.visible = ""
        self.prose_visible = ""
        self.table_visible = ""
        self.candidate_path = manifest_path
        self.resource_root = manifest_path.parent
        self.anchor_matches = {}
        self.table_blocks: list[dict] = []
        self.table_mappings: set[tuple[int, int]] = set()

    def fail(self, code: str, object_id: str, message: str, *, line: int | None = None):
        self.report.findings.append(DeliveryFinding(code, object_id, message, line=line))

    def read_json(self, path: Path, object_id: str) -> dict:
        try:
            value = json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=_unique_pairs)
        except (OSError, UnicodeError, ValueError) as error:
            self.fail("JSON_READ", object_id, str(error))
            return {}
        if not isinstance(value, dict):
            self.fail("JSON_OBJECT", object_id, "The document must be a JSON object.")
            return {}
        return value

    def record(self, record: Any, base: Path, object_id: str) -> Path | None:
        if not isinstance(record, dict) or not isinstance(record.get("path"), str):
            self.fail("FILE_RECORD", object_id, "Provide path and SHA-256 for this evidence file.")
            return None
        path = Path(record["path"])
        if not path.is_absolute():
            path = base.joinpath(path)
        path = path.resolve()
        if not path.is_file():
            self.fail("FILE_MISSING", object_id, f"Missing evidence file: {path}")
            return None
        if not re.fullmatch(r"[a-f0-9]{64}", str(record.get("sha256", ""))):
            self.fail("FILE_HASH_REQUIRED", object_id, "A real lowercase SHA-256 is required.")
            return None
        if path not in self.hashes:
            self.hashes[path] = source_tools.sha256_file(path)
        if self.hashes[path] != record["sha256"]:
            self.fail("FILE_HASH_MISMATCH", object_id, f"Evidence changed after binding: {path}")
            return None
        return path

    def rows(self, value: Any, object_id: str, name: str) -> list[dict]:
        if not isinstance(value, list) or any(not isinstance(item, dict) for item in value):
            self.fail("ARRAY_REQUIRED", object_id, f"{name} must be an array of objects.")
            return []
        return value

    def index(self, rows: list[dict], key: str, object_id: str) -> dict[str, dict]:
        result = {}
        for row in rows:
            identity = row.get(key)
            if not isinstance(identity, str) or not identity.strip():
                self.fail("ID_REQUIRED", object_id, f"Every entry needs a nonempty {key}.")
                continue
            if identity in result:
                self.fail("ID_DUPLICATE", identity, f"Duplicate {key}.")
                continue
            result[identity] = row
        return result

    def check_sources(self, inventory: dict, base: Path):
        self.sources = self.index(self.rows(inventory.get("sources"), "inventory", "sources"), "id", "sources")
        if not self.sources:
            self.fail("SOURCES_EMPTY", "inventory", "At least one source is required.")
        total = sum(len(value.get("pages", [])) for value in self.sources.values() if isinstance(value.get("pages"), list))
        done = 0
        for identity, source in self.sources.items():
            if source.get("role") not in {"main", "supplement"}:
                self.fail("SOURCE_ROLE", identity, "Choose main or supplement from the source identity.")
            path = self.record(source, base, identity)
            if path is None:
                continue
            try:
                document = fitz.open(path)
            except (fitz.mupdf.FzErrorBase, RuntimeError, ValueError, OSError) as error:
                self.fail("SOURCE_PDF", identity, str(error))
                continue
            with document:
                if not document.is_pdf or document.needs_pass:
                    self.fail("SOURCE_PDF", identity, "An accessible PDF is required by this checker.")
                    continue
                if source.get("page_count") != len(document):
                    self.fail("SOURCE_PAGE_COUNT", identity, f"Actual PDF has {len(document)} pages.")
                page_rows = self.rows(source.get("pages"), identity, "pages")
                numbers = [row.get("page") for row in page_rows]
                if numbers != list(range(1, len(document) + 1)):
                    self.fail("SOURCE_PAGE_SET", identity, "Record every actual PDF page once in source order.")
                for row in page_rows:
                    number = row.get("page")
                    if type(number) is not int or not 1 <= number <= len(document):
                        continue
                    key = (identity, number)
                    page = document[number - 1]
                    self.pages[key], self.rects[key] = row, page.rect
                    text_path = self.record(row.get("text"), base, f"{identity}:page-{number}:text")
                    if text_path is not None and text_path.read_text(encoding="utf-8") != page.get_text("text", sort=False):
                        self.fail("TEXT_PROVENANCE", identity, f"Page {number} text does not match the PDF text layer.")
                    actual_blocks = [{"id": f"{identity}-p{number}-b{block[5]}", "bbox": list(block[:4]), "text": block[4]}
                                     for block in page.get_text("blocks", sort=False)
                                     if block[6] == 0 and str(block[4]).strip()]
                    if row.get("blocks") != actual_blocks:
                        self.fail("BLOCK_PROVENANCE", identity, f"Page {number} text-block inventory differs from the PDF.")
                    for block in actual_blocks:
                        self.blocks[block["id"]] = (identity, number, block)
                    self.check_render(row.get("render"), base, identity, number, page)
                    done += 1
                    if self.progress:
                        self.progress(done, total, f"verify {identity} page {number}")

    def check_render(self, record: Any, base: Path, identity: str, number: int, page):
        label = f"{identity}:page-{number}:render"
        path = self.record(record, base, label)
        if path is None:
            return
        dpi = record.get("dpi")
        if type(dpi) is not int or not 72 <= dpi <= 600:
            self.fail("RENDER_DPI", label, "Use the actual integer render DPI, between 72 and 600.")
            return
        try:
            image = fitz.Pixmap(path)
            expected = page.get_pixmap(dpi=dpi, colorspace=fitz.csRGB, alpha=False)
        except (fitz.mupdf.FzErrorBase, RuntimeError, ValueError, OSError) as error:
            self.fail("RENDER_READ", label, str(error))
            return
        if (image.width, image.height, image.n, image.alpha) != (expected.width, expected.height, 3, 0):
            self.fail("RENDER_GEOMETRY", label, "The evidence must be the full RGB source page without alpha.")
            return
        actual_hash = hashlib.sha256(image.samples).hexdigest()
        expected_hash = hashlib.sha256(expected.samples).hexdigest()
        if actual_hash != expected_hash or record.get("pixel_sha256") != expected_hash:
            self.fail("RENDER_PROVENANCE", label, "Rendered pixels do not match this source page.")

    def evidence(self, evidence: Any, source: str, identity: str, *, visual: bool = False):
        rows = self.rows(evidence, identity, "evidence")
        if not rows:
            self.fail("SOURCE_EVIDENCE_REQUIRED", identity, "Locate this item on at least one source page.")
        for row in rows:
            page = row.get("page")
            if type(page) is not int or (source, page) not in self.pages:
                self.fail("EVIDENCE_PAGE", identity, "Evidence points to an unknown source page.")
                continue
            self.used_pages.add((source, page))
            basis = row.get("basis")
            if basis not in {"text", "visual"}:
                self.fail("EVIDENCE_BASIS", identity, "Declare text or visual evidence; a passed flag is not evidence.")
            if visual and basis != "visual":
                self.fail("VISIBLE_PAGE_REQUIRED", identity, "This field or object requires the visible source page.")
            quote, box = row.get("quote"), row.get("bbox")
            if not (isinstance(quote, str) and quote.strip()) and box is None:
                self.fail("SOURCE_LOCATOR", identity, "Provide a source quote or page-space bounding box.")
            if box is not None:
                valid = (isinstance(box, list) and len(box) == 4 and
                         all(type(value) in (int, float) and math.isfinite(value) for value in box))
                if not valid or not fitz.Rect(box).is_valid or fitz.Rect(box).is_empty:
                    self.fail("SOURCE_BBOX", identity, "Use a finite nonempty page-space rectangle.")
                elif not self.rects[(source, page)].contains(fitz.Rect(box)):
                    self.fail("SOURCE_BBOX", identity, "The evidence rectangle falls outside its source page.")
            if basis == "text" and isinstance(quote, str) and quote.strip():
                page_text = "\n".join(block["text"] for block in self.pages[(source, page)].get("blocks", []))
                if _normal(quote) not in _normal(page_text):
                    self.fail("TEXT_QUOTE", identity, "The declared text quote is absent from this page's text layer.")
            blocks = row.get("blocks", [])
            if not isinstance(blocks, list) or any(not isinstance(item, str) for item in blocks):
                self.fail("EVIDENCE_BLOCKS", identity, "blocks must be a list of source block IDs.")
                continue
            for block in blocks:
                if block not in self.blocks or self.blocks[block][:2] != (source, page):
                    self.fail("EVIDENCE_BLOCK_SOURCE", identity, f"Block {block} does not belong to this source page.")
                    continue
                self.used_blocks.add(block)

    def check_inventory(self, inventory: dict):
        self.units = self.index(self.rows(inventory.get("units"), "inventory", "units"), "id", "units")
        if not self.units:
            self.fail("UNITS_EMPTY", "inventory", "Classify the source contents before final validation.")
        for identity, unit in self.units.items():
            source, kind = unit.get("source"), unit.get("kind")
            if source not in self.sources:
                self.fail("UNIT_SOURCE", identity, "Choose a source ID from the inventory.")
                continue
            if kind not in KINDS:
                self.fail("UNIT_KIND", identity, f"Choose one unit kind from {sorted(KINDS)}.")
            self.evidence(unit.get("evidence"), source, identity, visual=kind in {"metadata", "figure", "table", "formula"})
            if kind == "metadata" and unit.get("field") not in {*FIELDS.values(), "doi", "email"}:
                self.fail("METADATA_FIELD", identity, "Declare one supported metadata field.")
            if kind == "figure" and type(unit.get("contains_table")) is not bool:
                self.fail("FIGURE_TABLE_CLASSIFICATION", identity, "Inspect the figure and explicitly classify whether it contains a table.")
            if kind == "table":
                shape = unit.get("shape")
                if not isinstance(shape, dict) or any(type(shape.get(key)) is not int or shape[key] < 1 for key in ("rows", "columns")):
                    self.fail("TABLE_SHAPE", identity, "Record the source table's positive row and column counts.")
                if not isinstance(unit.get("merges"), list):
                    self.fail("SOURCE_TABLE_MERGES", identity, "Record each source merged range; use an empty array only when the source has none.")
            if kind == "reference" and not str(unit.get("number", "")).strip():
                self.fail("REFERENCE_NUMBER", identity, "Retain the reference's source number or source label.")
        for source_id, source in self.sources.items():
            decisions = source.get("header_fields")
            if not isinstance(decisions, dict):
                self.fail("HEADER_DECISIONS", source_id, "Record the source's independent header fields.")
                continue
            for field_name, unit_field in FIELDS.items():
                decision = decisions.get(field_name)
                label = f"{source_id}:{field_name}"
                if not isinstance(decision, dict):
                    self.fail("HEADER_DECISION", label, "Record present or absent_in_source.")
                    continue
                if decision.get("status") == "absent_in_source":
                    self.evidence(decision.get("evidence"), source_id, label, visual=True)
                    if not isinstance(decision.get("reason"), str) or not decision["reason"].strip():
                        self.fail("ABSENCE_REASON", label, "Explain the source-specific absence without inventing a value.")
                    if any(unit.get("source") == source_id and unit.get("field") == unit_field for unit in self.units.values()):
                        self.fail("HEADER_ABSENCE_CONFLICT", label, "A declared absent field also has a delivered source unit.")
                    continue
                if decision.get("status") != "present":
                    self.fail("HEADER_PENDING", label, "Resolve this field from the visible source page.")
                    continue
                ids = decision.get("units")
                if not isinstance(ids, list) or not ids:
                    self.fail("HEADER_UNITS", label, "Point to each corresponding atomic metadata unit.")
                    continue
                expected = {key for key, unit in self.units.items() if unit.get("source") == source_id and unit.get("kind") == "metadata" and unit.get("field") == unit_field}
                if any(not isinstance(key, str) for key in ids) or set(ids) != expected or not expected:
                    self.fail("HEADER_UNITS", label, "The field mapping must cover exactly its metadata units in this source.")
        for identity, unit in self.units.items():
            if unit.get("kind") == "figure" and unit.get("contains_table") is True:
                if not any(item.get("kind") == "table" and item.get("parent_id") == identity for item in self.units.values()):
                    self.fail("EMBEDDED_TABLE_MISSING", identity, "Create a separate editable-table unit for the table inside this figure.")
            parent = unit.get("parent_id")
            if parent is not None and (unit.get("kind") != "table" or parent not in self.units or self.units[parent].get("kind") != "figure" or self.units[parent].get("source") != unit.get("source")):
                self.fail("OBJECT_PARENT", identity, "An embedded table must point to a figure in the same source.")
        excluded = set()
        for row in self.rows(inventory.get("excluded_blocks", []), "inventory", "excluded_blocks"):
            block = row.get("block")
            if block not in self.blocks:
                self.fail("EXCLUDED_BLOCK", str(block), "Unknown source block.")
                continue
            if not isinstance(row.get("reason"), str) or not row["reason"].strip():
                self.fail("EXCLUSION_REASON", block, "Give a specific source-based exclusion reason.")
                continue
            if block in self.used_blocks:
                self.fail("BLOCK_EXCLUSION_CONFLICT", block, "The block is both mapped and excluded.")
            excluded.add(block)
        for block in sorted(set(self.blocks) - self.used_blocks - excluded):
            self.fail("SOURCE_BLOCK_UNMAPPED", block, "Map this source text block or record a justified exclusion.")
        excluded_pages = set()
        for row in self.rows(inventory.get("excluded_pages", []), "inventory", "excluded_pages"):
            key = (row.get("source"), row.get("page"))
            if key not in self.pages or not isinstance(row.get("reason"), str) or not row["reason"].strip():
                self.fail("PAGE_EXCLUSION", str(key), "An excluded page needs an actual source page and a specific reason.")
                continue
            if key in self.used_pages:
                self.fail("PAGE_EXCLUSION_CONFLICT", str(key), "The page is both mapped and excluded.")
            excluded_pages.add(key)
        for key in sorted(set(self.pages) - self.used_pages - excluded_pages):
            self.fail("SOURCE_PAGE_UNMAPPED", f"{key[0]}:page-{key[1]}", "Record content from this page or a justified page exclusion.")

    def resolve_target(self, target: Any, identity: str) -> tuple[int, int, str] | None:
        if not isinstance(target, dict):
            self.fail("TARGET_REQUIRED", identity, "Provide one anchor, unique quote or hashed line range.")
            return None
        choices = set(target) & {"anchor", "quote", "lines"}
        if len(choices) != 1:
            self.fail("TARGET_SELECTOR", identity, "Use exactly one target selector.")
            return None
        if "quote" in target:
            quote = target["quote"]
            if not isinstance(quote, str) or not quote.strip() or self.text.count(quote) != 1:
                self.fail("TARGET_QUOTE", identity, "The target quote must occur exactly once in the final candidate.")
                return None
            start = self.text.index(quote)
            end = start + len(quote)
        elif "lines" in target:
            bounds = target["lines"]
            lines = self.text.splitlines(keepends=True)
            if not isinstance(bounds, list) or len(bounds) != 2 or any(type(value) is not int for value in bounds) or not 1 <= bounds[0] <= bounds[1] <= len(lines):
                self.fail("TARGET_LINES", identity, "Use a valid one-based inclusive line range.")
                return None
            start = sum(map(len, lines[:bounds[0] - 1]))
            end = sum(map(len, lines[:bounds[1]]))
            digest = hashlib.sha256(self.text[start:end].encode("utf-8")).hexdigest()
            if target.get("sha256") != digest:
                self.fail("TARGET_LINES_HASH", identity, "The exact line-range bytes need their own matching SHA-256.")
                return None
        else:
            anchor = target["anchor"]
            if not isinstance(anchor, str) or anchor not in self.anchor_matches:
                self.fail("TARGET_ANCHOR", identity, "The block anchor is absent from visible candidate content.")
                return None
            match = self.anchor_matches[anchor]
            start = self.text.rfind("\n", 0, match.start()) + 1
            end = self.text.find("\n", match.end())
            end = len(self.text) if end < 0 else end
            inline_anchor = bool(self.text[start:match.start()].strip())
            preceding = self.text[:start].splitlines(keepends=True)
            if inline_anchor and self.units[identity].get("kind") == "figure":
                cursor = start
                while preceding and not preceding[-1].strip():
                    cursor -= len(preceding.pop())
                if preceding and _image_destinations(preceding[-1]):
                    start = cursor - len(preceding[-1])
            elif inline_anchor:
                pass
            elif preceding and re.fullmatch(r"(?:`{3,}|~{3,}|\$\$)\s*", preceding[-1]):
                marker = preceding[-1].strip()
                boundary = next((i for i in range(len(preceding) - 2, -1, -1) if preceding[i].lstrip().startswith(marker)), None)
                if boundary is None:
                    self.fail("TARGET_BLOCK", identity, "The anchored data block has no opening delimiter.")
                    return None
                start = sum(map(len, preceding[:boundary]))
            else:
                while preceding and preceding[-1].strip():
                    start -= len(preceding.pop())
        if not self.visible[start:end].strip():
            self.fail("TARGET_HIDDEN", identity, "The target is only a comment, code example or frontmatter.")
            return None
        value = self.text[start:end]
        if not re.sub(r"\^[A-Za-z0-9-]+", "", value).strip():
            self.fail("TARGET_EMPTY", identity, "A bare anchor is not delivered content.")
            return None
        return start, end, value

    def check_deliveries(self, manifest: dict):
        rows = self.rows(manifest.get("deliveries"), "manifest", "deliveries")
        self.deliveries = self.index(rows, "unit", "deliveries")
        for identity in sorted(set(self.units) - set(self.deliveries)):
            self.fail("UNIT_NOT_DELIVERED", identity, "The source inventory item has no final delivery target.")
        for identity in sorted(set(self.deliveries) - set(self.units)):
            self.fail("DELIVERY_WITHOUT_SOURCE", identity, "The delivery does not correspond to a source inventory item.")
        self.visible = _visible(self.text)
        self.prose_visible = _visible(self.text, keep_tables=False)
        self.table_visible = _visible(self.text, keep_table_fences=True)
        self.anchor_matches = {match[1]: match for match in _ANCHOR.finditer(self.prose_visible)}
        self.table_blocks = self.parse_table_blocks()
        for identity, delivery in self.deliveries.items():
            if identity not in self.units:
                continue
            resolved = self.resolve_target(delivery.get("target"), identity)
            if resolved is None:
                continue
            self.targets[identity] = resolved
            self.check_target_content(identity, self.units[identity], delivery, resolved)
        self.check_worksheet_coverage()
        self.check_links()
        mapped_anchors = {match[1] for identity, (start, end, _) in self.targets.items()
                          if self.units[identity].get("kind") in {"reference", "figure", "table", "formula"}
                          for match in _ANCHOR.finditer(self.prose_visible[start:end])}
        for anchor in self.anchor_matches:
            if (anchor.startswith(("fig-", "tbl-", "eq-")) or note_checks._citation_preview_anchor(anchor)) and anchor not in mapped_anchors:
                self.fail("OUTPUT_OBJECT_UNMAPPED", anchor, "This output object lacks a corresponding source inventory unit.")

    def check_target_content(self, identity: str, unit: dict, delivery: dict, resolved: tuple[int, int, str]):
        start, end, _ = resolved
        text = self.visible[start:end]
        prose = self.prose_visible[start:end]
        kind = unit.get("kind")
        if kind == "figure":
            self.check_figure(identity, delivery, text)
        if kind == "table":
            self.check_table(identity, unit, delivery, self.table_visible[start:end])
        display_math = re.search(r"(?m)^ {0,3}\$\$[^\n]+\$\$[ \t]*$|^ {0,3}\$\$[ \t]*\n[\s\S]+?\n {0,3}\$\$[ \t]*$", prose)
        if kind == "formula" and display_math is None:
            self.fail("FORMULA_NOT_EDITABLE", identity, "The source formula must have a displayed LaTeX target.")
        if kind == "reference":
            anchors = [match[1] for match in _ANCHOR.finditer(prose)]
            if len(anchors) != 1:
                self.fail("REFERENCE_TARGET", identity, "A reference unit must resolve to exactly one bibliography anchor.")
        if kind == "section" and not re.search(r"(?m)^ {0,3}#{1,6}\s+\S", prose):
            self.fail("SECTION_TARGET", identity, "A section unit must resolve to an actual visible Markdown heading.")
        if kind != "metadata":
            return
        value = unit.get("value")
        hrefs = {unquote(link.href).lower() for link in scan_markdown_links(text) if not link.image}
        if unit.get("field") == "doi":
            if not isinstance(value, str) or not re.fullmatch(r"10\.\d{4,9}/\S+", value):
                self.fail("DOI_IDENTITY", identity, "Record the source DOI identifier without a URL prefix.")
            elif f"https://doi.org/{value}".lower() not in hrefs:
                self.fail("DOI_LINK", identity, "Deliver the source DOI as a canonical https://doi.org/ link.")
        if unit.get("field") == "email":
            if not isinstance(value, str) or not re.fullmatch(r"[^\s@]+@[^\s@]+\.[^\s@]+", value):
                self.fail("EMAIL_IDENTITY", identity, "Record the literal source email address.")
            elif f"mailto:{value}".lower() not in {href.split("?", 1)[0] for href in hrefs}:
                self.fail("EMAIL_LINK", identity, "Deliver the source address with a mailto target.")

    def check_figure(self, identity: str, delivery: dict, text: str):
        embedded = []
        for destination in _image_destinations(text):
            raw = unquote(destination)
            if re.match(r"[A-Za-z][A-Za-z0-9+.-]*:", raw):
                self.fail("FIGURE_LOCAL_ASSET", identity, "The delivered figure must resolve to a local asset.")
                continue
            path = self.resource_root.joinpath(raw).resolve()
            if not path.is_file():
                self.fail("FIGURE_ASSET_MISSING", identity, f"The embedded path does not resolve: {raw}")
            else:
                try:
                    if path.suffix.lower() == ".svg":
                        with fitz.open(path) as document:
                            if document.is_pdf or len(document) != 1:
                                raise ValueError("Expected one SVG image")
                            pixels = document[0].get_pixmap()
                    else:
                        pixels = fitz.Pixmap(path)
                    if pixels.width < 1 or pixels.height < 1:
                        raise ValueError("The decoded image is empty")
                except (fitz.mupdf.FzErrorBase, RuntimeError, ValueError, OSError) as error:
                    self.fail("FIGURE_IMAGE_DECODE", identity, f"The embedded asset is not a decodable image: {raw}: {error}")
            embedded.append(path)
        if not embedded:
            self.fail("FIGURE_EMBED_MISSING", identity, "The target must include an image embed, not only its caption.")
        assets = self.rows(delivery.get("assets"), identity, "assets")
        recorded = set()
        for record in assets:
            path = self.record(record, self.resource_root, identity)
            if path is not None:
                recorded.add(path)
        if set(embedded) != recorded:
            self.fail("FIGURE_ASSET_MAPPING", identity, "Hash exactly the local assets embedded by this figure target.")

    def check_table(self, identity: str, unit: dict, delivery: dict, text: str):
        match = re.search(r"(?m)^ {0,3}(`{3,}|~{3,})rich-table\s*\n", text)
        if match is None:
            self.fail("TABLE_NOT_EDITABLE", identity, "The source table requires a Rich Table data block.")
            return
        closing = re.search(r"(?m)^ {0,3}" + re.escape(match[1][0]) + "{" + str(len(match[1])) + r",}\s*$", text[match.end():])
        if closing is None:
            self.fail("TABLE_NOT_CLOSED", identity, "The target Rich Table fence is not closed.")
            return
        payload = note_checks._rich_table_payload(text[match.end():match.end() + closing.start()].splitlines())
        try:
            data = yaml.load(payload, Loader=note_checks._UniqueKeyLoader)
        except (yaml.YAMLError, ValueError, RecursionError):
            return  # The main note checker provides the precise YAML diagnostic.
        if not isinstance(data, dict):
            return
        block_start = self.targets[identity][0] + match.start()
        sheet_index = 0
        if data.get("version") == 3:
            sheets = data.get("sheets")
            selector = delivery.get("sheet")
            if not isinstance(sheets, list) or not isinstance(selector, dict):
                self.fail("TABLE_SHEET_TARGET", identity, "Select this source table's v3 worksheet by id or name.")
                return
            selected = [(index, sheet) for index, sheet in enumerate(sheets) if isinstance(sheet, dict) and len(selector) == 1 and
                        any(key in {"id", "name"} and sheet.get(key) == value for key, value in selector.items())]
            if len(selected) != 1:
                self.fail("TABLE_SHEET_TARGET", identity, "The worksheet selector must identify exactly one worksheet.")
                return
            sheet_index, data = selected[0]
        self.table_mappings.add((block_start, sheet_index))
        shape = unit.get("shape")
        if not isinstance(shape, dict):
            return
        for key in ("rows", "columns"):
            values = data.get(key)
            if isinstance(values, list) and len(values) != shape.get(key):
                self.fail("TABLE_" + key.upper(), identity, f"Source inventory expects {shape.get(key)} {key}; target has {len(values)}.")
        self.check_merges(identity, unit, data)

    def check_merges(self, identity: str, unit: dict | None, data: dict):
        """Compare source grid ranges with the plugin's rowId.colId ranges."""
        rows, columns = data.get("rows"), data.get("columns")
        if not isinstance(rows, list) or not isinstance(columns, list):
            return
        row_ids = [row.get("id") if isinstance(row, dict) else None for row in rows]
        column_ids = [column.get("id") if isinstance(column, dict) else None for column in columns]
        valid_rows = all(isinstance(key, str) and key and key != "header" for key in row_ids)
        valid_columns = all(isinstance(key, str) and key for key in column_ids)
        if not valid_rows or len(set(row_ids)) != len(row_ids):
            self.fail("TABLE_ROW_IDS", identity, "Every data row needs a unique nonempty ID other than the header sentinel.")
            return
        if not valid_columns or len(set(column_ids)) != len(column_ids):
            self.fail("TABLE_COLUMN_IDS", identity, "Every column needs a unique nonempty ID.")
            return
        for row in rows:
            cells = row.get("cells", {})
            if not isinstance(cells, dict):
                self.fail("TABLE_ROW_CELLS", identity, "Each row's cells must be a mapping keyed by column ID.")
                continue
            if any(key not in column_ids for key in cells):
                self.fail("TABLE_CELL_COLUMN", identity, "A cell refers to an unknown column ID in this worksheet.")
        row_index = {key: index + 1 for index, key in enumerate(row_ids)}
        row_index["header"] = 0
        column_index = {key: index + 1 for index, key in enumerate(column_ids)}

        def endpoint(value):
            if not isinstance(value, str):
                return None
            matches = [(row_index[value[:position]], column_index[value[position + 1:]])
                       for position, character in enumerate(value) if character == "."
                       and value[:position] in row_index and value[position + 1:] in column_index]
            return matches[0] if len(matches) == 1 else None

        actual_ranges = []
        actual = data.get("merges", [])
        if not isinstance(actual, list):
            self.fail("TABLE_MERGE_TYPE", identity, "merges must be an array of anchor/end objects.")
            return
        for merge in actual:
            if not isinstance(merge, dict):
                self.fail("TABLE_MERGE_ENDPOINT", identity, "A merge needs string anchor and end endpoints.")
                continue
            start, end = endpoint(merge.get("anchor")), endpoint(merge.get("end"))
            if start is None or end is None:
                self.fail("TABLE_MERGE_ENDPOINT", identity, "The merged-cell endpoint does not identify a cell in this worksheet.")
                continue
            if start[0] > end[0] or start[1] > end[1] or start == end or (start[0] == 0 and end[0] != 0):
                self.fail("TABLE_MERGE_RANGE", identity, "Use an ordered nontrivial rectangular range wholly in the header or data grid.")
                continue
            rectangle = (start[0], end[0], start[1], end[1])
            actual_ranges.append(rectangle)
            for row_number in range(start[0], end[0] + 1):
                for column_number in range(start[1], end[1] + 1):
                    if (row_number, column_number) == start:
                        continue
                    if row_number == 0:
                        value = columns[column_number - 1].get("name")
                    else:
                        cells = rows[row_number - 1].get("cells")
                        value = cells.get(column_ids[column_number - 1]) if isinstance(cells, dict) else None
                    if value is not None and (not isinstance(value, str) or value.strip()):
                        self.fail("TABLE_MERGE_COVERED_VALUE", identity, "Only the top-left anchor of a merged range may carry visible content.")
                        break
        ordered = sorted(actual_ranges)
        for index, left in enumerate(ordered):
            for right in ordered[index + 1:]:
                if right[0] > left[1]:
                    break
                if left[2] <= right[3] and right[2] <= left[3]:
                    self.fail("TABLE_MERGE_OVERLAP", identity, "Merged ranges must not overlap.")
        if unit is None:
            return
        expected_ranges = []
        expected = unit.get("merges")
        if not isinstance(expected, list):
            return
        for merge in expected:
            keys = ("row_start", "row_end", "column_start", "column_end")
            if not isinstance(merge, dict) or any(type(merge.get(key)) is not int for key in keys):
                self.fail("SOURCE_MERGE_RANGE", identity, "Source merges need integer row_start/row_end/column_start/column_end.")
                continue
            rectangle = tuple(merge[key] for key in keys)
            first_row, last_row, first_column, last_column = rectangle
            if not (0 <= first_row <= last_row <= len(rows) and 1 <= first_column <= last_column <= len(columns)):
                self.fail("SOURCE_MERGE_RANGE", identity, "A source merged range falls outside the recorded grid.")
                continue
            expected_ranges.append(rectangle)
        if sorted(actual_ranges) != sorted(expected_ranges):
            self.fail("TABLE_MERGES_MISMATCH", identity, f"Source merged ranges {sorted(expected_ranges)} differ from target ranges {sorted(actual_ranges)}.")

    def parse_table_blocks(self) -> list[dict]:
        result = []
        pattern = re.compile(r"(?m)^ {0,3}(`{3,}|~{3,})rich-table[ \t]*\n")
        for opening in pattern.finditer(self.table_visible):
            closing = re.search(r"(?m)^ {0,3}" + re.escape(opening[1][0]) + "{" + str(len(opening[1])) + r",}[ \t]*$", self.table_visible[opening.end():])
            if closing is None:
                continue
            body = self.table_visible[opening.end():opening.end() + closing.start()]
            payload = note_checks._rich_table_payload(body.splitlines())
            body_lines = body.splitlines(keepends=True)
            first, last = 0, len(body_lines)
            while first < last and not body_lines[first].strip():
                first += 1
            while last > first and not body_lines[last - 1].strip():
                last -= 1
            if last - first >= 2 and body_lines[first].strip() == body_lines[last - 1].strip() == "---":
                first += 1
            payload_start = opening.end() + sum(map(len, body_lines[:first]))
            try:
                data = yaml.load(payload, Loader=note_checks._UniqueKeyLoader)
                node = yaml.compose(payload, Loader=note_checks._UniqueKeyLoader)
            except (yaml.YAMLError, ValueError, RecursionError):
                continue
            if isinstance(data, dict):
                result.append({"start": opening.start(), "end": opening.end() + closing.end(), "data": data,
                               "fields": self.table_visible_fields(node, payload_start)})
        return result

    def table_visible_fields(self, root, offset: int) -> list[dict]:
        """Keep actual YAML scalar locations, not comments or encoded spellings."""
        def mapping(node):
            if not isinstance(node, yaml.MappingNode):
                return {}
            return {key.value: value for key, value in node.value if isinstance(key, yaml.ScalarNode)}

        def strings(node, ancestors=frozenset()):
            if node is None or id(node) in ancestors:
                return []
            if isinstance(node, yaml.ScalarNode):
                if node.tag == "tag:yaml.org,2002:str":
                    return [{"start": offset + node.start_mark.index, "end": offset + node.end_mark.index,
                             "value": node.value}]
                return []
            following = ancestors | {id(node)}
            if isinstance(node, yaml.SequenceNode):
                return [entry for value in node.value for entry in strings(value, following)]
            if isinstance(node, yaml.MappingNode):
                return [entry for _, value in node.value for entry in strings(value, following)]
            return []

        def sheet_fields(node):
            fields = mapping(node)
            result = strings(fields.get("title")) + strings(fields.get("footer"))
            columns = fields.get("columns")
            if isinstance(columns, yaml.SequenceNode):
                for column in columns.value:
                    result.extend(strings(mapping(column).get("name")))
            rows = fields.get("rows")
            if isinstance(rows, yaml.SequenceNode):
                for row in rows.value:
                    result.extend(strings(mapping(row).get("cells")))
            return result

        top = mapping(root)
        sheets = top.get("sheets")
        if not isinstance(sheets, yaml.SequenceNode):
            return sheet_fields(root)
        values = strings(top.get("title")) + strings(top.get("footer"))
        for sheet in sheets.value:
            values.extend(sheet_fields(sheet))
        return values

    def check_worksheet_coverage(self):
        for block in self.table_blocks:
            data = block["data"]
            sheets = data.get("sheets", []) if data.get("version") == 3 else [data]
            if not isinstance(sheets, list):
                continue
            for index, sheet in enumerate(sheets):
                if not isinstance(sheet, dict):
                    continue
                key = (block["start"], index)
                if key in self.table_mappings:
                    continue
                label = f"table-line-{self.text.count(chr(10), 0, block['start']) + 1}:sheet-{index + 1}"
                self.fail("TABLE_SHEET_UNMAPPED", label, "Each actual worksheet requires its own source table unit.")
                self.check_merges(label, None, sheet)

    def semantic_reference_links(self, start: int, end: int) -> list[tuple[str, int]]:
        raw_links = _reference_links(self.visible[start:end])
        for block in self.table_blocks:
            if not block["start"] <= start <= end <= block["end"]:
                continue
            fields = [entry for entry in block["fields"] if start < entry["end"] and entry["start"] < end]
            if not fields:
                return []
            if raw_links:
                return [(anchor, offset) for anchor, offset in raw_links
                        if any(entry["start"] <= start + offset < entry["end"] for entry in fields)]
            return [(anchor, max(entry["start"] - start, 0)) for entry in fields
                    for anchor, _ in _reference_links(entry["value"])]
        return raw_links

    def check_links(self):
        reference_anchors = {}
        citation_ranges = []
        for identity, (start, end, _) in self.targets.items():
            if self.units[identity].get("kind") != "reference":
                continue
            anchors = [match[1] for match in _ANCHOR.finditer(self.prose_visible[start:end])]
            if len(anchors) == 1:
                reference_anchors[identity] = anchors[0]
        for identity, unit in self.units.items():
            if unit.get("kind") not in {"citation", "cross_reference"} or identity not in self.targets:
                continue
            start, end, text = self.targets[identity]
            text = self.visible[start:end]
            actual = {target for target, _ in self.semantic_reference_links(start, end)}
            if unit.get("kind") == "cross_reference":
                destination = unit.get("target_unit")
                if destination not in self.targets:
                    self.fail("CROSS_REFERENCE_UNIT", identity, "Point to a delivered source object or section.")
                    continue
                target_start, target_end, _ = self.targets[destination]
                destination_text = self.prose_visible[target_start:target_end]
                anchors = {match[1] for match in _ANCHOR.finditer(destination_text)}
                headings = set(re.findall(r"(?m)^#{1,6}\s+(.+?)[ \t]*#*[ \t]*$", destination_text))
                heading_links = {unquote(value) for value in re.findall(r"\[\[#([^\]^|]+)(?:\|[^\]]*)?\]\]", text)}
                heading_links.update(unquote(link.href)[1:] for link in scan_markdown_links(text) if link.href.startswith("#") and not link.href.startswith("#^"))
                if not (anchors & actual or headings & heading_links):
                    self.fail("CROSS_REFERENCE_TARGET", identity, "The actual link does not reach the recorded source object or specific section.")
                continue
            references = unit.get("references")
            if not isinstance(references, list) or not references or any(not isinstance(value, str) for value in references):
                self.fail("CITATION_REFERENCES", identity, "List the source bibliography units for this citation occurrence.")
                continue
            unknown = set(references) - set(reference_anchors)
            if unknown:
                self.fail("CITATION_REFERENCE_TARGET", identity, f"Undelivered reference units: {sorted(unknown)}")
            expected = {reference_anchors[key] for key in references if key in reference_anchors}
            linked = {anchor for anchor in actual if note_checks._citation_preview_anchor(anchor) or anchor in reference_anchors.values()}
            if expected != linked:
                self.fail("CITATION_MAPPING", identity, f"Expected reference targets {sorted(expected)}; actual targets {sorted(linked)}.")
            for reference in references:
                if reference in self.units and self.units[reference].get("source") != unit.get("source") and not unit.get("cross_source_reason"):
                    self.fail("CITATION_SOURCE_SCOPE", identity, "Cross-source bibliography use needs an explicit source-based explanation.")
            citation_ranges.append((start, end))
        for anchor, offset in _reference_links(self.prose_visible):
            if (note_checks._citation_preview_anchor(anchor) or anchor in reference_anchors.values()) and not any(start <= offset < end for start, end in citation_ranges):
                self.fail("OUTPUT_CITATION_UNMAPPED", anchor, "This bibliography call has no source citation occurrence mapping.", line=self.text.count("\n", 0, offset) + 1)
        for block in self.table_blocks:
            actual = Counter(anchor for entry in block["fields"] for anchor, _ in _reference_links(entry["value"])
                             if note_checks._citation_preview_anchor(anchor) or anchor in reference_anchors.values())
            mapped = Counter(anchor for start, end in citation_ranges if block["start"] <= start and end <= block["end"]
                             for anchor, _ in self.semantic_reference_links(start, end)
                             if note_checks._citation_preview_anchor(anchor) or anchor in reference_anchors.values())
            if actual != mapped:
                self.fail("TABLE_CITATION_COVERAGE", f"table-line-{self.text.count(chr(10), 0, block['start']) + 1}",
                          f"Decoded visible table calls {dict(actual)} differ from mapped calls {dict(mapped)}.")

    def run(self) -> DeliveryReport:
        manifest = self.read_json(self.path, "manifest")
        if manifest.get("schema_version") != 1:
            self.fail("MANIFEST_VERSION", "manifest", "Expected schema_version 1.")
        inventory_path = self.record(manifest.get("inventory"), self.path.parent, "inventory")
        candidate_path = self.record(manifest.get("candidate"), self.path.parent, "candidate")
        if inventory_path is None or candidate_path is None:
            return self.report
        inventory = self.read_json(inventory_path, "inventory")
        if inventory.get("schema_version") != 1:
            self.fail("INVENTORY_VERSION", "inventory", "Expected schema_version 1.")
        self.candidate_path = candidate_path
        resource_root = manifest.get("resource_root", str(candidate_path.parent))
        if not isinstance(resource_root, str) or not resource_root.strip():
            self.fail("RESOURCE_ROOT", "manifest", "Provide the verified resource resolution root.")
            return self.report
        self.resource_root = Path(resource_root)
        if not self.resource_root.is_absolute():
            self.resource_root = self.path.parent.joinpath(self.resource_root)
        self.resource_root = self.resource_root.resolve()
        if not self.resource_root.is_dir():
            self.fail("RESOURCE_ROOT", "manifest", "The declared resource root is not an existing directory.")
            return self.report
        try:
            self.text = candidate_path.read_text(encoding="utf-8-sig").replace("\r\n", "\n").replace("\r", "\n")
        except (OSError, UnicodeError) as error:
            self.fail("CANDIDATE_READ", "candidate", str(error))
            return self.report
        for finding in note_checks.validate_note(self.text):
            self.report.findings.append(DeliveryFinding("NOTE_" + finding.code, "candidate", finding.message, finding.level, finding.line))
        self.check_sources(inventory, inventory_path.parent)
        self.check_inventory(inventory)
        self.check_deliveries(manifest)
        return self.report


def validate_delivery(manifest_path: Path, *, progress: Callable | None = None) -> DeliveryReport:
    """Return read-only machine findings; never grant source-content acceptance."""
    audit = _Audit(Path(manifest_path), progress)
    try:
        return audit.run()
    except (fitz.mupdf.FzErrorBase, OSError, UnicodeError, ValueError, TypeError, KeyError, RecursionError) as error:
        audit.fail("MANIFEST_OR_EVIDENCE_INVALID", "manifest", str(error))
        return audit.report


def main() -> int:
    if MANIFEST_PATH is None:
        print("Set MANIFEST_PATH at the top of this script or call validate_delivery(path).")
        return 2
    report = validate_delivery(MANIFEST_PATH, progress=source_tools.progress_bar)
    for finding in report.findings:
        print(f"{finding.level.upper()} [{finding.code}] {finding.object_id}: {finding.message}")
    print(f"Structural consistency: {report.structurally_consistent}; source fidelity: {report.source_fidelity}")
    if REPORT_PATH is not None:
        REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
        REPORT_PATH.write_text(json.dumps(report.to_dict(), ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return int(not report.structurally_consistent)


if __name__ == "__main__":
    raise SystemExit(main())
