"""Read native DOCX source order and relationships without inventing pages.

Call ``read_docx`` to inspect metadata or ``prepare_docx_source`` to preserve
the actual XML and embedded media. Image relationship calls are not figure
numbers. Unsupported display objects remain explicit review items.
"""
from __future__ import annotations

from copy import deepcopy
from contextlib import contextmanager
import hashlib
from pathlib import Path, PurePosixPath
import re
from typing import Callable, Mapping
from urllib.parse import unquote, urlsplit
from xml.etree import ElementTree as ET
from zipfile import BadZipFile, ZipFile


Progress = Callable[[int, int, str], None]
DOCUMENT_PART = "word/document.xml"
RELATIONSHIPS_PART = "word/_rels/document.xml.rels"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
M = "http://schemas.openxmlformats.org/officeDocument/2006/math"
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
V = "urn:schemas-microsoft-com:vml"
UNSUPPORTED = {
    "oMath": "math", "oMathPara": "math", "anchor": "floating_object",
    "txbxContent": "text_box", "object": "embedded_object", "chart": "chart",
    "altChunk": "alternate_content", "footnoteReference": "footnote",
    "endnoteReference": "endnote", "del": "tracked_change", "ins": "tracked_change",
    "moveFrom": "tracked_change", "moveTo": "tracked_change",
    "fldChar": "field", "instrText": "field", "sym": "symbol",
    "fldSimple": "field", "numPr": "generated_list_number",
    "AlternateContent": "alternate_content", "pict": "legacy_drawing",
}


