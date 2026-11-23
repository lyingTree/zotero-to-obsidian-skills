"""Keep one evidence record per atomic skill ID; never infer an acceptance result.

Call prepare_check_records(...) and validate_check_records(path) from a Python
script. A clean report establishes record shape and current file bindings only.
It cannot establish that an observation happened or that a source was understood.
"""

from __future__ import annotations

from collections import Counter
from collections.abc import Callable, Mapping, Sequence
from copy import deepcopy
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import tempfile
from typing import Any


CHECKS_PATH: Path | None = None
REPORT_PATH: Path | None = None
SCHEMA_VERSION = 1
STATUSES = frozenset({"passed", "failed", "blocked", "not_verified", "not_applicable"})
REQUIREMENT_TYPES = frozenset({"unclassified", "execution_process", "offline_outcome", "live_environment"})
EVIDENCE_TYPES = frozenset({"source_location", "offline_result", "execution_trace", "environment_observation"})
ENVIRONMENT_KINDS = frozenset({"actual_ui", "native_cache", "actual_cli"})
_ID = r"[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*-[A-Z][0-9]+"
_RULE = re.compile(rf"^ {{0,3}}(?:[-+*]|\d+[.)])\s+(?:\[[ xX]\]\s+)?\[(?P<id>{_ID})\]\s+(?P<text>\S.*)$")
_FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
_TYPE = re.compile(r"<!--\s*check-type:\s*(?P<type>[a-z_]+)(?:\s*;\s*environment:\s*(?P<environment>[a-z_]+))?\s*-->\s*$")
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
Progress = Callable[[int, int, str], None]


@dataclass(frozen=True)
class CheckFinding:
    code: str
    object_id: str
    message: str
    level: str = "error"

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass
class CheckRecordsReport:
    findings: list[CheckFinding] = field(default_factory=list)
    catalog_count: int = 0
    expected_count: int = 0
    recorded_count: int = 0
    status_counts: dict[str, int] = field(default_factory=dict)
    pending_ids: list[str] = field(default_factory=list)
    blocked_ids: list[str] = field(default_factory=list)
    unclassified_ids: list[str] = field(default_factory=list)
    source_fidelity: str = "not_assessed"
    execution_truth: str = "not_assessed"
    live_behavior: str = "not_assessed"
    independent_review_required: bool = True
    evidence_boundary: str = (
        "Only atomic-ID coverage, record structure, existing evidence locations and "
        "current file hashes are checked. Status, scope, applicability and observation "
        "claims are supplied by the recorder. Independent comparison with the source "
        "and independent observation of process or actual UI remain necessary."
    )

    @property
    def structurally_consistent(self) -> bool:
        return not any(item.level == "error" for item in self.findings)

    @property
    def recorded_checks_complete(self) -> bool:
        """Whether every record is resolved, not whether any claim is true."""
        return (self.structurally_consistent and self.expected_count > 0
                and not self.pending_ids and not self.blocked_ids and not self.unclassified_ids)

    def to_dict(self) -> dict:
        return {**asdict(self), "structurally_consistent": self.structurally_consistent,
                "recorded_checks_complete": self.recorded_checks_complete}


@dataclass(frozen=True)
class AtomicRule:
    id: str
    skill: str
    objective: str
    path: str
    line: int
    sha256: str
    declared_type: str = "unclassified"
    declared_environment_kind: str | None = None

    def location(self) -> dict:
        return {"path": self.path, "line": self.line, "sha256": self.sha256}

    def catalog_entry(self) -> dict:
        return {"id": self.id, "skill": self.skill, "objective": self.objective,
                "rule": self.location(), "declared_type": self.declared_type,
                "declared_environment_kind": self.declared_environment_kind}


class _Files:
    """Hash each distinct file once per operation, including shared evidence."""

    def __init__(self):
        self.hashes: dict[Path, str] = {}
        self.texts: dict[Path, str] = {}

    def digest(self, path: Path) -> str:
        if path in self.hashes:
            return self.hashes[path]
        digest = hashlib.sha256()
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        value = digest.hexdigest()
        self.hashes[path] = value
        return value

    def text(self, path: Path) -> str:
        if path in self.texts:
            return self.texts[path]
        data = path.read_bytes()
        self.hashes[path] = hashlib.sha256(data).hexdigest()
        self.texts[path] = data.decode("utf-8")
        return self.texts[path]


