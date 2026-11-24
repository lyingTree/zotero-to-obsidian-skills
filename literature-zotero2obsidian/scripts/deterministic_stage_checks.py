"""Deterministic stage helpers and individual file-based check results.

These functions never infer authors, metadata, source meaning, or identity.
A missing input is not an empty successful scope.  All changes to a note are
returned as bytes; the caller writes a separate draft, never the source.
"""
from __future__ import annotations

from collections import Counter
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
from typing import Callable, Mapping, Sequence

import yaml

from validate_note import _UniqueKeyLoader, _rich_table_payload, validate_note

DEPRECATED = "库内参考文献"
BASE_KEYS = ("类型", "年份", "关键词")
ITEM_TYPES = {"journalArticle", "conferencePaper", "thesis", "preprint"}
Progress = Callable[[int, int, str], None]
BOM = b"\xef\xbb\xbf"
DELIMITER = re.compile(rb"^(?:---|\.\.\.)[ \t]*(?:\r\n|\n|\r|$)")
OPEN = re.compile(rb"^---[ \t]*(?:\r\n|\n|\r|$)")


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def snapshot(path: Path) -> dict:
    """Hash actual readable bytes; do not infer MIME or semantic validity."""
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError(f"Not a regular file: {path}")
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            size += len(block)
            digest.update(block)
    return {"path": str(path), "sha256": digest.hexdigest(), "size": size}


def verify_binding(record: Mapping, actual_path: Path, *, base: Path) -> dict:
    """Compare against an independently supplied actual file, not another copy."""
    actual = snapshot(actual_path)
    if not isinstance(record, Mapping) or not isinstance(record.get("path"), str) or not record["path"]:
        return {"matches": False, "reason": "missing recorded file path", "actual": actual}
    bound = Path(record["path"])
    if not bound.is_absolute():
        bound = Path(base).joinpath(bound)
    same_path = bound.resolve() == Path(actual["path"])
    same_hash = record.get("sha256") == actual["sha256"]
    return {"matches": same_path and same_hash, "same_path": same_path,
            "same_hash": same_hash, "actual": actual}


def split_frontmatter(data: bytes) -> tuple[bytes, bytes, bytes | None]:
    """Return original prefix, remaining body, and YAML payload without editing.

    Only an opening document marker at the very beginning is frontmatter.
    Later thematic breaks and Rich Table delimiters are not frontmatter.
    An unclosed opening marker is a blocker, not a body that can be overwritten.
    """
    bom = BOM if data.startswith(BOM) else b""
    lines = data[len(bom):].splitlines(keepends=True)
    if not lines or not OPEN.fullmatch(lines[0]):
        return bom, data[len(bom):], None
    closing = next((i for i, line in enumerate(lines[1:], 1)
                    if DELIMITER.fullmatch(line)), None)
    if closing is None:
        raise ValueError("unclosed beginning YAML frontmatter")
    return (bom + b"".join(lines[:closing + 1]),
            b"".join(lines[closing + 1:]), b"".join(lines[1:closing]))


def _mapping(payload: bytes | None) -> dict:
    if payload is None:
        raise ValueError("missing beginning YAML frontmatter")
    value = yaml.load(payload.decode("utf-8"), Loader=_UniqueKeyLoader)
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("frontmatter must be a mapping of string keys")
    return value


def _managed_keys(item_type: str) -> tuple[str, ...]:
    if item_type not in ITEM_TYPES:
        raise ValueError("unsupported or unconfirmed Zotero itemType")
    return BASE_KEYS + (("期刊名",) if item_type == "journalArticle" else ())


def _valid_managed_types(values: Mapping, keys: Sequence[str]) -> bool:
    for key in keys:
        if key not in values:
            continue
        value = values[key]
        if key == "年份" and value is not None and type(value) is not int:
            return False
        if key == "关键词" and (not isinstance(value, list) or
                                any(not isinstance(word, str) for word in value)):
            return False
        if key in {"类型", "期刊名"} and value is not None and not isinstance(value, str):
            return False
    return True


