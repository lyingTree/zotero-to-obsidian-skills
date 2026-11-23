"""Start and inspect a complete PDF-to-Markdown task without awarding fidelity.

Use these functions from a task-local Python script.  The individual validators
remain useful for partial tasks; this entry point additionally requires the
records of the complete workflow.  It never supplies semantic review results.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
from typing import Callable, Mapping, Sequence


PROFILE = "complete_pdf_candidate_v1"
PROGRESS = Callable[[int, int, str], None]


def _load(path: Path):
    key = "literature_complete_" + path.stem + "_" + hashlib.sha256(str(path).encode()).hexdigest()[:12]
    if key in sys.modules:
        return sys.modules[key]
    spec = importlib.util.spec_from_file_location(key, path)
    if spec is None or spec.loader is None:
        raise ImportError(path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[key] = module
    spec.loader.exec_module(module)
    return module


def _module(name: str):
    return _load(Path(__file__).resolve().parent.joinpath(name + ".py"))


def _skill_input_paths(skills_root: Path) -> list[Path]:
    roots = [path for path in skills_root.iterdir() if path.is_dir()
             and (path.name == "template" or path.joinpath("SKILL.md").is_file())]
    return sorted(path.resolve() for root in roots for path in root.rglob("*")
                  if path.is_file() and path.suffix in {".md", ".py", ".txt", ".json", ".yaml", ".yml"}
                  and not any(part.startswith(".") or part == "__pycache__" for part in path.relative_to(root).parts))


def _read(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"A JSON object is required: {path}")
    return value


def _write_new(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    if _read(path) != value:
        raise ValueError(f"JSON round-trip failed: {path}")


def initialize_workflow(
    *, skills_root: Path, work_root: Path, sources: Mapping[str, Path],
    source_roles: Mapping[str, str], candidate_path: Path, resource_root: Path,
    glossary_path: Path | None = None, translation_required: bool = True,
    run_id: str = "local-candidate", progress: PROGRESS | None = None,
) -> Path:
    """Prepare actual source materials and an all-pending acceptance ledger.

    This is a fresh-run operation, not a resume operation.  A named output is
    only a plan until that file exists.  No Zotero identity or metadata is guessed.
    Read the returned run.json to resume; do not initialize over an earlier run.
    """
    skills_root = Path(skills_root).resolve(strict=True)
    work_root = Path(work_root).resolve()
    candidate_path = Path(candidate_path).resolve()
    resource_root = Path(resource_root).resolve()
    run_path = work_root.joinpath("run.json")
    if run_path.exists():
        raise FileExistsError(f"Resume the existing workflow: {run_path}")
    if not sources or set(sources) != set(source_roles):
        raise ValueError("Provide one explicit role for every source PDF.")
    if any(role not in {"main", "supplement"} for role in source_roles.values()):
        raise ValueError("Source roles must be main or supplement.")
    if not isinstance(translation_required, bool):
        raise ValueError("translation_required must be an explicit boolean.")
    if not isinstance(run_id, str) or not run_id.strip():
        raise ValueError("run_id must be nonempty.")
    resolved = {identity: Path(path).resolve(strict=True) for identity, path in sources.items()}
    protected = set(resolved.values())
    if candidate_path in protected or run_path in protected:
        raise ValueError("A workflow output must not replace a source PDF.")
    inventory_path = work_root.joinpath("evidence", "source-inventory.json")
    checks_path = work_root.joinpath("checks.json")
    for path in (inventory_path, checks_path):
        if path.exists():
            raise FileExistsError(f"Keep existing evidence: {path}")
    check_tools = _module("check_records")
    input_baseline_time = datetime.now(timezone.utc).isoformat()
    input_baseline = [check_tools.bind_file(path) for path in _skill_input_paths(skills_root)]
    source_tools = _module("prepare_source_inventory")
    inventory_path = source_tools.prepare_sources(
        resolved, inventory_path.parent, source_roles=source_roles,
        glossary_path=glossary_path, progress=progress or source_tools.progress_bar,
    )
    check_tools.prepare_check_records(
        skills_root, checks_path, sources=resolved, inventory_path=inventory_path,
        progress=progress,
    )
    records = {
        "source_inventory": str(inventory_path), "checks": str(checks_path),
        "source_map": str(work_root.joinpath("source-map.json")),
        "link_map": str(work_root.joinpath("link-map.json")),
        "delivery_manifest": str(work_root.joinpath("delivery-manifest.json")),
    }
    if "supplement" in source_roles.values():
        records["si_map"] = str(work_root.joinpath("si-map.json"))
    if translation_required:
        records["translation_review"] = str(work_root.joinpath("translation-review.json"))
    if glossary_path is not None:
        records["terminology"] = str(inventory_path.parent.joinpath("terminology.json"))
        search_tools = _load(Path(__file__).resolve().parents[2].joinpath(
            "literature-translate-zh", "scripts", "glossary_candidates.py"))
        records["glossary_candidates"] = str(search_tools.prepare_glossary_candidates(
            inventory_path, Path(glossary_path), work_root.joinpath("glossary-candidates.json"), progress=progress,
        ))
    ledger = _read(checks_path)
    run = {
        "schema_version": 1, "run_id": run_id, "workflow_profile": PROFILE,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "scope": {"source_ids": list(resolved), "source_roles": dict(source_roles), "write_vault": False,
                  "translation_required": translation_required,
                  "acceptance": "markdown_and_referenced_resources"},
        "identity": {"sources": ledger["bindings"]["sources"],
                     "zotero_library": None, "zotero_item": None,
                     "library_reason": "This file-only task does not establish a live library identity."},
        "config": {"skills_root": str(skills_root), "work_root": str(work_root),
                   "resource_root": str(resource_root), "candidate_path": str(candidate_path),
                   "glossary_path": str(Path(glossary_path).resolve()) if glossary_path else None},
        "versions": {"rule_files": ledger["rule_files"], "files": input_baseline,
                     "baseline_created_at": input_baseline_time},
        "records": records,
        "naming": {"status": "pending", "original_title": None, "author_token": None,
                   "year": None, "filename": None, "transformations": [], "conflicts": []},
        "placement": {"candidate_path": str(candidate_path), "resource_root": str(resource_root),
                      "vault_path": None, "library_mapping_status": "unavailable"},
        "content_input": None, "stages": {}, "issues": [],
        "publish": {"status": "prepared", "zotero_note_status": "not_requested"},
    }
    _write_new(run_path, run)
    return run_path


@dataclass
class CompletionFinding:
    code: str
    target: str
    message: str


@dataclass
class CompletionReport:
    findings: list[CompletionFinding] = field(default_factory=list)
    checked_files: dict[str, dict] = field(default_factory=dict)
    pending_acceptance: list[dict] = field(default_factory=list)
    source_fidelity: str = "not_assessed"

    @property
    def records_ready(self) -> bool:
        return not self.findings

    def as_dict(self) -> dict:
        return {**asdict(self), "records_ready": self.records_ready,
                "boundary": "Record completeness and syntax do not establish source fidelity."}

    def summary(self, *, examples_per_code: int = 2) -> dict:
        """Bound console output while retaining every finding in as_dict()."""
        if type(examples_per_code) is not int or examples_per_code < 1:
            raise ValueError("examples_per_code must be a positive integer.")
        unique = {}
        for finding in self.findings:
            key = (finding.code, finding.target, finding.message)
            unique.setdefault(key, finding)
        counts = Counter(finding.code for finding in unique.values())
        examples = {}
        for finding in unique.values():
            bucket = examples.setdefault(finding.code, [])
            if len(bucket) < examples_per_code:
                bucket.append({"target": finding.target, "message": finding.message})
        return {
            "records_ready": self.records_ready,
            "source_fidelity": self.source_fidelity,
            "finding_count": len(self.findings), "distinct_finding_count": len(unique),
            "counts_by_code": dict(counts), "examples_by_code": examples,
            "pending_file_goal_count": len(self.pending_acceptance),
            "next_pending_file_goals": self.pending_acceptance[:examples_per_code],
            "boundary": "This is a bounded display of the full report, not a smaller acceptance scope.",
        }


class IncompleteRecordsError(RuntimeError):
    """The current local work still has unresolved file requirements."""

    def __init__(self, report: CompletionReport):
        self.report = report
        super().__init__(
            "Local records are unfinished; continue the listed work before reporting completion. "
            + json.dumps(report.summary(examples_per_code=1), ensure_ascii=False)
        )


def require_completed_records(run_path: Path, *, progress: PROGRESS | None = None) -> CompletionReport:
    """Stop a final-report script when the existing workflow gate is not ready.

    This function changes neither records nor acceptance criteria. A successful
    return still does not certify the truth of source comparisons.
    """
    report = inspect_workflow(run_path, progress=progress)
    if not report.records_ready:
        raise IncompleteRecordsError(report)
    return report


def pending_check_batch(checks_path: Path, *, limit: int = 20, prefix: str | None = None) -> list[dict]:
    """Return exact remaining file goals, never a blanket pass operation."""
    if type(limit) is not int or limit < 1:
        raise ValueError("limit must be a positive integer.")
    rows = _read(Path(checks_path)).get("checks")
    if not isinstance(rows, list):
        raise ValueError("The ledger must contain checks.")
    if any(not isinstance(row, dict) or not isinstance(row.get("id"), str) for row in rows):
        raise ValueError("Every check needs its actual string rule ID.")
    prefixes = {row["id"].split("-", 1)[0] for row in rows}
    if prefix is not None and (not isinstance(prefix, str) or prefix not in prefixes):
        raise ValueError("prefix must name an actual rule group in this ledger.")
    pending = []
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("Every check must be an object.")
        if row.get("requirement_type") != "offline_outcome":
            continue
        if prefix is not None and row.get("id", "").split("-", 1)[0] != prefix:
            continue
        if row.get("status") in {"passed", "not_applicable"}:
            continue
        pending.append({key: row.get(key) for key in ("id", "objective", "rule", "scope", "status", "reason")})
        if len(pending) == limit:
            break
    return pending


def pending_check_rows(checks_path: Path, *, limit: int = 40,
                       prefix: str | None = None) -> list[list[str]]:
    """Display one ID and one unchanged objective per row, without losing goals.

    The full rule location, current state and scope remain in checks.json and
    pending_check_batch(). This compact view is only an authoring convenience.
    """
    return [[row["id"], row["objective"]] for row in
            pending_check_batch(checks_path, limit=limit, prefix=prefix)]


def record_decision_rows(checks_path: Path, rows: Sequence[Sequence[str]], *,
                         contexts: Mapping[str, dict], evidence: Mapping[str, dict]) -> None:
    """Expand explicit short decisions into the unchanged atomic-check ledger.

    A row is (ID, status, context_name, reason). A not_applicable row additionally
    supplies its actual condition as a fifth value. A context contains only
    explicit scope objects and evidence IDs; it supplies no default conclusion.
    Every ID still has its own status and reason. No goals are selected by
    keywords, and the ordinary ledger validator remains authoritative.
    """
    if isinstance(rows, (str, bytes)) or not isinstance(rows, Sequence) or not rows:
        raise ValueError("Provide a nonempty sequence of explicit decision rows.")
    if not isinstance(contexts, Mapping) or not contexts:
        raise ValueError("Provide explicitly named review contexts.")
    check_tools = _module("check_records")
    for name, context in contexts.items():
        if not isinstance(name, str) or not name.strip() or not isinstance(context, dict):
            raise ValueError("Each context needs a nonempty name and an object.")
        if set(context) != {"scope", "evidence"}:
            raise ValueError(f"{name}: a context contains scope and evidence only, never a status.")
        if not check_tools._scope_valid(context["scope"]):
            raise ValueError(f"{name}: record the actual objects and their locations.")
        links = context["evidence"]
        if not isinstance(links, list) or not links or any(not isinstance(link, str) or not link.strip() for link in links):
            raise ValueError(f"{name}: provide actual evidence IDs.")
    results = []
    for row in rows:
        if isinstance(row, (str, bytes)) or not isinstance(row, Sequence) or len(row) not in {4, 5}:
            raise ValueError("Each decision row needs ID, status, context and reason; only NA adds a condition.")
        if any(not isinstance(value, str) or not value.strip() for value in row):
            raise ValueError("Every decision-row value must be a nonempty string.")
        identity, status, context_name, reason = row[:4]
        if context_name not in contexts:
            raise ValueError(f"{identity}: unknown context {context_name!r}.")
        if (status == "not_applicable") != (len(row) == 5):
            raise ValueError(f"{identity}: supply a condition exactly when the status is not_applicable.")
        context = contexts[context_name]
        result = {"id": identity, "status": status, "reason": reason,
                  "scope": context["scope"], "evidence": context["evidence"]}
        if status == "not_applicable":
            result["applicability_condition"] = row[4]
        results.append(result)
    record_check_batch(checks_path, results, evidence=evidence)


def record_check_batch(checks_path: Path, results: Sequence[dict], *, evidence: Mapping[str, dict]) -> None:
    """Store explicit per-ID decisions after the caller has made the comparisons.

    Shared evidence reduces copying, not the number of decisions.  There is no
    skill-wide status, default pass, keyword applicability, or type override.
    The complete ledger validator still checks scopes, locations and bindings.
    """
    if isinstance(results, (str, bytes)) or not isinstance(results, Sequence) or not results:
        raise ValueError("Provide explicit individual check results.")
    path = Path(checks_path)
    ledger = _read(path)
    rows = ledger.get("checks")
    if not isinstance(rows, list):
        raise ValueError("The ledger must contain checks.")
    by_id = {row["id"]: row for row in rows}
    if len(by_id) != len(rows):
        raise ValueError("The existing ledger contains duplicate IDs.")
    identifiers = [row.get("id") for row in results if isinstance(row, dict)]
    if any(not isinstance(identity, str) or not identity.strip() for identity in identifiers):
        raise ValueError("Every result needs a nonempty ID string.")
    if len(identifiers) != len(results) or len(set(identifiers)) != len(identifiers):
        raise ValueError("Each batch entry must identify a different check.")
    unknown = set(identifiers) - set(by_id)
    if unknown:
        raise ValueError(f"Unknown acceptance IDs: {sorted(str(value) for value in unknown)}")
    registry = ledger.get("evidence")
    if not isinstance(registry, dict) or not isinstance(evidence, Mapping):
        raise ValueError("The shared evidence registry must be an object.")
    check_tools = _module("check_records")
    for identity, item in evidence.items():
        if not isinstance(identity, str) or not identity.strip() or not isinstance(item, dict):
            raise ValueError("Evidence entries need nonempty IDs and file bindings.")
        binding = check_tools.bind_file(Path(item["path"]))
        if item.get("sha256") != binding["sha256"]:
            raise ValueError(f"Evidence bytes changed: {identity}")
        if identity in registry and registry[identity] != item:
            raise ValueError(f"Use a new evidence ID for changed evidence: {identity}")
    allowed = {"id", "status", "scope", "evidence", "reason", "applicability_condition"}
    replacements = []
    for result in results:
        identity = result["id"]
        if set(result) - allowed:
            raise ValueError(f"{identity}: do not replace a goal, declaration, or requirement type.")
        if result.get("status") not in check_tools.STATUSES:
            raise ValueError(f"{identity}: provide an actual per-goal status.")
        if not isinstance(result.get("scope"), list) or not result["scope"]:
            raise ValueError(f"{identity}: list the actual objects or the tested absence condition.")
        if not isinstance(result.get("reason"), str) or not result["reason"].strip():
            raise ValueError(f"{identity}: record the reason for this individual result.")
        links = result.get("evidence")
        if not isinstance(links, list) or any(not isinstance(link, str) for link in links):
            raise ValueError(f"{identity}: evidence must contain explicit evidence IDs.")
        if set(links) - (set(registry) | set(evidence)):
            raise ValueError(f"{identity}: an evidence ID has not been registered.")
        if result["status"] in {"passed", "failed", "not_applicable"} and not links:
            raise ValueError(f"{identity}: an evaluated result needs actual evidence.")
        if result["status"] == "not_applicable" and not str(result.get("applicability_condition", "")).strip():
            raise ValueError(f"{identity}: record the actual applicability condition.")
        replacements.append((by_id[identity], result))
    ledger.setdefault("updates", []).append({
        "at": datetime.now(timezone.utc).isoformat(),
        "previous": [dict(row) for row, _ in replacements],
    })
    registry.update(evidence)
    for row, result in replacements:
        row.update(result)
    path.write_text(json.dumps(ledger, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if _read(path) != ledger:
        raise ValueError("Check results did not round-trip.")


def bind_candidate(run_path: Path) -> dict:
    """Bind the final file before reviewing it; reject silent later rebinding."""
    run = _read(Path(run_path))
    candidate = _module("check_records").bind_file(Path(run["config"]["candidate_path"]))
    path = Path(run["records"]["checks"])
    ledger = _read(path)
    binding = {"id": "candidate", **candidate}
    existing = ledger["bindings"].get("candidates", [])
    if existing == [binding]:
        return candidate
    if existing:
        raise ValueError("The candidate changed. Preserve the earlier ledger, invalidate affected results, and review the new file before rebinding.")
    ledger["bindings"]["candidates"] = [binding]
    path.write_text(json.dumps(ledger, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if _read(path) != ledger:
        raise ValueError("Candidate binding did not round-trip.")
    return candidate


def write_manifest(run_path: Path, deliveries: list[dict]) -> Path:
    """Use the workflow's actual paths when binding the final delivery index."""
    run = _read(Path(run_path))
    bind_candidate(run_path)
    records, config = run["records"], run["config"]
    translation_path = records.get("translation_review")
    if translation_path is not None and not Path(translation_path).is_file():
        raise ValueError("Prepare and complete translation-review.json before the final delivery index.")
    return _module("validate_delivery").write_delivery_manifest(
        Path(records["source_inventory"]), Path(config["candidate_path"]),
        deliveries, Path(records["delivery_manifest"]),
        resource_root=Path(config["resource_root"]),
        translation_review_path=Path(translation_path) if translation_path else None,
        figure_edge_reviews_required=True,
    )