def _unique_pairs(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"Duplicate JSON key: {key}")
        value[key] = item
    return value


def _load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_unique_pairs)


def _write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent,
                                     prefix=path.name + ".", suffix=".tmp", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
            stream.write("\n")
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def _rule_files(skills_root: Path) -> list[tuple[str, Path]]:
    if not skills_root.is_dir():
        raise ValueError(f"skills_root is not a directory: {skills_root}")
    # Deliberately inspect only the actual root and immediate skill directories.
    # A repository's test fixtures, archived skills and execution outputs are not skills.
    directories = [skills_root] if skills_root.joinpath("SKILL.md").is_file() else []
    directories.extend(path for path in sorted(skills_root.iterdir())
                       if path.is_dir() and path.name != "runs" and path.joinpath("SKILL.md").is_file())
    files = []
    for directory in directories:
        files.append((directory.name, directory.joinpath("SKILL.md")))
        references = directory.joinpath("references")
        if not references.is_dir():
            continue
        files.extend((directory.name, path) for path in sorted(references.rglob("*.md"))
                     if path.is_file() and "runs" not in path.relative_to(references).parts)
    if not files:
        raise ValueError("No actual SKILL.md files were found under skills_root.")
    return files


def _atomic_lines(text: str):
    """Read declarations, not inline ID mentions, code examples or comments."""
    fence = None
    in_comment = False
    in_frontmatter = False
    for number, line in enumerate(text.splitlines(), 1):
        if number == 1 and line.strip() == "---":
            in_frontmatter = True
            continue
        if in_frontmatter:
            if line.strip() in {"---", "..."}:
                in_frontmatter = False
            continue
        if fence is not None:
            if re.fullmatch(rf" {{0,3}}{re.escape(fence[0])}{{{fence[1]},}}\s*", line):
                fence = None
            continue
        opening = _FENCE.match(line)
        if opening and not in_comment:
            fence = (opening[1][0], len(opening[1]))
            continue
        declared = _TYPE.search(line) if not in_comment else None
        declared_type = declared["type"] if declared else "unclassified"
        environment_kind = declared["environment"] if declared else None
        if declared_type == "live_environment" and environment_kind is None:
            environment_kind = "actual_ui"
        if declared:
            line = line[:declared.start()].rstrip()
        visible = []
        cursor = 0
        while cursor < len(line):
            if in_comment:
                closing = line.find("-->", cursor)
                if closing < 0:
                    break
                cursor, in_comment = closing + 3, False
                continue
            opening_comment = line.find("<!--", cursor)
            if opening_comment < 0:
                visible.append(line[cursor:])
                break
            visible.append(line[cursor:opening_comment])
            cursor, in_comment = opening_comment + 4, True
        match = _RULE.match("".join(visible).rstrip())
        if match is None:
            continue
        if declared_type not in REQUIREMENT_TYPES:
            raise ValueError(f"Invalid check-type at line {number}: {declared_type}")
        if environment_kind is not None and (declared_type != "live_environment" or environment_kind not in ENVIRONMENT_KINDS):
            raise ValueError(f"Invalid environment kind at line {number}: {environment_kind}")
        yield number, match["id"], match["text"], declared_type, environment_kind


def _catalog(skills_root: Path, files: _Files, progress: Progress | None = None):
    paths = _rule_files(skills_root)
    rules: dict[str, AtomicRule] = {}
    bindings = []
    for index, (skill, path) in enumerate(paths, 1):
        path = path.resolve()
        text = files.text(path)
        digest = files.digest(path)
        bindings.append({"path": str(path), "sha256": digest})
        for line, identity, objective, declared_type, environment_kind in _atomic_lines(text):
            if identity in rules:
                previous = rules[identity]
                raise ValueError(f"Duplicate atomic ID {identity}: {previous.path}:{previous.line} and {path}:{line}")
            rules[identity] = AtomicRule(identity, skill, objective, str(path), line, digest, declared_type, environment_kind)
        if progress is not None:
            progress(index, len(paths), "rule_files")
    if not rules:
        raise ValueError("The skill files contain no atomic rule declarations.")
    return rules, bindings


