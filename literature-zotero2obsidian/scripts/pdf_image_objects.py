"""Index actual PDF image calls and export complete image objects.

Set the constants below to use this as a script, or import the functions.
An image object is not necessarily a complete figure. Its page may add text,
vector marks, other images, clipping, or transparency outside the object.
All exports therefore retain an explicit pending visual-review boundary.
"""

from __future__ import annotations

from contextlib import AbstractContextManager
import hashlib
from io import BytesIO
import json
import math
from pathlib import Path
import re
from typing import Callable, Mapping

import fitz
from PIL import Image
from pypdf import PdfReader
from pypdf.errors import PdfReadError
from pypdf.generic import ContentStream, DictionaryObject, IndirectObject


SOURCE_PDF: Path | None = None
PAGE_NUMBER: int | None = None
IMAGE_XREF: int | None = None
DESTINATION: Path | None = None
EXPECTED_SOURCE_SHA256: str | None = None
EXPORT_RECORD_PATH: Path | None = None
MAX_FORM_DEPTH = 64
MAX_PAGE_OPERATIONS = 1_000_000
PROGRESS_INTERVAL = 10_000
PDF_BBOX_COORDINATES = {
    "space": "pdf_page", "unit": "pt", "origin": "top_left",
    "x_axis": "right", "y_axis": "down", "rotation": "unrotated",
}
Progress = Callable[[int, int, str], None]
REVIEW_BOUNDARY = (
    "A complete image object can omit figure content drawn by other PDF objects. "
    "Compare the source page and final asset; object extraction is not figure acceptance."
)


class PageRenderRequired(ValueError):
    """The object's display effects require the original PDF page renderer."""


def progress_bar(done: int, total: int, label: str) -> None:
    width = 24
    filled = width * min(done, total) // max(total, 1)
    print(f"[{'#' * filled}{'.' * (width - filled)}] {done}/{total} {label}", flush=True)


def _hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _file_record(path: Path) -> dict:
    return {"path": str(path.resolve()), "sha256": _hash_file(path)}


def _write_once(path: Path, data: bytes) -> dict:
    """Reuse identical content, but never overwrite a different existing file."""
    digest = hashlib.sha256(data).hexdigest()
    if path.is_symlink():
        raise FileExistsError(f"Do not replace or follow an asset symlink: {path}")
    if path.exists():
        if not path.is_file() or _hash_file(path) != digest:
            raise FileExistsError(f"Keep the existing file: {path}")
        return {"path": str(path.resolve()), "sha256": digest}
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(data)
    return {"path": str(path.resolve()), "sha256": digest}


def _page(document, page_number: int):
    if not document.is_pdf or document.needs_pass:
        raise ValueError("An accessible PDF is required")
    if type(page_number) is not int or not 1 <= page_number <= len(document):
        raise ValueError("page_number must name an actual PDF page, starting at 1")
    return document[page_number - 1]


def _image_xref(document, image_xref: int) -> None:
    if type(image_xref) is not int or not 1 <= image_xref < document.xref_length():
        raise ValueError("image_xref must be an actual positive PDF xref")
    if document.xref_get_key(image_xref, "Subtype") != ("name", "/Image"):
        raise ValueError("image_xref does not identify an image object")


def _source_record(path: Path, expected: str | None) -> dict:
    actual = _file_record(path)
    if expected is not None and actual["sha256"] != expected:
        raise ValueError("The source PDF hash differs from the selected inventory")
    return actual


def _geometry(page) -> dict:
    """Take the PDF-to-page matrix without PyMuPDF's rotated-crop shortcut."""
    rotation = page.rotation
    page_to_display = list(page.rotation_matrix)
    # These edits affect only the open in-memory document, never the source file.
    try:
        if rotation:
            page.set_rotation(0)
        return {
            "coordinate_system": dict(PDF_BBOX_COORDINATES),
            "page_rotation": rotation,
            "page_bounds": list(page.rect),
            "media_box": list(page.mediabox),
            "crop_box": list(page.cropbox),
            "pdf_to_page_transform": list(page.transformation_matrix),
            "page_to_rotated_page_transform": page_to_display,
        }
    finally:
        if rotation:
            page.set_rotation(rotation)


