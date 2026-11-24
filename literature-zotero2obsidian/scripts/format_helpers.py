"""Offline formatting helpers; call the functions from a Python script.

No function writes a note, renames a file, resolves a literature identity, or
certifies a person's source review. File bindings check bytes, not meaning.
"""

from __future__ import annotations

from collections.abc import Collection, Mapping, Sequence
from hashlib import sha256
import json
import math
from pathlib import Path
import re
from typing import Any
import unicodedata

import yaml


_ILLEGAL = frozenset('<>:"/\\|?*')
_LINK_AMBIGUOUS = frozenset("[]#^|")
_TOKEN = re.compile(r"[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*\Z")
_ANCHOR_TOKEN = re.compile(r"[A-Za-z0-9]+\Z")
_SHA256 = re.compile(r"[0-9a-fA-F]{64}\Z")
_BOLD_SOURCE_LABEL = re.compile(r"^ {0,3}\*\*(?:\\[^\r\n]|[^*\\\r\n])+\*\*")
_RESERVED = frozenset({"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"} | {
    f"{prefix}{number}" for prefix in ("COM", "LPT") for number in "123456789¹²³"
})


def _text(value: Any, label: str, *, allow_empty: bool = False) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a string")
    if not allow_empty and not value.strip():
        raise ValueError(f"{label} must not be empty")
    if any(0xD800 <= ord(character) <= 0xDFFF for character in value):
        raise ValueError(f"{label} contains an unpaired Unicode surrogate")
    return value


def _positive_integer(value: Any, label: str) -> int:
    if type(value) is not int or value < 1:
        raise ValueError(f"{label} must be a positive integer")
    return value


def _utf16_units(value: str) -> int:
    return len(value.encode("utf-16-le")) // 2


def _json_copy(value: Any) -> Any:
    return json.loads(json.dumps(value, ensure_ascii=False, allow_nan=False))


def _file_binding(record: Mapping[str, Any], cache: dict[Path, str]) -> dict[str, str]:
    if not isinstance(record, Mapping):
        raise ValueError("file evidence must contain path and sha256")
    expected = record.get("sha256")
    if not isinstance(expected, str) or not _SHA256.fullmatch(expected):
        raise ValueError("file evidence requires a SHA-256 digest")
    raw_path = record.get("path")
    if not isinstance(raw_path, (str, Path)) or not str(raw_path):
        raise ValueError("file evidence requires a path")
    path = Path(raw_path).resolve()
    if not path.is_file():
        raise ValueError(f"evidence file does not exist: {path}")
    if path not in cache:
        digest = sha256()
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        cache[path] = digest.hexdigest()
    if cache[path] != expected.lower():
        raise ValueError(f"evidence file SHA-256 mismatch: {path}")
    return {"path": str(path), "sha256": cache[path]}


def _source_evidence(record: Any, cache: dict[Path, str]) -> dict[str, Any]:
    if not isinstance(record, Mapping):
        raise ValueError("each source field requires file evidence and a locator")
    locator = record.get("locator")
    if not isinstance(locator, (str, Mapping)) or not locator:
        raise ValueError("source evidence requires a nonempty locator")
    if isinstance(locator, str) and not locator.strip():
        raise ValueError("source evidence requires a nonempty locator")
    return {"file": _file_binding(record.get("file"), cache), "locator": _json_copy(locator)}


def _clean_filename_text(value: str, field: str) -> tuple[str, list[dict[str, Any]]]:
    characters = []
    transformations = []
    for position, character in enumerate(value):
        control = unicodedata.category(character) == "Cc"
        if character not in _ILLEGAL and not control:
            characters.append(character)
            continue
        characters.append("_")
        transformations.append({
            "field": field, "operation": "replace_illegal_character",
            "start": position, "end": position + 1, "unit": "unicode_code_points",
            "before": character, "after": "_",
            "reason": "control_character" if control else "portable_filename_character",
        })
    return "".join(characters), transformations


def _title_prefix(title: str, unit_budget: int, byte_budget: int | None) -> str:
    """Take one prefix in linear time, never splitting a Unicode code point."""
    if _utf16_units(title) <= unit_budget and (byte_budget is None or len(title.encode("utf-8")) <= byte_budget):
        return title
    units = 0
    size = 0
    for position, character in enumerate(title):
        next_units = units + (2 if ord(character) > 0xFFFF else 1)
        next_size = size + len(character.encode("utf-8")) if byte_budget is not None else 0
        if next_units > unit_budget or (byte_budget is not None and next_size > byte_budget):
            return title[:position]
        units, size = next_units, next_size
    return title


