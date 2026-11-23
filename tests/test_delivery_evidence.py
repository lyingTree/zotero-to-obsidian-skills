"""Evidence-chain regressions using generated PDFs, not a known paper's answer."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-zotero2obsidian", "scripts", "validate_delivery.py")
SPEC = importlib.util.spec_from_file_location("test_delivery_validator", SCRIPT)
assert SPEC and SPEC.loader
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)

TABLE = """```rich-table
---
version: 2
locked: true
theme: grid
columns:
  - id: a
    name: 分类
rows:
  - id: r1
    cells:
      a: B
---
```
^tbl-1"""

FIGURE = "![[figure.png]]\n图1：来源中的示意图。 ^fig-1"
CITATION = "完整段落的引文[1](<#^ref-1>)。"


class DeliveryEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)
        self.pdf = self.base.joinpath("source.pdf")
        document = fitz.open()
        page = document.new_page(width=300, height=420)
        page.insert_text((25, 35), "Example study")
        page.insert_text((25, 55), "Author One 1")
        page.insert_text((25, 75), "1 Institute X")
        page.insert_text((25, 95), "Published 2025-01-01")
        page.insert_text((25, 125), "A complete paragraph cites reference [1].")
        page.insert_text((25, 145), "E = m c^2")
        page.insert_text((25, 165), "Table: category B")
        page.draw_rect(fitz.Rect(25, 180, 160, 220))
        page.insert_text((25, 245), "1. Author. A source. 2024.")
        document.save(self.pdf)
        document.close()
        self.inventory_path = checker.source_tools.prepare_sources(
            {"main": self.pdf}, self.base.joinpath("work"), source_roles={"main": "main"}, progress=None)
        self.inventory = json.loads(self.inventory_path.read_text(encoding="utf-8"))
        self.source = self.inventory["sources"][0]
        blocks = self.source["pages"][0]["blocks"]
        self.evidence = {"page": 1, "bbox": [0, 0, 300, 420], "basis": "visual",
                         "blocks": [block["id"] for block in blocks]}
        self.units = []
        self.deliveries = []
        headers = [("title", "Example study", "Example study"),
                   ("author", "Author One 1", "Author One<sup>1</sup>"),
                   ("affiliation", "1 Institute X", "<sup>1</sup>Institute X"),
                   ("publication_date", "Published 2025-01-01", "发表于2025年1月1日。")]
        for field, original, target in headers:
            identity = "main-" + field
            self.units.append({"id": identity, "kind": "metadata", "source": "main", "field": field,
                               "evidence": [{**self.evidence, "quote": original}]})
            self.deliveries.append({"unit": identity, "target": {"quote": target}})
        self.source["header_fields"] = {
            name: {"status": "present", "units": ["main-" + field]} for name, field in checker.FIELDS.items()
        }
        for identity, kind, extra, target in [
            ("paragraph", "paragraph", {}, {"quote": CITATION}),
            ("citation", "citation", {"references": ["reference"]}, {"quote": CITATION}),
            ("reference", "reference", {"number": "1"}, {"anchor": "ref-1"}),
            ("formula", "formula", {}, {"anchor": "eq-1"}),
            ("table", "table", {"shape": {"rows": 1, "columns": 1}, "merges": []}, {"anchor": "tbl-1"}),
            ("figure", "figure", {"contains_table": False}, {"quote": FIGURE}),
        ]:
            self.units.append({"id": identity, "kind": kind, "source": "main", "evidence": [self.evidence], **extra})
            self.deliveries.append({"unit": identity, "target": target})
        self.note = self.base.joinpath("candidate.md")
        self.note.write_text("---\n类型: 期刊论文\n年份: 2025\n关键词: []\n期刊名: Example\n---\n"
                             "# Example study\n\nAuthor One<sup>1</sup>\n\n<sup>1</sup>Institute X\n\n"
                             "发表于2025年1月1日。\n\n## 1 正文\n\n" + CITATION + "\n\n"
                             "$$\nE = mc^2\n$$\n^eq-1\n\n" + FIGURE + "\n\n" + TABLE + "\n\n"
                             "## 2 参考文献\n\n1. Author. A source. 2024. ^ref-1\n", encoding="utf-8")
        with fitz.open(self.pdf) as document:
            document[0].get_pixmap().save(self.base.joinpath("figure.png"))
        self.deliveries[-1]["assets"] = [{"path": "figure.png"}]
        self.inventory["units"] = self.units
        self.manifest_path = self.base.joinpath("delivery-manifest.json")
        self.resource_root = None

    def bind(self):
        self.inventory_path.write_text(json.dumps(self.inventory), encoding="utf-8")
        checker.write_delivery_manifest(self.inventory_path, self.note, self.deliveries, self.manifest_path,
                                        resource_root=self.resource_root)

    def codes(self):
        self.bind()
        return {item.code for item in checker.validate_delivery(self.manifest_path).findings}

    def test_consistent_evidence_never_claims_semantic_acceptance(self):
        self.bind()
        report = checker.validate_delivery(self.manifest_path)
        self.assertEqual(report.findings, [])
        self.assertTrue(report.structurally_consistent)
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertTrue(report.independent_review_required)

    def test_unmapped_source_object_is_not_a_static_zero_error_pass(self):
        self.deliveries = [item for item in self.deliveries if item["unit"] != "formula"]
        self.assertIn("UNIT_NOT_DELIVERED", self.codes())

    def test_self_declared_pass_without_target_is_rejected(self):
        self.deliveries[0] = {"unit": "main-title", "status": "passed"}
        self.assertIn("TARGET_REQUIRED", self.codes())

    def test_text_layer_is_not_visible_date_evidence(self):
        date = next(unit for unit in self.units if unit.get("field") == "publication_date")
        date["evidence"] = [{"page": 1, "quote": "Published 2025-01-01", "basis": "text"}]
        self.assertIn("VISIBLE_PAGE_REQUIRED", self.codes())

    def test_render_from_wrong_page_is_rejected_even_with_new_file_hash(self):
        second = fitz.open()
        page = second.new_page(width=300, height=420)
        page.insert_text((25, 35), "Wrong page")
        record = self.source["pages"][0]["render"]
        path = Path(record["path"])
        pixels = page.get_pixmap(dpi=record["dpi"], colorspace=fitz.csRGB, alpha=False)
        pixels.save(path)
        record.update(checker.source_tools.file_record(path))
        record["pixel_sha256"] = hashlib.sha256(pixels.samples).hexdigest()
        second.close()
        self.assertIn("RENDER_PROVENANCE", self.codes())

    def test_candidate_change_after_binding_is_rejected(self):
        self.bind()
        self.note.write_text(self.note.read_text(encoding="utf-8") + "\n变化。\n", encoding="utf-8")
        report = checker.validate_delivery(self.manifest_path)
        self.assertIn("FILE_HASH_MISMATCH", {item.code for item in report.findings})

    def test_dropped_text_block_is_detected(self):
        omitted = self.evidence["blocks"][0]
        for unit in self.units:
            unit["evidence"] = [{**item, "blocks": [key for key in item.get("blocks", []) if key != omitted]}
                                for item in unit["evidence"]]
        self.assertIn("SOURCE_BLOCK_UNMAPPED", self.codes())

    def test_foreign_source_block_cannot_prove_coverage(self):
        self.units[0]["evidence"] = [{**self.evidence, "blocks": ["supplement-p1-b0"]}]
        self.assertIn("EVIDENCE_BLOCK_SOURCE", self.codes())

    def test_source_page_count_is_checked_against_pdf(self):
        self.source["page_count"] = 2
        self.assertIn("SOURCE_PAGE_COUNT", self.codes())

    def test_pending_si_header_cannot_be_skipped(self):
        self.source["header_fields"]["affiliations"] = {"status": "pending"}
        self.assertIn("HEADER_PENDING", self.codes())

    def test_absence_needs_visible_source_evidence(self):
        self.source["header_fields"]["authors"] = {"status": "absent_in_source", "reason": "No author line"}
        self.assertIn("SOURCE_EVIDENCE_REQUIRED", self.codes())

    def test_figure_table_requires_separate_editable_unit(self):
        figure = next(item for item in self.units if item["id"] == "figure")
        figure["contains_table"] = True
        self.assertIn("EMBEDDED_TABLE_MISSING", self.codes())
        next(item for item in self.units if item["id"] == "table")["parent_id"] = "figure"
        self.assertNotIn("EMBEDDED_TABLE_MISSING", self.codes())

    def test_table_shape_checks_source_inventory_against_target(self):
        next(item for item in self.units if item["id"] == "table")["shape"]["rows"] = 2
        self.assertIn("TABLE_ROWS", self.codes())

    def test_second_worksheet_is_not_silently_replaced_by_active_sheet(self):
        new_table = """```rich-table
version: 3
sheets:
  - id: first
    locked: true
    theme: grid
    columns: [{id: a, name: 名称}]
    rows: [{id: r1, cells: {a: B}}]
  - id: second
    locked: false
    theme: plain
    columns: [{id: a, name: 名称}]
    rows: [{id: r1, cells: {a: '[断链](#^absent)'}}]
```
^tbl-1"""
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, new_table), encoding="utf-8")
        next(item for item in self.deliveries if item["unit"] == "table")["sheet"] = {"id": "second"}
        codes = self.codes()
        self.assertIn("NOTE_TABLE_UNLOCKED", codes)
        self.assertIn("NOTE_TABLE_THEME", codes)
        self.assertIn("NOTE_MISSING_ANCHOR", codes)

    def test_citation_must_reach_declared_reference(self):
        next(item for item in self.units if item["id"] == "citation")["references"] = ["missing-reference"]
        codes = self.codes()
        self.assertIn("CITATION_REFERENCE_TARGET", codes)
        self.assertIn("CITATION_MAPPING", codes)

    def test_unmapped_output_citation_is_detected(self):
        self.units[:] = [item for item in self.units if item["id"] != "citation"]
        self.deliveries[:] = [item for item in self.deliveries if item["unit"] != "citation"]
        self.assertIn("OUTPUT_CITATION_UNMAPPED", self.codes())

    def test_broken_local_image_path_is_detected(self):
        self.base.joinpath("figure.png").rename(self.base.joinpath("moved.png"))
        next(item for item in self.deliveries if item["unit"] == "figure")["assets"] = [{"path": "moved.png"}]
        self.assertIn("FIGURE_ASSET_MISSING", self.codes())

    def test_hidden_quote_cannot_prove_title_delivery(self):
        text = self.note.read_text(encoding="utf-8").replace("# Example study", "<!-- Example study -->")
        self.note.write_text(text, encoding="utf-8")
        self.assertIn("TARGET_HIDDEN", self.codes())

    def test_new_inventory_never_overwrites_old_evidence(self):
        with self.assertRaises(FileExistsError):
            checker.source_tools.prepare_sources({"main": self.pdf}, self.inventory_path.parent, progress=None)

    def test_figure_anchor_resolves_preceding_image_with_caption(self):
        next(item for item in self.deliveries if item["unit"] == "figure")["target"] = {"anchor": "fig-1"}
        self.assertEqual(self.codes(), set())

    def test_justified_header_absence_has_a_compatible_schema(self):
        pdf = self.base.joinpath("supplement.pdf")
        document = fitz.open()
        page = document.new_page(width=300, height=420)
        page.insert_text((25, 35), "Supplement")
        page.insert_text((25, 75), "A supplemental paragraph.")
        document.save(pdf)
        document.close()
        prepared = checker.source_tools.prepare_sources(
            {"si": pdf}, self.base.joinpath("si-work"), source_roles={"si": "supplement"}, progress=None)
        source = json.loads(prepared.read_text(encoding="utf-8"))["sources"][0]
        evidence = {"page": 1, "basis": "visual", "bbox": [0, 0, 300, 420],
                    "blocks": [row["id"] for row in source["pages"][0]["blocks"]]}
        source["header_fields"] = {
            "title": {"status": "present", "units": ["si-title"]},
            **{field: {"status": "absent_in_source", "reason": "This source has only a title and body.",
                       "evidence": [evidence]} for field in ("authors", "affiliations", "publication_date")},
        }
        self.inventory["sources"].append(source)
        self.units.extend([
            {"id": "si-title", "kind": "metadata", "field": "title", "source": "si", "evidence": [evidence]},
            {"id": "si-paragraph", "kind": "paragraph", "source": "si", "evidence": [evidence]},
        ])
        self.deliveries.extend([
            {"unit": "si-title", "target": {"quote": "Supplement"}},
            {"unit": "si-paragraph", "target": {"quote": "补充材料的独立段落。"}},
        ])
        self.note.write_text(self.note.read_text(encoding="utf-8") + "\n## 3 补充材料\n\nSupplement\n\n补充材料的独立段落。\n", encoding="utf-8")
        self.assertEqual(self.codes(), set())

    def test_source_merge_is_preserved_as_plugin_merge(self):
        table = next(item for item in self.units if item["id"] == "table")
        table["shape"]["rows"] = 2
        table["merges"] = [{"row_start": 1, "row_end": 2, "column_start": 1, "column_end": 1}]
        new_table = TABLE.replace("---\n```", "  - id: r2\n    cells: {a: ''}\nmerges:\n  - anchor: r1.a\n    end: r2.a\n---\n```")
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, new_table), encoding="utf-8")
        self.assertEqual(self.codes(), set())

    def test_flattening_source_merge_is_detected(self):
        table = next(item for item in self.units if item["id"] == "table")
        table["shape"]["rows"] = 2
        table["merges"] = [{"row_start": 1, "row_end": 2, "column_start": 1, "column_end": 1}]
        new_table = TABLE.replace("---\n```", "  - id: r2\n    cells: {a: B}\n---\n```")
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, new_table), encoding="utf-8")
        self.assertIn("TABLE_MERGES_MISMATCH", self.codes())

    def test_merge_endpoint_and_covered_values_are_checked(self):
        table = next(item for item in self.units if item["id"] == "table")
        table["shape"]["rows"] = 2
        table["merges"] = [{"row_start": 1, "row_end": 2, "column_start": 1, "column_end": 1}]
        valid = TABLE.replace("---\n```", "  - id: r2\n    cells: {a: ''}\nmerges:\n  - anchor: r1.a\n    end: r2.a\n---\n```")
        original = self.note.read_text(encoding="utf-8")
        for changed, expected in [
            (valid.replace("end: r2.a", "end: other.a"), "TABLE_MERGE_ENDPOINT"),
            (valid.replace("cells: {a: ''}", "cells: {a: B}"), "TABLE_MERGE_COVERED_VALUE"),
            (valid.replace("anchor: r1.a\n    end: r2.a", "anchor: r2.a\n    end: r1.a"), "TABLE_MERGE_RANGE"),
        ]:
            with self.subTest(expected=expected):
                self.note.write_text(original.replace(TABLE, changed), encoding="utf-8")
                self.assertIn(expected, self.codes())

    def test_specific_cross_reference_cannot_stop_at_parent_heading(self):
        extra = "\n## 3 补充材料\n\n### 3.1 方法\n\n详情[[#3 补充材料|见方法]]。\n"
        self.note.write_text(self.note.read_text(encoding="utf-8") + extra, encoding="utf-8")
        self.units.extend([
            {"id": "method", "kind": "section", "source": "main", "evidence": [self.evidence]},
            {"id": "method-call", "kind": "cross_reference", "source": "main", "evidence": [self.evidence], "target_unit": "method"},
        ])
        self.deliveries.extend([
            {"unit": "method", "target": {"quote": "### 3.1 方法"}},
            {"unit": "method-call", "target": {"quote": "详情[[#3 补充材料|见方法]]。"}},
        ])
        self.assertIn("CROSS_REFERENCE_TARGET", self.codes())

    def test_doi_and_email_need_actual_hyperlink_targets(self):
        additions = [("doi", "10.5555/example", "https://doi.org/10.5555/example"),
                     ("email", "author@example.org", "mailto:author@example.org")]
        original = self.note.read_text(encoding="utf-8")
        for field, value, href in additions:
            with self.subTest(field=field):
                self.units.append({"id": field, "kind": "metadata", "field": field, "source": "main",
                                   "value": value, "evidence": [self.evidence]})
                self.deliveries.append({"unit": field, "target": {"quote": href}})
                self.note.write_text(original + "\n" + href + "\n", encoding="utf-8")
                self.assertIn(field.upper() + "_LINK", self.codes())
                self.deliveries[-1]["target"] = {"quote": f"[{value}]({href})"}
                self.note.write_text(original + f"\n[{value}]({href})\n", encoding="utf-8")
                self.assertNotIn(field.upper() + "_LINK", self.codes())
                self.units.pop()
                self.deliveries.pop()

    def test_preserved_legacy_reference_anchor_is_not_a_migration_error(self):
        old = self.note.read_text(encoding="utf-8").replace("ref-1", "ref-s1")
        self.note.write_text(old, encoding="utf-8")
        for delivery in self.deliveries:
            target = delivery["target"]
            if target.get("anchor") == "ref-1":
                target["anchor"] = "ref-s1"
            if "quote" in target:
                target["quote"] = target["quote"].replace("ref-1", "ref-s1")
        self.bind()
        report = checker.validate_delivery(self.manifest_path)
        self.assertTrue(report.structurally_consistent, report.findings)
        self.assertIn("NOTE_BIBLIOGRAPHY_PLUGIN_COMPAT", {item.code for item in report.findings})

    def test_malformed_inventory_is_a_diagnostic_not_a_false_success(self):
        self.units[0]["source"] = []
        self.assertIn("MANIFEST_OR_EVIDENCE_INVALID", self.codes())

    def test_hidden_formula_in_code_is_not_delivered_math(self):
        original = "$$\nE = mc^2\n$$\n^eq-1"
        hidden = "```text\n$$\nE = mc^2\n$$\n```"
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(original, hidden), encoding="utf-8")
        next(item for item in self.deliveries if item["unit"] == "formula")["target"] = {"quote": "$$\nE = mc^2\n$$"}
        self.assertIn("TARGET_HIDDEN", self.codes())

    def test_rich_table_nested_in_example_fence_is_not_delivered_table(self):
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, "Visible label\n````text\n" + TABLE + "\n````"), encoding="utf-8")
        next(item for item in self.deliveries if item["unit"] == "table")["target"] = {"quote": "Visible label\n````text\n" + TABLE + "\n````"}
        self.assertIn("TABLE_NOT_EDITABLE", self.codes())

    def test_visible_word_cannot_validate_hidden_bibliography_anchor(self):
        original = "1. Author. A source. 2024. ^ref-1"
        hidden = "Visible label\n```text\n" + original + "\n```"
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(original, hidden), encoding="utf-8")
        next(item for item in self.deliveries if item["unit"] == "reference")["target"] = {"quote": hidden}
        self.assertIn("REFERENCE_TARGET", self.codes())

    def test_non_image_bytes_cannot_be_a_delivered_figure(self):
        self.base.joinpath("figure.png").write_bytes(b"Not an actual PNG image")
        self.assertIn("FIGURE_IMAGE_DECODE", self.codes())

    def test_non_utf8_text_evidence_is_reported(self):
        record = self.source["pages"][0]["text"]
        path = Path(record["path"])
        path.write_bytes(b"\xff\xfe\x00")
        record.update(checker.source_tools.file_record(path))
        self.assertIn("MANIFEST_OR_EVIDENCE_INVALID", self.codes())

    def test_literal_escaped_reference_is_not_an_unmapped_call(self):
        text = self.note.read_text(encoding="utf-8").replace("## 2 参考文献", "字面示例\\[1](#^ref-1)。\n\n## 2 参考文献")
        self.note.write_text(text, encoding="utf-8")
        self.assertEqual(self.codes(), set())

    def test_yaml_comment_is_not_a_visible_citation(self):
        changed = TABLE.replace("---\n```", "# 隐藏注释[1](#^ref-1)\n---\n```")
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, changed), encoding="utf-8")
        self.assertEqual(self.codes(), set())

    def test_yaml_unicode_encoded_visible_citation_is_mapped(self):
        literal = '"\\u005b1](#^ref-1)"'
        changed = TABLE.replace("a: B", "a: " + literal)
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, changed), encoding="utf-8")
        self.units.append({"id": "table-citation", "kind": "citation", "source": "main",
                           "evidence": [self.evidence], "references": ["reference"]})
        self.deliveries.append({"unit": "table-citation", "target": {"quote": literal}})
        self.assertEqual(self.codes(), set())

    def test_scalar_cells_container_is_rejected(self):
        changed = TABLE.replace("cells:\n      a: B", "cells: 42")
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, changed), encoding="utf-8")
        self.assertIn("TABLE_ROW_CELLS", self.codes())

    def test_unmapped_v3_sheet_still_has_its_merges_checked(self):
        changed = """```rich-table
version: 3
sheets:
  - id: first
    locked: true
    theme: grid
    columns: [{id: a, name: 名称}]
    rows: [{id: r1, cells: {a: B}}]
  - id: second
    locked: true
    theme: grid
    columns: [{id: a, name: 名称}]
    rows: [{id: r2, cells: {a: C}}]
    merges: [{anchor: missing.a, end: r2.a}]
```
^tbl-1"""
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(TABLE, changed), encoding="utf-8")
        next(item for item in self.deliveries if item["unit"] == "table")["sheet"] = {"id": "first"}
        codes = self.codes()
        self.assertIn("TABLE_SHEET_UNMAPPED", codes)
        self.assertIn("TABLE_MERGE_ENDPOINT", codes)

    def test_supported_markdown_destination_and_title_forms(self):
        original = self.note.read_text(encoding="utf-8")
        for field, value, markup in [
            ("doi", "10.5555/example(test)", "[DOI](<https://doi.org/10.5555/example(test)>)"),
            ("doi", "10.5555/example(test)", "[DOI](https://doi.org/10.5555/example(test))"),
            ("email", "author@example.org", '[Email](mailto:author@example.org "Contact")'),
        ]:
            with self.subTest(markup=markup):
                self.units.append({"id": field, "kind": "metadata", "field": field, "source": "main",
                                   "value": value, "evidence": [self.evidence]})
                self.deliveries.append({"unit": field, "target": {"quote": markup}})
                self.note.write_text(original.replace("## 2 参考文献", markup + "\n\n## 2 参考文献"), encoding="utf-8")
                self.assertEqual(self.codes(), set())
                self.units.pop()
                self.deliveries.pop()

    def test_image_angle_path_with_spaces_supports_quote_and_anchor(self):
        self.base.joinpath("figure.png").rename(self.base.joinpath("figure space.png"))
        changed = FIGURE.replace("![[figure.png]]", "![Figure](<figure space.png>)")
        self.note.write_text(self.note.read_text(encoding="utf-8").replace(FIGURE, changed), encoding="utf-8")
        delivery = next(item for item in self.deliveries if item["unit"] == "figure")
        delivery["assets"] = [{"path": "figure space.png"}]
        for target in ({"quote": changed}, {"anchor": "fig-1"}):
            with self.subTest(target=target):
                delivery["target"] = target
                self.assertEqual(self.codes(), set())

    def test_nested_note_uses_explicit_verified_resource_root(self):
        vault = self.base.joinpath("vault")
        note = vault.joinpath("文献", "分类", "candidate.md")
        asset = vault.joinpath("文献", "attachments", "figure.png")
        note.parent.mkdir(parents=True)
        asset.parent.mkdir(parents=True)
        self.base.joinpath("figure.png").rename(asset)
        changed = FIGURE.replace("figure.png", "文献/attachments/figure.png")
        note.write_text(self.note.read_text(encoding="utf-8").replace(FIGURE, changed), encoding="utf-8")
        self.note = note
        self.resource_root = vault
        delivery = next(item for item in self.deliveries if item["unit"] == "figure")
        delivery["target"] = {"quote": changed}
        delivery["assets"] = [{"path": "文献/attachments/figure.png"}]
        self.assertEqual(self.codes(), set())

    def test_snapshot_imports_do_not_reuse_other_snapshot_siblings(self):
        directory = self.base.joinpath("another-snapshot")
        directory.mkdir()
        for name in ("validate_delivery.py", "validate_note.py", "prepare_source_inventory.py"):
            shutil.copyfile(SCRIPT.parent.joinpath(name), directory.joinpath(name))
        spec = importlib.util.spec_from_file_location("second_delivery_snapshot", directory.joinpath("validate_delivery.py"))
        assert spec and spec.loader
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        self.assertNotEqual(module.note_checks.__file__, checker.note_checks.__file__)
        self.assertNotEqual(module.source_tools.__file__, checker.source_tools.__file__)


if __name__ == "__main__":
    unittest.main(verbosity=2)