def build_properties(data: bytes, *, item_type: str, confirmed: Mapping) -> bytes:
    """Build new managed properties from confirmed fields, not guessed metadata.

    Values and order follow the templates. JSON scalars/lists are valid YAML
    and keep colon, hash, date-looking strings, quotes and newlines unambiguous.
    Never replace an existing property region through this new-note function.
    """
    prefix, body, payload = split_frontmatter(data)
    keys = _managed_keys(item_type)
    if payload is not None:
        raise ValueError("existing YAML: use repair_properties")
    if not isinstance(confirmed, Mapping) or set(confirmed) != set(keys):
        raise ValueError(f"confirmed fields must exactly match {keys}")
    if not _valid_managed_types(confirmed, keys):
        raise ValueError("managed property value type differs from template")
    if confirmed["年份"] is not None and not 1 <= confirmed["年份"] <= 9999:
        raise ValueError("year must be a confirmed positive four-digit-or-shorter integer")
    newline = b"\r\n" if b"\r\n" in body else b"\n"
    lines = ["---"] + [key + ": " + json.dumps(confirmed[key], ensure_ascii=False,
                                               allow_nan=False) for key in keys] + ["---"]
    output = prefix + newline.join(line.encode("utf-8") for line in lines) + newline + body
    _, actual_body, new_payload = split_frontmatter(output)
    if actual_body != body or _mapping(new_payload) != dict(confirmed):
        raise ValueError("generated properties did not round-trip")
    return output


def repair_properties(data: bytes) -> bytes:
    """Remove only the deprecated top-level property, preserving all other bytes.

    Complex layouts where removal cannot be isolated, duplicate keys, aliases
    depending on the removed key and unparseable YAML are blocked.  No global
    YAML reserialization, no deletion of a same-named nested/custom field.
    """
    prefix, body, payload = split_frontmatter(data)
    values = _mapping(payload)
    if DEPRECATED not in values:
        return data
    source = payload.decode("utf-8")
    root = yaml.compose(source, Loader=_UniqueKeyLoader)
    if not isinstance(root, yaml.MappingNode) or root.flow_style:
        raise ValueError("cannot isolate deprecated key in a flow mapping")
    match = [(key, val) for key, val in root.value
             if isinstance(key, yaml.ScalarNode) and key.value == DEPRECATED]
    if len(match) != 1:
        raise ValueError("deprecated key is not one literal top-level entry")
    key, value = match[0]
    if key.start_mark.column != 0:
        raise ValueError("cannot isolate non-top-level key")
    lines = source.splitlines(keepends=True)
    start = key.start_mark.line
    end = max(start + 1, value.end_mark.line + bool(value.end_mark.column))
    if end > len(lines) or any(line.startswith("#") for line in lines[start + 1:end]):
        raise ValueError("deprecated field intersects independent comments")
    remaining = "".join(lines[:start] + lines[end:]).encode("utf-8")
    if not remaining.strip():
        remaining = b"{}\r\n" if b"\r\n" in payload else b"{}\n"
    # Preserve exact opening/closing delimiters and the body.
    bom = BOM if prefix.startswith(BOM) else b""
    delimiter_lines = prefix[len(bom):].splitlines(keepends=True)
    output = bom + delimiter_lines[0] + remaining + delimiter_lines[-1] + body
    expected = dict(values)
    del expected[DEPRECATED]
    _, after_body, after_payload = split_frontmatter(output)
    after = _mapping(after_payload)
    if (after_body != body or
            yaml.safe_dump(after, sort_keys=False) != yaml.safe_dump(expected, sort_keys=False)):
        raise ValueError("removal changed other YAML values or body")
    return output


def author_token(confirmed_author_markers: Sequence[str], *, unknown_reason: str | None = None) -> str:
    """Compose verified author markers; never split surnames or infer roles."""
    if isinstance(confirmed_author_markers, (str, bytes)) or not isinstance(confirmed_author_markers, Sequence):
        raise ValueError("supply the ordered confirmed author markers as a sequence")
    markers = tuple(confirmed_author_markers)
    if any(not isinstance(marker, str) or not marker.strip() or marker != marker.strip()
           or "\n" in marker or "\r" in marker for marker in markers):
        raise ValueError("invalid confirmed author marker")
    if not markers:
        if not isinstance(unknown_reason, str) or not unknown_reason.strip():
            raise ValueError("missing evidence-based reason for unknown author")
        return "未知作者"
    if len(markers) == 1:
        return markers[0]
    if len(markers) == 2:
        return " and ".join(markers)
    return markers[0] + " et al"