def write_maps(run_path: Path, deliveries: list[dict]):
    """Derive file-bound views of explicitly inventoried source and target facts.

    The returned views retain extracted/pending review states.  They do not
    replace source comparison or fill any acceptance ID. Existing files are
    protected; use explicitly versioned paths when redoing a view.
    """
    run = _read(Path(run_path))
    records, config = run["records"], run["config"]
    sources = run["identity"]["sources"]
    roles = run["scope"]["source_roles"]
    expected = {source["id"]: {"path": source["path"], "sha256": source["sha256"],
                               "role": roles[source["id"]]} for source in sources}
    paths = {name: Path(records[name]) for name in ("source_map", "si_map", "link_map") if name in records}
    return _module("handoff_maps").write_handoff_maps(
        Path(records["source_inventory"]), Path(config["candidate_path"]),
        deliveries, paths, expected_source_bindings=expected,
    )


def inspect_workflow(run_path: Path, *, progress: PROGRESS | None = None) -> CompletionReport:
    """Inspect all required records, even if an individual index has no errors.

    Only file results participate.  Missing records remain findings; merely
    choosing not to write a ledger cannot suppress the completion checks.
    This function never changes a candidate, record, or acceptance result.
    """
    report = CompletionReport()
    check_tools = _module("check_records")

    def fail(code: str, target: str, message: str) -> None:
        report.findings.append(CompletionFinding(code, target, message))

    def read_record(value, target: str):
        if not isinstance(value, str) or not value.strip():
            fail("WORKFLOW_RECORD_MISSING", target, "Record the required file path.")
            return None
        path = Path(value)
        if not path.is_absolute():
            path = Path(run_path).resolve().parent.joinpath(path)
        try:
            data = _read(path)
            if not data:
                raise ValueError("An empty object is not a completed record.")
            report.checked_files[target] = check_tools.bind_file(path)
            return data, path
        except (OSError, ValueError) as error:
            fail("WORKFLOW_RECORD_MISSING", target, str(error))
            return None

    loaded = read_record(str(run_path), "run")
    if loaded is None:
        return report
    run, _ = loaded
    if run.get("workflow_profile") != PROFILE:
        fail("WORKFLOW_PROFILE", "run", "Use initialize_workflow for a complete PDF candidate task.")
        return report
    config, scope, records = run.get("config"), run.get("scope"), run.get("records")
    if not all(isinstance(value, dict) for value in (config, scope, records)):
        fail("WORKFLOW_SCHEMA", "run", "config, scope and records must be objects.")
        return report
    versions = run.get("versions")
    baseline = versions.get("files") if isinstance(versions, dict) else None
    if not isinstance(baseline, list) or not baseline:
        fail("WORKFLOW_SKILL_BASELINE", "versions.files", "Retain the actual input skill-file baseline.")
    else:
        for binding in baseline:
            if not isinstance(binding, dict) or not isinstance(binding.get("path"), str):
                fail("WORKFLOW_SKILL_BASELINE", "versions.files", "Each input baseline needs an actual path and hash.")
                continue
            try:
                if check_tools.bind_file(Path(binding["path"])) != binding:
                    fail("WORKFLOW_SKILL_CHANGED", binding["path"], "The skill input differs from the initial file bytes.")
            except (OSError, ValueError) as error:
                fail("WORKFLOW_SKILL_CHANGED", binding["path"], str(error))
        try:
            expected_paths = {Path(item["path"]).resolve() for item in baseline
                              if isinstance(item, dict) and isinstance(item.get("path"), str)}
            actual_paths = set(_skill_input_paths(Path(config["skills_root"])))
            if expected_paths != actual_paths:
                fail("WORKFLOW_SKILL_FILE_SET", "versions.files", "The skill input file set changed after initialization.")
        except (KeyError, TypeError, OSError) as error:
            fail("WORKFLOW_SKILL_BASELINE", "config.skills_root", str(error))
    required = {"source_inventory", "source_map", "link_map", "checks", "delivery_manifest"}
    if scope.get("translation_required") is True:
        required.add("translation_review")
    elif scope.get("translation_required") is not False:
        fail("WORKFLOW_SCOPE", "translation_required", "Declare whether translation is required.")
    if config.get("glossary_path"):
        required.add("terminology")
    loaded_records = {}
    # Read the inventory first so SI requirements come from actual source roles.
    inventory = read_record(records.get("source_inventory"), "source_inventory")
    original_sources = run.get("identity", {}).get("sources") if isinstance(run.get("identity"), dict) else None
    expected_sources = {}
    if not isinstance(original_sources, list) or not original_sources:
        fail("WORKFLOW_SOURCE_BASELINE", "identity.sources", "Retain the original source-file bindings.")
    else:
        for source in original_sources:
            if not isinstance(source, dict) or not isinstance(source.get("id"), str) or source["id"] in expected_sources:
                fail("WORKFLOW_SOURCE_BASELINE", "identity.sources", "Each original source needs a unique ID and file binding.")
                continue
            expected_sources[source["id"]] = {key: source.get(key) for key in ("path", "sha256")}
    declared_ids = scope.get("source_ids")
    if (not isinstance(declared_ids, list) or any(not isinstance(identity, str) for identity in declared_ids)
            or len(declared_ids) != len(set(declared_ids)) or set(declared_ids) != set(expected_sources)):
        fail("WORKFLOW_SOURCE_SCOPE", "scope.source_ids", "The declared source set must match the initialization bindings.")
    source_roles = scope.get("source_roles")
    if (not isinstance(source_roles, dict) or set(source_roles) != set(expected_sources)
            or any(not isinstance(role, str) or role not in {"main", "supplement"} for role in source_roles.values())):
        fail("WORKFLOW_SOURCE_ROLES", "scope.source_roles", "Retain one original role for every input source.")
        source_roles = {}
    if "supplement" in source_roles.values():
        required.add("si_map")
    if inventory is not None:
        loaded_records["source_inventory"] = inventory
        sources = inventory[0].get("sources")
        if not isinstance(sources, list) or any(not isinstance(source, dict) for source in sources):
            fail("WORKFLOW_INVENTORY_SCHEMA", "source_inventory.sources", "Source records must be a list of objects.")
            sources = []
        if any(source.get("role") == "supplement" for source in sources):
            required.add("si_map")
        actual_sources = {source.get("id"): source for source in sources if isinstance(source.get("id"), str)}
        if len(actual_sources) != len(sources) or set(actual_sources) != set(expected_sources):
            fail("WORKFLOW_SOURCE_COVERAGE", "source_inventory.sources", "Do not remove or duplicate an input source in the inventory.")
        for identity, expected in expected_sources.items():
            current = actual_sources.get(identity, {})
            if {key: current.get(key) for key in ("path", "sha256")} != expected:
                fail("WORKFLOW_SOURCE_BINDING", identity, "The inventory source must retain the original input file binding.")
            if current.get("role") != source_roles.get(identity):
                fail("WORKFLOW_SOURCE_ROLE", identity, "The inventory role must match the original explicit source role.")
    for index, name in enumerate(sorted(required - {"source_inventory"}), 1):
        value = read_record(records.get(name), name)
        if value is not None:
            loaded_records[name] = value
        if progress:
            progress(index, len(required) - 1, name)
    map_fields = {
        "source_map": ("blocks", "objects", "references", "citations"),
        "si_map": ("sources", "blocks", "objects"),
        "link_map": ("anchors", "internal_links", "library_matches", "unresolved"),
    }
    for name, fields in map_fields.items():
        if name not in loaded_records:
            continue
        data, _ = loaded_records[name]
        for field_name in fields:
            if not isinstance(data.get(field_name), (list, dict)):
                fail("WORKFLOW_MAP_FIELD_MISSING", f"{name}.{field_name}", "Retain this object collection, including an empty collection when its condition is absent.")
    naming = run.get("naming")
    if not isinstance(naming, dict) or not isinstance(naming.get("filename"), str) or not naming["filename"].strip():
        fail("WORKFLOW_NAMING_MISSING", "run.naming.filename", "Record the offline naming plan even when the delivery path is fixed.")
    elif naming.get("status") == "pending":
        fail("WORKFLOW_NAMING_PENDING", "run.naming", "Complete the source-based naming decision before reporting it as resolved.")
    try:
        candidate = Path(config["candidate_path"])
        report.checked_files["candidate"] = check_tools.bind_file(candidate)
    except (KeyError, TypeError, ValueError, OSError) as error:
        fail("WORKFLOW_CANDIDATE_MISSING", "candidate", str(error))
        candidate = None
    naming_report = _module("format_helpers").validate_naming_plan(naming, delivery_path=candidate)
    for finding in naming_report["findings"]:
        fail(finding["code"], "run.naming" + finding["field"], finding["message"])
    if naming_report["plan_status"] == "blocked":
        fail("WORKFLOW_NAMING_BLOCKED", "run.naming/status", "Resolve the actual naming-plan obstruction before reporting local completion.")
    if "checks" in loaded_records:
        ledger, path = loaded_records["checks"]
        try:
            if Path(ledger["skills_root"]).resolve() != Path(config["skills_root"]).resolve():
                fail("WORKFLOW_CHECK_SKILLS_ROOT", "checks.skills_root", "Use the original input skills directory; another catalog cannot replace its goals.")
        except (KeyError, TypeError, ValueError) as error:
            fail("WORKFLOW_CHECK_SKILLS_ROOT", "checks.skills_root", str(error))
        original_rule_files = versions.get("rule_files") if isinstance(versions, dict) else None
        if not isinstance(original_rule_files, list) or ledger.get("rule_files") != original_rule_files:
            fail("WORKFLOW_CHECK_RULE_FILES", "checks.rule_files", "The acceptance catalog must retain the exact input rule-file bindings.")
        result = check_tools.validate_check_records(path, progress=progress)
        for finding in result.findings:
            if finding.level == "error":
                fail(finding.code, finding.object_id, finding.message)
        try:
            report.pending_acceptance = pending_check_batch(path, limit=max(1, result.expected_count))
        except (KeyError, TypeError, ValueError) as error:
            fail("WORKFLOW_CHECK_SCHEMA", "checks", str(error))
        for identity in result.offline_unresolved_ids:
            fail("WORKFLOW_CHECK_PENDING", identity, "Complete this file goal using its own scope and evidence.")
        for identity in result.unclassified_ids:
            fail("WORKFLOW_CHECK_UNCLASSIFIED", identity, "Retain the requirement type from the skill declaration.")
        if result.expected_count == 0:
            fail("WORKFLOW_CHECK_EMPTY", "checks", "The complete task must retain the skill's full acceptance catalog.")
        if inventory is not None:
            pointer = inventory[0].get("checks_path")
            expected = loaded_records["checks"][1].resolve()
            actual = Path(pointer) if isinstance(pointer, str) and pointer else None
            if actual is not None and not actual.is_absolute():
                actual = inventory[1].parent.joinpath(actual)
            if actual is None or actual.resolve() != expected:
                fail("WORKFLOW_CHECK_BINDING", "source_inventory.checks_path", "Bind the workflow's actual per-ID ledger in the source inventory.")
    if "delivery_manifest" in loaded_records:
        manifest, manifest_path = loaded_records["delivery_manifest"]
        if manifest.get("figure_edge_reviews_required") is not True:
            fail("WORKFLOW_FIGURE_EDGES_REQUIRED", "delivery_manifest.figure_edge_reviews_required",
                 "The complete workflow requires the final per-asset edge observations; preserve the explicit manifest requirement.")
        expected_bindings = {"inventory": "source_inventory", "checks": "checks"}
        if "translation_review" in required:
            expected_bindings["translation_review"] = "translation_review"
        for label, record_name in expected_bindings.items():
            expected = report.checked_files.get(record_name)
            binding = manifest.get(label)
            if expected is not None and binding != expected:
                fail("WORKFLOW_MANIFEST_BINDING", label, "Bind the exact file and hash inspected by this workflow.")
        if candidate is not None and manifest.get("candidate") != report.checked_files["candidate"]:
            fail("WORKFLOW_MANIFEST_BINDING", "candidate", "The delivery index must bind the workflow's current candidate.")
        result = _module("validate_delivery").validate_delivery(manifest_path, progress=progress)
        for finding in result.findings:
            if finding.level == "error":
                fail(finding.code, finding.object_id, finding.message)
    if candidate is None:
        return report
    if inventory is not None:
        map_paths = {name: loaded_records[name][1] for name in ("source_map", "si_map", "link_map")
                     if name in loaded_records}
        expected = {identity: {**binding, "role": source_roles.get(identity)}
                    for identity, binding in expected_sources.items()}
        result = _module("handoff_maps").validate_handoff_maps(
            inventory[1], candidate, map_paths,
            expected_source_bindings=expected, progress=progress,
        )
        for finding in result.findings:
            if finding.level == "error":
                fail(finding.code, finding.object_id, finding.message)
        if not result.records_ready:
            fail("WORKFLOW_MAPS_UNRESOLVED", "handoff_maps", "Complete the source and link mapping records for this exact source/candidate scope.")
    if "translation_review" in loaded_records:
        validator = _load(Path(__file__).resolve().parents[2].joinpath(
            "literature-translate-zh", "scripts", "validate_translation_review.py"))
        result = validator.validate_translation_review(
            loaded_records["translation_review"][1],
            inventory_path=inventory[1] if inventory else None,
            candidate_path=candidate,
        )
        for finding in result.findings:
            fail(finding.code, finding.object_id, finding.message)
        if not result.records_ready:
            fail("WORKFLOW_TRANSLATION_UNRESOLVED", "translation_review", "Complete the translation record's remaining goals; structural validity alone is insufficient.")
    for finding in _module("validate_note").validate_file(candidate):
        if finding.level == "error":
            fail(finding.code, f"candidate:{finding.line}", finding.message)
    prompts = _module("lint_presentation").lint_file(candidate)
    if not prompts:
        return report
    dispositions = read_record(records.get("presentation_review"), "presentation_review")
    rows = dispositions[0].get("items", []) if dispositions else []
    if dispositions is not None:
        bindings = dispositions[0].get("bindings")
        if not isinstance(bindings, dict) or bindings.get("candidate") != report.checked_files["candidate"]:
            fail("WORKFLOW_PRESENTATION_BINDING", "presentation_review.bindings.candidate", "Bind this exact candidate file before reusing a prompt disposition.")
    if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
        fail("WORKFLOW_PRESENTATION_SCHEMA", "presentation_review.items", "Each disposition must be an object.")
        rows = []
    for prompt in prompts:
        matching = [row for row in rows if row.get("code") == prompt.code and row.get("line") == prompt.line]
        if len(matching) != 1 or matching[0].get("status") != "not_an_issue" or not matching[0].get("reason"):
            fail("WORKFLOW_PRESENTATION_UNRESOLVED", f"{prompt.code}:{prompt.line}", prompt.message)
    return report
