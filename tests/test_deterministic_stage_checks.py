"""File-based stage regressions; synthetic inputs only."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT.joinpath("literature-zotero2obsidian", "scripts")))
import deterministic_stage_checks as stage


class DeterministicStageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def file(self, name, payload):
        path = self.root.joinpath(name)
        path.write_bytes(payload)
        return path

    def test_new_properties_roundtrip_and_body_preserved(self):
        raw = b"\xef\xbb\xbf# Title\r\n\r\n---\r\nText\r\n"
        fields = {"类型": "期刊论文", "年份": 2024, "关键词": ["a: b", "a # b", "2020"], "期刊名": "J: X"}
        output = stage.build_properties(raw, item_type="journalArticle", confirmed=fields)
        self.assertEqual(stage.split_frontmatter(output)[1], raw[3:])
        report = stage.audit_properties(self.file("in.md", raw), self.file("out.md", output),
                                        mode="new", item_type="journalArticle")
        self.assertTrue(all(r["status"] == "passed" for r in report["checks"].values()))
        self.assertEqual(report["source_fidelity"], "not_assessed")
        with self.assertRaises(ValueError):
            stage.build_properties(output, item_type="journalArticle", confirmed=fields)

    def test_invalid_new_fields(self):
        for year in (True, "2024", -1):
            with self.subTest(year=year), self.assertRaises(ValueError):
                stage.build_properties(b"# A\n", item_type="thesis",
                    confirmed={"类型": "学位论文", "年份": year, "关键词": []})
        with self.assertRaises(ValueError):
            stage.build_properties(b"# A", item_type="thesis",
                confirmed={"类型": "学位论文", "年份": 2020, "关键词": [], "路径": "x"})

    def test_repair_exact_custom_values_and_crlf(self):
        source = ("---\r\n类型: '期刊论文'\r\n库内参考文献:\r\n"
                  "  - 'old'\r\ncustom: \"2020\"\r\n"
                  "nested:\r\n  库内参考文献: retain\r\n---\r\n# Title\r\n").encode()
        expected = source.replace("库内参考文献:\r\n  - 'old'\r\n".encode(), b"", 1)
        actual = stage.repair_properties(source)
        self.assertEqual(actual, expected)
        self.assertEqual(stage.repair_properties(actual), actual)
        report = stage.audit_properties(self.file("before.md", source), self.file("after.md", actual),
                                        mode="repair", item_type="journalArticle")
        self.assertEqual(report["checks"]["PROP-C18"]["status"], "passed")

    def test_repair_null_only_key(self):
        source = "---\n库内参考文献: null\n---\n# T\n".encode()
        self.assertEqual(stage.repair_properties(source), b"---\n{}\n---\n# T\n")

    def test_duplicate_keys_blocked(self):
        data = b"---\nx: 1\nx: 2\n---\n# T\n"
        with self.assertRaises(Exception):
            stage.repair_properties(data)
        report = stage.audit_properties(self.file("a.md", b"# T\n"), self.file("b.md", data),
                                        mode="new", item_type="thesis")
        self.assertEqual(report["checks"]["PROP-A02"]["status"], "failed")
        self.assertEqual(report["checks"]["PROP-A04"]["status"], "not_verified")

    def test_alias_and_flow_mapping_fail_closed(self):
        for source in ("---\n库内参考文献: &a [1]\ncustom: *a\n---\n# T\n",
                       "---\n{库内参考文献: [1], custom: x}\n---\n# T\n"):
            with self.subTest(source=source), self.assertRaises(Exception):
                stage.repair_properties(source.encode())

    def test_unclosed_frontmatter_and_body_difference(self):
        with self.assertRaises(ValueError):
            stage.split_frontmatter(b"---\nx: 1\n")
        report = stage.audit_properties(self.file("one.md", b"# T\n"),
                   self.file("two.md", "---\n类型: x\n年份: null\n关键词: []\n---\n# Changed\n".encode()),
                   mode="new", item_type="thesis")
        self.assertEqual(report["checks"]["PROP-A08"]["status"], "failed")

    def test_author_count_and_unicode(self):
        self.assertEqual(stage.author_token(["欧阳明"]), "欧阳明")
        self.assertEqual(stage.author_token(["van der Waals", "Smith Jr."]), "van der Waals and Smith Jr.")
        self.assertEqual(stage.author_token(["United Nations", "B", "C"]), "United Nations et al")
        self.assertEqual(stage.author_token([], unknown_reason="source missing"), "未知作者")
        for invalid in ("Smith", [], [" A"], ["A\nB"]):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                stage.author_token(invalid)

    def test_collection_chains_and_incomplete(self):
        library = {"type": "user", "id": 123}
        item = {"library": library, "data": {"collections": ["B"]}}
        a = {"key": "A", "library": library, "data": {"key": "A", "name": "Root", "parentCollection": False}}
        b = {"key": "B", "library": library, "data": {"key": "B", "name": "Child", "parentCollection": "A"}}
        output = stage.collection_ancestry(item, [b, a], pagination_complete=True)
        self.assertEqual(output["status"], "complete")
        self.assertEqual([x["key"] for x in output["collections"][0]["ancestry"]], ["A", "B"])
        self.assertEqual(stage.collection_ancestry(item, [b], pagination_complete=True)["status"], "incomplete")
        self.assertEqual(stage.collection_ancestry(item, [b, a], pagination_complete=False)["status"], "incomplete")
        a["data"]["parentCollection"] = "B"
        self.assertEqual(stage.collection_ancestry(item, [b, a], pagination_complete=True)["issues"][0]["reason"], "collection_cycle")
        with self.assertRaises(ValueError):
            stage.collection_ancestry(item, [b, b], pagination_complete=True)

    def test_collection_wrong_library(self):
        item = {"library": {"type": "user", "id": 1}, "data": {"collections": ["A"]}}
        record = {"library": {"type": "group", "id": 1}, "key": "A",
                  "data": {"name": "Wrong", "parentCollection": False}}
        self.assertEqual(stage.collection_ancestry(item, [record], pagination_complete=True)["status"], "incomplete")

    def test_resource_actual_read_and_traversal(self):
        self.file("x.md", b"hello")
        self.file("empty.md", b"")
        rows = stage.inspect_relative_files(self.root, ["x.md", "../x.md", "missing.md", "empty.md", "x.md", "C:/x.md", "./x.md"])
        self.assertEqual([r["status"] for r in rows], ["readable", "blocked", "blocked", "empty", "readable", "blocked", "blocked"])
        self.assertEqual(rows[0]["sha256"], rows[4]["sha256"])

    def test_stage_unchanged_yaml_and_readback(self):
        a = self.file("a.md", b"---\ncustom: 1\n---\n# A\n")
        b = self.file("b.md", b"---\ncustom: 1\n---\n# B\n")
        c = self.file("c.md", b"---\ncustom: 2\n---\n# A\n")
        self.assertEqual(stage.audit_unchanged_properties(a, b, stage="header")["checks"]["HDR-A14"]["status"], "passed")
        self.assertEqual(stage.audit_unchanged_properties(a, c, stage="library")["checks"]["LIB-A08"]["status"], "failed")
        self.assertEqual(stage.audit_write_readback(a, b)["checks"]["WRITE-A04"]["status"], "failed")
        d = self.file("d.md", a.read_bytes())
        self.assertEqual(stage.audit_write_readback(a, d)["checks"]["WRITE-A04"]["status"], "passed")
        with self.assertRaises(ValueError):
            stage.audit_write_readback(a, a)

    def test_translation_independent_file_identity_and_hash(self):
        actual = self.file("glossary.md", b"terms")
        other = self.file("copy.md", b"terms")
        binding = stage.snapshot(actual)
        record = self.file("term.json", json.dumps({"glossary": binding}).encode())
        result = stage.audit_translation_binding(record, actual, kind="glossary")
        self.assertEqual(result["checks"]["TRN-A17"]["status"], "passed")
        self.assertEqual(stage.audit_translation_binding(record, other, kind="glossary")["checks"]["TRN-A17"]["status"], "failed")
        actual.write_bytes(b"changed")
        self.assertEqual(stage.audit_translation_binding(record, actual, kind="glossary")["checks"]["TRN-A17"]["status"], "failed")

    def test_mapping_ids_are_real(self):
        report = stage.coverage_catalog(ROOT)
        self.assertEqual(report["unknown_mapping_ids"], [])
        self.assertGreater(report["counts"]["not_classified_by_this_module"], 0)
        self.assertEqual(report["counts"]["mapped"], len(stage.RULE_HANDLERS))


    def test_rich_tables_use_real_fences_and_per_sheet_results(self):
        fence = chr(96) * 3
        fake = fence + "text\n" + fence + "rich-table\n" + fence + "\n"
        real = (fence + "rich-table\nversion: 3\nsheets:\n"
                "  - locked: true\n    theme: grid\n    columns: [a]\n    rows: [b]\n"
                "  - locked: false\n    theme: grid\n    columns: [a]\n    rows: [b]\n"
                "    footer: 123\n" + fence + "\n^tbl-1\n")
        report = stage.audit_rich_tables(self.file("table.md", (fake + real).encode()))
        self.assertEqual(len(report["tables"]), 1)
        self.assertEqual(report["checks"]["BODY-A83"]["status"], "passed")
        self.assertEqual(report["checks"]["BODY-A84"]["status"], "failed")
        self.assertEqual(report["checks"]["BODY-A85"]["status"], "passed")
        self.assertEqual(report["checks"]["BODY-A111"]["status"], "failed")

    def test_rich_table_absence_not_verified(self):
        report = stage.audit_rich_tables(self.file("no-table.md", b"# Heading\n"))
        self.assertTrue(all(row["status"] == "not_verified" for row in report["checks"].values()))

    def test_file_checks_dispatch_rejects_arbitrary_report(self):
        with self.assertRaises(TypeError):
            stage.record_file_checks(self.root.joinpath("ledger.json"),
                                     self.root.joinpath("report.json"),
                                     {"checks": {"PROP-A01": {"status": "passed"}}})


if __name__ == "__main__":
    unittest.main()
