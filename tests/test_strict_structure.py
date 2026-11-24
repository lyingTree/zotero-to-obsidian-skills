"""Source-independent regressions for explicit table and front-material rules."""

from copy import deepcopy
import json
from pathlib import Path
import sys
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT.joinpath("literature-zotero2obsidian", "scripts")))
from format_helpers import build_rich_table_block
from lint_presentation import lint_presentation
from validate_note import validate_note


def sheet(name="first"):
    return {
        "id": name, "name": name, "locked": True, "theme": "grid",
        "title": "**表7：** 参数", "footer": "[来源](#^ref-1)",
        "columns": [{"id": "quantity", "name": "量"}, {"id": "value", "name": "数值"}],
        "rows": [{"id": "r1", "cells": {"quantity": "速度", "value": r"$\\bar{v}_{i}$"}}],
    }


def parsed_block(text):
    lines = text.splitlines()
    payload = "\n".join(lines[2:-3]) if lines[1] == "---" else "\n".join(lines[1:-2])
    return yaml.safe_load(payload)


class StrictTableStructureTests(unittest.TestCase):
    def test_flow_columns_and_rows_have_distinct_rules_and_source_locations(self):
        data = {"version": 2, **sheet()}
        payload = json.dumps(data, ensure_ascii=False, indent=2)
        note = "```rich-table\n---\n" + payload + "\n---\n```\n^tbl-7\n\n来源 ^ref-1\n"
        findings = validate_note(note.replace("\n", "\r\n"))
        self.assertEqual({item.code for item in findings}, {"TABLE_COLUMNS_FLOW_STYLE", "TABLE_ROWS_FLOW_STYLE"})
        for item, field, rule in ((findings[0], "columns", "BODY-C138"), (findings[1], "rows", "BODY-C139")):
            expected = next(i for i, line in enumerate(note.splitlines(), 1) if f'"{field}": [' in line)
            self.assertEqual(item.line, expected)
            self.assertIn(rule, item.message)

    def test_block_array_can_contain_a_flow_mapping_without_becoming_flow_array(self):
        note = """```rich-table
version: 2
locked: true
theme: grid
columns:
  - {id: c, name: 值}
rows:
  - {id: r, cells: {c: 8}}
```
^tbl-7
"""
        self.assertEqual(validate_note(note), [])

    def test_merged_yaml_styles_resolve_actual_field_and_explicit_override(self):
        note = """```rich-table
version: 3
defaults: &defaults
  locked: true
  theme: grid
  columns: [{id: c, name: 值}]
  rows: [{id: r, cells: {c: 8}}]
sheets:
  - <<: *defaults
    columns:
      - id: c
        name: 值
```
^tbl-7
"""
        findings = validate_note(note)
        self.assertEqual([item.code for item in findings], ["TABLE_ROWS_FLOW_STYLE"])
        self.assertEqual(findings[0].line, 7)
        self.assertIn("工作表 1", findings[0].message)

    def test_each_sheet_self_link_is_distinct_from_legal_body_call_and_citation(self):
        first, second = sheet(), sheet("second")
        second["title"] = "**[表7](<#^tbl-existing>)：** 参数 [来源](#^ref-1)"
        data = {"version": 3, "sheets": [first, second]}
        note = build_rich_table_block(data, anchor="tbl-existing") + "\n正文见[表7](#^tbl-existing)。\n\n来源 ^ref-1\n"
        findings = validate_note(note)
        self.assertEqual([item.code for item in findings], ["TABLE_CAPTION_SELF_LINK"])
        self.assertIn("工作表 2", findings[0].message)
        self.assertIn("LINK-C27", findings[0].message)
        self.assertIn("title:", note.splitlines()[findings[0].line - 1])

    def test_wiki_self_link_is_detected_but_code_comment_and_other_target_are_not(self):
        data = {"version": 2, **sheet()}
        data["title"] = "**[[#^tbl-7|表7]]：** 参数"
        note = build_rich_table_block(data, anchor="tbl-7") + "\n来源 ^ref-1\n"
        self.assertEqual([item.code for item in validate_note(note)], ["TABLE_CAPTION_SELF_LINK"])
        for title in (
            "**表7：** 参数 `[表7](#^tbl-7)`",
            "**表7：** 参数 <!-- [表7](#^tbl-7) -->",
            "**表7：** 参数 [来源](#^ref-1)",
        ):
            with self.subTest(title=title):
                data["title"] = title
                note = build_rich_table_block(data, anchor="tbl-7") + "\n来源 ^ref-1\n"
                self.assertNotIn("TABLE_CAPTION_SELF_LINK", {item.code for item in validate_note(note)})

    def test_figure_caption_self_link_uses_its_own_inline_or_next_line_anchor(self):
        for caption, anchor in (
            ("**[图7](<#^fig-old>)：** 原始题注。", "\n^fig-old"),
            ("**[[#^fig-old|图7]]：** 原始题注。", " ^fig-old"),
        ):
            with self.subTest(caption=caption, anchor=anchor):
                note = "![图](figure.png)  \n" + caption + anchor + "\n\n正文见[图7](#^fig-old)。\n"
                findings = validate_note(note)
                self.assertEqual([item.code for item in findings], ["FIGURE_CAPTION_SELF_LINK"])
                self.assertEqual(findings[0].line, 2)
        legal = "![图](figure.png)  \n**图7：** 题注 [来源](#^ref-1)。\n^fig-old\n\n正文见[图7](#^fig-old)。\n\n来源 ^ref-1\n"
        self.assertEqual(validate_note(legal), [])