def collection_ancestry(item: Mapping, records: Sequence[Mapping], *,
                        pagination_complete: bool, progress: Progress | None = None) -> dict:
    """Trace actual Zotero collection records; retain names and library identity.

    Input is the unmodified REST item object and fetched collection objects.
    A missing record, library, parent, duplicate, cycle or incomplete paging
    yields explicit incomplete evidence, not a guessed placement.
    """
    if type(pagination_complete) is not bool:
        raise ValueError("declare whether collection pagination completed")
    if not isinstance(item, Mapping) or not isinstance(item.get("data"), Mapping):
        raise ValueError("expected original Zotero item data")
    direct = item["data"].get("collections")
    library = item.get("library")
    def library_identity(value):
        if not isinstance(value, Mapping) or value.get("type") not in {"user", "group"}:
            return None
        identity = value.get("id")
        return (value["type"], identity) if type(identity) is int and identity > 0 else None
    if library_identity(library) is None:
        raise ValueError("actual library identity required; route alias 0 is not identity")
    if (not isinstance(direct, list) or
            any(not isinstance(key, str) or not key for key in direct) or
            len(set(direct)) != len(direct)):
        raise ValueError("missing or invalid original direct collections")
    index = {}
    issues = []
    for record in records:
        if not isinstance(record, Mapping) or not isinstance(record.get("data"), Mapping):
            raise ValueError("invalid collection API record")
        key, data = record.get("key"), record["data"]
        if not isinstance(key, str) or not key or data.get("key", key) != key:
            raise ValueError("collection key missing or inconsistent")
        if key in index:
            raise ValueError(f"duplicate collection key: {key}")
        if library_identity(record.get("library")) != library_identity(library):
            issues.append({"key": key, "reason": "collection_library_unverified_or_different"})
            continue
        index[key] = data
    if not pagination_complete:
        issues.append({"reason": "pagination_incomplete"})
    paths = []
    for number, leaf in enumerate(direct, 1):
        seen = set()
        ancestry = []
        key = leaf
        reason = None
        while key is not False:
            if not isinstance(key, str) or not key:
                reason = "invalid_or_missing_parent_field"
                break
            if key in seen:
                reason = "collection_cycle"
                break
            seen.add(key)
            data = index.get(key)
            if data is None:
                reason = "collection_record_missing"
                break
            if not isinstance(data.get("name"), str) or not data["name"]:
                reason = "collection_name_missing"
                break
            ancestry.append({"key": key, "name": data["name"], "library": dict(library)})
            key = data.get("parentCollection")
        if reason:
            issues.append({"key": leaf, "reason": reason})
        paths.append({"library": dict(library), "collection_key": leaf,
                      "ancestry": list(reversed(ancestry)),
                      "status": "incomplete" if reason else "traced"})
        if progress:
            progress(number, len(direct), "collections")
    return {"collections": paths, "issues": issues,
            "status": "incomplete" if issues else "complete",
            "pagination_complete": pagination_complete}


def inspect_relative_files(root: Path, relative_paths: Sequence[str], *,
                           progress: Progress | None = None) -> list[dict]:
    """Read each explicitly supplied local target; reject escaping paths/symlinks.

    The caller still must establish that this list covers all embeds/links and
    that each target belongs to the correct source object or bibliography.
    """
    root = Path(root).resolve(strict=True)
    if not root.is_dir():
        raise ValueError("resource root must be a directory")
    if isinstance(relative_paths, (str, bytes)) or not isinstance(relative_paths, Sequence):
        raise ValueError("provide explicit local relative paths")
    result = []
    cache = {}
    for i, value in enumerate(relative_paths, 1):
        try:
            if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
                raise ValueError("invalid vault-relative POSIX path")
            parts = PurePosixPath(value).parts
            if (not parts or PurePosixPath(value).is_absolute() or
                    any(part in {"..", "."} or ":" in part for part in value.split("/")) or
                    any(not part for part in value.split("/"))):
                raise ValueError("absolute, traversing or noncanonical relative path")
            target = root.joinpath(*parts).resolve(strict=True)
            if not target.is_relative_to(root):
                raise ValueError("target escapes the resource root")
            if target not in cache:
                cache[target] = snapshot(target)
            entry = {"relative_path": value, "status": "readable", **cache[target]}
            if entry["size"] == 0:
                entry["status"] = "empty"
        except (OSError, RuntimeError, ValueError) as error:
            entry = {"relative_path": value, "status": "blocked", "reason": str(error)}
        result.append(entry)
        if progress:
            progress(i, len(relative_paths), "relative_files")
    return result