def build_naming_plan(
    *,
    year: int | None,
    author_token: str,
    original_title: str,
    source_evidence: Mapping[str, Any],
    delivery_path: Path | None = None,
    old_path: Path | None = None,
    target_directory: Path | None = None,
    no_year_reason: str | None = None,
    unknown_author_reason: str | None = None,
    disambiguation_suffix: str | None = None,
    suffix_evidence: Mapping[str, Any] | None = None,
    max_utf16_units: int = 250,
    filename_max_utf8_bytes: int | None = None,
    full_path_max_utf16_units: int | None = None,
) -> dict[str, Any]:
    """Return a serializable ``run.json.naming`` plan without changing files.

    The caller supplies a source-checked year, already formed author token, and
    original title. Evidence for each field binds an existing file and locator;
    this function checks its hash, not whether the cited source supports a value.
    ``None`` year and the literal ``未知作者`` require explicit reasons. A stable
    disambiguation suffix is caller supplied and also requires source evidence.

    Only the title can be shortened. The limit includes prefix, suffix and .md.
    Optional byte/path limits describe caller-supplied limits; omitted limits
    and the real library remain unchecked. ``delivery_path`` is independent of
    the proposed filename, including when delivery is fixed as candidate.md.
    """
    _positive_integer(max_utf16_units, "max_utf16_units")
    if max_utf16_units > 250:
        raise ValueError("max_utf16_units must not exceed 250")
    for label, limit in (("filename_max_utf8_bytes", filename_max_utf8_bytes),
                         ("full_path_max_utf16_units", full_path_max_utf16_units)):
        if limit is not None:
            _positive_integer(limit, label)
    if full_path_max_utf16_units is not None and target_directory is None:
        raise ValueError("a full-path limit requires target_directory")
    if year is None:
        _text(no_year_reason, "no_year_reason")
        year_token = "无年份"
    else:
        _positive_integer(year, "year")
        if year > 9999:
            raise ValueError("year must contain at most four digits")
        year_token = str(year)
    _text(author_token, "author_token")
    _text(original_title, "original_title")
    if author_token == "未知作者":
        _text(unknown_author_reason, "unknown_author_reason")
    if not isinstance(source_evidence, Mapping):
        raise ValueError("source_evidence is required for year, author_token and original_title")
    cache: dict[Path, str] = {}
    evidence = {field: _source_evidence(source_evidence.get(field), cache)
                for field in ("year", "author_token", "original_title")}
    suffix = ""
    checked_suffix_evidence = None
    if disambiguation_suffix is not None:
        if not isinstance(disambiguation_suffix, str) or not _TOKEN.fullmatch(disambiguation_suffix):
            raise ValueError("disambiguation_suffix must be an explicit ASCII alphanumeric token, optionally hyphenated")
        checked_suffix_evidence = _source_evidence(suffix_evidence, cache)
        suffix = f"-{disambiguation_suffix}"
    elif suffix_evidence is not None:
        raise ValueError("suffix_evidence requires disambiguation_suffix")
    author, author_changes = _clean_filename_text(author_token, "author_token")
    title, title_changes = _clean_filename_text(original_title, "original_title")
    prefix = f"{year_token}-{author}-"
    ending = f"{suffix}.md"
    fixed_units = _utf16_units(prefix + ending)
    fixed_bytes = len((prefix + ending).encode("utf-8"))
    title_budget = max_utf16_units - fixed_units
    byte_budget = None if filename_max_utf8_bytes is None else filename_max_utf8_bytes - fixed_bytes
    used_title = _title_prefix(title, title_budget, byte_budget)
    filename = f"{prefix}{used_title}{ending}" if used_title else None
    transformations = author_changes + title_changes
    if used_title != title:
        transformations.append({
            "field": "original_title", "operation": "truncate_title_suffix",
            "start": len(used_title), "end": len(title), "unit": "unicode_code_points",
            "before": title[len(used_title):], "after": "",
            "reason": "declared_filename_length_limit",
        })
    issues = []
    if filename is None:
        issues.append({
            "code": "fixed_prefix_or_suffix_leaves_no_title_space", "severity": "block",
            "reason": "The year/author prefix, suffix and extension are not shortened; no whole title character fits.",
        })
    ambiguous = sorted(set(filename or "") & _LINK_AMBIGUOUS)
    if ambiguous:
        issues.append({"code": "link_ambiguity", "severity": "review", "characters": ambiguous})
    target_path = Path(target_directory).resolve().joinpath(filename) if target_directory is not None and filename else None
    path_units = _utf16_units(str(target_path)) if target_path is not None else None
    if full_path_max_utf16_units is not None and path_units is not None and path_units > full_path_max_utf16_units:
        issues.append({"code": "full_path_exceeds_declared_limit", "severity": "block"})
    reserved = filename.split(".", 1)[0].rstrip(" .").upper() in _RESERVED if filename else None
    return {
        "schema_version": 1,
        "status": "blocked" if any(issue["severity"] == "block" for issue in issues) else "planned",
        "year": year, "year_token": year_token, "no_year_reason": no_year_reason,
        "author_token": author_token, "unknown_author_reason": unknown_author_reason,
        "original_title": original_title, "source_evidence": evidence,
        "old_path": str(Path(old_path).resolve()) if old_path is not None else None,
        "filename": filename,
        "delivery_path": str(Path(delivery_path).resolve()) if delivery_path is not None else None,
        "target_directory": str(Path(target_directory).resolve()) if target_directory is not None else None,
        "target_path": str(target_path) if target_path is not None else None,
        "disambiguation_suffix": disambiguation_suffix, "suffix_evidence": checked_suffix_evidence,
        "transformations": transformations,
        "title_mapping": {"original": original_title, "cleaned": title, "used": used_title,
                          "truncated": used_title != title},
        "author_mapping": {"original": author_token, "cleaned": author},
        "length": {
            "max_utf16_units": max_utf16_units,
            "filename_utf16_units": _utf16_units(filename) if filename else None,
            "fixed_prefix_suffix_utf16_units": fixed_units,
            "filename_max_utf8_bytes": filename_max_utf8_bytes,
            "filename_utf8_bytes": len(filename.encode("utf-8")) if filename else None,
        },
        "character_checks": {
            "illegal_characters": sorted(set(filename) & _ILLEGAL) if filename else None,
            "control_characters": sorted({character for character in filename if unicodedata.category(character) == "Cc"}) if filename else None,
            "trailing_dot": filename.endswith(".") if filename else None,
            "trailing_space": filename.endswith(" ") if filename else None,
            "reserved_name": reserved, "link_ambiguity_characters": ambiguous,
            "scope": "complete filename including .md; legal title punctuation is preserved",
        },
        "path_check": {
            "status": "not_assessed" if full_path_max_utf16_units is None or target_path is None else "compared_to_declared_limit",
            "utf16_units": path_units, "declared_max_utf16_units": full_path_max_utf16_units,
            "within_declared_limit": path_units <= full_path_max_utf16_units if path_units is not None and full_path_max_utf16_units is not None else None,
            "boundary": "Does not discover filesystem or tool limits, or certify every target-path restriction.",
        },
        "local_conflicts": {"status": "not_checked"},
        "library_conflicts": {"status": "not_checked"},
        "identity_review": {"status": "not_assessed"},
        "semantic_review": {"status": "not_assessed"},
        "overwrite_authorized": False,
        "issues": issues,
    }


