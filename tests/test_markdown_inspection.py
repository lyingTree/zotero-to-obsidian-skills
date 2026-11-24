from pathlib import Path
from hashlib import sha256
import tempfile
import unittest

from PIL import Image

from evals.inspect_markdown import inspect_markdown


class MarkdownInspectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.note = self.root.joinpath("candidate.md")

    def test_inline_bibliography_anchors_and_actual_assets_are_retained(self):
        Image.new("RGB", (3, 4), "white").save(self.root.joinpath("figure.png"))
        self.note.write_text(
            "## 1 参考文献\n1. Alpha, 2020. A study. ^ref-1\n\n"
            "![](figure.png)\n![[missing.png]]\n", encoding="utf-8")
        result = inspect_markdown(self.note, self.root)
        self.assertEqual(len(result["references"]), 1)
        self.assertEqual(result["anchors"][0]["id"], "ref-1")
        self.assertEqual(result["image_references"][0]["width"], 3)
        self.assertFalse(result["image_references"][1]["exists"])
        self.assertTrue(result["acceptance"].startswith("not_assessed"))

    def test_paired_yaml_delimiters_and_math_are_preserved(self):
        self.note.write_text(
            "```rich-table\n---\nversion: 2\nlocked: true\ntheme: grid\n"
            "columns: []\nrows: []\n---\n```\n^tbl-1\n\n"
            "$$x=y\\tag{1}$$\n^eq-1\n", encoding="utf-8")
        result = inspect_markdown(self.note, self.root)
        self.assertTrue(result["rich_tables"][0]["data"]["locked"])
        self.assertEqual(result["display_equations"][0]["text"], "$$x=y\\tag{1}$$")

    def test_reference_to_outside_resource_root_is_not_read(self):
        self.note.write_text("![](../private.png)\n", encoding="utf-8")
        result = inspect_markdown(self.note, self.root)
        self.assertTrue(result["image_references"][0]["outside_resource_root"])
        self.assertNotIn("sha256", result["image_references"][0])

    def test_crlf_wrapped_table_is_parsed_without_rewriting_candidate(self):
        raw = ("```rich-table\r\n---\r\nversion: 2\r\nlocked: true\r\n"
               "theme: grid\r\ncolumns: []\r\nrows: []\r\n---\r\n```\r\n"
               "^tbl-1\r\n\r\n$$\r\nx=y\\tag{1}\r\n$$\r\n^eq-1\r\n").encode()
        self.note.write_bytes(raw)
        result = inspect_markdown(self.note, self.root)
        self.assertTrue(result["rich_tables"][0]["data"]["locked"])
        self.assertEqual(result["display_equations"][0]["text"], "$$\r\nx=y\\tag{1}\r\n$$")
        self.assertEqual(result["sha256"], sha256(raw).hexdigest())
        self.assertEqual(self.note.read_bytes(), raw)

    def test_parseable_tables_retain_block_and_flow_style_evidence(self):
        self.note.write_text(
            '```rich-table\n{"version": 2, "columns": [], "rows": []}\n```\n\n'
            '```rich-table\nversion: 3\nsheets:\n'
            '  - columns:\n      - id: c1\n        name: Value\n'
            '    rows:\n      - id: r1\n        cells: {c1: 1}\n'
            '  - columns: [{id: c2, name: Value}]\n'
            '    rows: [{id: r2, cells: {c2: 2}}]\n```\n', encoding="utf-8")
        result = inspect_markdown(self.note, self.root)
        tables = result["rich_tables"]
        self.assertEqual(tables[0]["collection_styles"], [
            {"path": "columns", "style": "flow"}, {"path": "rows", "style": "flow"}])
        self.assertEqual(tables[1]["collection_styles"], [
            {"path": "sheets[0].columns", "style": "block"},
            {"path": "sheets[0].rows", "style": "block"},
            {"path": "sheets[1].columns", "style": "flow"},
            {"path": "sheets[1].rows", "style": "flow"}])
        self.assertTrue(result["acceptance"].startswith("not_assessed"))


if __name__ == "__main__":
    unittest.main()