def _decision(identity: str, value: bool | None, reason: str) -> dict:
    return {"id": identity, "status": ("not_verified" if value is None else
                                        "passed" if value else "failed"),
            "reason": reason}


def audit_properties(before: Path, after: Path, *, mode: str, item_type: str) -> dict:
    """Compute individual property-stage results from actual file bytes."""
    if mode not in {"new", "repair"}:
        raise ValueError("mode must be new or repair")
    keys = _managed_keys(item_type)
    before, after = Path(before).resolve(strict=True), Path(after).resolve(strict=True)
    original, candidate = before.read_bytes(), after.read_bytes()
    result = {}
    def put(identity, value, reason):
        result[identity] = _decision(identity, value, reason)
    old_payload = new_payload = old_body = new_body = None
    errors = []
    for label, raw in (("before", original), ("after", candidate)):
        try:
            _, body, payload = split_frontmatter(raw)
        except ValueError as error:
            errors.append(f"{label}: {error}")
            continue
        if label == "before":
            old_body, old_payload = body, payload
        else:
            new_body, new_payload = body, payload
    put("PROP-A08", None if errors else old_body == new_body,
        "; ".join(errors) if errors else "compare actual body bytes outside beginning YAML")
    try:
        if new_payload is None:
            raise ValueError("beginning YAML missing or unclosed")
        yaml.safe_load(new_payload.decode("utf-8"))
        put("PROP-A01", True, "actual YAML parses with safe loader")
    except (ValueError, UnicodeError, yaml.YAMLError, RecursionError) as error:
        put("PROP-A01", False, str(error))
    values = None
    try:
        values = _mapping(new_payload)
        put("PROP-A02", True, "unique-key YAML loader accepted actual mapping")
    except (ValueError, UnicodeError, yaml.YAMLError, RecursionError) as error:
        duplicate = "duplicate key" in str(error)
        put("PROP-A02", False if duplicate else None, str(error))
    put("PROP-A04", None if values is None else DEPRECATED not in values,
        "check the actual parsed top-level key set")
    if values is None:
        put("PROP-A05", None, "valid property mapping required")
        put("PROP-A06", None, "valid property mapping required")
    else:
        put("PROP-A06", _valid_managed_types(values, keys), "check actual managed value types")
        if mode == "new":
            put("PROP-A05", set(values) == set(keys), "compare new keys with selected template")
        else:
            try:
                old_values = _mapping(old_payload)
                put("PROP-A05", set(values) == set(old_values) - {DEPRECATED},
                    "repair must retain original top-level keys except deprecated key")
            except (ValueError, UnicodeError, yaml.YAMLError, RecursionError) as error:
                put("PROP-A05", None, str(error))
    if mode == "repair":
        try:
            exact = repair_properties(original) == candidate
            put("PROP-C10", exact, "compare complete result with surgical removal")
            put("PROP-C18", exact, "compare original writing outside deprecated field")
        except (ValueError, UnicodeError, yaml.YAMLError, RecursionError) as error:
            put("PROP-C10", None, str(error))
            put("PROP-C18", None, str(error))
    return {"stage": "properties", "mode": mode, "item_type": item_type,
            "inputs": [{"path": str(before), "sha256": _digest(original)},
                       {"path": str(after), "sha256": _digest(candidate)}],
            "checks": result, "source_fidelity": "not_assessed"}