def _dictionary(value) -> DictionaryObject:
    if isinstance(value, IndirectObject):
        value = value.get_object()
    if not isinstance(value, DictionaryObject):
        raise ValueError("Expected a PDF resource dictionary")
    return value


def _matrix(values) -> fitz.Matrix:
    if len(values) != 6:
        raise ValueError("A PDF transformation requires six numbers")
    numbers = [float(value) for value in values]
    if not all(math.isfinite(value) for value in numbers):
        raise ValueError("A PDF transformation must be finite")
    return fitz.Matrix(numbers)


def _pdf_rectangle(values) -> tuple[list[float], fitz.Rect]:
    """Normalize PDF opposite corners, not PyMuPDF's ordered-Rect convention.

    PDF Reference 1.5, 3.8.4 permits either pair of opposite rectangle corners;
    4.9 specifies that a Form's BBox clips its content in Form space.
    """
    if not isinstance(values, (list, tuple)) or len(values) != 4:
        raise ValueError("Form BBox must contain four PDF numbers")
    if any(not isinstance(value, (int, float)) or isinstance(value, bool) for value in values):
        raise ValueError("Form BBox must contain four PDF numbers")
    numbers = [float(value) for value in values]
    if not all(math.isfinite(value) for value in numbers):
        raise ValueError("Form BBox coordinates must be finite")
    rectangle = fitz.Rect(numbers).normalize()
    if not all(math.isfinite(value) for value in rectangle):
        raise ValueError("Form BBox exceeds the renderer's coordinate range")
    return numbers, rectangle


