"""Drafting coverage must come from the frozen inventory, not chosen excerpts.

These fixtures exercise the queue's inventory contract. PDF/DOCX extraction
itself is covered by the source adapter tests; no source parser is mocked here.
"""
from __future__ import annotations

from copy import deepcopy
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-zotero2obsidian", "scripts", "draft_queue.py")
SPEC = importlib.util.spec_from_file_location("test_draft_queue_tools", SCRIPT)
assert SPEC and SPEC.loader
queue_tools = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(queue_tools)


def bind(path):
    return {"path": str(path), "sha256": sha256(path.read_bytes()).hexdigest()}


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")


class DraftQueueTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.pdf = self.base.joinpath("synthetic-source.pdf")
        self.docx = self.base.joinpath("synthetic-supplement.docx")
        self.pdf.write_bytes(b"Synthetic PDF source bytes bound by the supplied inventory.")
        self.docx.write_bytes(b"Synthetic DOCX source bytes bound by the supplied inventory.")
        self.inventory_path = self.base.joinpath("inventory.json")
        self.queue_path = self.base.joinpath("draft-queue.json")
        self.inventory = {
            "schema_version": 1, "units": [],
            "sources": [
                {"id": "main", "role": "main", **bind(self.pdf), "page_count": 2,
                 "pages": [
                     {"page": 1, "blocks": [
                         {"id": "abstract", "text": "An abstract.", "bbox": [10, 10, 70, 20]},
                         {"id": "body-a", "text": "The paragraph begins", "bbox": [10, 30, 70, 40]},
                         {"id": "header", "text": "A repeating publisher header", "bbox": [10, 1, 70, 9]},
                     ], "image_index": {"page_bounds": [0, 0, 100, 120], "occurrences": [
                         {"occurrence": 1, "image_xref": 6, "bbox": [10, 50, 40, 70]},
                         {"occurrence": 2, "image_xref": 7, "bbox": [40, 50, 70, 70]},
                     ]}},
                     {"page": 2, "blocks": [
                         {"id": "body-b", "text": "and continues on the next page.", "bbox": [10, 10, 70, 20]},
                         {"id": "conclusion", "text": "A conclusion.", "bbox": [10, 30, 70, 40]},
                     ], "image_index": {"page_bounds": [0, 0, 100, 120], "occurrences": []}},
                 ]},
                {"id": "si", "role": "supplement", "format": "docx", **bind(self.docx),
                 "pages": [], "page_count": None,
                 "native_blocks": [
                     {"id": "si-images", "kind": "paragraph", "text": "", "order": 0,
                      "locator": {"part": "word/document.xml", "element_path": [0]}, "media_calls": ["si-m1", "si-m2"]},
                     {"id": "si-caption", "kind": "paragraph", "text": "Two panels of one figure.", "order": 1,
                      "locator": {"part": "word/document.xml", "element_path": [1]}},
                     {"id": "si-table", "kind": "table", "text": "Column A\tColumn B\n1\t2", "order": 2,
                      "locator": {"part": "word/document.xml", "element_path": [2]}},
                 ], "media_calls": [
                     {"id": "si-m1", "block_id": "si-images", "media_id": "asset-1", "relationship_id": "rId8"},
                     {"id": "si-m2", "block_id": "si-images", "media_id": "asset-2", "relationship_id": "rId9"},
                 ], "native_issues": []},
            ],
        }
        save(self.inventory_path, self.inventory)

    def prepare(self):
        queue_tools.prepare_draft_queue(self.inventory_path, self.queue_path, progress=None)
        return queue_tools.DraftQueue.open(self.queue_path)

    def review_pages(self, queue):
        queue.review_pdf_page_order("main", 1,
            ["header", "abstract", "body-a", "main-p1-image-call-1", "main-p1-image-call-2"],
            "The synthetic page is read from the header into the abstract, paragraph and figure.")
        queue.review_additional_objects("main", 1, "The fixture has two raster panel calls and no additional object.")
        queue.review_pdf_page_order("main", 2, ["body-b", "conclusion"],
                                    "The continuing paragraph precedes the conclusion.")
        queue.review_additional_objects("main", 2, "This fixture page contains only the two inventoried text blocks.")

    def complete(self, queue):
        self.review_pages(queue)
        queue.exclude("header", "The synthetic repeated publisher line is outside the article content.")
        units = [
            ("abstract-unit", ["abstract"], "abstract", "## 摘要\n摘要内容。"),
            ("body-unit", ["body-a", "body-b"], "paragraph", "## 1 正文\r\n跨页段落与 $k_{\\Gamma W}$。"),
            ("main-figure", ["main-p1-image-call-1", "main-p1-image-call-2"], "figure", "主文双面板图。"),
            ("conclusion-unit", ["conclusion"], "paragraph", "## 2 结论\n结论内容。"),
            ("si-figure", ["si-images", "si-caption"], "figure", "## 3 补充材料\n两幅媒体构成同一幅补充图。"),
            ("si-table-unit", ["si-table"], "table", "补充表格。"),
        ]
        for unit_id, blocks, kind, text in units:
            queue.group(unit_id, blocks, kind=kind, reason="These exact fixture entries form this single output unit.")
            queue.write_unit(unit_id, text)

    def test_preparation_retains_every_block_and_media_occurrence(self):
        queue = self.prepare()
        self.assertEqual(len(queue.data["blocks"]), 10)
        self.assertEqual(queue.data["blocks"]["si-images"]["text"], "")
        self.assertEqual(len(queue.data["blocks"]["si-images"]["media_calls"]), 2)
        self.assertNotIn("page", queue.data["blocks"]["si-images"])
        self.assertFalse(queue.status()["draft_ready"])
        self.assertEqual(queue.status()["source_fidelity"], "not_assessed")
        self.assertEqual(queue.next_task()["stage"], "inventory_page")
        self.assertEqual(queue.next_task()["page"]["page"], 1)

    def test_only_abstract_and_conclusion_cannot_assemble_full_draft(self):
        queue = self.prepare()
        self.review_pages(queue)
        for name in ("abstract", "conclusion"):
            queue.group(name + "-unit", [name], kind="paragraph", reason="A single selected fixture paragraph.")
            queue.write_unit(name + "-unit", "已写出的部分译文。")
        status = queue.status()
        self.assertIn("body-a", status["unassigned_entries"])
        self.assertIn("body-b", status["unassigned_entries"])
        self.assertIn("si-images", status["unassigned_entries"])
        candidate = self.base.joinpath("candidate.md")
        with self.assertRaisesRegex(ValueError, "Continue drafting"):
            queue.assemble(candidate, progress=None)
        self.assertFalse(candidate.exists())

    def test_cross_page_grouping_and_multiple_media_keep_source_links(self):
        queue = self.prepare()
        self.complete(queue)
        self.assertTrue(queue.status()["draft_ready"])
        self.assertEqual(queue.next_task()["stage"], "assemble")
        candidate = self.base.joinpath("candidate.md")
        report = queue.assemble(candidate, progress=None)
        self.assertEqual([unit["unit_id"] for unit in report["units"]],
                         ["abstract-unit", "body-unit", "main-figure", "conclusion-unit", "si-figure", "si-table-unit"])
        self.assertEqual(report["units"][1]["block_ids"], ["body-a", "body-b"])
        self.assertEqual(report["units"][4]["block_ids"], ["si-images", "si-caption"])
        self.assertIn(b"\r\n", candidate.read_bytes())
        self.assertIn(b"$k_{\\Gamma W}$", candidate.read_bytes())
        candidate_lines = candidate.read_text(encoding="utf-8").splitlines()
        for unit in report["units"]:
            start, end = unit["candidate_lines"]
            fragment = Path(unit["fragment"]["path"]).read_text(encoding="utf-8").rstrip("\n").splitlines()
            self.assertEqual(candidate_lines[start - 1:end], fragment)
        self.assertEqual(report["source_fidelity"], "not_assessed")
        self.assertNotIn("passed", report)

    def test_working_inventory_can_grow_without_invalidating_frozen_queue(self):
        queue = self.prepare()
        snapshot = Path(queue.data["inventory_snapshot"]["path"])
        frozen = snapshot.read_bytes()
        changed = deepcopy(self.inventory)
        changed["units"] = [{"id": "new-semantic-unit"}]
        save(self.inventory_path, changed)
        reopened = queue_tools.DraftQueue.open(self.queue_path)
        self.assertEqual(snapshot.read_bytes(), frozen)
        self.assertEqual(len(reopened.data["blocks"]), 10)
        self.pdf.write_bytes(b"The original source was changed.")
        with self.assertRaisesRegex(ValueError, "Stale source"):
            queue_tools.DraftQueue.open(self.queue_path)

    def test_tampering_cannot_drop_frozen_body_entries(self):
        queue = self.prepare()
        data = deepcopy(queue.data)
        del data["blocks"]["body-a"]
        data["sources"]["main"]["block_ids"].remove("body-a")
        data["pages"][queue_tools._page_key("main", 1)]["block_ids"].remove("body-a")
        save(self.queue_path, data)
        with self.assertRaisesRegex(ValueError, "frozen source entry"):
            queue_tools.DraftQueue.open(self.queue_path)

    def test_native_document_order_cannot_be_replaced_by_media_number(self):
        queue = self.prepare()
        data = deepcopy(queue.data)
        data["sources"]["si"]["block_ids"].reverse()
        save(self.queue_path, data)
        with self.assertRaisesRegex(ValueError, "XML order"):
            queue_tools.DraftQueue.open(self.queue_path)

    def test_no_text_page_still_needs_an_object_inventory(self):
        inventory = deepcopy(self.inventory)
        source = inventory["sources"][0]
        source["pages"] = [{"page": 1, "blocks": [], "image_index": {
            "page_bounds": [0, 0, 100, 120], "occurrences": []}}]
        source["page_count"] = 1
        inventory["sources"] = [source]
        save(self.inventory_path, inventory)
        queue = self.prepare()
        self.assertEqual(queue.next_task()["stage"], "inventory_page")
        queue.review_pdf_page_order("main", 1, [], "No text or raster-image calls occur in this fixture.")
        queue.review_additional_objects("main", 1, "A vector drawing is visible and must be added.")
        self.assertFalse(queue.status()["draft_ready"])
        queue.add_page_object("vector-figure", "main", 1, kind="figure",
                              reason="The visible vector drawing is not a text-layer block.", bbox=[10, 10, 80, 90])
        self.assertEqual(queue.next_task()["stage"], "inventory_page")
        queue.review_pdf_page_order("main", 1, ["vector-figure"], "One visible drawing occupies the page.")
        queue.review_additional_objects("main", 1, "The registered drawing is the only fixture object.")
        queue.group("drawing", ["vector-figure"], kind="figure", reason="The registered drawing is a single source object.")
        queue.write_unit("drawing", "图件及其说明。")
        self.assertTrue(queue.status()["draft_ready"])
        queue.assemble(self.base.joinpath("drawing.md"), progress=None)

    def test_immutable_revisions_and_asset_changes_are_checked(self):
        queue = self.prepare()
        self.complete(queue)
        old = deepcopy(queue.data["units"]["body-unit"]["revisions"][-1])
        old_bytes = Path(old["path"]).read_bytes()
        asset = self.base.joinpath("two-panels.png")
        asset.write_bytes(b"Two source-backed panels.")
        queue.write_unit("body-unit", "修订后的完整段落。", assets=[asset])
        self.assertEqual(Path(old["path"]).read_bytes(), old_bytes)
        self.assertEqual(len(queue.data["units"]["body-unit"]["revisions"]), 2)
        asset.write_bytes(b"A later incompatible asset.")
        candidate = self.base.joinpath("candidate.md")
        with self.assertRaisesRegex(ValueError, "Stale source"):
            queue.assemble(candidate, progress=None)
        self.assertFalse(candidate.exists())

    def test_native_issue_must_have_explicit_disposition(self):
        self.inventory["sources"][1]["native_issues"] = [
            {"code": "EXTERNAL_IMAGE", "reason": "The fixture image uses an external relationship."}]
        save(self.inventory_path, self.inventory)
        queue = self.prepare()
        self.complete(queue)
        task = queue.next_task()
        self.assertEqual(task["stage"], "source_issue")
        issue_id = task["issue_id"]
        queue.record_issue_decision(issue_id, status="blocked", reason="The external resource remains unavailable.",
                                    evidence_paths=[self.docx])
        self.assertFalse(queue.status()["draft_ready"])
        queue.record_issue_decision(issue_id, status="resolved", reason="The fixture resource has been reconciled in the recorded comparison.",
                                    evidence_paths=[self.docx])
        self.assertTrue(queue.status()["draft_ready"])

    def test_added_page_object_rejects_nonfinite_and_outside_coordinates(self):
        queue = self.prepare()
        before = self.queue_path.read_bytes()
        invalid = [
            [float("nan"), 0, 50, 50], [0, 0, float("inf"), 50],
            [float("-inf"), 0, 50, 50], [-1, 0, 50, 50],
            [0, 0, 101, 50], [0, 0, 50, 121], [40, 10, 20, 50],
        ]
        for bbox in invalid:
            with self.subTest(bbox=bbox), self.assertRaises(ValueError):
                queue.add_page_object("invalid-object", "main", 1, kind="figure",
                                      reason="This fixture box is invalid.", bbox=bbox)
            self.assertEqual(self.queue_path.read_bytes(), before)
            self.assertNotIn("invalid-object", queue.data["blocks"])
        queue.add_page_object("page-frame", "main", 1, kind="figure",
                              reason="The whole actual page rectangle is valid.", bbox=[0, 0, 100, 120])
        self.assertEqual(queue.data["blocks"]["page-frame"]["bbox"], [0, 0, 100, 120])

    def test_native_issue_evidence_is_revalidated_before_assembly(self):
        self.inventory["sources"][1]["native_issues"] = [
            {"code": "EXTERNAL_IMAGE", "reason": "An external image needs an explicit disposition."}]
        save(self.inventory_path, self.inventory)
        queue = self.prepare()
        self.complete(queue)
        evidence = self.base.joinpath("native-image-comparison.txt")
        evidence.write_text("Actual fixture comparison before the resolution.", encoding="utf-8")
        issue_id = queue.next_task()["issue_id"]
        queue.record_issue_decision(issue_id, status="resolved", reason="The recorded comparison resolves this fixture issue.",
                                    evidence_paths=[evidence])
        self.assertTrue(queue.status()["draft_ready"])
        evidence.write_text("This later text is different evidence.", encoding="utf-8")
        candidate = self.base.joinpath("candidate.md")
        with self.assertRaisesRegex(ValueError, "Stale source binding"):
            queue.assemble(candidate, progress=None)
        self.assertFalse(candidate.exists())

    def test_invalid_grouping_and_exclusion_leave_saved_queue_unchanged(self):
        queue = self.prepare()
        before = self.queue_path.read_bytes()
        for blocks in (["abstract", "abstract"], ["missing"], ["abstract", "si-caption"]):
            with self.subTest(blocks=blocks), self.assertRaises(ValueError):
                queue.group("invalid", blocks, kind="paragraph", reason="Invalid fixture grouping.")
            self.assertEqual(self.queue_path.read_bytes(), before)
        with self.assertRaises(ValueError):
            queue.exclude("header", " ")
        self.assertEqual(self.queue_path.read_bytes(), before)
        queue.group("first", ["abstract"], kind="paragraph", reason="One exact source block.")
        with self.assertRaises(ValueError):
            queue.exclude("abstract", "This already belongs to a draft unit.")

    def test_stale_script_cannot_write_over_a_newer_queue(self):
        first = self.prepare()
        second = queue_tools.DraftQueue.open(self.queue_path)
        first.group("paragraph", ["abstract"], kind="paragraph", reason="The source abstract is a single unit.")
        with self.assertRaisesRegex(RuntimeError, "another script"):
            second.exclude("header", "A repeated publisher header.")
        self.assertIn("paragraph", queue_tools.DraftQueue.open(self.queue_path).data["units"])

    def test_snapshot_and_previous_outputs_are_not_overwritten(self):
        queue = self.prepare()
        with self.assertRaises(FileExistsError):
            queue_tools.prepare_draft_queue(self.inventory_path, self.queue_path, progress=None)
        self.complete(queue)
        candidate = self.base.joinpath("candidate.md")
        queue.assemble(candidate, progress=None)
        previous = candidate.read_bytes()
        with self.assertRaises(FileExistsError):
            queue.assemble(candidate, progress=None)
        self.assertEqual(candidate.read_bytes(), previous)


if __name__ == "__main__":
    unittest.main()
