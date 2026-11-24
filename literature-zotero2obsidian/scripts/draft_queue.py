"""Persist source-backed drafting work without translating or judging meaning.

Use this module from a task-local Python script. PDF block order must be
confirmed from the page; native DOCX blocks retain document XML order. Group
layout fragments into semantic units, write one unit at a time, and assemble
only after every inventoried source entry has an explicit disposition.
"""
from __future__ import annotations

from copy import deepcopy
from hashlib import sha256
import json
from math import isfinite
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Callable, Sequence


BOUNDARY = (
    "Draft coverage and file integrity only. Inventoried blocks, media calls, "
    "explicit exclusions and written fragments do not establish source "
    "completeness, translation fidelity, correct grouping or visual review. "
    "No acceptance ID receives a pass."
)
PROGRESS = Callable[[int, int, str], None]


def progress_bar(done: int, total: int, label: str) -> None:
    end = "\n" if done >= total else "\r"
    print(f"{label}: {done}/{total}", end=end, flush=True)


def _read(path: Path) -> dict:
    value = json.loads(path.read_bytes().decode("utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"Expected a JSON object: {path}")
    return value


def _hash(path: Path) -> str:
    digest = sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _file(path: Path) -> dict:
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError(f"Expected a regular file: {path}")
    return {"path": str(path), "sha256": _hash(path)}


def _binding(value: dict, base: Path) -> dict:
    if not isinstance(value, dict) or not isinstance(value.get("path"), str):
        raise ValueError("Every source needs a file path and SHA-256")
    path = Path(value["path"])
    actual = _file(path if path.is_absolute() else base.joinpath(path))
    if actual["sha256"] != value.get("sha256"):
        raise ValueError(f"Stale source binding: {actual['path']}")
    return actual


def _text(value, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label} must be nonempty text")
    return value


def _rows(value, label: str) -> list:
    if not isinstance(value, list) or any(not isinstance(row, dict) for row in value):
        raise ValueError(f"{label} must be an array of objects")
    return value


def _ids(values: Sequence[str], label: str) -> list[str]:
    if isinstance(values, (str, bytes)):
        raise ValueError(f"{label} must be a sequence of IDs")
    values = list(values)
    if any(not isinstance(value, str) or not value for value in values):
        raise ValueError(f"{label} contains an invalid ID")
    if len(values) != len(set(values)):
        raise ValueError(f"{label} contains duplicate IDs")
    return values


def _json_bytes(value: dict) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def _write_new(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(content)


def _page_key(source: str, page: int) -> str:
    return json.dumps([source, page], ensure_ascii=False, separators=(",", ":"))


def _extract_sources(inventory: dict, base: Path, progress: PROGRESS | None) -> dict:
    sources = _rows(inventory.get("sources"), "inventory.sources")
    if not sources:
        raise ValueError("A drafting queue needs actual sources")
    result = {"source_order": [], "sources": {}, "blocks": {}, "pages": {}, "issues": {}}

    def add_block(identity: str, value: dict) -> None:
        _text(identity, "source block ID")
        if identity in result["blocks"]:
            raise ValueError(f"Duplicate source block ID: {identity}")
        result["blocks"][identity] = {"id": identity, **deepcopy(value)}

    for number, source in enumerate(sources, 1):
        identity = _text(source.get("id"), "source ID")
        if identity in result["sources"]:
            raise ValueError(f"Duplicate source ID: {identity}")
        binding = _binding(source, base)
        source_format = source.get("format") or Path(binding["path"]).suffix.lower().lstrip(".")
        if source_format not in {"pdf", "docx"}:
            raise ValueError(f"Unsupported queue source format: {source_format}")
        result["source_order"].append(identity)
        result["sources"][identity] = {
            **binding, "format": source_format, "role": source.get("role"),
            "block_ids": [], "page_keys": [], "native_media": deepcopy(source.get("native_media", [])),
        }
        target = result["sources"][identity]
        if source_format == "docx":
            calls = _rows(source.get("media_calls", []), "DOCX media_calls")
            calls_by_block = {}
            for call in calls:
                calls_by_block.setdefault(call.get("block_id"), []).append(call)
            native = _rows(source.get("native_blocks"), "DOCX native_blocks")
            for block in native:
                block_id = _text(block.get("id"), "DOCX block ID")
                add_block(block_id, {
                    "source": identity, "kind": block.get("kind", "native"),
                    "text": block.get("text", ""), "native_locator": block.get("locator"),
                    "media_calls": calls_by_block.get(block_id, []), "native_record": block,
                })
                target["block_ids"].append(block_id)
            for index, issue in enumerate(_rows(source.get("native_issues", []), "DOCX native_issues")):
                issue_id = f"{identity}:native-issue:{index + 1}"
                result["issues"][issue_id] = {
                    "source": identity, "original": deepcopy(issue), "decision": None,
                }
            continue
        pages = _rows(source.get("pages"), "PDF pages")
        expected = list(range(1, len(pages) + 1))
        if [page.get("page") for page in pages] != expected or not pages:
            raise ValueError("Retain every actual PDF page once in physical order")
        if source.get("page_count") != len(pages):
            raise ValueError("PDF page_count and page records disagree")
        for page in pages:
            page_number = page["page"]
            key = _page_key(identity, page_number)
            page_ids = []
            for block in _rows(page.get("blocks"), "PDF blocks"):
                block_id = _text(block.get("id"), "PDF block ID")
                add_block(block_id, {"source": identity, "page": page_number,
                                    "kind": "text", "text": block.get("text", ""),
                                    "bbox": block.get("bbox"), "original_block": block})
                page_ids.append(block_id)
            image_index = page.get("image_index", {})
            if not isinstance(image_index, dict):
                raise ValueError("PDF image_index must be an object")
            for index, call in enumerate(_rows(image_index.get("occurrences", []), "PDF image occurrences")):
                block_id = f"{identity}-p{page_number}-image-call-{index + 1}"
                add_block(block_id, {"source": identity, "page": page_number,
                                    "kind": "image_call", "text": "", "bbox": call.get("bbox"),
                                    "image_call": call})
                page_ids.append(block_id)
            result["pages"][key] = {
                "source": identity, "page": page_number, "block_ids": page_ids,
                "render": deepcopy(page.get("render")), "text": deepcopy(page.get("text")),
                "image_index": deepcopy(image_index), "order_review": None,
                "additional_object_review": None,
            }
            target["page_keys"].append(key)
            target["block_ids"].extend(page_ids)
        if progress:
            progress(number, len(sources), "索引实际来源")
    if progress:
        progress(len(sources), len(sources), "索引实际来源")
    return result


def prepare_draft_queue(inventory_path: Path, queue_path: Path, *,
                        progress: PROGRESS | None = progress_bar) -> Path:
    """Snapshot actual inventory bytes; never mutate the working inventory.

    The snapshot is separate because later semantic-unit and link registration
    legitimately updates the working inventory. Source binaries remain bound.
    """
    inventory_path = Path(inventory_path).resolve(strict=True)
    queue_path = Path(queue_path).resolve()
    snapshot_path = queue_path.parent.joinpath(queue_path.stem + "-source-snapshot.json")
    if queue_path.exists() or snapshot_path.exists():
        raise FileExistsError("Resume the existing queue; do not overwrite its snapshot")
    original = inventory_path.read_bytes()
    inventory = json.loads(original.decode("utf-8"))
    if not isinstance(inventory, dict):
        raise ValueError("The inventory must be a JSON object")
    data = _extract_sources(inventory, inventory_path.parent, progress)
    source_paths = {Path(source["path"]) for source in data["sources"].values()}
    if queue_path in source_paths or snapshot_path in source_paths:
        raise ValueError("A queue must not replace a source")
    data.update({"schema_version": 1, "boundary": BOUNDARY,
                 "inventory_origin": str(inventory_path),
                 "inventory_snapshot": {"path": str(snapshot_path), "sha256": sha256(original).hexdigest()},
                 "parts_root": str(queue_path.parent.joinpath(queue_path.stem + "-parts")),
                 "units": {}, "exclusions": {}, "history": []})
    _write_new(snapshot_path, original)
    _write_new(queue_path, _json_bytes(data))
    return queue_path


class DraftQueue:
    """One script session validates inputs once and persists each small decision."""

    def __init__(self, queue_path: Path):
        self.path = Path(queue_path).resolve(strict=True)
        self.data = _read(self.path)
        self._disk_hash = _hash(self.path)
        if self.data.get("schema_version") != 1:
            raise ValueError("Unsupported drafting queue schema")
        _binding(self.data.get("inventory_snapshot"), self.path.parent)
        for source in self.data.get("sources", {}).values():
            _binding(source, self.path.parent)
        snapshot = Path(self.data["inventory_snapshot"]["path"])
        self._baseline = _extract_sources(_read(snapshot), Path(self.data["inventory_origin"]).parent, None)
        self._validate_coverage_structure()

    @classmethod
    def open(cls, queue_path: Path) -> "DraftQueue":
        return cls(queue_path)

    def _validate_coverage_structure(self) -> None:
        data = self.data
        for name in ("sources", "blocks", "pages", "units", "exclusions", "issues"):
            if not isinstance(data.get(name), dict):
                raise ValueError(f"Queue {name} must be an object")
        if data["source_order"] != self._baseline["source_order"]:
            raise ValueError("Do not drop or reorder sources from the frozen inventory")
        if set(data["pages"]) != set(self._baseline["pages"]):
            raise ValueError("Keep every frozen source page")
        for identity, expected in self._baseline["blocks"].items():
            if data["blocks"].get(identity) != expected:
                raise ValueError(f"Keep the frozen source entry unchanged: {identity}")
        for identity in set(data["blocks"]) - set(self._baseline["blocks"]):
            entry = data["blocks"][identity]
            key = _page_key(entry.get("source"), entry.get("page"))
            if not entry.get("added_from_page") or key not in data["pages"]:
                raise ValueError("Additional entries must refer to an actual PDF page")
        for source_id, expected in self._baseline["sources"].items():
            actual = data["sources"].get(source_id, {})
            for name in ("path", "sha256", "format", "role", "page_keys", "native_media"):
                if actual.get(name) != expected.get(name):
                    raise ValueError(f"Keep the frozen source identity: {source_id}")
            entries = {identity for identity, block in data["blocks"].items() if block["source"] == source_id}
            if set(_ids(actual.get("block_ids", []), "source entry list")) != entries:
                raise ValueError("The source entry list must cover every frozen and added entry")
            if actual["format"] == "docx" and actual["block_ids"] != expected["block_ids"]:
                raise ValueError("Keep native DOCX XML order")
        for key, expected in self._baseline["pages"].items():
            page = data["pages"][key]
            for name in ("source", "page", "render", "text", "image_index"):
                if page.get(name) != expected.get(name):
                    raise ValueError("Keep frozen page evidence unchanged")
            entries = {identity for identity, block in data["blocks"].items()
                       if block["source"] == page["source"] and block.get("page") == page["page"]}
            if set(_ids(page.get("block_ids", []), "page entries")) != entries:
                raise ValueError("The page entry list must cover all frozen and added entries")
            if page["order_review"] is not None:
                review = page["order_review"]
                if set(_ids(review.get("block_ids", []), "page reading order")) != entries:
                    raise ValueError("The confirmed reading order omits a source entry")
                _text(review.get("reason"), "reading order reason")
            if page["additional_object_review"] is not None:
                _text(page["additional_object_review"].get("reason"), "additional object review")
        if set(data["issues"]) != set(self._baseline["issues"]):
            raise ValueError("Keep every native source issue visible")
        for identity, expected in self._baseline["issues"].items():
            issue = data["issues"][identity]
            if issue.get("source") != expected["source"] or issue.get("original") != expected["original"]:
                raise ValueError("Keep each original native issue unchanged")
            decision = issue.get("decision")
            if decision is not None:
                if not isinstance(decision, dict) or decision.get("status") not in {"resolved", "excluded", "blocked"}:
                    raise ValueError("Keep an explicit valid native issue disposition")
                _text(decision.get("reason"), "native issue decision reason")
                if not _rows(decision.get("evidence"), "native issue evidence"):
                    raise ValueError("A native issue disposition requires evidence")
        if set(_ids(data.get("source_order", []), "source order")) != set(data["sources"]):
            raise ValueError("Source order must include all sources exactly once")
        assigned = []
        for identity, unit in data["units"].items():
            if not isinstance(unit, dict) or unit.get("id") != identity:
                raise ValueError("Keep the actual drafting unit identity")
            _text(unit.get("kind"), "draft unit kind")
            _text(unit.get("grouping_reason"), "grouping reason")
            _rows(unit.get("revisions"), "draft revisions")
            ids = _ids(unit.get("block_ids", []), "unit block IDs")
            if not ids or any(block not in data["blocks"] for block in ids):
                raise ValueError(f"A unit must refer to actual source entries: {identity}")
            if {data["blocks"][block]["source"] for block in ids} != {unit.get("source")}:
                raise ValueError("Keep different sources in different drafting units")
            assigned.extend(ids)
        if len(assigned) != len(set(assigned)):
            raise ValueError("A source entry cannot be assigned to multiple units")
        if not set(data["exclusions"]) <= set(data["blocks"]):
            raise ValueError("An exclusion must name an actual source entry")
        if set(assigned) & set(data["exclusions"]):
            raise ValueError("A source entry cannot be both assigned and excluded")
        for row in data["exclusions"].values():
            _text(row.get("reason"), "individual exclusion reason")

    def _assert_current(self) -> None:
        if _hash(self.path) != self._disk_hash:
            raise RuntimeError("The queue changed in another script; reload before writing")

    def _commit(self, action: str, target: str) -> None:
        self._assert_current()
        self._validate_coverage_structure()
        self.data["history"].append({"action": action, "target": target})
        content = _json_bytes(self.data)
        with NamedTemporaryFile(dir=self.path.parent, prefix=self.path.name + ".", delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(content)
        temporary.replace(self.path)
        self._disk_hash = sha256(content).hexdigest()

    def _page(self, source_id: str, page: int) -> dict:
        key = _page_key(source_id, page)
        if key not in self.data["pages"]:
            raise ValueError("Choose an actual PDF source page")
        return self.data["pages"][key]

    def review_pdf_page_order(self, source_id: str, page: int,
                              ordered_block_ids: Sequence[str], reason: str) -> None:
        """Record the caller's actual reading order, never a guessed column sort."""
        target = self._page(source_id, page)
        ids = _ids(ordered_block_ids, "confirmed page order")
        if set(ids) != set(target["block_ids"]):
            raise ValueError("Page order must include every text, image-call and added entry exactly once")
        target["order_review"] = {"block_ids": ids, "reason": _text(reason, "reading-order reason")}
        self._commit("review_page_order", _page_key(source_id, page))

    def review_additional_objects(self, source_id: str, page: int, reason: str) -> None:
        """Save the caller's page-level search for non-indexed content.

        Register each found vector figure, formula or other missing object with
        add_page_object() first. This declaration does not certify observation.
        """
        target = self._page(source_id, page)
        target["additional_object_review"] = {"reason": _text(reason, "object inventory observation")}
        self._commit("review_additional_objects", _page_key(source_id, page))

    def add_page_object(self, block_id: str, source_id: str, page: int, *,
                        kind: str, reason: str, bbox: Sequence[float] | None = None,
                        text: str = "") -> None:
        """Register a visible object absent from the extraction; page stays real."""
        _text(block_id, "new source entry ID")
        if block_id in self.data["blocks"]:
            raise ValueError("Keep source entry IDs unique")
        target = self._page(source_id, page)
        if bbox is not None:
            bbox = list(bbox)
            if len(bbox) != 4 or any(type(x) not in {int, float} or not isfinite(x) for x in bbox):
                raise ValueError("bbox needs four finite numeric coordinates")
            if not bbox[0] < bbox[2] or not bbox[1] < bbox[3]:
                raise ValueError("bbox must be nonempty")
            source = _binding(self.data["sources"][source_id], self.path.parent)
            bounds = target["image_index"].get("page_bounds")
            if bounds is None:
                import fitz

                with fitz.open(source["path"]) as document:
                    actual_page = document[page - 1]
                    bounds = list(actual_page.rect * actual_page.derotation_matrix)
            if (not isinstance(bounds, (list, tuple)) or len(bounds) != 4
                    or any(type(x) not in {int, float} or not isfinite(x) for x in bounds)
                    or not bounds[0] < bounds[2] or not bounds[1] < bounds[3]):
                raise ValueError("Frozen PDF page bounds must be finite and nonempty")
            if not (bounds[0] <= bbox[0] < bbox[2] <= bounds[2]
                    and bounds[1] <= bbox[1] < bbox[3] <= bounds[3]):
                raise ValueError("bbox must lie wholly inside the actual PDF page")
        if not isinstance(text, str):
            raise ValueError("Visible transcription must be text")
        self.data["blocks"][block_id] = {
            "id": block_id, "source": source_id, "page": page,
            "kind": _text(kind, "source object kind"), "text": text, "bbox": bbox,
            "registration_reason": _text(reason, "visible object observation"), "added_from_page": True,
        }
        target["block_ids"].append(block_id)
        target["order_review"] = None
        target["additional_object_review"] = None
        self.data["sources"][source_id]["block_ids"].append(block_id)
        self._commit("add_page_object", block_id)

    def group(self, unit_id: str, block_ids: Sequence[str], *, kind: str,
              reason: str) -> None:
        """Explicitly join layout fragments, including a cross-page paragraph."""
        _text(unit_id, "draft unit ID")
        if unit_id in self.data["units"]:
            raise ValueError("Keep existing units; use a new unit ID")
        ids = _ids(block_ids, "grouped source entries")
        if not ids or any(identity not in self.data["blocks"] for identity in ids):
            raise ValueError("Group one or more actual source entries")
        assigned = {identity for unit in self.data["units"].values() for identity in unit["block_ids"]}
        if set(ids) & (assigned | set(self.data["exclusions"])):
            raise ValueError("A source entry already has a disposition")
        source_ids = {self.data["blocks"][identity]["source"] for identity in ids}
        if len(source_ids) != 1:
            raise ValueError("Do not group different source documents")
        self.data["units"][unit_id] = {
            "id": unit_id, "source": source_ids.pop(), "block_ids": ids,
            "kind": _text(kind, "draft unit kind"), "grouping_reason": _text(reason, "grouping reason"),
            "revisions": [],
        }
        self._commit("group", unit_id)

    def exclude(self, block_id: str, reason: str) -> None:
        """Exclude one actual entry with its individual source-based reason."""
        if block_id not in self.data["blocks"]:
            raise ValueError("Choose an actual source entry")
        if block_id in self.data["exclusions"] or any(
                block_id in unit["block_ids"] for unit in self.data["units"].values()):
            raise ValueError("A source entry already has a disposition")
        self.data["exclusions"][block_id] = {"reason": _text(reason, "exclusion reason")}
        self._commit("exclude", block_id)

    def write_unit(self, unit_id: str, markdown: str, *, assets: Sequence[Path] = ()) -> dict:
        """Write an immutable revision; preserve earlier bytes and asset hashes."""
        self._assert_current()
        if unit_id not in self.data["units"]:
            raise ValueError("Group the actual source entries before writing")
        _text(markdown, "draft fragment")
        unit = self.data["units"][unit_id]
        assets = [_file(Path(path)) for path in assets]
        token = sha256(unit_id.encode("utf-8")).hexdigest()[:24]
        version = len(unit["revisions"]) + 1
        path = Path(self.data["parts_root"]).joinpath(f"{token}-v{version:04d}.md")
        content = markdown.encode("utf-8")
        _write_new(path, content)
        revision = {"path": str(path), "sha256": sha256(content).hexdigest(), "assets": assets}
        unit["revisions"].append(revision)
        self._commit("write_unit", unit_id)
        return deepcopy(revision)

    def record_issue_decision(self, issue_id: str, *, status: str, reason: str,
                              evidence_paths: Sequence[Path]) -> None:
        """Record an explicit resolution or exclusion; blocked stays unfinished."""
        if issue_id not in self.data["issues"]:
            raise ValueError("Choose an actual source issue")
        if status not in {"resolved", "excluded", "blocked"}:
            raise ValueError("Issue status must be resolved, excluded or blocked")
        evidence = [_file(Path(path)) for path in evidence_paths]
        if not evidence:
            raise ValueError("A source issue decision needs actual evidence files")
        self.data["issues"][issue_id]["decision"] = {
            "status": status, "reason": _text(reason, "issue decision reason"), "evidence": evidence,
        }
        self._commit("issue_decision", issue_id)

    def _ordered_blocks(self) -> list[str]:
        ordered = []
        for source_id in self.data["source_order"]:
            source = self.data["sources"][source_id]
            if source["format"] == "docx":
                ordered.extend(source["block_ids"])
                continue
            for key in source["page_keys"]:
                page = self.data["pages"][key]
                review = page["order_review"]
                ordered.extend(review["block_ids"] if review else page["block_ids"])
        return ordered

    def status(self) -> dict:
        assigned = {block: unit_id for unit_id, unit in self.data["units"].items() for block in unit["block_ids"]}
        undisposed = [identity for identity in self._ordered_blocks()
                      if identity not in assigned and identity not in self.data["exclusions"]]
        missing = [identity for identity, unit in self.data["units"].items() if not unit["revisions"]]
        unreviewed = [key for key, page in self.data["pages"].items()
                      if page["order_review"] is None or page["additional_object_review"] is None]
        issues = [identity for identity, issue in self.data["issues"].items()
                  if not issue["decision"] or issue["decision"]["status"] == "blocked"]
        return {"draft_ready": bool(self.data["units"]) and not (undisposed or missing or unreviewed or issues),
                "source_entry_count": len(self.data["blocks"]), "unit_count": len(self.data["units"]),
                "written_unit_count": len(self.data["units"]) - len(missing),
                "excluded_entry_count": len(self.data["exclusions"]), "unassigned_entries": undisposed,
                "unwritten_units": missing, "unreviewed_pdf_pages": unreviewed,
                "unresolved_source_issues": issues, "source_fidelity": "not_assessed", "boundary": BOUNDARY}

    def _source_context(self, entry_ids: Sequence[str]) -> dict:
        entries = [self.data["blocks"][identity] for identity in entry_ids]
        source_ids = dict.fromkeys(entry["source"] for entry in entries)
        page_keys = dict.fromkeys(_page_key(entry["source"], entry["page"])
                                 for entry in entries if "page" in entry)
        return {
            "source_files": {source_id: {
                key: deepcopy(self.data["sources"][source_id][key])
                for key in ("path", "sha256", "format", "role", "native_media")
            } for source_id in source_ids},
            "source_pages": [{key: deepcopy(self.data["pages"][page_key][key])
                              for key in ("source", "page", "render", "text")}
                             for page_key in page_keys],
        }

    def next_task(self) -> dict:
        """Return the first actual drafting task, before global review paperwork."""
        assigned = {block: unit_id for unit_id, unit in self.data["units"].items() for block in unit["block_ids"]}
        events = []
        for source_id in self.data["source_order"]:
            source = self.data["sources"][source_id]
            if source["format"] == "docx":
                events.extend(("block", identity) for identity in source["block_ids"])
                continue
            for key in source["page_keys"]:
                events.append(("page", key))
                page = self.data["pages"][key]
                review = page["order_review"]
                events.extend(("block", identity) for identity in (review["block_ids"] if review else page["block_ids"]))
        for event, identity in events:
            if event == "page":
                page = self.data["pages"][identity]
                if page["order_review"] and page["additional_object_review"]:
                    continue
                return {"stage": "inventory_page", "page": deepcopy(page),
                        "source_file": {key: self.data["sources"][page["source"]][key] for key in ("path", "sha256")},
                        "entries": [deepcopy(self.data["blocks"][block_id]) for block_id in page["block_ids"]],
                        "boundary": BOUNDARY}
            block_id = identity
            block = self.data["blocks"][block_id]
            if block_id in self.data["exclusions"]:
                continue
            if block_id not in assigned:
                return {"stage": "group_or_exclude", "entry": deepcopy(block),
                        **self._source_context([block_id]), "boundary": BOUNDARY}
            unit_id = assigned[block_id]
            unit = self.data["units"][unit_id]
            if not unit["revisions"]:
                return {"stage": "write_unit", "unit": deepcopy(unit),
                        "entries": [deepcopy(self.data["blocks"][identity]) for identity in unit["block_ids"]],
                        **self._source_context(unit["block_ids"]),
                        "boundary": BOUNDARY}
        status = self.status()
        if status["unresolved_source_issues"]:
            issue_id = status["unresolved_source_issues"][0]
            return {"stage": "source_issue", "issue_id": issue_id,
                    "issue": deepcopy(self.data["issues"][issue_id]), "boundary": BOUNDARY}
        return {"stage": "assemble" if status["draft_ready"] else "no_draft_units", **status}

    def assemble(self, output_path: Path, *, progress: PROGRESS | None = progress_bar) -> dict:
        """Assemble complete registered fragments, leaving all semantic checks open."""
        self._assert_current()
        self._validate_coverage_structure()
        status = self.status()
        if not status["draft_ready"]:
            raise ValueError("Continue drafting before assembly: " + json.dumps(status, ensure_ascii=False))
        output_path = Path(output_path).resolve()
        mapping_path = output_path.parent.joinpath(output_path.name + ".draft-map.json")
        protected = {self.path, Path(self.data["inventory_snapshot"]["path"]),
                     *[Path(source["path"]) for source in self.data["sources"].values()]}
        if output_path in protected or mapping_path in protected:
            raise ValueError("An assembly must not overwrite a protected file")
        if output_path.exists() or mapping_path.exists():
            raise FileExistsError("Assemble a new version; keep previous candidates")
        # Recheck real inputs at publication, even when this object was kept in a
        # long-running Python session. Fragment and asset integrity is checked too.
        _binding(self.data["inventory_snapshot"], self.path.parent)
        for source in self.data["sources"].values():
            _binding(source, self.path.parent)
        for issue in self.data["issues"].values():
            for evidence in issue["decision"]["evidence"]:
                _binding(evidence, self.path.parent)
        ranks = {identity: index for index, identity in enumerate(self._ordered_blocks())}
        ordered_units = sorted(self.data["units"].values(), key=lambda unit: min(ranks[identity] for identity in unit["block_ids"]))
        pieces, mapping, line = [], [], 1
        for number, unit in enumerate(ordered_units, 1):
            revision = unit["revisions"][-1]
            binding = _binding(revision, self.path.parent)
            for asset in revision["assets"]:
                _binding(asset, self.path.parent)
            # Preserve internal whitespace and math. Only boundary newlines are
            # canonicalized when joining separately saved Markdown fragments.
            text = Path(binding["path"]).read_bytes().decode("utf-8").rstrip("\r\n")
            if not text.strip():
                raise ValueError(f"Empty fragment: {unit['id']}")
            lines = text.count("\n") + 1
            mapping.append({"unit_id": unit["id"], "source": unit["source"],
                            "block_ids": list(unit["block_ids"]), "candidate_lines": [line, line + lines - 1],
                            "fragment": deepcopy(revision), "source_fidelity": "not_assessed"})
            pieces.append(text)
            line += lines + 1
            if progress:
                progress(number, len(ordered_units), "组装已保存片段")
        content = ("\n\n".join(pieces) + "\n").encode("utf-8")
        _write_new(output_path, content)
        report = {"candidate": {"path": str(output_path), "sha256": sha256(content).hexdigest()},
                  "inventory_snapshot": deepcopy(self.data["inventory_snapshot"]),
                  "queue": _file(self.path), "units": mapping, "excluded_entries": deepcopy(self.data["exclusions"]),
                  "draft_ready": True, "source_fidelity": "not_assessed", "boundary": BOUNDARY}
        _write_new(mapping_path, _json_bytes(report))
        return report