def audit_unchanged_properties(before: Path, after: Path, *, stage: str) -> dict:
    """The header and library-link steps must not edit the YAML bytes."""
    identities = {"header": "HDR-A14", "library": "LIB-A08"}
    if stage not in identities:
        raise ValueError("stage must be header or library")
    before, after = Path(before).resolve(strict=True), Path(after).resolve(strict=True)
    a, b = before.read_bytes(), after.read_bytes()
    try:
        result = split_frontmatter(a)[0] == split_frontmatter(b)[0]
        reason = "compare complete beginning property region byte-for-byte"
    except ValueError as error:
        result, reason = None, str(error)
    identity = identities[stage]
    return {"stage": stage, "inputs": [{"path": str(before), "sha256": _digest(a)},
                                        {"path": str(after), "sha256": _digest(b)}],
            "checks": {identity: _decision(identity, result, reason)},
            "source_fidelity": "not_assessed"}


def audit_write_readback(candidate: Path, written: Path) -> dict:
    """Compare real file bytes after writing; this is not authorization to write."""
    candidate, written = Path(candidate).resolve(strict=True), Path(written).resolve(strict=True)
    if candidate == written:
        raise ValueError("readback needs independent candidate and actual target files")
    a, b = candidate.read_bytes(), written.read_bytes()
    try:
        values = _mapping(split_frontmatter(b)[2])
        absent = DEPRECATED not in values
        reason = "parsed actual written YAML"
    except (ValueError, UnicodeError, yaml.YAMLError, RecursionError) as error:
        absent, reason = None, str(error)
    return {"stage": "write", "inputs": [{"path": str(candidate), "sha256": _digest(a)},
                                         {"path": str(written), "sha256": _digest(b)}],
            "checks": {
                "WRITE-A04": _decision("WRITE-A04", _digest(a) == _digest(b), "actual candidate and readback SHA-256"),
                "WRITE-A05": _decision("WRITE-A05", absent, reason)},
            "source_fidelity": "not_assessed"}


def audit_translation_binding(record_path: Path, actual_file: Path, *, kind: str) -> dict:
    """Check the independently selected glossary or reviewed draft binding."""
    choices = {"glossary": ("TRN-A17", ("glossary",)),
               "candidate": ("TRN-A25", ("bindings", "candidate"))}
    if kind not in choices:
        raise ValueError("kind must be glossary or candidate")
    identity, fields = choices[kind]
    record_path = Path(record_path).resolve(strict=True)
    raw = record_path.read_bytes()
    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result
    try:
        record = json.loads(raw, object_pairs_hook=unique_pairs)
        for field in fields:
            record = record[field]
        comparison = verify_binding(record, actual_file, base=record_path.parent)
        value, reason = comparison["matches"], "recorded path and SHA-256 compared to independently chosen actual file"
    except (KeyError, TypeError, ValueError, OSError, RuntimeError) as error:
        comparison = None
        value, reason = None, str(error)
    actual = snapshot(actual_file)
    if comparison is not None and actual != comparison["actual"]:
        value, reason = None, "actual file changed during comparison"
    return {"stage": "translation", "inputs": [
                {"path": str(record_path), "sha256": _digest(raw)},
                {"path": actual["path"], "sha256": actual["sha256"]}],
            "checks": {identity: _decision(identity, value, reason)},
            "source_fidelity": "not_assessed"}



