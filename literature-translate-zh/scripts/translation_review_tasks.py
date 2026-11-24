"""Present the next bounded translation-record task without choosing answers.

Use from a task-local Python script with an actual ReviewWriter. Scope comes
first, then one complete source unit, then remaining global records. The
existing validator checks each unit. No status, term decision or applicability
condition is filled by this module, and no prepared or delivered file is edited.
"""
from __future__ import annotations

from collections import Counter
from copy import deepcopy
import hashlib
import importlib.util
from pathlib import Path
import sys
from tempfile import TemporaryDirectory


def _load_writer():
    path = Path(__file__).resolve().parent.joinpath("translation_review_writer.py")
    key = hashlib.sha256(str(path).encode("utf-8")).hexdigest()[:16]
    name = "_translation_task_writer_" + key
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(str(path))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


WRITER = _load_writer()
VALIDATE = WRITER.VALIDATE
PREPARE = WRITER.PREPARE
BOUNDARY = (
    "Record readiness only. Every existing check, category and occurrence group "
    "remains required. Source meaning, applicability and the truth of review "
    "actions are not assessed. No atomic acceptance ID is awarded a pass."
)
CHECK_GUIDANCE = {
    "alignment": "定位本来源单元的完整对应译文。",
    "ambiguity": "判定本单元的歧义是否已有明确处理。",
    "negation": "核对每处否定的适用范围。",
    "strength": "核对每条断言的强度。",
    "number_changes": "核对每次数值表达的变化。",
    "terminology": "核对每个术语的语境含义。",
    "units": "核对每个量的单位，包括用文字表达的时间尺度。",
    "symbols": "核对每个符号的含义。",
    "formulas": "核对每条公式的结构。",
    "citation_labels": "核对每次引文调用的标号。",
    "completeness": "核对每条实质命题，包括具体动作本身，是否完整保留。",
    "directions": "核对每处方位关系的方向。",
    "citation_claims": "核对每次引文实际支持的论断。",
    "table_text": "核对每个表格单元格的文字含义。",
    "conditions": "核对每处条件的作用范围。",
    "antecedents": "核对每处指代的先行对象。",
    "modifier_scope": "核对每个修饰语的修饰对象。",
}


class IncompleteUnitRecordsError(RuntimeError):
    """One unit still has actual pending, failed, unresolved or invalid records."""

    def __init__(self, result):
        self.result = result
        first = result["findings"][:4]
        details = "; ".join(row["object_id"] + ": " + row["message"] for row in first)
        super().__init__(f"Finish records for {result['unit_id']}: {details}")


def _mapping(value):
    """Display malformed input without changing what the validator receives."""
    return value if isinstance(value, dict) else {}


def _rows(value):
    return value if isinstance(value, list) else []


class _Context:
    def __init__(self, writer):
        writer._verify_bindings(writer.data)
        for source_id, binding in writer.source_bindings.items():
            writer._verify_binding(binding, "inventory/sources/" + source_id)
        try:
            writer._check_hits()
        except (AttributeError, IndexError, KeyError, TypeError):
            # Incomplete shapes belong to the existing validator below. Actual
            # stale bindings and fabricated query hits still raise unchanged.
            pass
        self.writer = writer
        self.payload = deepcopy(writer.data)
        self.terminology = self._terminology_preview()
        inventory = WRITER._read(writer.inventory_path)
        self.units = PREPARE._index(inventory.get("units"), "inventory units")
        self.sources = PREPARE._index(inventory.get("sources"), "inventory sources")
        # One file/PDF cache serves all units in this call. No state survives a call.
        self.files = VALIDATE._Files()

    def _terminology_preview(self):
        try:
            return self.writer._terminology(self.payload)
        except (AttributeError, IndexError, KeyError, TypeError):
            # A missing original or adopted match must become a term diagnostic,
            # not an exception from preview assembly. Preserve all original rows
            # for validation; compile only independently well-formed rows to keep
            # unrelated units' existing terminology pointers resolvable.
            self.payload = deepcopy(self.writer.data)
        empty = {"bindings": deepcopy(self.payload.get("bindings")), "units": {}}
        terminology = self.writer._terminology(empty)
        for unit_id, unit in _mapping(self.payload.get("units")).items():
            for term in _rows(_mapping(unit).get("term_candidates")):
                if not isinstance(term, dict):
                    continue
                selected = deepcopy(term)
                partial = {"bindings": empty["bindings"], "units": {unit_id: {"term_candidates": [selected]}}}
                try:
                    compiled = self.writer._terminology(partial)
                except (AttributeError, IndexError, KeyError, TypeError):
                    continue
                for section in ("adopted", "unlisted"):
                    if not compiled[section]:
                        continue
                    term["terminology_pointer"] = f"/{section}/{len(terminology[section])}"
                    terminology[section].extend(compiled[section])
        return terminology

    def validator(self):
        report = VALIDATE.TranslationReviewReport()
        validator = VALIDATE._Validator(self.writer.path, report)
        validator.files = self.files
        validator.units = self.units
        validator.sources = self.sources
        validator.source_paths = {
            identity: Path(binding["path"])
            for identity, binding in self.writer.source_bindings.items()
        }
        validator.candidate = self.writer.candidate_path
        validator.glossary = self.writer.glossary_path
        validator.terminology = self.terminology
        return validator, report

    def scope_report(self):
        validator, report = self.validator()
        validator.check_scope(self.payload.get("scope"), _mapping(self.payload.get("units")))
        report.status_counts = dict(validator.statuses)
        return report

    def unit_report(self, unit_id):
        if unit_id not in self.writer.selected:
            raise ValueError("Choose an actual selected unit: " + str(unit_id))
        validator, report = self.validator()
        validator.unit(unit_id, _mapping(self.payload.get("units")).get(unit_id))
        report.status_counts = dict(validator.statuses)
        return report