class ImageObjectIndexer(AbstractContextManager):
    """Walk actual Do operators, including nested Form XObjects.

    Each appearance refers to its indirect object, not a pixel-digest match.
    ``object_directory`` enables a single extracted file per distinct xref.
    Object bboxes are full, unclipped bounds in unrotated PDF page points.
    The walker does not claim to resolve visibility or figure completeness.
    """

    def __init__(self, source_pdf: Path, document, object_directory: Path | None = None,
                 *, progress: Progress | None = progress_bar):
        self.source_pdf = Path(source_pdf)
        self.document = document
        self.object_directory = Path(object_directory) if object_directory is not None else None
        self.progress = progress
        self.objects: dict[int, dict] = {}
        self._files: dict[int, dict] = {}
        self._operations: dict[int, list] = {}
        self._stream = None
        self.reader = None
        self.reader_error = None

    def __enter__(self):
        self._stream = self.source_pdf.open("rb")
        try:
            self.reader = PdfReader(self._stream, strict=True)
            if self.reader.is_encrypted:
                raise ValueError("Encrypted content streams require a decrypted working PDF")
            if len(self.reader.pages) != len(self.document):
                raise ValueError("The two PDF readers disagree on page count")
        except (PdfReadError, ValueError, TypeError, KeyError, IndexError, RecursionError) as error:
            self.reader_error = f"{type(error).__name__}: {error}"
        return self

    def __exit__(self, *args):
        if self._stream is not None:
            self._stream.close()
        return False

    def _asset(self, image_xref: int, extracted: dict) -> dict:
        if image_xref in self._files:
            return self._files[image_xref]
        path = self.object_directory.joinpath(f"xref-{image_xref:06d}.{extracted['ext']}")
        record = _write_once(path, extracted["image"])
        self._files[image_xref] = record
        return record

    def _record_object(self, image_xref: int) -> None:
        if image_xref in self.objects:
            return
        try:
            extracted, record, mask = _inspect_object(self.document, image_xref)
        except (ValueError, RuntimeError) as error:
            self.objects[image_xref] = {
                "image_xref": image_xref, "status": "unresolved", "reason": str(error),
                "figure_completeness": "not_assessed",
            }
            return
        record["raw_asset"] = self._asset(image_xref, extracted)
        if mask is not None:
            record["smask"]["raw_asset"] = self._asset(record["smask"]["image_xref"], mask)
        self.objects[image_xref] = record

    def index_page(self, page_number: int) -> dict:
        page = _page(self.document, page_number)
        result = {
            "status": "indexed", **_geometry(page), "occurrences": [], "issues": [],
            "skipped_forms": [],
            "bbox_meaning": "full_object_bounds_before_clipping",
            "figure_completeness": "not_assessed", "evidence_boundary": REVIEW_BOUNDARY,
        }
        if self.reader_error is not None:
            result.update(status="unresolved", issues=[{"code": "CONTENT_READER", "reason": self.reader_error}])
            return result
        if self.reader is None:
            raise RuntimeError("Use ImageObjectIndexer as a context manager")
        operation_count = [0]
        pdf_page = self.reader.pages[page_number - 1]
        try:
            resources = _dictionary(pdf_page.get("/Resources", DictionaryObject()))
            self._walk(pdf_page.get_contents(), resources, fitz.Matrix(1, 1), [], [], [], [],
                       fitz.Matrix(result["pdf_to_page_transform"]), result, operation_count,
                       page_number, pdf_page.indirect_reference.idnum)
        except (PdfReadError, ValueError, TypeError, KeyError, IndexError, RecursionError, NotImplementedError) as error:
            result["issues"].append({"code": "CONTENT_UNRESOLVED", "reason": f"{type(error).__name__}: {error}"})
        if result["issues"]:
            result["status"] = "partial" if result["occurrences"] else "unresolved"
        if self.object_directory is not None:
            xrefs = dict.fromkeys(row["image_xref"] for row in result["occurrences"] if row["image_xref"] is not None)
            for done, xref in enumerate(xrefs, 1):
                if self.progress:
                    self.progress(done - 1, len(xrefs), f"page {page_number}: extract image xref {xref}")
                self._record_object(xref)
            if self.progress and xrefs:
                self.progress(len(xrefs), len(xrefs), f"page {page_number}: image objects indexed")
        return result

    def _walk(self, stream, resources, ctm, forms, names, effects, form_clips, pdf_to_page,
              result, operation_count, page_number, container_xref):
        if stream is None:
            return
        if len(forms) > MAX_FORM_DEPTH:
            raise ValueError("Form nesting exceeds the explicit parser limit")
        if container_xref not in self._operations:
            self._operations[container_xref] = ContentStream(stream, self.reader).operations
        operations = self._operations[container_xref]
        stack = []
        for operation_index, (operands, operator) in enumerate(operations):
            operation_count[0] += 1
            if operation_count[0] > MAX_PAGE_OPERATIONS:
                raise ValueError("Page content exceeds the explicit operation limit")
            if self.progress and operation_count[0] % PROGRESS_INTERVAL == 0:
                self.progress(operation_index + 1, len(operations), f"page {page_number}: parse image calls")
            if operator == b"q":
                stack.append((fitz.Matrix(ctm), list(effects)))
                continue
            if operator == b"Q":
                if not stack:
                    raise ValueError("Unbalanced PDF graphics-state restore")
                ctm, effects = stack.pop()
                continue
            if operator == b"cm":
                ctm = _matrix(operands) * ctm
                continue
            if operator in (b"W", b"W*"):
                effects = list(dict.fromkeys([*effects, "page_clipping_path"]))
                continue
            if operator == b"gs":
                if len(operands) != 1:
                    raise ValueError("Malformed extended graphics state")
                states = _dictionary(resources.get("/ExtGState", DictionaryObject()))
                state = _dictionary(states[operands[0]])
                display = [key for key in ("/SMask", "/ca", "/CA", "/BM", "/TR", "/TR2") if key in state]
                effects = list(dict.fromkeys([*effects, *(f"graphics_state:{key}" for key in display)]))
                continue
            if operator == b"Tf":
                if len(operands) != 2:
                    raise ValueError("Malformed font selection")
                fonts = _dictionary(resources.get("/Font", DictionaryObject()))
                font = _dictionary(fonts[operands[0]])
                if font.get("/Subtype") == "/Type3":
                    result["issues"].append({"code": "TYPE3_CONTENT", "container_xref": container_xref,
                                             "operation_index": operation_index,
                                             "reason": "Type 3 glyph programs may contain images; their calls are not resolved by this index."})
                continue
            if operator in (b"scn", b"SCN") and operands:
                patterns = _dictionary(resources.get("/Pattern", DictionaryObject()))
                if isinstance(operands[-1], str) and operands[-1] in patterns:
                    result["issues"].append({"code": "PATTERN_CONTENT", "container_xref": container_xref,
                                             "operation_index": operation_index,
                                             "reason": "Pattern painting may repeat images; pattern content is not resolved by this index."})
                continue
            if operator == b"INLINE IMAGE":
                result["issues"].append({"code": "INLINE_IMAGE", "container_xref": container_xref,
                                         "operation_index": operation_index,
                                         "reason": "An inline image has no selectable indirect image_xref; use page-region rendering."})
                continue
            if operator != b"Do":
                continue
            if len(operands) != 1:
                raise ValueError("Malformed PDF XObject call")
            name = operands[0]
            xobjects = _dictionary(resources.get("/XObject", DictionaryObject()))
            reference = xobjects.raw_get(name)
            if not isinstance(reference, IndirectObject):
                raise ValueError(f"XObject {name} is not an indirect object")
            xref = reference.idnum
            obj = _dictionary(reference)
            subtype = obj.get("/Subtype")
            if subtype == "/Form":
                form_location = {"form_xref": xref, "container_xref": container_xref,
                                 "operation_index": operation_index, "resource_path": [*names, str(name)]}
                try:
                    raw_box, form_box = _pdf_rectangle(obj.get("/BBox"))
                    if form_box.is_empty:
                        result["skipped_forms"].append({
                            **form_location, "code": "EMPTY_FORM_BBOX", "pdf_bbox": raw_box,
                            "bbox": list(form_box), "reason": "A zero-area Form BBox clips away its entire contents.",
                        })
                        continue
                    if xref in forms:
                        raise ValueError("Recursive Form XObject cycle")
                    child_matrix = _matrix(obj.get("/Matrix", [1, 0, 0, 1, 0, 0])) * ctm
                    child_resources = _dictionary(obj.get("/Resources", resources))
                    inverse = fitz.Matrix(child_matrix)
                    if inverse.invert():
                        raise ValueError("Singular Form transformation")
                    clip = {"form_xref": xref, "bbox": list(form_box), "pdf_bbox": raw_box,
                            "pdf_to_form_transform": list(inverse)}
                    child_effects = list(effects)
                    if "/Group" in obj:
                        child_effects.append("form_transparency_group")
                    self._walk(obj, child_resources, child_matrix, [*forms, xref], [*names, str(name)],
                               child_effects, [*form_clips, clip], pdf_to_page, result, operation_count, page_number, xref)
                except (PdfReadError, ValueError, TypeError, KeyError, IndexError, RecursionError, NotImplementedError) as error:
                    # A Form invocation saves/restores graphics state. A bad
                    # Form is local uncertainty, not a reason to discard its
                    # independently parseable following sibling objects.
                    result["issues"].append({**form_location, "code": "FORM_UNRESOLVED",
                                             "reason": f"{type(error).__name__}: {error}"})
                continue
            if subtype != "/Image":
                raise ValueError(f"Unsupported XObject subtype: {subtype}")
            _image_xref(self.document, xref)
            # PDF image samples start at top-left, while the image unit square
            # in content streams starts at bottom-left.
            transform = fitz.Matrix(1, 0, 0, -1, 0, 1) * ctm * pdf_to_page
            bbox = fitz.Rect(0, 0, 1, 1) * transform
            if bbox.is_empty or bbox.is_infinite or not all(math.isfinite(v) for v in bbox):
                raise ValueError(f"Degenerate image transformation for xref {xref}")
            appearance_effects = list(effects)
            color_resources = _dictionary(resources.get("/ColorSpace", DictionaryObject()))
            if any(name in color_resources for name in ("/DefaultRGB", "/DefaultGray", "/DefaultCMYK")):
                appearance_effects.append("default_colorspace_override")
            raw_quad = fitz.Rect(0, 0, 1, 1).quad * fitz.Matrix(1, 0, 0, -1, 0, 1) * ctm
            for clip in form_clips:
                form_box = fitz.Rect(clip["bbox"]) + (-0.0001, -0.0001, 0.0001, 0.0001)
                inverse = fitz.Matrix(clip["pdf_to_form_transform"])
                if not all(form_box.contains(point * inverse) for point in raw_quad):
                    appearance_effects.append("form_bbox_clipping")
                    break
            if "/OC" in obj:
                appearance_effects.append("optional_content_visibility")
            fully_inside_page = fitz.Rect(result["page_bounds"]).contains(bbox)
            if not fully_inside_page:
                appearance_effects.append("page_boundary_clipping")
            result["occurrences"].append({
                "occurrence": len(result["occurrences"]) + 1,
                "image_xref": xref, "generation": reference.generation,
                "resource_name": str(name), "resource_path": [*names, str(name)],
                "container_xref": container_xref, "operation_index": operation_index,
                "form_xrefs": list(forms), "bbox": list(bbox), "transform": list(transform),
                "fully_inside_page": fully_inside_page, "form_clips": list(form_clips),
                "context_effects": list(dict.fromkeys(appearance_effects)), "visibility": "not_assessed",
            })
        if stack:
            raise ValueError("Unbalanced PDF graphics-state save")