def bind_file(path: Path) -> dict:
    """Return an existing file's actual path/hash without changing any record."""
    path = Path(path).resolve()
    if not path.is_file():
        raise ValueError(f"Bound file does not exist: {path}")
    return {"path": str(path), "sha256": _Files().digest(path)}


def bind_evidence(path: Path, locator: dict, *, evidence_type: str = "offline_result",
                  observation: dict | None = None) -> dict:
    """Bind an existing file; this helper does not claim its content proves a check.

    Locators use lines/start/end, pdf_pages/start/end, json_pointer/pointer,
    image_region/bbox, or whole_file. Observation metadata is supplied explicitly.
    """
    if evidence_type not in EVIDENCE_TYPES:
        raise ValueError(f"Unknown evidence_type: {evidence_type}")
    result = {**bind_file(path),
              "locator": deepcopy(locator), "evidence_type": evidence_type}
    if observation is not None:
        result["observation"] = deepcopy(observation)
    return result


def _selected_ids(rules: Mapping[str, AtomicRule], mode: Any, additional_ids: Any) -> set[str]:
    if not isinstance(mode, str) or mode not in {"acceptance", "all"}:
        raise ValueError("rule_selection must be acceptance or all.")
    if isinstance(additional_ids, str) or not isinstance(additional_ids, Sequence):
        raise ValueError("additional_ids must be an explicit sequence of individual IDs.")
    if not all(_nonempty(identity) for identity in additional_ids):
        raise ValueError("additional_ids must contain nonempty ID strings.")
    if len(set(additional_ids)) != len(additional_ids):
        raise ValueError("additional_ids must not repeat an ID.")
    unknown = set(additional_ids) - rules.keys()
    if unknown:
        raise ValueError(f"Unknown additional rule IDs: {sorted(unknown)}")
    selected = set(rules) if mode == "all" else {identity for identity in rules if re.search(r"-A[0-9]+$", identity)}
    return selected | set(additional_ids)


