"""Pending, completed and defective records against temporary synthetic sources."""

from __future__ import annotations

from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


review = load_module(
    "test_translation_review_helper",
    ROOT.joinpath("literature-translate-zh", "scripts", "prepare_translation_review.py"),
)
checker = load_module(
    "test_translation_review_check_records",
    ROOT.joinpath("literature-zotero2obsidian", "scripts", "check_records.py"),
)
translation_checker = load_module(
    "test_translation_review_validator",
    ROOT.joinpath("literature-translate-zh", "scripts", "validate_translation_review.py"),
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class TranslationReviewFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.source = self.base.joinpath("synthetic.pdf")
        with fitz.open() as document:
            page = document.new_page(width=360, height=220)
            page.insert_text((30, 45), "The radial offset was estimated by trimmed regression.")
            page.insert_text((30, 75), "The operator checked the violet shutter.")
            document.save(self.source)
        self.candidate = self.base.joinpath("translated.md")
        self.candidate.write_text("径向偏移量由截尾回归估计。\n操作员检查了紫色快门。\n", encoding="utf-8")
        self.glossary = self.base.joinpath("glossary.md")
        self.glossary.write_text("| synthetic-radial | radial shift | 径向偏移 |\n", encoding="utf-8")
        self.inventory = {
            "schema_version": 1,
            "sources": [{"id": "source-a", "path": self.source.name,
                         "sha256": digest(self.source), "page_count": 1}],
            "units": [
                {"id": "paragraph-a", "source": "source-a", "kind": "paragraph",
                 "evidence": [{"page": 1, "quote": "The radial offset was estimated by trimmed regression."}]},
                {"id": "paragraph-b", "source": "source-a", "kind": "paragraph",
                 "evidence": [{"page": 1, "bbox": [25, 60, 330, 90]}]},
            ],
        }
        self.inventory_path = self.base.joinpath("source-inventory.json")
        self.path = self.base.joinpath("translation-review.json")
        self.save_inventory()

    def save_inventory(self):
        self.inventory_path.write_text(json.dumps(self.inventory), encoding="utf-8")

    def prepare(self, **options):
        arguments = {"unit_ids": ["paragraph-a"], "glossary_path": self.glossary, "progress": None}
        arguments.update(options)
        return review.prepare_translation_review(
            self.inventory_path, self.candidate, self.path, **arguments)

    def read(self):
        return json.loads(self.path.read_text(encoding="utf-8"))

    def blocks_only_inventory(self):
        with fitz.open(self.source) as document:
            blocks = [{"id": f"source-a-p1-b{block[5]}", "bbox": list(block[:4]), "text": block[4]}
                      for block in document[0].get_text("blocks") if block[6] == 0]
        self.inventory["sources"][0]["pages"] = [{"page": 1, "blocks": blocks}]
        self.inventory["units"][0]["evidence"] = [{"page": 1, "basis": "text_blocks", "blocks": [blocks[0]["id"]]}]
        self.save_inventory()
        return blocks


class TranslationReviewTests(TranslationReviewFixture):
    def test_blocks_only_locations_expand_actual_inventory_metadata(self):
        blocks = self.blocks_only_inventory()
        before = self.inventory_path.read_bytes()
        self.prepare()
        location = self.read()["units"]["paragraph-a"]["source_locations"][0]
        self.assertEqual(location["blocks"], [blocks[0]["id"]])
        self.assertEqual(location["basis"], "text_blocks")
        self.assertEqual(location["page"], 1)
        self.assertEqual(location["bbox"], blocks[0]["bbox"])
        self.assertEqual(location["quote"], blocks[0]["text"])
        self.assertEqual(location["block_locations"][0]["block_id"], blocks[0]["id"])
        self.assertEqual(self.read()["bindings"]["sources"]["source-a"]["sha256"], digest(self.source))
        self.assertEqual(self.inventory_path.read_bytes(), before)
        self.assertEqual(self.read()["units"]["paragraph-a"]["checks"]["completeness"]["status"], "pending")

    def test_blocks_only_rejects_unknown_duplicate_or_wrong_page_ids(self):
        blocks = self.blocks_only_inventory()
        for bad in (
            {"page": 1, "blocks": ["unknown"]},
            {"page": 2, "blocks": [blocks[0]["id"]]},
            {"page": 1, "blocks": [blocks[0]["id"], blocks[0]["id"]]},
            {"page": 1, "blocks": []},
        ):
            self.inventory["units"][0]["evidence"] = [bad]
            self.save_inventory()
            with self.subTest(location=bad), self.assertRaises(ValueError):
                self.prepare()
            self.assertFalse(self.path.exists())

    def test_blocks_only_keeps_order_and_exact_subspans_without_inferring_paragraphs(self):
        blocks = self.blocks_only_inventory()
        self.inventory["units"][0]["evidence"][0]["blocks"] = [blocks[1]["id"], blocks[0]["id"]]
        source = self.inventory["sources"][0]
        expanded = review.expand_source_locations(self.inventory["units"][0], source)
        self.assertEqual([row["block_id"] for row in expanded[0]["block_locations"]], [blocks[1]["id"], blocks[0]["id"]])
        self.assertEqual(expanded[0]["quote"], blocks[1]["text"] + "\n" + blocks[0]["text"])
        self.assertNotIn("status", expanded[0])

    def test_direct_visual_locator_does_not_require_text_blocks(self):
        location = {"page": 1, "basis": "visual", "bbox": [25, 30, 330, 60], "blocks": []}
        self.inventory["units"][0]["evidence"] = [location]
        self.save_inventory()
        self.prepare()
        self.assertEqual(self.read()["units"]["paragraph-a"]["source_locations"], [location])
    def test_scaffold_preserves_pending_scope_without_inventing_results(self):
        self.prepare()
        ledger = self.read()
        unit = ledger["units"]["paragraph-a"]
        self.assertEqual(ledger["review_status"], "pending")
        self.assertEqual(ledger["scope"]["selection_review"]["status"], "pending")
        self.assertEqual(ledger["scope"]["unselected_inventory_units"], ["paragraph-b"])
        self.assertEqual(ledger["scope"]["excluded_units"], [])
        self.assertEqual({value["status"] for value in unit["checks"].values()}, {"pending"})
        self.assertEqual({value["status"] for value in unit["term_scan"].values()}, {"pending"})
        self.assertTrue(all(value == [] for value in unit["occurrences"].values()))
        self.assertEqual(unit["candidate_locations"], [])
        self.assertEqual(ledger["events"], [])

    def test_bindings_use_real_bytes_and_inventory_relative_source_path(self):
        inventory_before = self.inventory_path.read_bytes()
        self.prepare()
        bindings = self.read()["bindings"]
        self.assertEqual(bindings["sources"]["source-a"],
                         {"path": str(self.source.resolve()), "sha256": digest(self.source)})
        self.assertEqual(bindings["candidate"]["sha256"], digest(self.candidate))
        self.assertEqual(bindings["glossary"]["sha256"], digest(self.glossary))
        self.assertEqual(bindings["source_inventory"]["sha256"], digest(self.inventory_path))
        self.assertEqual(self.inventory_path.read_bytes(), inventory_before)

    def test_existing_record_is_never_overwritten(self):
        self.prepare()
        original = self.path.read_bytes()
        with self.assertRaises(FileExistsError):
            self.prepare()
        self.assertEqual(self.path.read_bytes(), original)

    def test_changed_source_bytes_are_rejected_before_record_creation(self):
        self.source.write_bytes(self.source.read_bytes() + b"\n% changed source\n")
        with self.assertRaisesRegex(ValueError, "source bytes differ"):
            self.prepare()
        self.assertFalse(self.path.exists())

    def test_unit_selection_must_be_explicit_unique_and_known(self):
        for selected in ([], "paragraph-a", ["paragraph-a", "paragraph-a"], ["missing"]):
            with self.subTest(selected=selected), self.assertRaises(ValueError):
                self.prepare(unit_ids=selected)
        self.assertFalse(self.path.exists())

    def test_source_locations_require_a_page_and_object_locator(self):
        invalid = [
            {"page": 0, "quote": "The operator checked the violet shutter."},
            {"page": True, "quote": "The operator checked the violet shutter."},
            {"page": 1},
            {"page": 1, "quote": " "},
            {"page": 1, "bbox": [30, 40, 20, 60]},
        ]
        for location in invalid:
            self.inventory["units"][0]["evidence"] = [location]
            self.save_inventory()
            with self.subTest(location=location), self.assertRaises(ValueError):
                self.prepare()
            self.assertFalse(self.path.exists())

    def test_candidate_locations_cannot_escape_selected_scope(self):
        with self.assertRaises(ValueError):
            self.prepare(candidate_locations={"paragraph-b": [{"lines": [2, 2]}]})
        self.assertFalse(self.path.exists())

    def test_locator_copy_and_progress_follow_selected_units(self):
        targets = {"paragraph-b": [{"lines": [2, 2]}]}
        progress = []
        self.prepare(unit_ids=["paragraph-b"], candidate_locations=targets,
                     progress=lambda done, total, label: progress.append((done, total, label)))
        self.assertEqual(self.read()["units"]["paragraph-b"]["candidate_locations"], targets["paragraph-b"])
        self.assertEqual(progress, [(1, 1, "paragraph-b")])

    def test_missing_glossary_does_not_imply_terminology_review(self):
        self.prepare(glossary_path=None)
        ledger = self.read()
        self.assertIsNone(ledger["bindings"]["glossary"])
        self.assertEqual(ledger["units"]["paragraph-a"]["checks"]["terminology"]["status"], "pending")

    def test_term_candidate_is_a_pending_copy_without_automatic_search(self):
        location = {"page": 1, "quote": "radial offset"}
        term = review.new_term_candidate(
            "candidate-a", "radial offset", "parameter",
            source_excerpt="The radial offset was estimated by trimmed regression.",
            source_locator=location)
        location["page"] = 99
        self.assertEqual(term["source_locator"]["page"], 1)
        self.assertEqual(term["decision"], "pending")
        self.assertIsNone(term["translation"])
        self.assertEqual(set(term["searches"]), {"full_phrase", "aliases", "family"})
        self.assertTrue(all(row["status"] == "pending" and row["queries"] == []
                            for row in term["searches"].values()))
        with self.assertRaises(ValueError):
            review.new_term_candidate("candidate-b", "violet", "unknown",
                                      source_excerpt="violet shutter", source_locator={"page": 1})

    def test_documented_json_pointer_binding_passes_real_record_validation(self):
        self.prepare()
        skills = self.base.joinpath("skills")
        skill = skills.joinpath("sample-review")
        skill.mkdir(parents=True)
        skill.joinpath("SKILL.md").write_text(
            "# Sample review\n- [ ] [SAMPLE-A01] Record the pending paragraph comparison. "
            "<!-- check-type: offline_outcome -->\n", encoding="utf-8")
        checks_path = self.base.joinpath("checks.json")
        checker.prepare_check_records(
            skills, checks_path, sources={"source-a": self.source},
            candidate_path=self.candidate,
            scopes={"SAMPLE-A01": [{"object_id": "paragraph-a", "locator": "source page 1 / translated line 1"}]},
            progress=None)
        checks = json.loads(checks_path.read_text(encoding="utf-8"))
        binding = checker.bind_evidence(
            self.path, {"kind": "json_pointer", "pointer": "/units/paragraph-a/checks/completeness"})
        checks["evidence"]["paragraph-completeness"] = binding
        checks["checks"][0]["evidence"] = ["paragraph-completeness"]
        checks_path.write_text(json.dumps(checks), encoding="utf-8")
        report = checker.validate_check_records(checks_path)
        self.assertEqual(report.findings, [])
        self.assertFalse(report.recorded_checks_complete)
        self.assertEqual(report.source_fidelity, "not_assessed")
        invalid = deepcopy(checks)
        invalid["evidence"]["paragraph-completeness"]["locator"] = {
            "type": "json_pointer", "pointer": "/units/paragraph-a/checks/completeness"}
        checks_path.write_text(json.dumps(invalid), encoding="utf-8")
        bad_report = checker.validate_check_records(checks_path)
        self.assertIn("CHECK_EVIDENCE_LOCATION_INVALID", {finding.code for finding in bad_report.findings})


class TranslationRecordValidationTests(TranslationReviewFixture):
    """Concrete synthetic decisions, never an automatic production approval helper."""

    def setUp(self):
        super().setUp()
        self.glossary.write_text(
            "| sample-offset | radial offset | 径向偏移量 |\n"
            "| sample-regression | regression | 回归 |\n", encoding="utf-8")
        self.terminology_path = self.base.joinpath("terminology.json")
        self.source_a = "The radial offset was estimated by trimmed regression."
        self.source_b = "The operator checked the violet shutter."

    def save_ledger(self, ledger):
        self.path.write_text(json.dumps(ledger, ensure_ascii=False), encoding="utf-8")

    def validate(self, ledger=None, **options):
        if ledger is not None:
            self.save_ledger(ledger)
        return translation_checker.validate_translation_review(self.path, **options)

    def codes(self, report):
        return {finding.code for finding in report.findings}

    def result(self, status, reason, evidence):
        return {"status": status, "reason": reason, "evidence": deepcopy(evidence)}

    def pair_evidence(self, quote, line):
        return [review.bind_review_evidence(self.source, {"page": 1, "quote": quote}),
                review.bind_review_evidence(self.candidate, {"lines": [line, line]})]

    def completed(self):
        self.prepare(unit_ids=["paragraph-a", "paragraph-b"], candidate_locations={
            "paragraph-a": [{"lines": [1, 1]}], "paragraph-b": [{"lines": [2, 2]}],
        })
        ledger = self.read()
        ledger["review_status"] = "reviewed"
        scope_evidence = [review.bind_review_evidence(self.inventory_path, {
            "kind": "json_pointer", "pointer": "/units/0"}),
            review.bind_review_evidence(self.inventory_path, {"kind": "json_pointer", "pointer": "/units/1"})]
        ledger["scope"]["selection_review"] = self.result(
            "passed", "The synthetic assignment covers both independent source paragraphs.", scope_evidence)
        ledger["source_issues_review"] = self.result(
            "passed", "Both synthetic source sentences are legible; no conflicting readings were found in these two sentences.",
            self.pair_evidence(self.source_a, 1) + self.pair_evidence(self.source_b, 2))
        candidate_lines = self.candidate.read_text(encoding="utf-8").splitlines()
        for unit_id, line, quote, clause in (
            ("paragraph-a", 1, self.source_a, candidate_lines[0]),
            ("paragraph-b", 2, self.source_b, candidate_lines[1]),
        ):
            unit = ledger["units"][unit_id]
            evidence = self.pair_evidence(quote, line)
            # Every target is deliberately stated below for this synthetic pair.
            # This fixture is not exported as a way to fill real review outcomes.
            unit["checks"] = {
                "alignment": self.result("passed", f"The complete sentence maps to candidate line {line}.", evidence),
                "ambiguity": self.result("not_applicable", "This sentence names its actor and action directly; there is no competing referent or ambiguous attachment in this fixture.", evidence),
                "negation": self.result("not_applicable", "This synthetic sentence has no negative predicate or quantifier.", evidence),
                "strength": self.result("passed", "The stated action remains an action, with no new causal or certainty claim.", evidence),
                "number_changes": self.result("not_applicable", "This sentence contains no numeric expression.", evidence),
                "terminology": self.result("passed", "The ten categories were separately recorded for this sentence.", evidence),
                "units": self.result("not_applicable", "This sentence contains no measurement-unit expression.", evidence),
                "symbols": self.result("not_applicable", "This sentence contains no defined mathematical symbol.", evidence),
                "formulas": self.result("not_applicable", "This sentence contains no inline or displayed formula.", evidence),
                "citation_labels": self.result("not_applicable", "This sentence has no displayed citation label.", evidence),
                "completeness": self.result("passed", "The explicit sentence-level claim is preserved in its mapped candidate sentence.", evidence),
                "directions": self.result("not_applicable", "There is no subject-to-reference spatial relation in this sentence; a parameter name alone is not such a relation.", evidence),
                "citation_claims": self.result("not_applicable", "No reference is invoked to support this sentence.", evidence),
                "table_text": self.result("not_applicable", "This source unit is prose, not a table cell.", evidence),
                "conditions": self.result("not_applicable", "This sentence does not restrict its claim by an only/unless or spatial applicability clause.", evidence),
                "antecedents": self.result("not_applicable", "No pronoun or other anaphoric expression occurs in this sentence.", evidence),
                "modifier_scope": self.result("not_applicable", "No competing modifier attachment occurs in this synthetic sentence.", evidence),
            }
            claim = review.new_occurrence("claim-1", "claims", source_span={"page": 1, "quote": quote})
            claim.update(self.result("passed", "The source actor/action/object remain in the candidate sentence.", evidence))
            claim.update({"candidate_span": {"lines": [line, line]}, "source_claim": quote, "candidate_claim": clause})
            unit["occurrences"]["claims"] = [claim]
            strength = review.new_occurrence("strength-1", "strength", source_span={"page": 1, "quote": quote})
            strength.update(self.result("passed", "The fixture reports the same stated operation on each side.", evidence))
            strength.update({"candidate_span": {"lines": [line, line]},
                             "source_strength": "stated operation", "candidate_strength": "陈述的操作"})
            unit["occurrences"]["strength"] = [strength]
            unit["term_scan"] = {
                category: {**self.result("not_applicable", f"The complete sentence contains no term of category {category} beyond the separately registered candidates.", evidence),
                           "candidate_ids": []}
                for category in review.TERM_CATEGORIES
            }
        source_evidence = self.pair_evidence(self.source_a, 1)
        glossary_evidence = [review.bind_review_evidence(self.glossary, {"lines": [1, 2]})]
        offset_hit = {"entry_id": "sample-offset", "glossary_locator": {"lines": [1, 1]},
                      "excerpt": "radial offset | 径向偏移量"}
        regression_hit = {"entry_id": "sample-regression", "glossary_locator": {"lines": [2, 2]},
                          "excerpt": "regression | 回归"}
        offset = review.new_term_candidate("offset", "radial offset", "parameter",
            source_excerpt=self.source_a, source_locator={"page": 1, "quote": "radial offset"})
        regression = review.new_term_candidate("trimmed", "trimmed regression", "method",
            source_excerpt=self.source_a, source_locator={"page": 1, "quote": "trimmed regression"})
        for term, full_hits, family_query, family_hit in (
            (offset, [offset_hit], "radial", offset_hit),
            (regression, [], "regression", regression_hit),
        ):
            term["searches"] = {
                "full_phrase": {**self.result("searched", "The complete source phrase was looked up in the two-row synthetic glossary.", glossary_evidence),
                                "queries": [{"query": term["original"], "hits": full_hits,
                                             "interpretation": "The full phrase is present." if full_hits else "The full phrase has no entry."}]},
                "aliases": {**self.result("not_applicable", "The synthetic source states no alias or abbreviation for this phrase.", source_evidence), "queries": []},
                "family": {**self.result("searched", "The core word was separately searched and its entry read.", glossary_evidence),
                           "queries": [{"query": family_query, "hits": [family_hit],
                                        "interpretation": "The core word supplies related lexical context; the full-phrase decision remains separate."}]},
            }
        offset.update({"decision": "adopted", "decision_reason": "The synthetic glossary has the same offset parameter phrase.",
                       "translation": "径向偏移量", "glossary_matches": [offset_hit], "terminology_pointer": "/adopted/0"})
        regression.update({"decision": "unlisted", "decision_reason": "Only regression is listed; the full method phrase remains an unlisted term.",
                           "translation": "截尾回归", "glossary_matches": [regression_hit], "terminology_pointer": "/unlisted/0"})
        unit = ledger["units"]["paragraph-a"]
        unit["term_candidates"] = [offset, regression]
        for category, ids in (("parameter", ["offset"]), ("concept", ["offset"]), ("method", ["trimmed"])):
            unit["term_scan"][category] = {**self.result("scanned", f"The stated {category} candidate is explicitly registered for this source sentence.", source_evidence), "candidate_ids": ids}
        terminology = {
            "schema_version": 1, "review_status": "reviewed", "glossary": deepcopy(ledger["bindings"]["glossary"]),
            "covered_units": ["paragraph-a", "paragraph-b"],
            "adopted": [{"original": "radial offset", "translation": "径向偏移量", "context": self.source_a,
                         "source_units": ["paragraph-a"], "glossary_locator": "sample-offset, line 1"}],
            "unlisted": [{"original": "trimmed regression", "translation": "截尾回归", "context": self.source_a,
                          "source_units": ["paragraph-a"], "reason": "No complete-phrase glossary entry.", "status": "resolved",
                          "resolution_basis": "For this synthetic fixture, the method phrase modifies regression by trimming; the core glossary entry is retained as family context."}],
            "no_unlisted_reason": None,
        }
        self.terminology_path.write_text(json.dumps(terminology, ensure_ascii=False), encoding="utf-8")
        ledger["terminology_record"] = {"path": str(self.terminology_path), "sha256": digest(self.terminology_path)}
        self.save_ledger(ledger)
        return ledger

    def test_complete_synthetic_record_is_ready_without_semantic_certification(self):
        ledger = self.completed()
        before = self.path.read_bytes()
        report = self.validate(inventory_path=self.inventory_path, candidate_path=self.candidate)
        self.assertEqual(report.findings, [])
        self.assertTrue(report.records_ready)
        self.assertTrue(report.to_dict()["records_ready"])
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertEqual(report.execution_truth, "not_assessed")
        self.assertEqual(report.selected_unit_count, 2)
        self.assertEqual(self.path.read_bytes(), before)
        self.assertEqual(ledger["events"], [])  # Offline records do not require invented execution traces.

    def test_complete_blocks_only_inventory_round_trips_through_real_validator(self):
        self.blocks_only_inventory()
        ledger = self.completed()
        report = self.validate(ledger)
        self.assertEqual(report.findings, [])
        self.assertTrue(report.records_ready)
        ledger["units"]["paragraph-a"]["source_locations"][0]["block_locations"][0]["block_id"] = "invented"
        self.assertIn("TRN_SOURCE_LOCATIONS_CHANGED", self.codes(self.validate(ledger)))

    def test_pending_scaffold_is_not_ready_and_is_never_modified(self):
        self.prepare()
        before = self.path.read_bytes()
        report = self.validate()
        self.assertIn("TRN_PENDING", self.codes(report))
        self.assertIn("TRN_SCOPE_UNACCOUNTED", self.codes(report))
        self.assertIn("TRN_CANDIDATE_LOCATION_MISSING", self.codes(report))
        self.assertFalse(report.records_ready)
        self.assertEqual(self.path.read_bytes(), before)

    def test_missing_and_malformed_whole_review_return_diagnostics(self):
        report = self.validate()
        self.assertIn("TRN_JSON_UNREADABLE", self.codes(report))
        for content in (b"{", b"\xff", b"[]", b'{"units":{},"units":{}}', b'{"v":NaN}', b"null"):
            with self.subTest(content=content):
                self.path.write_bytes(content)
                report = self.validate()
                self.assertFalse(report.records_ready)
                self.assertTrue(report.findings)
                self.assertEqual(report.source_fidelity, "not_assessed")

    def test_selected_unit_missing_from_records_is_reported(self):
        ledger = self.completed()
        del ledger["units"]["paragraph-b"]
        report = self.validate(ledger)
        self.assertIn("TRN_UNIT_MISSING", self.codes(report))
        self.assertFalse(report.records_ready)

    def test_unselected_text_cannot_silently_escape_coverage(self):
        ledger = self.completed()
        ledger["scope"]["selected_units"] = ["paragraph-a"]
        ledger["scope"]["unselected_inventory_units"] = ["paragraph-b"]
        del ledger["units"]["paragraph-b"]
        report = self.validate(ledger)
        self.assertIn("TRN_SCOPE_UNACCOUNTED", self.codes(report))
        self.assertFalse(report.records_ready)
        ledger["scope"]["unselected_inventory_units"] = []
        self.assertIn("TRN_UNSELECTED_INVENTORY_MISMATCH", self.codes(self.validate(ledger)))

    def test_explicit_partial_scope_needs_condition_and_actual_evidence(self):
        ledger = self.completed()
        authorization = self.base.joinpath("synthetic-task.txt")
        authorization.write_text("Revise only paragraph-a; retain paragraph-b outside this revision.\n", encoding="utf-8")
        ledger["scope"]["selected_units"] = ["paragraph-a"]
        ledger["scope"]["unselected_inventory_units"] = ["paragraph-b"]
        del ledger["units"]["paragraph-b"]
        ledger["scope"]["excluded_units"] = [{"id": "paragraph-b", "applicability_condition": "The synthetic revision request names paragraph-a only.",
            "reason": "The other paragraph is explicitly outside this local revision.",
            "evidence": [review.bind_review_evidence(authorization, {"lines": [1, 1]})]}]
        self.assertTrue(self.validate(ledger).records_ready)
        ledger["scope"]["excluded_units"][0]["applicability_condition"] = ""
        self.assertIn("TRN_EXCLUSION_REASON", self.codes(self.validate(ledger)))

    def test_missing_individual_check_scan_or_occurrence_group_is_not_ignored(self):
        ledger = self.completed()
        for section, key in (("checks", "negation"), ("term_scan", "metric"), ("occurrences", "formulas")):
            changed = deepcopy(ledger)
            del changed["units"]["paragraph-a"][section][key]
            with self.subTest(section=section, key=key):
                self.assertIn("TRN_TARGET_MISSING", self.codes(self.validate(changed)))

    def test_blanket_pass_with_empty_occurrences_is_rejected(self):
        ledger = self.completed()
        ledger["units"]["paragraph-a"]["checks"]["negation"]["status"] = "passed"
        self.assertIn("TRN_OCCURRENCES_MISSING", self.codes(self.validate(ledger)))

    def test_any_stale_bound_file_is_reported(self):
        ledger = self.completed()
        for key in ("source_inventory", "candidate", "glossary"):
            changed = deepcopy(ledger)
            changed["bindings"][key]["sha256"] = "0" * 64
            with self.subTest(binding=key):
                report = self.validate(changed)
                self.assertIn("TRN_HASH_CHANGED", self.codes(report))
                self.assertFalse(report.records_ready)
        ledger["bindings"]["sources"]["source-a"]["sha256"] = "0" * 64
        self.assertIn("TRN_HASH_CHANGED", self.codes(self.validate(ledger)))

    def test_actual_changed_bytes_are_not_rebound_by_validator(self):
        self.completed()
        self.candidate.write_text(self.candidate.read_text(encoding="utf-8") + "changed\n", encoding="utf-8")
        self.assertIn("TRN_HASH_CHANGED", self.codes(self.validate()))

    def test_override_pins_identity_even_when_copies_have_same_bytes(self):
        self.completed()
        other = self.base.joinpath("other-candidate.md")
        other.write_bytes(self.candidate.read_bytes())
        report = self.validate(candidate_path=other)
        self.assertIn("TRN_BINDING_IDENTITY", self.codes(report))
        self.assertFalse(report.records_ready)

    def test_bad_candidate_source_and_evidence_locations_are_diagnosed(self):
        ledger = self.completed()
        for location in ({"lines": [99, 99]}, {"lines": [True, 1]}, {"quote": "missing"}, {"lines": [1, 1], "quote": "wrong excerpt"}, {}):
            changed = deepcopy(ledger)
            changed["units"]["paragraph-a"]["candidate_locations"] = [location]
            with self.subTest(location=location):
                report = self.validate(changed)
                self.assertFalse(report.records_ready)
                self.assertTrue(self.codes(report) & {"TRN_LOCATION_INVALID", "TRN_LOCATION_MISSING"})
        changed = deepcopy(ledger)
        changed["units"]["paragraph-a"]["source_locations"] = [{"page": 2, "quote": self.source_a}]
        codes = self.codes(self.validate(changed))
        self.assertIn("TRN_SOURCE_LOCATIONS_CHANGED", codes)
        self.assertIn("TRN_LOCATION_INVALID", codes)
        changed = deepcopy(ledger)
        changed["scope"]["selection_review"]["evidence"][0]["locator"]["pointer"] = "/units/99"
        self.assertIn("TRN_LOCATION_INVALID", self.codes(self.validate(changed)))

    def test_repeated_quote_needs_a_narrower_location(self):
        ledger = self.completed()
        # A punctuation mark occurs on both candidate lines.
        ledger["units"]["paragraph-a"]["candidate_locations"] = [{"quote": "。"}]
        self.assertIn("TRN_LOCATION_INVALID", self.codes(self.validate(ledger)))

    def test_occurrence_cannot_point_to_a_different_valid_paragraph(self):
        ledger = self.completed()
        ledger["units"]["paragraph-a"]["occurrences"]["claims"][0]["candidate_span"] = {"lines": [2, 2]}
        self.assertIn("TRN_CANDIDATE_SPAN_OUTSIDE_UNIT", self.codes(self.validate(ledger)))

    def test_family_search_cannot_be_pending_missing_empty_or_inapplicable(self):
        ledger = self.completed()
        for change in ("pending", "missing", "empty", "not_applicable"):
            changed = deepcopy(ledger)
            searches = changed["units"]["paragraph-a"]["term_candidates"][0]["searches"]
            if change == "missing":
                del searches["family"]
            elif change == "empty":
                searches["family"]["queries"] = []
            else:
                searches["family"]["status"] = change
            with self.subTest(change=change):
                report = self.validate(changed)
                self.assertFalse(report.records_ready)
                self.assertTrue(self.codes(report) & {"TRN_PENDING", "TRN_TARGET_MISSING", "TRN_TERM_QUERIES_MISSING", "TRN_TERM_SEARCH_REQUIRED"})

    def test_aliases_require_searches_when_the_candidate_lists_them(self):
        ledger = self.completed()
        ledger["units"]["paragraph-a"]["term_candidates"][0]["aliases"] = ["RO"]
        self.assertIn("TRN_TERM_SEARCH_REQUIRED", self.codes(self.validate(ledger)))

    def test_term_search_hits_must_locate_actual_glossary_text(self):
        ledger = self.completed()
        hit = ledger["units"]["paragraph-a"]["term_candidates"][0]["searches"]["family"]["queries"][0]["hits"][0]
        hit["excerpt"] = "invented entry"
        self.assertIn("TRN_LOCATION_INVALID", self.codes(self.validate(ledger)))

    def test_term_pointer_and_scan_reference_closure_are_checked(self):
        ledger = self.completed()
        changes = (
            ("bad-pointer", lambda u: u["term_candidates"][0].update(terminology_pointer="/adopted/99"), "TRN_TERMINOLOGY_POINTER"),
            ("wrong-term", lambda u: u["term_candidates"][0].update(terminology_pointer="/unlisted/0"), "TRN_TERM_MAPPING"),
            ("unknown-candidate", lambda u: u["term_scan"]["method"].update(candidate_ids=["unknown"]), "TRN_TERM_CANDIDATE_UNKNOWN"),
            ("missing-primary", lambda u: u["term_scan"]["parameter"].update(candidate_ids=[]), "TRN_TERM_CATEGORY_MAPPING"),
        )
        for label, change, code in changes:
            changed = deepcopy(ledger)
            change(changed["units"]["paragraph-a"])
            with self.subTest(change=label):
                self.assertIn(code, self.codes(self.validate(changed)))

    def test_terminology_cannot_claim_terms_omitted_from_the_per_unit_scan(self):
        ledger = self.completed()
        del ledger["units"]["paragraph-a"]["term_candidates"][1]
        ledger["units"]["paragraph-a"]["term_scan"]["method"] = {
            **self.result("not_applicable", "Incorrect synthetic omission of the method candidate.", self.pair_evidence(self.source_a, 1)),
            "candidate_ids": []}
        self.assertIn("TRN_TERMINOLOGY_CANDIDATE_MISSING", self.codes(self.validate(ledger)))

    def test_source_occurrence_must_belong_to_its_inventory_span(self):
        ledger = self.completed()
        ledger["units"]["paragraph-a"]["term_candidates"][0]["source_locator"] = {"page": 1, "quote": self.source_b}
        self.assertIn("TRN_SOURCE_SPAN_OUTSIDE_UNIT", self.codes(self.validate(ledger)))

    def test_record_shape_checks_do_not_guess_translation_meaning(self):
        ledger = self.completed()
        # Deliberately wrong color remains a human semantic-review issue; even a
        # fully re-bound and explicitly claimed record cannot certify the color.
        self.candidate.write_text(self.candidate.read_text(encoding="utf-8").replace("紫色", "红色"), encoding="utf-8")
        ledger["bindings"]["candidate"]["sha256"] = digest(self.candidate)
        ledger["units"]["paragraph-b"]["occurrences"]["claims"][0]["candidate_claim"] = "操作员检查了红色快门。"

        candidate_identity = self.candidate.resolve()
        candidate_sha256 = digest(self.candidate)

        def rebind_claimed_evidence(value):
            if isinstance(value, dict):
                evidence_path = value.get("path")
                if isinstance(evidence_path, str) and Path(evidence_path).resolve() == candidate_identity:
                    value["sha256"] = candidate_sha256
                for child in value.values():
                    rebind_claimed_evidence(child)
            elif isinstance(value, list):
                for child in value:
                    rebind_claimed_evidence(child)

        rebind_claimed_evidence(ledger)
        report = self.validate(ledger)
        self.assertTrue(report.records_ready, report.to_dict())
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertTrue(report.independent_review_required)

    def test_reason_evidence_hash_and_location_are_independent_requirements(self):
        ledger = self.completed()
        for missing, code in (("reason", "TRN_REASON_MISSING"), ("evidence", "TRN_LIST_REQUIRED")):
            changed = deepcopy(ledger)
            del changed["units"]["paragraph-a"]["checks"]["completeness"][missing]
            self.assertIn(code, self.codes(self.validate(changed)))
        changed = deepcopy(ledger)
        del changed["units"]["paragraph-a"]["checks"]["completeness"]["evidence"][0]["sha256"]
        self.assertIn("TRN_HASH_MISSING", self.codes(self.validate(changed)))

    def test_numeric_unit_symbol_formula_records_are_complete_individual_examples(self):
        self.source_a = "The radial offset was estimated by trimmed regression: r = (a + b)/(a + b), 4 mm."
        with fitz.open() as document:
            page = document.new_page(width=600, height=220)
            page.insert_text((30, 45), self.source_a)
            page.insert_text((30, 75), self.source_b)
            document.save(self.source)
        self.candidate.write_text("径向偏移量由截尾回归估计：r = (a + b)/(a + b)，4 mm。\n操作员检查了紫色快门。\n", encoding="utf-8")
        self.inventory["sources"][0]["sha256"] = digest(self.source)
        self.inventory["units"][0]["evidence"] = [{"page": 1, "quote": self.source_a}]
        self.save_inventory()
        ledger = self.completed()
        unit = ledger["units"]["paragraph-a"]
        evidence = self.pair_evidence(self.source_a, 1)
        examples = {
            "numbers": {"source_expression": "4", "candidate_expression": "4", "difference_type": "unchanged"},
            "units": {"source_expression": "mm", "candidate_expression": "mm", "source_quantity": "radial offset", "candidate_quantity": "径向偏移量"},
            "symbols": {"source_expression": "r", "candidate_expression": "r", "source_definition": "radial offset", "candidate_definition": "径向偏移量"},
            "formulas": {"source_expression": "r = (a + b)/(a + b)", "candidate_expression": "r = (a + b)/(a + b)",
                         "source_structure": {"denominators": ["a + b"], "radicals": [], "exponents": [], "brackets": ["a + b", "a + b"]},
                         "candidate_structure": {"denominators": ["a + b"], "radicals": [], "exponents": [], "brackets": ["a + b", "a + b"]}},
        }
        for kind, fields in examples.items():
            occurrence = review.new_occurrence(kind + "-one", kind, source_span={"page": 1, "quote": fields["source_expression"]})
            occurrence.update(fields)
            occurrence.update(self.result("passed", "The explicit synthetic expression is recorded on both sides.", evidence))
            occurrence["candidate_span"] = {"lines": [1, 1], "quote": fields["candidate_expression"]}
            unit["occurrences"][kind] = [occurrence]
            check = "number_changes" if kind == "numbers" else kind
            unit["checks"][check] = self.result("passed", f"The single {kind} occurrence was compared explicitly.", evidence)
        report = self.validate(ledger)
        self.assertEqual(report.findings, [])
        self.assertTrue(report.records_ready)
        del unit["occurrences"]["formulas"][0]["candidate_structure"]["denominators"]
        self.assertIn("TRN_TARGET_MISSING", self.codes(self.validate(ledger)))

    def test_complete_recorded_failure_is_visible_and_not_ready(self):
        ledger = self.completed()
        ledger["units"]["paragraph-a"]["checks"]["completeness"]["status"] = "failed"
        report = self.validate(ledger)
        self.assertIn("TRN_RECORDED_FAILURE", self.codes(report))
        self.assertFalse(report.records_ready)
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_unresolved_terms_need_a_separate_registry_entry(self):
        ledger = self.completed()
        term = ledger["units"]["paragraph-a"]["term_candidates"][1]
        term["decision"] = "unresolved"
        terminology = json.loads(self.terminology_path.read_text(encoding="utf-8"))
        terminology["unlisted"][0]["status"] = "unresolved"
        self.terminology_path.write_text(json.dumps(terminology), encoding="utf-8")
        ledger["terminology_record"]["sha256"] = digest(self.terminology_path)
        report = self.validate(ledger)
        self.assertIn("TRN_UNRESOLVED_TERM_MISSING", self.codes(report))
        ledger["unresolved_terms"] = [{"unit_id": "paragraph-a", "candidate_id": "trimmed", "reason": "Synthetic unresolved method definition."}]
        report = self.validate(ledger)
        self.assertNotIn("TRN_UNRESOLVED_TERM_MISSING", self.codes(report))
        self.assertFalse(report.records_ready)

    def test_malformed_nested_shapes_produce_diagnostics_instead_of_exceptions(self):
        ledger = self.completed()
        targets = [
            ("bindings",), ("scope",), ("units",), ("source_issues",), ("events",),
            ("scope", "selected_units"), ("scope", "excluded_units"),
            ("units", "paragraph-a"), ("units", "paragraph-a", "checks"),
            ("units", "paragraph-a", "source_locations"),
            ("units", "paragraph-a", "candidate_locations"),
            ("units", "paragraph-a", "term_scan"),
            ("units", "paragraph-a", "term_candidates"),
            ("units", "paragraph-a", "occurrences"),
        ]
        for target in targets:
            for malformed in (None, 3, "bad", []):
                if malformed == [] and target in (("source_issues",), ("events",), ("scope", "excluded_units")):
                    continue  # These are legitimate empty sets in the complete fixture.
                changed = deepcopy(ledger)
                parent = changed
                for key in target[:-1]:
                    parent = parent[key]
                parent[target[-1]] = malformed
                with self.subTest(target=target, malformed=malformed):
                    report = self.validate(changed)
                    self.assertFalse(report.records_ready)
                    self.assertTrue(report.findings)

    def test_single_occurrence_helper_preserves_pending_and_copies_inputs(self):
        source_span = {"page": 1, "quote": "a/(b+c)"}
        occurrence = review.new_occurrence("formula-a", "formulas", source_span=source_span)
        source_span["page"] = 20
        self.assertEqual(occurrence["source_span"]["page"], 1)
        self.assertEqual(occurrence["status"], "pending")
        self.assertIsNone(occurrence["candidate_span"])
        self.assertIsNone(occurrence["source_structure"])
        self.assertEqual(occurrence["evidence"], [])

    def test_script_example_records_only_the_explicit_fact_and_shares_its_evidence(self):
        example = load_module("test_translation_fill_example", ROOT.joinpath(
            "literature-translate-zh", "references", "fill_review_example.py"))
        result = example.demonstrate(self.base.joinpath("worked-example"))
        report = result["report"]
        self.assertFalse(report["records_ready"])
        self.assertEqual(report["source_fidelity"], "not_assessed")
        ledger_path = Path(result["review_path"])
        ledger = json.loads(ledger_path.read_text(encoding="utf-8"))
        checks = ledger["units"]["fictional-paragraph"]["checks"]
        self.assertEqual(checks["directions"]["status"], "passed")
        self.assertTrue(all(row["status"] == "pending" for key, row in checks.items() if key != "directions"))
        links = json.loads(ledger_path.parent.joinpath("example-atomic-links.json").read_text(encoding="utf-8"))
        self.assertEqual([row["id"] for row in links["checks"]], ["TRN-C68", "TRN-A14"])
        self.assertEqual(links["checks"][0]["evidence"], links["checks"][1]["evidence"])
        self.assertEqual(links["evidence"]["direction-fact"]["sha256"], digest(ledger_path))

    def test_nested_term_shapes_are_diagnosed_without_crashing(self):
        ledger = self.completed()
        paths = [
            ("category",), ("aliases",), ("searches",), ("searches", "family"),
            ("searches", "family", "queries"), ("source_locator",),
            ("glossary_matches",), ("terminology_pointer",), ("decision",),
        ]
        for target in paths:
            for value in (None, 7, {}, []):
                changed = deepcopy(ledger)
                parent = changed["units"]["paragraph-a"]["term_candidates"][0]
                for key in target[:-1]:
                    parent = parent[key]
                parent[target[-1]] = value
                if target == ("aliases",) and value == []:
                    continue
                with self.subTest(target=target, value=value):
                    report = self.validate(changed)
                    self.assertFalse(report.records_ready)
                    self.assertTrue(report.findings)


if __name__ == "__main__":
    unittest.main()