def _key(document, xref: int, name: str) -> str | None:
    kind, value = document.xref_get_key(xref, name)
    return None if kind == "null" else value


def _mask_xref(document, xref: int) -> int:
    value = _key(document, xref, "SMask")
    if value in (None, "/None"):
        return 0
    match = re.fullmatch(r"(\d+)\s+\d+\s+R", value)
    if match is None:
        raise ValueError("The SMask is not an indirect image object")
    return int(match.group(1))


def _colorspace(document, xref: int) -> str | None:
    value = _key(document, xref, "ColorSpace")
    seen = set()
    while value is not None:
        match = re.fullmatch(r"(\d+)\s+\d+\s+R", value)
        if match is None:
            return value
        reference = int(match.group(1))
        if reference in seen:
            raise ValueError("Recursive ColorSpace reference")
        seen.add(reference)
        value = document.xref_object(reference, compressed=True).strip()
    return None


def _inspect_object(document, xref: int):
    _image_xref(document, xref)
    extracted = document.extract_image(xref)
    if not extracted or not extracted.get("image") or not re.fullmatch(r"[a-z0-9]+", extracted.get("ext", "")):
        raise ValueError("The PDF decoder did not return an encoded image")
    filters = _key(document, xref, "Filter")
    original_ext = next((extension for filter_name, extension in
                         (("DCTDecode", "jpeg"), ("JPXDecode", "jpx"), ("JBIG2Decode", "jb2"))
                         if filters and filter_name in filters), None)
    data = extracted["image"]
    record = {
        "image_xref": xref, "status": "extracted", "width": extracted["width"],
        "height": extracted["height"], "bits_per_component": extracted["bpc"],
        "colorspace": extracted["cs-name"], "pdf_filter": filters,
        "pdf_colorspace": _colorspace(document, xref),
        "original_encoded_extension": original_ext,
        "extracted_extension": extracted["ext"],
        "extracted_sha256": hashlib.sha256(data).hexdigest(),
        "pdf_stream_sha256": hashlib.sha256(document.xref_stream_raw(xref)).hexdigest(),
        "encoding_note": "Original PDF image encoding" if original_ext == extracted["ext"] else
                         "Standalone lossless extraction; a PDF sample stream need not have a standalone file extension",
        "decode": _key(document, xref, "Decode"),
        "image_mask": _key(document, xref, "ImageMask") == "true",
        "explicit_mask": _key(document, xref, "Mask"),
        "smask_in_data": _key(document, xref, "SMaskInData"),
        "figure_completeness": "not_assessed",
    }
    smask_xref = _mask_xref(document, xref)
    if not smask_xref:
        record["smask"] = {"image_xref": 0, "status": "absent"}
        return extracted, record, None
    _image_xref(document, smask_xref)
    mask = document.extract_image(smask_xref)
    mask_pixels = fitz.Pixmap(document, smask_xref)
    if mask_pixels.n != 1 or mask_pixels.alpha:
        raise ValueError("SMask samples are not an unambiguous one-channel opacity image")
    # Pillow performs this scan in native code without a Python pixel loop.
    with Image.frombuffer("L", (mask_pixels.width, mask_pixels.height),
                          mask_pixels.samples, "raw", "L", 0, 1) as mask_image:
        extrema = mask_image.getextrema()
    record["smask"] = {
        "image_xref": smask_xref, "status": "opaque" if extrema == (255, 255) else "nonopaque",
        "width": mask_pixels.width, "height": mask_pixels.height,
        "extracted_extension": mask["ext"], "extracted_sha256": hashlib.sha256(mask["image"]).hexdigest(),
        "alpha_min": extrema[0], "alpha_max": extrema[1],
        "decode": _key(document, smask_xref, "Decode"), "matte": _key(document, smask_xref, "Matte"),
        "pdf_colorspace": _colorspace(document, smask_xref),
    }
    return extracted, record, mask


