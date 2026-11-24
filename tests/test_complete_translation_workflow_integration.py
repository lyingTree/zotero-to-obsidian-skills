"""A fully bound translation-required workflow using only invented source text.

The four-sentence example supplies its explicit semantic decisions, never saved
review files. This fixture generates fresh PDFs, full-page provenance, candidate,
glossary, inventory, evidence and all final bindings on every construction.
"""
from __future__ import annotations

from copy import deepcopy
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(str(path))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


support = load_module("translation_workflow_support", ROOT.joinpath("tests", "test_complete_workflow_integration.py"))
example = load_module("translation_workflow_example", ROOT.joinpath("literature-translate-zh", "references", "complete_review_example.py"))
writer_tools = load_module("translation_workflow_writer", ROOT.joinpath("literature-translate-zh", "scripts", "translation_review_writer.py"))
workflow, read, save, digest = support.workflow, support.read, support.save, support.digest
TITLE_ID = "fictional-title"
PARAGRAPH_ID = example.UNIT_ID
SOURCE_TITLE = "Example"
TRANSLATED_TITLE = "示例"
FORMATTED_VALUE = r"$2\,\text{mm}$"
TRANSLATED_PARAGRAPH = example.CANDIDATE_TEXT.replace("2 mm", FORMATTED_VALUE)


def decision(status: str, reason: str, evidence: list[dict]) -> dict:
    return {"status": status, "reason": reason, "evidence": deepcopy(evidence)}


def title_decisions(pair: list[dict], line: int) -> dict:
    """Decide each title target explicitly; there are no fallback statuses."""
    checks = {
        "alignment": decision("passed", f"The one-word source title Example maps to 示例 on candidate line {line}; the English title is also retained above it.", pair),
        "ambiguity": decision("not_applicable", "The source uses Example as a document label, with no competing technical reading or attachment ambiguity.", pair),
        "negation": decision("not_applicable", "The entire title is Example and contains no negative predicate or negating prefix.", pair),
        "strength": decision("not_applicable", "Example is a title label; it makes no modal, evidential or certainty assertion whose force could change.", pair),
        "number_changes": decision("not_applicable", "The one-word title contains no numeral, numerical expression or count.", pair),
        "terminology": decision("passed", "All ten categories below were considered for the actual title Example; this ordinary document label names no professional term or abbreviation.", pair),
        "units": decision("not_applicable", "Example contains no measurement unit or measured quantity.", pair),
        "symbols": decision("not_applicable", "Example contains no mathematical symbol or variable definition.", pair),
        "formulas": decision("not_applicable", "The entire title is a word, without an equation, operator or formula.", pair),
        "citation_labels": decision("not_applicable", "The title contains no reference number or author-year citation label.", pair),
        "completeness": decision("passed", "The title's only independent content is the label Example; 示例 retains this complete label without adding a subject or result.", pair),
        "directions": decision("not_applicable", "Example specifies no subject/reference pair, direction or spatial relation.", pair),
        "citation_claims": decision("not_applicable", "The title makes no attribution to a publication or bibliography entry.", pair),
        "table_text": decision("not_applicable", "This block is the document title, with no table caption, header, cell or table note.", pair),
        "conditions": decision("not_applicable", "Example contains no condition, exception, temporal restriction or applicability boundary.", pair),
        "antecedents": decision("not_applicable", "The title contains no pronoun, demonstrative or other reference to an earlier object.", pair),
        "modifier_scope": decision("not_applicable", "The title is the unmodified noun Example; there is no adjective or subordinate phrase to attach.", pair),
    }
    scans = {
        "concept": {**decision("not_applicable", "Example is an ordinary title label, not a named technical concept.", pair), "candidate_ids": []},
        "method": {**decision("not_applicable", "Example names no analysis, fitting, computation or optimization method.", pair), "candidate_ids": []},
        "estimator": {**decision("not_applicable", "No estimator or estimation rule occurs in the title Example.", pair), "candidate_ids": []},
        "statistic": {**decision("not_applicable", "No statistical summary or statistical term occurs in Example.", pair), "candidate_ids": []},
        "metric": {**decision("not_applicable", "The title Example introduces no measured quantity or evaluation metric.", pair), "candidate_ids": []},
        "procedure": {**decision("not_applicable", "Example names a document, not a procedure or sequence of actions.", pair), "candidate_ids": []},
        "model": {**decision("not_applicable", "Example does not name a model, simulation, solver or model family.", pair), "candidate_ids": []},
        "parameter": {**decision("not_applicable", "Example specifies no parameter, variable or tunable value.", pair), "candidate_ids": []},
        "classification": {**decision("not_applicable", "The title provides no classification system, group code or category definition.", pair), "candidate_ids": []},
        "abbreviation": {**decision("not_applicable", "Example is written as a complete ordinary word and introduces no abbreviation.", pair), "candidate_ids": []},
    }
    title = example.prepare.new_occurrence("title-label", "claims", source_span={"page": 1, "quote": SOURCE_TITLE})
    title.update({
        "candidate_span": {"lines": [line, line], "quote": TRANSLATED_TITLE},
        "source_claim": "Example", "candidate_claim": "示例",
        **decision("passed", "The complete one-word title label is preserved as 示例, without inventing a research topic or conclusion.", pair),
    })
    occurrences = {
        "claims": [title], "directions": [], "negations": [], "conditions": [],
        "antecedents": [], "modifiers": [], "strength": [], "numbers": [],
        "units": [], "symbols": [], "formulas": [], "citation_labels": [],
        "citation_claims": [], "table_cells": [],
    }
    return {"checks": checks, "occurrences": occurrences, "term_scan": scans, "term_candidates": []}


