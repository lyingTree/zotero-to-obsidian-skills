"""Native Word text must retain source identity throughout translation review."""
from copy import deepcopy
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from xml.sax.saxutils import escape
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


WRITER = load("native_translation_writer_test", ROOT.joinpath(
    "literature-translate-zh", "scripts", "translation_review_writer.py"))
GLOSSARY = load("native_glossary_test", ROOT.joinpath(
    "literature-translate-zh", "scripts", "glossary_candidates.py"))
URLS = load("native_urls_test", ROOT.joinpath(
    "literature-zotero2obsidian", "scripts", "source_url_inventory.py"))


class NativeTranslationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root.joinpath("invented.docx")
        self.texts = [
            "The cobalt badge moved. See https://example.org/data.",
            "The violet shutter closed. See https://example.org/data.",
        ]
        paragraphs = "".join(f"<w:p><w:r><w:t>{escape(text)}</w:t></w:r></w:p>"
                             for text in self.texts)
        with ZipFile(self.source, "w") as package:
            package.writestr("[Content_Types].xml",
                '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                '<Default Extension="xml" ContentType="application/xml"/>'
                '<Override PartName="/word/document.xml" '
                'ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
                "</Types>")
            package.writestr("word/document.xml",
                '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                f"<w:body>{paragraphs}<w:sectPr/></w:body></w:document>")
        native = WRITER.PREPARE._native_tools().read_docx(self.source, "si")
        self.source_record = {
            "id": "si", "path": str(self.source),
            "sha256": sha256(self.source.read_bytes()).hexdigest(), **native,
        }
        self.blocks = self.source_record["native_blocks"]
        self.inventory = {
            "schema_version": 1, "sources": [self.source_record],
            "units": [
                {"id": f"unit-{index}", "source": "si", "kind": "paragraph",
                 "evidence": [{"basis": "native", "native_locator": block["locator"],
                               "blocks": [block["id"]]}]}
                for index, block in enumerate(self.blocks)
            ],
        }
        self.inventory_path = self.root.joinpath("inventory.json")
        self.inventory_path.write_text(json.dumps(self.inventory), encoding="utf-8")
        self.candidate = self.root.joinpath("candidate.md")
        self.candidate.write_text("钴蓝色徽片移动了。\n紫色快门关闭了。\n", encoding="utf-8")
        self.prepared = self.root.joinpath("review.json")
        self.glossary = self.root.joinpath("glossary.md")
        self.glossary.write_text(
            "| ID | English | Chinese | Context |\n| --- | --- | --- | --- |\n"
            "| T001 | cobalt badge | 钴蓝色徽片 | An invented toy part. |\n",
            encoding="utf-8",
        )

    def prepare(self):
        WRITER.PREPARE.prepare_translation_review(
            self.inventory_path, self.candidate, self.prepared,
            unit_ids=["unit-0", "unit-1"], glossary_path=self.glossary, progress=None)
        return json.loads(self.prepared.read_text(encoding="utf-8"))

    def test_native_prepare_and_writer_never_request_pdf_pages(self):
        with patch.object(WRITER.VALIDATE._Files, "pages", side_effect=AssertionError("DOCX is not PDF")):
            data = self.prepare()
            location = data["units"]["unit-0"]["source_locations"][0]
            self.assertNotIn("page", location)
            self.assertNotIn("bbox", location)
            self.assertEqual(location["quote"], self.texts[0])
            writer = WRITER.ReviewWriter(self.prepared, progress=None)
            unit = writer.unit("unit-0", candidate_locations=[{"lines": [1, 1]}])
            selected = unit.source("cobalt badge", block_id=self.blocks[0]["id"])
            self.assertEqual(selected.locator["native_locator"], self.blocks[0]["locator"])
            self.assertNotIn("page", selected.locator)
            with self.assertRaises(WRITER.ReviewWriterError):
                unit.source("cobalt badge", page=1)
        report = WRITER.VALIDATE.validate_translation_review(self.prepared)
        self.assertFalse(report.records_ready)
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertFalse(any(row.code == "TRN_LOCATION_INVALID" for row in report.findings))

    def test_native_locations_reject_wrong_block_or_invented_geometry(self):
        unit = deepcopy(self.inventory["units"][0])
        for change in (
            {"blocks": [self.blocks[1]["id"]]},
            {"page": 1},
            {"bbox": [0, 0, 100, 100]},
            {"quote": "A source assertion that is absent."},
        ):
            with self.subTest(change=change):
                modified = deepcopy(unit)
                modified["evidence"][0].update(change)
                with self.assertRaises(ValueError):
                    WRITER.PREPARE.expand_source_locations(modified, self.source_record)

    def test_validator_checks_actual_native_quote_not_only_inventory_text(self):
        data = self.prepare()
        data["units"]["unit-0"]["source_locations"][0]["quote"] = "Fabricated sentence."
        self.prepared.write_text(json.dumps(data), encoding="utf-8")
        report = WRITER.VALIDATE.validate_translation_review(self.prepared)
        self.assertIn("TRN_LOCATION_INVALID", {row.code for row in report.findings})

    def test_native_occurrence_cannot_use_a_different_units_block(self):
        data = self.prepare()
        value = WRITER.PREPARE.new_occurrence(
            "wrong-claim", "claims",
            source_span={"basis": "native", "native_locator": self.blocks[1]["locator"],
                         "quote": self.texts[1]})
        data["units"]["unit-0"]["occurrences"]["claims"].append(value)
        self.prepared.write_text(json.dumps(data), encoding="utf-8")
        report = WRITER.VALIDATE.validate_translation_review(self.prepared)
        self.assertIn("TRN_SOURCE_SPAN_OUTSIDE_UNIT", {row.code for row in report.findings})

    def test_native_glossary_leads_keep_real_block_locator(self):
        output = self.root.joinpath("glossary-leads.json")
        GLOSSARY.prepare_glossary_candidates(self.inventory_path, self.glossary, output)
        data = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(len(data["blocks"]), 2)
        row = data["blocks"][0]
        self.assertEqual(row["native_locator"], self.blocks[0]["locator"])
        self.assertNotIn("page", row)
        self.assertTrue(row["matches"])
        self.assertEqual(data["source_fidelity"], "not_assessed")

    def test_malformed_native_review_returns_findings_instead_of_raising(self):
        original = self.prepare()
        for field in ("block_locations", "quote"):
            with self.subTest(field=field):
                data = deepcopy(original)
                data["units"]["unit-0"]["source_locations"][0][field] = None
                data["units"]["unit-0"]["occurrences"]["claims"].append(
                    WRITER.PREPARE.new_occurrence(
                        "actual-claim", "claims", source_span={
                            "basis": "native", "native_locator": self.blocks[0]["locator"],
                            "quote": self.texts[0]}))
                self.prepared.write_text(json.dumps(data), encoding="utf-8")
                report = WRITER.VALIDATE.validate_translation_review(self.prepared)
                self.assertFalse(report.records_ready)
                self.assertIn("TRN_SOURCE_LOCATIONS_CHANGED", {row.code for row in report.findings})

    def test_repeated_native_urls_need_distinct_source_units(self):
        occurrences = URLS.scan_source_url_occurrences(self.inventory)
        self.assertEqual(len(occurrences), 2)
        self.assertNotEqual(occurrences[0]["id"], occurrences[1]["id"])
        self.assertNotIn("page", occurrences[0])
        units = [
            {"id": f"url-{index}", "source": "si", "kind": "metadata",
             "field": "external_url", "display_text": "https://example.org/data",
             "evidence": self.inventory["units"][index]["evidence"]}
            for index in range(2)
        ]
        self.inventory["units"] = units[:1]
        partial = URLS.source_url_coverage(self.inventory)
        self.assertEqual(len(partial["recorded"]), 1)
        self.assertEqual(partial["unmapped"][0]["block"], self.blocks[1]["id"])
        self.inventory["units"] = units
        complete = URLS.source_url_coverage(self.inventory)
        self.assertEqual(len(complete["recorded"]), 2)
        self.assertEqual(complete["unmapped"], [])
        self.assertEqual(complete["source_fidelity"], "not_assessed")

    def test_native_url_binding_cannot_mix_one_block_with_another_locator(self):
        evidence = deepcopy(self.inventory["units"][0]["evidence"])
        evidence[0]["native_locator"] = self.blocks[1]["locator"]
        self.inventory["units"] = [{
            "id": "wrong-url", "source": "si", "kind": "metadata", "field": "external_url",
            "value": "https://example.org/data", "evidence": evidence,
            "source_url_occurrence": f"{self.blocks[0]['id']}:url:1",
        }]
        report = URLS.source_url_coverage(self.inventory)
        self.assertEqual(report["recorded"], [])
        self.assertEqual(len(report["invalid_bindings"]), 1)


if __name__ == "__main__":
    unittest.main()