def _destination(path: Path, extension: str) -> Path:
    path = Path(path)
    aliases = {"jpeg": {".jpeg", ".jpg"}, "jpx": {".jpx", ".jp2"}, "tiff": {".tiff", ".tif"}}
    suffixes = aliases.get(extension, {"." + extension})
    if not path.suffix:
        return path.with_suffix("." + extension)
    if path.suffix.lower() not in suffixes:
        raise ValueError(f"Destination extension must match the {extension} encoding")
    return path


def export_image_object(source_pdf: Path, *, page_number: int, image_xref: int,
                        destination: Path, expected_source_sha256: str | None = None,
                        progress: Progress | None = progress_bar) -> dict:
    """Copy a selected, actually invoked xref, or compose its nonopaque SMask.

    A suffix-free destination acquires the actual encoding extension. Wrong
    suffixes and conflicting existing files are rejected. PNG compositing is
    lossless; unsupported display effects require ``render_page_region``.
    The returned asset reference contains only path and SHA-256.
    """
    path = Path(source_pdf).resolve(strict=True)
    source = _source_record(path, expected_source_sha256)
    with fitz.open(path) as document:
        _page(document, page_number)
        _image_xref(document, image_xref)
        with ImageObjectIndexer(path, document, progress=progress) as indexer:
            page_index = indexer.index_page(page_number)
        local_form_issues = page_index["issues"] and all(
            issue["code"] == "FORM_UNRESOLVED" for issue in page_index["issues"])
        if page_index["status"] != "indexed" and not local_form_issues:
            raise PageRenderRequired("Page image calls are not fully parsed; inspect the image index and render the page region")
        locations = [row for row in page_index["occurrences"] if row["image_xref"] == image_xref]
        if not locations:
            raise ValueError("image_xref is not actually invoked on the selected PDF page")
        effects = sorted({effect for location in locations for effect in location["context_effects"]})
        if effects:
            raise PageRenderRequired("Page display context requires page-region rendering: " + ", ".join(effects))
        extracted, record, _ = _inspect_object(document, image_xref)
        page_rotation = fitz.Matrix(page_index["page_to_rotated_page_transform"])
        for location in locations:
            displayed = fitz.Matrix(location["transform"]) * page_rotation
            if displayed.a <= 0 or displayed.d <= 0 or abs(displayed.b) > 0.0001 or abs(displayed.c) > 0.0001:
                raise PageRenderRequired("A rotated, mirrored, or skewed image appearance requires page-region rendering")
            if not math.isclose(displayed.a / record["width"], displayed.d / record["height"], rel_tol=0.0001):
                raise PageRenderRequired("A nonuniformly scaled image appearance requires page-region rendering")
        if record["pdf_colorspace"] not in ("/DeviceRGB", "/DeviceGray"):
            raise PageRenderRequired("ICC, CMYK, indexed, or resource-defined ColorSpace requires page-region rendering")
        if record["smask_in_data"] not in (None, "0"):
            raise PageRenderRequired("SMaskInData requires page-region rendering")
        if record["image_mask"] or record["explicit_mask"] is not None or record["decode"] is not None:
            raise PageRenderRequired("ImageMask, explicit Mask, or Decode requires page-region rendering")
        data, extension, method = extracted["image"], extracted["ext"], "original_extracted_bytes"
        if extension == "jpeg":
            with Image.open(BytesIO(data)) as standalone:
                if standalone.info.get("icc_profile"):
                    raise PageRenderRequired("A JPEG-embedded ICC profile requires page-region rendering")
                if standalone.getexif().get(274, 1) != 1:
                    raise PageRenderRequired("JPEG EXIF orientation requires page-region rendering")
            decode_parameters = _key(document, image_xref, "DecodeParms")
            if decode_parameters is not None and "ColorTransform" in decode_parameters:
                raise PageRenderRequired("A PDF JPEG ColorTransform override requires page-region rendering")
        smask = record["smask"]
        if smask["status"] == "nonopaque":
            if smask["pdf_colorspace"] != "/DeviceGray":
                raise PageRenderRequired("A non-device grayscale SMask requires page-region rendering")
            if smask["matte"] is not None or (smask["width"], smask["height"]) != (record["width"], record["height"]):
                raise PageRenderRequired("SMask Matte or differing mask dimensions require page-region rendering")
            base_pixels = fitz.Pixmap(document, image_xref)
            mask_pixels = fitz.Pixmap(document, smask["image_xref"])
            if base_pixels.alpha:
                raise PageRenderRequired("An additional embedded alpha channel requires page-region rendering")
            if base_pixels.colorspace.n not in (1, 3):
                base_pixels = fitz.Pixmap(fitz.csRGB, base_pixels)
            # Keep straight color samples. A premultiply/unpremultiply roundtrip
            # would unnecessarily change low-alpha colors through rounding.
            mode = "RGB" if base_pixels.colorspace.n == 3 else "L"
            composed = Image.frombuffer(mode, (base_pixels.width, base_pixels.height),
                                        base_pixels.samples, "raw", mode, 0, 1)
            opacity = Image.frombuffer("L", (mask_pixels.width, mask_pixels.height),
                                       mask_pixels.samples, "raw", "L", 0, 1)
            composed.putalpha(opacity)
            encoded = BytesIO()
            composed.save(encoded, format="PNG")
            data = encoded.getvalue()
            composed.close()
            opacity.close()
            extension, method = "png", "smask_composited_png"
        output = _destination(destination, extension)
        if output.resolve() == path:
            raise ValueError("The source PDF cannot be used as the asset destination")
        asset = _write_once(output, data)
    if progress:
        progress(1, 1, f"export image xref {image_xref}")
    return {
        "schema_version": 1, "source_file": source, "page": page_number,
        "image_xref": image_xref, "source_locations": locations,
        "source_index_status": page_index["status"], "source_index_issues": page_index["issues"],
        "page_rotation": page_index["page_rotation"],
        "page_to_rotated_page_transform": page_index["page_to_rotated_page_transform"],
        "coordinate_system": dict(PDF_BBOX_COORDINATES), "source_object": record,
        "asset": asset, "output_extension": extension, "method": method,
        "orientation": "native_image_samples; apply the occurrence transform when comparing with the page",
        "visual_review": {"status": "pending", "figure_completeness": "not_assessed"},
        "evidence_boundary": REVIEW_BOUNDARY,
    }


