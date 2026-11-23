"""PDF image identities and exports, using exclusively synthetic source files."""

from __future__ import annotations

import hashlib
import importlib.util
from io import BytesIO
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz
from PIL import Image, ImageChops, ImageCms, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT.joinpath("literature-zotero2obsidian", "scripts")


def load_script(name):
    spec = importlib.util.spec_from_file_location("test_pdf_objects_" + name, SCRIPTS.joinpath(name + ".py"))
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


images = load_script("pdf_image_objects")
prepare = load_script("prepare_source_inventory")


def encoded(image, format):
    stream = BytesIO()
    image.save(stream, format=format)
    return stream.getvalue()


def append_content(document, page, content):
    xref = document.get_new_xref()
    document.update_object(xref, "<<>>")
    document.update_stream(xref, content)
    page.set_contents(xref)


def add_xobject(document, page, name, xref):
    resources_xref = int(document.xref_get_key(page.xref, "Resources")[1].split()[0])
    document.xref_set_key(resources_xref, "XObject/" + name, f"{xref} 0 R")


class ImageObjectTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.pdf = self.base.joinpath("synthetic.pdf")
        sample = Image.new("RGB", (80, 40), (30, 80, 200))
        ImageDraw.Draw(sample).rectangle((1, 1, 78, 38), outline=(240, 70, 40), width=2)
        self.jpeg = encoded(sample, "JPEG")
        alpha = Image.new("RGBA", (40, 40), (20, 180, 60, 255))
        ImageDraw.Draw(alpha).rectangle((0, 0, 9, 39), fill=(20, 180, 60, 0))
        ImageDraw.Draw(alpha).rectangle((10, 0, 19, 39), fill=(20, 180, 60, 128))
        self.alpha_png = encoded(alpha, "PNG")
        with fitz.open() as document:
            page = document.new_page(width=400, height=300)
            self.jpeg_xref = page.insert_image(fitz.Rect(30, 40, 190, 120), stream=self.jpeg)
            page.insert_image(fitz.Rect(220, 40, 380, 120), xref=self.jpeg_xref)
            page.insert_text((30, 30), "External panel label A")
            page.draw_line(fitz.Point(20, 40), fitz.Point(20, 120), color=(0, 0, 0))
            page = document.new_page(width=400, height=300)
            page.insert_image(fitz.Rect(50, 80, 210, 160), xref=self.jpeg_xref)
            page = document.new_page(width=400, height=300)
            self.alpha_xref = page.insert_image(fitz.Rect(30, 40, 70, 80), stream=self.alpha_png)
            self.mask_xref = document.extract_image(self.alpha_xref)["smask"]
            page = document.new_page(width=400, height=300)
            page.insert_image(fitz.Rect(70, 90, 230, 170), xref=self.jpeg_xref, rotate=90, keep_proportion=False)
            page.set_cropbox(fitz.Rect(20, 30, 380, 280))
            page.set_rotation(90)
            # Explicit device spaces isolate native JPEG/SMask tests from the
            # library's optional automatically generated ICC profiles.
            document.xref_set_key(self.jpeg_xref, "ColorSpace", "/DeviceRGB")
            document.xref_set_key(self.alpha_xref, "ColorSpace", "/DeviceRGB")
            document.save(self.pdf)

    def inventory(self):
        path = prepare.prepare_sources({"main": self.pdf}, self.base.joinpath("work"),
                                       source_roles={"main": "main"}, dpi=72, progress=None)
        return json.loads(path.read_text(encoding="utf-8"))["sources"][0]

    def index(self, page_number=1):
        with fitz.open(self.pdf) as document, images.ImageObjectIndexer(self.pdf, document, progress=None) as indexer:
            return indexer.index_page(page_number)

    def export(self, *, xref=None, page=1, destination=None, **kwargs):
        return images.export_image_object(self.pdf, page_number=page,
                                          image_xref=self.jpeg_xref if xref is None else xref,
                                          destination=destination or self.base.joinpath("final"), progress=None, **kwargs)

    def render(self, **kwargs):
        options = {"page_number": 1, "bbox": [30, 40, 190, 120],
                   "coordinate_system": images.PDF_BBOX_COORDINATES,
                   "destination": self.base.joinpath("region.png"), "progress": None}
        options.update(kwargs)
        return images.render_page_region(self.pdf, **options)

    def test_preparation_indexes_each_actual_appearance_and_extracts_xref_once(self):
        source = self.inventory()
        self.assertEqual(len(source["image_objects"]), 2)
        first = source["pages"][0]["image_index"]
        self.assertEqual(first["status"], "indexed")
        self.assertEqual([row["image_xref"] for row in first["occurrences"]], [self.jpeg_xref] * 2)
        self.assertEqual([row["bbox"] for row in first["occurrences"]], [[30, 40, 190, 120], [220, 40, 380, 120]])
        self.assertEqual(source["pages"][1]["image_index"]["occurrences"][0]["image_xref"], self.jpeg_xref)
        obj = next(row for row in source["image_objects"] if row["image_xref"] == self.jpeg_xref)
        self.assertEqual(obj["original_encoded_extension"], "jpeg")
        self.assertEqual(obj["extracted_extension"], "jpeg")
        self.assertEqual(Path(obj["raw_asset"]["path"]).read_bytes(), self.jpeg)
        self.assertEqual(obj["raw_asset"]["sha256"], hashlib.sha256(self.jpeg).hexdigest())
        self.assertEqual(set(obj["raw_asset"]), {"path", "sha256"})
        self.assertEqual(len(list(self.base.joinpath("work", "source-evidence", "main", "image-objects").iterdir())), 3)

    def test_export_preserves_original_jpeg_bytes(self):
        result = self.export(destination=self.base.joinpath("final.jpg"))
        self.assertEqual(Path(result["asset"]["path"]).read_bytes(), self.jpeg)
        self.assertEqual(result["method"], "original_extracted_bytes")
        self.assertEqual(result["source_object"]["smask"]["status"], "absent")
        self.assertEqual(set(result["asset"]), {"path", "sha256"})

    def test_suffix_is_derived_from_encoding_and_wrong_suffix_is_rejected(self):
        self.assertTrue(self.export()["asset"]["path"].endswith(".jpeg"))
        wrong = self.base.joinpath("wrong.png")
        with self.assertRaisesRegex(ValueError, "extension"):
            self.export(destination=wrong)
        self.assertFalse(wrong.exists())

    def test_identical_destination_is_reused_and_different_content_is_preserved(self):
        first = self.export()
        self.assertEqual(self.export()["asset"], first["asset"])
        output = Path(first["asset"]["path"])
        output.write_bytes(b"preexisting evidence")
        with self.assertRaises(FileExistsError):
            self.export()
        self.assertEqual(output.read_bytes(), b"preexisting evidence")

    def test_symlink_destination_is_not_followed(self):
        target = self.base.joinpath("keep.jpeg")
        target.write_bytes(self.jpeg)
        link = self.base.joinpath("link.jpeg")
        try:
            link.symlink_to(target)
        except OSError as error:
            self.skipTest(f"Creating a symlink is unavailable: {error}")
        with self.assertRaises(FileExistsError):
            self.export(destination=link)
        self.assertEqual(target.read_bytes(), self.jpeg)

    def test_same_pixel_digest_does_not_merge_distinct_actual_xrefs(self):
        with fitz.open(self.pdf) as document:
            page = document[0]
            duplicate = document.get_new_xref()
            document.update_object(duplicate, document.xref_object(self.jpeg_xref))
            document.update_stream(duplicate, self.jpeg, compress=False)
            document.xref_set_key(duplicate, "Filter", "/DCTDecode")
            add_xobject(document, page, "Distinct", duplicate)
            append_content(document, page, b"q 80 0 0 40 30 220 cm /fzImg0 Do Q\n"
                           b"q 80 0 0 40 150 220 cm /Distinct Do Q\n")
            self.assertEqual(fitz.Pixmap(document, duplicate).digest, fitz.Pixmap(document, self.jpeg_xref).digest)
            document.saveIncr()
        rows = self.index()["occurrences"]
        self.assertEqual([row["image_xref"] for row in rows], [self.jpeg_xref, duplicate])
        self.assertEqual(rows[0]["bbox"], [30, 40, 110, 80])
        self.assertEqual(rows[1]["bbox"], [150, 40, 230, 80])

    def test_unused_resource_entry_is_not_an_actual_appearance(self):
        with fitz.open(self.pdf) as document:
            add_xobject(document, document[0], "Unused", self.alpha_xref)
            document.saveIncr()
        self.assertNotIn(self.alpha_xref, [row["image_xref"] for row in self.index()["occurrences"]])
        with self.assertRaisesRegex(ValueError, "not actually invoked"):
            self.export(xref=self.alpha_xref)

    def test_nonimage_and_out_of_range_xrefs_are_rejected(self):
        with fitz.open(self.pdf) as document:
            page_xref, limit = document[0].xref, document.xref_length()
        for invalid in (0, -1, True, "4", page_xref, limit, limit + 100):
            with self.subTest(xref=invalid), self.assertRaises(ValueError):
                self.export(xref=invalid)
        self.assertFalse(self.base.joinpath("final.jpeg").exists())

    def test_wrong_page_and_invalid_page_numbers_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "not actually invoked"):
            self.export(page=3)
        for page in (0, -1, True, 1.0, 5):
            with self.subTest(page=page), self.assertRaises(ValueError):
                self.export(page=page)

    def test_changed_source_hash_blocks_export_before_writing(self):
        with self.assertRaisesRegex(ValueError, "source PDF hash"):
            self.export(expected_source_sha256="0" * 64)
        self.assertFalse(self.base.joinpath("final.jpeg").exists())

    def test_transparent_mask_is_indexed_and_composited(self):
        source = self.inventory()
        obj = next(row for row in source["image_objects"] if row["image_xref"] == self.alpha_xref)
        self.assertEqual(obj["smask"]["image_xref"], self.mask_xref)
        self.assertEqual(obj["smask"]["status"], "nonopaque")
        self.assertEqual((obj["smask"]["alpha_min"], obj["smask"]["alpha_max"]), (0, 255))
        self.assertEqual(images._hash_file(Path(obj["smask"]["raw_asset"]["path"])), obj["smask"]["raw_asset"]["sha256"])
        result = self.export(xref=self.alpha_xref, page=3)
        self.assertEqual(result["method"], "smask_composited_png")
        with Image.open(result["asset"]["path"]) as final:
            rgba = final.convert("RGBA")
        self.assertEqual([rgba.getpixel((x, 20))[3] for x in (5, 15, 25)], [0, 128, 255])
        white = Image.new("RGBA", rgba.size, "white")
        white.alpha_composite(rgba)
        with fitz.open(self.pdf) as document:
            pixels = document[2].get_pixmap(dpi=72, clip=fitz.Rect(30, 40, 70, 80), alpha=False)
        rendered = Image.frombytes("RGB", (pixels.width, pixels.height), pixels.samples)
        differences = ImageChops.difference(white.convert("RGB"), rendered).getextrema()
        # Independent PDF and Pillow compositors round 8-bit alpha differently;
        # exact opacity above plus a two-level color tolerance detects lost masks.
        self.assertTrue(all(high <= 2 for _, high in differences), differences)

    def test_an_opaque_smask_allows_original_jpeg_preservation(self):
        with fitz.open(self.pdf) as document:
            mask = document.get_new_xref()
            document.update_object(mask, "<</Type /XObject /Subtype /Image /Width 80 /Height 40 /ColorSpace /DeviceGray /BitsPerComponent 8>>")
            document.update_stream(mask, b"\xff" * (80 * 40))
            document.xref_set_key(self.jpeg_xref, "SMask", f"{mask} 0 R")
            document.saveIncr()
        result = self.export()
        self.assertEqual(result["source_object"]["smask"]["status"], "opaque")
        self.assertEqual(Path(result["asset"]["path"]).read_bytes(), self.jpeg)

    def test_mask_decode_is_applied_to_exported_alpha(self):
        with fitz.open(self.pdf) as document:
            document.xref_set_key(self.mask_xref, "Decode", "[1 0]")
            document.saveIncr()
        result = self.export(xref=self.alpha_xref, page=3)
        with Image.open(result["asset"]["path"]) as final:
            self.assertEqual([final.getpixel((x, 20))[3] for x in (5, 15, 25)], [255, 127, 0])

    def test_icc_colorspace_is_not_silently_dropped_from_original_jpeg(self):
        with fitz.open(self.pdf) as document:
            profile = document.get_new_xref()
            document.update_object(profile, "<</N 3 /Alternate /DeviceRGB>>")
            profile_bytes = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
            document.update_stream(profile, profile_bytes)
            document.xref_set_key(self.jpeg_xref, "ColorSpace", f"[/ICCBased {profile} 0 R]")
            document.saveIncr()
        with self.assertRaisesRegex(images.PageRenderRequired, "ICC"):
            self.export()

    def test_cmyk_jpeg_requires_pdf_color_rendering(self):
        cmyk = encoded(Image.new("CMYK", (80, 40), (10, 50, 20, 5)), "JPEG")
        with fitz.open() as document:
            page = document.new_page(width=400, height=300)
            xref = page.insert_image(fitz.Rect(30, 40, 190, 120), stream=cmyk)
            document.xref_set_key(xref, "ColorSpace", "/DeviceCMYK")
            self.pdf = self.base.joinpath("cmyk.pdf")
            document.save(self.pdf)
        with self.assertRaisesRegex(images.PageRenderRequired, "CMYK"):
            self.export(xref=xref)

    def test_smask_in_data_does_not_silently_export_without_its_effect(self):
        with fitz.open(self.pdf) as document:
            document.xref_set_key(self.jpeg_xref, "SMaskInData", "1")
            document.saveIncr()
        with self.assertRaisesRegex(images.PageRenderRequired, "SMaskInData"):
            self.export()

    def test_external_clipping_requires_page_render(self):
        with fitz.open(self.pdf) as document:
            append_content(document, document[0], b"q 30 220 40 40 re W n 80 0 0 40 30 220 cm /fzImg0 Do Q")
            document.saveIncr()
        self.assertIn("page_clipping_path", self.index()["occurrences"][0]["context_effects"])
        with self.assertRaisesRegex(images.PageRenderRequired, "clipping"):
            self.export()

    def test_matte_requires_page_render_instead_of_silently_dropping_mask(self):
        with fitz.open(self.pdf) as document:
            document.xref_set_key(self.mask_xref, "Matte", "[1 1 1]")
            document.saveIncr()
        with self.assertRaisesRegex(images.PageRenderRequired, "Matte"):
            self.export(xref=self.alpha_xref, page=3)
        result = self.render(page_number=3, bbox=[30, 40, 70, 80])
        self.assertTrue(Path(result["asset"]["path"]).is_file())
        self.assertEqual(result["visual_review"]["figure_completeness"], "not_assessed")

    def test_rotated_cropped_page_uses_unrotated_coordinates(self):
        before = self.pdf.read_bytes()
        result = self.index(4)
        self.assertEqual(result["status"], "indexed")
        self.assertEqual(result["page_rotation"], 90)
        self.assertEqual(result["page_bounds"], [0, 0, 360, 250])
        row = result["occurrences"][0]
        self.assertEqual(row["bbox"], [50, 60, 210, 140])
        self.assertEqual(list(fitz.Rect(0, 0, 1, 1) * fitz.Matrix(row["transform"])), row["bbox"])
        self.assertEqual(self.pdf.read_bytes(), before)
        with self.assertRaises(images.PageRenderRequired):
            self.export(page=4)
        rendered = self.render(page_number=4, bbox=row["bbox"], margin_pt=0, dpi=72)
        with Image.open(rendered["asset"]["path"]) as image:
            final = image.convert("RGB")
        self.assertEqual(final.size, (80, 160))
        with fitz.open(self.pdf) as document:
            page = document[3]
            source_pixels = page.get_pixmap(dpi=72, alpha=False)
            full = Image.frombytes("RGB", (source_pixels.width, source_pixels.height), source_pixels.samples)
            displayed_bbox = fitz.Rect(row["bbox"]) * page.rotation_matrix
            expected = full.crop(tuple(round(value) for value in displayed_bbox))
        self.assertIsNone(ImageChops.difference(final, expected).getbbox())

    def test_nested_form_image_calls_keep_actual_xref_and_transform(self):
        form_pdf = self.base.joinpath("form.pdf")
        with fitz.open() as original:
            page = original.new_page(width=100, height=100)
            page.insert_image(fitz.Rect(10, 20, 90, 60), stream=self.jpeg)
            original.save(form_pdf)
        with fitz.open() as outer, fitz.open(form_pdf) as original:
            page = outer.new_page(width=400, height=300)
            page.show_pdf_page(fitz.Rect(30, 40, 230, 240), original, 0)
            outer.save(self.base.joinpath("nested.pdf"))
        self.pdf = self.base.joinpath("nested.pdf")
        result = self.index()
        self.assertEqual(result["status"], "indexed")
        self.assertEqual(len(result["occurrences"]), 1)
        row = result["occurrences"][0]
        self.assertTrue(row["form_xrefs"])
        self.assertEqual(row["bbox"], [50, 80, 210, 160])
        with fitz.open(self.pdf) as document:
            self.assertEqual(document.extract_image(row["image_xref"])["image"], self.jpeg)

    def test_form_bbox_accepts_either_pair_of_opposite_corners(self):
        with fitz.open(self.pdf) as document:
            form = document.get_new_xref()
            document.update_object(form, f"<</Type /XObject /Subtype /Form /BBox [80 40 0 0] "
                                   f"/Resources <</XObject <</Native {self.jpeg_xref} 0 R>>>>>>")
            document.update_stream(form, b"q 80 0 0 40 0 0 cm /Native Do Q")
            add_xobject(document, document[0], "ReversedCorners", form)
            append_content(document, document[0], b"/ReversedCorners Do\n"
                           b"q 80 0 0 40 100 100 cm /fzImg0 Do Q")
            document.saveIncr()
        result = self.index()
        self.assertEqual(result["status"], "indexed")
        self.assertEqual(len(result["occurrences"]), 2)
        first = result["occurrences"][0]
        self.assertEqual(first["bbox"], [0, 260, 80, 300])
        self.assertEqual(first["form_clips"][0]["pdf_bbox"], [80, 40, 0, 0])
        self.assertEqual(first["form_clips"][0]["bbox"], [0, 0, 80, 40])
        self.assertEqual(Path(self.export()["asset"]["path"]).read_bytes(), self.jpeg)

    def test_zero_area_form_is_recorded_as_empty_clip_and_later_images_survive(self):
        with fitz.open(self.pdf) as document:
            form = document.get_new_xref()
            document.update_object(form, f"<</Type /XObject /Subtype /Form /BBox [0 0 0 40] "
                                   f"/Resources <</XObject <</Hidden {self.alpha_xref} 0 R>>>>>>")
            document.update_stream(form, b"q 40 0 0 40 0 0 cm /Hidden Do Q")
            add_xobject(document, document[0], "EmptyClip", form)
            append_content(document, document[0], b"/EmptyClip Do\n"
                           b"q 80 0 0 40 100 100 cm /fzImg0 Do Q")
            document.saveIncr()
        result = self.index()
        self.assertEqual(result["status"], "indexed")
        self.assertEqual([row["image_xref"] for row in result["occurrences"]], [self.jpeg_xref])
        self.assertEqual(result["skipped_forms"][0]["code"], "EMPTY_FORM_BBOX")
        self.assertEqual(Path(self.export()["asset"]["path"]).read_bytes(), self.jpeg)

    def test_malformed_form_is_local_uncertainty_not_loss_of_later_image_calls(self):
        with fitz.open(self.pdf) as document:
            form = document.get_new_xref()
            document.update_object(form, "<</Type /XObject /Subtype /Form /BBox [0 /Bad 80 40] /Resources <<>>>>")
            document.update_stream(form, b"q Q")
            add_xobject(document, document[0], "Malformed", form)
            append_content(document, document[0], b"/Malformed Do\n"
                           b"q 80 0 0 40 100 100 cm /fzImg0 Do Q")
            document.saveIncr()
        result = self.index()
        self.assertEqual(result["status"], "partial")
        self.assertEqual(result["issues"][0]["code"], "FORM_UNRESOLVED")
        self.assertEqual([row["image_xref"] for row in result["occurrences"]], [self.jpeg_xref])
        exported = self.export()
        self.assertEqual(exported["source_index_status"], "partial")
        self.assertEqual(exported["source_index_issues"], result["issues"])
        self.assertEqual(exported["visual_review"]["figure_completeness"], "not_assessed")
        self.assertEqual(Path(exported["asset"]["path"]).read_bytes(), self.jpeg)

    def test_external_vector_labels_do_not_grant_figure_acceptance(self):
        source = self.inventory()
        self.assertTrue(any("External panel label" in block["text"] for block in source["pages"][0]["blocks"]))
        self.assertEqual(source["pages"][0]["image_index"]["figure_completeness"], "not_assessed")
        result = self.export()
        self.assertEqual(result["visual_review"], {"status": "pending", "figure_completeness": "not_assessed"})
        self.assertIn("other PDF objects", result["evidence_boundary"])
        self.assertEqual(result["source_locations"][0]["bbox"], [30, 40, 190, 120])

    def test_crop_margin_and_four_pending_edges_are_explicit(self):
        result = self.render()
        self.assertEqual(result["rendered_bbox"], [24, 34, 196, 126])
        self.assertEqual(result["actual_margin_pt"], {edge: 6 for edge in ("top", "right", "bottom", "left")})
        self.assertEqual([row["edge"] for row in result["edge_reviews"]], ["top", "right", "bottom", "left"])
        self.assertTrue(all(row["status"] == "pending" for row in result["edge_reviews"]))
        self.assertIn("Pixel equality within the chosen crop cannot prove", result["evidence_boundary"])

    def test_margin_is_bounded_by_the_actual_page(self):
        result = self.render(bbox=[0, 0, 400, 300])
        self.assertEqual(result["rendered_bbox"], [0, 0, 400, 300])
        self.assertEqual(result["actual_margin_pt"], {edge: 0 for edge in ("top", "right", "bottom", "left")})
        self.assertEqual(result["visual_review"]["status"], "pending")

    def test_crop_rejects_pixels_out_of_page_and_malformed_bounds(self):
        with self.assertRaisesRegex(ValueError, "PDF-point"):
            self.render(coordinate_system={**images.PDF_BBOX_COORDINATES, "unit": "px"})
        for bbox in ([0, 0, 800, 600], [2, 2, 1, 1], [0, 0, 0, 1], [0, 0, float("inf"), 1],
                     [0, 0, 10 ** 1000, 1], [False, 0, 1, 1], [1, 2]):
            with self.subTest(bbox=bbox), self.assertRaises(ValueError):
                self.render(bbox=bbox)

    def test_unbalanced_content_is_explicitly_unresolved(self):
        with fitz.open(self.pdf) as document:
            append_content(document, document[0], b"Q\n")
            document.saveIncr()
        result = self.index()
        self.assertEqual(result["status"], "unresolved")
        self.assertEqual(result["issues"][0]["code"], "CONTENT_UNRESOLVED")
        with self.assertRaises(images.PageRenderRequired):
            self.export()

    def test_inline_image_has_no_invented_xref(self):
        with fitz.open(self.pdf) as document:
            append_content(document, document[0], b"q 20 0 0 20 30 230 cm BI /W 1 /H 1 /CS /RGB /BPC 8 ID \x01\x02\x03 EI Q\n")
            document.saveIncr()
        result = self.index()
        self.assertEqual(result["status"], "unresolved")
        self.assertEqual(result["occurrences"], [])
        self.assertEqual(result["issues"][0]["code"], "INLINE_IMAGE")

    def test_recursive_form_does_not_hang_or_silently_pass(self):
        with fitz.open(self.pdf) as document:
            form = document.get_new_xref()
            document.update_object(form, f"<</Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources <</XObject <</Loop {form} 0 R>>>>>>")
            document.update_stream(form, b"/Loop Do")
            add_xobject(document, document[0], "Loop", form)
            append_content(document, document[0], b"/Loop Do")
            document.saveIncr()
        result = self.index()
        self.assertEqual(result["status"], "unresolved")
        self.assertIn("cycle", result["issues"][0]["reason"])

    def test_vector_only_page_has_empty_image_inventory(self):
        with fitz.open(self.pdf) as document:
            page = document.new_page(width=400, height=300)
            page.draw_rect(fitz.Rect(20, 20, 120, 120))
            document.saveIncr()
        result = self.index(5)
        self.assertEqual(result["status"], "indexed")
        self.assertEqual(result["occurrences"], [])
        self.assertEqual(result["figure_completeness"], "not_assessed")

    def test_preparation_reports_progress_for_extraction(self):
        calls = []
        prepare.prepare_sources({"main": self.pdf}, self.base.joinpath("work"), dpi=72,
                                progress=lambda done, total, label: calls.append((done, total, label)))
        self.assertTrue(any("image" in label for _, _, label in calls))
        self.assertTrue(all(0 <= done <= total for done, total, _ in calls))


if __name__ == "__main__":
    unittest.main(verbosity=2)
