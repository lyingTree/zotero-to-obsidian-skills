"""Realistic static acceptance regressions; no Zotero or Obsidian access."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT.joinpath("literature-zotero2obsidian", "scripts", "validate_note.py")
SPEC = importlib.util.spec_from_file_location("literature_validate_note", MODULE_PATH)
assert SPEC and SPEC.loader
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


TABLE = """```rich-table
version: 2
locked: true
theme: grid
title: '**表1：** 示例'
columns:
  - id: source
    name: 来源
rows:
  - id: row1
    cells:
      source: '[1](<#^ref-1>)'
```
^tbl-1
"""

NOTE = """---
类型: 期刊论文
年份: 2025
关键词:
  - 重力波
期刊名: Example
---
# A test study

## 摘要
摘要内容。

## 1 正文
正文引文<sup>\\[[1](<#^ref-1>)\\]</sup>。

$$
E = mc^2 \\tag{1}
$$
^eq-1

公式引用[[#^eq-1|公式1]]。

## 2 参考文献
1. Example, 2025. ^ref-1

## 3 补充材料
补充引文[1](<#^sref-1>)。

### 3.1 参考文献
1. Supplement, 2024. ^sref-1

---

## [Zotero原条目](zotero://select/library/items/EXAMPLE1)
"""


class ValidateNoteTests(unittest.TestCase):
    def codes(self, text: str) -> set[str]:
        return {finding.code for finding in validator.validate_note(text)}

    def test_complete_main_and_supplement_have_distinct_namespaces(self):
        self.assertEqual(validator.validate_note(NOTE), [])

    def test_windows_line_endings_and_bom(self):
        self.assertEqual(validator.validate_note("\ufeff" + NOTE.replace("\n", "\r\n")), [])

    def test_duplicate_frontmatter_keys_are_not_silently_overwritten(self):
        text = "---\n年份: 2024\n年份: 2025\n---\n正文\n"
        findings = validator.validate_note(text)
        self.assertEqual([item.code for item in findings], ["FRONTMATTER_YAML"])
        self.assertEqual(findings[0].line, 3)

    def test_nested_duplicate_yaml_keys_are_rejected(self):
        text = "---\n人工字段:\n  name: first\n  name: second\n---\n正文\n"
        self.assertIn("FRONTMATTER_YAML", self.codes(text))

    def test_invalid_implicit_yaml_date_is_a_finding_and_body_still_checked(self):
        text = "---\ndate: 2025-02-30\n---\n[x](#^absent)\n"
        self.assertEqual(self.codes(text), {"FRONTMATTER_YAML", "MISSING_ANCHOR"})
        table = TABLE.replace("theme: grid", "theme: grid\ndate: 2025-02-30")
        self.assertEqual(self.codes(table + "\n[x](#^absent)\n"), {"TABLE_YAML", "MISSING_ANCHOR"})

    def test_yaml_merge_overrides_remain_legal(self):
        text = "---\nbase: &base\n  color: blue\nlocal:\n  <<: *base\n  color: red\n---\n"
        self.assertEqual(validator.validate_note(text), [])

    def test_deprecated_property_is_found_even_when_empty(self):
        self.assertIn("DEPRECATED_PROPERTY", self.codes("---\n库内参考文献: null\n---\n"))

    def test_nonmapping_and_unclosed_frontmatter(self):
        self.assertIn("FRONTMATTER_TYPE", self.codes("---\n- 列表\n---\n"))
        self.assertIn("FRONTMATTER_UNCLOSED", self.codes("---\n年份: 2025\n"))

    def test_duplicate_id_reports_original_line(self):
        findings = validator.validate_note("第一块 ^ref-1\n\n第二块 ^ref-1\n")
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0].code, "DUPLICATE_ANCHOR")
        self.assertEqual(findings[0].line, 3)
        self.assertIn("第 1 行", findings[0].message)

    def test_same_note_markdown_and_wiki_missing_links(self):
        text = "[甲](<#^a>) [乙](#^b) [[#^c|丙]] ![[#^d]]\n"
        findings = validator.validate_note(text)
        self.assertEqual(len(findings), 4)
        self.assertTrue(all(item.code == "MISSING_ANCHOR" for item in findings))

    def test_target_in_later_block_resolves(self):
        self.assertEqual(validator.validate_note("[先引用](#^later)\n\n后定义 ^later\n"), [])

    def test_cross_note_urls_and_escaped_links_are_not_local(self):
        text = (
            "[跨文件](其他.md#^ref-1) [[其他#^ref-1|引用]] "
            "[网址](https://example.org/a#^ref-1) "
            "\\[字面](<#^ref-1>) \\[[#^ref-1]]\n"
        )
        self.assertEqual(validator.validate_note(text), [])

    def test_code_inline_math_and_comments_are_excluded(self):
        text = (
            "`[示例](#^missing)` 和 $[x](#^missing)$\n"
            "<!-- [隐藏](#^missing) ^hidden\n"
            "```\n"
            "-->\n"
            "%% [隐藏](#^missing)\n^hidden %%\n"
            "    [缩进代码](#^missing)\n"
            "正文 ^valid\n"
        )
        self.assertEqual(validator.validate_note(text), [])

    def test_comment_markers_inside_inline_code_are_literal(self):
        text = "`<!--` `%%`\n[可见](#^absent)\n"
        self.assertEqual(self.codes(text), {"MISSING_ANCHOR"})

    def test_comment_markers_inside_inline_math_are_literal(self):
        self.assertEqual(validator.validate_note("$x %% y$ 与 $x <!-- y$\n"), [])

    def test_code_span_can_cross_adjacent_lines(self):
        text = "`一个示例\n[示例引用](#^missing)\n示例结束`\n"
        self.assertEqual(validator.validate_note(text), [])
        long = "``一个含单反引号 ` 的示例\n[示例引用](#^missing)\n结束``\n"
        self.assertEqual(validator.validate_note(long), [])
        unmatched = "``未配对\n[实际引用](#^missing)\n`\n"
        self.assertEqual(self.codes(unmatched), {"MISSING_ANCHOR"})

    def test_multiline_code_comment_markers_do_not_hide_later_links(self):
        for marker in ("<!--", "%%", "$$"):
            with self.subTest(marker=marker):
                text = f"`literal\n{marker}\nend`\n\n[x](#^absent)\n"
                self.assertEqual(self.codes(text), {"MISSING_ANCHOR"})

    def test_indented_code_cannot_open_comment_or_math_states(self):
        for indentation in ("    ", "\t"):
            with self.subTest(indentation=repr(indentation)):
                text = f"{indentation}$$\n\n正文 ^p\n\n[x](#^p)\n"
                self.assertEqual(validator.validate_note(text), [])
                comment = f"{indentation}<!--\n\n[x](#^absent)\n"
                self.assertEqual(self.codes(comment), {"MISSING_ANCHOR"})

    def test_fence_length_and_marker_must_match(self):
        text = (
            "````markdown\n[代码](#^missing)\n```\n"
            "^duplicate\n~~~~\n^duplicate\n````\n"
            "~~~text\n[代码](#^missing)\n~~~\n正文\n"
        )
        self.assertEqual(validator.validate_note(text), [])

    def test_unclosed_fence_is_reported(self):
        self.assertEqual(self.codes("```python\nprint('hi')\n"), {"FENCE_UNCLOSED"})

    def test_valid_rich_table_resolves_cell_citation(self):
        text = TABLE + "\n## 1 参考文献\n1. Source ^ref-1\n"
        self.assertEqual(validator.validate_note(text), [])

    def test_rich_table_checks_visible_cell_links(self):
        findings = validator.validate_note(TABLE)
        self.assertEqual({item.code for item in findings}, {"MISSING_ANCHOR"})
        self.assertEqual(findings[0].line, 1)

    def test_rich_table_ignores_arbitrary_plugin_metadata(self):
        text = TABLE.replace("theme: grid", "theme: grid\nmetadata: '[x](#^missing)'")
        self.assertEqual(validator.validate_note(text + "\n参考文献 ^ref-1\n"), [])

    def test_table_lock_version_rows_and_duplicate_yaml(self):
        text = TABLE.replace("locked: true", "locked: false").replace("version: 2", "version: '2'")
        self.assertTrue({"TABLE_UNLOCKED", "TABLE_VERSION"} <= self.codes(text))
        empty = "```rich-table\nversion: 2\nlocked: true\ncolumns: []\nrows: []\n```\n^tbl-1\n"
        self.assertIn("TABLE_STRUCTURE", self.codes(empty))
        duplicate = TABLE.replace("locked: true", "locked: true\nlocked: false")
        self.assertIn("TABLE_YAML", self.codes(duplicate))

    def test_null_table_and_orphan_table_anchor_are_rejected(self):
        self.assertIn("TABLE_TYPE", self.codes("```rich-table\nnull\n```\n^tbl-1\n"))
        self.assertIn("TABLE_ANCHOR", self.codes("只是题注 ^tbl-1\n"))

    def test_table_anchor_must_touch_closing_fence(self):
        text = TABLE.replace("```\n^tbl-1", "```\n\n^tbl-1")
        self.assertIn("ANCHOR_GAP", self.codes(text))
        self.assertIn("TABLE_ANCHOR", self.codes(TABLE.replace("^tbl-1", "")))

    def test_table_anchor_must_stand_alone(self):
        self.assertIn("TABLE_ANCHOR", self.codes(TABLE.replace("^tbl-1", "  ^tbl-1")))
        self.assertIn("ANCHOR_SPACING", self.codes(TABLE + "下一段\n"))

    def test_equation_anchor_spacing_and_unclosed_math(self):
        self.assertEqual(validator.validate_note("$$\nx=1\n$$\n^eq-1\n\n后文\n"), [])
        bad = "$$\nx=1\n$$\n\n^eq-1\n后文\n"
        self.assertTrue({"ANCHOR_GAP", "ANCHOR_SPACING", "EQUATION_ANCHOR"} <= self.codes(bad))
        self.assertIn("MATH_UNCLOSED", self.codes("$$\nx=1\n"))

    def test_unnumbered_equation_requires_no_invented_anchor(self):
        self.assertEqual(validator.validate_note("$$\nx=1\n$$\n\n后文\n"), [])

    def test_single_line_display_math(self):
        self.assertEqual(validator.validate_note("$$ x=1 $$\n^eq-1\n"), [])

    def test_formula_contents_do_not_define_anchors(self):
        text = "$$\n[x](#^missing) ^example\n$$\n\n正文 ^example\n"
        self.assertEqual(validator.validate_note(text), [])

    def test_main_and_si_cannot_reuse_ref_1(self):
        findings = validator.validate_note(NOTE.replace("sref-1", "ref-1"))
        self.assertTrue(any(item.code == "DUPLICATE_ANCHOR" and item.level == "error" for item in findings))

    def test_multiple_si_sources_keep_their_own_numbering(self):
        text = (
            "## 3 补充材料\n"
            "[A](#^ref-sab12cd34-1) [B](#^ref-sxy98zz76-1)\n\n"
            "### 3.1 参考文献\n1. A ^ref-sab12cd34-1\n\n"
            "### 3.2 参考文献\n1. B ^ref-sxy98zz76-1\n"
        )
        self.assertEqual(validator.validate_note(text), [])

    def test_legacy_si_namespace_can_coexist_with_new_source(self):
        text = (
            "## 3 补充材料\n[A](#^ref-s1) [B](#^sref-src0123456789ab-1)\n\n"
            "### 3.1 参考文献\n1. A ^ref-s1\n\n"
            "### 3.2 参考文献\n1. B ^sref-src0123456789ab-1\n"
        )
        findings = validator.validate_note(text)
        self.assertEqual([item.code for item in findings], ["BIBLIOGRAPHY_PLUGIN_COMPAT"])
        self.assertEqual(findings[0].level, "warning")

    def test_plugin_130_anchor_matrix(self):
        # Expected values were checked against the actual 1.3.0 referenceAnchor
        # and parseReferences functions, commit 165503f647590b7d9379e8b1ba72a2b9b61de049.
        cases = {
            "ref-1": True, "sref-1": True, "ref1": True, "sref1": True,
            "SREF-1": True, "sref-a-1": True, "ref-appendix-a2-1": True,
            "sref-src0123456789ab-1": True, "ref-sab12cd34ef56-1": True,
            "ref-s1": False, "sref-0123456789ab-1": False,
            "sref-0": False, "sref-01": False, "sref-1000000000": False,
        }
        for anchor, supported in cases.items():
            with self.subTest(anchor=anchor):
                text = f"## 3 补充材料\n[文献](#^{anchor})\n\n### 3.1 参考文献\n1. Entry ^{anchor}\n"
                findings = validator.validate_note(text)
                self.assertEqual(any(item.code == "BIBLIOGRAPHY_PLUGIN_COMPAT" for item in findings), not supported)
                self.assertFalse(any(item.level == "error" for item in findings))

    def test_new_si_sources_have_plugin_compatible_prefixes(self):
        text = (
            "## 3 补充材料\n[A](#^sref-src0123456789ab-1) [B](#^sref-srcabc123def456-1)\n\n"
            "### 3.1 参考文献\n1. A ^sref-src0123456789ab-1\n\n"
            "### 3.2 参考文献\n1. B ^sref-srcabc123def456-1\n"
        )
        self.assertEqual(validator.validate_note(text), [])

    def test_existing_number_mismatch_is_a_warning_and_file_stays_unchanged(self):
        text = "[原链接](#^sref-12) [旧格式](#^ref-s1)\n\n## 3 补充材料\n### 3.1 参考文献\n1. A ^sref-12\n2. B ^ref-s1\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).joinpath("existing.md")
            original = text.encode("utf-8")
            path.write_bytes(original)
            findings = validator.validate_file(path)
            self.assertEqual({item.code for item in findings}, {"BIBLIOGRAPHY_NUMBER", "BIBLIOGRAPHY_PLUGIN_COMPAT"})
            self.assertTrue(all(item.level == "warning" for item in findings))
            self.assertEqual(path.read_bytes(), original)

    def test_missing_and_mismatched_bibliography_anchors(self):
        self.assertIn("BIBLIOGRAPHY_ANCHOR", self.codes("## 2 参考文献\n1. No anchor\n"))
        self.assertIn("BIBLIOGRAPHY_NUMBER", self.codes("## 2 参考文献\n1. Existing ^ref-2\n"))
        self.assertIn("BIBLIOGRAPHY_ANCHOR", self.codes("## 2 参考文献\n1. Early ^ref-1\n   more text\n"))

    def test_multiline_reference_can_have_anchor_on_its_last_line(self):
        text = "## 2 参考文献\n1. Author, title,\n   journal. ^ref-1\n"
        self.assertEqual(validator.validate_note(text), [])

    def test_four_space_reference_continuation_is_not_a_code_block(self):
        text = "## 2 参考文献\n1. Author\n    journal. ^ref-1\n"
        self.assertEqual(validator.validate_note(text), [])
        with_link = "[正确](#^ref-1) [待补](#^absent)\n\n" + text
        self.assertEqual(self.codes(with_link), {"MISSING_ANCHOR"})

    def test_file_validation_never_modifies_source(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).joinpath("文献.md")
            original = ("\ufeff" + NOTE.replace("\n", "\r\n")).encode("utf-8")
            path.write_bytes(original)
            self.assertEqual(validator.validate_file(path), [])
            self.assertEqual(path.read_bytes(), original)

    def test_missing_and_non_utf8_files_are_findings(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).joinpath("missing.md")
            self.assertEqual(validator.validate_file(path)[0].code, "FILE_READ")
            path.write_bytes(b"\xff\xfeinvalid")
            self.assertEqual(validator.validate_file(path)[0].code, "FILE_READ")


if __name__ == "__main__":
    unittest.main()
