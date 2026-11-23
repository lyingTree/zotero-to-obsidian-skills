"""Prepare PDF source evidence without claiming visual or semantic acceptance.

Set SOURCE_PDFS, SOURCE_ROLES and WORK_DIRECTORY below, then run this file.
Or call prepare_sources(source_pdfs, work_directory, source_roles=...).
The generated inventory deliberately contains pending decisions. Fill those
decisions from the rendered pages before constructing a delivery manifest.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import math
from contextlib import ExitStack
from pathlib import Path
import re
import sys
from time import monotonic
from typing import Callable, Mapping, Sequence

import fitz


SOURCE_PDFS: dict[str, Path] = {}
SOURCE_ROLES: dict[str, str] = {}
WORK_DIRECTORY: Path | None = None
GLOSSARY_PATH: Path | None = None
RENDER_DPI = 144
Progress = Callable[[int, int, str], None]
_LAST_PROGRESS_TIME = 0.0
PDF_BBOX_COORDINATES = {
    "space": "pdf_page", "unit": "pt", "origin": "top_left",
    "x_axis": "right", "y_axis": "down", "rotation": "unrotated",
}
FIGURE_REVIEW_GOALS = {
    "FIG-A01": "子图面板齐全。",
    "FIG-A02": "面板号完整。",
    "FIG-A03": "图内文字完整。",
    "FIG-A04": "轴线完整。",
    "FIG-A05": "刻度完整。",
    "FIG-A06": "单位完整。",
    "FIG-A07": "图例完整。",
    "FIG-A08": "色标完整。",
    "FIG-A09": "图内要素可读。",
    "FIG-A10": "图外题注没有混入图件。",
    "FIG-A11": "相邻图件没有混入。",
    "FIG-A12": "正文没有混入图件。",
    "FIG-A13": "页眉没有混入图件。",
    "FIG-A14": "页脚没有混入图件。",
    "FIG-A15": "页码没有混入图件。",
    "FIG-A16": "图件没有大块页边空白。",
    "FIG-A17": "图号与图件对应。",
    "FIG-A18": "题注与图件对应。",
    "FIG-A19": "来源页码已登记。",
    "FIG-A20": "来源附件哈希已登记。",
    "FIG-A21": "图框或原图对象定位已登记。",
    "FIG-A22": "适用的坐标系已登记。",
    "FIG-A23": "最终图片哈希已登记。",
    "FIG-A24": "视觉证据位置已登记。",
    "FIG-A25": "最终嵌入图片与已验资源哈希一致。",
    "FIG-A26": "输出图像的透明度效果与源页一致。",
    "FIG-A27": "输出图像的颜色与源页一致。",
    "FIG-A28": "图内数据表在原图中完整保留。",
}


def sha256_file(path: Path) -> str:
    """Hash a file in bounded chunks; do not read large PDFs into one buffer."""
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def file_record(path: Path) -> dict:
    return {"path": str(path.resolve()), "sha256": sha256_file(path)}


def progress_bar(done: int, total: int, label: str) -> None:
    """Show ongoing work without printing one line for every small check."""
    global _LAST_PROGRESS_TIME
    now = monotonic()
    if done not in (1, total) and now - _LAST_PROGRESS_TIME < 0.25:
        return
    _LAST_PROGRESS_TIME = now
    width = 24
    filled = width * done // max(total, 1)
    print(f"\r[{'#' * filled}{'.' * (width - filled)}] {done}/{total} {label}",
          end="\n" if done == total else "", flush=True)


def _image_tools():
    """Load this snapshot's helper only when PDF image indexing is requested."""
    path = Path(__file__).resolve().parent.joinpath("pdf_image_objects.py")
    suffix = hashlib.sha256(str(path).encode("utf-8")).hexdigest()[:16]
    name = f"literature_pdf_image_objects_{suffix}"
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def prepare_sources(
    source_pdfs: Mapping[str, Path],
    work_directory: Path,
    *,
    source_roles: Mapping[str, str] | None = None,
    glossary_path: Path | None = None,
    dpi: int = RENDER_DPI,
    progress: Progress | None = progress_bar,
) -> Path:
    """Render pages, record text blocks, and index actual image-object calls.

    Source IDs are stable task-local names, not bibliography anchor prefixes.
    Existing inventories are never overwritten. Use a new work directory for
    another extraction, so a delivery cannot silently acquire different input.
    The text layer is diagnostic evidence; it is not the visible-page truth.
    Each page's ``image_index`` records actual xrefs, full object bounds, and
    transforms. Each source's ``image_objects`` stores unique raw extractions
    and SMask evidence. Neither identifies complete figures automatically.
    """
    if not source_pdfs:
        raise ValueError("Provide at least one source PDF")
    if not 72 <= dpi <= 600:
        raise ValueError("dpi must be between 72 and 600")
    invalid = [key for key in source_pdfs if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", key)]
    if invalid:
        raise ValueError(f"Invalid source ID: {invalid[0]}")
    inventory_path = work_directory.joinpath("source-inventory.json")
    if inventory_path.exists():
        raise FileExistsError(f"Keep the existing inventory: {inventory_path}")
    terminology_path = work_directory.joinpath("terminology.json")
    glossary = None
    if glossary_path is not None:
        if terminology_path.exists():
            raise FileExistsError(f"Keep the existing terminology record: {terminology_path}")
        glossary = file_record(Path(glossary_path).resolve(strict=True))
    resolved = {key: Path(value).resolve(strict=True) for key, value in source_pdfs.items()}
    page_counts: dict[str, int] = {}
    for key, path in resolved.items():
        with fitz.open(path) as document:
            if not document.is_pdf or document.needs_pass:
                raise ValueError(f"{key}: an accessible PDF is required")
            page_counts[key] = len(document)
    total = sum(page_counts.values())
    image_tools = _image_tools()
    done = 0
    sources = []
    for key, path in resolved.items():
        directory = work_directory.joinpath("source-evidence", key)
        directory.mkdir(parents=True, exist_ok=True)
        pages = []
        with fitz.open(path) as document, image_tools.ImageObjectIndexer(
            path, document, directory.joinpath("image-objects"), progress=progress,
        ) as image_indexer:
            for index, page in enumerate(document):
                page_number = index + 1
                text_path = directory.joinpath(f"page-{page_number:04d}.txt")
                image_path = directory.joinpath(f"page-{page_number:04d}.png")
                # Preserve PDF-emitted CR and LF exactly on every host platform.
                text_path.write_bytes(page.get_text("text", sort=False).encode("utf-8"))
                pixels = page.get_pixmap(dpi=dpi, colorspace=fitz.csRGB, alpha=False)
                pixels.save(image_path)
                blocks = [
                    {"id": f"{key}-p{page_number}-b{block[5]}",
                     "bbox": list(block[:4]), "text": block[4]}
                    for block in page.get_text("blocks", sort=False)
                    if block[6] == 0 and str(block[4]).strip()
                ]
                pages.append({
                    "page": page_number,
                    "render": {**file_record(image_path), "dpi": dpi,
                               "width": pixels.width, "height": pixels.height,
                               "pixel_sha256": hashlib.sha256(pixels.samples).hexdigest()},
                    "text": file_record(text_path),
                    "blocks": blocks,
                    "image_index": image_indexer.index_page(page_number),
                })
                done += 1
                if progress:
                    progress(done, total, f"{key} page {page_number}")
        sources.append({
            "id": key, "role": (source_roles or {}).get(key, "pending"),
            **file_record(path), "page_count": page_counts[key], "pages": pages,
            "image_objects": list(image_indexer.objects.values()),
            "header_fields": {
                field: {"status": "pending"}
                for field in ("title", "authors", "affiliations", "publication_date")
            },
        })
    inventory = {
        "schema_version": 1,
        "evidence_boundary": "Rendered files do not prove that a model viewed or understood them.",
        "sources": sources,
        "units": [],
        "excluded_blocks": [],
        "excluded_pages": [],
    }
    if glossary is not None:
        terminology = {
            "schema_version": 1,
            "glossary": glossary,
            "review_status": "pending",
            "covered_units": [],
            "adopted": [],
            "unlisted": [],
            "no_unlisted_reason": None,
            "evidence_boundary": "A ledger records decisions; it does not prove translation correctness.",
        }
        terminology_path.write_text(json.dumps(terminology, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        inventory["terminology_path"] = str(terminology_path.resolve())
    inventory_path.write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return inventory_path


def pdf_page_rect(page) -> fitz.Rect:
    """Return the unrotated, top-left-origin page rectangle in PDF points."""
    return page.rect * page.derotation_matrix


def valid_pdf_bbox(box, page_rect: fitz.Rect) -> bool:
    if not isinstance(box, list) or len(box) != 4:
        return False
    if any(type(value) not in (int, float) for value in box):
        return False
    try:
        if not all(math.isfinite(value) for value in box):
            return False
    except OverflowError:
        return False
    rectangle = fitz.Rect(box)
    return rectangle.is_valid and not rectangle.is_empty and page_rect.contains(rectangle)


def _figure_index(rows, key: str, label: str) -> dict[str, dict]:
    if not isinstance(rows, list):
        raise ValueError(f"{label} must be an array of objects")
    indexed = {}
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError(f"{label} must contain objects")
        identity = row.get(key)
        if not isinstance(identity, str) or not identity.strip():
            raise ValueError(f"{label} requires nonempty {key} values")
        if identity in indexed:
            raise ValueError(f"Duplicate {label} {key}: {identity}")
        indexed[identity] = row
    return indexed


def _figure_json_pairs(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"Duplicate JSON key: {key}")
        value[key] = item
    return value


def _figure_file_reference(record, base: Path, label: str) -> dict:
    if not isinstance(record, dict) or not isinstance(record.get("path"), str) or not record["path"].strip():
        raise ValueError(f"{label}: a file path is required")
    if not isinstance(record.get("sha256"), str) or not re.fullmatch(r"[a-f0-9]{64}", record["sha256"]):
        raise ValueError(f"{label}: a lowercase SHA-256 is required")
    path = Path(record["path"])
    if not path.is_absolute():
        path = base.joinpath(path)
    actual = file_record(path.resolve(strict=True))
    if actual["sha256"] != record["sha256"]:
        raise ValueError(f"{label}: the file changed after its inventory record")
    return actual


def prepare_figure_reviews(
    inventory_path: Path,
    final_assets: Mapping[str, Sequence[Path]],
    work_directory: Path,
    *,
    resource_root: Path | None = None,
    progress: Progress | None = progress_bar,
) -> Path:
    """Create pending atomic review records for the inventory's actual figures.

    ``final_assets`` maps each figure unit ID to its final image paths, in order.
    Relative image paths use ``resource_root`` (default: the inventory directory).
    Figure evidence must explicitly use PDF-point coordinates, or identify an
    original image XRef. No coordinate units or visual outcomes are inferred.
    This binds an immutable inventory snapshot and registers the new ledger path
    in the live inventory. Other tool pointers can then be registered in either
    order. Bind the delivery after filling the ledger. Existing evidence files
    are never overwritten.
    """
    inventory_path = Path(inventory_path).resolve(strict=True)
    work_directory = Path(work_directory)
    review_path = work_directory.joinpath("figure-reviews.json").resolve()
    snapshot_path = work_directory.joinpath("figure-inventory-snapshot.json").resolve()
    if review_path.exists():
        raise FileExistsError(f"Keep the existing figure reviews: {review_path}")
    if snapshot_path.exists():
        raise FileExistsError(f"Keep the existing figure inventory snapshot: {snapshot_path}")
    inventory = json.loads(inventory_path.read_text(encoding="utf-8-sig"), object_pairs_hook=_figure_json_pairs)
    if not isinstance(inventory, dict) or type(inventory.get("schema_version")) is not int or inventory["schema_version"] != 1:
        raise ValueError("Expected source inventory schema_version 1")
    if "figure_reviews_path" in inventory:
        raise ValueError("This inventory already names a figure-review ledger; preserve that review history")
    units = _figure_index(inventory.get("units"), "id", "units")
    sources = _figure_index(inventory.get("sources"), "id", "sources")
    figures = {key: unit for key, unit in units.items() if unit.get("kind") == "figure"}
    if not isinstance(final_assets, Mapping) or any(not isinstance(key, str) for key in final_assets):
        raise ValueError("final_assets must map figure unit IDs to image path lists")
    if set(final_assets) != set(figures):
        raise ValueError("final_assets must cover exactly the inventory's figure unit IDs")
    root = Path(resource_root or inventory_path.parent).resolve(strict=True)
    if not root.is_dir():
        raise NotADirectoryError(root)
    rows = []
    with ExitStack() as stack:
        documents, source_files = {}, {}
        for done, (identity, unit) in enumerate(figures.items(), 1):
            source_id = unit.get("source")
            if not isinstance(source_id, str) or source_id not in sources:
                raise ValueError(f"{identity}: choose a source ID from the inventory")
            source = sources[source_id]
            if source_id not in documents:
                source_files[source_id] = _figure_file_reference(source, inventory_path.parent, source_id)
                document = stack.enter_context(fitz.open(source_files[source_id]["path"]))
                if not document.is_pdf or document.needs_pass:
                    raise ValueError(f"{source_id}: an accessible PDF is required")
                documents[source_id] = document
            document = documents[source_id]
            page_rows = source.get("pages")
            if not isinstance(page_rows, list) or any(not isinstance(row, dict) for row in page_rows):
                raise ValueError(f"{source_id}: pages must be an array of objects")
            page_numbers = [row.get("page") for row in page_rows]
            if any(type(number) is not int for number in page_numbers) or page_numbers != list(range(1, len(document) + 1)):
                raise ValueError(f"{source_id}: record every actual PDF page once in source order")
            evidence = unit.get("evidence")
            if not isinstance(evidence, list) or not evidence or any(not isinstance(row, dict) for row in evidence):
                raise ValueError(f"{identity}: source locations are required")
            locations = []
            for location in evidence:
                number = location.get("page")
                if type(number) is not int or not 1 <= number <= len(document):
                    raise ValueError(f"{identity}: unknown source page")
                if location.get("basis") != "visual":
                    raise ValueError(f"{identity}: figure locations require visible-page evidence")
                page = document[number - 1]
                locator = {"page": number}
                box = location.get("bbox")
                xref = location.get("image_xref")
                if box is None and xref is None:
                    raise ValueError(f"{identity}: locate the figure with a bbox or image_xref")
                if box is not None:
                    if location.get("coordinate_system") != PDF_BBOX_COORDINATES:
                        raise ValueError(f"{identity}: declare unrotated PDF points with a top-left origin and right/down axes")
                    if not valid_pdf_bbox(box, pdf_page_rect(page)):
                        raise ValueError(f"{identity}: bbox must be a finite nonempty PDF-point rectangle inside its page")
                    locator.update({"bbox": box, "coordinate_system": dict(PDF_BBOX_COORDINATES)})
                if xref is not None:
                    if type(xref) is not int or xref not in {image[0] for image in page.get_images(full=True)}:
                        raise ValueError(f"{identity}: image_xref must name an image on this source page")
                    locator["image_xref"] = xref
                locator["page_render"] = _figure_file_reference(page_rows[number - 1].get("render"), inventory_path.parent,
                                                                f"{identity}:page-{number}")
                locations.append(locator)
            paths = final_assets[identity]
            if not isinstance(paths, Sequence) or isinstance(paths, (str, bytes)) or not paths:
                raise ValueError(f"{identity}: provide a nonempty sequence of final image paths")
            assets = []
            for raw in paths:
                if not isinstance(raw, (str, Path)) or not str(raw).strip():
                    raise ValueError(f"{identity}: each final image path must be a path string or Path")
                path = Path(raw)
                if not path.is_absolute():
                    path = root.joinpath(path)
                assets.append(file_record(path.resolve(strict=True)))
            if len({asset["path"] for asset in assets}) != len(assets):
                raise ValueError(f"{identity}: do not repeat final image paths")
            rows.append({
                "unit": identity, "source": source_id, "source_file": source_files[source_id],
                "source_locations": locations, "final_assets": assets,
                "checks": [{"id": key, "goal": goal, "status": "pending", "reason": "", "evidence": []}
                           for key, goal in FIGURE_REVIEW_GOALS.items()],
            })
            if progress:
                progress(done, len(figures), f"prepare pending figure review {identity}")
    review_path.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation preserves the exact inventory content reviewed here.
    # Only tool-pointer fields may differ later; source objects are not mutable
    # under this binding. The validator checks that boundary explicitly.
    with snapshot_path.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n")
    inventory["figure_reviews_path"] = str(review_path)
    inventory_path.write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    ledger = {
        "schema_version": 1,
        "inventory": file_record(snapshot_path),
        "inventory_path": str(inventory_path),
        "evidence_boundary": "These are reviewer-supplied records. Their existence or self-reported passes do not prove independent visual inspection.",
        "figures": rows,
    }
    review_path.write_text(json.dumps(ledger, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return review_path


def main() -> int:
    if not SOURCE_PDFS or WORK_DIRECTORY is None:
        print("Set SOURCE_PDFS, SOURCE_ROLES and WORK_DIRECTORY at the top of this script.")
        return 2
    path = prepare_sources(SOURCE_PDFS, WORK_DIRECTORY, source_roles=SOURCE_ROLES, glossary_path=GLOSSARY_PATH)
    print(f"Source inventory created: {path}")
    print("Pending classifications still require source inspection. No acceptance has been granted.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