def _record_ready(report):
    return (not any(row.level == "error" for row in report.findings)
            and not report.pending_ids and not report.failed_ids and not report.unresolved_ids)


def _report_fields(report, *, limit=None):
    value = {
        "findings": [row.to_dict() for row in report.findings[:limit]],
        "pending_ids": list(report.pending_ids[:limit]),
        "failed_ids": list(report.failed_ids[:limit]),
        "unresolved_ids": list(report.unresolved_ids[:limit]),
        "status_counts": dict(report.status_counts),
    }
    if limit is None:
        return value
    value["diagnostic_totals"] = {
        "findings": len(report.findings), "pending_ids": len(report.pending_ids),
        "failed_ids": len(report.failed_ids), "unresolved_ids": len(report.unresolved_ids),
        "codes": dict(Counter(row.code for row in report.findings)),
    }
    value["display_limit"] = limit
    return value


def unit_record_report(writer, unit_id):
    """Inspect all required fields of one unit using the unchanged validator.

    A ready unit does not establish overall scope or global-ledger readiness.
    No file is written, and the writer's decisions remain byte-for-byte equal.
    """
    report = _Context(writer).unit_report(unit_id)
    return {
        "unit_id": unit_id, "unit_records_ready": _record_ready(report),
        **_report_fields(report), "source_fidelity": "not_assessed",
        "execution_truth": "not_assessed", "boundary": BOUNDARY,
    }


def require_unit_records(writer, unit_id):
    """Check the current unit before moving to the next one; never fill a result."""
    result = unit_record_report(writer, unit_id)
    if not result["unit_records_ready"]:
        raise IncompleteUnitRecordsError(result)
    return result


def _source_context(context, unit_id):
    unit = context.units[unit_id]
    source_id = unit["source"]
    locations = PREPARE.expand_source_locations(unit, context.sources[source_id])
    pages = {location.get("page") for location in locations}
    page_records = context.sources[source_id].get("pages", [])
    return {
        "unit_id": unit_id, "kind": unit.get("kind"), "source_id": source_id,
        "source_file": deepcopy(context.writer.source_bindings[source_id]),
        "source_locations": locations,
        "source_page_records": [
            {key: deepcopy(page[key]) for key in ("page", "render", "text") if key in page}
            for page in page_records if isinstance(page, dict) and page.get("page") in pages
        ],
    }


def _candidate_context(writer, unit_id):
    row = _mapping(_mapping(writer.data.get("units")).get(unit_id))
    result = []
    for index, locator in enumerate(_rows(row.get("candidate_locations"))):
        try:
            begin, end = WRITER._text_range(writer.candidate_text, locator, unit_id)
        except WRITER.ReviewWriterError as error:
            result.append({"locator": deepcopy(locator), "location_error": str(error)})
            continue
        result.append({"locator": deepcopy(locator), "char_range": [begin, end],
                       "text": writer.candidate_text[begin:end], "location_index": index})
    return result


