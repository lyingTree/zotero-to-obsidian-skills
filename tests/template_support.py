"""Generate regression inputs through the real Templater files, without a vault.

Node runs the template program. PyYAML supplies the mocked Obsidian YAML codec;
the template's control flow, payload, document wrappers and final note are real.
Neither codec emulation nor this harness proves Obsidian rendering behavior.
"""

from __future__ import annotations

import copy
from functools import lru_cache
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Any

import yaml


ROOT = Path(__file__).resolve().parents[1]
TEMPLATE_TYPES = {
    "期刊论文.md": "journalArticle",
    "学位论文.md": "thesis",
    "会议论文.md": "conferencePaper",
    "预印本.md": "preprint",
}
TABLE_BLOCK = re.compile(r"(?m)^```rich-table\n(?P<payload>[\s\S]*?)^```[ \t]*$")


def yaml_payload(payload: str) -> dict[str, Any]:
    """Decode the wrapper emitted by the template, retaining its actual data."""
    lines = payload.splitlines()
    if lines and lines[0] == "---" and lines[-1] == "---":
        lines.pop()
    data = yaml.safe_load("\n".join(lines))
    if not isinstance(data, dict):
        raise ValueError("Template Rich Table payload must be a mapping")
    return data


def table_data(note: str) -> dict[str, Any]:
    match = TABLE_BLOCK.search(note)
    if match is None:
        raise ValueError("The rendered note has no Rich Table")
    return yaml_payload(match["payload"])


def replace_table(note: str, data: dict[str, Any], *, wrapped: bool = True) -> str:
    """Apply a focused fault/variant to an already-rendered template table."""
    match = TABLE_BLOCK.search(note)
    if match is None:
        raise ValueError("The rendered note has no Rich Table")
    payload = yaml.safe_dump(data, allow_unicode=True, sort_keys=False).rstrip()
    if wrapped:
        payload = f"---\n{payload}\n---"
    block = f"```rich-table\n{payload}\n```"
    return note[:match.start()] + block + note[match.end():]


def document_body(table: str) -> str:
    return (
        "## 1 正文\n"
        "表内引用可回溯到本文书目。\n\n"
        f"{table}\n^tbl-1\n\n"
        "## 2 参考文献\n"
        "1. Example Author, 2025. A fixture reference. ^ref-1\n"
    )


def v2_body() -> str:
    return document_body("| Variable | Source |\n| --- | --- |\n| u | [1](<#^ref-1>) |")


def workbook_body(template_name: str) -> str:
    """Seed each sheet from an actual template-generated v2 table, then import v3."""
    generated = table_data(render_template(template_name, v2_body()))
    sheets = []
    for number in (1, 2):
        sheet = copy.deepcopy(generated)
        sheet.pop("version")
        sheet.update({
            "id": f"sheet-{number}", "name": f"Sheet {number}",
            "title": f"工作表 {number}", "footer": "[1](<#^ref-1>)",
            # Deliberately differ from the required result, so the real template
            # must normalize every sheet rather than pass through ideal data.
            "locked": False, "theme": "minimal",
        })
        sheets.append(sheet)
    workbook = {"version": 3, "title": "**表1：** Workbook", "sheets": sheets}
    return document_body("```rich-table\n" + json.dumps(workbook, ensure_ascii=False) + "\n```")


@lru_cache(maxsize=32)
def _render(template_name: str, template: str, body: str) -> dict[str, Any]:
    item = {
        "itemType": TEMPLATE_TYPES[template_name], "title": "Template integration fixture",
        "date": "2025", "publicationTitle": "Fixture Journal",
        "creators": [{"creatorType": "author", "firstName": "Example", "lastName": "Author"}],
        "tags": [],
    }
    profile = {
        "originalTitle": item["title"], "titleZh": "模板集成测试", "keywords": [],
        "authors": [{"name": "Example Author", "email": "", "units": ["1"], "corresponding": False}],
        "affiliations": [{"id": "1", "name": "Example Institute"}],
    }
    payload = {
        "templateName": template_name, "template": template, "body": body,
        "item": item, "profile": profile,
        "python": sys.executable, "bridge": str(Path(__file__).resolve()),
    }
    completed = subprocess.run(
        ["node", str(ROOT.joinpath("tests", "template_harness.js"))],
        input=json.dumps(payload, ensure_ascii=False), text=True, encoding="utf-8",
        capture_output=True, check=True, timeout=30,
    )
    return json.loads(completed.stdout)


def render_template_result(template_name: str, body: str) -> dict[str, Any]:
    template = ROOT.joinpath("template", template_name).read_text(encoding="utf-8")
    # Include source text in the cache key: edits to a template invalidate reuse.
    return copy.deepcopy(_render(template_name, template, body))


def render_template(template_name: str, body: str) -> str:
    return render_template_result(template_name, body)["markdown"]


def _yaml_adapter() -> None:
    request = json.loads(sys.stdin.buffer.read().decode("utf-8"))
    operation = request["operation"]
    if operation == "parse":
        value = yaml.safe_load(request["value"])
    elif operation == "stringify":
        value = yaml.safe_dump(request["value"], allow_unicode=True, sort_keys=False)
    else:
        raise ValueError("Unsupported YAML test adapter operation")
    # Keep the pipe transport ASCII, including on Windows legacy code pages.
    sys.stdout.write(json.dumps(value, ensure_ascii=True))


if __name__ == "__main__":
    _yaml_adapter()
