"""A complete translation-review record for one entirely invented paragraph.

From a Python script, call ``demonstrate(Path("an unused example directory"))``.
The example creates its own PDF, translation and four-entry glossary, calls the
real preparer, fills every target explicitly, and calls the unchanged validator.
It does not accept a paper or a candidate supplied by the caller. The judgments
below apply only to SOURCE_TEXT and CANDIDATE_TEXT; they are not defaults for a
real task and must never be copied as that task's review decisions.

Sections: fixed fictional inputs; file/search helpers; explicit occurrences;
seventeen checks; ten category scans; terminology decisions; final validation.
The resulting records_ready=True means that this record is complete and bound
to its files. It does not certify meaning or execution: source_fidelity and
execution_truth remain not_assessed, and independent review is still required.
Finish the current task's own self-review before validation; that independent
review flag is not a reason to leave the current task's targets pending.
The earlier fill_review_example.py remains a separate partial-record example.
"""

from __future__ import annotations

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile

import fitz


SKILL = Path(__file__).resolve().parents[1]
UNIT_ID = "fictional-paragraph"
SOURCE_SENTENCES = (
    "The blue tile lies inward of the amber rim.",
    "The tile's radial offset (RO) is 2 mm.",
    "During a static check, the operator is allowed to inspect the tile only after closing the latch.",
    "During the same check, the operator does not move the rim.",
)
CANDIDATE_SENTENCES = (
    "蓝色方片位于琥珀色边框的内侧。",
    "方片的径向偏移量（RO）为2 mm。",
    "在静态核验过程中，操作者只有在关上锁扣后才获准检查方片。",
    "在同一次核验过程中，操作者不移动边框。",
)
SOURCE_TEXT = " ".join(SOURCE_SENTENCES)
CANDIDATE_TEXT = "".join(CANDIDATE_SENTENCES)
GLOSSARY_TEXT = (
    "| ID | English | 中文 | Context |\n"
    "| --- | --- | --- | --- |\n"
    "| DEMO001 | inward of | 位于……内侧 | Object-to-reference relation in this fictional layout. |\n"
    "| DEMO002 | radial offset；别名/缩写：RO | 径向偏移量 | Displacement quantity in this fictional layout. |\n"
    "| DEMO003 | check | 核验 | General inspection procedure; no static modifier is specified. |\n"
    "| DEMO004 | radius | 半径 | Related geometric word; a radius is not an offset. |\n"
)


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, SKILL.joinpath("scripts", filename))
    if spec is None or spec.loader is None:
        raise ImportError(filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


prepare = _load("complete_example_translation_prepare", "prepare_translation_review.py")
validate = _load("complete_example_translation_validate", "validate_translation_review.py")
glossary_tools = _load("complete_example_glossary_candidates", "glossary_candidates.py")


def _write_new(path: Path, value) -> None:
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def _result(status: str, reason: str, evidence: list[dict]) -> dict:
    # No default status: every call below names its own judgment and reason.
    return {"status": status, "reason": reason, "evidence": deepcopy(evidence)}


def _query(index, query: str, expected_ids: tuple[str, ...], interpretation: str) -> dict:
    """Run the actual search; expected IDs guard this fixed demonstration only."""
    hits = index.search(query)
    if tuple(hit["id"] for hit in hits) != expected_ids:
        raise ValueError(f"The fictional glossary changed; reconsider the explicit decision for {query!r}.")
    return {
        "query": query,
        "hits": [
            {"entry_id": hit["id"], "glossary_locator": {"lines": [hit["line"], hit["line"]]},
             "excerpt": hit["raw_line"]}
            for hit in hits
        ],
        "interpretation": interpretation,
    }


def _occurrence(identity: str, kind: str, source_quote: str, candidate_quote: str,
                status: str, reason: str, evidence: list[dict], **comparison) -> dict:
    if source_quote not in SOURCE_TEXT or candidate_quote not in CANDIDATE_TEXT:
        raise ValueError("An example occurrence must locate the fixed fictional pair exactly.")
    row = prepare.new_occurrence(identity, kind, source_span={"page": 1, "quote": source_quote})
    row.update({"candidate_span": {"lines": [1, 1], "quote": candidate_quote}, **comparison})
    row.update(_result(status, reason, evidence))
    return row


def _explicit_occurrences(evidence: list[dict]) -> dict:
    """Every applicable instance is stated here, including repeated claim types."""
    a, b, c, d = SOURCE_SENTENCES
    za, zb, zc, zd = CANDIDATE_SENTENCES
    return {
        "claims": [
            _occurrence("claim-position", "claims", a, za, "passed",
                        "The blue tile, amber rim and inward relation all remain in the first translated sentence.", evidence,
                        source_claim=a, candidate_claim=za),
            _occurrence("claim-offset", "claims", b, zb, "passed",
                        "The tile owns the radial offset; RO, the value 2 and the unit mm are all retained.", evidence,
                        source_claim=b, candidate_claim=zb),
            _occurrence("claim-permission", "claims", c, zc, "passed",
                        "Inspection is permitted during the static check only after latch closure; the translation does not say inspection actually occurred.", evidence,
                        source_claim=c, candidate_claim=zc),
            _occurrence("claim-no-movement", "claims", d, zd, "passed",
                        "The same check, operator, negative move predicate and rim object remain in the last sentence.", evidence,
                        source_claim=d, candidate_claim=zd),
        ],
        "directions": [
            _occurrence("direction-inward", "directions", a, za, "passed",
                        "Inward is the tile's position relative to the rim; 内侧 preserves that ordered relation.", evidence,
                        source_relation={"subject": "blue tile", "reference": "amber rim", "relation": "inward of"},
                        candidate_relation={"subject": "蓝色方片", "reference": "琥珀色边框", "relation": "位于……内侧"}),
        ],
        "negations": [
            _occurrence("negative-move", "negations", "the operator does not move the rim", "操作者不移动边框", "passed",
                        "Does not negates moving the rim, not inspection or latch closure; 不 modifies the same predicate.", evidence,
                        source_scope={"actor": "operator", "negated_predicate": "move", "object": "rim"},
                        candidate_scope={"actor": "操作者", "negated_predicate": "移动", "object": "边框"}),
        ],
        "conditions": [
            _occurrence("condition-latch", "conditions", c, zc, "passed",
                        "Only after makes latch closure necessary before permission to inspect; 只有……后才 preserves that restriction and order.", evidence,
                        source_condition={"claim": "permission to inspect the tile", "necessary_prior_event": "closing the latch"},
                        candidate_condition={"claim": "获准检查方片", "necessary_prior_event": "关上锁扣"}),
            _occurrence("condition-inspection-check", "conditions", c, zc, "passed",
                        "The inspection permission applies during a static check; 在静态核验过程中 retains that stated context rather than extending the permission to every situation.", evidence,
                        source_condition={"claim": "conditional inspection permission", "context": "during a static check"},
                        candidate_condition={"claim": "有条件地获准检查方片", "context": "在静态核验过程中"}),
            _occurrence("condition-movement-check", "conditions", d, zd, "passed",
                        "The negative movement statement is restricted to the same check; 在同一次核验过程中 preserves this separate applicability boundary.", evidence,
                        source_condition={"claim": "operator does not move the rim", "context": "during the same check"},
                        candidate_condition={"claim": "操作者不移动边框", "context": "在同一次核验过程中"}),
        ],
        "antecedents": [
            _occurrence("antecedent-same-check", "antecedents", "the same check", "同一次核验", "passed",
                        "The only earlier check is a static check; 同一次 refers back to that same event without adding a second procedure.", evidence,
                        source_candidates=["a static check in the preceding sentence"],
                        candidate_candidates=["前句的静态核验"]),
        ],
        "modifiers": [
            _occurrence("modifier-blue", "modifiers", "blue tile", "蓝色方片", "passed",
                        "Blue modifies tile, not rim; 蓝色 attaches to 方片.", evidence,
                        source_candidates=["tile"], candidate_candidates=["方片"]),
            _occurrence("modifier-amber", "modifiers", "amber rim", "琥珀色边框", "passed",
                        "Amber modifies rim, not tile; 琥珀色 attaches to 边框.", evidence,
                        source_candidates=["rim"], candidate_candidates=["边框"]),
            _occurrence("modifier-radial", "modifiers", "radial offset", "径向偏移量", "passed",
                        "Radial qualifies the offset quantity, and 径向 qualifies 偏移量.", evidence,
                        source_candidates=["offset"], candidate_candidates=["偏移量"]),
            _occurrence("modifier-static", "modifiers", "static check", "静态核验", "passed",
                        "Static qualifies the check, and 静态 qualifies 核验.", evidence,
                        source_candidates=["check"], candidate_candidates=["核验"]),
            _occurrence("modifier-inspection-context", "modifiers", c, zc, "passed",
                        "During a static check restricts the inspection-permission statement in this sentence; the Chinese opening clause has the same scope.", evidence,
                        source_candidates=["operator is allowed to inspect the tile only after closing the latch"],
                        candidate_candidates=["操作者只有在关上锁扣后才获准检查方片"]),
            _occurrence("modifier-movement-context", "modifiers", d, zd, "passed",
                        "During the same check restricts the no-movement statement; the translation retains that context in this separate sentence.", evidence,
                        source_candidates=["operator does not move the rim"], candidate_candidates=["操作者不移动边框"]),
        ],
        "strength": [
            _occurrence("strength-position", "strength", a, za, "passed",
                        "Both versions assert a position without adding uncertainty, causality or a measurement method.", evidence,
                        source_strength="asserted spatial state", candidate_strength="陈述空间状态"),
            _occurrence("strength-offset", "strength", b, zb, "passed",
                        "Both versions state the offset value; neither calls it an estimate or an exact-error guarantee.", evidence,
                        source_strength="stated value", candidate_strength="陈述数值"),
            _occurrence("strength-permission", "strength", c, zc, "passed",
                        "Is allowed to is permission, retained as 获准; no completed inspection is asserted.", evidence,
                        source_strength="conditional permission, not an observed action", candidate_strength="有条件的许可，未声称已经检查"),
            _occurrence("strength-no-movement", "strength", d, zd, "passed",
                        "Does not move remains a negative action statement, not a weaker possibility or a stronger inability.", evidence,
                        source_strength="negative action statement", candidate_strength="否定动作陈述"),
        ],
        "numbers": [
            _occurrence("number-offset", "numbers", "2 mm", "2 mm", "passed",
                        "The only numeral is 2 and its value and association with the offset are unchanged.", evidence,
                        source_expression="2", candidate_expression="2", difference_type="unchanged"),
        ],
        "units": [
            _occurrence("unit-offset", "units", "2 mm", "2 mm", "passed",
                        "The only unit occurrence is mm attached to radial offset; no conversion or unit substitution was made.", evidence,
                        source_expression="mm", candidate_expression="mm",
                        source_quantity="tile's radial offset", candidate_quantity="方片的径向偏移量"),
        ],
        # Empty groups are justified individually by the corresponding checks.
        "symbols": [],
        "formulas": [],
        "citation_labels": [],
        "citation_claims": [],
        "table_cells": [],
    }


def _explicit_checks(evidence: list[dict]) -> dict:
    return {
        "alignment": _result("passed", "All four source sentences form the sole source paragraph and map, in order, to candidate line 1.", evidence),
        "ambiguity": _result("not_applicable", "The named tile, rim and operator have distinct roles; RO is explicitly expanded, and the same check has exactly one earlier check as its referent. This invented paragraph has no unresolved competing reading to preserve.", evidence),
        "negation": _result("passed", "The one negative predicate, does not move the rim, is retained as 不移动边框; negative-move records its scope.", evidence),
        "strength": _result("passed", "The four strength records preserve two asserted states, one conditional permission and one negative action; permission was not changed into a completed inspection.", evidence),
        "number_changes": _result("passed", "The sole numeral 2 in the offset value remains 2, as recorded in number-offset.", evidence),
        "terminology": _result("passed", "All ten categories are explicitly scanned below. Inward of and radial offset use same-concept entries; static check is resolved as an unlisted phrase using its actual modifier and the related check entry.", evidence),
        "units": _result("passed", "The sole mm occurrence still measures the tile's radial offset; unit-offset compares both expressions and quantities.", evidence),
        "symbols": _result("not_applicable", "No mathematical variable or symbol definition occurs. RO is the spelled-out quantity's abbreviation, and mm is a unit; both are reviewed in their actual categories.", evidence),
        "formulas": _result("not_applicable", "The entire paragraph has no equation or inline/displayed formula; the prose value 2 mm is covered by numbers and units.", evidence),
        "citation_labels": _result("not_applicable", "None of the four source sentences displays a reference number or author-year citation label.", evidence),
        "completeness": _result("passed", "Four independent claims are recorded: tile position, offset value, conditional inspection permission, and no rim movement during the same check. Each has its corresponding translated sentence.", evidence),
        "directions": _result("passed", "The one subject/reference spatial relation places the blue tile inward of the amber rim; direction-inward preserves all three parts.", evidence),
        "citation_claims": _result("not_applicable", "The invented paragraph invokes no bibliography or external source to support any of its four claims.", evidence),
        "table_text": _result("not_applicable", "The PDF consists of one prose paragraph, with no data table, table caption or table cell to translate.", evidence),
        "conditions": _result("passed", "Three condition records preserve latch closure before inspection permission and the separate check contexts for inspection permission and no rim movement.", evidence),
        "antecedents": _result("passed", "The same check refers to the single previously introduced static check; 同一次核验 preserves that reference in antecedent-same-check.", evidence),
        "modifier_scope": _result("passed", "Six records preserve the blue, amber, radial and static modifiers and the separate check-context clauses for inspection permission and rim movement.", evidence),
    }


def _explicit_scans(evidence: list[dict]) -> dict:
    return {
        "concept": {**_result("scanned", "The paragraph uses the inward relation, radial-offset quantity and static-check procedure; these three candidates are registered once each.", evidence), "candidate_ids": ["inward", "offset", "static-check"]},
        "method": {**_result("not_applicable", "No analysis, fitting, optimization or computation method is named. The static check is an inspection procedure, registered under procedure.", evidence), "candidate_ids": []},
        "estimator": {**_result("not_applicable", "The source states an offset value but gives no estimator, estimation rule or estimation algorithm.", evidence), "candidate_ids": []},
        "statistic": {**_result("not_applicable", "The single value 2 mm is not presented as a mean, variance, quantile or other statistical summary.", evidence), "candidate_ids": []},
        "metric": {**_result("scanned", "Radial offset is the stated displacement quantity; the same offset candidate is relevant to this metric scan.", evidence), "candidate_ids": ["offset"]},
        "procedure": {**_result("scanned", "Static check names the inspection procedure. Closing the latch is its explicit prior action, not a separately named technical algorithm.", evidence), "candidate_ids": ["static-check"]},
        "model": {**_result("not_applicable", "No model, model family, simulation or solver is named anywhere in the paragraph.", evidence), "candidate_ids": []},
        "parameter": {**_result("scanned", "The tile has the explicitly named radial offset quantity, with abbreviation RO and value 2 mm.", evidence), "candidate_ids": ["offset"]},
        "classification": {**_result("not_applicable", "Blue and amber describe two objects; the source defines no classification scheme, class code or grouping rule.", evidence), "candidate_ids": []},
        "abbreviation": {**_result("scanned", "RO is explicitly introduced after radial offset and belongs to that candidate; mm is a unit and has its own occurrence record.", evidence), "candidate_ids": ["offset"]},
    }


def _explicit_terms(index, glossary_evidence: list[dict], pair_evidence: list[dict]) -> list[dict]:
    inward = prepare.new_term_candidate("inward", "inward of", "concept",
        source_excerpt=SOURCE_SENTENCES[0], source_locator={"page": 1, "quote": "inward of"})
    inward_full = _query(index, "inward of", ("DEMO001",), "The full phrase describes the same object/reference relation as the source.")
    inward_family = _query(index, "inward", ("DEMO001",), "The core directional word returns the same entry; no different reference direction is substituted.")
    inward.update({
        "searches": {
            "full_phrase": {**_result("searched", "The actual complete phrase inward of was queried in the four-row glossary.", glossary_evidence), "queries": [inward_full]},
            "aliases": {**_result("not_applicable", "The paragraph provides no alias for inward of; RO explicitly names radial offset instead.", pair_evidence), "queries": []},
            "family": {**_result("searched", "The core word inward was separately queried and its hit read in the tile/rim context.", glossary_evidence), "queries": [inward_family]},
        },
        "decision": "adopted", "translation": "位于……内侧",
        "decision_reason": "DEMO001 expresses the same tile-to-rim relation; its Chinese construction retains the explicit reference object.",
        "glossary_matches": deepcopy(inward_full["hits"]), "terminology_pointer": "/adopted/0",
    })

    offset = prepare.new_term_candidate("offset", "radial offset", "parameter",
        source_excerpt=SOURCE_SENTENCES[1], source_locator={"page": 1, "quote": "radial offset (RO)"})
    offset_full = _query(index, "radial offset", ("DEMO002",), "The full quantity name matches the fictional glossary's displacement entry.")
    offset_alias = _query(index, "RO", ("DEMO002",), "The source and glossary both explicitly give RO as radial offset's abbreviation.")
    offset_radial = _query(index, "radial", ("DEMO002",), "The core modifier occurs in the offset entry and is retained as 径向.")
    offset_radius = _query(index, "radius", ("DEMO004",), "Radius is a related geometric word, not the same quantity; 半径 must not replace the full offset translation.")
    offset.update({
        "aliases": ["RO"],
        "searches": {
            "full_phrase": {**_result("searched", "The actual complete phrase radial offset was searched.", glossary_evidence), "queries": [offset_full]},
            "aliases": {**_result("searched", "The source's explicit abbreviation RO was separately searched.", glossary_evidence), "queries": [offset_alias]},
            "family": {**_result("searched", "Radial and the related word radius were separately queried; the radius hit was read and rejected as a full-phrase substitute.", glossary_evidence), "queries": [offset_radial, offset_radius]},
        },
        "decision": "adopted", "translation": "径向偏移量",
        "decision_reason": "DEMO002 matches the source quantity and explicit abbreviation. DEMO004 is family context only and does not rename the quantity as a radius.",
        "glossary_matches": deepcopy(offset_full["hits"]), "terminology_pointer": "/adopted/1",
    })

    static_check = prepare.new_term_candidate("static-check", "static check", "procedure",
        source_excerpt=SOURCE_SENTENCES[2], source_locator={"page": 1, "quote": "static check"})
    static_full = _query(index, "static check", (), "There is no entry for the complete phrase; a check-only match is not a full-phrase adoption.")
    static_family = _query(index, "check", ("DEMO003",), "The related generic procedure entry supplies 核验; the source's static modifier still needs to be retained.")
    static_check.update({
        "searches": {
            "full_phrase": {**_result("searched", "The complete phrase static check was queried and returned zero hits.", glossary_evidence), "queries": [static_full]},
            "aliases": {**_result("not_applicable", "No alias or abbreviation for static check is introduced. The same check is a later reference to the event, not an alternative term.", pair_evidence), "queries": []},
            "family": {**_result("searched", "The core word check was actually queried and the generic entry read.", glossary_evidence), "queries": [static_family]},
        },
        "decision": "unlisted", "translation": "静态核验",
        "decision_reason": "The full phrase is absent. For this fictional procedure, retain the stated static modifier as 静态 and the related check entry as 核验; do not claim a full glossary match.",
        "glossary_matches": deepcopy(static_family["hits"]), "terminology_pointer": "/unlisted/0",
    })
    return [inward, offset, static_check]


def demonstrate(work_root: Path, *, progress=prepare.progress_bar) -> dict:
    """Create a new, complete fictional example without overwriting any files."""
    work = Path(work_root).resolve()
    work.mkdir(parents=True, exist_ok=False)
    source = work.joinpath("fictional-source.pdf")
    candidate = work.joinpath("fictional-translation.md")
    glossary = work.joinpath("fictional-glossary.md")
    inventory_path = work.joinpath("source-inventory.json")
    terminology_path = work.joinpath("terminology.json")
    reviewed_path = work.joinpath("translation-review.json")

    # All task data is authored here; nothing is copied from a real paper/run.
    glossary.write_text(GLOSSARY_TEXT, encoding="utf-8")
    glossary_binding = prepare.bind_review_file(glossary)
    index = glossary_tools.GlossaryIndex(glossary)
    with fitz.open() as document:
        page = document.new_page(width=640, height=220)
        remaining = page.insert_textbox(fitz.Rect(24, 24, 616, 196), SOURCE_TEXT, fontsize=11)
        if remaining < 0:
            raise ValueError("The complete fictional source did not fit on its page.")
        document.save(source)
    with fitz.open(source) as document:
        blocks = document[0].get_text("blocks")
        actual_text = " ".join(document[0].get_text().split())
    if len(blocks) != 1 or actual_text != SOURCE_TEXT:
        raise ValueError("The generated PDF no longer contains exactly the reviewed fictional paragraph.")
    candidate.write_text(CANDIDATE_TEXT + "\n", encoding="utf-8")
    if candidate.read_text(encoding="utf-8") != CANDIDATE_TEXT + "\n":
        raise ValueError("The candidate no longer matches this example's explicit judgments.")
    block = blocks[0]
    inventory = {
        "schema_version": 1,
        "sources": [{"id": "fictional-source", **prepare.bind_review_file(source), "page_count": 1,
                     "pages": [{"page": 1, "blocks": [{"id": "fictional-block", "bbox": list(block[:4]), "text": block[4]}]}]}],
        "units": [{"id": UNIT_ID, "source": "fictional-source", "kind": "paragraph",
                   "evidence": [{"page": 1, "basis": "text_blocks", "blocks": ["fictional-block"]}]}],
    }
    _write_new(inventory_path, inventory)
    pending_path = prepare.prepare_translation_review(
        inventory_path, candidate, work.joinpath("translation-review.pending.json"),
        unit_ids=[UNIT_ID], glossary_path=glossary,
        candidate_locations={UNIT_ID: [{"lines": [1, 1], "quote": CANDIDATE_TEXT}]}, progress=progress,
    )
    ledger = json.loads(pending_path.read_text(encoding="utf-8"))
    if ledger["bindings"]["glossary"] != glossary_binding or index.binding != glossary_binding:
        raise ValueError("The example must search and review the same actual glossary bytes.")

    # Hash the common pair once, then reuse its concrete evidence for each
    # explicitly named judgment. No loop decides pass/NA for unknown targets.
    pair_evidence = [
        prepare.bind_review_evidence(source, {"page": 1, "quote": SOURCE_TEXT}),
        prepare.bind_review_evidence(candidate, {"lines": [1, 1], "quote": CANDIDATE_TEXT}),
    ]
    glossary_evidence = [prepare.bind_review_evidence(glossary, {"lines": [1, 6]})]
    unit = ledger["units"][UNIT_ID]
    unit["checks"] = _explicit_checks(pair_evidence)
    unit["occurrences"] = _explicit_occurrences(pair_evidence)
    unit["term_scan"] = _explicit_scans(pair_evidence)
    unit["term_candidates"] = _explicit_terms(index, glossary_evidence, pair_evidence)
    if set(unit["checks"]) != set(prepare.CHECKS) or set(unit["term_scan"]) != set(prepare.TERM_CATEGORIES):
        raise ValueError("The review contract changed; add new explicit judgments instead of defaulting their status.")

    scope_evidence = [prepare.bind_review_evidence(inventory_path, {"kind": "json_pointer", "pointer": "/units/0"})]
    ledger["scope"]["selection_review"] = _result(
        "passed", "The entire fictional PDF is one paragraph containing these four sentences. Its sole inventory unit is selected and wholly mapped to candidate line 1; no unit is excluded.", scope_evidence + pair_evidence)
    ledger["source_issues_review"] = _result(
        "passed", "All four invented sentences have one legible reading in the generated PDF. RO is expanded, the offset value is unambiguous, and the same check has one explicit antecedent; no source-reading conflict was found in this complete scope.", pair_evidence)

    terminology = {
        "schema_version": 1, "review_status": "reviewed", "glossary": deepcopy(glossary_binding),
        "covered_units": [UNIT_ID],
        "adopted": [
            {"original": "inward of", "translation": "位于……内侧", "context": SOURCE_SENTENCES[0],
             "source_units": [UNIT_ID], "glossary_locator": {"lines": [3, 3]}},
            {"original": "radial offset", "translation": "径向偏移量", "context": SOURCE_SENTENCES[1],
             "source_units": [UNIT_ID], "glossary_locator": {"lines": [4, 4]}},
        ],
        "unlisted": [
            {"original": "static check", "translation": "静态核验", "context": SOURCE_SENTENCES[2],
             "source_units": [UNIT_ID], "status": "resolved",
             "reason": "The actual complete-phrase search has zero hits; only the core word check is listed.",
             "resolution_basis": "For this fictional source, retain static as 静态 and use 核验 from the related DEMO003 entry. This is an explicitly reasoned phrase choice, not a claimed whole-phrase glossary match."},
        ],
        "no_unlisted_reason": None,
    }
    _write_new(terminology_path, terminology)
    ledger["terminology_record"] = prepare.bind_review_file(terminology_path)
    ledger["review_status"] = "reviewed"
    ledger["evidence_boundary"] = (
        "Explicit judgments for this fixed fictional pair only. Completeness of the record is checked by the existing validator; "
        "the validator neither certifies these semantic judgments nor proves historical review actions."
    )
    # No invented event trace and no unresolved object were added. The source
    # issues and term decisions above explain the reviewed empty issue lists.
    _write_new(reviewed_path, ledger)
    report = validate.validate_translation_review(reviewed_path, inventory_path, candidate).to_dict()
    _write_new(work.joinpath("translation-record-validation.json"), report)
    if not report["records_ready"]:
        raise ValueError(json.dumps(report["findings"], ensure_ascii=False, indent=2))
    if report["source_fidelity"] != "not_assessed" or report["execution_truth"] != "not_assessed":
        raise ValueError("A structural example must not claim machine certification of meaning or execution.")
    return {
        "review_path": str(reviewed_path), "pending_path": str(pending_path),
        "inventory_path": str(inventory_path), "candidate_path": str(candidate),
        "glossary_path": str(glossary), "terminology_path": str(terminology_path), "report": report,
    }


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as temporary:
        result = demonstrate(Path(temporary).joinpath("complete-example"))
        print(json.dumps({key: result["report"][key] for key in (
            "records_ready", "source_fidelity", "execution_truth", "independent_review_required",
            "selected_unit_count", "pending_ids", "unresolved_ids", "failed_ids",
        )}, ensure_ascii=False, indent=2))
