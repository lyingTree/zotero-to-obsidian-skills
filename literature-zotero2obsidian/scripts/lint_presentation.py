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
INLINE_MATH = re.compile(r"(?<!\\)\$(?!\$)(?:\\.|[^$\n])*(?<!\\)\$")
EMAIL = re.compile(r"(?<![\w@])[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+")
DOI = re.compile(r"\b10\.\d{4,9}/[^\s<>]+", re.IGNORECASE)
URL = re.compile(r"(?:https?://|www\.)[^\s<>]+", re.IGNORECASE)
PERCENT = re.compile(r"\d+(?:\.\d+)?\s*[%％]")
NON_ASCII_DASH = re.compile(r"[\u2010-\u2015]")
SPACE = re.compile(r"[\u3400-\u9fff] +(?=[0-9A-Za-zⓜ])|[0-9A-Za-zⓜ] +(?=[\u3400-\u9fff])")
NUMBERED_HEADING_PREFIX = re.compile(r"^ {0,3}#{1,6} +[0-9]+(?:\.[0-9]+)* +")
HEADING = re.compile(r"^ {0,3}#{1,6} +\S")
PANEL_LABEL = re.compile(r"(?P<prefix>图)?\s*(?P<object>S?\d+)(?P<panel>[a-z])", re.IGNORECASE)
PANEL_JOIN = re.compile(r"(?:\s|[,，、和及与\-\u2010-\u2015]|以及)*")
MATH_NUMBER_RANGE = re.compile(r"(?<![0-9A-Za-z_.])\d+(?:\.\d+)?\s*-\s*\d+(?:\.\d+)?(?![0-9.])")
QUANTITY = re.compile(
    r"(?<![0-9A-Za-z_.])(?P<number>\d+(?:\.\d+)?(?:\s*[-\u2010-\u2015]\s*\d+(?:\.\d+)?)?)"
    r"\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年|km|cm|mm|m|h|s)(?![A-Za-z])"
)
CHINESE_QUANTITY = re.compile(
    r"(?P<number>[数几]?[零〇一二三四五六七八九十百千万亿两半]+(?:[-\u2010-\u2015至到][数几]?[零〇一二三四五六七八九十百千万亿两半]+)?)"
    r"\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年)"
)
CHINESE_YEAR_DIGITS = str.maketrans("零〇一二三四五六七八九", "00123456789")


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


def _scan_math(text: str, line: int, *, display: bool) -> list[Prompt]:
    findings = [
        Prompt("BODY-C185", line, "MATH_RANGE_OR_SUBTRACTION", match[0],
               r"对照来源判断这是范围还是减法；范围使用 \text{-}，真实减法保留运算符。")
        for match in MATH_NUMBER_RANGE.finditer(text)
    ]
    if not display:
        return findings
    tail = text.rstrip()
    for _ in range(2):
        tail = re.sub(r"\s*\\tag\*?\{[^{}]*\}\s*$", "", tail).rstrip()
        tail = re.sub(r"(?:\\end\{(?:aligned|align\*?|gathered|gather\*?|split|equation\*?)\}\s*)+$", "", tail).rstrip()
    tail = re.sub(r"\\\\\s*$", "", tail).rstrip()
    punctuation = bool(re.search(r"(?<!\\)[,;，；。.]$", tail))
    if tail.endswith(".") and (tail.endswith("..") or re.search(r"\\(?:left|right|middle)\s*\.$", tail)):
        punctuation = False
    if punctuation:
        findings.append(Prompt("BODY-C202", line, "DISPLAY_TRAILING_PUNCTUATION", tail[-40:],
                               "核对显示公式末尾的标点；正文标点归入正文，数学分隔符保留。"))
    return findings


