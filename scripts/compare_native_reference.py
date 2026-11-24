"""Development-only differential tests; the Rust executable never runs Python."""
from __future__ import annotations

import base64
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT.joinpath("literature-zotero2obsidian", "scripts")))
import deterministic_stage_checks as reference
import format_helpers as formatting
import source_url_inventory as urls

BINARY = ROOT.joinpath("native", "target", "debug", "z2o.exe" if sys.platform == "win32" else "z2o")


def encoded(data: bytes) -> dict:
    return {"$bytes": base64.b64encode(data).decode("ascii")}


def statuses(report: dict) -> dict:
    return {key: row["status"] for key, row in report["checks"].items()}


class ReferenceComparison(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def invoke(self, operation, *args, **kwargs):
        request = self.root.joinpath("request.json")
        request.write_text(json.dumps({"schema_version": 1, "operation": operation,
                                      "args": args, "kwargs": kwargs}, ensure_ascii=False), encoding="utf-8")
        completed = subprocess.run([str(BINARY), "invoke", str(request)], check=True,
                                   capture_output=True, encoding="utf-8", timeout=45)
        return json.loads(completed.stdout)["result"]

    def test_author_and_collection(self):
        for markers in (["欧阳"], ["van der Waals", "Smith"], ["联合国", "A", "B"]):
            self.assertEqual(self.invoke("deterministic_stage_checks.author_token", markers),
                             reference.author_token(markers))
        library = {"type": "group", "id": 123}
        item = {"library": library, "data": {"collections": ["B"]}}
        records = [
            {"key": "A", "library": library, "data": {"key": "A", "name": "Root", "parentCollection": False}},
            {"key": "B", "library": library, "data": {"key": "B", "name": "Child", "parentCollection": "A"}},
        ]
        self.assertEqual(self.invoke("deterministic_stage_checks.collection_ancestry", item, records,
                                     pagination_complete=True),
                         reference.collection_ancestry(item, records, pagination_complete=True))

    def test_file_binding(self):
        source = self.root.joinpath("论文.md")
        source.write_bytes(b"abc")
        self.assertEqual(self.invoke("deterministic_stage_checks.snapshot", str(source)),
                         reference.snapshot(source))
        self.assertEqual(self.invoke("deterministic_stage_checks.verify_binding", reference.snapshot(source),
                                     str(source), base=str(self.root)),
                         reference.verify_binding(reference.snapshot(source), source, base=self.root))

    def test_properties_byte_output_and_audit(self):
        original = b"\xef\xbb\xbf# Title\r\n\r\n---\r\nText\r\n"
        fields = {"类型": "期刊论文", "年份": 2024, "关键词": ["a: b", "a # b", "2020"], "期刊名": "J: X"}
        expected = reference.build_properties(original, item_type="journalArticle", confirmed=fields)
        self.assertEqual(self.invoke("deterministic_stage_checks.build_properties", encoded(original),
                                     item_type="journalArticle", confirmed=fields), encoded(expected))
        old = ("---\r\n类型: '期刊论文'\r\n库内参考文献:\r\n"
               "  - 'old'\r\ncustom: \"2020\"\r\n"
               "nested:\r\n  库内参考文献: retain\r\n---\r\n# Title\r\n").encode()
        self.assertEqual(self.invoke("deterministic_stage_checks.repair_properties", encoded(old)),
                         encoded(reference.repair_properties(old)))
        before, after = self.root.joinpath("before.md"), self.root.joinpath("after.md")
        before.write_bytes(original)
        after.write_bytes(expected)
        self.assertEqual(statuses(self.invoke("deterministic_stage_checks.audit_properties",
                                            str(before), str(after), mode="new", item_type="journalArticle")),
                         statuses(reference.audit_properties(before, after, mode="new", item_type="journalArticle")))

    def test_naming_plan_complete_mapping(self):
        source = self.root.joinpath("source.txt")
        source.write_text("Synthetic source", encoding="utf-8")
        file = reference.snapshot(source)
        evidence = {key: {"file": file, "locator": "line 1"}
                    for key in ("year", "author_token", "original_title")}
        fields = dict(year=2024, author_token="Smith", original_title="a:🌊🌊🌊",
                      source_evidence=evidence, max_utf16_units=17)
        expected = formatting.build_naming_plan(**fields, target_directory=self.root)
        actual = self.invoke("format_helpers.build_naming_plan", **fields, target_directory=str(self.root))
        self.assertEqual(actual, expected)

    def test_figure_caption_and_anchor(self):
        images = [{"target": "a#b.png", "alt": "[A]"}]
        fields = dict(caption="**图1：** 内容", anchor="fig-1", newline="\r\n")
        self.assertEqual(self.invoke("format_helpers.build_figure_block", images, **fields),
                         formatting.build_figure_block(images, **fields))
        self.assertEqual(self.invoke("format_helpers.plan_supplement_figure_caption", "7c", "内容"),
                         formatting.plan_supplement_figure_caption("7c", "内容"))
        self.assertEqual(self.invoke("format_helpers.new_figure_table_anchor",
                                     source_token="ab12", figure_number_token="S7c"),
                         formatting.new_figure_table_anchor(source_token="ab12", figure_number_token="S7c"))

    def test_table_checks(self):
        fence = chr(96) * 3
        note = self.root.joinpath("table.md")
        note.write_text(fence + "rich-table\nversion: 3\nsheets:\n"
                        "  - locked: true\n    theme: grid\n    columns: [a]\n    rows: [b]\n"
                        "  - locked: false\n    theme: grid\n    columns: [a]\n    rows: [b]\n"
                        "    footer: 123\n" + fence + "\n^tbl-1\n", encoding="utf-8")
        self.assertEqual(statuses(self.invoke("deterministic_stage_checks.audit_rich_tables", str(note))),
                         statuses(reference.audit_rich_tables(note)))


    def test_source_url_occurrence_assignment(self):
        value = "https://example.org/Fn_(x)"
        inventory = {"sources": [{"id": "main", "pages": [{"page": 1, "blocks": [
            {"id": "b1", "text": "中文 🌊 (" + value + ")."},
            {"id": "b2", "text": value},
        ]}]}], "units": [
            {"id": "broad", "kind": "metadata", "field": "external_url", "source": "main",
             "value": value, "evidence": [{"page": 1}]},
            {"id": "narrow", "kind": "metadata", "field": "external_url", "source": "main",
             "value": value, "evidence": [{"page": 1, "blocks": ["b1"]}]},
        ]}
        self.assertEqual(self.invoke("source_url_inventory.source_url_coverage", inventory),
                         urls.source_url_coverage(inventory))
        self.assertEqual(self.invoke("source_url_inventory.scan_source_url_occurrences", inventory,
                                     excluded_blocks=["b2"]),
                         urls.scan_source_url_occurrences(inventory, excluded_blocks=["b2"]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