def render_page_region(source_pdf: Path, *, page_number: int, bbox: list[float],
                       coordinate_system: Mapping[str, str], destination: Path,
                       margin_pt: float = 6.0, dpi: int = 300,
                       expected_source_sha256: str | None = None,
                       progress: Progress | None = progress_bar) -> dict:
    """Render a reviewed region with safety margin and pending four-edge checks.

    This applies PDF masks, clipping and page graphics state. Input coordinates
    are unrotated; the output retains the source page's display orientation.
    Matching pixels inside this crop says nothing
    about content outside it; all four source boundaries still require review.
    """
    if coordinate_system != PDF_BBOX_COORDINATES:
        raise ValueError("Declare unrotated PDF-point coordinates explicitly")
    if type(dpi) is not int or not 72 <= dpi <= 1200:
        raise ValueError("dpi must be an integer between 72 and 1200")
    if type(margin_pt) not in (int, float) or not math.isfinite(margin_pt) or margin_pt < 0:
        raise ValueError("margin_pt must be a finite nonnegative number")
    if not isinstance(bbox, list) or len(bbox) != 4 or any(type(v) not in (int, float) for v in bbox):
        raise ValueError("bbox must contain four finite PDF-point numbers")
    try:
        finite = all(math.isfinite(value) for value in bbox)
    except OverflowError:
        finite = False
    if not finite:
        raise ValueError("bbox must contain four finite PDF-point numbers")
    path = Path(source_pdf).resolve(strict=True)
    source = _source_record(path, expected_source_sha256)
    output = _destination(destination, "png")
    with fitz.open(path) as document:
        page = _page(document, page_number)
        geometry = _geometry(page)
        bounds, rectangle = fitz.Rect(geometry["page_bounds"]), fitz.Rect(bbox)
        if not rectangle.is_valid or rectangle.is_empty or not bounds.contains(rectangle):
            raise ValueError("bbox must be nonempty and fully inside the unrotated page")
        clip = (rectangle + (-margin_pt, -margin_pt, margin_pt, margin_pt)) & bounds
        rotation = page.rotation
        try:
            if rotation:
                page.set_rotation(0)
            matrix = fitz.Matrix(dpi / 72, dpi / 72).prerotate(rotation)
            pixels = page.get_pixmap(matrix=matrix, clip=clip, colorspace=fitz.csRGB, alpha=False)
            pixels.set_dpi(dpi, dpi)
        finally:
            if rotation:
                page.set_rotation(rotation)
        asset = _write_once(output, pixels.tobytes("png"))
    if progress:
        progress(1, 1, f"render page {page_number} region")
    return {
        "schema_version": 1, "source_file": source, "page": page_number,
        "requested_bbox": bbox, "rendered_bbox": list(clip), "coordinate_system": dict(PDF_BBOX_COORDINATES),
        "requested_margin_pt": margin_pt,
        "actual_margin_pt": {"left": rectangle.x0 - clip.x0, "top": rectangle.y0 - clip.y0,
                             "right": clip.x1 - rectangle.x1, "bottom": clip.y1 - rectangle.y1},
        "dpi": dpi, "asset": asset, "method": "page_region_render", "page_geometry": geometry,
        "orientation": "source_page_display",
        "edge_reviews": [{"edge": edge, "status": "pending", "reason": "", "evidence": []}
                         for edge in ("top", "right", "bottom", "left")],
        "visual_review": {"status": "pending", "figure_completeness": "not_assessed"},
        "evidence_boundary": REVIEW_BOUNDARY + " Pixel equality within the chosen crop cannot prove that nothing was cropped off.",
    }


def main() -> int:
    if SOURCE_PDF is None or PAGE_NUMBER is None or IMAGE_XREF is None or DESTINATION is None:
        print("Set SOURCE_PDF, PAGE_NUMBER, IMAGE_XREF and DESTINATION at the top of this script.")
        return 2
    record = export_image_object(SOURCE_PDF, page_number=PAGE_NUMBER, image_xref=IMAGE_XREF,
                                 destination=DESTINATION, expected_source_sha256=EXPECTED_SOURCE_SHA256)
    data = (json.dumps(record, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    if EXPORT_RECORD_PATH is not None:
        _write_once(EXPORT_RECORD_PATH, data)
    print(data.decode("utf-8"), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