def _scan_panel_links(text: str, line: int) -> list[Prompt]:
    masked = note_checks._mask_delimited(note_checks._mask_delimited(text, "`"), "$")
    links = [link for link in delivery_checks.scan_markdown_links(masked) if not link.image]
    for match in WIKI.finditer(masked):
        if note_checks._escaped(masked, match.start()) or (match.start() and masked[match.start() - 1] == "!"):
            continue
        target, separator, label = match[1].partition("|")
        if not separator:
            continue
        links.append(delivery_checks.MarkdownLink(target, match.start(), match.end(), False, label))
    links.sort(key=lambda link: link.start)
    findings = []
    for left, right in zip(links, links[1:]):
        left_label = PANEL_LABEL.fullmatch(left.label)
        right_label = PANEL_LABEL.fullmatch(right.label)
        if left_label is None or right_label is None:
            continue
        if not left_label["prefix"] and not right_label["prefix"]:
            continue
        if left_label["object"].casefold() != right_label["object"].casefold() or left.href != right.href:
            continue
        if not PANEL_JOIN.fullmatch(masked[left.end:right.start]):
            continue
        findings.append(Prompt("LINK-C13", line, "SPLIT_PANEL_CALL", text[left.start:right.end],
                               "同一对象的同组多面板调用应整体链接；核对来源调用边界后处理本次范围内的新建调用。"))
    return findings


def _visible(text: str) -> str:
    value = note_checks._mask_delimited(text, "`")
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
    findings.extend(_scan_panel_links(text, line))
    math_visible = _replace_links(note_checks._mask_delimited(text, "`"), keep_labels=True)
    math_visible = WIKI.sub(lambda match: match[1].split("|", 1)[-1], math_visible)
    for math in INLINE_MATH.finditer(math_visible):
        findings.extend(_scan_math(math[0][1:-1], line, display=False))
    # A valid existing Markdown/autolink must not be flagged as a naked link.
    naked = note_checks._mask_delimited(text, "`")
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
    for match in (*QUANTITY.finditer(prose), *CHINESE_QUANTITY.finditer(prose)):
        number, unit = match["number"], match["unit"]
        years = re.split(r"\s*[-\u2010-\u2015至到]\s*", number.translate(CHINESE_YEAR_DIGITS))
        if unit == "年" and all(year.isdigit() and 1500 <= int(year) <= 2199 for year in years):
            continue  # Calendar years and ranges are dates, not durations.
        findings.append(Prompt("BODY-C169", line, "QUANTITY_OUTSIDE_MATH", match[0], "按完整原句核对这是量值而非日期/编号；适用时数值与单位放入同一数学环境，保留原有近似限定。"))
    heading_prefix = NUMBERED_HEADING_PREFIX.match(prose)
    for match in SPACE.finditer(prose):
        if heading_prefix and match.end() <= heading_prefix.end():
            continue  # The section-number separator is required heading syntax.
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


