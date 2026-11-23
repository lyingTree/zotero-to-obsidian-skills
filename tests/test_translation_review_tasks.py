"""Bounded task ordering on actual, wholly invented source/candidate files."""
from __future__ import annotations

from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import fitz


ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(str(path))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


queue = load("test_translation_tasks", ROOT.joinpath("literature-translate-zh", "scripts", "translation_review_tasks.py"))
example = load("test_translation_tasks_example", ROOT.joinpath("literature-translate-zh", "references", "compact_review_example.py"))
writer_tools = queue.WRITER


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


class TranslationTaskTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.example = example.demonstrate(self.base.joinpath("invented-complete"))

    def complete_writer(self):
        return writer_tools.ReviewWriter(self.example["review_path"], progress=None)

    def snapshot_files(self):
        return {str(path.relative_to(self.base)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in self.base.rglob("*") if path.is_file()}

    def two_units(self, selected=("toy", "toy-extra")):
        """The same invented sentence is actually printed on two separate pages.

        This deliberately exercises repeated text with distinct source identities
        and candidate lines. Decisions are replayed only inside this test fixture;
        production code never copies a judgment to a different unit.
        """
        directory = self.base.joinpath("two-printed-units")
        directory.mkdir()
        source = directory.joinpath("invented.pdf")
        candidate = directory.joinpath("candidate.md")
        inventory = directory.joinpath("inventory.json")
        prepared = directory.joinpath("prepared.json")
        units = []
        with fitz.open() as document:
            for number, unit_id in enumerate(("toy", "toy-extra"), 1):
                page = document.new_page(width=440, height=180)
                page.insert_text((24, 42), example.SOURCE_TEXT)
                units.append({"id": unit_id, "source": "invented", "kind": "paragraph",
                              "evidence": [{"page": number, "bbox": list(page.get_text("blocks")[0][:4]),
                                            "quote": example.SOURCE_TEXT}]})
            document.save(source)
        candidate.write_bytes(((example.CANDIDATE_TEXT + "\n") * 2).encode("utf-8"))
        inventory.write_text(json.dumps({
            "schema_version": 1,
            "sources": [{"id": "invented", **writer_tools.PREPARE.bind_review_file(source), "page_count": 2}],
            "units": units,
        }), encoding="utf-8")
        locations = {identity: [{"lines": [number, number]}]
                     for number, identity in enumerate(("toy", "toy-extra"), 1) if identity in selected}
        writer_tools.PREPARE.prepare_translation_review(inventory, candidate, prepared,
            unit_ids=list(selected), glossary_path=self.example["glossary_path"],
            candidate_locations=locations, progress=None)
        writer = writer_tools.ReviewWriter(prepared, progress=None)
        writer.scope("passed", "The requested invented span contains these explicitly selected printed units.")
        return writer

    def fill_invented_unit(self, writer, unit_id, line):
        for raw in read(self.example["draft_path"])["decisions"]:
            if "unit_id" not in raw:
                continue
            command = deepcopy(raw)
            command["unit_id"] = unit_id
            if command["op"] == "unit":
                command["candidate_locations"] = [{"lines": [line, line]}]
            writer._replay(command)

    def test_scope_precedes_units_and_never_infers_applicability(self):
        writer = self.two_units(selected=("toy",))
        self.fill_invented_unit(writer, "toy", 1)
        before = deepcopy(writer.data)
        task = queue.next_review_task(writer, scope_limit=1)
        self.assertEqual(task["stage"], "scope")
        self.assertEqual(task["unaccounted_count"], 1)
        self.assertEqual(task["unaccounted_units"][0]["unit_id"], "toy-extra")
        self.assertEqual(task["unaccounted_units"][0]["source_locations"][0]["page"], 2)
        self.assertEqual(writer.data, before)
        writer.exclude("toy-extra", "This invented local task requests only the first printed sentence.",
                       "The second printed sentence is outside that explicit test task span.")
        self.assertNotEqual(queue.next_review_task(writer)["stage"], "scope")

    def test_current_unit_is_complete_before_queue_moves_to_next_unit(self):
        writer = self.two_units()
        self.assertEqual(queue.next_review_task(writer)["unit_id"], "toy")
        self.fill_invented_unit(writer, "toy", 1)
        self.assertTrue(queue.require_unit_records(writer, "toy")["unit_records_ready"])
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "unit")
        self.assertEqual(task["unit_id"], "toy-extra")
        self.assertEqual(task["completed_preceding_units"], ["toy"])
        self.assertEqual(task["source_locations"][0]["page"], 2)
        self.assertEqual(task["candidate_locations"][0]["text"], example.CANDIDATE_TEXT)
        self.assertEqual(task["candidate_locations"][0]["locator"]["lines"], [2, 2])
        self.assertEqual(len(task["checks"]), 17)
        self.assertEqual(len(task["term_scan"]), 10)
        self.assertEqual(len(task["occurrence_groups"]), 14)
        with self.assertRaises(queue.IncompleteUnitRecordsError):
            queue.require_unit_records(writer, "toy-extra")

    def test_complete_units_do_not_hide_pending_global_records(self):
        writer = self.two_units()
        self.fill_invented_unit(writer, "toy", 1)
        self.fill_invented_unit(writer, "toy-extra", 2)
        self.assertTrue(queue.require_unit_records(writer, "toy-extra")["unit_records_ready"])
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "global_records")
        self.assertFalse(task["preview_records_ready"])
        self.assertIn("review_status", task["pending_ids"])
        writer.source_issues_review("passed", "Both printed invented sentences and their positions were read; no unresolved source reading remains.")
        writer.terminology_review("reviewed", covered_units=["toy", "toy-extra"],
            no_unlisted_reason="Both units use the same two invented component concepts; their actual glossary phrase and family queries find matching entries.")
        writer.review("reviewed")
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "save_final_records")
        self.assertTrue(task["preview_records_ready"])
        self.assertEqual(task["source_fidelity"], "not_assessed")
        final = self.base.joinpath("actual-final.json")
        report = writer.save(final, terminology_path=self.base.joinpath("actual-final-terms.json"))
        self.assertTrue(report.records_ready)
        self.assertEqual(report.findings, [])

    def test_queue_and_unit_inspection_preserve_all_files_and_decisions(self):
        writer = self.complete_writer()
        data_before, commands_before = deepcopy(writer.data), deepcopy(writer._commands)
        files_before = self.snapshot_files()
        self.assertTrue(queue.unit_record_report(writer, example.UNIT_ID)["unit_records_ready"])
        self.assertEqual(queue.next_review_task(writer)["stage"], "save_final_records")
        self.assertEqual(writer.data, data_before)
        self.assertEqual(writer._commands, commands_before)
        self.assertEqual(self.snapshot_files(), files_before)

    def test_passed_completeness_with_empty_claims_cannot_advance(self):
        writer = self.complete_writer()
        writer.data["units"][example.UNIT_ID]["occurrences"]["claims"] = []
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "unit")
        self.assertTrue(any(row["code"] == "TRN_OCCURRENCES_MISSING" for row in task["findings"]))
        with self.assertRaises(queue.IncompleteUnitRecordsError):
            queue.require_unit_records(writer, example.UNIT_ID)

    def test_removed_category_is_still_rejected_by_original_validator(self):
        writer = self.complete_writer()
        writer.data["units"][example.UNIT_ID]["term_scan"].pop("metric")
        result = queue.unit_record_report(writer, example.UNIT_ID)
        self.assertFalse(result["unit_records_ready"])
        self.assertTrue(result["findings"])

    def test_explicit_failed_or_unresolved_unit_decisions_are_not_ready(self):
        for status in ("failed", "unresolved", "blocked", "pending"):
            with self.subTest(status=status):
                writer = self.complete_writer()
                writer.unit(example.UNIT_ID).check("alignment", status, "This is the fixture's explicit current undecided or negative alignment result.")
                result = queue.unit_record_report(writer, example.UNIT_ID)
                self.assertFalse(result["unit_records_ready"])
                self.assertEqual(queue.next_review_task(writer)["stage"], "unit")

    def test_changed_candidate_is_rejected_without_automatic_rebinding(self):
        writer = self.complete_writer()
        writer.candidate_path.write_bytes(writer.candidate_path.read_bytes() + b"changed\n")
        with self.assertRaises(writer_tools.ReviewWriterError) as caught:
            queue.next_review_task(writer)
        self.assertEqual(caught.exception.code, "TRW_STALE_BINDING")

    def test_forged_glossary_hit_is_rejected_before_inspecting_progress(self):
        writer = self.complete_writer()
        term = writer.data["units"][example.UNIT_ID]["term_candidates"][0]
        term["glossary_matches"][0]["entry_id"] = "NOT_AN_ACTUAL_HIT"
        with self.assertRaises(writer_tools.ReviewWriterError) as caught:
            queue.next_review_task(writer)
        self.assertEqual(caught.exception.code, "TRW_GLOSSARY_HIT")

    def test_unknown_units_and_nonpositive_batch_sizes_are_rejected(self):
        writer = self.complete_writer()
        with self.assertRaises(ValueError):
            queue.unit_record_report(writer, "absent")
        for size in (0, -1, True, "8"):
            with self.subTest(size=size), self.assertRaises(ValueError):
                queue.next_review_task(writer, scope_limit=size)

    def test_scope_status_counts_include_the_actual_pending_scope_decision(self):
        writer = self.complete_writer()
        writer.scope("pending", "This synthetic scope still requires an explicit final decision.")
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "scope")
        self.assertEqual(task["status_counts"], {"pending": 1})
        self.assertEqual(task["diagnostic_totals"]["pending_ids"], 1)

    def test_missing_mappings_and_invalid_check_rows_return_original_diagnostics(self):
        for field in ("checks", "term_scan", "occurrences", "alignment-row"):
            with self.subTest(field=field):
                writer = self.complete_writer()
                row = writer.data["units"][example.UNIT_ID]
                if field == "alignment-row":
                    row["checks"]["alignment"] = None
                else:
                    row.pop(field)
                before = deepcopy(writer.data)
                task = queue.next_review_task(writer)
                self.assertEqual(task["stage"], "unit")
                self.assertEqual(task["unit_id"], example.UNIT_ID)
                self.assertEqual(len(task["checks"]), 17)
                self.assertEqual(len(task["term_scan"]), 10)
                self.assertEqual(len(task["occurrence_groups"]), 14)
                self.assertIn("TRN_OBJECT_REQUIRED", task["diagnostic_totals"]["codes"])
                with self.assertRaises(queue.IncompleteUnitRecordsError):
                    queue.require_unit_records(writer, example.UNIT_ID)
                self.assertEqual(writer.data, before)

    def test_incomplete_terminology_rows_cannot_crash_or_become_ready(self):
        cases = (
            ("missing-match", "TRN_GLOSSARY_MATCH_MISSING"),
            ("missing-original", "TRN_FIELD_MISSING"),
            ("bad-search-object", "TRN_OBJECT_REQUIRED"),
        )
        for condition, expected_code in cases:
            with self.subTest(condition=condition):
                writer = self.complete_writer()
                term = writer.data["units"][example.UNIT_ID]["term_candidates"][0]
                if condition == "missing-match":
                    term["glossary_matches"] = []
                elif condition == "missing-original":
                    term.pop("original")
                else:
                    term["searches"]["full_phrase"] = None
                before = deepcopy(writer.data)
                task = queue.next_review_task(writer)
                self.assertEqual(task["stage"], "unit")
                self.assertIn(expected_code, task["diagnostic_totals"]["codes"])
                self.assertFalse(queue.unit_record_report(writer, example.UNIT_ID)["unit_records_ready"])
                self.assertEqual(writer.data, before)

    def test_later_incomplete_term_does_not_poison_the_ready_preceding_unit(self):
        writer = self.two_units()
        self.fill_invented_unit(writer, "toy", 1)
        self.fill_invented_unit(writer, "toy-extra", 2)
        writer.data["units"]["toy-extra"]["term_candidates"][0]["glossary_matches"] = []
        self.assertTrue(queue.require_unit_records(writer, "toy")["unit_records_ready"])
        task = queue.next_review_task(writer)
        self.assertEqual(task["unit_id"], "toy-extra")
        self.assertEqual(task["completed_preceding_units"], ["toy"])
        self.assertIn("TRN_GLOSSARY_MATCH_MISSING", task["diagnostic_totals"]["codes"])

    def test_invalid_exclusion_row_retains_scope_failure(self):
        writer = self.complete_writer()
        writer.data["scope"]["excluded_units"].append(None)
        before = deepcopy(writer.data)
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "scope")
        self.assertIn("TRN_OBJECT_REQUIRED", task["diagnostic_totals"]["codes"])
        self.assertEqual(task["unaccounted_count"], 0)
        self.assertEqual(writer.data, before)

    def test_scope_unaccounted_count_comes_from_all_actual_inventory_units(self):
        writer = self.two_units(selected=("toy",))
        writer.data["scope"]["unselected_inventory_units"] = []
        task = queue.next_review_task(writer, scope_limit=1)
        self.assertEqual(task["stage"], "scope")
        self.assertEqual(task["unaccounted_count"], 1)
        self.assertEqual(task["unaccounted_units"][0]["unit_id"], "toy-extra")
        self.assertIn("TRN_UNSELECTED_INVENTORY_MISMATCH", task["diagnostic_totals"]["codes"])
        self.assertIn("TRN_SCOPE_UNACCOUNTED", task["diagnostic_totals"]["codes"])

    def test_bounded_findings_keep_full_counts_and_late_failure(self):
        writer = self.complete_writer()
        unit = writer.unit(example.UNIT_ID)
        for name in writer_tools.CHECKS:
            status = "failed" if name == "modifier_scope" else "pending"
            unit.check(name, status, "This target has the explicitly recorded unfinished test status.")
        for category in writer_tools.TERM_CATEGORIES:
            unit.scan(category, "pending", unit.row["term_scan"][category]["candidate_ids"],
                      "This category still requires its own explicit test decision.")
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "unit")
        self.assertEqual(len(task["findings"]), 12)
        self.assertEqual(task["diagnostic_totals"]["findings"], 27)
        self.assertEqual(task["diagnostic_totals"]["pending_ids"], 26)
        self.assertEqual(task["diagnostic_totals"]["failed_ids"], 1)
        self.assertEqual(task["status_counts"]["pending"], 26)
        self.assertEqual(task["status_counts"]["failed"], 1)
        self.assertNotIn("TRN_RECORDED_FAILURE", {row["code"] for row in task["findings"]})
        self.assertIn("TRN_RECORDED_FAILURE", task["diagnostic_totals"]["codes"])
        self.assertIn("units/" + example.UNIT_ID + "/checks/modifier_scope", task["failed_ids"])
        self.assertFalse(queue.unit_record_report(writer, example.UNIT_ID)["unit_records_ready"])

    def test_cross_page_source_and_split_crlf_candidate_are_shown_completely(self):
        directory = self.base.joinpath("cross-page")
        directory.mkdir()
        source = directory.joinpath("invented.pdf")
        candidate = directory.joinpath("candidate.md")
        inventory = directory.joinpath("inventory.json")
        source_parts = ("A wooden pointer faces the fabric gate.", "The gate stays open.")
        candidate_parts = ("木制指针朝向织物门。", "门保持打开状态。")
        locations = []
        with fitz.open() as document:
            for number, text in enumerate(source_parts, 1):
                page = document.new_page(width=440, height=180)
                page.insert_text((24, 42), text)
                locations.append({"page": number, "bbox": list(page.get_text("blocks")[0][:4]), "quote": text})
            document.save(source)
        actual_text = candidate_parts[0] + "\r\n\r\n" + candidate_parts[1] + "\r\n"
        candidate.write_bytes(actual_text.encode("utf-8"))
        inventory.write_text(json.dumps({
            "schema_version": 1,
            "sources": [{"id": "cross", **writer_tools.PREPARE.bind_review_file(source), "page_count": 2}],
            "units": [{"id": "cross-paragraph", "source": "cross", "kind": "paragraph", "evidence": locations}],
        }), encoding="utf-8")
        pending = directory.joinpath("prepared.json")
        writer_tools.PREPARE.prepare_translation_review(inventory, candidate, pending, unit_ids=["cross-paragraph"],
            glossary_path=self.example["glossary_path"], candidate_locations={"cross-paragraph": [{"lines": [1, 1]}, {"lines": [3, 3]}]}, progress=None)
        writer = writer_tools.ReviewWriter(pending, progress=None)
        writer.scope("passed", "The invented two-page paragraph is the complete selected test scope.")
        task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "unit")
        self.assertEqual([row["page"] for row in task["source_locations"]], [1, 2])
        self.assertEqual([row["quote"] for row in task["source_locations"]], list(source_parts))
        self.assertEqual([row["text"] for row in task["candidate_locations"]], list(candidate_parts))
        second = task["candidate_locations"][1]
        self.assertEqual(second["char_range"][0], len(candidate_parts[0]) + 4)
        self.assertEqual(actual_text[second["char_range"][0]:second["char_range"][1]], candidate_parts[1])
        self.assertTrue(all(row["status"] == "pending" for row in task["checks"]))

    def test_same_binding_draft_resume_retains_next_exact_unit(self):
        writer = self.two_units()
        self.fill_invented_unit(writer, "toy", 1)
        draft = writer.save_draft(self.base.joinpath("task-checkpoint.json"))
        before = queue.next_review_task(writer)
        restored = writer_tools.ReviewWriter.from_draft(draft, progress=None)
        self.assertEqual(queue.next_review_task(restored), before)
        self.assertEqual(before["unit_id"], "toy-extra")
        self.assertEqual(before["completed_preceding_units"], ["toy"])

    def test_stale_non_candidate_bindings_are_checked_before_progress(self):
        for kind in ("source", "inventory", "glossary", "prepared"):
            with self.subTest(binding=kind):
                current = example.demonstrate(self.base.joinpath("changed-" + kind))
                writer = writer_tools.ReviewWriter(current["review_path"], progress=None)
                paths = {"source": Path(next(iter(writer.source_bindings.values()))["path"]),
                         "inventory": writer.inventory_path, "glossary": writer.glossary_path, "prepared": writer.path}
                path = paths[kind]
                path.write_bytes(path.read_bytes() + b"\nchanged\n")
                with self.assertRaises(writer_tools.ReviewWriterError) as caught:
                    queue.next_review_task(writer)
                self.assertEqual(caught.exception.code, "TRW_STALE_BINDING")

    def test_ready_preview_is_removed_and_does_not_export_delivery_bindings(self):
        writer = self.complete_writer()
        actual_save = writer.save
        preview_paths = []

        def observe_save(output, *, terminology_path):
            preview_paths.extend((Path(output), Path(terminology_path)))
            return actual_save(output, terminology_path=terminology_path)

        with patch.object(writer, "save", side_effect=observe_save):
            task = queue.next_review_task(writer)
        self.assertEqual(task["stage"], "save_final_records")
        self.assertTrue(task["preview_records_ready"])
        self.assertEqual(len(preview_paths), 2)
        self.assertTrue(all(not path.exists() for path in preview_paths))
        self.assertNotIn("translation_review_path", task)
        self.assertNotIn("terminology_record", task)
        self.assertIn("Temporary preview files are not a delivery", task["next_action"])


if __name__ == "__main__":
    unittest.main()
