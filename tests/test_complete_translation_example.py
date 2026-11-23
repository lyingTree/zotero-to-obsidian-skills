"""Run the complete fictional example against the real preparation/validation API."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
EXAMPLE_PATH = ROOT.joinpath("literature-translate-zh", "references", "complete_review_example.py")
spec = importlib.util.spec_from_file_location("tested_complete_translation_example", EXAMPLE_PATH)
if spec is None or spec.loader is None:
    raise ImportError(str(EXAMPLE_PATH))
example = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = example
spec.loader.exec_module(example)


class CompleteTranslationExampleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.work = Path(self.temporary.name).joinpath("complete-example")
        self.result = example.demonstrate(self.work, progress=None)
        self.review_path = Path(self.result["review_path"])

    def read_review(self):
        return json.loads(self.review_path.read_text(encoding="utf-8"))

    def validate(self):
        return example.validate.validate_translation_review(
            self.review_path, Path(self.result["inventory_path"]), Path(self.result["candidate_path"]))

    def test_complete_record_is_ready_without_certifying_meaning_or_execution(self):
        before = self.review_path.read_bytes()
        report = self.validate()
        self.assertEqual(report.findings, [])
        self.assertTrue(report.records_ready)
        self.assertTrue(self.result["report"]["records_ready"])
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertEqual(report.execution_truth, "not_assessed")
        self.assertTrue(report.independent_review_required)
        self.assertEqual(self.review_path.read_bytes(), before)

        ledger = self.read_review()
        unit = ledger["units"][example.UNIT_ID]
        self.assertEqual(set(unit["checks"]), set(example.prepare.CHECKS))
        self.assertEqual(len(unit["checks"]), 17)
        self.assertEqual(set(unit["term_scan"]), set(example.prepare.TERM_CATEGORIES))
        self.assertEqual(len(unit["term_scan"]), 10)
        self.assertEqual(ledger["scope"]["selected_units"], [example.UNIT_ID])
        self.assertEqual(ledger["scope"]["unselected_inventory_units"], [])
        self.assertEqual(ledger["scope"]["excluded_units"], [])
        self.assertEqual(ledger["events"], [])
        self.assertEqual(ledger["source_issues_review"]["status"], "passed")
        self.assertEqual(len(unit["occurrences"]["claims"]), 4)
        self.assertEqual(len(unit["occurrences"]["strength"]), 4)
        self.assertEqual(len(unit["occurrences"]["modifiers"]), 6)
        self.assertEqual(len(unit["occurrences"]["conditions"]), 3)
        self.assertEqual(unit["occurrences"]["conditions"][0]["source_condition"]["necessary_prior_event"], "closing the latch")

        # The preparer remains unchanged: the original scaffold is still pending.
        pending = json.loads(Path(self.result["pending_path"]).read_text(encoding="utf-8"))
        self.assertEqual(pending["review_status"], "pending")
        self.assertTrue(all(row["status"] == "pending" for row in pending["units"][example.UNIT_ID]["checks"].values()))

    def test_actual_searches_bind_full_phrase_alias_family_and_unlisted_choice(self):
        ledger = self.read_review()
        candidates = {row["id"]: row for row in ledger["units"][example.UNIT_ID]["term_candidates"]}
        offset = candidates["offset"]
        self.assertEqual(offset["aliases"], ["RO"])
        self.assertEqual(offset["searches"]["aliases"]["queries"][0]["query"], "RO")
        self.assertEqual(offset["searches"]["aliases"]["queries"][0]["hits"][0]["entry_id"], "DEMO002")
        self.assertEqual([row["query"] for row in offset["searches"]["family"]["queries"]], ["radial", "radius"])
        self.assertEqual(offset["glossary_matches"][0]["entry_id"], "DEMO002")
        self.assertEqual(offset["translation"], "径向偏移量")

        static_check = candidates["static-check"]
        self.assertEqual(static_check["searches"]["full_phrase"]["queries"][0]["hits"], [])
        self.assertEqual(static_check["searches"]["family"]["queries"][0]["hits"][0]["entry_id"], "DEMO003")
        self.assertEqual(static_check["decision"], "unlisted")
        self.assertEqual(static_check["terminology_pointer"], "/unlisted/0")
        terms = json.loads(Path(self.result["terminology_path"]).read_text(encoding="utf-8"))
        self.assertEqual(terms["glossary"], ledger["bindings"]["glossary"])
        self.assertEqual(terms["unlisted"][0]["status"], "resolved")
        self.assertEqual(terms["covered_units"], [example.UNIT_ID])
        self.assertTrue(self.validate().records_ready)

    def test_changed_candidate_invalidates_the_previously_complete_record(self):
        candidate = Path(self.result["candidate_path"])
        candidate.write_text(candidate.read_text(encoding="utf-8").replace("内侧", "外侧", 1), encoding="utf-8")
        report = self.validate()
        self.assertFalse(report.records_ready)
        self.assertIn("TRN_HASH_CHANGED", {finding.code for finding in report.findings})
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_missing_required_target_cannot_pass_as_a_complete_record(self):
        ledger = self.read_review()
        del ledger["units"][example.UNIT_ID]["checks"]["negation"]
        self.review_path.write_text(json.dumps(ledger, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        report = self.validate()
        self.assertFalse(report.records_ready)
        missing = [finding.object_id for finding in report.findings if finding.code == "TRN_TARGET_MISSING"]
        self.assertIn(f"units/{example.UNIT_ID}/checks/negation", missing)
        self.assertEqual(report.source_fidelity, "not_assessed")


if __name__ == "__main__":
    unittest.main()