def prepare_check_records(
    skills_root: Path,
    output_path: Path,
    *,
    sources: Mapping[str, Path],
    candidates: Mapping[str, Path] | None = None,
    candidate_path: Path | None = None,
    scopes: Mapping[str, list[dict]] | None = None,
    requirement_types: Mapping[str, str] | None = None,
    environment_kinds: Mapping[str, str] | None = None,
    rule_selection: str = "acceptance",
    additional_ids: Sequence[str] = (),
    inventory_path: Path | None = None,
    overwrite: bool = False,
    progress: Progress | None = None,
) -> Path:
    """Create a full-workflow, all-not_verified ledger from actual skill files.

    The catalog includes every atomic declaration. By default checks contains
    every A-numbered acceptance ID, including references. Use rule_selection=all
    or additional_ids to also track constraints. Neither option removes A IDs.
    Empty scopes remain empty for the recorder to fill; validation diagnoses them.
    Types come only from an explicit ID mapping or a declaration's trailing
    ``<!-- check-type: ... -->`` annotation. No keyword changes a status or scope.
    ``inventory_path`` optionally receives checks_path, but its hash is never bound.
    """
    skills_root, output_path = Path(skills_root).resolve(), Path(output_path).resolve()
    if output_path.exists() and not overwrite:
        raise FileExistsError(f"Refusing to replace an existing ledger: {output_path}")
    if not sources:
        raise ValueError("At least one actual source file must be bound.")
    if candidates is not None and candidate_path is not None:
        raise ValueError("Use candidates or candidate_path, not both.")
    candidates = candidates if candidates is not None else ({"candidate": candidate_path} if candidate_path else {})
    scopes, requirement_types, environment_kinds = scopes or {}, requirement_types or {}, environment_kinds or {}
    files = _Files()
    rules, rule_bindings = _catalog(skills_root, files, progress)
    selected_ids = _selected_ids(rules, rule_selection, additional_ids)
    unknown = (set(scopes) | set(requirement_types) | set(environment_kinds)) - rules.keys()
    if unknown:
        raise ValueError(f"Unknown rule IDs in explicit mappings: {sorted(unknown)}")
    bindings = {}
    protected = {Path(item["path"]) for item in rule_bindings}
    for label, mapping in (("sources", sources), ("candidates", candidates)):
        bindings[label] = []
        for identity, value in mapping.items():
            if not isinstance(identity, str) or not identity.strip():
                raise ValueError(f"Empty {label} binding identity")
            path = Path(value).resolve()
            if not path.is_file():
                raise ValueError(f"Bound file does not exist: {path}")
            protected.add(path)
            bindings[label].append({"id": identity, "path": str(path), "sha256": files.digest(path)})
    inventory = None
    if inventory_path is not None:
        inventory_path = Path(inventory_path).resolve()
        inventory = _load_json(inventory_path)
        if not isinstance(inventory, dict):
            raise ValueError("inventory_path must contain a JSON object.")
        if inventory_path in protected:
            raise ValueError("Do not hash-bind the mutable inventory as a source, candidate or rule file.")
        protected.add(inventory_path)
    if output_path in protected:
        raise ValueError("The ledger output would overwrite a bound input file.")
    checks = []
    for rule in rules.values():
        if rule.id not in selected_ids:
            continue
        requirement_type = requirement_types.get(rule.id, rule.declared_type)
        if requirement_type not in REQUIREMENT_TYPES:
            raise ValueError(f"Unknown requirement type for {rule.id}: {requirement_type}")
        if rule.declared_type != "unclassified" and requirement_type != rule.declared_type:
            raise ValueError(f"Explicit type conflicts with the source declaration for {rule.id}")
        environment_kind = environment_kinds.get(rule.id, rule.declared_environment_kind or
                                                 ("actual_ui" if requirement_type == "live_environment" else None))
        if environment_kind is not None and (requirement_type != "live_environment" or environment_kind not in ENVIRONMENT_KINDS):
            raise ValueError(f"Invalid environment kind for {rule.id}: {environment_kind}")
        if rule.declared_environment_kind is not None and environment_kind != rule.declared_environment_kind:
            raise ValueError(f"Explicit environment kind conflicts with the source declaration for {rule.id}")
        checks.append({"id": rule.id, "skill": rule.skill, "objective": rule.objective,
                       "rule": rule.location(), "requirement_type": requirement_type,
                       "environment_kind": environment_kind,
                       "scope": deepcopy(scopes.get(rule.id, [])), "status": "not_verified",
                       "evidence": [], "reason": "尚未逐项核验；此骨架不构成执行、内容或界面通过证据。",
                       "applicability_condition": ""})
    value = {"schema_version": SCHEMA_VERSION,
             "created_at": datetime.now(timezone.utc).isoformat(),
             "skills_root": str(skills_root), "rule_files": rule_bindings,
             "catalog": [rule.catalog_entry() for rule in rules.values()],
             "rule_selection": {"mode": rule_selection, "additional_ids": list(additional_ids)},
             "bindings": bindings, "evidence": {}, "checks": checks}
    _write_json(output_path, value)
    if inventory is not None:
        inventory["checks_path"] = str(output_path)
        _write_json(inventory_path, inventory)
    return output_path


