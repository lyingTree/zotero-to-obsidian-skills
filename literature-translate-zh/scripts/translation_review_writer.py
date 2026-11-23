"""Record explicit translation decisions without repeating file bindings.

Use ReviewWriter from a task-local Python script after the real preparer. The
writer does not choose a status, discover all occurrences, certify a visible
transcription, or carry conclusions to changed inputs. The original preparer
and validator remain the schema and final record-completeness authority.
"""
from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass, field
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys


def _load(filename):
    path = Path(__file__).resolve().parent.joinpath(filename)
    path_key = hashlib.sha256(str(path).encode("utf-8")).hexdigest()[:16]
    name = "_compact_translation_" + path.stem + "_" + path_key
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


PREPARE = _load("prepare_translation_review.py")
VALIDATE = _load("validate_translation_review.py")
GlossaryIndex = _load("glossary_candidates.py").GlossaryIndex
CHECKS, TERM_CATEGORIES = PREPARE.CHECKS, PREPARE.TERM_CATEGORIES
OCCURRENCE_KINDS = PREPARE.OCCURRENCE_KINDS
_TEXT_PAIRS = {
    "claims": ("source_claim", "candidate_claim"),
    "citation_claims": ("source_claim", "candidate_claim"),
    "numbers": ("source_expression", "candidate_expression"),
    "units": ("source_expression", "candidate_expression"),
    "symbols": ("source_expression", "candidate_expression"),
    "formulas": ("source_expression", "candidate_expression"),
    "citation_labels": ("source_label", "candidate_label"),
}


class ReviewWriterError(ValueError):
    """One concrete recording target failed; no semantic verdict is implied."""

    def __init__(self, code, object_id, message):
        self.code, self.object_id = code, object_id
        super().__init__(f"{code} at {object_id}: {message}")


def _require(condition, code, identity, message):
    if not condition:
        raise ReviewWriterError(code, identity, message)


def _text(value):
    return isinstance(value, str) and bool(value.strip())


def _pointer(part):
    return str(part).replace("~", "~0").replace("/", "~1")


def _read(path):
    return json.loads(Path(path).read_text(encoding="utf-8"),
                      object_pairs_hook=VALIDATE._unique_pairs,
                      parse_constant=VALIDATE._reject_constant)