def _unit_task(context, unit_id, report, completed):
    writer = context.writer
    row = _mapping(_mapping(writer.data.get("units")).get(unit_id))
    checks = _mapping(row.get("checks"))
    scans = _mapping(row.get("term_scan"))
    occurrences = _mapping(row.get("occurrences"))
    return {
        "stage": "unit", "unit_id": unit_id,
        "completed_preceding_units": list(completed),
        "selected_unit_count": len(writer.selected),
        **_source_context(context, unit_id),
        "candidate_file": deepcopy(writer.data["bindings"]["candidate"]),
        "candidate_locations": _candidate_context(writer, unit_id),
        "checks": [{"check": name, "question": CHECK_GUIDANCE[name],
                    "status": _mapping(checks.get(name)).get("status"),
                    "reason": _mapping(checks.get(name)).get("reason")}
                   for name in PREPARE.CHECKS],
        "term_scan": [{"category": name, "status": _mapping(scans.get(name)).get("status"),
                       "candidate_ids": deepcopy(_mapping(scans.get(name)).get("candidate_ids")),
                       "reason": _mapping(scans.get(name)).get("reason")}
                      for name in PREPARE.TERM_CATEGORIES],
        "glossary_file": deepcopy(writer.data["bindings"].get("glossary")),
        "term_candidates": [_term_summary(term) for term in _rows(row.get("term_candidates"))],
        "occurrence_groups": [
            {"group": name, "recorded_count": (len(occurrences[name])
                                                if isinstance(occurrences.get(name), list) else None),
             "comparison_fields": list(PREPARE.OCCURRENCE_FIELDS[name])}
            for name in PREPARE.OCCURRENCE_KINDS
        ],
        **_report_fields(report, limit=12),
        "next_action": (
            "Compare this complete source/candidate pair. Record each required "
            "check and category separately; register every applicable occurrence "
            "and actual term query. Explain absence separately for each absent "
            "category. Call require_unit_records(writer, unit_id) before moving "
            "to another unit; save_draft() can preserve unfinished work."
        ),
    }


def _term_summary(term):
    term = _mapping(term)
    return {
        **{key: deepcopy(term.get(key)) for key in
           ("id", "original", "category", "decision", "translation", "decision_reason")},
        "searches": {
            mode: {"status": row.get("status"), "reason": row.get("reason"),
                   "queries": [{"query": query.get("query"),
                                "entry_ids": [_mapping(hit).get("entry_id") for hit in _rows(query.get("hits"))],
                                "interpretation": query.get("interpretation")}
                               for query in map(_mapping, _rows(row.get("queries")))]}
            for mode, raw in _mapping(term.get("searches")).items()
            for row in [_mapping(raw)]
        },
    }


def next_review_task(writer, *, scope_limit=8):
    """Return the first actual unfinished stage in a fixed dependency order.

    Scope batches expose each unselected unit without deciding its applicability.
    Unit text is never silently truncated. Global checking writes only temporary
    preview files and invokes the real, unchanged full validator. Even a ready
    preview must still be saved and bound as a real delivery by the caller.
    """
    if type(scope_limit) is not int or scope_limit < 1:
        raise ValueError("scope_limit must be a positive integer")
    context = _Context(writer)
    scope = context.scope_report()
    common = {"source_fidelity": "not_assessed", "execution_truth": "not_assessed",
              "boundary": BOUNDARY}
    if not _record_ready(scope):
        missing = {row.object_id for row in scope.findings if row.code == "TRN_SCOPE_UNACCOUNTED"}
        unaccounted = [identity for identity in context.units if identity in missing]
        return {
            **common, "stage": "scope", **_report_fields(scope, limit=12),
            "selected_units": list(writer.selected),
            "unaccounted_count": len(unaccounted),
            "unaccounted_units": [_source_context(context, identity)
                                  for identity in unaccounted[:scope_limit]],
            "next_action": (
                "Read each unit's source and task role. If it needs translation, "
                "prepare the correct selected-unit scope before reviewing units. "
                "Otherwise use writer.exclude(id, actual_condition, reason) for "
                "that unit. Do not infer exclusion from kind or keywords. Finish "
                "the explicit writer.scope(status, reason) decision as well."
            ),
        }
    completed = []
    for unit_id in writer.selected:
        report = context.unit_report(unit_id)
        if not _record_ready(report):
            return {**common, **_unit_task(context, unit_id, report, completed)}
        completed.append(unit_id)
    with TemporaryDirectory(prefix="translation-record-preview-") as temporary:
        directory = Path(temporary)
        report = writer.save(directory.joinpath("review.json"),
                             terminology_path=directory.joinpath("terminology.json"))
    return {
        **common, "stage": "save_final_records" if report.records_ready else "global_records",
        "completed_units": completed, "preview_records_ready": report.records_ready,
        **_report_fields(report, limit=12),
        "next_action": (
            "Save new final review and terminology files with writer.save(), "
            "check its real report, then bind those exact files in the workflow. "
            "Temporary preview files are not a delivery."
            if report.records_ready else
            "Finish each reported global scope, source-issue or terminology "
            "record. Preserve each explicit decision and actual evidence. "
            "Then call next_review_task() again; do not end at a pending preview."
        ),
    }