def paragraph_decisions(pair: list[dict], glossary_pair: list[dict], index, line: int) -> dict:
    """Reuse the fixed invented facts while binding this run's real locations."""
    checks = example._explicit_checks(pair)
    checks["alignment"]["reason"] = f"The source paragraph's four sentences map in order to all four translated sentences on candidate line {line}; the title has a separate complete review."
    checks["table_text"]["reason"] = "This generated PDF has a title and one prose paragraph, with no table, caption, cell or table note."
    checks["formulas"]["reason"] = "The source paragraph contains no equation or formula. The candidate uses math typography only for the unchanged quantity 2 mm, reviewed under numbers and units."
    occurrences = example._explicit_occurrences(pair)
    for rows in occurrences.values():
        for row in rows:
            row["candidate_span"]["lines"] = [line, line]
            row["candidate_span"]["quote"] = row["candidate_span"]["quote"].replace("2 mm", FORMATTED_VALUE)
            if "candidate_claim" in row:
                row["candidate_claim"] = row["candidate_claim"].replace("2 mm", FORMATTED_VALUE)
    occurrences["numbers"][0]["difference_type"] = "representation_only"
    occurrences["numbers"][0]["reason"] = "The only numeral remains 2 and measures radial offset; its containing quantity uses math typography without changing value."
    return {
        "checks": checks, "occurrences": occurrences,
        "term_scan": example._explicit_scans(pair),
        "term_candidates": example._explicit_terms(index, glossary_pair, pair),
    }


