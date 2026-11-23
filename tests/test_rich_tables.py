"""Rich Table payload and worksheet regressions; templates are tested separately."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT.joinpath("literature-zotero2obsidian", "scripts", "validate_note.py")
SPEC = importlib.util.spec_from_file_location("rich_table_validator", MODULE_PATH)
assert SPEC and SPEC.loader
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


def sheet(identifier: str) -> dict:
    return {
        "id": identifier,
        "name": identifier,
        "title": "表格",
        "columns": [{"id": "source", "name": "来源"}],
        "rows": [{"id": "r1", "cells": {"source": "[1](<#^ref-1>)"}}],
        "theme": "grid",
        "locked": True,
    }


def workbook() -> dict:
    return {"version": 3, "activeSheetId": "first", "sheets": [sheet("first"), sheet("second")]}


def note(data: dict, *, wrapped: bool = True, as_json: bool = False) -> str:
    payload = json.dumps(data, ensure_ascii=False) + "\n" if as_json else yaml.safe_dump(
        data, allow_unicode=True, sort_keys=False
    )
    if wrapped:
        payload = "---\n" + payload + "---\n"
    return "```rich-table\n" + payload + "```\n^tbl-1\n\n书目 ^ref-1\n"


class RichTableTests(unittest.TestCase):
    def test_wrapped_v2_and_v3_are_valid_without_top_level_workbook_locks(self):
        for data in ({"version": 2, **sheet("single")}, workbook()):
            for wrapped in (True, False):
                with self.subTest(version=data["version"], wrapped=wrapped):
                    self.assertEqual(validator.validate_note(note(data, wrapped=wrapped)), [])

    def test_inner_second_yaml_document_is_not_removed_with_outer_wrapper(self):
        text = note({"version": 2, **sheet("single")}).replace(
            "\n---\n```", "\n---\nversion: 2\n---\n```"
        )
        self.assertIn("TABLE_YAML", {item.code for item in validator.validate_note(text)})

    def test_each_sheet_lock_and_theme_are_checked(self):
        data = workbook()
        for item in data["sheets"]:
            item["locked"] = False
            item["theme"] = "plain"
        findings = validator.validate_note(note(data))
        self.assertEqual(len(findings), 4)
        for index, identifier in enumerate(("first", "second"), 1):
            matching = [item for item in findings if f"工作表 {index}" in item.message]
            self.assertEqual({item.code for item in matching}, {"TABLE_UNLOCKED", "TABLE_THEME"})
            self.assertTrue(all(identifier in item.message for item in matching))

    def test_inactive_sheet_visible_links_are_checked_and_identified(self):
        data = workbook()
        second = data["sheets"][1]
        second["title"] = "[题注](#^missing-title)"
        second["footer"] = "[[#^missing-footer|表注]]"
        second["columns"][0]["name"] = "[表头](#^missing-header)"
        second["rows"][0]["cells"]["source"] = "[单元格](#^missing-cell)"
        text = note(data)
        findings = validator.validate_note(text)
        self.assertEqual(len(findings), 4)
        start = next(index + 1 for index, line in enumerate(text.splitlines()) if line == "- id: second")
        self.assertTrue(all(item.code == "MISSING_ANCHOR" and item.line == start for item in findings))
        self.assertTrue(all("工作表 2" in item.message and "id=second" in item.message for item in findings))

    def test_single_line_json_uses_sheet_context_without_invented_cell_lines(self):
        data = workbook()
        data["sheets"][1]["rows"][0]["cells"]["source"] = "[bad](#^absent)"
        findings = validator.validate_note(note(data, as_json=True))
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0].line, 3)
        self.assertIn("工作表 2", findings[0].message)

    def test_second_sheet_structure_is_not_inherited_from_first(self):
        data = workbook()
        data["sheets"][1]["rows"] = []
        data["sheets"][1]["columns"] = None
        findings = validator.validate_note(note(data))
        self.assertEqual(len(findings), 2)
        self.assertTrue(all(item.code == "TABLE_STRUCTURE" and "工作表 2" in item.message for item in findings))

    def test_invalid_sheet_does_not_stop_validation_of_later_sheets(self):
        data = workbook()
        data["sheets"][0] = None
        data["sheets"][1]["rows"][0]["cells"]["source"] = "[bad](#^absent)"
        findings = validator.validate_note(note(data))
        self.assertEqual({item.code for item in findings}, {"TABLE_SHEET_TYPE", "MISSING_ANCHOR"})
        missing = next(item for item in findings if item.code == "MISSING_ANCHOR")
        self.assertIn("工作表 2", missing.message)

    def test_empty_workbook_and_unknown_version_fail(self):
        for value in (None, [], {}):
            with self.subTest(sheets=value):
                findings = validator.validate_note(note({"version": 3, "sheets": value}))
                self.assertEqual([item.code for item in findings], ["TABLE_STRUCTURE"])
        findings = validator.validate_note(note({"version": 4, **sheet("single")}))
        self.assertIn("TABLE_VERSION", {item.code for item in findings})

    def test_duplicate_sheet_yaml_keys_still_fail(self):
        text = note(workbook()).replace("  name: second", "  name: second\n  name: overwritten")
        self.assertIn("TABLE_YAML", {item.code for item in validator.validate_note(text)})


if __name__ == "__main__":
    unittest.main()