def audit_rich_tables(candidate: Path) -> dict:
    """Evaluate the actual recognized Rich Table blocks and their sheets.

    Reuse the validator's Markdown-aware scanner rather than matching fences
    inside examples or comments. No table is not a successful table scope.
    Source fidelity, row completeness and application rendering remain separate.
    """
    candidate = Path(candidate).resolve(strict=True)
    raw = candidate.read_bytes()
    text = raw.decode("utf-8")
    tables: list[dict] = []
    findings = validate_note(text, table_records=tables)
    scan_uncertain = any(f.code in {"FENCE_UNCLOSED", "MATH_UNCLOSED", "COMMENT_UNCLOSED"}
                         for f in findings)
    ids = ("BODY-A83", "BODY-A84", "BODY-A85", "BODY-A86", "BODY-A87", "BODY-A111")
    outcomes = {identity: [] for identity in ids}
    details = []
    for table in tables:
        item = {"line": table["line"], "sheets": []}
        details.append(item)
        try:
            if table["closing_line"] is None:
                raise ValueError("unclosed Rich Table fence")
            value = yaml.load(_rich_table_payload(table["content"]), Loader=_UniqueKeyLoader)
        except (yaml.YAMLError, ValueError, RecursionError) as error:
            outcomes["BODY-A83"].append(False)
            item["error"] = str(error)
            for identity in ids[1:]:
                outcomes[identity].append(None)
            continue
        outcomes["BODY-A83"].append(True)
        version = value.get("version") if isinstance(value, dict) else None
        sheets = ([value] if type(version) is int and version == 2
                  else value.get("sheets") if type(version) is int and version == 3 else None)
        if not isinstance(sheets, list) or not sheets or not all(isinstance(sheet, dict) for sheet in sheets):
            item["error"] = "unsupported or incomplete sheet structure"
            for identity in ids[1:]:
                outcomes[identity].append(None)
            continue
        if version == 3 and "footer" in value:
            outcomes["BODY-A111"].append(isinstance(value["footer"], str))
        for index, sheet in enumerate(sheets, 1):
            columns, rows = sheet.get("columns"), sheet.get("rows")
            outcomes["BODY-A84"].append(sheet.get("locked") is True)
            outcomes["BODY-A85"].append(sheet.get("theme") == "grid")
            outcomes["BODY-A86"].append(isinstance(columns, list) and bool(columns))
            outcomes["BODY-A87"].append(isinstance(rows, list) and bool(rows))
            if "footer" in sheet:
                outcomes["BODY-A111"].append(isinstance(sheet["footer"], str))
            item["sheets"].append({"index": index, "columns": len(columns) if isinstance(columns, list) else None,
                                   "rows": len(rows) if isinstance(rows, list) else None})
    def combine(values):
        if False in values:
            return False
        if scan_uncertain or not values or None in values:
            return None
        return True
    checks = {identity: _decision(identity, combine(values),
                "evaluate each recognized Rich Table block or sheet; incomplete scope stays unresolved")
              for identity, values in outcomes.items()}
    return {"stage": "tables", "inputs": [{"path": str(candidate), "sha256": _digest(raw)}],
            "checks": checks, "tables": details, "source_fidelity": "not_assessed"}

