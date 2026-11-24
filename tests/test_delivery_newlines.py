"""Real PDF font mappings may emit CR inside an otherwise LF text stream."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-zotero2obsidian", "scripts", "validate_delivery.py")
SPEC = importlib.util.spec_from_file_location("test_delivery_newline_validator", SCRIPT)
assert SPEC and SPEC.loader
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)

CMAP = b"""/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /SyntheticCR def
/CMapType 2 def
1 begincodespacerange
<00> <FF>
endcodespacerange
3 beginbfchar
<41> <0041>
<58> <0058000D>
<42> <0042>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end
"""


class TextEvidenceNewlinesTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        source = self.base.joinpath("source.pdf")
        with fitz.open() as document:
            page = document.new_page(width=200, height=100)
            page.insert_text((20, 30), "AXB")
            font_xref = page.get_fonts()[0][0]
            cmap_xref = document.get_new_xref()
            document.update_object(cmap_xref, "<<>>")
            document.update_stream(cmap_xref, CMAP)
            document.xref_set_key(font_xref, "ToUnicode", f"{cmap_xref} 0 R")
            document.save(source)
        with fitz.open(source) as document:
            self.raw_text = document[0].get_text("text", sort=False)
        self.assertIn("\r", self.raw_text)
        self.inventory_path = checker.source_tools.prepare_sources(
            {"main": source}, self.base.joinpath("work"),
            source_roles={"main": "main"}, progress=None)
        self.inventory = json.loads(self.inventory_path.read_text(encoding="utf-8"))
        page_record = self.inventory["sources"][0]["pages"][0]
        self.text_path = Path(page_record["text"]["path"])
        visual = {"page": 1, "basis": "visual", "bbox": [0, 0, 200, 100]}
        self.inventory["sources"][0]["header_fields"] = {
            field: {"status": "absent_in_source", "reason": "Synthetic single-paragraph source has no header field.", "evidence": [visual]}
            for field in checker.FIELDS
        }
        self.inventory["units"] = [{
            "id": "body", "source": "main", "kind": "paragraph",
            "evidence": [{"page": 1, "basis": "text", "quote": self.raw_text,
                          "blocks": [block["id"] for block in page_record["blocks"]]}],
        }]
        self.candidate = self.base.joinpath("candidate.md")
        self.candidate.write_text("---\n类型: 测试片段\n年份: null\n关键词: []\n---\n# 合成片段\n\nAXB\n", encoding="utf-8")
        self.manifest = self.base.joinpath("delivery-manifest.json")

    def report(self):
        self.inventory_path.write_text(json.dumps(self.inventory), encoding="utf-8")
        checker.write_delivery_manifest(
            self.inventory_path, self.candidate,
            [{"unit": "body", "target": {"quote": "AXB"}}], self.manifest)
        return checker.validate_delivery(self.manifest)

    def test_source_text_is_written_and_compared_without_universal_newline_conversion(self):
        self.assertEqual(self.text_path.read_bytes(), self.raw_text.encode("utf-8"))
        self.assertEqual(self.report().findings, [])

    def test_rehashed_normalised_text_is_still_not_the_original_text_layer(self):
        self.text_path.write_bytes(self.raw_text.replace("\r", "\n").encode("utf-8"))
        self.inventory["sources"][0]["pages"][0]["text"] = checker.source_tools.file_record(self.text_path)
        self.assertIn("TEXT_PROVENANCE", {finding.code for finding in self.report().findings})


if __name__ == "__main__":
    unittest.main()