def _local(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _xml(payload: bytes, name: str):
    if re.search(br"<!\s*(?:DOCTYPE|ENTITY)\b", payload, re.I):
        raise ValueError(f"{name}: DTD/entity declarations are not supported")
    try:
        return ET.fromstring(payload)
    except ET.ParseError as error:
        raise ValueError(f"{name}: invalid OOXML: {error}") from error


@contextmanager
def _archive(path: Path):
    try:
        with ZipFile(path) as archive:
            yield archive
    except (BadZipFile, KeyError, RuntimeError) as error:
        raise ValueError(f"An accessible DOCX package is required: {error}") from error


def _digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def _file_record(path: Path) -> dict:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return {"path": str(path.resolve()), "sha256": result.hexdigest()}


def _locator(path: tuple[int, ...]) -> dict:
    return {"part": DOCUMENT_PART, "element_path": list(path)}


def valid_native_locator(value) -> bool:
    if not isinstance(value, Mapping) or set(value) != {"part", "element_path"}:
        return False
    indices = value.get("element_path")
    return (value.get("part") == DOCUMENT_PART and isinstance(indices, list)
            and all(type(index) is int and index >= 0 for index in indices))


def _target_part(target: str) -> str | None:
    """Resolve OPC relative paths without allowing traversal outside the ZIP."""
    parsed = urlsplit(target)
    if parsed.scheme or parsed.netloc or parsed.query or parsed.fragment or "\\" in target:
        return None
    raw = unquote(parsed.path)
    if "\x00" in raw:
        return None
    parts = [] if raw.startswith("/") else ["word"]
    for item in raw.split("/"):
        if item in {"", "."}:
            continue
        if item == "..":
            if not parts:
                return None
            parts.pop()
            continue
        parts.append(item)
    return "/".join(parts)


def _walk(element, path=()):
    yield element, path
    for index, child in enumerate(element):
        yield from _walk(child, (*path, index))


def _has_image_effect(element) -> bool:
    metadata = {f"{{{A}}}extLst", f"{{{A}}}ext",
                "{http://schemas.microsoft.com/office/drawing/2010/main}useLocalDpi"}
    return any(child.tag not in metadata for child in element.iter() if child is not element)


def read_docx(path: Path, source_id: str, *, progress: Progress | None = None) -> dict:
    """Return deterministic native provenance; no semantic outcomes are supplied.

    ``element_path`` is a zero-based child-index path from the unmodified XML
    document root. ``native_blocks`` follows body order, with one table block
    retaining its rows/cells. Empty paragraphs containing media are retained.
    The document has no asserted page count or page-space geometry.
    """
    path = Path(path).resolve(strict=True)
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", source_id):
        raise ValueError("A stable task-local source ID is required")
    with _archive(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("DOCX contains duplicate ZIP member names")
        if DOCUMENT_PART not in names or "[Content_Types].xml" not in names:
            raise ValueError("An accessible DOCX WordprocessingML package is required")
        document_bytes = archive.read(DOCUMENT_PART)
        document = _xml(document_bytes, DOCUMENT_PART)
        if document.tag != f"{{{W}}}document":
            raise ValueError("Only transitional WordprocessingML documents are supported by this adapter")
        body_entries = [(node, (index,)) for index, node in enumerate(document) if node.tag == f"{{{W}}}body"]
        if len(body_entries) != 1:
            raise ValueError("DOCX must have exactly one document body")
        body, body_path = body_entries[0]
        types = _xml(archive.read("[Content_Types].xml"), "[Content_Types].xml")
        defaults = {node.get("Extension", "").lower(): node.get("ContentType", "")
                    for node in types if _local(node.tag) == "Default"}
        overrides = {node.get("PartName", "").lstrip("/"): node.get("ContentType", "")
                     for node in types if _local(node.tag) == "Override"}
        relationships = []
        if RELATIONSHIPS_PART in names:
            for node in _xml(archive.read(RELATIONSHIPS_PART), RELATIONSHIPS_PART):
                if _local(node.tag) != "Relationship":
                    continue
                relationship = {key: node.get(key, "") for key in ("Id", "Type", "Target", "TargetMode")}
                relationship["resolved_part"] = None if relationship["TargetMode"] == "External" else _target_part(relationship["Target"])
                relationships.append(relationship)
        relation_ids = [row["Id"] for row in relationships]
        if any(not identity for identity in relation_ids) or len(set(relation_ids)) != len(relation_ids):
            raise ValueError("DOCX document relationships require unique nonempty IDs")
        relation_map = {row["Id"]: row for row in relationships}
        media_parts = sorted(name for name in names if not name.endswith("/") and
                             (name.startswith("word/media/") or overrides.get(name, defaults.get(PurePosixPath(name).suffix.lstrip(".").lower(), "")).startswith("image/")))
        media = []
        for index, part in enumerate(media_parts, 1):
            payload = archive.read(part)
            media.append({"id": f"{source_id}-media-{index}", "part": part,
                          "sha256": _digest(payload), "bytes": len(payload),
                          "content_type": overrides.get(part, defaults.get(PurePosixPath(part).suffix.lstrip(".").lower(), "application/octet-stream"))})
        media_by_part = {row["part"]: row for row in media}
        calls, blocks, issues = [], [], []

        def issue(kind, locator, *, detail, block_id=None):
            row = {"id": f"{source_id}-native-issue-{len(issues) + 1}", "kind": kind,
                   "locator": deepcopy(locator), "detail": detail, "status": "pending"}
            if block_id is not None:
                row["block_id"] = block_id
            issues.append(row)
            return row["id"]

        def paragraph(node, node_path, block_id):
            chunks, events, own_calls = [], [], []

            def visit(current, position):
                local = _local(current.tag)
                locator = _locator(position)
                if local in UNSUPPORTED:
                    pending = issue(UNSUPPORTED[local], locator, block_id=block_id,
                                    detail=f"Inspect original OOXML {local}; no reading order or rendered meaning is inferred.")
                    events.append({"kind": "pending_object", "issue": pending, "locator": locator,
                                   "xml": ET.tostring(current, encoding="unicode")})
                    # Retain embedded-image relationship calls even in a floating
                    # container, while its actual placement remains unresolved.
                    if local not in {"anchor", "pict"}:
                        return
                if current.tag == f"{{{W}}}t":
                    value = current.text or ""
                    chunks.append(value)
                    events.append({"kind": "text", "text": value, "locator": locator})
                    return
                if current.tag == f"{{{W}}}hyperlink":
                    relation_id = current.get(f"{{{R}}}id")
                    relationship = relation_map.get(relation_id)
                    events.append({"kind": "hyperlink", "locator": locator,
                                   "relationship_id": relation_id, "anchor": current.get(f"{{{W}}}anchor"),
                                   "target": relationship.get("Target") if relationship else None})
                    issue("hyperlink_target", locator, block_id=block_id,
                          detail="Inspect this source hyperlink target as well as its visible text; the target is not inferred from that text.")
                if current.tag in {f"{{{W}}}tab", f"{{{W}}}br", f"{{{W}}}cr"}:
                    value = "\t" if local == "tab" else "\n"
                    chunks.append(value)
                    events.append({"kind": local, "text": value, "locator": locator})
                    return
                if current.tag in {f"{{{A}}}blip", f"{{{V}}}imagedata"}:
                    relation_id = current.get(f"{{{R}}}embed") or current.get(f"{{{R}}}link") or current.get(f"{{{R}}}id")
                    relation = relation_map.get(relation_id)
                    call = {"id": f"{source_id}-image-call-{len(calls) + 1}", "block_id": block_id,
                            "locator": locator, "relationship_id": relation_id,
                            "media_id": None, "status": "resolved", "figure_number": None}
                    if relation is None:
                        call["status"] = "missing_relationship"
                    elif relation["TargetMode"] == "External":
                        call.update(status="external_relationship", external_target=relation["Target"])
                    elif not relation["Type"].endswith("/image"):
                        call["status"] = "non_image_relationship"
                    elif relation["resolved_part"] not in media_by_part:
                        call.update(status="missing_media", target_part=relation["resolved_part"])
                    else:
                        call["media_id"] = media_by_part[relation["resolved_part"]]["id"]
                    calls.append(call)
                    own_calls.append(call["id"])
                    events.append({"kind": "image", "media_call": call["id"], "locator": locator})
                    if call["status"] != "resolved":
                        issue(call["status"], locator, block_id=block_id,
                              detail=f"Image call {call['id']} cannot be resolved to embedded bytes; no external content was fetched.")
                    if _has_image_effect(current):
                        issue("image_effect", locator, block_id=block_id,
                              detail="DrawingML image effects require a visible-source comparison.")
                    return
                if local == "srcRect" and any(value not in {"0", "0.0"} for value in current.attrib.values()):
                    issue("image_crop", locator, block_id=block_id,
                          detail="OOXML applies an image crop; native media bytes alone do not reproduce that visible crop.")
                if local == "xfrm" and any(name in current.attrib for name in ("rot", "flipH", "flipV")):
                    issue("image_transform", locator, block_id=block_id,
                          detail="OOXML applies rotation or reflection; compare a rendered source before delivering pixels.")
                for index, child in enumerate(current):
                    visit(child, (*position, index))

            visit(node, node_path)
            style = node.find(f"{{{W}}}pPr/{{{W}}}pStyle")
            return {"text": "".join(chunks), "inline_content": events, "media_calls": own_calls,
                    "style_id": style.get(f"{{{W}}}val") if style is not None else None}

        def append_block(node, position):
            local = _local(node.tag)
            if node.tag in {f"{{{W}}}{name}" for name in ("sectPr", "bookmarkStart", "bookmarkEnd", "proofErr")}:
                return
            block_id = f"{source_id}-docx-b{len(blocks) + 1}"
            base = {"id": block_id, "kind": local, "order": len(blocks), "locator": _locator(position)}
            if node.tag == f"{{{W}}}p":
                base.update(kind="paragraph", **paragraph(node, position, block_id))
                blocks.append(base)
                return
            if node.tag == f"{{{W}}}tbl":
                rows, table_calls = [], []
                for row_index, row in enumerate(node):
                    if row.tag != f"{{{W}}}tr":
                        continue
                    cells = []
                    for cell_index, cell in enumerate(row):
                        if cell.tag != f"{{{W}}}tc":
                            continue
                        cell_path = (*position, row_index, cell_index)
                        paragraphs = []
                        for child_index, child in enumerate(cell):
                            if child.tag == f"{{{W}}}p":
                                item = paragraph(child, (*cell_path, child_index), block_id)
                                paragraphs.append({"locator": _locator((*cell_path, child_index)), **item})
                                table_calls.extend(item["media_calls"])
                            elif child.tag == f"{{{W}}}tbl":
                                issue("nested_table", _locator((*cell_path, child_index)), block_id=block_id,
                                      detail="Nested table content requires explicit inspection; it is not flattened into an inferred cell layout.")
                        span = cell.find(f"{{{W}}}tcPr/{{{W}}}gridSpan")
                        merge = cell.find(f"{{{W}}}tcPr/{{{W}}}vMerge")
                        cells.append({"locator": _locator(cell_path), "paragraphs": paragraphs,
                                      "text": "\n".join(item["text"] for item in paragraphs),
                                      "grid_span": span.get(f"{{{W}}}val") if span is not None else "1",
                                      "vertical_merge": (merge.get(f"{{{W}}}val", "continue") if merge is not None else None)})
                    rows.append({"cells": cells})
                base.update(kind="table", rows=rows, media_calls=table_calls,
                            text="\n".join("\t".join(cell["text"] for cell in row["cells"]) for row in rows))
                blocks.append(base)
                return
            issue("body_container", _locator(position), block_id=block_id,
                  detail=f"Unsupported body container {local}; inspect original XML before deciding reading order.")
            base.update(kind="unsupported", text="", media_calls=[], xml=ET.tostring(node, encoding="unicode"))
            blocks.append(base)

        for index, node in enumerate(body):
            append_block(node, (*body_path, index))
            if progress:
                progress(index + 1, len(body), f"{source_id} native block {index + 1}")
        block_by_id = {row["id"]: row for row in blocks}
        before, after = [], [""] * len(blocks)
        latest = ""
        for block in blocks:
            before.append(latest)
            if block["text"].strip():
                latest = block["text"]
        latest = ""
        for index in range(len(blocks) - 1, -1, -1):
            after[index] = latest
            if blocks[index]["text"].strip():
                latest = blocks[index]["text"]
        for call in calls:
            order = block_by_id[call["block_id"]]["order"]
            call["nearby_text"] = {
                "same_block": block_by_id[call["block_id"]]["text"],
                "previous": before[order],
                "next": after[order],
            }
        used_media = {call["media_id"] for call in calls if call["media_id"] is not None}
        for item in media:
            if item["id"] not in used_media:
                issue("unresolved_media_use", {"part": item["part"]},
                      detail="This embedded media part has no resolved body image call; inspect headers, footers, drawings or unused package content.")
        for relationship in relationships:
            role = relationship["Type"].rsplit("/", 1)[-1]
            if role in {"header", "footer", "footnotes", "endnotes", "comments", "oleObject", "chart", "aFChunk"}:
                issue("additional_part", {"part": relationship["resolved_part"]},
                      detail=f"Inspect related {role} part before declaring source coverage; relationship {relationship['Id']}.")
        return {
            "format": "docx", "page_count": None, "pages": [],
            "document_part": {"part": DOCUMENT_PART, "sha256": _digest(document_bytes)},
            "relationships": relationships, "native_blocks": blocks,
            "media_calls": calls, "native_media": media, "native_issues": issues,
            "native_boundary": "XML order and embedded bytes are provenance, not figure numbering, pagination, displayed appearance or semantic acceptance.",
        }


def prepare_docx_source(path: Path, source_id: str, directory: Path, *, progress: Progress | None = None) -> dict:
    """Preserve native evidence files and return a source inventory record."""
    result = read_docx(path, source_id, progress=progress)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    with _archive(path) as archive:
        xml_path = directory.joinpath("document.xml")
        with xml_path.open("xb") as stream:
            stream.write(archive.read(DOCUMENT_PART))
        result["native_document"] = _file_record(xml_path)
        for index, item in enumerate(result["native_media"], 1):
            suffix = PurePosixPath(item["part"]).suffix
            suffix = suffix if re.fullmatch(r"\.[A-Za-z0-9]{1,12}", suffix) else ".bin"
            media_path = directory.joinpath(f"media-{index:04d}{suffix}")
            with media_path.open("xb") as stream:
                stream.write(archive.read(item["part"]))
            item["file"] = _file_record(media_path)
    text_path = directory.joinpath("native-blocks.txt")
    with text_path.open("x", encoding="utf-8", newline="") as stream:
        for block in result["native_blocks"]:
            stream.write(f"[{block['id']} | {block['kind']}]\n{block['text']}\n\n")
    result["native_text"] = _file_record(text_path)
    return result


def native_evidence_errors(source: Mapping, location: Mapping) -> list[str]:
    """Check one declared native location against the registered source facts."""
    if not isinstance(location, Mapping):
        return ["A native source location must be an object."]
    errors = []
    if location.get("basis") != "native":
        errors.append("Declare basis=native for an OOXML location.")
    if any(key in location for key in ("page", "bbox", "image_xref", "coordinate_system")):
        errors.append("A native OOXML location must not invent page-space geometry.")
    locator = location.get("native_locator")
    if not valid_native_locator(locator):
        errors.append("Use the recorded document part and zero-based XML element_path.")
    blocks = {row["id"]: row for row in source.get("native_blocks", []) if isinstance(row, Mapping) and "id" in row}
    references = location.get("blocks")
    if not isinstance(references, list) or not references or any(not isinstance(value, str) or value not in blocks for value in references):
        errors.append("Name the actual native source block IDs.")
        return errors
    if len(references) != len(set(references)):
        errors.append("Do not repeat source block IDs within one location.")
    call_id = location.get("media_call")
    if call_id is not None:
        calls = {row["id"]: row for row in source.get("media_calls", []) if isinstance(row, Mapping) and "id" in row}
        call = calls.get(call_id) if isinstance(call_id, str) else None
        if call is None or call.get("status") != "resolved":
            errors.append("The image call must resolve to an embedded media part.")
        elif locator != call.get("locator") or references != [call.get("block_id")]:
            errors.append("Use this image call's exact native locator and parent block.")
    elif len(references) != 1 or locator != blocks[references[0]].get("locator"):
        errors.append("Each native text/table location must identify exactly one original block.")
    quote = location.get("quote")
    if quote is not None and (not isinstance(quote, str) or not quote or quote not in "\n".join(blocks[key]["text"] for key in references)):
        errors.append("The declared quote must occur in its native source block.")
    return errors


def source_native_text(source: Mapping, location: Mapping) -> str:
    errors = native_evidence_errors(source, location)
    if errors:
        raise ValueError(" ".join(errors))
    blocks = {row["id"]: row for row in source["native_blocks"]}
    return "\n".join(blocks[key]["text"] for key in location["blocks"])


class NativeDocument:
    """Cache actual OOXML provenance for repeated translation-span checks."""

    def __init__(self, path: Path):
        self.inventory = read_docx(path, "native")
        blocks = {row["id"]: row for row in self.inventory["native_blocks"]}
        self._text = {tuple(row["locator"]["element_path"]): row["text"] for row in blocks.values()}
        for call in self.inventory["media_calls"]:
            self._text[tuple(call["locator"]["element_path"])] = blocks[call["block_id"]]["text"]

    def resolve_native_locator(self, locator: Mapping) -> str:
        if not valid_native_locator(locator):
            raise ValueError("Use the exact native part and XML child-index path, without page/bbox fields")
        key = tuple(locator["element_path"])
        if key not in self._text:
            raise ValueError("The native locator does not identify an actual body block or image call")
        return self._text[key]


def native_figure_location(source: Mapping, location: Mapping) -> dict:
    """Bind an explicitly chosen image call; never infer a logical figure ID."""
    errors = native_evidence_errors(source, location)
    if errors:
        raise ValueError(" ".join(errors))
    calls = {row["id"]: row for row in source["media_calls"]}
    call = calls.get(location.get("media_call"))
    if call is None:
        raise ValueError("A native figure location needs an explicit media_call")
    media = {row["id"]: row for row in source["native_media"]}
    item = media[call["media_id"]]
    record = item.get("file")
    if not isinstance(record, dict) or set(record) != {"path", "sha256"}:
        raise ValueError("The original embedded media needs its actual file binding")
    return {"native_locator": deepcopy(call["locator"]), "media_call": call["id"],
            "relationship_id": call["relationship_id"], "source_media": deepcopy(record)}
