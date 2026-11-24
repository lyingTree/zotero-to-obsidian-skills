"""Native Word provenance uses XML locations and image calls, never fake pages."""
from __future__ import annotations

import base64
import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT.joinpath("literature-zotero2obsidian", "scripts")
SPEC = importlib.util.spec_from_file_location("test_native_docx_tools", SCRIPTS.joinpath("docx_source_inventory.py"))
assert SPEC and SPEC.loader
native = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = native
SPEC.loader.exec_module(native)
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aV1EAAAAASUVORK5CYII=")


def paragraph(text):
    return f"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"


def image_call(relation_id, *, floating=False):
    wrapper = "anchor" if floating else "inline"
    return (f"<w:p><w:r><w:drawing><wp:{wrapper}><a:graphic><a:graphicData>"
            f'<a:blip r:embed="{relation_id}"/>'
            f"</a:graphicData></a:graphic></wp:{wrapper}></w:drawing></w:r></w:p>")


def make_docx(path, body, *, relationships=(), media=None, extra_parts=None):
    media = media or {}
    document = (
        f'<w:document xmlns:w="{native.W}" xmlns:r="{native.R}" xmlns:a="{native.A}" '
        f'xmlns:m="{native.M}" xmlns:wp="{native.WP}"><w:body>{body}<w:sectPr/></w:body></w:document>'
    ).encode()
    rels = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(
        '<Relationship ' + " ".join(f'{key}="{value}"' for key, value in row.items()) + '/>' for row in relationships
    ) + '</Relationships>'
    types = ('<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
             '<Default Extension="png" ContentType="image/png"/>'
             '<Default Extension="xml" ContentType="application/xml"/>'
             '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
             '</Types>')
    with ZipFile(path, "w") as package:
        package.writestr(native.DOCUMENT_PART, document)
        package.writestr(native.RELATIONSHIPS_PART, rels)
        package.writestr("[Content_Types].xml", types)
        for name, payload in {**media, **(extra_parts or {})}.items():
            package.writestr(name, payload)
    return document


def image_relation(identity, target, **changes):
    return {"Id": identity, "Type": native.R + "/image", "Target": target, **changes}


class NativeDocxTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.path = self.base.joinpath("supplement.docx")

    def test_document_order_and_multiple_media_do_not_invent_figure_numbers(self):
        body = paragraph("Before") + image_call("rId9") + image_call("rId3") + paragraph("Figure S1. Both panels.")
        document = make_docx(self.path, body,
                             relationships=[image_relation("rId9", "media/image2.png"), image_relation("rId3", "media/image1.png")],
                             media={"word/media/image1.png": PNG, "word/media/image2.png": PNG + b"second"})
        result = native.prepare_docx_source(self.path, "si", self.base.joinpath("evidence"))
        self.assertEqual([row["text"] for row in result["native_blocks"]], ["Before", "", "", "Figure S1. Both panels."])
        self.assertEqual([row["relationship_id"] for row in result["media_calls"]], ["rId9", "rId3"])
        self.assertEqual([row["media_id"] for row in result["media_calls"]], ["si-media-2", "si-media-1"])
        self.assertTrue(all(row["figure_number"] is None for row in result["media_calls"]))
        self.assertEqual(result["media_calls"][0]["nearby_text"]["next"], "Figure S1. Both panels.")
        self.assertEqual(result["pages"], [])
        self.assertIsNone(result["page_count"])
        self.assertFalse(any("page" in row["locator"] for row in result["native_blocks"]))
        self.assertEqual(Path(result["native_document"]["path"]).read_bytes(), document)
        for record in result["native_media"]:
            data = Path(record["file"]["path"]).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), record["sha256"])
        self.assertEqual(result["native_issues"], [])
        with self.assertRaises(FileExistsError):
            native.prepare_docx_source(self.path, "si", self.base.joinpath("evidence"))

    def test_table_rows_cells_and_image_parent_follow_actual_xml_order(self):
        table = ('<w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr>'
                 + paragraph("Joined heading") + '</w:tc></w:tr><w:tr><w:tc>'
                 + paragraph("x") + '</w:tc><w:tc>' + image_call("rImage") + '</w:tc></w:tr></w:tbl>')
        make_docx(self.path, paragraph("Lead") + table + paragraph("Tail"),
                  relationships=[image_relation("rImage", "media/native.png")], media={"word/media/native.png": PNG})
        result = native.read_docx(self.path, "si")
        self.assertEqual([row["kind"] for row in result["native_blocks"]], ["paragraph", "table", "paragraph"])
        block = result["native_blocks"][1]
        self.assertEqual(block["rows"][0]["cells"][0]["grid_span"], "2")
        self.assertEqual(block["text"], "Joined heading\nx\t")
        self.assertEqual(result["media_calls"][0]["block_id"], block["id"])

    def test_cell_content_control_keeps_an_explicit_pending_xml_record(self):
        controlled = ('<w:sdt><w:sdtPr/><w:sdtContent>'
                      + paragraph("The retained tracer concentration is 37 mg/L.")
                      + image_call("rHidden") + '</w:sdtContent></w:sdt>')
        original = make_docx(self.path, '<w:tbl><w:tr><w:tc>' + paragraph("Before.") + controlled
                             + paragraph("After.") + '</w:tc></w:tr></w:tbl>',
                             relationships=[image_relation("rHidden", "media/hidden.png")],
                             media={"word/media/hidden.png": PNG})
        source = native.read_docx(self.path, "si")
        block = source["native_blocks"][0]
        self.assertEqual(block["text"], "Before.\nAfter.")
        pending = next(row for row in source["native_issues"] if row["kind"] == "table_content_container")
        self.assertEqual(pending["status"], "pending")
        self.assertEqual(pending["block_id"], block["id"])
        raw = block["unparsed_content"][0]
        self.assertEqual(raw["issue"], pending["id"])
        self.assertEqual(raw["locator"], pending["locator"])
        element = ET.fromstring(original)
        for index in raw["locator"]["element_path"]:
            element = element[index]
        self.assertEqual(element.tag, f"{{{native.W}}}sdt")
        self.assertEqual(raw["xml"], ET.tostring(element, encoding="unicode"))
        self.assertIn("37 mg/L", raw["xml"])
        self.assertIn("rHidden", raw["xml"])
        self.assertTrue(any(row["kind"] == "unresolved_media_use" for row in source["native_issues"]))

    def test_unknown_table_row_and_cell_containers_remain_pending(self):
        sentence = paragraph("Do not silently omit this source content.")
        cases = {
            "table": '<w:tbl><w:customXml><w:tr><w:tc>' + sentence + '</w:tc></w:tr></w:customXml></w:tbl>',
            "row": '<w:tbl><w:tr><w:sdt><w:sdtContent><w:tc>' + sentence + '</w:tc></w:sdtContent></w:sdt></w:tr></w:tbl>',
            "cell": '<w:tbl><w:tr><w:tc><w:customXml>' + sentence + '</w:customXml></w:tc></w:tr></w:tbl>',
        }
        for container, body in cases.items():
            with self.subTest(container=container):
                make_docx(self.path, body)
                source = native.read_docx(self.path, "si")
                self.assertEqual(len(source["native_issues"]), 1)
                issue = source["native_issues"][0]
                self.assertEqual(issue["kind"], "table_content_container")
                self.assertEqual(issue["status"], "pending")
                record = source["native_blocks"][0]["unparsed_content"][0]
                self.assertEqual(record["container"], container)
                self.assertIn("Do not silently omit", record["xml"])

    def test_table_properties_and_bookmarks_are_not_content_failures(self):
        body = ('<w:tbl><w:tblPr/><w:tblGrid/><w:bookmarkStart w:id="1" w:name="table"/>'
                '<w:tr><w:tblPrEx/><w:trPr/><w:bookmarkEnd w:id="1"/><w:tc><w:tcPr/>'
                '<w:proofErr w:type="spellStart"/>' + paragraph("A complete ordinary cell.")
                + '</w:tc></w:tr></w:tbl>')
        make_docx(self.path, body)
        source = native.read_docx(self.path, "si")
        self.assertEqual(source["native_issues"], [])
        self.assertEqual(source["native_blocks"][0]["text"], "A complete ordinary cell.")

    def test_nonbreaking_hyphen_keeps_character_event_and_exact_xml_locator(self):
        for left, right in (("4", "7 units"), ("ID", "17"), ("storm", "driven")):
            with self.subTest(left=left, right=right):
                original = make_docx(self.path, '<w:p><w:r><w:t>' + left
                                     + '</w:t><w:noBreakHyphen/><w:t>' + right + '</w:t></w:r></w:p>')
                source = native.read_docx(self.path, "si")
                block = source["native_blocks"][0]
                self.assertEqual(block["text"], left + "\u2011" + right)
                self.assertEqual(source["native_issues"], [])
                event = block["inline_content"][1]
                self.assertEqual(event["kind"], "noBreakHyphen")
                self.assertEqual(event["text"], "\u2011")
                self.assertEqual(event["locator"], {"part": native.DOCUMENT_PART, "element_path": [0, 0, 0, 1]})
                element = ET.fromstring(original)
                for index in event["locator"]["element_path"]:
                    element = element[index]
                self.assertEqual(element.tag, f"{{{native.W}}}noBreakHyphen")
                self.assertEqual(native.NativeDocument(self.path).resolve_native_locator(block["locator"]), left + "\u2011" + right)

    def test_permission_range_markers_do_not_create_unparsed_table_content(self):
        markers = '<w:permStart w:id="1" w:edGrp="everyone"/><w:permEnd w:id="1"/>'
        body = ('<w:tbl>' + markers + '<w:tr>' + markers + '<w:tc>' + markers
                + paragraph("Visible scientific content.") + '</w:tc></w:tr></w:tbl>')
        make_docx(self.path, body)
        source = native.read_docx(self.path, "si")
        self.assertEqual(source["native_issues"], [])
        block = source["native_blocks"][0]
        self.assertEqual(block["text"], "Visible scientific content.")
        self.assertNotIn("unparsed_content", block)

    def test_missing_external_and_traversal_relationships_are_explicit(self):
        make_docx(self.path, image_call("missing") + image_call("external") + image_call("escape"),
                  relationships=[image_relation("external", "https://example.invalid/image.png", TargetMode="External"),
                                 image_relation("escape", "../../../../outside.png")])
        result = native.read_docx(self.path, "si")
        self.assertEqual([row["status"] for row in result["media_calls"]],
                         ["missing_relationship", "external_relationship", "missing_media"])
        self.assertEqual(len(result["native_issues"]), 3)
        self.assertTrue(all(row["status"] == "pending" for row in result["native_issues"]))
        self.assertEqual(result["native_media"], [])

    def test_math_and_floating_objects_are_pending_not_silent_text(self):
        equation = '<w:p><w:r><w:t>Rate </w:t></w:r><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></w:p>'
        make_docx(self.path, equation + image_call("rImage", floating=True),
                  relationships=[image_relation("rImage", "media/native.png")], media={"word/media/native.png": PNG})
        result = native.read_docx(self.path, "si")
        self.assertEqual({row["kind"] for row in result["native_issues"]}, {"math", "floating_object"})
        self.assertEqual(result["native_blocks"][0]["text"], "Rate ")
        self.assertEqual(result["media_calls"][0]["status"], "resolved")

    def test_bookmarks_and_dpi_metadata_do_not_create_spurious_content_issues(self):
        picture = image_call("rImage").replace('<a:blip r:embed="rImage"/>',
            '<a:blip r:embed="rImage"><a:extLst><a:ext uri="fixture">'
            '<a14:useLocalDpi xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" val="0"/>'
            '</a:ext></a:extLst></a:blip>')
        make_docx(self.path, '<w:bookmarkStart w:id="0" w:name="fixture"/>' + paragraph("Title")
                  + '<w:bookmarkEnd w:id="0"/>' + picture,
                  relationships=[image_relation("rImage", "media/a.png")], media={"word/media/a.png": PNG})
        source = native.read_docx(self.path, "si")
        self.assertEqual([row["kind"] for row in source["native_blocks"]], ["paragraph", "paragraph"])
        self.assertEqual(source["native_issues"], [])
        self.assertEqual(source["native_blocks"][0]["locator"]["element_path"], [0, 1])

    def test_text_only_document_has_native_blocks_without_media_or_fake_pages(self):
        make_docx(self.path, paragraph("A complete native paragraph."))
        result = native.read_docx(self.path, "si")
        self.assertEqual(result["native_media"], [])
        self.assertEqual(result["media_calls"], [])
        locator = result["native_blocks"][0]["locator"]
        document = native.NativeDocument(self.path)
        self.assertEqual(document.resolve_native_locator(locator), "A complete native paragraph.")
        for invalid in ({**locator, "page": 1}, {"part": native.DOCUMENT_PART, "element_path": [0, 90]}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                document.resolve_native_locator(invalid)

    def test_native_location_requires_exact_block_or_call_and_no_page(self):
        make_docx(self.path, paragraph("First") + paragraph("Second"))
        source = native.read_docx(self.path, "si")
        first, second = source["native_blocks"]
        valid = {"basis": "native", "native_locator": first["locator"], "blocks": [first["id"]], "quote": "First"}
        self.assertEqual(native.native_evidence_errors(source, valid), [])
        for wrong in ({**valid, "page": 1}, {**valid, "blocks": [first["id"], second["id"]]}, {**valid, "quote": "Second"}):
            with self.subTest(wrong=wrong):
                self.assertTrue(native.native_evidence_errors(source, wrong))

    def test_non_docx_and_duplicate_parts_fail_cleanly(self):
        self.path.write_bytes(b"%PDF-1.7\n")
        with self.assertRaises(ValueError):
            native.read_docx(self.path, "si")


if __name__ == "__main__":
    unittest.main()