class TableBuilderTests(unittest.TestCase):
    def test_v2_v3_roundtrip_preserves_every_value_and_caller_data(self):
        data_sheet = sheet()
        data_sheet["rows"][0]["cells"].update({"value": r"$\theta_S=\frac{Q_0}{A}$", "quantity": "on: [yes] 'no'"})
        data_sheet["merges"] = [{"anchor": "r1.quantity", "end": "r1.value"}]
        for data in ({"version": 2, **data_sheet}, {"version": 3, "sheets": [data_sheet, sheet("second")]}):
            before = deepcopy(data)
            for wrapped in (True, False):
                for newline in ("\n", "\r\n"):
                    with self.subTest(version=data["version"], wrapped=wrapped, newline=repr(newline)):
                        block = build_rich_table_block(data, anchor="^tbl-legacy-id", wrapped=wrapped, newline=newline)
                        self.assertEqual(parsed_block(block), before)
                        self.assertEqual(data, before)
                        self.assertTrue(block.endswith("^tbl-legacy-id" + newline))
                        self.assertEqual(validate_note(block + newline + "来源 ^ref-1" + newline), [])
                        if newline == "\r\n":
                            self.assertNotIn("\n", block.replace("\r\n", ""))

    def test_cell_fences_cannot_close_the_outer_data_fence(self):
        data = {"version": 2, **sheet()}
        data["rows"][0]["cells"]["value"] = "first\n```\n```text\nexample\n```\nlast"
        block = build_rich_table_block(data, anchor="tbl-7")
        self.assertTrue(block.startswith("````rich-table\n"))
        self.assertEqual(parsed_block(block), data)
        self.assertEqual(validate_note(block + "\n来源 ^ref-1\n"), [])

    def test_builder_requires_explicit_fields_instead_of_filling_missing_data(self):
        for key, value in (("locked", False), ("theme", "plain"), ("columns", []), ("rows", None)):
            data = {"version": 3, "sheets": [sheet(), sheet("second")]}
            data["sheets"][1][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "sheet 2"):
                build_rich_table_block(data, anchor="tbl-7")
        for version in (True, 1, 4, "2"):
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, "version"):
                build_rich_table_block({"version": version, **sheet()}, anchor="tbl-7")
        with self.assertRaisesRegex(ValueError, "without type conversion"):
            build_rich_table_block({"version": 2, **sheet(), "custom": {1: "value"}}, anchor="tbl-7")


class FrontMaterialPlacementTests(unittest.TestCase):
    def placements(self, text):
        return [item for item in lint_presentation(text) if item.code == "FRONT_MATERIAL_AFTER_ABSTRACT"]

    def test_standalone_front_labels_between_abstract_and_body_are_located(self):
        text = "## 摘要\n摘要内容。\n\n**研究亮点**\n- 来源中的要点。\n\n**图文摘要**\n\n## 1 引言\n正文。\n"
        findings = self.placements(text)
        self.assertEqual([(item.line, item.rule_id) for item in findings], [(4, "BODY-C10"), (7, "BODY-C10")])

    def test_numbered_front_section_after_body_is_a_source_order_prompt(self):
        text = "## 摘要\n摘要内容。\n\n## 1 引言\n正文。\n\n## 2 研究要点与图文摘要\n- 要点。\n"
        findings = self.placements(text)
        self.assertEqual(len(findings), 1)
        self.assertEqual((findings[0].line, findings[0].rule_id), (7, "BODY-C16"))

    def test_prose_examples_comments_and_supplement_abstract_are_excluded(self):
        text = """## 摘要
摘要讨论图形摘要设计。

## 1 引言
研究亮点可以帮助读者，本文讨论 Highlights 的写法。

```markdown
**研究亮点**
```

<!-- **图文摘要** -->

## 2 补充材料
### 摘要
补充文件自己的摘要。

**研究亮点**
"""
        self.assertEqual(self.placements(text), [])

    def test_source_front_labels_before_abstract_are_accepted(self):
        text = "**研究亮点**\n- 要点。\n\n**图文摘要**\n\n## 摘要\n摘要内容。\n\n## 1 引言\n正文。\n"
        self.assertEqual(self.placements(text), [])

    def test_front_section_after_supplement_is_not_hidden_by_supplement_scope(self):
        text = ("## 摘要\n摘要。\n\n## 1 引言\n正文。\n\n"
                "## 2 补充材料\n### 摘要\n补充摘要。\n\n"
                "## 3 研究亮点\n要点。\n")
        findings = self.placements(text)
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0].rule_id, "BODY-C16")


if __name__ == "__main__":
    unittest.main()