class SyntheticTranslatedWorkflow:
    """Generate and complete a new source-to-candidate translation workflow."""

    def __init__(self, base: Path):
        self.base = base
        self.base.mkdir(parents=True, exist_ok=True)
        self.skills = base.joinpath("skills")
        self.rule_file = self.skills.joinpath("synthetic-translation", "SKILL.md")
        self.rule_file.parent.mkdir(parents=True)
        self.rule_file.write_text(
            "# Synthetic translation workflow rules\n"
            "- [ ] [FLOW-A01] Each source title and paragraph has its mapped candidate translation. <!-- check-type: offline_outcome -->\n"
            "- [ ] [FLOW-A02] Each recorded local link resolves to its declared anchor. <!-- check-type: offline_outcome -->\n"
            "- [ ] [FLOW-A03] If source figures exist, each final image asset is recorded. <!-- check-type: offline_outcome -->\n"
            "- [ ] [FLOW-A04] A historical opening action was observed. <!-- check-type: execution_process -->\n",
            encoding="utf-8",
        )
        self.source = base.joinpath("input", "example.pdf")
        self.source.parent.mkdir()
        with fitz.open() as document:
            page = document.new_page(width=640, height=240)
            page.insert_text((24, 30), SOURCE_TITLE, fontsize=11)
            remaining = page.insert_textbox(fitz.Rect(24, 65, 616, 216), example.SOURCE_TEXT, fontsize=11)
            if remaining < 0:
                raise ValueError("The invented paragraph must fit entirely on the source page.")
            document.save(self.source)
        with fitz.open(self.source) as document:
            blocks = document[0].get_text("blocks")
            actual = [" ".join(block[4].split()) for block in blocks]
        if actual != [SOURCE_TITLE, example.SOURCE_TEXT]:
            raise ValueError("Reconsider the explicit decisions if either generated source block changes.")
        self.output = base.joinpath("output")
        self.output.mkdir()
        self.candidate = self.output.joinpath("candidate.md")
        self.candidate.write_text(
            "---\n类型: 期刊论文\n年份: null\n关键词: []\n期刊名: null\n---\n"
            "# Example\n示例\n\n## 1 正文\n" + TRANSLATED_PARAGRAPH + "\n^fictional-paragraph\n",
            encoding="utf-8",
        )
        lines = self.candidate.read_text(encoding="utf-8").splitlines()
        self.title_line = lines.index(TRANSLATED_TITLE) + 1
        self.paragraph_line = lines.index(TRANSLATED_PARAGRAPH) + 1
        self.glossary = base.joinpath("input", "glossary.md")
        self.glossary.write_text(example.GLOSSARY_TEXT, encoding="utf-8")
        self.initial_bindings = {str(path): digest(path) for path in (self.source, self.candidate, self.glossary)}
        self.work = base.joinpath("work")
        self.run_path = workflow.initialize_workflow(
            skills_root=self.skills, work_root=self.work,
            sources={"main": self.source}, source_roles={"main": "main"},
            candidate_path=self.candidate, resource_root=self.output,
            glossary_path=self.glossary, translation_required=True, progress=lambda *_: None,
        )
        self.run = read(self.run_path)
        self.records = {name: Path(value) for name, value in self.run["records"].items()}
        self.inventory_path = self.records["source_inventory"]
        self.check_tools = workflow._module("check_records")
        inventory = read(self.inventory_path)
        # Keep the initialization's pending terminology file. The writer emits
        # fresh final files and binds the inventory after these pointers settle.
        self.records["terminology"] = self.work.joinpath("evidence", "terminology-reviewed.json")
        self.run["records"]["terminology"] = str(self.records["terminology"])
        inventory["terminology_path"] = str(self.records["terminology"])
        source = inventory["sources"][0]
        title_block, paragraph_block = source["pages"][0]["blocks"]
        all_page = {"page": 1, "bbox": [0, 0, 640, 240], "basis": "visual"}
        source["header_fields"] = {
            "title": {"status": "present", "units": [TITLE_ID]},
            **{field: {"status": "absent_in_source",
                       "reason": f"The full generated page prints only Example and the four-sentence paragraph, with no {field}.",
                       "evidence": [deepcopy(all_page)]}
               for field in ("authors", "affiliations", "publication_date")},
        }
        inventory["units"] = [
            {"id": TITLE_ID, "source": "main", "kind": "metadata", "field": "title",
             "evidence": [{**all_page, "blocks": [title_block["id"]], "quote": SOURCE_TITLE}]},
            {"id": PARAGRAPH_ID, "source": "main", "kind": "paragraph",
             # The generated PDF's wrapped text was independently joined and
             # checked above; retain that logical quote alongside exact blocks.
             "evidence": [{"page": 1, "basis": "text", "blocks": [paragraph_block["id"]], "quote": example.SOURCE_TEXT}]},
        ]
        save(self.inventory_path, inventory)
        self.deliveries = [
            {"unit": TITLE_ID, "target": {"quote": TRANSLATED_TITLE}},
            {"unit": PARAGRAPH_ID, "target": {"anchor": PARAGRAPH_ID}},
        ]
        self.run["naming"] = workflow._module("format_helpers").build_naming_plan(
            year=None, no_year_reason="The generated source prints no publication year.",
            author_token="未知作者", unknown_author_reason="The generated source prints no author.",
            original_title=SOURCE_TITLE,
            source_evidence={field: {"file": self.check_tools.bind_file(self.source),
                                     "locator": {"page": 1, "bbox": [0, 0, 640, 240]}}
                             for field in ("year", "author_token", "original_title")},
            delivery_path=self.candidate,
        )
        save(self.run_path, self.run)
        self._review_translation()
        maps = workflow.write_maps(self.run_path, self.deliveries)
        if not maps.records_ready:
            raise ValueError(str(maps.as_dict()))
        workflow.bind_candidate(self.run_path)
        self._review_workflow_checks(len(lines))
        workflow.write_manifest(self.run_path, self.deliveries)

    def _review_translation(self) -> None:
        self.pending_path = self.work.joinpath("translation-review.pending.json")
        self.draft_path = self.work.joinpath("translation-decisions.json")
        locations = {
            TITLE_ID: [{"lines": [self.title_line, self.title_line]}],
            PARAGRAPH_ID: [{"lines": [self.paragraph_line, self.paragraph_line]}],
        }
        example.prepare.prepare_translation_review(
            self.inventory_path, self.candidate, self.pending_path,
            unit_ids=[TITLE_ID, PARAGRAPH_ID], glossary_path=self.glossary,
            candidate_locations=locations, progress=None,
        )
        writer = writer_tools.ReviewWriter(self.pending_path, progress=None)
        title_pair = [
            example.prepare.bind_review_evidence(self.source, {"page": 1, "quote": SOURCE_TITLE}),
            example.prepare.bind_review_evidence(self.candidate, {"lines": [self.title_line, self.title_line], "quote": TRANSLATED_TITLE}),
        ]
        paragraph_pair = [
            example.prepare.bind_review_evidence(self.source, {"page": 1, "quote": example.SOURCE_TEXT}),
            example.prepare.bind_review_evidence(self.candidate, {"lines": [self.paragraph_line, self.paragraph_line], "quote": TRANSLATED_PARAGRAPH}),
        ]
        glossary_evidence = [example.prepare.bind_review_evidence(self.glossary, {"lines": [1, 6]})]
        index = example.glossary_tools.GlossaryIndex(self.glossary)
        decisions = {
            TITLE_ID: title_decisions(title_pair, self.title_line),
            PARAGRAPH_ID: paragraph_decisions(paragraph_pair, glossary_evidence, index, self.paragraph_line),
        }
        for identity, reviewed in decisions.items():
            unit = writer.unit(identity, candidate_locations=locations[identity])
            if (set(reviewed["checks"]) != set(example.prepare.CHECKS)
                    or set(reviewed["term_scan"]) != set(example.prepare.TERM_CATEGORIES)
                    or set(reviewed["occurrences"]) != set(example.prepare.OCCURRENCE_KINDS)):
                raise ValueError("The review contract changed; author each new decision explicitly.")
            for kind, rows in reviewed["occurrences"].items():
                for row in rows:
                    comparison = {field: deepcopy(row[field]) for field in example.prepare.OCCURRENCE_FIELDS[kind]}
                    source = unit.source(row["source_span"]["quote"])
                    candidate = unit.candidate(row["candidate_span"]["quote"])
                    if kind == "numbers":
                        # Select actual math source, and explain its comparison
                        # reading without pretending the rendered value is raw.
                        candidate = unit.candidate(FORMATTED_VALUE, comparison_text="2 mm",
                            interpretation_basis="Inline math delimiters, thin spacing and the text-unit wrapper display the same numeral 2 followed by mm.")
                        comparison.pop("source_expression")
                        comparison.pop("candidate_expression")
                    if kind == "units":
                        source = unit.source("mm")
                        candidate = unit.candidate(r"\text{mm}", comparison_text="mm",
                            interpretation_basis="The text command displays the upright unit letters mm within inline math.")
                        comparison.pop("source_expression")
                        comparison.pop("candidate_expression")
                    unit.occurrence(kind, row["id"], source, candidate, row["status"], row["reason"], **comparison)
            for row in reviewed["term_candidates"]:
                term = unit.term(row["id"], row["original"], row["category"],
                                 unit.source(row["source_excerpt"]), aliases=row["aliases"])
                for mode, search in row["searches"].items():
                    if search["status"] != "searched":
                        term.search_status(mode, search["status"], search["reason"])
                        continue
                    for query in search["queries"]:
                        actual = term.search(mode, query["query"], query["interpretation"])
                        if [hit["entry_id"] for hit in actual] != [hit["entry_id"] for hit in query["hits"]]:
                            raise ValueError("The invented glossary search changed; reconsider this term decision.")
                resolution = None
                if row["id"] == "static-check":
                    resolution = "The full phrase has zero actual hits; retain static as 静态 and the related DEMO003 check entry as 核验, without claiming whole-phrase glossary adoption."
                term.decide(row["decision"], row["translation"], row["decision_reason"],
                            matches=[hit["entry_id"] for hit in row["glossary_matches"]], resolution_basis=resolution)
            for category, row in reviewed["term_scan"].items():
                unit.scan(category, row["status"], row["candidate_ids"], row["reason"])
            for name, row in reviewed["checks"].items():
                unit.check(name, row["status"], row["reason"])
        writer.save_draft(self.draft_path)
        writer = writer_tools.ReviewWriter.from_draft(self.draft_path, progress=None)
        writer.scope("passed", "The generated source has exactly the Example title and the four-sentence paragraph. Both actual inventory units have complete translation reviews; no source unit is excluded.")
        writer.source_issues_review("passed", "Both generated source blocks are legible and have one stated reading: the title label, tile/rim roles, expanded RO, 2 mm value, latch prerequisite and same-check antecedent contain no unresolved source conflict.")
        writer.terminology_review("reviewed", covered_units=[TITLE_ID, PARAGRAPH_ID])
        writer.review("reviewed")
        self.translation_report = writer.save(self.records["translation_review"], terminology_path=self.records["terminology"])
        if not self.translation_report.records_ready:
            raise ValueError(str(self.translation_report.to_dict()))

    def _review_workflow_checks(self, line_count: int) -> None:
        evidence = {
            "source": self.check_tools.bind_evidence(self.source, {"kind": "pdf_pages", "start": 1, "end": 1}),
            "candidate": self.check_tools.bind_evidence(self.candidate, {"kind": "lines", "start": 7, "end": line_count}),
        }
        contexts = {
            "translated-content": {"scope": [
                {"object_id": TITLE_ID, "locator": {"page": 1}},
                {"object_id": PARAGRAPH_ID, "locator": {"page": 1}},
            ], "evidence": ["source", "candidate"]},
            "local-links": {"scope": [{"object_id": "candidate-local-links", "locator": {"start": 7, "end": line_count}}], "evidence": ["candidate"]},
            "figures": {"scope": [{"object_id": "main:page1", "locator": {"page": 1}}], "evidence": ["source"]},
            "opening-event": {"scope": [{"object_id": "historical-open", "locator": {"source": "main", "page": 1}}], "evidence": ["source"]},
        }
        workflow.record_decision_rows(self.records["checks"], [
            ("FLOW-A01", "passed", "translated-content", "The title 示例 and the four Chinese sentences retain the source label, inward relation, 2 mm offset, conditional permission after latch closure, and negative rim movement during the same check."),
            ("FLOW-A02", "not_applicable", "local-links", "The candidate defines one paragraph anchor but has no local link calls.", "The generated candidate contains no local link call requiring resolution."),
            ("FLOW-A03", "not_applicable", "figures", "The complete generated page contains two native text blocks and no images, drawings or figure objects.", "This source has no figure requiring a delivered image asset."),
            ("FLOW-A04", "not_verified", "opening-event", "The bound PDF identifies the input only; this file-only test records no historical executor opening event."),
        ], contexts=contexts, evidence=evidence)

    def inspect(self):
        return workflow.inspect_workflow(self.run_path)

    def hashes(self) -> dict:
        return {str(path): digest(path) for path in self.base.rglob("*") if path.is_file()}


