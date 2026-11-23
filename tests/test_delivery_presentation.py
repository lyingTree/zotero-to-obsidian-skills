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


if __name__ == "__main__":
    unittest.main()
