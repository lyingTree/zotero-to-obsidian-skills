"""Pending translation records tested with temporary synthetic source files."""

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


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class TranslationReviewTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