def _heading_spacing(events: list[tuple[str, int, str]]) -> list[Prompt]:
    """Check rendered block adjacency; hidden comments are not block events."""
    findings = []
    for index, (kind, line, text) in enumerate(events):
        if kind != "heading":
            continue
        if index and events[index - 1][0] != "blank":
            findings.append(Prompt("BODY-C67", line, "HEADING_GAP_MISSING", text,
                                   "标题前应留一空行；父标题直接接下级标题时也按该间距处理。"))
        if index > 1 and events[index - 1][0] == events[index - 2][0] == "blank":
            findings.append(Prompt("BODY-C67", line, "HEADING_GAP_EXTRA", text,
                                   "核对标题前的多余空行，保留一空行。"))
        following = index + 1
        while following < len(events) and events[following][0] == "blank":
            following += 1
        if following > index + 1 and following < len(events) and events[following][0] == "text":
            findings.append(Prompt("BODY-C68", line, "HEADING_BODY_GAP", text,
                                   "标题后的普通正文块紧接下一行；下级标题和显示公式按各自间距处理。"))
    return findings


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
    math_payload: list[str] = []
    math_first_line = 0
    comment = None
    events: list[tuple[str, int, str]] = []
    code_lines: dict[int, str] = {}
    list_indent = None
    last_visible: tuple[str, str] | None = None
    for position in range(start, len(lines)):
        raw = lines[position]
        line = raw
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
        if in_math:
            if line.strip().startswith("$$"):
                math_payload.append(line.strip()[2:])
                findings.extend(_scan_math("\n".join(math_payload), math_first_line, display=True))
                in_math, math_payload = False, []
            else:
                math_payload.append(line)
            continue
        indentation = len(raw.expandtabs(4)) - len(raw.expandtabs(4).lstrip(" "))
        continuation = list_indent is not None and indentation >= list_indent
        if raw.strip() and not continuation:
            list_indent = None
        if comment is None and indentation >= 4 and not (
            continuation and list_indent is not None and indentation < list_indent + 4
        ):
            events.append(("code", position + 1, raw))
            last_visible = ("code", raw.strip())
            continue
        numbered = note_checks._ENTRY.match(raw) if comment is None else None
        if numbered:
            list_indent = numbered.end()
        raw_fence = FENCE.match(raw) if comment is None else None
        if raw_fence:
            fence = (raw_fence[1][0], len(raw_fence[1]), raw_fence[2].strip(), position + 1)
            events.append(("code", position + 1, raw))
            last_visible, list_indent = ("code", raw.strip()), None
            continue
        if position not in code_lines:
            end = position + 1
            if raw.strip() and not HEADING.match(raw):
                while end < len(lines) and lines[end].strip() and not (
                    FENCE.match(lines[end]) or HEADING.match(lines[end]) or note_checks._ENTRY.match(lines[end])
                ):
                    end += 1
            paragraph = note_checks._mask_paragraph_code("\n".join(lines[position:end]), comment).split("\n")
            code_lines.update(enumerate(paragraph, start=position))
        code_safe = code_lines[position]
        active_comment = comment
        line, comment = note_checks._mask_comments(code_safe, comment)
        if not line.strip():
            if not raw.strip() and active_comment is None:
                events.append(("blank", position + 1, ""))
            elif raw.strip() and not code_safe.strip():
                events.append(("code", position + 1, raw))
                last_visible = ("code", raw.strip())
            continue
        match = FENCE.match(line)
        if match:
            fence = (match[1][0], len(match[1]), match[2].strip(), position + 1)
            events.append(("code", position + 1, line))
            last_visible = ("code", line.strip())
            continue
        if line.strip().startswith("$$"):
            stripped = line.strip()
            if stripped.count("$$") > 1:
                findings.extend(_scan_math(stripped[2:stripped.rfind("$$")], position + 1, display=True))
            else:
                in_math, math_payload, math_first_line = True, [stripped[2:]], position + 1
            events.append(("math", position + 1, line))
            last_visible = ("math", stripped)
            continue
        if re.match(r"^\^[A-Za-z0-9-]+\s*$", line):
            if line.startswith("^tbl-fig-") and not re.fullmatch(r"\^tbl-fig-[A-Za-z0-9]+(?:-[A-Za-z0-9]+)+\s*", line):
                findings.append(Prompt("LINK-C133", position + 1, "EMBEDDED_TABLE_SOURCE_TOKEN", line,
                                       "新建图内表锚应含来源 token 与图原编号 token；已核实的既有 ID 保留，仅按本次新建范围核对。"))
            events.append(("anchor", position + 1, line))
            last_visible = ("anchor", line.strip())
            continue
        kind = "heading" if HEADING.match(line) else "text"
        events.append((kind, position + 1, line))
        if re.fullmatch(r" {0,3}##\s+Zotero原条目(?:\s+#+)?\s*", _visible(line)) and last_visible != ("text", "---"):
            findings.append(Prompt("LINK-C120", position + 1, "ZOTERO_SEPARATOR_MISSING", line,
                                   "文献内容与末尾 Zotero 原条目标题之间应有独立的 --- 分隔线。"))
        findings.extend(_scan(line, position + 1))
        last_visible = (kind, line.strip())
    findings.extend(_heading_spacing(events))
    return list(dict.fromkeys(findings))


def lint_file(path: Path) -> list[Prompt]:
    return lint_presentation(path.read_text(encoding="utf-8-sig"))


def main() -> None:
    findings = lint_file(NOTE_PATH)
    print(json.dumps({"prompts": [asdict(item) for item in findings], "source_fidelity": "not_verified"}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
