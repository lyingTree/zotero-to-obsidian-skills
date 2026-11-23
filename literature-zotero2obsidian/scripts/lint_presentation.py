"""Read-only presentation prompts; inspect each prompt against its rule.

Run in an editor after setting NOTE_PATH, or import lint_presentation(text).
No automatic replacements or source-fidelity claims are made here.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys

import yaml


NOTE_PATH = Path("candidate.md")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
WIKI = re.compile(r"\[\[([^\]\n]+)\]\]")
INLINE_CODE = re.compile(r"(`+).*?\1(?!`)")
INLINE_MATH = re.compile(r"(?<!\\)\$(?!\$)(?:\\.|[^$\n])*(?<!\\)\$")
EMAIL = re.compile(r"(?<![\w@])[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+")
DOI = re.compile(r"\b10\.\d{4,9}/[^\s<>]+", re.IGNORECASE)
URL = re.compile(r"(?:https?://|www\.)[^\s<>]+", re.IGNORECASE)
PERCENT = re.compile(r"\d+(?:\.\d+)?\s*[%％]")
NON_ASCII_DASH = re.compile(r"[\u2010-\u2015]")
SPACE = re.compile(r"[\u3400-\u9fff] +(?=[0-9A-Za-zⓜ])|[0-9A-Za-zⓜ] +(?=[\u3400-\u9fff])")
QUANTITY = re.compile(
    r"(?<![0-9A-Za-z_.])(?P<number>\d+(?:\.\d+)?(?:\s*[-\u2010-\u2015]\s*\d+(?:\.\d+)?)?)"
    r"\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年|km|cm|mm|m|h|s)(?![A-Za-z])"
)


def _sibling(name: str):
    path = Path(__file__).resolve().parent.joinpath(name + ".py")
    identifier = "literature_presentation_" + hashlib.sha256(str(path).encode()).hexdigest()[:16]
    if identifier in sys.modules:
        return sys.modules[identifier]
    specification = importlib.util.spec_from_file_location(identifier, path)
    if specification is None or specification.loader is None:
        raise ImportError(path)
    module = importlib.util.module_from_spec(specification)
    sys.modules[identifier] = module
    specification.loader.exec_module(module)
    return module


note_checks = _sibling("validate_note")
delivery_checks = _sibling("validate_delivery")


def _replace_links(text: str, *, keep_labels: bool) -> str:
    parts = []
    start = 0
    for link in delivery_checks.scan_markdown_links(text):
        parts.append(text[start:link.start])
        parts.append(link.label if keep_labels else "")
        start = link.end
    parts.append(text[start:])
    return "".join(parts)


@dataclass(frozen=True)
class Prompt:
    rule_id: str
    line: int
    code: str
    excerpt: str
    message: str
    level: str = "review"


def _visible(text: str) -> str:
    value = INLINE_CODE.sub("", text)
    value = _replace_links(value, keep_labels=True)
    value = WIKI.sub(lambda match: match[1].split("|", 1)[-1], value)
    value = re.sub(r"<[^>]*>", "", value)
    return value


def _scan(text: str, line: int) -> list[Prompt]:
    if not text.strip():
        return []
    findings = []
    active = None
    visible_lines = []
    for source_line in text.splitlines():
        source_line, active = note_checks._mask_comments(source_line, active)
        visible_lines.append(source_line)
    text = "\n".join(visible_lines)
    # A valid existing Markdown/autolink must not be flagged as a naked link.
    naked = INLINE_CODE.sub("", text)
    naked = _replace_links(naked, keep_labels=False)
    naked = WIKI.sub("", naked)
    naked = re.sub(r"<(?:https?://|mailto:)[^>]+>", "", naked, flags=re.IGNORECASE)
    naked = INLINE_MATH.sub("", naked)
    for pattern, rule, code, message in (
        (EMAIL, "BODY-C260", "NAKED_EMAIL", "核对邮箱来源后建立 mailto 链接。"),
        (DOI, "BODY-C269", "NAKED_DOI", "核对 DOI 身份后建立对应链接。"),
        (URL, "BODY-C54", "NAKED_URL", "核对网址边界后建立 Markdown 链接。"),
    ):
        for match in pattern.finditer(naked):
            findings.append(Prompt(rule, line, code, match[0], message))
    visible = _visible(text)
    for match in NON_ASCII_DASH.finditer(visible):
        findings.append(Prompt("BODY-C173", line, "VISIBLE_DASH", match[0], "核对该可见连接线是否应使用 ASCII -；不得修改链接目标。"))
    prose = INLINE_MATH.sub("ⓜ", visible)
    for match in PERCENT.finditer(prose):
        findings.append(Prompt("BODY-C184", line, "PERCENT_OUTSIDE_MATH", match[0], "百分量整体置于数学环境，百分号写作 \\%。"))
    for match in QUANTITY.finditer(prose):
        number, unit = match["number"], match["unit"]
        if unit == "年" and number.isdigit() and 1500 <= int(number) <= 2199:
            continue  # Publication/calendar years are explicitly exempt.
        findings.append(Prompt("BODY-C169", line, "QUANTITY_OUTSIDE_MATH", match[0], "核对这是量值而非日期/编号；适用时数值与单位放入同一数学环境。"))
    for match in SPACE.finditer(prose):
        findings.append(Prompt("BODY-C69", line, "CJK_SPACE", match[0], "核对并移除中文与数字、拉丁字母或数学环境之间的多余空格。"))
    return findings


def _table_strings(value: object):
    if not isinstance(value, dict):
        return
    sheets = value.get("sheets") if value.get("version") == 3 else [value]
    if not isinstance(sheets, list):
        return
    for sheet in sheets:
        if not isinstance(sheet, dict):
            continue
        for key in ("title", "footer"):
            if isinstance(sheet.get(key), str):
                yield sheet[key]
        columns = sheet.get("columns", [])
        if isinstance(columns, list):
            for column in columns:
                if isinstance(column, dict) and isinstance(column.get("name"), str):
                    yield column["name"]
        rows = sheet.get("rows", [])
        if not isinstance(rows, list):
            continue
        for row in rows:
            if not isinstance(row, dict) or not isinstance(row.get("cells"), dict):
                continue
            for cell in row["cells"].values():
                if isinstance(cell, str):
                    yield cell


def lint_presentation(text: str) -> list[Prompt]:
    """Return review prompts, including visible values in every Rich Table sheet.

    Syntax errors remain the responsibility of validate_note. These heuristics
    never prove completeness, correct translation, or semantic correctness.
    """
    lines = text.splitlines()
    findings: list[Prompt] = []
    start = 0
    if lines and lines[0].lstrip("\ufeff") == "---":
        try:
            start = lines.index("---", 1) + 1
        except ValueError:
            return findings
    fence = None
    payload: list[str] = []
    in_math = False
    comment = None
    for position in range(start, len(lines)):
        line = lines[position]
        match = FENCE.match(line)
        if fence:
            marker, length, language, first_line = fence
            closes = match and match[1][0] == marker and len(match[1]) >= length and not match[2].strip()
            if not closes:
                payload.append(line)
                continue
            if language == "rich-table":
                try:
                    table = yaml.safe_load(note_checks._rich_table_payload(payload))
                except (yaml.YAMLError, ValueError):
                    table = None
                for value in _table_strings(table):
                    findings.extend(_scan(value, first_line))
            fence, payload = None, []
            continue
        line, comment = note_checks._mask_comments(line, comment)
        match = FENCE.match(line)
        if match:
            fence = (match[1][0], len(match[1]), match[2].strip(), position + 1)
            continue
        if line.strip().startswith("$$"):
            if line.strip().count("$$") == 1:
                in_math = not in_math
            continue
        if in_math or re.match(r"^\^[A-Za-z0-9-]+\s*$", line):
            continue
        findings.extend(_scan(line, position + 1))
    return list(dict.fromkeys(findings))


def lint_file(path: Path) -> list[Prompt]:
    return lint_presentation(path.read_text(encoding="utf-8-sig"))


def main() -> None:
    findings = lint_file(NOTE_PATH)
    print(json.dumps({"prompts": [asdict(item) for item in findings], "source_fidelity": "not_verified"}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
