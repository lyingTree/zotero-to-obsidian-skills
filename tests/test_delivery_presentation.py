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

    def test_unit_after_closed_math_is_still_part_of_the_quantity(self):
        text = r"近$18$年与$6\text{-}9$年；每$8$小时；距离$2$km。"
        prompts = [item.excerpt for item in lint_presentation(text) if item.code == "QUANTITY_SPLIT_MATH"]
        self.assertEqual(prompts, [r"$18$年", r"$6\text{-}9$年", "$8$小时", "$2$km"])

    def test_split_quantity_exempts_dates_and_ignores_invisible_syntax(self):
        text = r"$2018$年；$2001\text{-}2024$年；$18\,\text{年}$；`$18$年`；[标签](https://example.org/$18$年)"
        self.assertNotIn("QUANTITY_SPLIT_MATH", self.codes(text))

    def test_every_rich_table_sheet_checks_split_quantities_and_titles(self):
        table = {"version": 3, "sheets": [
            {"title": "**图8内表格。**", "columns": [], "rows": []},
            {"title": "**图S2内表。**", "columns": [{"name": "时长"}],
             "rows": [{"cells": {"c1": "$12$年"}}]},
        ]}
        text = "```rich-table\n---\n" + yaml.safe_dump(table, allow_unicode=True) + "---\n```\n"
        self.assertEqual(self.codes(text), {"EMBEDDED_TABLE_TITLE", "QUANTITY_SPLIT_MATH"})

    def test_figure_table_title_variants_cannot_omit_the_required_marker(self):
        for title in ("**图7内的数据表：** 检测结果。", "图S4的数据表", "图12内表。", "图2说明表"):
            with self.subTest(title=title):
                table = {"version": 2, "title": title, "columns": [], "rows": []}
                text = "```rich-table\n---\n" + yaml.safe_dump(table, allow_unicode=True) + "---\n```\n"
                self.assertIn("EMBEDDED_TABLE_TITLE", self.codes(text))
        for title in ("**图7内表格。** 检测结果。", "图S4内表格", "**表2。** 图7的检测结果。"):
            with self.subTest(title=title):
                table = {"version": 2, "title": title, "columns": [], "rows": []}
                text = "```rich-table\n" + yaml.safe_dump(table, allow_unicode=True) + "```\n"
                self.assertNotIn("EMBEDDED_TABLE_TITLE", self.codes(text))

    def test_second_worksheet_title_is_not_hidden_by_correct_first_sheet(self):
        table = {"version": 3, "sheets": [
            {"title": "图6内表格", "columns": [], "rows": []},
            {"title": "图9内的数据表", "columns": [], "rows": []},
        ]}
        text = "```rich-table\n---\n" + yaml.safe_dump(table, allow_unicode=True) + "---\n```\n"
        prompts = [item for item in lint_presentation(text) if item.code == "EMBEDDED_TABLE_TITLE"]
        self.assertEqual([item.excerpt for item in prompts], ["图9内的数据表"])

    def test_standalone_figure_images_keep_the_required_two_space_break(self):
        for image in ("![[assets/figure.png]]", "![图7](assets/figure.png)", "![图S2](<assets/figure (two).png>)"):
            with self.subTest(image=image):
                for ending in ("", " ", "\\"):
                    self.assertIn("IMAGE_HARD_BREAK", self.codes(image + ending + "\n**图7：** 题注。"))
                self.assertNotIn("IMAGE_HARD_BREAK", self.codes(image + "  \n**图7：** 题注。"))

    def test_image_hard_break_check_ignores_nonvisible_and_inline_examples(self):
        for text in ("`![[assets/a.png]]`", "<!-- ![[assets/a.png]] -->", "    ![[assets/a.png]]",
                     "```text\n![[assets/a.png]]\n```", "$$\n![[assets/a.png]]\n$$",
                     "文字中的 ![示例](assets/a.png) 是行内说明。"):
            with self.subTest(text=text):
                self.assertNotIn("IMAGE_HARD_BREAK", self.codes(text))

    def test_image_hard_break_check_ignores_nonimage_wiki_transclusions(self):
        for text in ("![[supplement.pdf]]", "![[movie.mp4]]", "![[Methods#^calibration]]",
                     "![[notes/Methods.md|400]]", "![[supplement.pdf#page=2]]"):
            with self.subTest(text=text):
                self.assertNotIn("IMAGE_HARD_BREAK", self.codes(text))
        for text in ("![[assets/image.PNG|400]]", "![[assets/image.svg]]", "![plot](https://example.org/image)"):
            with self.subTest(text=text):
                self.assertIn("IMAGE_HARD_BREAK", self.codes(text))

    def test_each_header_metadata_item_occupies_its_own_line(self):
        merged = "> Received: 3 May 2020；Accepted: 7 June 2020；Published online: 19 June 2020"
        self.assertIn("HEADER_ITEMS_SAME_LINE", self.codes(merged))
        self.assertNotIn("HEADER_ITEMS_SAME_LINE", self.codes(merged.replace("；", "\n> ")))
        self.assertIn("HEADER_ITEMS_SAME_LINE", self.codes("> 接收日期：2020年5月3日；接受日期：2020年6月7日"))

    def test_header_check_ignores_plain_prose_and_code(self):
        text = "Received: A; Accepted: B\n\n```text\n> Received: A; Accepted: B\n```"
        self.assertNotIn("HEADER_ITEMS_SAME_LINE", self.codes(text))

    def test_duplicate_paragraph_blank_lines_are_actionable(self):
        prompts = [item for item in lint_presentation("首段。\n\n\n第二段。") if item.code == "PARAGRAPH_GAP_EXTRA"]
        self.assertEqual(len(prompts), 1)
        self.assertEqual(prompts[0].line, 2)

    def test_paragraph_spacing_does_not_inspect_code_or_file_edges(self):
        text = "\n\n首段。\n\n第二段。\n\n```text\n第一行\n\n\n第四行\n```\n\n\n"
        self.assertNotIn("PARAGRAPH_GAP_EXTRA", self.codes(text))

    def test_angle_command_requires_its_backslash_in_inline_and_display_math(self):
        text = "$12^circ$\n\n$$\nx=35^{circ}\n$$"
        prompts = [item for item in lint_presentation(text) if item.code == "DEGREE_COMMAND_MISSING_BACKSLASH"]
        self.assertEqual([item.excerpt for item in prompts], ["^circ", "^{circ"])
        self.assertTrue(all(item.rule_id == "BODY-C183" for item in prompts))

    def test_valid_angles_and_invisible_examples_do_not_prompt(self):
        text = r"$12^\circ$ and $35^{\circ}$; `12^circ`; [label](https://example.test/^circ)."
        self.assertNotIn("DEGREE_COMMAND_MISSING_BACKSLASH", self.codes(text))
        self.assertEqual(self.codes("```text\n$12^circ$\n```"), set())

    def test_second_worksheet_checks_angle_command(self):
        table = {"version": 3, "sheets": [
            {"columns": [], "rows": []},
            {"columns": [{"id": "a", "name": "Angle"}], "rows": [{"id": "r", "cells": {"a": "$12^circ$"}}]},
        ]}
        text = "```rich-table\n---\n" + yaml.safe_dump(table, allow_unicode=True) + "---\n```"
        self.assertEqual(self.codes(text), {"DEGREE_COMMAND_MISSING_BACKSLASH"})

    def test_placeholder_header_date_requires_visible_source_review(self):
        for value in ("xx/xx/xxxx", "DD Month YYYY", "YYYY-MM-DD"):
            with self.subTest(value=value):
                prompts = [item for item in lint_presentation("> Published online: " + value)
                           if item.code == "HEADER_DATE_PLACEHOLDER"]
                self.assertEqual(len(prompts), 1)
                self.assertEqual(prompts[0].rule_id, "HDR-C42")

    def test_real_dates_and_non_header_placeholders_are_not_rejected(self):
        text = "> Published online: 19 June 2020\n\nxx/xx/xxxx\n`> 发表：xx xx xxxx`\n"
        self.assertNotIn("HEADER_DATE_PLACEHOLDER", self.codes(text))

    def test_century_duration_keeps_approximation_without_treating_calendar_century_as_quantity(self):
        text = "记录跨越一个多世纪和数个世纪；20世纪与第十一个世纪是年代。"
        quantities = [item.excerpt for item in lint_presentation(text) if item.code == "QUANTITY_OUTSIDE_MATH"]
        self.assertEqual(quantities, ["一个多世纪", "数个世纪"])

    def test_dimensionless_range_needs_math_but_dates_and_object_numbers_are_exempt(self):
        text = "数值范围2到4，另一范围-3至-1；记录覆盖2001至2024年；第3至5节，图3至5；12:00至13:00。"
        ranges = [item.excerpt for item in lint_presentation(text) if item.code == "NUMERIC_RANGE_OUTSIDE_MATH"]
        self.assertEqual(ranges, ["2到4", "-3至-1"])

    def test_math_code_and_destinations_do_not_create_new_quantity_prompts(self):
        text = r"$\text{一个多}\,\text{世纪}$与$2\text{-}4$；`一个多世纪，2至4`；[标签](https://example.test/2至4)。"
        self.assertNotIn("QUANTITY_OUTSIDE_MATH", self.codes(text))
        self.assertNotIn("NUMERIC_RANGE_OUTSIDE_MATH", self.codes(text))

    def test_range_values_alone_cannot_establish_calendar_context(self):
        text = "无量纲参数1600至1800，另一参数1400至1450；资料覆盖1600至1800年。"
        ranges = [item.excerpt for item in lint_presentation(text) if item.code == "NUMERIC_RANGE_OUTSIDE_MATH"]
        self.assertEqual(ranges, ["1600至1800", "1400至1450"])

    def test_later_worksheet_checks_dimensionless_range(self):
        table = {"version": 3, "sheets": [
            {"columns": [], "rows": []},
            {"columns": [{"id": "c", "name": "Range"}], "rows": [{"id": "r", "cells": {"c": "2至4"}}]},
        ]}
        text = "```rich-table\n---\n" + yaml.safe_dump(table, allow_unicode=True) + "---\n```"
        self.assertEqual(self.codes(text), {"NUMERIC_RANGE_OUTSIDE_MATH"})

    def table_note(self, table, *, wrapped=True):
        payload = yaml.safe_dump(table, allow_unicode=True)
        if wrapped:
            payload = "---\n" + payload + "---\n"
        return "```rich-table\n" + payload + "```\n"

    def test_plain_table_title_requires_bold_label_in_wrapped_v2(self):
        text = self.table_note({"version": 2, "title": "表7：校准数据。", "columns": [], "rows": []})
        self.assertEqual(self.codes(text), {"TABLE_TITLE_BOLD", "TABLE_TITLE_LABEL"})

    def test_complete_table_label_is_accepted_with_or_without_wrapper(self):
        table = {"version": 2, "title": "**表7：** 校准数据。", "columns": [], "rows": []}
        for wrapped in (True, False):
            with self.subTest(wrapped=wrapped):
                self.assertEqual(self.codes(self.table_note(table, wrapped=wrapped)), set())

    def test_unclosed_table_bold_is_not_a_complete_label(self):
        text = self.table_note({"version": 2, "title": "**表7：校准数据。", "columns": [], "rows": []})
        self.assertEqual(self.codes(text), {"TABLE_TITLE_BOLD", "TABLE_TITLE_LABEL"})

    def test_later_sheet_table_label_has_its_own_prompt(self):
        table = {"version": 3, "sheets": [
            {"title": "**表7：** 校准数据。", "columns": [], "rows": []},
            {"title": "表8：校准数据。", "columns": [], "rows": []},
        ]}
        prompts = lint_presentation(self.table_note(table))
        self.assertEqual({item.code for item in prompts}, {"TABLE_TITLE_BOLD", "TABLE_TITLE_LABEL"})
        self.assertTrue(all("第2张工作表" in item.message for item in prompts))

    def test_identical_bad_titles_on_different_sheets_keep_separate_prompts(self):
        table = {"version": 3, "sheets": [
            {"title": "表7：校准数据。", "columns": [], "rows": []},
            {"title": "表7：校准数据。", "columns": [], "rows": []},
        ]}
        prompts = lint_presentation(self.table_note(table))
        self.assertEqual(sum(item.code == "TABLE_TITLE_BOLD" for item in prompts), 2)
        self.assertEqual(sum(item.code == "TABLE_TITLE_LABEL" for item in prompts), 2)

    def test_figure_table_title_checks_bold_without_treating_it_as_independent_table(self):
        table = {"version": 2, "title": "图7内表格：校准数据。", "columns": [], "rows": []}
        self.assertEqual(self.codes(self.table_note(table)), {"TABLE_TITLE_BOLD"})
        table["title"] = "**图7内表格：** 校准数据。"
        self.assertEqual(self.codes(self.table_note(table)), set())

    def test_main_and_supplement_caption_labels_require_the_template_space(self):
        prompts = [item for item in lint_presentation("**图7：**说明。\n\n**图S7：**说明。")
                   if item.code == "FIGURE_CAPTION_SPACE"]
        self.assertEqual([item.rule_id for item in prompts], ["BODY-C95", "BODY-C96"])
        self.assertNotIn("FIGURE_CAPTION_SPACE", self.codes("**图7：** 说明。\n\n**图S7：** 说明。"))

    def test_caption_space_prompt_ignores_nonvisible_examples(self):
        for text in ("`**图S7：**说明。`", "<!-- **图S7：**说明。 -->", "    **图S7：**说明。",
                     "```text\n**图S7：**说明。\n```", "$$\n**图S7：**说明。\n$$"):
            with self.subTest(text=text):
                self.assertNotIn("FIGURE_CAPTION_SPACE", self.codes(text))

    def test_alphabetic_labels_with_hard_breaks_remain_plain_markdown(self):
        for labels in (("a)", "b)"), ("(a)", "(b)"), ("**a)**", "**b)**"), ("__a.__", "__b.__")):
            with self.subTest(labels=labels):
                text = labels[0] + " First class.  \n" + labels[1] + " Second class.\n"
                prompts = [item for item in lint_presentation(text) if item.code == "LETTER_LIST_NOT_MARKDOWN"]
                self.assertEqual(len(prompts), 1)
                self.assertEqual(prompts[0].line, 1)

    def test_real_markdown_lists_and_isolated_letters_are_not_prompted(self):
        for text in ("1. a) First class.\n2. b) Second class.", "- a) First class.\n- b) Second class.",
                     "a) An isolated label.\n\nOrdinary paragraph.",
                     "a) First panel.\n\n## Separate section\nb) Other panel."):
            with self.subTest(text=text):
                self.assertNotIn("LETTER_LIST_NOT_MARKDOWN", self.codes(text))

    def test_alphabetic_list_prompt_ignores_code_comments_and_math(self):
        for text in ("```text\na) First class.\nb) Second class.\n```",
                     "<!--\na) First class.\nb) Second class.\n-->",
                     "$$\na) First class.\nb) Second class.\n$$",
                     "    a) First class.\n    b) Second class."):
            with self.subTest(text=text):
                self.assertNotIn("LETTER_LIST_NOT_MARKDOWN", self.codes(text))

    def test_english_durations_include_ranges_hyphenated_values_and_reference_titles(self):
        text = "Period: 7.25 year; a 3-year survey; span 2-4 months.\n\n1. A. Example. A 5.5 year cycle. *Journal* (2020)."
        quantities = [item.excerpt for item in lint_presentation(text) if item.code == "QUANTITY_OUTSIDE_MATH"]
        self.assertEqual(quantities, ["7.25 year", "3-year", "2-4 months", "5.5 year"])

    def test_english_duration_split_across_math_boundary_is_prompted(self):
        text = r"A $7$ year period and $2\text{-}4$ months."
        quantities = [item.excerpt for item in lint_presentation(text) if item.code == "QUANTITY_SPLIT_MATH"]
        self.assertEqual(quantities, ["$7$ year", r"$2\text{-}4$ months"])

    def test_english_duration_prompts_exclude_dates_math_code_and_destinations(self):
        text = r"Published: 7 June 2020. $7.25\,\mathrm{yr}$; `3-year`; [label](https://example.test/5-year)."
        self.assertNotIn("QUANTITY_OUTSIDE_MATH", self.codes(text))
        self.assertNotIn("QUANTITY_SPLIT_MATH", self.codes(text))
        self.assertEqual(self.codes("```text\n7.25 year\n```"), set())

    def test_linked_or_commented_complete_table_labels_are_visible_and_valid(self):
        for title in ("[**表9：**](<https://example.test/catalog/7-year>) 虚构光学实验。",
                      "<!-- internal remark -->**表9：** 虚构光学实验。",
                      "%% internal remark %%**表9：** 虚构光学实验。"):
            with self.subTest(title=title):
                text = self.table_note({"version": 2, "title": title, "columns": [], "rows": []})
                self.assertEqual(self.codes(text), set())

    def test_partial_bold_table_marker_is_not_a_bold_label(self):
        for title in ("**表**9：虚构光学实验。", "**图9**内表格：虚构光学实验。"):
            with self.subTest(title=title):
                text = self.table_note({"version": 2, "title": title, "columns": [], "rows": []})
                expected = {"TABLE_TITLE_BOLD", "TABLE_TITLE_LABEL"} if title.startswith("**表") else {"TABLE_TITLE_BOLD"}
                self.assertEqual(self.codes(text), expected)

    def test_visible_title_without_object_marker_requests_review_without_inventing_number(self):
        for title, expected in (("虚构校验结果", {"TABLE_TITLE_BOLD", "TABLE_TITLE_LABEL"}),
                                ("**虚构校验结果**", {"TABLE_TITLE_LABEL"})):
            with self.subTest(title=title):
                prompts = lint_presentation(self.table_note({"version": 2, "title": title, "columns": [], "rows": []}))
                self.assertEqual({item.code for item in prompts}, expected)
                self.assertTrue(all(item.level == "review" for item in prompts))
                self.assertTrue(any("不自动补造或重排编号" in item.message for item in prompts))

    def test_title_literal_subnumbers_and_unicode_numbers_are_not_flattened(self):
        for title in ("**表IV：** 虚构光学实验。", "**表9.2(a)：** 虚构光学实验。",
                      "**表甲：** 虚构光学实验。", "**图S 9(a)内表格：** 虚构光学实验。"):
            with self.subTest(title=title):
                text = self.table_note({"version": 2, "title": title, "columns": [], "rows": []})
                self.assertNotIn("TABLE_TITLE_BOLD", self.codes(text))
                self.assertNotIn("TABLE_TITLE_LABEL", self.codes(text))
                self.assertNotIn("EMBEDDED_TABLE_TITLE", self.codes(text))

    def test_each_v3_sheet_still_has_distinct_title_prompts_after_visibility_projection(self):
        table = {"version": 3, "sheets": [
            {"title": "[**表9：**](<#^kept-table>) 虚构校准。", "columns": [], "rows": []},
            {"title": "%% internal %%表9：虚构校准。", "columns": [], "rows": []},
            {"title": "**表**9：虚构校准。", "columns": [], "rows": []},
        ]}
        prompts = [item for item in lint_presentation(self.table_note(table)) if item.code.startswith("TABLE_TITLE_")]
        self.assertEqual(len(prompts), 4)
        self.assertEqual(sum("第2张工作表" in item.message for item in prompts), 2)
        self.assertEqual(sum("第3张工作表" in item.message for item in prompts), 2)

    def test_hidden_title_examples_do_not_become_table_labels(self):
        for title in ("`表9：虚构校验。`", "<!-- 表9：虚构校验。 -->", "%% 表9：虚构校验。 %%"):
            with self.subTest(title=title):
                text = self.table_note({"version": 2, "title": title, "columns": [], "rows": []})
                self.assertEqual(self.codes(text), set())

    def test_english_duration_ignores_embed_targets_and_html_attributes(self):
        for text in ("![[assets/9-year-lens.png]]  ", "![[assets/lens.png|9-year]]  ",
                     "![9 years](<https://example.test/7-year.png>)  ",
                     '<span data-description="$9$ years">Mirror</span>',
                     "[[assets/9-year|Mirror]]"):
            with self.subTest(text=text):
                self.assertNotIn("QUANTITY_OUTSIDE_MATH", self.codes(text))
                self.assertNotIn("QUANTITY_SPLIT_MATH", self.codes(text))
        self.assertIn("QUANTITY_OUTSIDE_MATH", self.codes("[[9-year]]"))
        self.assertIn("QUANTITY_OUTSIDE_MATH", self.codes("[9 years](<https://example.test/catalog>)"))

    def test_hidden_reference_link_destinations_and_multiline_titles_are_not_prose(self):
        text = ('[catalog]: <https://example.test/9-year> "7 years"\n'
                '[other]:\n  <https://example.test/8-year>\n  "6 years\n'
                'a) Example one.\nb) Example two."\n\nVisible for 4 years.')
        prompts = lint_presentation(text)
        quantities = [item.excerpt for item in prompts if item.code == "QUANTITY_OUTSIDE_MATH"]
        self.assertEqual(quantities, ["4 years"])
        self.assertNotIn("LETTER_LIST_NOT_MARKDOWN", {item.code for item in prompts})

    def test_reference_style_visible_label_remains_eligible_for_duration_review(self):
        text = '[9 years][catalog]\n\n[catalog]: <https://example.test/7-year> "5 years"'
        quantities = [item.excerpt for item in lint_presentation(text) if item.code == "QUANTITY_OUTSIDE_MATH"]
        self.assertEqual(quantities, ["9 years"])

    def test_footnotes_and_malformed_or_inline_definition_like_prose_are_not_hidden(self):
        for text in ("[^detail]: 9 years", "[catalog]: target not a 9-year title",
                     'Ordinary prose.\n[catalog]: /target "9 years"'):
            with self.subTest(text=text):
                self.assertIn("QUANTITY_OUTSIDE_MATH", self.codes(text))

    def test_figure_caption_space_handles_literal_source_number_shapes(self):
        for number in ("S 9(a)", "SIV", r"S9\*", "S9.2(a)"):
            with self.subTest(number=number):
                prefix = "**图" + number + "：**"
                prompts = [item for item in lint_presentation(prefix + "孔径示意。") if item.code == "FIGURE_CAPTION_SPACE"]
                self.assertEqual(len(prompts), 1)
                self.assertEqual(prompts[0].rule_id, "BODY-C96")
                self.assertNotIn("FIGURE_CAPTION_SPACE", self.codes(prefix + " 孔径示意。"))

    def test_visible_linked_letter_labels_remain_non_markdown_list(self):
        text = '[a) Mirror](<#^kept-a>)  \n[b) Lens](<#^kept-b>)'
        prompts = [item for item in lint_presentation(text) if item.code == "LETTER_LIST_NOT_MARKDOWN"]
        self.assertEqual(len(prompts), 1)
        self.assertEqual(prompts[0].level, "review")
        self.assertNotIn("LETTER_LIST_NOT_MARKDOWN", self.codes('[Mirror](<a)>)\n[Lens](<b)>)'))


if __name__ == "__main__":
    unittest.main()
