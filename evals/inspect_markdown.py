"""Collect read-only Markdown evidence; never infer semantic acceptance."""

from __future__ import annotations

from dataclasses import asdict
from hashlib import sha256
from pathlib import Path
import re
import sys

from PIL import Image
import yaml

SKILLS_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SKILLS_ROOT.joinpath("literature-zotero2obsidian", "scripts")))
from validate_note import validate_note
from lint_presentation import lint_presentation


def collection_styles(node: yaml.Node | None, path: str = "",
                      ancestors: frozenset[int] = frozenset()) -> list[dict]:
    """Locate columns/rows styles, including every v3 sheet, without judging data."""
    if node is None or id(node) in ancestors:
        return []
    ancestors = ancestors | {id(node)}
    result = []
    if isinstance(node, yaml.MappingNode):
        for key, value in node.value:
            name = str(key.value)
            child_path = f"{path}.{name}" if path else name
            if name in ("columns", "rows") and isinstance(value, yaml.SequenceNode):
                result.append({"path": child_path,
                               "style": "flow" if value.flow_style else "block"})
            result.extend(collection_styles(value, child_path, ancestors))
        return result
    if isinstance(node, yaml.SequenceNode):
        for index, value in enumerate(node.value):
            result.extend(collection_styles(value, f"{path}[{index}]", ancestors))
    return result


def inspect_markdown(path: Path, resource_root: Path) -> dict:
    """Return inspection clues and static findings, not a pass/fail score."""
    data = path.read_bytes()
    resource_root = resource_root.resolve(strict=True)
    text = data.decode("utf-8-sig")
    lines = text.splitlines()
    headings = []
    references = []
    anchors = []
    images = []
    for number, line in enumerate(lines, 1):
        if re.match(r"^#{1,6} ", line):
            headings.append({"line": number, "text": line})
        match = re.match(r"^\s*\d+\.\s.*\^(?:ref|sref)[\w-]+\s*$", line)
        if match:
            references.append({"line": number, "text": line})
        for match in re.finditer(r"(?:^|\s)\^([\w-]+)(?=\s|$)", line):
            anchors.append({"line": number, "id": match.group(1)})
        targets = [(match.group(1).split("|", 1)[0], "wiki") for match in
                   re.finditer(r"!\[\[([^\]\n]+)\]\]", line)]
        targets.extend((match.group(1).strip("<>"), "markdown") for match in
                       re.finditer(r"!\[[^\]\n]*\]\((<[^>\n]+>|[^)\n]+)\)", line))
        for target, syntax in targets:
            record = {"line": number, "target": target, "syntax": syntax}
            if re.match(r"[a-zA-Z]+://", target):
                record["remote"] = True
                images.append(record)
                continue
            asset = resource_root.joinpath(target).resolve()
            if not asset.is_relative_to(resource_root):
                record["outside_resource_root"] = True
                images.append(record)
                continue
            record["exists"] = asset.is_file()
            if asset.is_file():
                record["sha256"] = sha256(asset.read_bytes()).hexdigest()
                try:
                    with Image.open(asset) as image:
                        record.update(width=image.width, height=image.height, format=image.format)
                except (OSError, ValueError) as error:
                    record["image_decode_error"] = str(error)
            images.append(record)
    tables = []
    for match in re.finditer(r"^```rich-table[^\n]*\n(.*?)^```\s*$", text, re.M | re.S):
        payload = match.group(1).strip()
        if payload.startswith("---\n") and payload.endswith("\n---"):
            payload = payload[4:-4]
        record = {"line": text.count("\n", 0, match.start()) + 1}
        try:
            record["data"] = yaml.safe_load(payload)
            record["collection_styles"] = collection_styles(yaml.compose(payload))
        except yaml.YAMLError as error:
            record["parse_error"] = str(error)
        tables.append(record)
    equations = [{"line": text.count("\n", 0, match.start()) + 1,
                  "text": match.group(0)} for match in
                 re.finditer(r"\$\$[\s\S]*?\$\$", text)]
    return {
        "path": str(path.resolve()), "sha256": sha256(data).hexdigest(),
        "bytes": len(data), "lines": len(lines),
        "headings": headings, "references": references, "anchors": anchors,
        "image_references": images, "rich_tables": tables, "display_equations": equations,
        "static_findings": [asdict(item) for item in validate_note(text)],
        "presentation_prompts": [asdict(item) for item in lint_presentation(text)],
        "acceptance": "not_assessed; counts and diagnostics require source review",
    }