def _write_new(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
        stream.write("\n")


def _ids(values, identity):
    _require(isinstance(values, (list, tuple)) and all(_text(item) for item in values),
             "TRW_IDS", identity, "Supply an explicit list of nonempty IDs.")
    _require(len(set(values)) == len(values), "TRW_IDS", identity, "IDs must be unique.")
    return list(values)


def _result(status, reason, evidence, identity, allowed=VALIDATE.SEMANTIC_STATUSES):
    _require(isinstance(status, str) and status in allowed,
             "TRW_STATUS", identity, "Unknown explicit status: " + str(status))
    _require(_text(reason), "TRW_REASON", identity, "Explain this individual decision, including any actual absence condition.")
    return {"status": status, "reason": reason, "evidence": deepcopy(evidence)}


def _matches(text, quote, start=0, end=None):
    """Find overlapping exact occurrences in one selected range."""
    end = len(text) if end is None else end
    return [(start + match.start(), start + match.start() + len(quote))
            for match in re.finditer("(?=" + re.escape(quote) + ")", text[start:end])]


def _choose(rows, occurrence, identity):
    _require(bool(rows), "TRW_LOCATION", identity, "The exact selection is absent from this unit's recorded location.")
    if occurrence is None:
        _require(len(rows) == 1, "TRW_LOCATION_AMBIGUOUS", identity,
                 "Select explicit lines, page/block/bbox, or a 1-based occurrence for repeated text.")
        return rows[0]
    _require(type(occurrence) is int and 1 <= occurrence <= len(rows),
             "TRW_LOCATION", identity, "The selected 1-based occurrence does not exist.")
    return rows[occurrence - 1]


def _text_range(text, locator, identity):
    _require(isinstance(locator, dict) and bool(locator), "TRW_LOCATION", identity, "Use an actual text locator.")
    _require(locator.get("kind") in (None, "lines", "quote"), "TRW_LOCATION", identity, "Candidate locations must identify actual UTF-8 source text.")
    begin, end = 0, len(text)
    if "lines" in locator or locator.get("kind") == "lines":
        lines = locator.get("lines", [locator.get("start"), locator.get("end")])
        parts = text.splitlines(keepends=True)
        _require(isinstance(lines, list) and len(lines) == 2 and all(type(n) is int for n in lines)
                 and 1 <= lines[0] <= lines[1] <= len(parts), "TRW_LOCATION", identity, "Line range is outside the actual file.")
        begin = sum(map(len, parts[:lines[0] - 1]))
        end = begin + len("".join(parts[lines[0] - 1:lines[1]]).rstrip("\r\n"))
    quote = locator.get("quote")
    if quote is not None:
        _require(_text(quote), "TRW_LOCATION", identity, "An exact quote must be nonempty.")
        return _choose(_matches(text, quote, begin, end), locator.get("occurrence"), identity)
    _require("lines" in locator or locator.get("kind") == "lines", "TRW_LOCATION", identity, "Supply lines or an exact unique quote.")
    return begin, end


@dataclass(frozen=True)
class Selection:
    """Actual location and separately identified comparison text."""

    unit_id: str
    side: str
    locator: dict
    text: str
    raw_text: str | None
    basis: str | None
    selector: dict = field(repr=False)
    _owner: object = field(repr=False, compare=False)


class ReviewWriter:
    """Load a prepared or previously saved ledger, preserving every target.

    save_draft()/from_draft() checkpoint short calls only while all original
    bindings remain current. A changed source, candidate, inventory or glossary
    requires a new preparation and explicit decisions; there is no rebase API.
    """

    def __init__(self, prepared_path, *, progress=PREPARE.progress_bar):
        self.path = Path(prepared_path).resolve(strict=True)
        self.prepared_binding = PREPARE.bind_review_file(self.path)
        self.data = _read(self.path)
        _require(isinstance(self.data, dict) and self.data.get("schema_version") == 1,
                 "TRW_SCHEMA", "review", "Load the actual schema-version-1 prepared ledger.")
        self.progress, self._commands, self._handles, self._replaying = progress, [], {}, False
        self._files = VALIDATE._Files()
        self._verify_bindings(self.data)
        bindings = self.data.get("bindings", {})
        self.inventory_path = self._bound_path(bindings.get("source_inventory"), "bindings/source_inventory")
        self.candidate_path = self._bound_path(bindings.get("candidate"), "bindings/candidate")
        self.glossary_path = (self._bound_path(bindings["glossary"], "bindings/glossary")
                              if bindings.get("glossary") is not None else None)
        self.inventory = _read(self.inventory_path)
        self.inventory_units = PREPARE._index(self.inventory.get("units"), "inventory units")
        self.sources = PREPARE._index(self.inventory.get("sources"), "inventory sources")
        self.selected = _ids(self.data.get("scope", {}).get("selected_units"), "scope/selected_units")
        units = self.data.get("units")
        _require(bool(self.selected) and isinstance(units, dict) and set(units) == set(self.selected),
                 "TRW_TARGETS", "units", "Keep exactly the prepared selected-unit records.")
        _require(set(self.data["scope"].get("unselected_inventory_units", [])) == set(self.inventory_units) - set(self.selected),
                 "TRW_TARGETS", "scope/unselected_inventory_units", "Do not drop unselected inventory units.")
        self.source_bindings = {}
        for source_id, source in self.sources.items():
            self.source_bindings[source_id] = self._verify_binding(source, "inventory/sources/" + source_id, self.inventory_path.parent)
        for number, unit_id in enumerate(self.selected, 1):
            unit, original = units[unit_id], self.inventory_units.get(unit_id)
            _require(isinstance(original, dict) and isinstance(unit, dict), "TRW_UNIT", unit_id, "Unknown source unit.")
            source_id = original.get("source")
            _require(source_id in self.sources and unit.get("source_id") == source_id and unit.get("kind") == original.get("kind"),
                     "TRW_UNIT", unit_id, "Keep this inventory unit's source identity and kind.")
            _require(unit.get("source_locations") == PREPARE.expand_source_locations(original, self.sources[source_id]),
                     "TRW_SOURCE_LOCATION", unit_id, "Keep every source location from the bound inventory.")
            _require(bindings.get("sources", {}).get(source_id) is not None,
                     "TRW_BINDING", unit_id, "Missing selected source binding.")
            actual = self._verify_binding(bindings["sources"][source_id], "bindings/sources/" + source_id)
            _require(actual == self.source_bindings[source_id], "TRW_BINDING", unit_id, "Source binding differs from its inventory identity.")
            for name, expected in (("checks", CHECKS), ("term_scan", TERM_CATEGORIES), ("occurrences", OCCURRENCE_KINDS)):
                _require(isinstance(unit.get(name), dict) and set(unit[name]) == set(expected),
                         "TRW_TARGETS", f"units/{unit_id}/{name}", "Restore every original target; unknown or removed targets are rejected.")
            if self.progress:
                self.progress(number, len(self.selected), "translation writer bindings")
        self.candidate_text = self.candidate_path.read_bytes().decode("utf-8")
        self._glossary = None
        self._terminology_summary = deepcopy(self.data.get("writer_metadata", {}).get("terminology_summary", {
            "review_status": "pending", "covered_units": [], "no_unlisted_reason": None}))

    def _bound_path(self, binding, identity):
        _require(isinstance(binding, dict) and _text(binding.get("path")), "TRW_BINDING", identity, "Missing real file binding.")
        path = Path(binding["path"])
        return (path if path.is_absolute() else self.path.parent.joinpath(path)).resolve(strict=True)

    def _verify_binding(self, binding, identity, base=None, cache=None):
        _require(isinstance(binding, dict) and _text(binding.get("path")), "TRW_BINDING", identity, "Missing real file binding.")
        path = Path(binding["path"])
        path = (path if path.is_absolute() else (base or self.path.parent).joinpath(path)).resolve()
        try:
            if cache is not None and path in cache:
                actual = cache[path]
            else:
                actual = PREPARE.bind_review_file(path)
                if cache is not None:
                    cache[path] = actual
        except (OSError, ValueError) as error:
            raise ReviewWriterError("TRW_BINDING", identity, str(error)) from error
        _require(binding.get("sha256") == actual["sha256"], "TRW_STALE_BINDING", identity, "File bytes changed; prepare a new record and explicitly recheck affected decisions: " + str(path))
        return actual

    def _verify_bindings(self, value):
        cache = {}
        stack = [("", value)]
        while stack:
            identity, row = stack.pop()
            if isinstance(row, dict):
                if "path" in row and "sha256" in row:
                    self._verify_binding(row, identity, cache=cache)
                stack.extend((identity + "/" + _pointer(key), child) for key, child in row.items())
            elif isinstance(row, list):
                stack.extend((identity + "/" + str(index), child) for index, child in enumerate(row))
        self._verify_binding(self.prepared_binding, "prepared", cache=cache)

    def _record(self, op, **arguments):
        if not self._replaying:
            self._commands.append(deepcopy({"op": op, **arguments}))

    def _inventory_evidence(self, unit_id=None):
        pointer = "/units"
        if unit_id is not None:
            index = next(index for index, row in enumerate(self.inventory["units"]) if row["id"] == unit_id)
            pointer += "/" + str(index)
        return [{**PREPARE.bind_review_file(self.inventory_path), "locator": {"kind": "json_pointer", "pointer": pointer}}]

    def unit(self, unit_id, *, candidate_locations=None):
        _require(unit_id in self.selected, "TRW_UNIT", str(unit_id), "Use an explicitly selected prepared unit.")
        if unit_id in self._handles:
            handle = self._handles[unit_id]
            _require(candidate_locations is None or candidate_locations == handle.row["candidate_locations"],
                     "TRW_LOCATION_REBOUND", unit_id, "This unit is already bound; do not silently move its decisions.")
            return handle
        row = self.data["units"][unit_id]
        locations = row.get("candidate_locations") if candidate_locations is None else candidate_locations
        if row.get("candidate_locations") and candidate_locations is not None:
            _require(row["candidate_locations"] == candidate_locations, "TRW_LOCATION_REBOUND", unit_id, "Keep the prepared candidate mapping or prepare an explicit revision.")
        _require(isinstance(locations, list) and bool(locations), "TRW_LOCATION", unit_id, "Bind this complete unit to its actual candidate locations once.")
        handle = UnitReview(self, unit_id, locations)
        row["candidate_locations"] = deepcopy(locations)
        self._handles[unit_id] = handle
        self._record("unit", unit_id=unit_id, candidate_locations=locations)
        return handle

    def scope(self, status, reason):
        self.data["scope"]["selection_review"] = _result(status, reason, self._inventory_evidence(), "scope/selection_review")
        self._record("scope", status=status, reason=reason)

    def exclude(self, unit_id, condition, reason):
        _require(unit_id in self.data["scope"]["unselected_inventory_units"], "TRW_UNIT", unit_id, "Only an existing unselected inventory unit can be excluded.")
        _require(_text(condition) and _text(reason), "TRW_REASON", unit_id, "State this unit's actual applicability condition and reason.")
        rows = self.data["scope"]["excluded_units"]
        _require(not any(row["id"] == unit_id for row in rows), "TRW_DUPLICATE", unit_id, "Exclusion is already recorded.")
        rows.append({"id": unit_id, "applicability_condition": condition, "reason": reason,
                     "evidence": self._inventory_evidence(unit_id)})
        self._record("exclude", unit_id=unit_id, condition=condition, reason=reason)

    def source_issues_review(self, status, reason):
        evidence = [item for unit_id in self.selected for item in self._source_evidence(unit_id)]
        self.data["source_issues_review"] = _result(status, reason, evidence, "source_issues_review")
        self._record("source_issues_review", status=status, reason=reason)

    def _source_evidence(self, unit_id):
        row = self.data["units"][unit_id]
        binding = self.source_bindings[row["source_id"]]
        return [{**binding, "locator": deepcopy(location)} for location in row["source_locations"]]

    def terminology_review(self, status, *, covered_units, no_unlisted_reason=None):
        _require(status in {"pending", "reviewed"}, "TRW_STATUS", "terminology/review_status", "Use pending or explicitly reviewed.")
        covered = _ids(covered_units, "terminology/covered_units")
        _require(set(covered) <= set(self.selected), "TRW_UNIT", "terminology/covered_units", "Name only this ledger's selected units.")
        _require(no_unlisted_reason is None or _text(no_unlisted_reason), "TRW_REASON", "terminology/no_unlisted_reason", "Explain the actual empty unlisted set when applicable.")
        self._terminology_summary = {"review_status": status, "covered_units": covered, "no_unlisted_reason": no_unlisted_reason}
        self._record("terminology_review", status=status, covered_units=covered, no_unlisted_reason=no_unlisted_reason)

    def review(self, status):
        _require(status in {"pending", "reviewed"}, "TRW_STATUS", "review_status", "Use pending or explicitly reviewed; this is not a semantic verdict.")
        self.data["review_status"] = status
        self._record("review", status=status)

    def _index(self):
        _require(self.glossary_path is not None, "TRW_GLOSSARY", "bindings/glossary", "Bind the actual glossary before recording a search.")
        self._verify_binding(self.data["bindings"]["glossary"], "bindings/glossary")
        if self._glossary is None:
            self._glossary = GlossaryIndex(self.glossary_path)
        _require(self._glossary.binding == PREPARE.bind_review_file(self.glossary_path),
                 "TRW_STALE_BINDING", "bindings/glossary", "The indexed glossary changed.")
        return self._glossary

    def _check_hits(self):
        """Recheck persisted queries too; callers cannot inject fake hit rows."""
        index = None
        for unit_id, unit in self.data["units"].items():
            for term in unit["term_candidates"]:
                actual_hits = {}
                for mode, search in term["searches"].items():
                    for number, query in enumerate(search.get("queries", [])):
                        if index is None:
                            index = self._index()
                        expected = [_hit(row) for row in index.search(query["query"])]
                        identity = f"units/{unit_id}/term_candidates/{term['id']}/searches/{mode}/{number}"
                        _require(query.get("hits") == expected, "TRW_GLOSSARY_HIT", identity, "Stored hits differ from the actual glossary query.")
                        actual_hits.update((hit["entry_id"], hit) for hit in expected)
                for hit in term.get("glossary_matches", []):
                    _require(hit == actual_hits.get(hit.get("entry_id")), "TRW_GLOSSARY_HIT", f"units/{unit_id}/term_candidates/{term['id']}", "A chosen match must be one of this term's actual query results.")

    def _terminology(self, payload):
        result = {"schema_version": 1, "glossary": deepcopy(payload["bindings"]["glossary"]),
                  **deepcopy(self._terminology_summary), "adopted": [], "unlisted": []}
        unresolved = []
        for unit_id, unit in payload["units"].items():
            for term in unit["term_candidates"]:
                decision = term["decision"]
                if decision in {"pending", "not_a_term"}:
                    continue
                section = "adopted" if decision == "adopted" else "unlisted"
                term["terminology_pointer"] = f"/{section}/{len(result[section])}"
                entry = {"original": term["original"], "translation": term["translation"],
                         "context": term["source_excerpt"], "source_units": [unit_id]}
                if decision == "adopted":
                    line = term["glossary_matches"][0]["glossary_locator"]["lines"][0]
                    entry["glossary_locator"] = f"{self.glossary_path}#L{line}"
                else:
                    entry.update(status="unresolved" if decision == "unresolved" else "resolved",
                                 reason=term["decision_reason"], resolution_basis=term.get("resolution_basis"))
                if decision == "unresolved":
                    unresolved.append({"unit_id": unit_id, "candidate_id": term["id"], "reason": term["decision_reason"]})
                result[section].append(entry)
        payload["unresolved_terms"] = unresolved
        return result

    def save(self, output_path, *, terminology_path=None):
        """Write fresh files and return the unchanged validator's actual report.

        Partial records are intentionally saveable. Inspect pending_ids and
        findings; this function never fills missing judgments to make them ready.
        """
        output_path = Path(output_path).resolve()
        terminology_path = Path(terminology_path).resolve() if terminology_path is not None else None
        outputs = [output_path] + ([terminology_path] if terminology_path is not None else [])
        _require(len(set(outputs)) == len(outputs), "TRW_OUTPUT", "output", "Review and terminology need separate new files.")
        for path in outputs:
            if path.exists():
                raise FileExistsError("Keep the existing record: " + str(path))
        self._verify_bindings(self.data)
        for source_id, binding in self.source_bindings.items():
            self._verify_binding(binding, "inventory/sources/" + source_id)
        self._check_hits()
        payload = deepcopy(self.data)
        payload["writer_metadata"] = {"format": "explicit-translation-decisions-v1", "terminology_summary": deepcopy(self._terminology_summary),
                                      "source_fidelity": "not_assessed", "execution_truth": "not_assessed"}
        if terminology_path is not None:
            terminology = self._terminology(payload)
            _write_new(terminology_path, terminology)
            payload["terminology_record"] = PREPARE.bind_review_file(terminology_path)
        # A fresh check also covers source/candidate changes during compilation.
        self._verify_bindings(payload)
        _write_new(output_path, payload)
        return VALIDATE.validate_translation_review(output_path, self.inventory_path, self.candidate_path)

    def save_draft(self, output_path):
        """Checkpoint short decisions; do not pretend this is a finished ledger."""
        self._verify_bindings(self.data)
        self._check_hits()
        path = Path(output_path).resolve()
        _write_new(path, {"schema_version": 1, "format": "translation-review-decisions-v1",
                          "prepared": deepcopy(self.prepared_binding), "decisions": deepcopy(self._commands),
                          "boundary": "Explicit decisions only. Resume requires the original unchanged prepared inputs; no cross-version inheritance."})
        return path

    @classmethod
    def from_draft(cls, draft_path, *, progress=PREPARE.progress_bar):
        draft_path = Path(draft_path).resolve(strict=True)
        draft = _read(draft_path)
        _require(isinstance(draft, dict) and draft.get("format") == "translation-review-decisions-v1" and draft.get("schema_version") == 1,
                 "TRW_DRAFT", "draft", "Use a writer decision draft.")
        binding = draft.get("prepared")
        _require(isinstance(binding, dict) and _text(binding.get("path")), "TRW_DRAFT", "prepared", "Missing prepared-file binding.")
        _require(PREPARE.bind_review_file(binding["path"]) == binding, "TRW_STALE_BINDING", "prepared", "The prepared record changed; no automatic migration is allowed.")
        writer = cls(binding["path"], progress=progress)
        commands = draft.get("decisions")
        _require(isinstance(commands, list), "TRW_DRAFT", "decisions", "A draft contains explicit short calls in order.")
        writer._replaying = True
        for number, command in enumerate(commands, 1):
            writer._replay(command)
            if progress:
                progress(number, len(commands), "translation decisions restored")
        writer._replaying = False
        writer._commands = deepcopy(commands)
        return writer

    def _replay(self, command):
        _require(isinstance(command, dict), "TRW_DRAFT", "decisions", "Invalid short call.")
        row = deepcopy(command)
        op = row.pop("op", None)
        top = {"unit", "scope", "exclude", "source_issues_review", "terminology_review", "review"}
        if op in top:
            getattr(self, op)(**row)
            return
        _require(op in {"check", "scan", "occurrence", "term", "issue", "search", "search_status", "decide"}, "TRW_DRAFT", "decisions/op", "Unknown operation.")
        unit = self.unit(row.pop("unit_id"))
        if "source" in row:
            row["source"] = unit.source(**row["source"])
        if "candidate" in row:
            row["candidate"] = unit.candidate(**row["candidate"])
        if op in {"search", "search_status", "decide"}:
            term = unit.terms[row.pop("term_id")]
            getattr(term, op)(**row)
            return
        if op == "occurrence":
            row.update(row.pop("comparison"))
        getattr(unit, op)(**row)


class UnitReview:
    def __init__(self, writer, unit_id, locations):
        self.writer, self.id, self.row = writer, unit_id, writer.data["units"][unit_id]
        self.ranges = [_text_range(writer.candidate_text, location, f"units/{unit_id}/candidate_locations/{index}")
                       for index, location in enumerate(locations)]
        self.terms = {row["id"]: TermReview(self, row) for row in self.row["term_candidates"]}
        self.source_path = Path(writer.source_bindings[self.row["source_id"]]["path"])
        for location in self.row["source_locations"]:
            self._pdf_location(location)

    def _pdf_location(self, locator):
        pages = self.writer._files.pages(self.source_path)
        page = locator.get("page")
        _require(type(page) is int and 1 <= page <= len(pages), "TRW_LOCATION", self.id, "Page is outside the actual PDF.")
        if "bbox" in locator:
            try:
                VALIDATE._Validator.box(locator["bbox"], pages[page - 1])
            except ValueError as error:
                raise ReviewWriterError("TRW_LOCATION", self.id, str(error)) from error

    def source(self, quote, *, page=None, bbox=None, block_id=None, occurrence=None, observation_basis=None):
        """Select recorded source text or an explicitly observed visible region.

        Visible transcription needs page+bbox+observation_basis. Its box must
        belong to this unit; the writer never claims to have viewed that region.
        """
        _require(_text(quote), "TRW_LOCATION", self.id, "Supply the selected original text or visible transcription.")
        selector = dict(quote=quote, page=page, bbox=deepcopy(bbox), block_id=block_id, occurrence=occurrence, observation_basis=observation_basis)
        parents = self.row["source_locations"]
        if block_id is not None:
            parents = [block for parent in parents for block in parent.get("block_locations", []) if block.get("block_id") == block_id]
            _require(bool(parents), "TRW_LOCATION", self.id, "The block ID does not belong to this source unit.")
        if page is not None:
            parents = [parent for parent in parents if parent["page"] == page]
        if bbox is not None:
            _require(page is not None, "TRW_LOCATION", self.id, "An explicit bbox needs its explicit page.")
            self._pdf_location({"page": page, "bbox": bbox})
            parents = [parent for parent in parents if PREPARE._valid_box(parent.get("bbox"))
                       and parent["bbox"][0] <= bbox[0] < bbox[2] <= parent["bbox"][2]
                       and parent["bbox"][1] <= bbox[1] < bbox[3] <= parent["bbox"][3]]
        if observation_basis is not None:
            _require(_text(observation_basis) and page is not None and bbox is not None,
                     "TRW_VISIBLE_BASIS", self.id, "Visible transcription needs a concrete page, bbox and caller-supplied observation basis.")
            parent = _choose(parents, occurrence, self.id)
            locator = {"page": parent["page"], "bbox": deepcopy(bbox), "quote": quote, "observation_basis": observation_basis}
            return Selection(self.id, "source", locator, quote, None, observation_basis, selector, self)
        matches = [(parent, index) for parent in parents
                   for index, _ in enumerate(_matches(parent.get("quote") or "", quote), 1)]
        parent, position = _choose(matches, occurrence, self.id)
        locator = {"page": parent["page"], "quote": quote, "occurrence_in_recorded_span": position}
        if "bbox" in parent:
            locator["bbox"] = deepcopy(bbox if bbox is not None else parent["bbox"])
        if block_id is not None:
            locator["block_id"] = block_id
        return Selection(self.id, "source", locator, quote, quote, None, selector, self)

    def candidate(self, quote, *, lines=None, occurrence=None, comparison_text=None, interpretation_basis=None):
        """Select actual Markdown bytes; keep any rendered reading separate.

        For an escaped table cell, quote selects the real YAML source; an
        explicitly supplied comparison_text and interpretation_basis record the
        caller's reading without claiming it is a substring of those bytes.
        """
        _require(_text(quote), "TRW_LOCATION", self.id, "Select a nonempty exact candidate source fragment.")
        text = self.writer.candidate_text
        limits = _text_range(text, {"lines": lines}, self.id) if lines is not None else (0, len(text))
        positions = sorted({span for begin, end in self.ranges for span in _matches(text, quote, max(begin, limits[0]), min(end, limits[1]))})
        begin, end = _choose(positions, occurrence, self.id)
        if comparison_text is not None:
            _require(_text(comparison_text) and _text(interpretation_basis), "TRW_RENDERED_BASIS", self.id, "A decoded/visible reading needs its explicit interpretation basis.")
        else:
            _require(interpretation_basis is None, "TRW_RENDERED_BASIS", self.id, "A rendering basis must accompany explicit comparison_text.")
        start_line, end_line = text.count("\n", 0, begin) + 1, text.count("\n", 0, end - 1) + 1
        locator = {"lines": [start_line, end_line], "quote": quote, "char_range": [begin, end]}
        line_range = _text_range(text, {"lines": [start_line, end_line]}, self.id)
        if not any(left <= line_range[0] <= line_range[1] <= right for left, right in self.ranges):
            # The unchanged validator resolves a lines locator to whole lines.
            # Keep an inline unit bounded by an actual unique quote instead.
            if len(_matches(text, quote)) == 1:
                locator = {"quote": quote, "source_lines": [start_line, end_line], "char_range": [begin, end]}
            else:
                parents = [(left, right) for left, right in self.ranges if left <= begin <= end <= right
                           and len(_matches(text, text[left:right])) == 1]
                _require(bool(parents), "TRW_LOCATION", self.id, "Use explicit candidate lines for this repeated inline fragment; no unique containing source span is available.")
                left, right = parents[0]
                locator = {"quote": text[left:right], "selected_quote": quote, "source_lines": [start_line, end_line], "char_range": [begin, end]}
        selector = dict(quote=quote, lines=deepcopy(lines), occurrence=occurrence,
                        comparison_text=comparison_text, interpretation_basis=interpretation_basis)
        return Selection(self.id, "candidate", locator, comparison_text if comparison_text is not None else quote,
                         quote, interpretation_basis, selector, self)

    def _selection(self, selected, side):
        _require(isinstance(selected, Selection) and selected._owner is self and selected.side == side,
                 "TRW_SELECTION_UNIT", self.id, "Use a selection made on this unit and the indicated side.")
        return selected

    def _evidence(self, source=None, candidate=None):
        result = ([{**self.writer.source_bindings[self.row["source_id"]], "locator": deepcopy(source.locator)}]
                  if source else self.writer._source_evidence(self.id))
        locations = [candidate.locator] if candidate else self.row["candidate_locations"]
        result.extend({**self.writer.data["bindings"]["candidate"], "locator": deepcopy(location)} for location in locations)
        return result

    def check(self, name, status, reason):
        _require(name in CHECKS, "TRW_CHECK", f"units/{self.id}/checks/{name}", "Unknown check; every prepared check remains required.")
        self.row["checks"][name] = _result(status, reason, self._evidence(), f"units/{self.id}/checks/{name}")
        self.writer._record("check", unit_id=self.id, name=name, status=status, reason=reason)

    def scan(self, category, status, candidate_ids, reason):
        _require(category in TERM_CATEGORIES, "TRW_CATEGORY", self.id, "Unknown term category.")
        ids = _ids(candidate_ids, f"units/{self.id}/term_scan/{category}")
        _require(all(term_id in self.terms for term_id in ids),
                 "TRW_TERM_MAPPING", self.id, "Register these terms in this unit before the scan decision; a term may explicitly participate in several categories.")
        _require(not (status == "scanned" and not ids) and not (status == "not_applicable" and ids),
                 "TRW_APPLICABILITY", self.id, "Scanned needs actual candidates; not_applicable cannot have candidates.")
        result = _result(status, reason, self._evidence(), f"units/{self.id}/term_scan/{category}",
                         {"pending", "scanned", "not_applicable", "blocked", "unresolved"})
        self.row["term_scan"][category] = {**result, "candidate_ids": ids}
        self.writer._record("scan", unit_id=self.id, category=category, status=status, candidate_ids=ids, reason=reason)

    def occurrence(self, kind, occurrence_id, source, candidate, status, reason, **comparison):
        _require(kind in OCCURRENCE_KINDS, "TRW_OCCURRENCE_KIND", self.id, "Unknown occurrence kind.")
        _require(_text(occurrence_id), "TRW_OCCURRENCE_ID", self.id, "Use an explicit nonempty occurrence ID.")
        _require(not any(row["id"] == occurrence_id for rows in self.row["occurrences"].values() for row in rows),
                 "TRW_DUPLICATE", occurrence_id, "Occurrence IDs are unique across this unit's categories.")
        source, candidate = self._selection(source, "source"), self._selection(candidate, "candidate")
        explicit = deepcopy(comparison)
        for key, selected in zip(_TEXT_PAIRS.get(kind, ()), (source, candidate)):
            _require(key not in comparison or comparison[key] == selected.text, "TRW_COMPARISON_TEXT", occurrence_id + "/" + key, "Use the explicitly selected text; select a different fragment if needed.")
            comparison[key] = selected.text
        if kind == "table_cells":
            for key, selected in (("source_cell", source), ("candidate_cell", candidate)):
                _require(isinstance(comparison.get(key), dict), "TRW_COMPARISON_FIELD", occurrence_id + "/" + key, "Supply this cell's actual table ID, row and column.")
                cell = comparison[key]
                _require("text" not in cell or cell["text"] == selected.text, "TRW_COMPARISON_TEXT", occurrence_id + "/" + key, "Cell text must be the explicitly selected comparison text.")
                cell["text"] = selected.text
        required = PREPARE.OCCURRENCE_FIELDS[kind]
        _require(set(comparison) == set(required), "TRW_COMPARISON_FIELD", occurrence_id,
                 "Supply the independent comparison fields: " + ", ".join(required))
        row = PREPARE.new_occurrence(occurrence_id, kind, source_span=source.locator)
        row.update(candidate_span=deepcopy(candidate.locator), **comparison,
                   **_result(status, reason, self._evidence(source, candidate), occurrence_id))
        for side, selected in (("source", source), ("candidate", candidate)):
            row[side + "_selection"] = {"raw_text": selected.raw_text, "comparison_text": selected.text, "interpretation_basis": selected.basis}
        # Reuse the unchanged validator's field/structure rules on this one row.
        report = VALIDATE.TranslationReviewReport()
        validator = VALIDATE._Validator(self.writer.path, report)
        validator.files = self.writer._files
        validator.source_paths[self.row["source_id"]] = self.source_path
        validator.candidate = self.writer.candidate_path
        groups = {name: [row] if name == kind else [] for name in OCCURRENCE_KINDS}
        validator.occurrences(groups, self.row, self.row["source_id"], self.ranges, self.id, {})
        errors = [finding for finding in report.findings if finding.level == "error"]
        if errors:
            error = errors[0]
            raise ReviewWriterError(error.code, error.object_id, error.message)
        self.row["occurrences"][kind].append(row)
        self.writer._record("occurrence", unit_id=self.id, kind=kind, occurrence_id=occurrence_id,
                            source=source.selector, candidate=candidate.selector, status=status, reason=reason, comparison=explicit)

    def term(self, term_id, original, category, source, *, aliases=()):
        _require(category in TERM_CATEGORIES, "TRW_CATEGORY", self.id, "Unknown term category.")
        _require(_text(term_id) and term_id not in self.terms, "TRW_DUPLICATE", self.id, "Use a new explicit term ID in this unit.")
        source = self._selection(source, "source")
        _require(_text(original) and original in source.text, "TRW_TERM_SOURCE", term_id, "Locate the original term in the explicitly selected source excerpt.")
        row = PREPARE.new_term_candidate(term_id, original, category, source_excerpt=source.text, source_locator=source.locator)
        row["aliases"] = _ids(aliases, term_id + "/aliases")
        row["source_selection"] = {"raw_text": source.raw_text, "comparison_text": source.text, "observation_basis": source.basis}
        self.row["term_candidates"].append(row)
        handle = TermReview(self, row)
        self.terms[term_id] = handle
        self.writer._record("term", unit_id=self.id, term_id=term_id, original=original, category=category, source=source.selector, aliases=list(aliases))
        return handle

    def issue(self, issue_id, source, description, candidate_readings, disposition, status, reason):
        source = self._selection(source, "source")
        _require(_text(issue_id) and not any(row["id"] == issue_id for row in self.writer.data["source_issues"]),
                 "TRW_DUPLICATE", str(issue_id), "Use a new explicit source-issue ID.")
        readings = _ids(candidate_readings, issue_id + "/candidate_readings")
        _require(bool(readings) and _text(description) and _text(disposition), "TRW_ISSUE", issue_id, "Record the actual ambiguity, possible readings and treatment.")
        self.writer.data["source_issues"].append({"id": issue_id, "unit_id": self.id, "source_locator": deepcopy(source.locator),
            "description": description, "candidate_readings": readings, "disposition": disposition,
            **_result(status, reason, self._evidence(source), issue_id)})
        self.writer._record("issue", unit_id=self.id, issue_id=issue_id, source=source.selector, description=description,
                            candidate_readings=readings, disposition=disposition, status=status, reason=reason)


def _hit(row):
    return {"entry_id": row["id"], "glossary_locator": {"lines": [row["line"], row["line"]]}, "excerpt": row["raw_line"]}


class TermReview:
    def __init__(self, unit, row):
        self.unit, self.row = unit, row

    def search(self, mode, query, interpretation):
        """Execute the real query; searched records that action, not equivalence."""
        _require(mode in {"full_phrase", "aliases", "family"}, "TRW_SEARCH_MODE", self.row["id"], "Unknown glossary-search mode.")
        _require(_text(query) and _text(interpretation), "TRW_REASON", self.row["id"], "Supply the actual query and its contextual interpretation.")
        if mode == "aliases":
            _require(bool(self.row["aliases"]), "TRW_ALIASES", self.row["id"], "Register the actual alias before searching it.")
        index = self.unit.writer._index()
        hits = [_hit(row) for row in index.search(query)]
        evidence = [{**index.binding, "locator": {"lines": [1, len(Path(index.binding["path"]).read_text(encoding="utf-8").splitlines())]}}]
        search = self.row["searches"][mode]
        search.update(_result("searched", interpretation, evidence, self.row["id"], {"searched"}))
        search["queries"].append({"query": query, "hits": hits, "interpretation": interpretation})
        self.unit.writer._record("search", unit_id=self.unit.id, term_id=self.row["id"], mode=mode, query=query, interpretation=interpretation)
        return deepcopy(hits)

    def search_status(self, mode, status, reason):
        _require(mode in {"full_phrase", "aliases", "family"}, "TRW_SEARCH_MODE", self.row["id"], "Unknown glossary-search mode.")
        _require(status != "searched", "TRW_SEARCH_ACTION", self.row["id"], "Use search() to perform and record a real query.")
        if status == "not_applicable":
            _require(mode == "aliases" and not self.row["aliases"], "TRW_APPLICABILITY", self.row["id"], "Only an explicitly absent alias set can skip this search; full phrase and family remain required.")
        _require(not self.row["searches"][mode]["queries"], "TRW_SEARCH_ACTION", self.row["id"], "Do not erase queries already performed.")
        self.row["searches"][mode].update(_result(status, reason, self.unit._evidence(), self.row["id"],
                                                 {"pending", "not_applicable", "blocked", "unresolved"}))
        self.unit.writer._record("search_status", unit_id=self.unit.id, term_id=self.row["id"], mode=mode, status=status, reason=reason)

    def decide(self, decision, translation, reason, *, matches=(), resolution_basis=None):
        _require(decision in {"pending", "adopted", "unlisted", "unresolved", "not_a_term"}, "TRW_TERM_DECISION", self.row["id"], "Supply an explicit terminology decision.")
        _require(_text(reason), "TRW_REASON", self.row["id"], "Explain this term's actual context and choice.")
        if decision in {"adopted", "unlisted"}:
            _require(_text(translation), "TRW_TERM_TRANSLATION", self.row["id"], "Supply the chosen translation explicitly.")
        if decision == "unlisted":
            _require(_text(resolution_basis), "TRW_TERM_BASIS", self.row["id"], "Explain the actual basis for resolving the unlisted translation.")
        ids = _ids(matches, self.row["id"] + "/matches")
        hits = {hit["entry_id"]: hit for search in self.row["searches"].values() for query in search["queries"] for hit in query["hits"]}
        _require(all(identity in hits for identity in ids), "TRW_GLOSSARY_HIT", self.row["id"], "Choose only entry IDs returned by this term's real searches; supplied hit objects are not accepted.")
        _require(decision != "adopted" or bool(ids), "TRW_GLOSSARY_HIT", self.row["id"], "Adoption requires an explicitly chosen actual query hit.")
        self.row.update(decision=decision, translation=translation, decision_reason=reason,
                        glossary_matches=[deepcopy(hits[identity]) for identity in ids], resolution_basis=resolution_basis)
        self.unit.writer._record("decide", unit_id=self.unit.id, term_id=self.row["id"], decision=decision,
                                 translation=translation, reason=reason, matches=ids, resolution_basis=resolution_basis)
