"""Prepare PDF source evidence without claiming visual or semantic acceptance.

Set SOURCE_PDFS, SOURCE_ROLES and WORK_DIRECTORY below, then run this file.
Or call prepare_sources(source_pdfs, work_directory, source_roles=...).
The generated inventory deliberately contains pending decisions. Fill those
decisions from the rendered pages before constructing a delivery manifest.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
from typing import Callable, Mapping

import fitz


SOURCE_PDFS: dict[str, Path] = {}
SOURCE_ROLES: dict[str, str] = {}
WORK_DIRECTORY: Path | None = None
RENDER_DPI = 144
Progress = Callable[[int, int, str], None]


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
    width = 24
    filled = width * done // max(total, 1)
    print(f"[{'#' * filled}{'.' * (width - filled)}] {done}/{total} {label}", flush=True)


def prepare_sources(
    source_pdfs: Mapping[str, Path],
    work_directory: Path,
    *,
    source_roles: Mapping[str, str] | None = None,
    dpi: int = RENDER_DPI,
    progress: Progress | None = progress_bar,
) -> Path:
    """Render every PDF page and record every nonempty text-layer block.

    Source IDs are stable task-local names, not bibliography anchor prefixes.
    Existing inventories are never overwritten. Use a new work directory for
    another extraction, so a delivery cannot silently acquire different input.
    The text layer is diagnostic evidence; it is not the visible-page truth.
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
    resolved = {key: Path(value).resolve(strict=True) for key, value in source_pdfs.items()}
    page_counts: dict[str, int] = {}
    for key, path in resolved.items():
        with fitz.open(path) as document:
            if not document.is_pdf or document.needs_pass:
                raise ValueError(f"{key}: an accessible PDF is required")
            page_counts[key] = len(document)
    total = sum(page_counts.values())
    done = 0
    sources = []
    for key, path in resolved.items():
        directory = work_directory.joinpath("source-evidence", key)
        directory.mkdir(parents=True, exist_ok=True)
        pages = []
        with fitz.open(path) as document:
            for index, page in enumerate(document):
                page_number = index + 1
                text_path = directory.joinpath(f"page-{page_number:04d}.txt")
                image_path = directory.joinpath(f"page-{page_number:04d}.png")
                text_path.write_text(page.get_text("text", sort=False), encoding="utf-8")
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
                })
                done += 1
                if progress:
                    progress(done, total, f"{key} page {page_number}")
        sources.append({
            "id": key, "role": (source_roles or {}).get(key, "pending"),
            **file_record(path), "page_count": page_counts[key], "pages": pages,
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
    inventory_path.write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return inventory_path


def main() -> int:
    if not SOURCE_PDFS or WORK_DIRECTORY is None:
        print("Set SOURCE_PDFS, SOURCE_ROLES and WORK_DIRECTORY at the top of this script.")
        return 2
    path = prepare_sources(SOURCE_PDFS, WORK_DIRECTORY, source_roles=SOURCE_ROLES)
    print(f"Source inventory created: {path}")
    print("Pending classifications still require source inspection. No acceptance has been granted.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