def validate_naming_plan(
    plan: Any, *, delivery_path: Path | None = None,
) -> dict[str, Any]:
    """Read and recompute a serialized naming plan; never repair or write it.

    The same ``build_naming_plan`` algorithm checks the original/cleaned/used
    title, author mapping, exact transformations, filename, declared length and
    character checks. Each source field is independently bound to current file
    bytes and a nonempty locator. Locator truth, metadata meaning, directory
    conflict identity, actual filesystem/tool limits and authorization remain
    outside this function. Unknown limits are valid ``not_assessed`` records.

    ``delivery_path`` optionally supplies the workflow's independently selected
    output path. It need not exist and does not replace the proposed filename.
    ``structurally_consistent`` concerns this record only: a correctly recorded
    blocked plan can be consistent and still have ``plan_status == 'blocked'``.
    Findings have one JSON-pointer ``field`` each, plus ``code`` and ``message``.
    Existing version-1 plans without the additive ``target_directory`` field
    remain supported by using their recorded target path when available.
    """
    findings: list[dict[str, str]] = []
    validated_fields: list[str] = []

    def problem(code: str, field: str, message: str) -> None:
        findings.append({"code": code, "field": field, "message": message})

    def report(*, recomputed: bool = False) -> dict[str, Any]:
        return {
            "structurally_consistent": not findings,
            "plan_status": plan.get("status") if isinstance(plan, Mapping) else None,
            "recomputed": recomputed,
            "validated_source_fields": validated_fields,
            "findings": findings,
            "semantic_review": {"status": "not_assessed"},
            "library_conflicts": {"status": "not_assessed"},
            "authorization_granted": False,
            "boundary": "Record consistency and current source-file bindings only; does not certify locator meaning, metadata identity, real path limits, library conflicts, or permission to write.",
        }

    if not isinstance(plan, Mapping):
        problem("NAMING_PLAN_SHAPE", "", "Naming plan must be a mapping, not a filename or status alone.")
        return report()

    required = (
        "schema_version", "status", "year", "year_token", "no_year_reason",
        "author_token", "unknown_author_reason", "original_title", "source_evidence",
        "old_path", "filename", "delivery_path", "target_path",
        "disambiguation_suffix", "suffix_evidence", "transformations",
        "title_mapping", "author_mapping", "length", "character_checks", "path_check", "issues",
    )
    missing = {field for field in required if field not in plan}
    for field in required:
        if field not in missing:
            continue
        problem("NAMING_PLAN_FIELD_MISSING", f"/{field}", f"Missing naming-plan field: {field}.")

    # Validate all supplied field bindings, so one stale field cannot hide the
    # identities of other missing or stale fields. The cache hashes shared files
    # once during this pass; build_naming_plan independently checks them again.
    source_records = plan.get("source_evidence")
    cache: dict[Path, str] = {}
    checked_evidence = {}
    if not isinstance(source_records, Mapping):
        if "source_evidence" not in missing:
            problem("NAMING_PLAN_FIELD_INVALID", "/source_evidence", "source_evidence must map year, author_token and original_title to file bindings and locators.")
    else:
        for field in ("year", "author_token", "original_title"):
            try:
                checked_evidence[field] = _source_evidence(source_records.get(field), cache)
                validated_fields.append(field)
            except (ValueError, TypeError, OSError, RuntimeError) as error:
                problem("NAMING_PLAN_SOURCE_BINDING", f"/source_evidence/{field}", str(error))

    suffix = plan.get("disambiguation_suffix")
    checked_suffix = None
    if suffix is not None:
        try:
            checked_suffix = _source_evidence(plan.get("suffix_evidence"), cache)
            validated_fields.append("disambiguation_suffix")
        except (ValueError, TypeError, OSError, RuntimeError) as error:
            problem("NAMING_PLAN_SOURCE_BINDING", "/suffix_evidence", str(error))

    paths = {}
    invalid_paths = set()
    for field in ("old_path", "delivery_path", "target_path", "target_directory"):
        raw = plan.get(field)
        if raw is None:
            paths[field] = None
            continue
        try:
            paths[field] = Path(_text(raw, field)).resolve()
        except (ValueError, TypeError, OSError, RuntimeError) as error:
            invalid_paths.add(field)
            problem("NAMING_PLAN_FIELD_INVALID", f"/{field}", str(error))
    if delivery_path is not None:
        try:
            expected_delivery = Path(delivery_path).resolve()
        except (ValueError, TypeError, OSError, RuntimeError) as error:
            problem("NAMING_PLAN_DELIVERY_CONTEXT", "/delivery_path", f"Invalid supplied delivery_path: {error}")
        else:
            if "delivery_path" not in missing and "delivery_path" not in invalid_paths and paths.get("delivery_path") != expected_delivery:
                problem("NAMING_PLAN_DELIVERY_MISMATCH", "/delivery_path", "Recorded delivery_path does not match the workflow's supplied output path.")

    containers = {}
    for field in ("length", "path_check"):
        value = plan.get(field)
        if not isinstance(value, Mapping):
            if field not in missing:
                problem("NAMING_PLAN_FIELD_INVALID", f"/{field}", f"{field} must be a mapping.")
            continue
        containers[field] = value
    for container, fields in (("length", ("max_utf16_units", "filename_max_utf8_bytes")),
                              ("path_check", ("declared_max_utf16_units",))):
        if container not in containers:
            continue
        for field in fields:
            if field in containers[container]:
                continue
            problem("NAMING_PLAN_FIELD_MISSING", f"/{container}/{field}", f"Missing declared naming input: {container}.{field}.")

    input_fields = {"year", "no_year_reason", "author_token", "unknown_author_reason", "original_title",
                    "source_evidence", "old_path", "delivery_path", "target_path", "disambiguation_suffix", "suffix_evidence"}
    if missing & input_fields or invalid_paths or len(checked_evidence) != 3 or (suffix is not None and checked_suffix is None):
        return report()
    if len(containers) != 2 or any(item["field"] in {"/length/max_utf16_units", "/length/filename_max_utf8_bytes", "/path_check/declared_max_utf16_units"} for item in findings):
        return report()

    target_directory = paths.get("target_directory")
    target_path = paths.get("target_path")
    if target_directory is None and target_path is not None:
        target_directory = target_path.parent
    path_limit = containers["path_check"]["declared_max_utf16_units"]
    legacy_blocked_path = target_directory is None and target_path is None and plan.get("filename") is None and path_limit is not None
    if path_limit is not None and target_directory is None and not legacy_blocked_path:
        problem("NAMING_PLAN_PATH_INPUT_MISSING", "/target_path", "A declared full-path limit needs a recorded target directory or target path.")
        return report()
    try:
        if legacy_blocked_path:
            _positive_integer(path_limit, "full_path_max_utf16_units")
        rebuilt = build_naming_plan(
            year=plan["year"], author_token=plan["author_token"], original_title=plan["original_title"],
            source_evidence=checked_evidence, delivery_path=paths["delivery_path"], old_path=paths["old_path"],
            target_directory=target_directory, no_year_reason=plan["no_year_reason"],
            unknown_author_reason=plan["unknown_author_reason"], disambiguation_suffix=suffix,
            suffix_evidence=checked_suffix if suffix is not None else plan["suffix_evidence"],
            max_utf16_units=containers["length"]["max_utf16_units"],
            filename_max_utf8_bytes=containers["length"]["filename_max_utf8_bytes"],
            full_path_max_utf16_units=None if legacy_blocked_path else path_limit,
        )
    except (ValueError, TypeError, OSError, RuntimeError) as error:
        labels = {
            "year": "/year", "no_year_reason": "/no_year_reason", "author_token": "/author_token",
            "unknown_author_reason": "/unknown_author_reason", "original_title": "/original_title",
            "disambiguation_suffix": "/disambiguation_suffix", "suffix_evidence": "/suffix_evidence",
            "max_utf16_units": "/length/max_utf16_units", "filename_max_utf8_bytes": "/length/filename_max_utf8_bytes",
            "full_path_max_utf16_units": "/path_check/declared_max_utf16_units",
        }
        field = next((pointer for label, pointer in labels.items() if str(error).startswith(label)), "/source_evidence")
        problem("NAMING_PLAN_INPUT_INVALID", field, str(error))
        return report()

    if legacy_blocked_path and rebuilt["filename"] is None:
        # Older blocked plans did not preserve their directory, but no target
        # path exists to check when no complete filename can be formed.
        rebuilt["path_check"]["declared_max_utf16_units"] = path_limit

    def compare(expected: Any, actual: Any, pointer: str, code: str) -> None:
        if isinstance(expected, Mapping):
            if not isinstance(actual, Mapping):
                problem("NAMING_PLAN_FIELD_INVALID", pointer, "Expected a mapping of computed naming fields.")
                return
            for key, value in expected.items():
                child = f"{pointer}/{key}"
                if key not in actual:
                    problem("NAMING_PLAN_FIELD_MISSING", child, "Missing computed naming field.")
                    continue
                compare(value, actual[key], child, code)
            return
        try:
            equal = json.dumps(expected, ensure_ascii=True, sort_keys=True, allow_nan=False) == json.dumps(actual, ensure_ascii=True, sort_keys=True, allow_nan=False)
        except (ValueError, TypeError, RecursionError):
            equal = False
        if not equal:
            problem(code, pointer, "Recorded value differs from build_naming_plan recomputation using the original fields and declared limits.")

    groups = {
        "schema_version": "NAMING_PLAN_SCHEMA", "year_token": "NAMING_PLAN_MAPPING_MISMATCH",
        "title_mapping": "NAMING_PLAN_MAPPING_MISMATCH", "author_mapping": "NAMING_PLAN_MAPPING_MISMATCH",
        "transformations": "NAMING_PLAN_TRANSFORMATION_MISMATCH", "filename": "NAMING_PLAN_FILENAME_MISMATCH",
        "length": "NAMING_PLAN_LENGTH_MISMATCH", "character_checks": "NAMING_PLAN_CHARACTER_CHECK_MISMATCH",
        "path_check": "NAMING_PLAN_PATH_MISMATCH", "target_path": "NAMING_PLAN_PATH_MISMATCH",
        "delivery_path": "NAMING_PLAN_DELIVERY_MISMATCH", "old_path": "NAMING_PLAN_PATH_MISMATCH",
        "status": "NAMING_PLAN_STATUS_MISMATCH", "issues": "NAMING_PLAN_ISSUES_MISMATCH",
    }
    for field, code in groups.items():
        if field in missing:
            continue
        compare(rebuilt[field], plan[field], f"/{field}", code)
    if "target_directory" in plan:
        compare(rebuilt["target_directory"], plan["target_directory"], "/target_directory", "NAMING_PLAN_PATH_MISMATCH")
    return report(recomputed=True)