def record_file_checks(checks_path: Path, report_path: Path, *, stage: str,
                       after: Path, before: Path | None = None, mode: str | None = None,
                       item_type: str | None = None, kind: str | None = None) -> list[str]:
    """Record evaluated offline A IDs with file-bound evidence in a single writer.

    Dispatch only to the actual audit functions; accept no caller-supplied
    decision report. This function rechecks bound input bytes,
    refuses overwriting a different report, and leaves other decisions intact.
    The full workflow validator still checks the ledger and all semantic gates.
    """
    from complete_workflow import record_check_batch, _module
    check_tools = _module("check_records")
    if stage == "tables":
        report = audit_rich_tables(after)
    else:
        if before is None:
            raise ValueError("this audit requires independently supplied before and after files")
        if stage == "properties":
            report = audit_properties(before, after, mode=mode, item_type=item_type)
        elif stage in {"header", "library"}:
            report = audit_unchanged_properties(before, after, stage=stage)
        elif stage == "write":
            report = audit_write_readback(before, after)
        elif stage == "translation":
            report = audit_translation_binding(before, after, kind=kind)
        else:
            raise ValueError(f"unsupported file audit stage: {stage}")
    path = Path(checks_path).resolve(strict=True)
    output = Path(report_path).resolve()
    if not isinstance(report.get("inputs"), list) or not report["inputs"]:
        raise ValueError("file-bound machine inputs required")
    inputs = []
    for binding in report["inputs"]:
        actual = snapshot(Path(binding["path"]))
        if actual["sha256"] != binding["sha256"]:
            raise ValueError("audit input changed")
        inputs.append(actual)
    if output == path or output in {Path(value["path"]) for value in inputs}:
        raise ValueError("report would overwrite an input")
    ledger = check_tools._load_json(path)
    rows = {row["id"]: row for row in ledger["checks"]}
    if len(rows) != len(ledger["checks"]):
        raise ValueError("duplicate rule IDs in ledger")
    encoded = (json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
    output.parent.mkdir(parents=True, exist_ok=True)
    try:
        with output.open("xb") as stream:
            stream.write(encoded)
    except FileExistsError:
        if output.read_bytes() != encoded:
            raise ValueError("use a new report path for changed evidence")
    report_hash = _digest(encoded)
    evidence = {}
    input_ids = []
    for index, binding in enumerate(inputs):
        if binding["size"] == 0:
            raise ValueError("empty input cannot support a whole-file evidence locator")
        eid = f"python-file-{report_hash}-input-{index}"
        evidence[eid] = check_tools.bind_evidence(Path(binding["path"]), {"kind": "whole_file"})
        if evidence[eid]["sha256"] != binding["sha256"]:
            raise ValueError("audit input changed")
        input_ids.append(eid)
    results = []
    for identity, decision in report["checks"].items():
        row = rows.get(identity)
        if (row is None or row["requirement_type"] != "offline_outcome" or
                not re.fullmatch(r"[A-Z]+-A[0-9]+", identity) or
                decision.get("status") not in {"passed", "failed"}):
            continue
        if row["status"] not in {"not_verified", "blocked"}:
            if row["status"] != decision["status"]:
                raise ValueError(f"existing decision conflicts with evaluated file result: {identity}")
            continue
        eid = f"python-file-{report_hash}-{identity}"
        evidence[eid] = check_tools.bind_evidence(output,
            {"kind": "json_pointer", "pointer": "/checks/" + identity})
        if evidence[eid]["sha256"] != report_hash:
            raise ValueError("audit report changed")
        scope = [{"object_id": binding["path"], "locator": {"kind": "whole_file"}}
                 for binding in inputs]
        results.append({"id": identity, "status": decision["status"],
                        "reason": decision["reason"], "scope": scope, "evidence": [eid, *input_ids]})
    for binding in inputs:
        if snapshot(Path(binding["path"])) != binding:
            raise ValueError("audit input changed before recording")
    if results:
        record_check_batch(path, results, evidence=evidence)
    return [result["id"] for result in results]


# Explicitly mapped only. Unmapped never means that a rule requires a model;
# it may already have another checker or require additional implementation.
RULE_HANDLERS = {
    **{f"PROP-A{i:02d}": "audit_properties" for i in (1, 2, 4, 5, 6, 8)},
    "PROP-C10": "audit_properties", "PROP-C18": "audit_properties",
    **{f"PROP-C{i:02d}": "build_properties/repair_properties" for i in (1, 2, 3, 5, 6, 7, 8, 9, 13, 14, 17)},
    **{f"NAME-C{i:02d}": "author_token" for i in (4, 7, 8, 9)},
    **{f"ZOT-C{i:02d}": "collection_ancestry" for i in (5, 6, 7, 8, 9, 12, 13, 14)},
    "ZOT-C20": "snapshot", "ZOT-C21": "snapshot",
    "HDR-A14": "audit_unchanged_properties", "LIB-A08": "audit_unchanged_properties",
    "WRITE-A04": "audit_write_readback", "WRITE-A05": "audit_write_readback",
    "TRN-A17": "audit_translation_binding", "TRN-A25": "audit_translation_binding",
    **{f"BODY-A{i:02d}": "audit_rich_tables" for i in (83, 84, 85, 86, 87, 111)},
}


def coverage_catalog(skills_root: Path, *, progress: Progress | None = None) -> dict:
    """Enumerate actual declarations, including constraints and reference rules."""
    from complete_workflow import _module
    tools = _module("check_records")
    rules, bindings = tools._catalog(Path(skills_root).resolve(), tools._Files(), progress)
    entries = []
    for identity, rule in rules.items():
        entry = rule.catalog_entry()
        entry["new_stage_helper"] = RULE_HANDLERS.get(identity)
        entry["classification"] = "mapped" if identity in RULE_HANDLERS else "not_classified_by_this_module"
        entries.append(entry)
    return {"rule_files": bindings, "rules": entries,
            "counts": dict(Counter(entry["classification"] for entry in entries)),
            "unknown_mapping_ids": sorted(set(RULE_HANDLERS) - rules.keys())}
