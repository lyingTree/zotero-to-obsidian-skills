"""Repeated URLs need repeated source identities, independently of note syntax."""
from __future__ import annotations

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT.joinpath("literature-zotero2obsidian", "scripts")


def load(name):
    spec = importlib.util.spec_from_file_location("url_inventory_test_" + name, SCRIPTS.joinpath(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


urls = load("source_url_inventory")
delivery = load("validate_delivery")


def inventory(texts):
    return {"sources": [{"id": "main", "pages": [{"page": 1, "blocks": [
        {"id": f"block-{number}", "text": text} for number, text in enumerate(texts, 1)
    ]}]}], "units": []}


def unit(identity, value, *, blocks=(), source="main", page=1, field="external_url", **extra):
    return {"id": identity, "kind": "metadata", "source": source, "field": field,
            "display_text": value, "value": value,
            "evidence": [{"page": page, "blocks": list(blocks)}], **extra}


class SourceUrlInventoryTests(unittest.TestCase):
    def test_identical_occurrences_are_not_deduplicated(self):
        value = "https://example.org/Archive/"
        data = inventory([f"Data: {value}; comparison: {value}."])
        before = deepcopy(data)
        leads = urls.scan_source_url_occurrences(data)
        self.assertEqual(len(leads), 2)
        self.assertEqual(len({lead["id"] for lead in leads}), 2)
        self.assertEqual(data, before)
        data["units"] = [unit("first", value)]
        self.assertEqual(len(urls.source_url_coverage(data)["unmapped"]), 1)
        data["units"].append(unit("second", value))
        self.assertEqual(urls.source_url_coverage(data)["unmapped"], [])

    def test_ordinary_paragraph_mapping_does_not_replace_a_url_unit(self):
        value = "https://example.org/a"
        data = inventory([value])
        data["units"] = [{**unit("paragraph", value), "kind": "paragraph"}]
        self.assertEqual(len(urls.source_url_coverage(data)["unmapped"]), 1)

    def test_a_different_source_or_page_cannot_satisfy_the_occurrence(self):
        value = "www.example.org/data"
        data = inventory([value])
        for row in (unit("wrong-source", value, source="supplement"), unit("wrong-page", value, page=2)):
            with self.subTest(row=row):
                data["units"] = [row]
                self.assertEqual(len(urls.source_url_coverage(data)["unmapped"]), 1)

    def test_narrow_location_is_preserved_when_a_broad_match_would_steal_it(self):
        value = "https://example.org/repeated"
        data = inventory([value, value])
        data["units"] = [unit("broad", value), unit("narrow", value, blocks=["block-1"])]
        report = urls.source_url_coverage(data)
        self.assertEqual(report["unmapped"], [])
        self.assertEqual({row["occurrence"]: row["unit"] for row in report["recorded"]},
                         {"block-1:url:1": "narrow", "block-2:url:1": "broad"})

    def test_wrapped_source_tokens_need_an_explicit_page_review_binding(self):
        data = inventory(["Archive https://example.org/long-\npath/"])
        data["units"] = [unit("url", "https://example.org/long-path/")]
        self.assertEqual(len(urls.source_url_coverage(data)["unmapped"]), 1)
        data["units"][0]["source_url_occurrence"] = "block-1:url:1"
        report = urls.source_url_coverage(data)
        self.assertEqual(report["unmapped"], [])
        self.assertEqual(report["source_fidelity"], "not_assessed")
        self.assertEqual(report["occurrences"][0]["raw_text"], "https://example.org/long-")

    def test_explicit_bindings_reject_unknown_duplicate_and_wrong_location(self):
        value = "https://example.org/a"
        for rows in (
            [unit("a", value, source_url_occurrence="unknown")],
            [unit("a", value, page=2, source_url_occurrence="block-1:url:1")],
            [unit("a", value, source_url_occurrence="block-1:url:1"),
             unit("b", value, source_url_occurrence="block-1:url:1")],
        ):
            with self.subTest(rows=rows):
                data = inventory([value])
                data["units"] = rows
                self.assertEqual(len(urls.source_url_coverage(data)["invalid_bindings"]), 1)

    def test_punctuation_www_and_doi_keep_the_source_token_and_path_case(self):
        data = inventory(["(https://example.org/Fn_(x)). www.example.org/Data; https://doi.org/10.9999/demo."])
        data["units"] = [unit("a", "https://example.org/Fn_(x)"),
                         unit("b", "https://www.example.org/Data"),
                         unit("c", "doi: 10.9999/demo", field="doi")]
        report = urls.source_url_coverage(data)
        self.assertEqual(report["unmapped"], [])
        self.assertEqual(report["occurrences"][0]["raw_text"], "https://example.org/Fn_(x)")
        data["units"][1]["value"] = data["units"][1]["display_text"] = "www.example.org/data"
        self.assertEqual(len(urls.source_url_coverage(data)["unmapped"]), 1)

    def test_adjacent_parenthesized_urls_keep_separate_occurrence_bindings(self):
        data = inventory(["Archives (https://example.org/one),(https://example.net/two)."])
        data["units"] = [unit("first", "https://example.org/one", source_url_occurrence="block-1:url:1")]
        report = urls.source_url_coverage(data)
        self.assertEqual([row["lookup_text"] for row in report["occurrences"]],
                         ["https://example.org/one", "https://example.net/two"])
        self.assertEqual([row["id"] for row in report["unmapped"]], ["block-1:url:2"])
        for row in report["occurrences"]:
            text = data["sources"][0]["pages"][0]["blocks"][0]["text"]
            self.assertEqual(text[row["start"]:row["end"]], row["raw_text"])

    def test_balanced_path_ipv6_and_redirect_query_remain_single_tokens(self):
        values = ["https://example.org/Fn_(nested(x))", "https://[2001:db8::1]/data",
                  "https://example.org/redirect?to=https://example.net/data"]
        data = inventory([f"({value})." for value in values])
        self.assertEqual([row["lookup_text"] for row in urls.scan_source_url_occurrences(data)], values)

    def test_only_explicitly_excluded_source_locations_are_skipped(self):
        data = inventory(["https://example.org/running-header", "https://example.org/data"])
        self.assertEqual(len(urls.scan_source_url_occurrences(data, excluded_blocks={"block-1"})), 1)
        self.assertEqual(urls.scan_source_url_occurrences(data, excluded_pages={("main", 1)}), [])

    def test_delivery_inventory_checks_use_actual_pdf_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pdf = root.joinpath("source.pdf")
            value = "https://example.org/source-data"
            with fitz.open() as document:
                page = document.new_page(width=500, height=160)
                page.insert_text((20, 40), "Data: " + value)
                document.save(pdf)
            path = delivery.source_tools.prepare_sources({"main": pdf}, root.joinpath("evidence"),
                                                        source_roles={"main": "main"}, progress=None)
            data = json.loads(path.read_text(encoding="utf-8"))
            leads = urls.scan_source_url_occurrences(data)
            self.assertEqual([row["lookup_text"] for row in leads], [value])
            for mapped in (False, True):
                if mapped:
                    data["units"] = [unit("data-url", value, blocks=[leads[0]["block"]])]
                audit = delivery._Audit(root.joinpath("manifest.json"), None)
                audit.check_sources(data, path.parent)
                audit.check_inventory(data)
                codes = {finding.code for finding in audit.report.findings}
                self.assertEqual("SOURCE_URL_UNMAPPED" in codes, not mapped)

    def test_real_pdf_adjacent_url_cannot_hide_behind_the_first_binding(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pdf = root.joinpath("source.pdf")
            with fitz.open() as document:
                page = document.new_page(width=600, height=160)
                page.insert_text((20, 40), "Archives (https://example.org/one),(https://example.net/two).")
                document.save(pdf)
            path = delivery.source_tools.prepare_sources({"main": pdf}, root.joinpath("evidence"),
                                                        source_roles={"main": "main"}, progress=None)
            data = json.loads(path.read_text(encoding="utf-8"))
            leads = urls.scan_source_url_occurrences(data)
            self.assertEqual(len(leads), 2)
            data["units"] = [unit("first-url", "https://example.org/one", blocks=[leads[0]["block"]],
                                  source_url_occurrence=leads[0]["id"])]
            audit = delivery._Audit(root.joinpath("manifest.json"), None)
            audit.check_sources(data, path.parent)
            audit.check_inventory(data)
            findings = [row for row in audit.report.findings if row.code == "SOURCE_URL_UNMAPPED"]
            self.assertEqual(len(findings), 1)
            self.assertEqual(findings[0].object_id, leads[1]["id"])
            self.assertIn("https://example.net/two", findings[0].message)


if __name__ == "__main__":
    unittest.main()
