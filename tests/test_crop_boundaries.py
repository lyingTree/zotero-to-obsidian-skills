"""Source-boundary evidence, using only synthetic PDF pages and labels."""

from __future__ import annotations

import hashlib
import importlib.util
from io import BytesIO
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import fitz
from PIL import Image, ImageChops


SCRIPTS = Path(__file__).resolve().parents[1].joinpath("literature-zotero2obsidian", "scripts")
SPEC = importlib.util.spec_from_file_location("test_crop_pdf_objects", SCRIPTS.joinpath("pdf_image_objects.py"))
assert SPEC and SPEC.loader
images = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = images
SPEC.loader.exec_module(images)


class CropBoundaryTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.pdf = self.base.joinpath("source.pdf")
        self.bbox = [100, 80, 280, 220]
        self.make_pdf()

    def make_pdf(self, *, labels=("left",), invisible=False, rotation=0, cropbox=False, multiline=False):
        stream = BytesIO()
        with Image.new("RGB", (180, 140), (220, 235, 250)) as background:
            background.save(stream, format="PNG")
        offset = fitz.Point(20, 30) if cropbox else fitz.Point(0, 0)
        with fitz.open() as document:
            page = document.new_page(width=400, height=300)
            rectangle = fitz.Rect(self.bbox) + (offset.x, offset.y, offset.x, offset.y)
            page.insert_image(rectangle, stream=stream.getvalue())
            text = {"left": ((95, 130), "Asterisk note"),
                    "right": ((275, 160), "Right note"),
                    "top": ((150, 81), "Top note"),
                    "bottom": ((160, 219), "Bottom note"),
                    "external_left": ((65, 175), "Out"),
                    "external_top": ((150, 65), "External caption")}
            for label in labels:
                point, value = text[label]
                page.insert_text(fitz.Point(point) + offset, value, fontsize=12)
            if invisible:
                page.insert_text(fitz.Point(95, 110) + offset, "Hidden OCR", fontsize=12, render_mode=3)
                page.insert_text(fitz.Point(95, 190) + offset, "Zero opacity", fontsize=12, fill_opacity=0)
            if multiline:
                page.insert_text(fitz.Point(120, 70) + offset, ["Separate heading", "Crossing line"], fontsize=12)
            if cropbox:
                page.set_cropbox(fitz.Rect(20, 30, 380, 280))
            page.set_rotation(rotation)
            document.save(self.pdf)

    def inspect(self, **kwargs):
        options = {"page_number": 1, "bbox": self.bbox, "coordinate_system": images.PDF_BBOX_COORDINATES,
                   "context_destination": self.base.joinpath("context.png"), "progress": None}
        options.update(kwargs)
        return images.inspect_crop_boundaries(self.pdf, **options)

    def render(self, **kwargs):
        options = {"page_number": 1, "bbox": self.bbox, "coordinate_system": images.PDF_BBOX_COORDINATES,
                   "destination": self.base.joinpath("final.png"), "margin_pt": 0, "dpi": 72, "progress": None}
        options.update(kwargs)
        return images.render_page_region(self.pdf, **options)

    def assert_source_pixels(self, asset, bbox, dpi):
        with fitz.open(self.pdf) as document:
            page = document[0]
            pixmap = page.get_pixmap(dpi=dpi, colorspace=fitz.csRGB, alpha=False)
            with Image.frombytes("RGB", (pixmap.width, pixmap.height), pixmap.samples) as full:
                display_box = fitz.Rect(bbox) * page.rotation_matrix * fitz.Matrix(dpi / 72, dpi / 72)
                expected = full.crop(tuple(display_box.irect))
        with Image.open(asset["path"]) as actual:
            self.assertEqual(actual.size, expected.size)
            self.assertIsNone(ImageChops.difference(actual.convert("RGB"), expected).getbbox())

    def test_image_bbox_clips_a_leading_letter_and_returns_source_evidence(self):
        result = self.render()
        check = result["boundary_check"]
        self.assertEqual(result["requested_bbox"], self.bbox)
        self.assertEqual(result["rendered_bbox"], self.bbox)
        self.assertEqual(check["checked_bbox"], self.bbox)
        self.assertEqual(check["status"], "crossing_text_detected")
        row, = check["edges"]["left"]["crossing_text"]
        self.assertEqual(row["text"], "Asterisk note")
        self.assertEqual(row["bbox"][0], 95)
        self.assertEqual([char["text"] for char in row["boundary_crossing_characters"]], ["A"])
        self.assertEqual(check["suggested_bbox"], [93, 80, 280, 220])
        self.assertFalse(check["suggestion_applied"])
        self.assertEqual(check["figure_membership"], "not_assessed")
        self.assertEqual(check["visual_review"]["status"], "pending")
        self.assertTrue(all(row["status"] == "pending" for row in result["edge_reviews"]))
        self.assertTrue(all(amount > 0 for amount in check["context"]["outside_crop_pt"].values()))
        for field in ("asset", "annotated_asset"):
            asset = check["context"][field]
            self.assertEqual(set(asset), {"path", "sha256"})
            self.assertEqual(hashlib.sha256(Path(asset["path"]).read_bytes()).hexdigest(), asset["sha256"])
        self.assertNotEqual(check["context"]["asset"]["sha256"], check["context"]["annotated_asset"]["sha256"])
        self.assert_source_pixels(result["asset"], self.bbox, 72)
        self.assert_source_pixels(check["context"]["asset"], check["context"]["bbox"], 144)

    def test_each_source_edge_has_real_text_and_geometry(self):
        self.make_pdf(labels=("left", "right", "top", "bottom"))
        result = self.inspect()
        expected = {"left": "Asterisk note", "right": "Right note", "top": "Top note", "bottom": "Bottom note"}
        self.assertEqual(result["crossing_text_count"], 4)
        for edge, text in expected.items():
            with self.subTest(edge=edge):
                evidence = result["edges"][edge]
                self.assertEqual(evidence["status"], "crossing_text_detected")
                row, = evidence["crossing_text"]
                self.assertEqual(row["text"], text)
                self.assertGreater(row["outward_extent_pt"][edge], 0)
                self.assertEqual(len(row["bbox"]), 4)
                self.assertTrue(evidence["hint"])
                self.assertTrue(fitz.Rect(result["suggested_bbox"]).contains(fitz.Rect(row["bbox"])))

    def test_existing_inclusive_bbox_produces_no_automatic_acceptance(self):
        self.make_pdf(labels=("left", "right", "top", "bottom"))
        result = self.inspect(bbox=[90, 60, 360, 240])
        self.assertEqual(result["status"], "no_crossing_text_detected")
        self.assertEqual(result["crossing_text_count"], 0)
        self.assertIsNone(result["suggested_bbox"])
        self.assertEqual(result["visual_review"]["figure_completeness"], "not_assessed")
        self.assertTrue(result["full_page_review_required"])

    def test_complete_external_labels_are_clues_and_do_not_expand_the_suggestion(self):
        self.make_pdf(labels=("left", "external_left", "external_top"))
        result = self.inspect()
        self.assertEqual(result["suggested_bbox"], [93, 80, 280, 220])
        self.assertEqual([row["text"] for row in result["edges"]["left"]["nearby_external_text"]], ["Out"])
        self.assertEqual([row["text"] for row in result["edges"]["top"]["nearby_external_text"]], ["External caption"])
        self.assertEqual(result["edges"]["top"]["crossing_text"], [])

    def test_one_pdf_span_with_two_baselines_does_not_merge_external_text(self):
        self.make_pdf(labels=(), multiline=True)
        result = self.inspect()
        crossing, = result["edges"]["top"]["crossing_text"]
        self.assertEqual(crossing["text"], "Crossing line")
        self.assertEqual([row["text"] for row in result["edges"]["top"]["nearby_external_text"]], ["Separate heading"])
        self.assertGreater(result["suggested_bbox"][1], 70)

    def test_inspection_checks_actual_rendered_margin_without_changing_it(self):
        result = self.render(margin_pt=6)
        self.assertEqual(result["rendered_bbox"], [94, 74, 286, 226])
        self.assertEqual(result["boundary_check"]["checked_bbox"], result["rendered_bbox"])
        self.assertEqual(result["boundary_check"]["crossing_text_count"], 0)
        self.assertIsNone(result["boundary_check"]["suggested_bbox"])
        self.assert_source_pixels(result["asset"], result["rendered_bbox"], 72)

    def test_rotated_cropped_page_keeps_unrotated_evidence_and_display_pixels(self):
        self.make_pdf(rotation=90, cropbox=True)
        before = self.pdf.read_bytes()
        result = self.render()
        check = result["boundary_check"]
        self.assertEqual(check["page_geometry"]["page_rotation"], 90)
        self.assertEqual(check["page_geometry"]["page_bounds"], [0, 0, 360, 250])
        self.assertEqual(check["edges"]["left"]["crossing_text"][0]["bbox"][0], 95)
        self.assertEqual(check["suggested_bbox"], [93, 80, 280, 220])
        self.assertEqual(self.pdf.read_bytes(), before)
        self.assert_source_pixels(result["asset"], self.bbox, 72)
        self.assert_source_pixels(check["context"]["asset"], check["context"]["bbox"], 144)
        transform = fitz.Matrix(check["context"]["page_to_pixel_transform"])
        marked_crop = fitz.Rect(self.bbox) * transform
        with Image.open(check["context"]["annotated_asset"]["path"]) as annotated:
            # The outline uses the same exact rotated map supplied to a reviewer.
            self.assertEqual(annotated.getpixel((round(marked_crop.x0), round(marked_crop.y0))), (220, 45, 55))

    def test_no_extractable_text_and_invisible_text_keep_review_pending(self):
        self.make_pdf(labels=(), invisible=True)
        result = self.inspect()
        self.assertEqual(result["status"], "no_extractable_painted_text_in_context")
        self.assertEqual(result["text_extraction"]["ignored_invisible_spans"], 2)
        self.assertEqual(result["crossing_text_count"], 0)
        self.assertIsNone(result["suggested_bbox"])
        self.assertEqual(result["visual_review"]["status"], "pending")
        self.assertTrue(result["full_page_review_required"])

    def test_image_only_page_still_exports_source_context(self):
        self.make_pdf(labels=())
        result = self.inspect()
        self.assertEqual(result["status"], "no_extractable_painted_text_in_context")
        self.assertEqual(result["text_extraction"]["paint_spans"], 0)
        self.assertTrue(Path(result["context"]["asset"]["path"]).is_file())
        self.assertEqual(result["visual_review"]["status"], "pending")

    def test_full_page_crop_records_that_no_outside_page_pixels_exist(self):
        result = self.inspect(bbox=[0, 0, 400, 300])
        self.assertEqual(result["context"]["bbox"], [0, 0, 400, 300])
        self.assertEqual(result["context"]["outside_crop_pt"], {edge: 0 for edge in images.CROP_EDGES})
        self.assertEqual(result["visual_review"]["status"], "pending")

    def test_changed_source_hash_is_rejected_before_any_new_output(self):
        selected_hash = hashlib.sha256(self.pdf.read_bytes()).hexdigest()
        with fitz.open(self.pdf) as document:
            document[0].insert_text((10, 10), "Changed source")
            document.saveIncr()
        with self.assertRaisesRegex(ValueError, "source PDF hash"):
            self.inspect(expected_source_sha256=selected_hash)
        with self.assertRaisesRegex(ValueError, "source PDF hash"):
            self.render(expected_source_sha256=selected_hash)
        self.assertEqual(list(self.base.glob("*.png")), [])

    def test_source_change_during_render_does_not_bind_stale_hash_to_new_evidence(self):
        original = images._render_pixels

        def changed_source(*args, **kwargs):
            pixels = original(*args, **kwargs)
            with self.pdf.open("ab") as stream:
                stream.write(b"\n")
            return pixels

        with patch.object(images, "_render_pixels", side_effect=changed_source):
            with self.assertRaisesRegex(ValueError, "source PDF hash"):
                self.inspect()
        self.assertEqual(list(self.base.glob("*.png")), [])

    def test_context_is_write_once_and_identical_results_are_reusable(self):
        first = self.inspect()
        second = self.inspect()
        self.assertEqual(first, second)
        path = Path(first["context"]["annotated_asset"]["path"])
        path.write_bytes(b"existing annotation evidence")
        with self.assertRaises(FileExistsError):
            self.inspect()
        self.assertEqual(path.read_bytes(), b"existing annotation evidence")

    def test_context_symlink_and_aliasing_final_destination_are_rejected(self):
        target = self.base.joinpath("retained.png")
        target.write_bytes(b"retained evidence")
        link = self.base.joinpath("context.png")
        link.symlink_to(target)
        with self.assertRaises(FileExistsError):
            self.inspect()
        self.assertEqual(target.read_bytes(), b"retained evidence")
        with self.assertRaisesRegex(ValueError, "destinations must be different"):
            self.render(context_destination=self.base.joinpath("final.png"))

    def test_unavailable_text_extraction_is_explicit_and_still_has_source_context(self):
        with patch.object(fitz.Page, "get_texttrace", side_effect=RuntimeError("synthetic extraction failure")):
            result = self.inspect()
        self.assertEqual(result["status"], "incomplete_text_evidence")
        self.assertEqual(result["text_extraction"]["status"], "unavailable")
        self.assertTrue(all(edge["status"] == "text_evidence_unresolved" for edge in result["edges"].values()))
        self.assertTrue(Path(result["context"]["asset"]["path"]).is_file())
        self.assertEqual(result["visual_review"]["status"], "pending")

    def test_failed_extraction_restores_rotation_before_context_render(self):
        self.make_pdf(rotation=90, cropbox=True)

        def failed_after_derotation(page):
            page.set_rotation(0)
            raise RuntimeError("synthetic failure after temporary derotation")

        with patch.object(fitz.Page, "get_texttrace", new=failed_after_derotation):
            result = self.render()
        self.assertEqual(result["boundary_check"]["status"], "incomplete_text_evidence")
        self.assert_source_pixels(result["asset"], self.bbox, 72)
        context = result["boundary_check"]["context"]
        self.assert_source_pixels(context["asset"], context["bbox"], context["dpi"])

    def test_context_requires_valid_pdf_coordinates_and_real_outside_margin(self):
        with self.assertRaisesRegex(ValueError, "PDF-point"):
            self.inspect(coordinate_system={**images.PDF_BBOX_COORDINATES, "unit": "px"})
        for margin in (0, -1, float("nan"), 10 ** 1000, True):
            with self.subTest(margin=margin), self.assertRaises(ValueError):
                self.inspect(context_margin_pt=margin)
        for bbox in ([1, 2], [0, 0, 1000, 1000], [4, 4, 2, 2]):
            with self.subTest(bbox=bbox), self.assertRaises(ValueError):
                self.inspect(bbox=bbox)

    def test_boundary_extraction_and_context_render_report_progress(self):
        calls = []
        self.inspect(progress=lambda done, total, label: calls.append((done, total, label)))
        self.assertTrue(any("text paint" in label for _, _, label in calls))
        self.assertTrue(any("boundary context" in label for _, _, label in calls))
        self.assertTrue(all(0 <= done <= total for done, total, _ in calls))


if __name__ == "__main__":
    unittest.main(verbosity=2)
