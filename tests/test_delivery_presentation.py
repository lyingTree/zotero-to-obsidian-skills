"""Presentation checks must catch omissions without changing source material."""

from pathlib import Path
import sys
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT.joinpath("literature-zotero2obsidian", "scripts")))
from lint_presentation import lint_presentation


class PresentationTests(unittest.TestCase):
    def codes(self, text):
        return {item.code for item in lint_presentation(text)}

    def test_naked_identifiers_are_actionable(self):
        codes = self.codes("联系 data@example.org。DOI：10.9999/example。网址 https://example.org/path。")
        self.assertTrue({"NAKED_EMAIL", "NAKED_DOI", "NAKED_URL"}.issubset(codes))

    def test_valid_links_and_excluded_syntax_are_not_rejected(self):
        text = """---
类型: 期刊论文
备注: data@example.org
---
[data@example.org](mailto:data@example.org)
[10.9999/example](<https://doi.org/10.9999/example>)
<https://example.org/a–b>
`10% data@example.org`
```text
10% data@example.org
```
"""
        self.assertEqual(self.codes(text), set())

    def test_quantities_and_percentages_do_not_escape_checking(self):
        codes = self.codes("每8小时采样，持续2-3年，包含25%的站点。")
        self.assertIn("QUANTITY_OUTSIDE_MATH", codes)
        self.assertIn("PERCENT_OUTSIDE_MATH", codes)

    def test_math_and_calendar_year_are_exempt(self):
        text = r"2019年发表；每$8\,\mathrm{h}$采样，持续$2\text{-}3\,\mathrm{yr}$，包含$25\%$。"
        self.assertEqual(self.codes(text), set())

    def test_calendar_range_is_exempt_without_exempting_duration(self):
        prompts = lint_presentation("资料覆盖2003-2021年；持续4-5年。")
        quantities = [item.excerpt for item in prompts if item.code == "QUANTITY_OUTSIDE_MATH"]
        self.assertEqual(quantities, ["4-5年"])

    def test_numbered_heading_separator_is_not_an_extra_prose_space(self):
        prompts = lint_presentation("## 2 方法\n\n### 2.3 参数 A定义\n正文 A定义。")
        spaces = [(item.line, item.excerpt) for item in prompts if item.code == "CJK_SPACE"]
        self.assertEqual(spaces, [(3, "数 "), (4, "文 ")])

    def test_visible_label_is_checked_but_target_is_not(self):
        text = "中文 [标签](<https://example.org/a–b>)；[甲–乙](<#^item-a–b>)。"
        prompts = lint_presentation(text)
        self.assertEqual(sum(item.code == "VISIBLE_DASH" for item in prompts), 1)
        self.assertNotIn("NAKED_URL", {item.code for item in prompts})

    def test_second_sheet_visible_cells_are_checked(self):
        table = {"version": 3, "sheets": [
            {"id": "a–b", "columns": [{"id": "c1", "name": "名称"}], "rows": [{"id": "r1", "cells": {"c1": "正常"}}]},
            {"columns": [{"id": "c1", "name": "数值"}], "rows": [{"id": "r1", "cells": {"c1": "50%"}}]},
        ]}
        text = "```rich-table\n---\n" + yaml.safe_dump(table, allow_unicode=True) + "---\n```\n^tbl-1\n"
        self.assertEqual(self.codes(text), {"PERCENT_OUTSIDE_MATH"})

    def test_cjk_adjacent_quantity_is_not_hidden_by_unicode_word_boundary(self):
        prompts = lint_presentation("每8小时采样。")
        quantities = [item.excerpt for item in prompts if item.code == "QUANTITY_OUTSIDE_MATH"]
        self.assertEqual(quantities, ["8小时"])

    def test_table_wrapper_with_outer_whitespace_does_not_hide_values(self):
        table = {"version": 2, "columns": [{"id": "c", "name": "量"}], "rows": [{"id": "r", "cells": {"c": "25%"}}]}
        text = "```rich-table\n\n---\n" + yaml.safe_dump(table, allow_unicode=True) + "---\n\n```\n^tbl-1\n"
        self.assertIn("PERCENT_OUTSIDE_MATH", self.codes(text))

    def test_comments_are_hidden_without_hiding_following_visible_prose(self):
        self.assertEqual(self.codes("<!--\nHidden 25% data@example.test\n-->\n"), set())
        self.assertIn("PERCENT_OUTSIDE_MATH", self.codes("<!-- Hidden --> Visible 25%."))

    def test_link_title_and_balanced_destination_are_preserved(self):
        text = '[Contact](mailto:person@example.test "Contact")\n[DOI](<https://doi.org/10.9999/a(b)> "Source")'
        self.assertEqual(self.codes(text), set())

    def test_adjacent_panel_links_are_reviewed_as_one_source_call(self):
        text = "见[图7a](<#^fig-7>)和[图7c](<#^fig-7>)；另见[图8a](<#^fig-8>)。"
        prompts = [item for item in lint_presentation(text) if item.code == "SPLIT_PANEL_CALL"]
        self.assertEqual(len(prompts), 1)
        self.assertIn("图7c", prompts[0].excerpt)

    def test_distinct_panel_claims_and_cross_object_calls_are_not_merged(self):
        text = "[图7a](<#^fig-7>)显示波高，[图7c](<#^fig-7>)显示流速。见[图7a](<#^fig-7>)和[图8b](<#^fig-8>)。"
        self.assertNotIn("SPLIT_PANEL_CALL", self.codes(text))
        self.assertNotIn("SPLIT_PANEL_CALL", self.codes("[图7a、7c](<#^fig-7>)"))

    def test_panel_call_positions_survive_prior_inline_code(self):
        text = "`示例` [图7a](<#^fig-7>)和[图7b](<#^fig-7>)。"
        self.assertIn("SPLIT_PANEL_CALL", self.codes(text))
        self.assertIn("SPLIT_PANEL_CALL", self.codes("[[#^fig-7|图7a]]、[[#^fig-7|图7b]]"))

    def test_numeric_range_prompt_does_not_change_real_subtraction(self):
        prompts = lint_presentation(r"周期$6-8\,\mathrm{yr}$；差值$8-6$；规范范围$6\text{-}8$；符号$x-y$。")
        ranges = [item.excerpt for item in prompts if item.code == "MATH_RANGE_OR_SUBTRACTION"]
        self.assertEqual(ranges, ["6-8", "8-6"])
        self.assertTrue(all("真实减法" in item.message for item in prompts))

    def test_display_trailing_comma_is_detected_without_flagging_tuple(self):
        text = "$$\nx = (a,b),\n$$\n\n$$x=(a,b)$$\n\n$$\\left. x\\right.$$\n"
        prompts = [item for item in lint_presentation(text) if item.code == "DISPLAY_TRAILING_PUNCTUATION"]
        self.assertEqual(len(prompts), 1)
        self.assertEqual(prompts[0].line, 1)

    def test_aligned_trailing_punctuation_is_checked_before_closing_command(self):
        text = "$$\n\\begin{aligned}\nx &= y;\n\\end{aligned}\n$$\n"
        self.assertIn("DISPLAY_TRAILING_PUNCTUATION", self.codes(text))
        tagged = text.replace("x &= y;", "x &= y; \\tag{2}")
        self.assertIn("DISPLAY_TRAILING_PUNCTUATION", self.codes(tagged))

    def test_zotero_footer_separator_is_independent_of_live_uri(self):
        self.assertIn("ZOTERO_SEPARATOR_MISSING", self.codes("正文\n\n## Zotero原条目\n"))
        self.assertNotIn("ZOTERO_SEPARATOR_MISSING", self.codes("正文\n\n---\n\n## Zotero原条目\n"))
        self.assertNotIn("ZOTERO_SEPARATOR_MISSING", self.codes("正文\n\n---\n\n## [Zotero原条目](zotero://select/library/items/ABC)\n"))
        self.assertIn("ZOTERO_SEPARATOR_MISSING", self.codes("正文\n\n---\n```text\n末块\n```\n\n## Zotero原条目\n"))

    def test_new_presentation_prompts_do_not_read_code_examples(self):
        text = "```text\n[图7a](#^fig-7)和[图7b](#^fig-7)\n$$x=3-4,$$\n## Zotero原条目\n```\n"
        self.assertEqual(self.codes(text), set())

    def test_embedded_table_anchor_prompts_do_not_demand_legacy_migration(self):
        prompts = lint_presentation("^tbl-fig-9\n\n^tbl-fig-main-9\n\n^tbl-fig-supA-A2\n")
        self.assertEqual(len(prompts), 1)
        self.assertEqual(prompts[0].code, "EMBEDDED_TABLE_SOURCE_TOKEN")
        self.assertIn("既有 ID 保留", prompts[0].message)

    def test_chinese_duration_is_checked_but_chinese_calendar_year_is_exempt(self):
        prompts = lint_presentation("过去十二年的资料，二〇一九年发表，模型覆盖五至七年，另有数十年记录。")
        quantities = [item.excerpt for item in prompts if item.code == "QUANTITY_OUTSIDE_MATH"]
        self.assertEqual(quantities, ["十二年", "五至七年", "数十年"])

    def test_adjacent_headings_require_the_child_heading_gap(self):
        prompts = lint_presentation("## 1 方法\n### 1.1 数据\n正文。")
        self.assertIn("HEADING_GAP_MISSING", {item.code for item in prompts})
        self.assertEqual(self.codes("## 1 方法\n\n### 1.1 数据\n正文。"), set())

    def test_heading_body_gap_exempts_display_math(self):
        self.assertIn("HEADING_BODY_GAP", self.codes("## 1 方法\n\n正文。"))
        self.assertEqual(self.codes("## 1 方法\n\n$$\nx=y\n$$\n"), set())

    def test_math_panel_syntax_does_not_create_a_visible_call(self):
        self.assertNotIn("SPLIT_PANEL_CALL", self.codes("$[图1a](#^fig-1)、[图1b](#^fig-1)$"))

    def test_indented_code_does_not_create_a_visible_call(self):
        self.assertNotIn("SPLIT_PANEL_CALL", self.codes("    [图1a](#^fig-1)、[图1b](#^fig-1)"))

    def test_linked_panel_range_is_reviewed(self):
        self.assertIn("SPLIT_PANEL_CALL", self.codes("[图2a](#^fig-2)-[图2c](#^fig-2)"))

    def test_second_panel_label_can_omit_figure_prefix(self):
        self.assertIn("SPLIT_PANEL_CALL", self.codes("[图1a](#^fig-1)、[1b](#^fig-1)"))

    def test_math_in_link_destination_is_not_visible(self):
        self.assertNotIn("MATH_RANGE_OR_SUBTRACTION", self.codes("[标签](https://example.org/$5-7$)"))

    def test_math_in_link_title_is_not_visible(self):
        self.assertNotIn("MATH_RANGE_OR_SUBTRACTION", self.codes('[标签](https://example.org "值$5-7$")'))

    def test_display_formula_trailing_period_is_reviewed(self):
        self.assertIn("DISPLAY_TRAILING_PUNCTUATION", self.codes("$$\nx=y.\n$$"))
        self.assertNotIn("DISPLAY_TRAILING_PUNCTUATION", self.codes(r"$$x=1.25$$"))

    def test_hidden_comment_does_not_change_preceding_heading_gap(self):
        self.assertNotIn("HEADING_GAP_MISSING", self.codes("正文\n\n<!-- hidden -->\n## 标题\n正文"))

    def test_hidden_comment_does_not_change_parent_child_spacing(self):
        self.assertEqual(self.codes("## 父标题\n\n<!-- hidden -->\n### 子标题\n正文"), set())

    def test_indented_separator_does_not_separate_the_footer(self):
        self.assertIn("ZOTERO_SEPARATOR_MISSING", self.codes("正文\n\n    ---\n\n## Zotero原条目\n条目"))

    def test_embedded_table_anchor_requires_nonempty_source_token(self):
        self.assertIn("EMBEDDED_TABLE_SOURCE_TOKEN", self.codes("^tbl-fig--1"))

    def test_embedded_table_anchor_requires_nonempty_object_token(self):
        self.assertIn("EMBEDDED_TABLE_SOURCE_TOKEN", self.codes("^tbl-fig-main-"))

    def test_escaped_backticks_do_not_hide_visible_quantity(self):
        self.assertIn("QUANTITY_OUTSIDE_MATH", self.codes(r"\`十二年\`"))

    def test_cross_line_inline_code_does_not_hide_following_prose(self):
        self.assertEqual(self.codes("`示例\n[图1a](#^fig-1)、[图1b](#^fig-1)`"), set())
        self.assertIn("QUANTITY_OUTSIDE_MATH", self.codes("`示例\n隐藏`十二年"))


if __name__ == "__main__":
    unittest.main()