def _nonempty(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _bound_path(value: Any, base: Path) -> Path:
    if not _nonempty(value):
        raise ValueError("A nonempty file path is required.")
    path = Path(value)
    return (path if path.is_absolute() else base.joinpath(path)).resolve()


def _validate_binding(binding: Any, identity: str, base: Path, files: _Files,
                      report: CheckRecordsReport) -> Path | None:
    if not isinstance(binding, dict):
        report.findings.append(CheckFinding("CHECK_FILE_BINDING_INVALID", identity, "A file binding must be an object."))
        return None
    try:
        path = _bound_path(binding.get("path"), base)
        if not path.is_file():
            raise ValueError(f"Bound file does not exist: {path}")
        actual = files.digest(path)
    except (OSError, ValueError) as error:
        report.findings.append(CheckFinding("CHECK_FILE_MISSING", identity, str(error)))
        return None
    expected = binding.get("sha256")
    if not isinstance(expected, str) or _SHA256.fullmatch(expected) is None:
        report.findings.append(CheckFinding("CHECK_HASH_MISSING", identity, "A recorded SHA-256 digest is required."))
        return None
    if actual != expected:
        report.findings.append(CheckFinding("CHECK_HASH_CHANGED", identity, f"File bytes changed: {path}"))
        return None
    return path


def _integer(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _locator_error(path: Path, locator: Any, files: _Files) -> str | None:
    if not isinstance(locator, dict) or not locator:
        return "Evidence needs a structured locator, not a free-form claim."
    kind = locator.get("kind")
    if kind == "whole_file":
        return None if path.stat().st_size else "The referenced whole file is empty."
    if kind == "lines":
        start, end = locator.get("start"), locator.get("end")
        if not _integer(start) or not _integer(end) or not 1 <= start <= end:
            return "Line locations require integer 1 <= start <= end."
        if end > len(files.text(path).splitlines()):
            return "The evidence line range is outside the actual file."
        return None
    if kind == "json_pointer":
        pointer = locator.get("pointer")
        if not isinstance(pointer, str) or (pointer != "" and not pointer.startswith("/")):
            return "A JSON pointer must be empty (root) or start with /."
        value = json.loads(files.text(path), object_pairs_hook=_unique_pairs)
        for part in pointer.split("/")[1:] if pointer else []:
            if re.search(r"~(?![01])", part):
                return "Invalid JSON pointer escape."
            part = part.replace("~1", "/").replace("~0", "~")
            if isinstance(value, dict) and part in value:
                value = value[part]
                continue
            if isinstance(value, list) and re.fullmatch(r"0|[1-9][0-9]*", part) and int(part) < len(value):
                value = value[int(part)]
                continue
            return "The JSON pointer does not identify an existing value."
        return None
    if kind == "pdf_pages":
        start, end = locator.get("start"), locator.get("end")
        if not _integer(start) or not _integer(end) or not 1 <= start <= end:
            return "PDF page locations require integer 1 <= start <= end."
        import fitz
        with fitz.open(path) as document:
            if not document.is_pdf or end > len(document):
                return "The evidence page range is outside the actual PDF."
        return None
    if kind == "image_region":
        from PIL import Image
        bbox = locator.get("bbox")
        if not isinstance(bbox, list) or len(bbox) != 4:
            return "An image region needs four pixel coordinates."
        if not all(_integer(value) for value in bbox):
            return "Image-region coordinates must be integers."
        with Image.open(path) as image:
            left, top, right, bottom = bbox
            if not (0 <= left < right <= image.width and 0 <= top < bottom <= image.height):
                return "The evidence image region is outside the actual image."
        return None
    return f"Unsupported evidence locator kind: {kind!r}"


def _observation_valid(evidence: dict, requirement_type: str, environment_kind: str | None = None) -> bool:
    expected = {"execution_process": ("execution_trace", "direct_event"),
                "live_environment": ("environment_observation", environment_kind)}
    evidence_type, basis = expected[requirement_type]
    if evidence.get("evidence_type") != evidence_type:
        return False
    observation = evidence.get("observation")
    if not isinstance(observation, dict) or observation.get("observed") is not True:
        return False
    if observation.get("basis") != basis or not _nonempty(observation.get("observer")):
        return False
    observed_at = observation.get("observed_at")
    if not _nonempty(observed_at):
        return False
    try:
        when = datetime.fromisoformat(observed_at.replace("Z", "+00:00"))
    except ValueError:
        return False
    if when.tzinfo is None:
        return False
    return requirement_type != "live_environment" or _nonempty(observation.get("environment"))


def _scope_valid(scope: Any) -> bool:
    if not isinstance(scope, list) or not scope:
        return False
    for item in scope:
        if not isinstance(item, dict) or not _nonempty(item.get("object_id")):
            return False
        location = item.get("locator")
        if not _nonempty(location) and not (isinstance(location, dict) and location):
            return False
    return True


def validate_check_records(path: Path, *, progress: Progress | None = None) -> CheckRecordsReport:
    """Validate each record and actual file location without assigning results.

    Honest blocked/not_verified records with scope and reasons are valid pending
    records. They do not prevent offline candidate generation. An unclassified
    record may remain pending but cannot claim passed, failed or not_applicable.
    """
    report = CheckRecordsReport()
    path = Path(path).resolve()
    base, files = path.parent, _Files()
    try:
        ledger = _load_json(path)
    except (OSError, ValueError) as error:
        report.findings.append(CheckFinding("CHECK_LEDGER_UNREADABLE", "checks", str(error)))
        return report
    if not isinstance(ledger, dict):
        report.findings.append(CheckFinding("CHECK_LEDGER_INVALID", "checks", "The ledger must be a JSON object."))
        return report
    if ledger.get("schema_version") != SCHEMA_VERSION:
        report.findings.append(CheckFinding("CHECK_SCHEMA_VERSION", "checks", "Unsupported or missing schema_version."))
    try:
        root = _bound_path(ledger.get("skills_root"), base)
        rules, actual_rule_files = _catalog(root, files, progress)
    except (OSError, ValueError) as error:
        report.findings.append(CheckFinding("CHECK_RULE_CATALOG_INVALID", "rules", str(error)))
        return report
    report.catalog_count = len(rules)
    selection = ledger.get("rule_selection")
    try:
        if not isinstance(selection, dict):
            raise ValueError("rule_selection must explicitly declare mode and additional_ids.")
        selected_ids = _selected_ids(rules, selection.get("mode"), selection.get("additional_ids"))
    except ValueError as error:
        report.findings.append(CheckFinding("CHECK_SELECTION_INVALID", "checks", str(error)))
        selected_ids = {identity for identity in rules if re.search(r"-A[0-9]+$", identity)}
    report.expected_count = len(selected_ids)
    catalog = ledger.get("catalog")
    if not isinstance(catalog, list):
        catalog = []
        report.findings.append(CheckFinding("CHECK_CATALOG_INVALID", "catalog", "catalog must retain all actual atomic rule declarations."))
    catalog_ids = []
    for number, item in enumerate(catalog):
        identity = item.get("id") if isinstance(item, dict) else None
        if not _nonempty(identity):
            report.findings.append(CheckFinding("CHECK_CATALOG_ID_INVALID", f"catalog:{number}", "Every catalog entry needs an atomic ID."))
            continue
        catalog_ids.append(identity)
        rule = rules.get(identity)
        if rule is None or item != rule.catalog_entry():
            report.findings.append(CheckFinding("CHECK_CATALOG_CHANGED", identity, "The catalog entry differs from the actual rule declaration."))
    for identity, count in Counter(catalog_ids).items():
        if count > 1:
            report.findings.append(CheckFinding("CHECK_CATALOG_ID_DUPLICATE", identity, "The full rule catalog contains a duplicate ID."))
    for identity in sorted(rules.keys() - set(catalog_ids)):
        report.findings.append(CheckFinding("CHECK_CATALOG_ID_MISSING", identity, "The full rule catalog lacks this current atomic declaration."))
    rule_files = ledger.get("rule_files")
    if not isinstance(rule_files, list):
        rule_files = []
        report.findings.append(CheckFinding("CHECK_RULE_FILES_INVALID", "rules", "rule_files must list actual rule file bindings."))
    seen_paths = []
    for number, binding in enumerate(rule_files):
        _validate_binding(binding, f"rules:{number}", base, files, report)
        try:
            seen_paths.append(str(_bound_path(binding.get("path"), base)))
        except (AttributeError, ValueError):
            continue
    for file_path, count in Counter(seen_paths).items():
        if count > 1:
            report.findings.append(CheckFinding("CHECK_RULE_FILE_DUPLICATE", file_path, "A rule file was bound more than once."))
    expected_paths = {item["path"] for item in actual_rule_files}
    if set(seen_paths) != expected_paths:
        report.findings.append(CheckFinding("CHECK_RULE_FILES_CHANGED", "rules", "The complete current skill/reference file set differs from the recorded set."))
    bindings = ledger.get("bindings")
    if not isinstance(bindings, dict):
        bindings = {}
        report.findings.append(CheckFinding("CHECK_BINDINGS_INVALID", "bindings", "File bindings must be an object."))
    for label in ("sources", "candidates"):
        entries = bindings.get(label)
        if not isinstance(entries, list) or not entries:
            report.findings.append(CheckFinding("CHECK_BINDING_MISSING", label, f"At least one actual {label} file binding is required."))
            continue
        identities = []
        for number, binding in enumerate(entries):
            identity = binding.get("id") if isinstance(binding, dict) else None
            if not _nonempty(identity):
                report.findings.append(CheckFinding("CHECK_BINDING_ID_MISSING", f"{label}:{number}", "File bindings need nonempty IDs."))
            else:
                identities.append(identity)
            _validate_binding(binding, f"{label}:{identity or number}", base, files, report)
        for identity, count in Counter(identities).items():
            if count > 1:
                report.findings.append(CheckFinding("CHECK_BINDING_ID_DUPLICATE", f"{label}:{identity}", "Duplicate file binding ID."))
    evidence = ledger.get("evidence")
    if not isinstance(evidence, dict):
        evidence = {}
        report.findings.append(CheckFinding("CHECK_EVIDENCE_REGISTRY_INVALID", "evidence", "evidence must be an ID-to-file-location object."))
    valid_evidence = set()
    locator_cache = {}
    for identity, item in evidence.items():
        label = f"evidence:{identity}"
        if not _nonempty(identity):
            report.findings.append(CheckFinding("CHECK_EVIDENCE_ID_INVALID", label, "Evidence IDs must be nonempty."))
        bound = _validate_binding(item, label, base, files, report)
        if bound is None:
            continue
        if not isinstance(item.get("evidence_type"), str) or item["evidence_type"] not in EVIDENCE_TYPES:
            report.findings.append(CheckFinding("CHECK_EVIDENCE_TYPE_INVALID", label, "An explicit supported evidence_type is required."))
            continue
        locator = item.get("locator")
        key = (bound, json.dumps(locator, sort_keys=True, ensure_ascii=False))
        try:
            if key not in locator_cache:
                locator_cache[key] = _locator_error(bound, locator, files)
            error = locator_cache[key]
        except (OSError, ValueError, RuntimeError) as exception:
            error = str(exception)
        if error:
            report.findings.append(CheckFinding("CHECK_EVIDENCE_LOCATION_INVALID", label, error))
            continue
        valid_evidence.add(identity)
    checks = ledger.get("checks")
    if not isinstance(checks, list):
        checks = []
        report.findings.append(CheckFinding("CHECK_RECORDS_INVALID", "checks", "checks must be one flat list of atomic records."))
    report.recorded_count = len(checks)
    seen_ids, statuses = [], Counter()
    for number, item in enumerate(checks, 1):
        if progress is not None:
            progress(number, len(checks), "checks")
        if not isinstance(item, dict) or not _nonempty(item.get("id")):
            report.findings.append(CheckFinding("CHECK_ID_INVALID", f"checks:{number}", "An atomic check ID is required."))
            continue
        identity = item["id"]
        seen_ids.append(identity)
        rule = rules.get(identity)
        if rule is None:
            report.findings.append(CheckFinding("CHECK_ID_UNKNOWN", identity, "This ID is not a current atomic rule declaration."))
        elif identity not in selected_ids:
            report.findings.append(CheckFinding("CHECK_ID_UNSELECTED", identity, "Add this constraint to additional_ids or use rule_selection=all before tracking it."))
        elif item.get("rule") != rule.location() or item.get("objective") != rule.objective or item.get("skill") != rule.skill:
            report.findings.append(CheckFinding("CHECK_RULE_CHANGED", identity, "The recorded objective, skill or source location differs from the current rule."))
        if not _scope_valid(item.get("scope")):
            report.findings.append(CheckFinding("CHECK_SCOPE_EMPTY", identity, "List actual covered object_id and locator pairs; a skill-wide status is not a scope."))
        status = item.get("status")
        if not isinstance(status, str) or status not in STATUSES:
            report.findings.append(CheckFinding("CHECK_STATUS_INVALID", identity, f"Unknown status: {status!r}"))
        else:
            statuses[status] += 1
        if "reason" not in item or not isinstance(item["reason"], str):
            report.findings.append(CheckFinding("CHECK_REASON_MISSING", identity, "Every record needs a reason string."))
        elif isinstance(status, str) and status in {"blocked", "not_verified", "not_applicable"} and not _nonempty(item["reason"]):
            report.findings.append(CheckFinding("CHECK_REASON_MISSING", identity, "Pending, blocked and inapplicable records require a specific reason."))
        if status == "not_applicable" and not _nonempty(item.get("applicability_condition")):
            report.findings.append(CheckFinding("CHECK_NA_CONDITION_MISSING", identity, "Record the actual condition that makes this check inapplicable."))
        requirement_type = item.get("requirement_type")
        if not isinstance(requirement_type, str) or requirement_type not in REQUIREMENT_TYPES:
            report.findings.append(CheckFinding("CHECK_REQUIREMENT_TYPE_INVALID", identity, "Use an explicit supported requirement_type."))
        elif requirement_type == "unclassified":
            report.unclassified_ids.append(identity)
            if isinstance(status, str) and status in {"passed", "failed", "not_applicable"}:
                report.findings.append(CheckFinding("CHECK_TYPE_UNCLASSIFIED", identity, "Classify the evidence boundary explicitly before recording a resolved result."))
        if rule is not None and rule.declared_type != "unclassified" and requirement_type != rule.declared_type:
            report.findings.append(CheckFinding("CHECK_TYPE_CONFLICT", identity, "The record's type conflicts with the source rule declaration."))
        environment_kind = item.get("environment_kind")
        if requirement_type == "live_environment":
            if not isinstance(environment_kind, str) or environment_kind not in ENVIRONMENT_KINDS:
                report.findings.append(CheckFinding("CHECK_ENVIRONMENT_KIND_INVALID", identity, "A live-environment record needs an explicit actual_ui, native_cache or actual_cli context."))
            if rule is not None and rule.declared_environment_kind is not None and environment_kind != rule.declared_environment_kind:
                report.findings.append(CheckFinding("CHECK_ENVIRONMENT_CONFLICT", identity, "The environment kind conflicts with the source rule declaration."))
        elif environment_kind is not None:
            report.findings.append(CheckFinding("CHECK_ENVIRONMENT_KIND_INVALID", identity, "Only live-environment records may declare an environment kind."))
        refs = item.get("evidence")
        if not isinstance(refs, list) or not all(_nonempty(ref) for ref in refs):
            report.findings.append(CheckFinding("CHECK_EVIDENCE_REFS_INVALID", identity, "evidence must be a list of individual evidence IDs."))
            refs = []
        for ref in refs:
            if ref not in evidence:
                report.findings.append(CheckFinding("CHECK_EVIDENCE_UNKNOWN", identity, f"Unknown evidence ID: {ref}"))
            elif ref not in valid_evidence:
                report.findings.append(CheckFinding("CHECK_EVIDENCE_UNUSABLE", identity, f"Evidence has no valid current file/location binding: {ref}"))
        if isinstance(status, str) and status in {"passed", "failed"}:
            actual_refs = [evidence[ref] for ref in refs if ref in valid_evidence]
            if not actual_refs:
                report.findings.append(CheckFinding("CHECK_RESULT_EVIDENCE_MISSING", identity, "Passed/failed results require an existing, hash-bound evidence location."))
            if isinstance(requirement_type, str) and requirement_type in {"execution_process", "live_environment"} and not any(
                    _observation_valid(entry, requirement_type, environment_kind) for entry in actual_refs):
                code = ("CHECK_PROCESS_NOT_OBSERVED" if requirement_type == "execution_process"
                        else "CHECK_UI_NOT_OBSERVED" if environment_kind == "actual_ui" else "CHECK_ENVIRONMENT_NOT_OBSERVED")
                report.findings.append(CheckFinding(code, identity, "A static result or self-report cannot replace the required direct observation; retain not_verified if it was not observed."))
        if status == "not_verified":
            report.pending_ids.append(identity)
        elif status == "blocked":
            report.blocked_ids.append(identity)
    for identity, count in Counter(seen_ids).items():
        if count > 1:
            report.findings.append(CheckFinding("CHECK_ID_DUPLICATE", identity, f"Atomic ID occurs {count} times; keep one record with multiple scope objects if necessary."))
    for identity in sorted(selected_ids - set(seen_ids)):
        report.findings.append(CheckFinding("CHECK_ID_MISSING", identity, "The full-workflow ledger lacks this atomic rule ID."))
    report.status_counts = dict(statuses)
    return report


if __name__ == "__main__":
    if CHECKS_PATH is None:
        raise SystemExit("Set CHECKS_PATH in this script, or call validate_check_records(path) from Python.")
    result = validate_check_records(CHECKS_PATH)
    if REPORT_PATH is not None:
        _write_json(Path(REPORT_PATH), result.to_dict())
    else:
        print(json.dumps(result.to_dict(), ensure_ascii=False, indent=2))