class CompleteTranslationWorkflowIntegrationTests(unittest.TestCase):
    artifact_root: Path | None = None

    def setUp(self):
        if self.artifact_root is None:
            temporary = tempfile.TemporaryDirectory()
            self.addCleanup(temporary.cleanup)
            base = Path(temporary.name)
        else:
            base = self.artifact_root.joinpath(self._testMethodName)
        self.fixture = SyntheticTranslatedWorkflow(base)

    def test_translation_required_workflow_reaches_final_gate_with_all_targets(self):
        f = self.fixture
        before = f.hashes()
        report = workflow.require_completed_records(f.run_path)
        self.assertTrue(report.records_ready)
        self.assertEqual(report.findings, [])
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertEqual(before, f.hashes())
        self.assertTrue(read(f.run_path)["scope"]["translation_required"])
        review = read(f.records["translation_review"])
        self.assertEqual(set(review["units"]), {TITLE_ID, PARAGRAPH_ID})
        self.assertEqual(review["scope"]["unselected_inventory_units"], [])
        self.assertEqual(review["scope"]["excluded_units"], [])
        self.assertEqual(review["events"], [])
        for unit in review["units"].values():
            self.assertEqual(len(unit["checks"]), 17)
            self.assertEqual(set(unit["checks"]), set(example.prepare.CHECKS))
            self.assertEqual(len(unit["term_scan"]), 10)
            self.assertEqual(set(unit["term_scan"]), set(example.prepare.TERM_CATEGORIES))
            self.assertEqual(len(unit["occurrences"]), 14)
            self.assertEqual(set(unit["occurrences"]), set(example.prepare.OCCURRENCE_KINDS))
        body = review["units"][PARAGRAPH_ID]
        self.assertEqual([len(body["occurrences"][name]) for name in ("claims", "strength", "modifiers", "conditions")], [4, 4, 6, 3])
        self.assertEqual(body["occurrences"]["numbers"][0]["candidate_span"]["quote"], FORMATTED_VALUE)
        self.assertEqual(body["occurrences"]["numbers"][0]["difference_type"], "representation_only")
        self.assertEqual(f.initial_bindings, {path: digest(Path(path)) for path in f.initial_bindings})
        self.assertEqual(review["bindings"]["candidate"], example.prepare.bind_review_file(f.candidate))
        self.assertEqual(review["bindings"]["source_inventory"], example.prepare.bind_review_file(f.inventory_path))
        self.assertEqual(review["bindings"]["sources"]["main"], example.prepare.bind_review_file(f.source))
        self.assertEqual(review["bindings"]["glossary"], example.prepare.bind_review_file(f.glossary))
        manifest = read(f.records["delivery_manifest"])
        self.assertEqual(manifest["translation_review"], example.prepare.bind_review_file(f.records["translation_review"]))
        self.assertIs(manifest["figure_edge_reviews_required"], True)

    def test_reducing_translation_targets_is_rejected_with_current_manifest_hashes(self):
        f = self.fixture
        original = read(f.records["translation_review"])
        cases = (
            ("checks", "alignment", "remove"),
            ("term_scan", "metric", "remove"),
            ("occurrences", "numbers", "remove"),
            ("occurrences", "negations", "empty"),
        )
        for section, name, mutation in cases:
            with self.subTest(section=section, name=name):
                altered = deepcopy(original)
                group = altered["units"][PARAGRAPH_ID][section]
                if mutation == "empty":
                    group[name] = []
                else:
                    del group[name]
                save(f.records["translation_review"], altered)
                # The final manifest binds the actual altered record, so this
                # test must fail on lost review targets, not on an old hash.
                workflow.write_manifest(f.run_path, f.deliveries)
                with self.assertRaises(workflow.IncompleteRecordsError) as raised:
                    workflow.require_completed_records(f.run_path)
                report = raised.exception.report
                self.assertFalse(report.records_ready)
                self.assertTrue(any(row.code == "WORKFLOW_TRANSLATION_UNRESOLVED" for row in report.findings))
                expected_code = "TRN_OCCURRENCES_MISSING" if mutation == "empty" else "TRN_TARGET_MISSING"
                expected_target = (f"units/{PARAGRAPH_ID}/checks/negation" if mutation == "empty"
                                   else f"units/{PARAGRAPH_ID}/{section}/{name}")
                self.assertTrue(any(row.code == expected_code and row.target == expected_target
                                    for row in report.findings), msg=report.as_dict())
        save(f.records["translation_review"], original)
        workflow.write_manifest(f.run_path, f.deliveries)

    def test_final_translation_review_cannot_be_replaced_by_the_pending_scaffold(self):
        f = self.fixture
        pending_before = digest(f.pending_path)
        pending = read(f.pending_path)
        self.assertEqual(pending["review_status"], "pending")
        self.assertTrue(all(row["status"] == "pending" for unit in pending["units"].values()
                            for row in unit["checks"].values()))
        f.records["translation_review"].write_bytes(f.pending_path.read_bytes())
        workflow.write_manifest(f.run_path, f.deliveries)
        with self.assertRaises(workflow.IncompleteRecordsError) as raised:
            workflow.require_completed_records(f.run_path)
        self.assertTrue(any(row.code == "WORKFLOW_TRANSLATION_UNRESOLVED" for row in raised.exception.report.findings))
        self.assertEqual(pending_before, digest(f.pending_path))


if __name__ == "__main__":
    unittest.main()
