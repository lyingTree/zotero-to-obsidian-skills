"""Compact recording against entirely invented, temporary source material."""
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
        raise ImportError(path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


tools = load("test_compact_translation_writer", ROOT.joinpath("literature-translate-zh", "scripts", "translation_review_writer.py"))
example = load("test_compact_translation_example", ROOT.joinpath("literature-translate-zh", "references", "compact_review_example.py"))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class WriterFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.source = self.base.joinpath("invented.pdf")
        self.candidate = self.base.joinpath("candidate.md")
        self.glossary = self.base.joinpath("glossary.md")
        self.inventory_path = self.base.joinpath("inventory.json")
        self.prepared = self.base.joinpath("pending.json")
        self.output = self.base.joinpath("review-new.json")
        self.term_output = self.base.joinpath("terms-new.json")
        self.source_text = "The cobalt badge is below the paper loop. Its size is 7 px. A=B."
        self.candidate_text = "钴蓝色徽片位于纸环下方。其尺寸为7 px。A=B。"
        with fitz.open() as document:
            page = document.new_page(width=700, height=240)
            page.insert_text((25, 40), self.source_text)
            page.insert_text((25, 100), "The toy window is closed.")
            blocks = page.get_text("blocks")
            self.box, self.other_box = list(blocks[0][:4]), list(blocks[1][:4])
            document.save(self.source)
        self.candidate.write_text(self.candidate_text + "\n玩具窗处于关闭状态。\n", encoding="utf-8")
        self.glossary.write_text(
            "| ID | English | 中文 | Context |\n| --- | --- | --- | --- |\n"
            "| BADGE | cobalt badge | 钴蓝色徽片 | An invented toy part. |\n"
            "| WINDOW | toy window | 玩具窗 | A different invented part. |\n", encoding="utf-8")
        self.inventory = {
            "schema_version": 1,
            "sources": [{"id": "source", "path": self.source.name, "sha256": digest(self.source), "page_count": 1}],
            "units": [
                {"id": "one", "source": "source", "kind": "paragraph", "evidence": [{"page": 1, "bbox": self.box, "quote": self.source_text}]},
                {"id": "two", "source": "source", "kind": "paragraph", "evidence": [{"page": 1, "bbox": self.other_box, "quote": "The toy window is closed."}]},
            ],
        }
        self.inventory_path.write_text(json.dumps(self.inventory), encoding="utf-8")

    def writer(self, *, selected=("one",), locations=None):
        tools.PREPARE.prepare_translation_review(self.inventory_path, self.candidate, self.prepared,
            unit_ids=list(selected), glossary_path=self.glossary, candidate_locations=locations, progress=None)
        return tools.ReviewWriter(self.prepared, progress=None)

    def active(self, **kwargs):
        writer = self.writer(**kwargs)
        return writer, writer.unit("one", candidate_locations=[{"lines": [1, 1]}])

    def assert_writer_error(self, code, action):
        with self.assertRaises(tools.ReviewWriterError) as caught:
            action()
        self.assertEqual(caught.exception.code, code)
        self.assertTrue(caught.exception.object_id)


class CompactReviewTests(WriterFixture):
    def test_module_cache_is_isolated_by_resolved_skill_root(self):
        alias = self.base.joinpath("path-alias")
        alias.mkdir()
        first = alias.joinpath("..", "invented-snapshot-one")
        second = self.base.joinpath("invented-snapshot-two")
        for directory, value in ((first, "first-contract"), (second, "second-contract")):
            directory.mkdir()
            directory.joinpath("invented_contract.py").write_text("VALUE = " + repr(value) + "\n", encoding="utf-8")
        with patch.object(tools, "__file__", str(first.joinpath("translation_review_writer.py"))):
            one = tools._load("invented_contract.py")
        with patch.object(tools, "__file__", str(second.joinpath("translation_review_writer.py"))):
            two = tools._load("invented_contract.py")
        self.assertEqual(one.VALUE, "first-contract")
        self.assertEqual(two.VALUE, "second-contract")
        self.assertIsNot(one, two)
        self.assertEqual(Path(one.__file__).resolve(), first.joinpath("invented_contract.py").resolve())
        self.assertEqual(Path(two.__file__).resolve(), second.joinpath("invented_contract.py").resolve())

    def test_real_prepare_and_unchanged_validate_complete_invented_example(self):
        result = example.demonstrate(self.base.joinpath("complete-example"))
        report = result["report"]
        self.assertTrue(report["records_ready"])
        self.assertEqual(report["findings"], [])
        self.assertEqual(report["source_fidelity"], "not_assessed")
        self.assertEqual(report["execution_truth"], "not_assessed")
        ledger = json.loads(result["review_path"].read_text(encoding="utf-8"))
        unit = ledger["units"][example.UNIT_ID]
        self.assertEqual(set(unit["checks"]), set(tools.CHECKS))
        self.assertEqual(set(unit["term_scan"]), set(tools.TERM_CATEGORIES))
        self.assertEqual(set(unit["occurrences"]), set(tools.OCCURRENCE_KINDS))
        self.assertEqual(unit["occurrences"]["claims"][0]["source_claim"], example.SOURCE_TEXT)
        self.assertEqual(unit["occurrences"]["claims"][0]["candidate_claim"], example.CANDIDATE_TEXT)
        pending = json.loads(result["prepared_path"].read_text(encoding="utf-8"))
        self.assertEqual(pending["review_status"], "pending")
        self.assertTrue(all(row["status"] == "pending" for row in pending["units"][example.UNIT_ID]["checks"].values()))

    def test_omitted_targets_stay_pending_and_scope_is_not_pruned(self):
        writer, unit = self.active()
        before = self.prepared.read_bytes()
        unit.check("alignment", "passed", "This explicitly compared sentence occupies candidate line one.")
        writer.review("reviewed")
        report = writer.save(self.output, terminology_path=self.term_output)
        self.assertFalse(report.records_ready)
        self.assertIn("units/one/checks/directions", report.pending_ids)
        self.assertIn("units/one/term_scan/model", report.pending_ids)
        self.assertTrue(any(item.code == "TRN_SCOPE_UNACCOUNTED" and item.object_id == "two" for item in report.findings))
        self.assertEqual(self.prepared.read_bytes(), before)
        row = json.loads(self.output.read_text(encoding="utf-8"))["units"]["one"]
        self.assertEqual(len(row["checks"]), 17)
        self.assertEqual(len(row["term_scan"]), 10)
        self.assertEqual(len(row["occurrences"]), 14)

    def test_compact_draft_resumes_exact_decisions_and_keeps_remaining_targets(self):
        writer, unit = self.active()
        unit.check("alignment", "passed", "The displayed complete pair was compared.")
        unit.occurrence("claims", "claim-a", unit.source(self.source_text), unit.candidate(self.candidate_text),
                        "failed", "The recorder retains this explicit failed judgment; no result is converted to pass.")
        draft = writer.save_draft(self.base.joinpath("one-draft.json"))
        restored = tools.ReviewWriter.from_draft(draft, progress=None)
        self.assertEqual(restored.data, writer.data)
        report = restored.save(self.output, terminology_path=self.term_output)
        self.assertIn("units/one/occurrences/claims/claim-a", report.failed_ids)
        self.assertIn("units/one/checks/strength", report.pending_ids)

    def test_file_outputs_never_overwrite_existing_records(self):
        writer, _ = self.active()
        pending = self.prepared.read_bytes()
        with self.assertRaises(FileExistsError):
            writer.save(self.prepared, terminology_path=self.term_output)
        self.assertEqual(self.prepared.read_bytes(), pending)
        self.assertFalse(self.term_output.exists())
        self.term_output.write_text("keep this record", encoding="utf-8")
        with self.assertRaises(FileExistsError):
            writer.save(self.output, terminology_path=self.term_output)
        self.assertFalse(self.output.exists())
        self.assertEqual(self.term_output.read_text(encoding="utf-8"), "keep this record")

    def test_occurrence_must_belong_to_selected_unit_on_each_side(self):
        writer, one = self.active(selected=("one", "two"))
        two = writer.unit("two", candidate_locations=[{"lines": [2, 2]}])
        self.assert_writer_error("TRW_LOCATION", lambda: one.source("The toy window is closed."))
        self.assert_writer_error("TRW_LOCATION", lambda: one.candidate("玩具窗处于关闭状态。"))
        self.assert_writer_error("TRW_SELECTION_UNIT", lambda: one.occurrence("claims", "wrong-unit",
            two.source("The toy window is closed."), one.candidate(self.candidate_text), "passed", "This intentionally wrong cross-unit pairing must be rejected."))

    def test_actual_page_box_and_block_identity_are_checked(self):
        _, unit = self.active()
        self.assert_writer_error("TRW_LOCATION", lambda: unit.source("visible token", page=1, bbox=self.other_box,
            observation_basis="An intentionally wrong region of the actual page."))
        self.assert_writer_error("TRW_LOCATION", lambda: unit.source("visible token", page=2, bbox=self.box,
            observation_basis="This requested page does not exist."))
        self.assert_writer_error("TRW_LOCATION", lambda: unit.source("cobalt badge", block_id="other-unit-block"))

    def test_visual_source_transcription_does_not_require_a_text_layer_match(self):
        writer, unit = self.active()
        self.assert_writer_error("TRW_LOCATION", lambda: unit.source("A visible invented glyph"))
        source = unit.source("A visible invented glyph", page=1, bbox=self.box,
            observation_basis="Caller transcribed the visible shape within this actual page region; this statement is not automatic proof of viewing.")
        candidate = unit.candidate("钴蓝色徽片")
        unit.occurrence("claims", "visible-glyph", source, candidate, "unresolved", "The supplied visible transcription remains an unresolved comparison.")
        row = unit.row["occurrences"]["claims"][0]
        self.assertIsNone(row["source_selection"]["raw_text"])
        self.assertEqual(row["source_claim"], "A visible invented glyph")
        self.assertIn("observation_basis", row["source_span"])
        report = writer.save(self.output, terminology_path=self.term_output)
        self.assertIn("units/one/occurrences/claims/visible-glyph", report.unresolved_ids)

    def test_escaped_candidate_cell_keeps_source_bytes_and_interpreted_text_separate(self):
        self.candidate.write_text('content: "upper\\nlower"\n', encoding="utf-8")
        writer, unit = self.active()
        self.assert_writer_error("TRW_LOCATION", lambda: unit.candidate("upper\nlower"))
        self.assert_writer_error("TRW_RENDERED_BASIS", lambda: unit.candidate(r"upper\nlower", comparison_text="upper\nlower"))
        candidate = unit.candidate(r"upper\nlower", comparison_text="upper\nlower",
                                  interpretation_basis="Caller decoded the actual YAML double-quoted scalar's newline escape for this cell.")
        source = unit.source("upper and lower", page=1, bbox=self.box,
                             observation_basis="Caller supplied the visible cell transcription at the actual source region.")
        unit.occurrence("table_cells", "invented-cell", source, candidate, "unresolved", "The two cell readings remain an explicit unresolved comparison.",
                        source_cell={"table_id": "source-table", "row": 1, "column": 1},
                        candidate_cell={"table_id": "candidate-table", "row": 1, "column": 1})
        row = unit.row["occurrences"]["table_cells"][0]
        self.assertEqual(row["candidate_span"]["quote"], r"upper\nlower")
        self.assertEqual(row["candidate_cell"]["text"], "upper\nlower")
        self.assertEqual(row["candidate_selection"]["raw_text"], r"upper\nlower")
        self.assertNotEqual(row["candidate_selection"]["raw_text"], row["candidate_cell"]["text"])

    def test_repeated_text_needs_explicit_choice_including_within_one_line(self):
        cases = (
            ("lf", "徽片，徽片。\n徽片。\n".encode("utf-8"), [7, 9]),
            ("crlf", "徽片，徽片。\r\n徽片。\r\n".encode("utf-8"), [8, 10]),
        )
        for ending, raw, expected in cases:
            with self.subTest(line_ending=ending):
                self.candidate.write_bytes(raw)
                self.prepared = self.base.joinpath("pending-" + ending + ".json")
                writer = self.writer()
                unit = writer.unit("one", candidate_locations=[{"lines": [1, 2]}])
                self.assert_writer_error("TRW_LOCATION_AMBIGUOUS", lambda: unit.candidate("徽片"))
                self.assert_writer_error("TRW_LOCATION_AMBIGUOUS", lambda: unit.candidate("徽片", lines=[1, 1]))
                second = unit.candidate("徽片", lines=[1, 1], occurrence=2)
                self.assertEqual(second.locator["char_range"], [3, 5])
                selected = unit.candidate("徽片", lines=[2, 2])
                self.assertEqual(selected.locator["char_range"], expected)
                actual = self.candidate.read_bytes().decode("utf-8")
                self.assertEqual(actual[expected[0]:expected[1]], selected.raw_text)

    def test_ambiguous_complete_unit_quote_requires_explicit_lines(self):
        cases = (("lf", b"same\nsame\n", [5, 9]), ("crlf", b"same\r\nsame\r\n", [6, 10]))
        for ending, raw, expected in cases:
            with self.subTest(line_ending=ending):
                self.candidate.write_bytes(raw)
                self.prepared = self.base.joinpath("pending-" + ending + ".json")
                writer = self.writer()
                self.assert_writer_error("TRW_LOCATION_AMBIGUOUS", lambda: writer.unit("one", candidate_locations=[{"quote": "same"}]))
                unit = writer.unit("one", candidate_locations=[{"lines": [2, 2]}])
                selected = unit.candidate("same")
                self.assertEqual(selected.locator["char_range"], expected)
                actual = self.candidate.read_bytes().decode("utf-8")
                self.assertEqual(actual[expected[0]:expected[1]], selected.raw_text)

    def test_inline_unit_selection_keeps_occurrence_inside_unique_parent_quote(self):
        self.candidate.write_text("prefix 徽片，徽片。 suffix\n", encoding="utf-8")
        writer = self.writer()
        unit = writer.unit("one", candidate_locations=[{"quote": "徽片，徽片。"}])
        selected = unit.candidate("徽片", occurrence=2)
        unit.occurrence("claims", "inline-second", unit.source("cobalt badge"), selected,
                        "unresolved", "This explicit repeated inline position remains an unresolved comparison.")
        report = writer.save(self.output, terminology_path=self.term_output)
        self.assertFalse(any(item.code == "TRN_CANDIDATE_SPAN_OUTSIDE_UNIT" for item in report.findings))
        self.assertEqual(selected.locator["selected_quote"], "徽片")
        self.assertEqual(selected.locator["char_range"], [10, 12])

    def test_candidate_line_endings_are_not_silently_normalized(self):
        self.candidate.write_bytes(b"first\r\nsecond\r\n")
        writer = self.writer()
        unit = writer.unit("one", candidate_locations=[{"lines": [1, 2]}])
        self.assert_writer_error("TRW_LOCATION", lambda: unit.candidate("first\nsecond"))
        selected = unit.candidate("first\r\nsecond")
        self.assertEqual(selected.raw_text, "first\r\nsecond")
        self.assertEqual(selected.locator["char_range"], [0, 13])

    def test_direction_and_formula_interpretations_are_never_inferred(self):
        _, unit = self.active()
        source, candidate = unit.source("A=B"), unit.candidate("A=B")
        self.assert_writer_error("TRW_COMPARISON_FIELD", lambda: unit.occurrence("directions", "missing-relation", source, candidate,
            "passed", "A textual presence cannot supply a direction judgment."))
        self.assert_writer_error("TRW_COMPARISON_FIELD", lambda: unit.occurrence("formulas", "missing-structure", source, candidate,
            "passed", "A source-code check cannot supply formula structure comparisons."))
        structure = {"denominators": [], "radicals": [], "exponents": [], "brackets": []}
        unit.occurrence("formulas", "explicit-structure", source, candidate, "passed", "The caller compared this invented equality and explicitly found no listed structures on either side.",
                        source_structure=structure, candidate_structure=deepcopy(structure))
        self.assertEqual(unit.row["checks"]["formulas"]["status"], "pending")

    def test_missing_independent_structure_component_gets_specific_diagnostic(self):
        _, unit = self.active()
        source, candidate = unit.source("A=B"), unit.candidate("A=B")
        incomplete = {"denominators": [], "radicals": [], "exponents": []}
        with self.assertRaises(tools.ReviewWriterError) as caught:
            unit.occurrence("formulas", "bad-structure", source, candidate, "passed", "This deliberately incomplete structure must be rejected.",
                            source_structure=incomplete, candidate_structure=incomplete)
        self.assertEqual(caught.exception.code, "TRN_TARGET_MISSING")
        self.assertTrue(caught.exception.object_id.endswith("source_structure/brackets"))

    def test_unknown_categories_checks_and_occurrence_kinds_are_rejected(self):
        _, unit = self.active()
        self.assert_writer_error("TRW_CHECK", lambda: unit.check("everything", "passed", "Not a real target."))
        self.assert_writer_error("TRW_CATEGORY", lambda: unit.scan("technical_words", "not_applicable", [], "Not an allowed category."))
        self.assert_writer_error("TRW_CATEGORY", lambda: unit.term("term-a", "cobalt badge", "technical_words", unit.source("cobalt badge")))
        self.assert_writer_error("TRW_OCCURRENCE_KIND", lambda: unit.occurrence("all_claims", "x", unit.source(self.source_text), unit.candidate(self.candidate_text), "passed", "Unknown group."))

    def test_unqueried_or_fabricated_glossary_hits_cannot_be_adopted(self):
        _, unit = self.active()
        term = unit.term("badge", "cobalt badge", "concept", unit.source("cobalt badge"))
        hits = term.search("full_phrase", "cobalt badge", "The actual returned entry is a search lead for this invented part.")
        self.assertEqual([hit["entry_id"] for hit in hits], ["BADGE"])
        self.assert_writer_error("TRW_GLOSSARY_HIT", lambda: term.decide("adopted", "钴蓝色徽片", "A fabricated hit must not be accepted.", matches=["FAKE"]))
        self.assert_writer_error("TRW_GLOSSARY_HIT", lambda: term.decide("adopted", "玩具窗", "An existing but unqueried entry is not this term's recorded hit.", matches=["WINDOW"]))
        self.assert_writer_error("TRW_IDS", lambda: term.decide("adopted", "钴蓝色徽片", "A caller cannot provide its own hit body.", matches=[{"entry_id": "BADGE"}]))
        term.decide("adopted", "钴蓝色徽片", "The caller explicitly chose this matching invented part.", matches=["BADGE"])
        self.assertEqual(unit.row["checks"]["terminology"]["status"], "pending")

    def test_tampered_query_results_are_recomputed_before_saving(self):
        writer, unit = self.active()
        term = unit.term("badge", "cobalt badge", "concept", unit.source("cobalt badge"))
        term.search("full_phrase", "cobalt badge", "One actual row was retrieved.")
        term.row["searches"]["full_phrase"]["queries"][0]["hits"][0]["excerpt"] = "invented forged row"
        self.assert_writer_error("TRW_GLOSSARY_HIT", lambda: writer.save(self.output, terminology_path=self.term_output))
        self.assertFalse(self.output.exists())
        self.assertFalse(self.term_output.exists())

    def test_term_search_and_scan_absence_need_explicit_conditions(self):
        _, unit = self.active()
        term = unit.term("badge", "cobalt badge", "concept", unit.source("cobalt badge"))
        self.assert_writer_error("TRW_APPLICABILITY", lambda: term.search_status("family", "not_applicable", "No family query was performed."))
        self.assert_writer_error("TRW_SEARCH_ACTION", lambda: term.search_status("full_phrase", "searched", "A claimed query is not an actual query."))
        self.assert_writer_error("TRW_APPLICABILITY", lambda: unit.scan("concept", "scanned", [], "No candidates supplied."))
        self.assert_writer_error("TRW_APPLICABILITY", lambda: unit.scan("concept", "not_applicable", ["badge"], "A registered candidate contradicts absence."))
        self.assert_writer_error("TRW_TERM_MAPPING", lambda: unit.scan("model", "scanned", ["unknown-term"], "This term was never registered."))

    def test_one_term_may_be_explicitly_scanned_in_multiple_categories(self):
        _, unit = self.active()
        unit.term("badge", "cobalt badge", "concept", unit.source("cobalt badge"))
        unit.scan("concept", "scanned", ["badge"], "The explicit primary concept scan includes this candidate.")
        unit.scan("classification", "scanned", ["badge"], "The caller also explicitly reviews this candidate as a possible class label without inventing a second term.")
        self.assertEqual(len(unit.row["term_candidates"]), 1)
        self.assertEqual(unit.row["term_scan"]["classification"]["candidate_ids"], ["badge"])

    def test_unlisted_term_decision_generates_actual_terminology_pointer(self):
        writer, unit = self.active()
        term = unit.term("loop", "paper loop", "concept", unit.source("paper loop"))
        term.search("full_phrase", "paper loop", "There is no phrase entry in this actual glossary.")
        term.search("family", "loop", "The actual family query also returns zero entries.")
        term.search_status("aliases", "not_applicable", "This source unit supplies no alias for paper loop.")
        self.assert_writer_error("TRW_TERM_BASIS", lambda: term.decide("unlisted", "纸环", "An unlisted decision still needs a contextual basis."))
        term.decide("unlisted", "纸环", "The caller chooses the component's material and shape reading.",
                    resolution_basis="Paper qualifies the physical loop in this invented toy layout; 纸环 preserves those parts of the expression.")
        unit.scan("concept", "scanned", ["loop"], "The explicitly recorded concept was searched and decided.")
        writer.save(self.output, terminology_path=self.term_output)
        saved = json.loads(self.output.read_text(encoding="utf-8"))
        terms = json.loads(self.term_output.read_text(encoding="utf-8"))
        self.assertEqual(saved["units"]["one"]["term_candidates"][0]["terminology_pointer"], "/unlisted/0")
        self.assertEqual(terms["unlisted"][0]["source_units"], ["one"])
        self.assertEqual(terms["unlisted"][0]["translation"], "纸环")

    def test_changed_inputs_reject_save_before_any_output(self):
        for name in ("candidate", "source", "glossary", "inventory"):
            with self.subTest(input=name):
                fixture = WriterFixture()
                fixture.setUp()
                try:
                    writer, unit = fixture.active()
                    unit.check("alignment", "passed", "A decision bound before the intentional change.")
                    path = {"candidate": fixture.candidate, "source": fixture.source,
                            "glossary": fixture.glossary, "inventory": fixture.inventory_path}[name]
                    path.write_bytes(path.read_bytes() + b"\nchanged")
                    fixture.assert_writer_error("TRW_STALE_BINDING", lambda: writer.save(fixture.output, terminology_path=fixture.term_output))
                    self.assertFalse(fixture.output.exists())
                    self.assertFalse(fixture.term_output.exists())
                finally:
                    fixture.doCleanups()

    def test_changed_candidate_does_not_resume_old_decisions(self):
        writer, unit = self.active()
        unit.check("alignment", "passed", "This belongs only to the earlier candidate bytes.")
        draft = writer.save_draft(self.base.joinpath("before-change.json"))
        self.candidate.write_text(self.candidate_text + "\nA new paragraph.\n", encoding="utf-8")
        self.assert_writer_error("TRW_STALE_BINDING", lambda: tools.ReviewWriter.from_draft(draft, progress=None))

    def test_changed_glossary_rejects_search_against_cached_index(self):
        _, unit = self.active()
        term = unit.term("badge", "cobalt badge", "concept", unit.source("cobalt badge"))
        term.search("full_phrase", "cobalt badge", "The original glossary was actually searched.")
        self.glossary.write_text(self.glossary.read_text(encoding="utf-8") + "| EXTRA | cobalt | 蓝 | extra |\n", encoding="utf-8")
        self.assert_writer_error("TRW_STALE_BINDING", lambda: term.search("family", "cobalt", "Do not use a stale index."))

    def test_duplicate_occurrence_id_cannot_hide_another_instance(self):
        _, unit = self.active()
        source, candidate = unit.source("7"), unit.candidate("7")
        unit.occurrence("numbers", "value", source, candidate, "passed", "The invented integer is unchanged.", difference_type="unchanged")
        self.assert_writer_error("TRW_DUPLICATE", lambda: unit.occurrence("claims", "value", source, candidate, "passed", "This duplicate ID must not replace the earlier independent instance."))

    def test_removed_required_target_rejected_when_loading(self):
        self.writer()
        data = json.loads(self.prepared.read_text(encoding="utf-8"))
        del data["units"]["one"]["checks"]["conditions"]
        self.prepared.write_text(json.dumps(data), encoding="utf-8")
        self.assert_writer_error("TRW_TARGETS", lambda: tools.ReviewWriter(self.prepared, progress=None))

    def test_selected_text_cannot_be_silently_replaced_by_a_claim_field(self):
        _, unit = self.active()
        self.assert_writer_error("TRW_COMPARISON_TEXT", lambda: unit.occurrence("claims", "invented-replacement",
            unit.source(self.source_text), unit.candidate(self.candidate_text), "passed", "Explicit selected text is the comparison basis.",
            source_claim="An unrelated assertion"))


if __name__ == "__main__":
    unittest.main()