def check_local_name_conflicts(
    *, filename: str, directory: Path, case_sensitive: bool, unicode_normalization: str,
) -> dict[str, Any]:
    """Compare names in one existing directory under caller-declared rules.

    This is a nonrecursive directory-entry snapshot, not a Zotero/vault identity
    check. A clear snapshot authorizes no write and can become stale immediately.
    The caller must establish whether these comparison rules match the target.
    """
    _text(filename, "filename")
    if any(character in filename for character in "/\\\0") or filename in (".", ".."):
        raise ValueError("filename must be one directory-entry name")
    if type(case_sensitive) is not bool:
        raise ValueError("case_sensitive must be explicitly True or False")
    if unicode_normalization not in ("none", "NFC", "NFD"):
        raise ValueError("unicode_normalization must be none, NFC or NFD")
    root = Path(directory).resolve()
    if not root.is_dir():
        raise ValueError(f"local comparison directory does not exist: {root}")

    def comparable(name: str) -> str:
        normalized = name if unicode_normalization == "none" else unicodedata.normalize(unicode_normalization, name)
        return normalized if case_sensitive else normalized.casefold()

    names = sorted(entry.name for entry in root.iterdir())
    wanted = comparable(filename)
    matches = [{"name": name, "path": str(root.joinpath(name))} for name in names if comparable(name) == wanted]
    snapshot_bytes = json.dumps(names, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return {
        "status": "name_collision" if matches else "clear_in_directory_snapshot",
        "directory": str(root), "filename": filename, "entries": names,
        "entry_names_sha256": sha256(snapshot_bytes).hexdigest(), "matches": matches,
        "comparison": {"case_sensitive": case_sensitive, "unicode_normalization": unicode_normalization,
                       "case_insensitive_algorithm": "Unicode casefold", "policy_origin": "caller"},
        "scope": "nonrecursive names only; entry contents, identities and other directories are not checked",
        "identity_review": {"status": "not_assessed"},
        "library_conflicts": {"status": "not_checked"}, "overwrite_authorized": False,
    }


def new_figure_table_anchor(
    *, source_token: str, figure_number_token: str, occupied_ids: Collection[str] = (),
) -> str:
    """Generate only a new LINK-C133 anchor; never rename or migrate an old ID.

    Tokens are caller-registered ASCII alphanumeric values, not inferred source
    identities or rewritten figure labels. Reuse an existing verified anchor in
    the calling code instead. Occupied IDs may include or omit their leading ^.
    """
    for label, token in (("source_token", source_token), ("figure_number_token", figure_number_token)):
        if not isinstance(token, str) or not _ANCHOR_TOKEN.fullmatch(token):
            raise ValueError(f"{label} must be a registered ASCII alphanumeric token")
    if not isinstance(occupied_ids, Collection) or isinstance(occupied_ids, (str, bytes)):
        raise ValueError("occupied_ids must be a collection of ID strings")
    if any(not isinstance(value, str) for value in occupied_ids):
        raise ValueError("occupied_ids must be a collection of ID strings")
    anchor = f"^tbl-fig-{source_token}-{figure_number_token}"
    if anchor in occupied_ids or anchor[1:] in occupied_ids:
        raise ValueError(f"new anchor is already occupied: {anchor}")
    return anchor


def build_rich_table_block(
    data: Mapping[str, Any], *, anchor: str, wrapped: bool = True, newline: str = "\n",
) -> str:
    """Serialize supplied v2/v3 table data as block YAML with its supplied ID.

    Supply the complete table, including every source cell, locked=True and
    theme='grid' for every sheet. This function neither invents those fields nor
    changes captions, cells, mathematics, identifiers, merges or reference links.
    It writes no file and does not certify source completeness or link targets.
    Run validate_note and the source checks on the assembled note afterwards.
    """
    if not isinstance(data, Mapping):
        raise ValueError("data must be the complete Rich Table mapping")
    if not isinstance(anchor, str) or not re.fullmatch(r"\^?tbl-[A-Za-z0-9-]+", anchor):
        raise ValueError("anchor must be the supplied tbl- block ID, optionally prefixed by ^")
    if type(wrapped) is not bool:
        raise ValueError("wrapped must be True or False")
    if newline not in {"\n", "\r\n"}:
        raise ValueError("newline must be LF or CRLF")
    try:
        prepared = _json_copy(dict(data))
    except (TypeError, ValueError, RecursionError) as error:
        raise ValueError("table data must contain finite, acyclic JSON-compatible values") from error
    if prepared != data:
        raise ValueError("table data must use JSON string keys and list values without type conversion")
    version = prepared.get("version")
    if type(version) is not int or version not in (2, 3):
        raise ValueError("version must be the integer 2 or 3")
    sheets = prepared.get("sheets") if version == 3 else [prepared]
    if not isinstance(sheets, list) or not sheets:
        raise ValueError("a v3 workbook requires a nonempty sheets list")
    for index, sheet in enumerate(sheets, 1):
        if not isinstance(sheet, dict):
            raise ValueError(f"sheet {index} must be a mapping")
        if sheet.get("locked") is not True:
            raise ValueError(f"sheet {index} must explicitly supply locked=True")
        if sheet.get("theme") != "grid":
            raise ValueError(f"sheet {index} must explicitly supply theme='grid'")
        for field in ("columns", "rows"):
            if not isinstance(sheet.get(field), list) or not sheet[field]:
                raise ValueError(f"sheet {index} must supply nonempty {field}")
    payload = yaml.safe_dump(prepared, allow_unicode=True, sort_keys=False,
                             default_flow_style=False, width=100000)
    if yaml.safe_load(payload) != prepared:
        raise ValueError("YAML serialization changed a supplied field value")
    if wrapped:
        payload = "---\n" + payload + "---\n"
    # A cell can contain Markdown code. Its longest backtick run must not close
    # the surrounding data fence, even when YAML emits a multiline string.
    longest = max((len(match[0]) for match in re.finditer(r"`+", payload)), default=0)
    fence = "`" * max(3, longest + 1)
    block = f"{fence}rich-table\n{payload}{fence}\n^{anchor.removeprefix('^')}\n"
    return block if newline == "\n" else block.replace("\n", newline)


def starts_with_bold_source_label(text: str) -> bool:
    """Recognize an explicit bold label, including escaped literal punctuation."""
    return isinstance(text, str) and _BOLD_SOURCE_LABEL.match(text) is not None


def build_figure_block(
    images: Sequence[Mapping[str, str]], *, caption: str, anchor: str, newline: str = "\n",
) -> str:
    """Build one contiguous image/caption/anchor block from explicit content.

    Unnumbered figures use a source-supported label supplied in caption. The
    helper invents neither a figure number nor descriptive caption text and
    does not inspect image bytes or certify source completeness.
    """
    if isinstance(images, (str, bytes)) or not isinstance(images, Sequence) or not images:
        raise ValueError("images must be a nonempty sequence of target/alt mappings")
    if not isinstance(anchor, str) or not re.fullmatch(r"\^?[A-Za-z0-9-]+", anchor):
        raise ValueError("anchor must be an explicit valid block ID")
    if newline not in {"\n", "\r\n"}:
        raise ValueError("newline must be LF or CRLF")
    if not isinstance(caption, str) or caption != caption.strip() or "\n" in caption or "\r" in caption:
        raise ValueError("caption must be one explicitly supplied Markdown line")
    if not starts_with_bold_source_label(caption):
        raise ValueError("caption must start with its explicit bold source label")
    lines = []
    for item in images:
        if not isinstance(item, Mapping) or set(item) != {"target", "alt"}:
            raise ValueError("each image must contain exactly target and alt")
        target = _text(item["target"], "image target")
        alt = _text(item["alt"], "image alt", allow_empty=True)
        if any(char in target for char in "<>\r\n") or target != target.strip():
            raise ValueError("supply an unwrapped single-line image target")
        if any(char in alt for char in "\r\n"):
            raise ValueError("image alt must be one line")
        escaped_alt = alt.replace("\\", "\\\\").replace("[", "\\[").replace("]", "\\]").replace("&", "&amp;")
        # Inputs are literal resource paths. Preserve entity-like names and
        # URI punctuation after Markdown and URL decoding by the consumer.
        escaped_target = (target.replace("%", "%25").replace("#", "%23").replace("?", "%3F")
                          .replace("\\", "\\\\").replace("&", "&amp;"))
        lines.append(f"![{escaped_alt}](<{escaped_target}>)  ")
    lines.extend((caption, "^" + anchor.removeprefix("^")))
    return newline.join(lines) + newline


def plan_supplement_figure_caption(
    original_number: str | None,
    caption: str,
    *,
    display_number: str | None = None,
    original_number_reason: str | None = None,
) -> dict[str, Any]:
    """Format a new SI figure caption from explicit source-number facts.

    The caller supplies the actually observed original number (or its actual
    absence). Existing S prefixes and literal punctuation are preserved. The
    function does not read a PDF, invent a source number, or rename any anchor.
    """
    body = _text(caption, "caption")
    if body != body.strip():
        raise ValueError("caption must not have leading or trailing whitespace")

    def number_text(value: Any, label: str) -> str:
        text = _text(value, label)
        if text != text.strip() or any(unicodedata.category(character) in {"Cc", "Cf", "Zl", "Zp"}
                                      for character in text):
            raise ValueError(f"{label} must be one literal number without controls, line breaks or outer whitespace")
        return text

    def escaped(value: str) -> str:
        # The number is literal text, never math, a comment, strikethrough or
        # an HTML entity. Keep the caller's original metadata unescaped.
        return re.sub(r"([\\`*_\[\]<>$~%&])", r"\\\1", value)

    if original_number is None:
        reason = _text(original_number_reason, "original_number_reason")
        display = number_text(display_number, "display_number")
        if not display.lower().startswith("s"):
            raise ValueError("An explicitly generated SI navigation number needs its S prefix")
        return {
            "original_number": None, "original_number_reason": reason,
            "display_number": display, "display_number_origin": "generated_navigation",
            "caption": f"**图{escaped(display)}：** {body}（来源无图号）",
        }
    original = number_text(original_number, "original_number")
    if original_number_reason is not None:
        raise ValueError("Use an absence reason only for an actually unnumbered source figure")
    expected = original if original.lower().startswith("s") else "S" + original
    display = expected if display_number is None else number_text(display_number, "display_number")
    if display != expected:
        raise ValueError("Preserve the original number; only add one S prefix when it is absent")
    note = f"（来源原号：{escaped(original)}）" if display != original else ""
    return {
        "original_number": original, "display_number": display,
        "display_number_origin": "source_literal" if display == original else "normalized_source",
        "caption": f"**图{escaped(display)}：** {body}{note}",
    }


def apply_visible_text_edits(
    *, text_file: Mapping[str, Any], edits: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    """Apply explicit, nonoverlapping source transcriptions to a UTF-8 snapshot.

    Each edit supplies an original-code-point range [start, end), expected_text,
    a hashed visible-page file/page/bbox/coordinate_system/text, and a resolved
    decision with selected_text, reason, reviewer and observed=True. The selected
    text must equal the caller's recorded visible reading. Observation is a
    caller declaration: the helper neither sees the page nor certifies it.

    Replacements are assembled once against the original text, so a soft hyphen
    and a visible hyphen cannot be duplicated by sequential global rewrites.
    No dictionary correction, automatic hyphen removal, or file write occurs.
    """
    cache: dict[Path, str] = {}
    binding = _file_binding(text_file, cache)
    raw_bytes = Path(binding["path"]).read_bytes()
    if sha256(raw_bytes).hexdigest() != binding["sha256"]:
        raise ValueError("text snapshot changed while being read")
    original = raw_bytes.decode("utf-8")
    if isinstance(edits, (str, bytes)) or not isinstance(edits, Sequence):
        raise ValueError("edits must be a sequence of explicit edit records")
    prepared = []
    identifiers = set()
    for edit in edits:
        if not isinstance(edit, Mapping):
            raise ValueError("each edit must be a mapping")
        identifier = _text(edit.get("id"), "edit.id")
        if identifier in identifiers:
            raise ValueError(f"duplicate edit ID: {identifier}")
        identifiers.add(identifier)
        start, end = edit.get("start"), edit.get("end")
        if type(start) is not int or type(end) is not int or not 0 <= start < end <= len(original):
            raise ValueError("edit range must be a nonempty original-text code-point range")
        expected = _text(edit.get("expected_text"), "expected_text", allow_empty=True)
        if original[start:end] != expected:
            raise ValueError(f"stale or incorrect original text for edit: {identifier}")
        visible = edit.get("visible_page")
        if not isinstance(visible, Mapping):
            raise ValueError("an edit requires visible-page evidence")
        page_file = _file_binding(visible.get("file"), cache)
        page = _positive_integer(visible.get("page"), "visible_page.page")
        bbox = visible.get("bbox")
        if not isinstance(bbox, (list, tuple)) or len(bbox) != 4:
            raise ValueError("visible_page.bbox requires four coordinates")
        if any(type(value) not in (int, float) or not math.isfinite(value) for value in bbox):
            raise ValueError("visible_page.bbox coordinates must be finite numbers")
        if not 0 <= bbox[0] < bbox[2] or not 0 <= bbox[1] < bbox[3]:
            raise ValueError("visible_page.bbox must have positive area in nonnegative coordinates")
        coordinates = visible.get("coordinate_system")
        if not isinstance(coordinates, (str, Mapping)) or not coordinates:
            raise ValueError("visible_page.coordinate_system must be explicitly recorded")
        if isinstance(coordinates, str) and not coordinates.strip():
            raise ValueError("visible_page.coordinate_system must be explicitly recorded")
        visible_text = _text(visible.get("text"), "visible_page.text", allow_empty=True)
        decision = edit.get("decision")
        if not isinstance(decision, Mapping) or decision.get("status") != "resolved":
            raise ValueError("only a caller-resolved decision can be applied")
        if decision.get("observed") is not True:
            raise ValueError("the caller must explicitly declare that the visible page was observed")
        selected = _text(decision.get("selected_text"), "decision.selected_text", allow_empty=True)
        reason = _text(decision.get("reason"), "decision.reason")
        reviewer = _text(decision.get("reviewer"), "decision.reviewer")
        if selected != visible_text:
            raise ValueError("selected_text must equal the caller's recorded visible reading")
        prepared.append((start, end, selected, {
            "id": identifier,
            "text_layer": {"file": binding, "locator": {
                "kind": "characters", "unit": "unicode_code_points", "start": start,
                "end": end, "end_exclusive": True,
            }, "text": expected},
            "visible_page": {"file": page_file, "page": page, "bbox": list(bbox),
                             "coordinate_system": _json_copy(coordinates), "text": visible_text},
            "decision": {"status": "resolved", "selected_text": selected, "reason": reason,
                         "reviewer": reviewer, "observed": True, "declaration_origin": "caller"},
        }))
    prepared.sort(key=lambda row: row[0])
    parts = []
    cursor = 0
    for start, end, selected, _record in prepared:
        if start < cursor:
            raise ValueError("edit ranges overlap in the original text")
        parts.extend((original[cursor:start], selected))
        cursor = end
    parts.append(original[cursor:])
    return {
        "text": "".join(parts), "text_file": binding,
        "text_visual_conflicts": [record for _start, _end, _selected, record in prepared],
        "observation_certified": False, "semantic_review": {"status": "not_assessed"},
        "boundary": "Applies caller-declared readings only; hashes bind files, not observation or semantic correctness.",
    }
