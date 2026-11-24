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
        "and scientific meaning require comparison against the original files. "
        "This index validator alone does not check complete-workflow record presence."
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


def _visible_link_label(text: str) -> str:
    """Decode Markdown punctuation escapes and HTML entities in a link label."""
    unescaped = re.sub(r"\\([!\"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])", r"\1", text)
    return _normal(unescape(unescaped))


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
    translation_review_path: Path | None = None,
    figure_edge_reviews_required: bool = False,
) -> Path:
    """Bind completed files to a mapping. This does not invent missing targets."""
    if not inventory_path.is_file() or not candidate_path.is_file():
        raise FileNotFoundError("Create the inventory and final candidate before binding their hashes")
    if not isinstance(figure_edge_reviews_required, bool):
        raise ValueError("figure_edge_reviews_required must be an explicit boolean")
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
    if figure_edge_reviews_required:
        manifest["figure_edge_reviews_required"] = True
    inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
    if inventory.get("terminology_path"):
        terminology_path = Path(inventory["terminology_path"])
        if not terminology_path.is_absolute():
            terminology_path = inventory_path.parent.joinpath(terminology_path)
        manifest["terminology"] = source_tools.file_record(terminology_path)
    if "figure_reviews_path" in inventory:
        review_name = inventory["figure_reviews_path"]
        if not isinstance(review_name, str) or not review_name.strip():
            raise ValueError("figure_reviews_path must name the actual figure-review ledger")
        review_path = Path(review_name)
        if not review_path.is_absolute():
            review_path = inventory_path.parent.joinpath(review_path)
        manifest["figure_reviews"] = source_tools.file_record(review_path)
    if "checks_path" in inventory:
        checks_name = inventory["checks_path"]
        if not isinstance(checks_name, str) or not checks_name.strip():
            raise ValueError("checks_path must name the actual per-ID acceptance ledger")
        checks_path = Path(checks_name)
        if not checks_path.is_absolute():
            checks_path = inventory_path.parent.joinpath(checks_path)
        manifest["checks"] = source_tools.file_record(checks_path)
    if translation_review_path is not None:
        manifest["translation_review"] = source_tools.file_record(translation_review_path)
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
        self.figure_rects: dict[tuple[str, int], fitz.Rect] = {}
        self.image_xrefs: dict[tuple[str, int], set[int]] = {}
        self.figure_reviews_requested = False
        self.blocks: dict[str, tuple[str, int, dict]] = {}
        self.used_blocks: set[str] = set()
        self.used_pages: set[tuple[str, int]] = set()
        self.native_sources: dict[str, dict] = {}
        self.used_media_calls: set[tuple[str, str]] = set()
        self.units: dict[str, dict] = {}
        self.targets: dict[str, tuple[int, int, str]] = {}
        self.deliveries: dict[str, dict] = {}
        self.text = ""
        self.visible = ""
        self.prose_visible = ""
        self.metadata_visible = ""
        self.metadata_prose = ""
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
            if source.get("format") == "docx":
                self.check_native_source(identity, source, path, base)
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
                    self.figure_rects[key] = source_tools.pdf_page_rect(page)
                    if self.figure_reviews_requested:
                        self.image_xrefs[key] = {image[0] for image in page.get_images(full=True)}
                    text_path = self.record(row.get("text"), base, f"{identity}:page-{number}:text")
                    if text_path is not None and text_path.read_bytes().decode("utf-8") != page.get_text("text", sort=False):
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

    def check_native_source(self, identity: str, source: dict, path: Path, base: Path):
        try:
            actual = source_tools._docx_tools().read_docx(path, identity, progress=self.progress)
        except (OSError, ValueError) as error:
            self.fail("SOURCE_DOCX", identity, str(error))
            return
        for name in ("format", "page_count", "pages", "document_part", "relationships", "native_blocks", "media_calls", "native_issues"):
            if json.dumps(source.get(name), sort_keys=True, allow_nan=False) != json.dumps(actual[name], sort_keys=True, allow_nan=False):
                self.fail("NATIVE_PROVENANCE", identity + ":" + name, "Preserve the actual OOXML source inventory; pagination and image numbering are not inferred.")
        registered = self.index(self.rows(source.get("native_media"), identity, "native_media"), "id", identity + ":media")
        if list(registered) != [row["id"] for row in actual["native_media"]]:
            self.fail("NATIVE_MEDIA_SET", identity, "Retain every actual embedded media part once in source inventory order.")
        for media in actual["native_media"]:
            entry = registered.get(media["id"], {})
            if {key: entry.get(key) for key in media} != media:
                self.fail("NATIVE_MEDIA_PROVENANCE", media["id"], "Native media metadata must match the original DOCX part.")
            saved = self.record(entry.get("file"), base, media["id"])
            if saved is not None and self.hashes[saved] != media["sha256"]:
                self.fail("NATIVE_MEDIA_BYTES", media["id"], "Preserve the original embedded image bytes.")
            media["file"] = entry.get("file")
        document_path = self.record(source.get("native_document"), base, identity + ":native-document")
        if document_path is not None and self.hashes[document_path] != actual["document_part"]["sha256"]:
            self.fail("NATIVE_DOCUMENT_BYTES", identity, "Preserve the original document XML bytes.")
        text_path = self.record(source.get("native_text"), base, identity + ":native-text")
        expected_text = "".join(f"[{block['id']} | {block['kind']}]\n{block['text']}\n\n" for block in actual["native_blocks"])
        if text_path is not None and text_path.read_bytes() != expected_text.encode("utf-8"):
            self.fail("NATIVE_TEXT_BYTES", identity, "The native block text evidence differs from the original OOXML inventory.")
        self.native_sources[identity] = {**source, **actual}
        for block in actual["native_blocks"]:
            self.blocks[block["id"]] = (identity, None, block)
        reviews = self.index(self.rows(source.get("native_issue_reviews", []), identity, "native_issue_reviews"), "id", identity + ":native-reviews")
        issues = {row["id"]: row for row in actual["native_issues"]}
        for key in set(reviews) - set(issues):
            self.fail("NATIVE_ISSUE_UNKNOWN", key, "Review an actual native extraction issue.")
        for key in issues:
            review = reviews.get(key, {})
            if review.get("status") not in {"resolved", "not_applicable"}:
                self.fail("NATIVE_ISSUE_PENDING", key, "Unsupported source content requires explicit inspection before completion.")
                continue
            if not isinstance(review.get("reason"), str) or not review["reason"].strip():
                self.fail("NATIVE_ISSUE_REASON", key, "Record a source-specific reason for resolving this extraction issue.")
            evidence = self.rows(review.get("evidence"), key, "evidence")
            if not evidence:
                self.fail("NATIVE_ISSUE_EVIDENCE", key, "Bind the actually inspected source or rendered evidence.")
            for index, entry in enumerate(evidence):
                self.record(entry.get("file"), base, f"{key}:evidence-{index}")
                if not isinstance(entry.get("locator"), dict) or not entry["locator"]:
                    self.fail("NATIVE_ISSUE_LOCATOR", key, "Locate the actual evidence used to resolve this issue.")

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
            if self.sources.get(source, {}).get("format") == "docx":
                native = self.native_sources.get(source)
                if native is None:
                    self.fail("NATIVE_SOURCE_UNAVAILABLE", identity, "Native source provenance could not be verified.")
                    continue
                errors = source_tools._docx_tools().native_evidence_errors(native, row)
                for message in errors:
                    self.fail("NATIVE_EVIDENCE", identity, message)
                if errors:
                    continue
                self.used_blocks.update(row["blocks"])
                call = row.get("media_call")
                if call is not None:
                    if self.units.get(identity, {}).get("kind") != "figure":
                        self.fail("NATIVE_MEDIA_UNIT", identity, "Assign a native image call to an explicit figure unit.")
                    else:
                        self.used_media_calls.add((source, call))
                elif self.units.get(identity, {}).get("kind") == "figure":
                    self.fail("NATIVE_FIGURE_CALL", identity, "A native figure location needs its actual image relationship call.")
                continue
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
            image_xref = row.get("image_xref")
            native_locator = (self.units.get(identity, {}).get("kind") == "figure" and
                              type(image_xref) is int and image_xref in self.image_xrefs.get((source, page), set()))
            if not (isinstance(quote, str) and quote.strip()) and box is None and not native_locator:
                self.fail("SOURCE_LOCATOR", identity, "Provide a source quote or page-space bounding box.")
            if box is not None:
                valid = (isinstance(box, list) and len(box) == 4 and
                         all(type(value) in (int, float) and math.isfinite(value) for value in box))
                if not valid or not fitz.Rect(box).is_valid or fitz.Rect(box).is_empty:
                    self.fail("SOURCE_BBOX", identity, "Use a finite nonempty page-space rectangle.")
                elif not (self.figure_rects if row.get("coordinate_system") == source_tools.PDF_BBOX_COORDINATES
                          else self.rects)[(source, page)].contains(fitz.Rect(box)):
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
            if kind == "metadata" and unit.get("field") not in {*FIELDS.values(), "doi", "email", "header_note", "external_url"}:
                self.fail("METADATA_FIELD", identity, "Declare one supported metadata field.")
            if kind == "metadata" and unit.get("field") == "author":
                self.check_author_affiliations(identity, unit)
            if kind == "figure" and type(unit.get("contains_table")) is not bool:
                self.fail("FIGURE_TABLE_CLASSIFICATION", identity, "Inspect the figure and explicitly classify whether it contains a table.")
            if kind in {"figure", "table"} and self.sources[source].get("role") == "supplement":
                self.check_supplement_number(identity, unit)
            if kind == "table":
                shape = unit.get("shape")
                if not isinstance(shape, dict) or any(type(shape.get(key)) is not int or shape[key] < 1 for key in ("rows", "columns")):
                    self.fail("TABLE_SHAPE", identity, "Record the source table's positive row and column counts.")
                if not isinstance(unit.get("merges"), list):
                    self.fail("SOURCE_TABLE_MERGES", identity, "Record each source merged range; use an empty array only when the source has none.")
            if kind == "reference" and not str(unit.get("number", "")).strip():
                self.fail("REFERENCE_NUMBER", identity, "Retain the reference's source number or source label.")
            if kind == "reference" and (not isinstance(unit.get("bibliography_id"), str)
                                        or not unit["bibliography_id"].strip()):
                self.fail("BIBLIOGRAPHY_ID_MISSING", identity,
                          "Record this entry's evidence-based bibliography_id explicitly; an anchor prefix is not a bibliography identity.")
            if kind == "reference" and (not isinstance(unit.get("original_text"), str)
                                        or not unit["original_text"].strip()):
                self.fail("REFERENCE_ORIGINAL_TEXT", identity,
                          "Keep this complete original bibliography entry in original_text; a short locator quote or the formatted candidate is not the source record.")
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
        excluded_calls = {}
        calls = {(source_id, row["id"]): row for source_id, source in self.native_sources.items()
                 for row in source["media_calls"]}
        for row in self.rows(inventory.get("excluded_media_calls", []), "inventory", "excluded_media_calls"):
            key = (row.get("source"), row.get("media_call"))
            if key not in calls or not isinstance(row.get("reason"), str) or not row["reason"].strip():
                self.fail("NATIVE_MEDIA_EXCLUSION", str(key), "Exclude an actual media call only with a source-specific reason.")
                continue
            if key in excluded_calls or key in self.used_media_calls:
                self.fail("NATIVE_MEDIA_EXCLUSION_CONFLICT", str(key), "A call cannot be both delivered and excluded or excluded twice.")
            excluded_calls[key] = row["reason"]
        for key, call in calls.items():
            if key in self.used_media_calls or key in excluded_calls or call["block_id"] in excluded:
                continue
            self.fail("NATIVE_MEDIA_UNMAPPED", call["id"], "Map this actual image call to a figure or record an explicit exclusion; a file on disk alone is not delivery.")
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
        if not isinstance(inventory.get("sources"), list):
            return  # Already diagnosed by the source-schema checks.
        url_coverage = _load_sibling("source_url_inventory").source_url_coverage(
            inventory, excluded_blocks=excluded, excluded_pages=excluded_pages)
        for item in url_coverage["unmapped"]:
            self.fail("SOURCE_URL_UNMAPPED", item["id"],
                      f"Register this source URL occurrence as its own metadata/external_url (or DOI) unit: {item['raw_text']!r}. "
                      "Inspect wrapped text on the source page; source_url_occurrence can bind its exact occurrence ID.")
        for item in url_coverage["invalid_bindings"]:
            self.fail("SOURCE_URL_BINDING", item["unit"], item["reason"])

    def check_author_affiliations(self, identity: str, unit: dict):
        numbers, affiliations = unit.get("affiliation_numbers"), unit.get("affiliation_units")
        if not isinstance(numbers, list) or not isinstance(affiliations, list):
            self.fail("AUTHOR_AFFILIATION_MAPPING", identity,
                      "Record parallel affiliation_numbers and affiliation_units arrays for this author.")
            return
        if len(numbers) != len(affiliations):
            self.fail("AUTHOR_AFFILIATION_MAPPING", identity,
                      "Each source affiliation marker must map to one affiliation unit in the same position.")
            return
        if any(value is not None and (not isinstance(value, str) or not value.strip()) for value in numbers):
            self.fail("AUTHOR_AFFILIATION_NUMBER", identity,
                      "Preserve visible affiliation markers as strings; use null only for an unnumbered source relationship.")
        if not affiliations or None in numbers:
            reason = unit.get("affiliation_reason")
            if not isinstance(reason, str) or not reason.strip():
                self.fail("AUTHOR_AFFILIATION_REASON", identity,
                          "Explain the source-specific absent or unnumbered affiliation relationship.")
        seen = set()
        for affiliation_id in affiliations:
            if not isinstance(affiliation_id, str) or affiliation_id not in self.units:
                self.fail("AUTHOR_AFFILIATION_UNIT", identity,
                          "Every affiliation relationship must name an actual source unit.")
                continue
            affiliation = self.units[affiliation_id]
            if (affiliation.get("kind") != "metadata" or affiliation.get("field") != "affiliation"
                    or affiliation.get("source") != unit.get("source")):
                self.fail("AUTHOR_AFFILIATION_UNIT", identity,
                          "An author's affiliation must be an affiliation metadata unit from the same source.")
            if affiliation_id in seen:
                self.fail("AUTHOR_AFFILIATION_DUPLICATE", identity,
                          "Record each author-to-affiliation relationship once.")
            seen.add(affiliation_id)

    def check_supplement_number(self, identity: str, unit: dict):
        original = unit.get("original_number")
        if "original_number" not in unit or (original is not None and
                (not isinstance(original, str) or not original.strip())):
            self.fail("SUPPLEMENT_ORIGINAL_NUMBER", identity,
                      "Record the literal source number separately from the normalized display number.")
        if "original_number" in unit and original is None:
            reason = unit.get("original_number_reason")
            if not isinstance(reason, str) or not reason.strip():
                self.fail("SUPPLEMENT_NUMBER_ABSENCE", identity,
                          "Explain that this source object is unnumbered instead of inventing an original number.")
        if not isinstance(unit.get("display_number"), str) or not unit["display_number"].strip():
            self.fail("SUPPLEMENT_DISPLAY_NUMBER", identity,
                      "Record the delivery's display number independently from the source's literal number.")

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
        # Mask the complete text before slicing: delimiters may surround a target.
        self.metadata_visible = note_checks._mask_delimited(self.visible, "$")
        self.metadata_prose = note_checks._mask_delimited(self.prose_visible, "$")
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

    def figure_file_reference(self, record: Any, base: Path, identity: str) -> Path | None:
        if not isinstance(record, dict) or set(record) != {"path", "sha256"}:
            self.fail("FIG_REVIEW_FILE_REFERENCE", identity, "Keep a file reference as exactly path and sha256; store source locations separately.")
            return None
        return self.record(record, base, identity)

    def check_figure_review_locations(self, identity: str, unit: dict, row: dict, base: Path):
        locations = self.rows(row.get("source_locations"), identity, "source_locations")
        expected = self.rows(unit.get("evidence"), identity, "figure source evidence")
        if not locations or len(locations) != len(expected):
            self.fail("FIG_REVIEW_LOCATION_SET", identity, "Record every inventory figure location once, in its source order.")
        source = unit.get("source")
        if self.sources.get(source, {}).get("format") == "docx":
            native = self.native_sources.get(source)
            if native is None:
                self.fail("FIG_REVIEW_NATIVE_SOURCE", identity, "Verify the original DOCX provenance before reviewing its media.")
                return
            for index, location in enumerate(locations):
                label = f"{identity}:source-{index}"
                if index >= len(expected):
                    continue
                try:
                    original = source_tools._docx_tools().native_figure_location(native, expected[index])
                except (ValueError, KeyError, TypeError) as error:
                    self.fail("FIG_REVIEW_NATIVE_LOCATOR", label, str(error))
                    continue
                if location != original:
                    self.fail("FIG_REVIEW_LOCATION_MAPPING", label, "Retain the actual native image call, relationship and original media binding.")
                image = self.figure_file_reference(location.get("source_media"), base, label)
                actual = self.figure_file_reference(original["source_media"], self.inventory_base, label + ":original")
                if image is not None and image != actual:
                    self.fail("FIG_REVIEW_SOURCE_MEDIA", label, "Compare against the original embedded media of this image call.")
            return
        for index, location in enumerate(locations):
            label = f"{identity}:source-{index}"
            number = location.get("page")
            if not isinstance(source, str) or type(number) is not int or (source, number) not in self.pages:
                self.fail("FIG_REVIEW_PAGE", label, "Choose an actual page of this figure's own source.")
                continue
            key = (source, number)
            box, xref = location.get("bbox"), location.get("image_xref")
            if box is None and xref is None:
                self.fail("FIG_REVIEW_LOCATOR", label, "Record a figure bbox or an original image_xref.")
            if box is not None:
                if location.get("coordinate_system") != source_tools.PDF_BBOX_COORDINATES:
                    self.fail("FIG_REVIEW_COORDINATES", label, "Declare unrotated PDF points, a top-left origin, and right/down axes; pixel boxes are not PDF boxes.")
                if not source_tools.valid_pdf_bbox(box, self.figure_rects[key]):
                    self.fail("FIG_REVIEW_BBOX", label, "Use a finite nonempty PDF-point rectangle inside this source page.")
            if xref is not None and (type(xref) is not int or xref not in self.image_xrefs[key]):
                self.fail("FIG_REVIEW_XREF", label, "image_xref must be an image belonging to this source page.")
            if index < len(expected):
                original = expected[index]
                fields = ("page", "bbox", "coordinate_system", "image_xref")
                if any(location.get(field) != original.get(field) for field in fields):
                    self.fail("FIG_REVIEW_LOCATION_MAPPING", label, "The reviewed source locator differs from its inventory figure evidence.")
            image = self.figure_file_reference(location.get("page_render"), base, label)
            original_render = self.pages[key].get("render")
            if not isinstance(original_render, dict) or not isinstance(original_render.get("path"), str):
                self.fail("FIG_REVIEW_SOURCE_RENDER", label, "This source page lacks its recorded rendered evidence.")
                continue
            original_path = Path(original_render["path"])
            if not original_path.is_absolute():
                original_path = self.inventory_base.joinpath(original_path)
            if image is not None and (image != original_path.resolve() or location["page_render"].get("sha256") != original_render.get("sha256")):
                self.fail("FIG_REVIEW_SOURCE_RENDER", label, "Use the rendered image bound to this exact source page.")

    def check_figure_review_checks(self, identity: str, row: dict):
        entries = self.rows(row.get("checks"), identity, "checks")
        checks = self.index(entries, "id", identity)
        goals = source_tools.FIGURE_REVIEW_GOALS
        for missing in sorted(set(goals) - set(checks)):
            self.fail("FIG_REVIEW_CHECK_MISSING", f"{identity}:{missing}", "Retain one result for this atomic figure-review target.")
        for unknown in sorted(set(checks) - set(goals)):
            self.fail("FIG_REVIEW_CHECK_UNKNOWN", f"{identity}:{unknown}", "This is not a FIG-A01 through FIG-A28 target.")
        locations, assets = row.get("source_locations"), row.get("final_assets")
        source_kind = "source_location" if self.sources.get(self.units.get(identity, {}).get("source"), {}).get("format") == "docx" else "source_page"
        lengths = {source_kind: len(locations) if isinstance(locations, list) else 0,
                   "final_asset": len(assets) if isinstance(assets, list) else 0}
        for check_id, check in checks.items():
            if check_id not in goals:
                continue
            label = f"{identity}:{check_id}"
            if check.get("goal") != goals[check_id]:
                self.fail("FIG_REVIEW_GOAL", label, "Keep the atomic target text associated with this ID.")
            status = check.get("status")
            statuses = {"pending", "pass", "fail", "not_applicable", "blocked", "not_tested", "ambiguous"}
            if not isinstance(status, str) or status not in statuses:
                self.fail("FIG_REVIEW_STATUS", label, "Choose one supported review status.")
                continue
            if status in {"pending", "blocked", "not_tested", "ambiguous"}:
                self.fail("FIG_REVIEW_PENDING", label, f"The atomic review remains unresolved ({status}).")
            if status == "fail":
                self.fail("FIG_REVIEW_FAILED", label, "Resolve the failed figure target and review the final resource again.")
            reason = check.get("reason")
            if not isinstance(reason, str) or status != "pending" and not reason.strip():
                self.fail("FIG_REVIEW_REASON", label, "Record the source-specific reason for this atomic decision.")
            if status == "not_applicable":
                condition = check.get("applicability_condition")
                if not isinstance(condition, str) or not condition.strip():
                    self.fail("FIG_REVIEW_APPLICABILITY", label, "Explain the concrete source condition making this target inapplicable.")
            evidence = self.rows(check.get("evidence"), label, "evidence")
            if status != "pending" and not evidence:
                self.fail("FIG_REVIEW_EVIDENCE", label, "Locate the evidence for this atomic decision.")
            kinds = set()
            for pointer in evidence:
                kind, index = pointer.get("kind"), pointer.get("index")
                if not isinstance(kind, str) or kind not in lengths or type(index) is not int or not 0 <= index < lengths.get(kind, 0):
                    self.fail("FIG_REVIEW_EVIDENCE_TARGET", label, f"Evidence must point to an existing {source_kind} or final_asset by its zero-based index.")
                    continue
                observation = pointer.get("observation")
                if not isinstance(observation, str) or not observation.strip():
                    self.fail("FIG_REVIEW_OBSERVATION", label, "Record what was inspected at this evidence location.")
                kinds.add(kind)
            visual_target = int(check_id[-2:]) <= 18 or int(check_id[-2:]) >= 26
            if status == "pass" and visual_target and kinds != {source_kind, "final_asset"}:
                self.fail("FIG_REVIEW_COMPARISON", label, "A recorded visual pass needs both source-page and final-image evidence.")

    def check_figure_review_edges(self, identity: str, row: dict, base: Path):
        """Check explicit per-asset edge records, without certifying observation."""
        assets = self.rows(row.get("final_assets"), identity, "final_assets")
        locations = self.rows(row.get("source_locations"), identity, "source_locations")
        entries = self.rows(row.get("edge_reviews"), identity, "edge_reviews")
        expected = {(index, side) for index in range(len(assets)) for side in source_tools.FIGURE_EDGES}
        seen, covered_locations = set(), set()

        def bound_file(record, expected_record, label):
            actual = self.figure_file_reference(record, base, label)
            if not isinstance(expected_record, dict) or not isinstance(expected_record.get("path"), str):
                self.fail("FIG_REVIEW_EDGE_BINDING", label, "The parent figure lacks this file binding.")
                return
            expected_path = Path(expected_record["path"])
            if not expected_path.is_absolute():
                expected_path = base.joinpath(expected_path)
            stale_hash = isinstance(record, dict) and record.get("sha256") != expected_record.get("sha256")
            if stale_hash or (actual is not None and actual != expected_path.resolve()):
                self.fail("FIG_REVIEW_EDGE_BINDING", label, "This edge observation is bound to a different or superseded source/final file.")

        for position, entry in enumerate(entries):
            label = f"{identity}:edge-{position}"
            index, side = entry.get("asset_index"), entry.get("edge")
            if type(index) is not int or not isinstance(side, str) or (index, side) not in expected:
                self.fail("FIG_REVIEW_EDGE_IDENTITY", label, "Identify an actual final asset and exactly one of left/right/top/bottom.")
                continue
            key = (index, side)
            label = f"{identity}:asset-{index}:{side}"
            if key in seen:
                self.fail("FIG_REVIEW_EDGE_DUPLICATE", label, "Record each side of each final asset once.")
            seen.add(key)
            status = entry.get("status")
            if not isinstance(status, str) or status not in {"pending", "pass", "fail", "blocked", "not_tested", "ambiguous"}:
                self.fail("FIG_REVIEW_EDGE_STATUS", label, "Use an explicit edge status; every final image has four geometrical edges.")
            elif status == "fail":
                self.fail("FIG_REVIEW_EDGE_FAILED", label, "Repair and re-observe this failed edge before completion.")
            elif status != "pass":
                self.fail("FIG_REVIEW_EDGE_PENDING", label, "This final-image edge has no completed comparison result.")
            observation = entry.get("observation")
            if not isinstance(observation, str) or not observation.strip():
                self.fail("FIG_REVIEW_EDGE_OBSERVATION", label, "Record the actual source-page/final-image comparison for this edge.")
            bound_file(entry.get("source_file"), row.get("source_file"), label + ":source")
            bound_file(entry.get("final_asset"), assets[index], label + ":final")
            evidence = self.rows(entry.get("source_locations"), label, "source_locations")
            if not evidence:
                self.fail("FIG_REVIEW_EDGE_SOURCE", label, "Choose the actual complete source page(s) used to compare this asset's edge.")
            selected = set()
            for location in evidence:
                location_index = location.get("index")
                if type(location_index) is not int or not 0 <= location_index < len(locations):
                    self.fail("FIG_REVIEW_EDGE_SOURCE", label, "Choose a location index of this figure's own source_locations.")
                    continue
                if location_index in selected:
                    self.fail("FIG_REVIEW_EDGE_SOURCE", label, "Do not duplicate a source location within one edge observation.")
                selected.add(location_index)
                original = locations[location_index]
                file_key = "source_media" if "native_locator" in original else "page_render"
                supplied_locator = {key: value for key, value in location.items() if key not in {"index", file_key}}
                original_locator = {key: value for key, value in original.items() if key != file_key}
                if json.dumps(supplied_locator, sort_keys=True) != json.dumps(original_locator, sort_keys=True):
                    self.fail("FIG_REVIEW_EDGE_SOURCE", label, "Retain the exact page and source locator used for this edge comparison.")
                bound_file(location.get(file_key), original.get(file_key), label + f":source-{location_index}")
            covered_locations.update(selected)
        for index, side in sorted(expected - seen):
            self.fail("FIG_REVIEW_EDGE_MISSING", f"{identity}:asset-{index}:{side}", "Retain an explicit observation slot for each side of every final image.")
        if covered_locations != set(range(len(locations))):
            self.fail("FIG_REVIEW_EDGE_SOURCE_COVERAGE", identity, "Across this figure's assets, edge evidence must account for every registered source location; each asset chooses its own applicable pages.")

    def check_figure_reviews(self, inventory: dict, manifest: dict, base: Path):
        expected = inventory.get("figure_reviews_path")
        expected_figures = {identity: unit for identity, unit in self.units.items() if unit.get("kind") == "figure"}
        require_edges = manifest.get("figure_edge_reviews_required") is True
        if "figure_reviews_path" not in inventory and "figure_reviews" not in manifest:
            if require_edges and expected_figures:
                self.fail("FIG_REVIEW_REQUIRED", "figure_reviews", "This complete workflow requires a final figure ledger with per-asset edge observations.")
            return  # Older manifests remain supported; no visual acceptance is inferred.
        if not isinstance(expected, str) or not expected.strip():
            self.fail("FIG_REVIEW_PATH", "figure_reviews", "Record the figure-review ledger path in this inventory.")
            return
        expected_path = Path(expected)
        if not expected_path.is_absolute():
            expected_path = base.joinpath(expected_path)
        path = self.figure_file_reference(manifest.get("figure_reviews"), self.path.parent, "figure_reviews")
        if path is None:
            return
        if path != expected_path.resolve():
            self.fail("FIG_REVIEW_PATH", "figure_reviews", "Bind the ledger named by this source inventory.")
        ledger = self.read_json(path, "figure_reviews")
        version = ledger.get("schema_version")
        if type(version) is not int or version not in {1, 2}:
            self.fail("FIG_REVIEW_VERSION", "figure_reviews", "Expected figure-review schema_version 1 (legacy) or 2 (explicit final-image edges).")
        current_inventory = self.inventory_base.joinpath(self.inventory_name).resolve()
        inventory_name = ledger.get("inventory_path")
        if not isinstance(inventory_name, str) or not inventory_name.strip():
            self.fail("FIG_REVIEW_INVENTORY_PATH", "figure_reviews", "Record the actual live inventory path separately from its immutable snapshot.")
        else:
            recorded_path = Path(inventory_name)
            if not recorded_path.is_absolute():
                recorded_path = path.parent.joinpath(recorded_path)
            if recorded_path.resolve() != current_inventory:
                self.fail("FIG_REVIEW_INVENTORY_PATH", "figure_reviews", "The live inventory identity must match this delivery's inventory.")
        snapshot_path = self.figure_file_reference(ledger.get("inventory"), path.parent, "figure_reviews:inventory-snapshot")
        if snapshot_path is not None:
            if snapshot_path == current_inventory:
                self.fail("FIG_REVIEW_INVENTORY_SNAPSHOT", "figure_reviews", "Bind a separate immutable inventory snapshot, not the mutable live inventory.")
            snapshot = self.read_json(snapshot_path, "figure_reviews:inventory-snapshot")
            if type(snapshot.get("schema_version")) is not int or snapshot.get("schema_version") != 1:
                self.fail("FIG_REVIEW_SNAPSHOT_VERSION", "figure_reviews", "Expected source inventory snapshot schema_version 1.")
            tool_pointers = {"terminology_path", "figure_reviews_path", "checks_path"}
            current_content = {key: value for key, value in inventory.items() if key not in tool_pointers}
            snapshot_content = {key: value for key, value in snapshot.items() if key not in tool_pointers}
            # Canonical JSON preserves scalar types (True is not 1), array order,
            # and all nested fields while ignoring object-key formatting/order.
            if json.dumps(current_content, sort_keys=True, allow_nan=False) != json.dumps(snapshot_content, sort_keys=True, allow_nan=False):
                self.fail("FIG_REVIEW_INVENTORY_CHANGED", "figure_reviews", "Inventory content changed after figure review; only the three top-level tool-pointer fields may differ.")
        rows = self.rows(ledger.get("figures"), "figure_reviews", "figures")
        figures = self.index(rows, "unit", "figure_reviews")
        for missing in sorted(set(expected_figures) - set(figures)):
            self.fail("FIG_REVIEW_UNIT_MISSING", missing, "Record this inventory figure's atomic visual reviews.")
        for unknown in sorted(set(figures) - set(expected_figures)):
            self.fail("FIG_REVIEW_UNIT_UNKNOWN", unknown, "A figure review must correspond to an actual figure unit.")
        for identity, row in figures.items():
            if identity not in expected_figures:
                continue
            unit = expected_figures[identity]
            source = unit.get("source")
            if not isinstance(source, str) or source not in self.sources:
                self.fail("FIG_REVIEW_SOURCE", identity, "Choose this figure's own source ID.")
                continue
            if row.get("source") != source:
                self.fail("FIG_REVIEW_SOURCE", identity, "A reviewed figure cannot use another source's identity.")
            source_path = self.figure_file_reference(row.get("source_file"), path.parent, f"{identity}:source")
            original = self.sources[source]
            expected_source_path = self.record(original, base, f"{identity}:inventory-source")
            if source_path is not None and expected_source_path is not None and source_path != expected_source_path:
                self.fail("FIG_REVIEW_SOURCE_FILE", identity, "Use the source PDF bound to this figure's inventory unit.")
            self.check_figure_review_locations(identity, unit, row, path.parent)
            assets = self.rows(row.get("final_assets"), identity, "final_assets")
            if not assets:
                self.fail("FIG_REVIEW_ASSETS_EMPTY", identity, "Record the final image files reviewed for this figure.")
            recorded_assets = []
            for index, asset in enumerate(assets):
                asset_path = self.figure_file_reference(asset, path.parent, f"{identity}:asset-{index}")
                if asset_path is not None:
                    recorded_assets.append((asset_path, asset["sha256"]))
            if len(recorded_assets) != len(set(recorded_assets)):
                self.fail("FIG_REVIEW_ASSET_DUPLICATE", identity, "Record each final image once.")
            delivery = self.deliveries.get(identity, {})
            bound_assets = []
            for asset in self.rows(delivery.get("assets"), identity, "delivery assets"):
                asset_path = self.figure_file_reference(asset, self.resource_root, f"{identity}:delivery-asset")
                if asset_path is not None:
                    bound_assets.append((asset_path, asset["sha256"]))
            if recorded_assets != bound_assets:
                self.fail("FIG_REVIEW_ASSET_MAPPING", identity, "The reviewed final-image paths and hashes must match the delivery assets in order.")
            self.check_figure_review_checks(identity, row)
            if require_edges or version == 2 or "edge_reviews" in row:
                self.check_figure_review_edges(identity, row, path.parent)

    def check_terminology(self, inventory: dict, manifest: dict, base: Path):
        expected = inventory.get("terminology_path")
        if expected is None and "terminology" not in manifest:
            return
        if not isinstance(expected, str) or not expected.strip():
            self.fail("TERMINOLOGY_PATH", "terminology", "Record the terminology ledger path in the source inventory.")
            return
        expected_path = Path(expected)
        if not expected_path.is_absolute():
            expected_path = base.joinpath(expected_path)
        path = self.record(manifest.get("terminology"), self.path.parent, "terminology")
        if path is None:
            return
        if path.resolve() != expected_path.resolve():
            self.fail("TERMINOLOGY_PATH", "terminology", "Bind the ledger named by this source inventory.")
        ledger = self.read_json(path, "terminology")
        if type(ledger.get("schema_version")) is not int or ledger["schema_version"] != 1:
            self.fail("TERMINOLOGY_VERSION", "terminology", "Expected terminology schema_version 1.")
        self.record(ledger.get("glossary"), path.parent, "terminology:glossary")
        status = ledger.get("review_status")
        if status == "not_applicable":
            if not isinstance(ledger.get("reason"), str) or not ledger["reason"].strip():
                self.fail("TERMINOLOGY_APPLICABILITY", "terminology", "Explain why terminology review does not apply to this task.")
            return
        if status != "reviewed":
            self.fail("TERMINOLOGY_PENDING", "terminology", "Complete the terminology ledger after source review.")
        expected_units = {identity for identity, unit in self.units.items()
                          if unit.get("kind") in {"paragraph", "section", "figure", "table"}
                          or (unit.get("kind") == "metadata" and unit.get("field") in {"title", "header_note"})}
        covered = ledger.get("covered_units")
        if (not isinstance(covered, list) or any(not isinstance(value, str) for value in covered)
                or set(covered) != expected_units or len(covered) != len(set(covered))):
            self.fail("TERMINOLOGY_COVERAGE", "terminology", "List each applicable source unit once in the terminology review coverage.")
        for category in ("adopted", "unlisted"):
            entries = self.rows(ledger.get(category), "terminology", category)
            empty_reason = ledger.get("no_unlisted_reason")
            if category == "unlisted" and not entries and (not isinstance(empty_reason, str) or not empty_reason.strip()):
                self.fail("TERMINOLOGY_EMPTY_REASON", "terminology", "Explain a reviewed empty unlisted-term set; empty does not mean reviewed.")
            for index, entry in enumerate(entries, 1):
                identity = f"terminology:{category}:{index}"
                fields = ["original", "translation", "context"]
                fields.append("glossary_locator" if category == "adopted" else "reason")
                for field in fields:
                    if not isinstance(entry.get(field), str) or not entry[field].strip():
                        self.fail("TERM_" + field.upper(), identity, f"Record this term's {field}.")
                source_units = entry.get("source_units")
                if (not isinstance(source_units, list) or not source_units
                        or any(not isinstance(value, str) or value not in self.units for value in source_units)):
                    self.fail("TERM_SOURCE_UNITS", identity, "Point to actual source units containing this term.")
                if category != "unlisted":
                    continue
                if entry.get("status") != "resolved":
                    self.fail("TERM_UNRESOLVED", identity, "The unlisted term still needs a source-based translation decision.")
                if not isinstance(entry.get("resolution_basis"), str) or not entry["resolution_basis"].strip():
                    self.fail("TERM_RESOLUTION_BASIS", identity, "Record the evidence used to resolve the proposed translation.")

    def check_translation_review(self, manifest: dict, inventory_path: Path):
        """Check immutable file bindings; never grade a translation ledger's prose."""
        if "translation_review" not in manifest:
            return
        path = self.record(manifest["translation_review"], self.path.parent, "translation_review")
        if path is None:
            return
        ledger = self.read_json(path, "translation_review")
        if ledger.get("schema_version") != 1:
            self.fail("TRN_REVIEW_VERSION", "translation_review", "Expected translation-review schema_version 1.")
        bindings = ledger.get("bindings")
        if not isinstance(bindings, dict):
            self.fail("TRN_REVIEW_BINDINGS", "translation_review", "Bind the actual inventory and final candidate files.")
            return
        for key, expected in (("source_inventory", inventory_path), ("candidate", self.candidate_path)):
            bound = self.record(bindings.get(key), path.parent, "translation_review:" + key)
            if bound is not None and bound != expected.resolve():
                self.fail("TRN_REVIEW_FILE_IDENTITY", key, "Bind this delivery's actual file, not a different copy.")
        source_bindings = bindings.get("sources")
        if not isinstance(source_bindings, dict):
            self.fail("TRN_REVIEW_SOURCES", "translation_review", "Bind every source used in the selected review units.")
            return
        units = ledger.get("units")
        if not isinstance(units, dict) or not units:
            self.fail("TRN_REVIEW_UNITS", "translation_review", "Select the source units reviewed in this translation ledger.")
            return
        required_sources = set()
        for identity, row in units.items():
            if identity not in self.units or not isinstance(row, dict):
                self.fail("TRN_REVIEW_UNIT", str(identity), "Every review unit must name an actual inventory unit.")
                continue
            source_id = self.units[identity].get("source")
            if row.get("source_id") != source_id:
                self.fail("TRN_REVIEW_UNIT_SOURCE", identity, "Keep the inventory source identity for this review unit.")
            required_sources.add(source_id)
        if set(source_bindings) != required_sources:
            self.fail("TRN_REVIEW_SOURCES", "translation_review", "The bound sources must match the selected units' sources.")
        for source_id, binding in source_bindings.items():
            bound = self.record(binding, path.parent, "translation_review:source:" + source_id)
            source = self.sources.get(source_id)
            if source is None:
                self.fail("TRN_REVIEW_SOURCE_IDENTITY", source_id, "This source does not belong to the inventory.")
                continue
            expected = Path(source["path"])
            if not expected.is_absolute():
                expected = inventory_path.parent.joinpath(expected)
            if bound is not None and bound != expected.resolve():
                self.fail("TRN_REVIEW_SOURCE_IDENTITY", source_id, "Bind the actual source PDF used by this inventory.")
        if bindings.get("glossary") is not None:
            self.record(bindings["glossary"], path.parent, "translation_review:glossary")
        if ledger.get("terminology_record") is not None:
            self.record(ledger["terminology_record"], path.parent, "translation_review:terminology")

    def check_acceptance_records(self, inventory: dict, manifest: dict, base: Path):
        expected = inventory.get("checks_path")
        if expected is None and "checks" not in manifest:
            return
        if not isinstance(expected, str) or not expected.strip():
            self.fail("CHECK_LEDGER_PATH", "checks", "Record the actual per-ID ledger path in the source inventory.")
            return
        expected_path = Path(expected)
        if not expected_path.is_absolute():
            expected_path = base.joinpath(expected_path)
        path = self.record(manifest.get("checks"), self.path.parent, "checks")
        if path is None:
            return
        if path.resolve() != expected_path.resolve():
            self.fail("CHECK_LEDGER_PATH", "checks", "Bind the per-ID ledger named by this inventory.")
        ledger = self.read_json(path, "checks")
        checker = _load_sibling("check_records")
        result = checker.validate_check_records(path, progress=self.progress)
        for item in result.findings:
            self.report.findings.append(DeliveryFinding(item.code, item.object_id, item.message, item.level))
        for identity in result.offline_unresolved_ids:
            self.fail("CHECK_OFFLINE_INCOMPLETE", identity,
                      "This offline acceptance record is unresolved or failed; complete its source comparison before claiming an offline-ready delivery.")
        for identity in result.unclassified_ids:
            self.fail("CHECK_BOUNDARY_UNCLASSIFIED", identity,
                      "Classify this check's actual evidence boundary; an unclassified record cannot be excluded from offline completion.")
        bindings = ledger.get("bindings")
        if not isinstance(bindings, dict):
            return  # The ledger checker already diagnoses the missing structure.

        def identities(rows: list[dict], root: Path, *, include_id: bool) -> set[tuple]:
            values = set()
            for row in rows:
                if not isinstance(row.get("path"), str) or not isinstance(row.get("sha256"), str):
                    continue
                if include_id and not isinstance(row.get("id"), str):
                    continue
                file_path = Path(row["path"])
                if not file_path.is_absolute():
                    file_path = root.joinpath(file_path)
                identity = (str(file_path.resolve()), row["sha256"])
                values.add((row.get("id"), *identity) if include_id else identity)
            return values

        source_rows = self.rows(bindings.get("sources"), "checks", "sources")
        expected_sources = identities(list(self.sources.values()), base, include_id=True)
        if identities(source_rows, path.parent, include_id=True) != expected_sources:
            self.fail("CHECK_SOURCE_BINDING", "checks", "The ledger must bind exactly the source PDFs in this inventory.")
        candidates = self.rows(bindings.get("candidates"), "checks", "candidates")
        expected_candidate = identities([manifest["candidate"]], self.path.parent, include_id=False)
        if not expected_candidate.issubset(identities(candidates, path.parent, include_id=False)):
            self.fail("CHECK_CANDIDATE_BINDING", "checks", "The ledger must include this final candidate with its current hash.")

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
        metadata = self.metadata_visible[start:end]
        if unit.get("field") == "header_note":
            line_start = self.text.rfind("\n", 0, start) + 1
            lines = [line for line in self.metadata_prose[line_start:end].splitlines() if line.strip()]
            if not lines or any(not re.match(r"^ {0,3}>[ \t]?", line) for line in lines):
                self.fail("HEADER_NOTE_LAYOUT", identity, "Deliver this source header note inside its source-specific metadata blockquote.")
        value = unit.get("value")
        if unit.get("field") == "external_url":
            if not isinstance(value, str) or not re.match(r"https?://\S+$", value, re.IGNORECASE):
                self.fail("URL_IDENTITY", identity, "Record the full source URL after resolving source line wrapping.")
            else:
                matches = [link for link in scan_markdown_links(metadata) if not link.image and link.href == value]
                if not matches:
                    self.fail("URL_LINK", identity, "Deliver the recorded source URL with its original target.")
                display = unit.get("display_text")
                if not isinstance(display, str) or not display.strip():
                    self.fail("URL_SOURCE_LABEL", identity, "Record the source's original visible link text.")
                elif matches and not any(_visible_link_label(link.label) == _normal(display) for link in matches):
                    self.fail("URL_DISPLAY_TEXT", identity, "Keep the source URL's original visible text instead of substituting a new label.")
        hrefs = {unquote(link.href).lower() for link in scan_markdown_links(metadata) if not link.image}
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
        self.inventory_base, self.inventory_name = inventory_path.parent, inventory_path.name
        self.figure_reviews_requested = "figure_reviews_path" in inventory or "figure_reviews" in manifest
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
        self.check_terminology(inventory, manifest, inventory_path.parent)
        self.check_deliveries(manifest)
        self.check_figure_reviews(inventory, manifest, inventory_path.parent)
        self.check_translation_review(manifest, inventory_path)
        self.check_acceptance_records(inventory, manifest, inventory_path.parent)
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
