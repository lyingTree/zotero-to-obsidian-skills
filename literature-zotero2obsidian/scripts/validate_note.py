"""Read-only checks for generated literature Markdown, not a rendering verdict.

Edit NOTE_PATH below and run this file, or import validate_note / validate_file.
Checks cover frontmatter, top-level fenced Rich Tables, display equations, block
IDs, explicit same-note block links, and recognizable Chinese bibliography
sections. Ordinary fenced / indented code, inline code and mathematics, HTML
comments and Obsidian comments are excluded. This deliberately does not resolve
heading links, reference-style Markdown links, nested plugin syntax, external
files, source fidelity, translation quality, or Obsidian's actual block ranges.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
from typing import Any

import yaml


# Set this to a candidate Markdown file. No file is read when it remains None.
NOTE_PATH: Path | None = None


@dataclass(frozen=True)
class Finding:
    level: str
    code: str
    line: int
    message: str


class _UniqueKeyLoader(yaml.SafeLoader):
    """Reject duplicate literal mapping keys, while allowing YAML merge overrides."""

    def construct_mapping(self, node: yaml.MappingNode, deep: bool = False) -> dict:
        seen: dict[Any, yaml.Node] = {}
        for key_node, _ in node.value:
            if key_node.tag == "tag:yaml.org,2002:merge":
                continue
            key = self.construct_object(key_node, deep=deep)
            try:
                repeated = key in seen
            except TypeError as error:
                raise yaml.constructor.ConstructorError(
                    None, None, "mapping keys must be scalar", key_node.start_mark
                ) from error
            if repeated:
                raise yaml.constructor.ConstructorError(
                    "duplicate mapping key", seen[key].start_mark,
                    f"duplicate key: {key!r}", key_node.start_mark,
                )
            seen[key] = key_node
        self.flatten_mapping(node)
        return super().construct_mapping(node, deep=deep)


_FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
_ANCHOR = re.compile(r"(?<!\S)\^([A-Za-z0-9-]+)[ \t]*$")
_WHOLE_ANCHOR = re.compile(r"^\^([A-Za-z0-9-]+)[ \t]*$")
_HEADING = re.compile(r"^ {0,3}(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$")
_ENTRY = re.compile(r"^ {0,3}([1-9][0-9]*)\.[ \t]+")
_BIBLIOGRAPHY = re.compile(r"^(?:[0-9]+(?:\.[0-9]+)*[ \t]+)?参考文献$")
_SUPPLEMENT = re.compile(r"^(?:[0-9]+[ \t]+)?补充材料$")
# Match Citation Preview 1.3.0 referenceAnchor(), not an invented ID grammar.
# Source: lyingTree/citation-preview @ 165503f647590b7d9379e8b1ba72a2b9b61de049
# src/bibliography.ts. Prefix segments must start with an ASCII letter.
_CITATION_ANCHOR_COMPACT = re.compile(r"(s?ref)-?([1-9][0-9]{0,8})", re.IGNORECASE | re.ASCII)
_CITATION_ANCHOR_NAMESPACED = re.compile(
    r"((?:s?ref)(?:-[A-Za-z][A-Za-z0-9]*)+)-([1-9][0-9]{0,8})",
    re.IGNORECASE | re.ASCII,
)
_LOCAL_MARKDOWN = re.compile(
    r"\[(?:\\.|[^\[\]\\\n])*\]\([ \t]*"
    r"(?:<#\^(?P<angle>[A-Za-z0-9-]+)>|#\^(?P<bare>[A-Za-z0-9-]+))"
    r"(?:[ \t]+(?:\"[^\"\n]*\"|'[^'\n]*'|\([^\n)]*\)))?[ \t]*\)"
)
_LOCAL_WIKI = re.compile(r"\[\[#\^(?P<id>[A-Za-z0-9-]+)(?:\|[^\]\n]*)?\]\]")
_YAML_ERROR = object()


def _blank(text: str) -> str:
    return "".join("\n" if char == "\n" else " " for char in text)


def _escaped(text: str, index: int) -> bool:
    start = index
    while start and text[start - 1] == "\\":
        start -= 1
    return (index - start) % 2 == 1


def _mask_delimited(text: str, character: str) -> str:
    """Mask paired exact-length code/math delimiters; retain line positions."""
    runs = [match for match in re.finditer(re.escape(character) + r"+", text)
            if not _escaped(text, match.start())]
    next_same: dict[int, int] = {}
    pairs: dict[int, int] = {}
    for index in range(len(runs) - 1, -1, -1):
        length = len(runs[index].group())
        if length in next_same:
            pairs[index] = next_same[length]
        next_same[length] = index
    parts: list[str] = []
    cursor = 0
    index = 0
    while index < len(runs):
        closing = pairs.get(index)
        if closing is None:
            index += 1
            continue
        opening, ending = runs[index], runs[closing]
        parts.append(text[cursor:opening.start()])
        parts.append(_blank(text[opening.start():ending.end()]))
        cursor = ending.end()
        index = closing + 1
    parts.append(text[cursor:])
    return "".join(parts)


def _mask_paragraph_code(text: str, active_comment: str | None) -> str:
    """Mask cross-line code before interpreting its literal comment markers."""
    tokens = list(re.finditer(r"<!--|%%|`+", text))
    next_tick: dict[int, int] = {}
    pairs: dict[int, int] = {}
    for index in range(len(tokens) - 1, -1, -1):
        token = tokens[index]
        if not token.group().startswith("`") or _escaped(text, token.start()):
            continue
        length = len(token.group())
        if length in next_tick:
            pairs[index] = next_tick[length]
        next_tick[length] = index
    parts: list[str] = []
    cursor = 0
    index = 0
    while index < len(tokens):
        token = tokens[index]
        if active_comment:
            closing = text.find(active_comment, token.start())
            if closing < 0:
                break
            end = closing + len(active_comment)
            while index < len(tokens) and tokens[index].start() < end:
                index += 1
            active_comment = None
            continue
        if _escaped(text, token.start()):
            index += 1
            continue
        if token.group() in {"<!--", "%%"}:
            active_comment = "-->" if token.group() == "<!--" else "%%"
            # Start after the opener so %% does not close itself.
            closing = text.find(active_comment, token.end())
            if closing < 0:
                break
            end = closing + len(active_comment)
            while index < len(tokens) and tokens[index].start() < end:
                index += 1
            active_comment = None
            continue
        closing_index = pairs.get(index)
        if closing_index is None:
            index += 1
            continue
        end = tokens[closing_index].end()
        parts.append(text[cursor:token.start()])
        parts.append(_blank(text[token.start():end]))
        cursor, index = end, closing_index + 1
    parts.append(text[cursor:])
    return "".join(parts)


def _mask_comments(line: str, active: str | None) -> tuple[str, str | None]:
    pieces: list[str] = []
    index = 0
    while index < len(line):
        if active:
            end = line.find(active, index)
            if end < 0:
                pieces.append(_blank(line[index:]))
                break
            end += len(active)
            pieces.append(_blank(line[index:end]))
            index, active = end, None
            continue
        if not _escaped(line, index) and line.startswith("<!--", index):
            pieces.append("    ")
            active, index = "-->", index + 4
            continue
        if not _escaped(line, index) and line.startswith("%%", index):
            pieces.append("  ")
            active, index = "%%", index + 2
            continue
        pieces.append(line[index])
        index += 1
    return "".join(pieces), active


def _links(text: str, line: int) -> list[tuple[str, int]]:
    visible = _mask_delimited(_mask_delimited(text, "`"), "$")
    result: list[tuple[str, int]] = []
    for pattern in (_LOCAL_MARKDOWN, _LOCAL_WIKI):
        for match in pattern.finditer(visible):
            if _escaped(visible, match.start()):
                continue
            target = next(value for value in match.groupdict().values() if value)
            result.append((target, line + visible.count("\n", 0, match.start())))
    return result


def _load_yaml(text: str, first_line: int, code: str,
               findings: list[Finding]) -> Any:
    try:
        return yaml.load(text, Loader=_UniqueKeyLoader)
    except (yaml.YAMLError, RecursionError, ValueError) as error:
        mark = getattr(error, "problem_mark", None)
        problem = getattr(error, "problem", None) or str(error).splitlines()[0]
        line = first_line + (mark.line if mark else 0)
        findings.append(Finding("error", code, line, f"YAML 无法安全解析：{problem}"))
        return _YAML_ERROR


def _frontmatter(lines: list[str], findings: list[Finding]) -> int:
    if not lines or lines[0].strip() != "---":
        return 0
    closing = next((index for index in range(1, len(lines))
                    if lines[index].strip() in {"---", "..."}), None)
    if closing is None:
        findings.append(Finding("error", "FRONTMATTER_UNCLOSED", 1, "起始 YAML 未闭合。"))
        return len(lines)
    data = _load_yaml("\n".join(lines[1:closing]), 2, "FRONTMATTER_YAML", findings)
    if data is not _YAML_ERROR and data is not None and not isinstance(data, dict):
        findings.append(Finding("error", "FRONTMATTER_TYPE", 1, "笔记属性必须是 YAML 映射。"))
    if isinstance(data, dict) and "库内参考文献" in data:
        findings.append(Finding("error", "DEPRECATED_PROPERTY", 1,
                                "属性中仍有废弃键“库内参考文献”。"))
    return closing + 1


def _strings(value: Any, seen: set[int] | None = None):
    """Yield scalar values without following recursive YAML aliases indefinitely."""
    if isinstance(value, str):
        yield value
        return
    if not isinstance(value, (dict, list)):
        return
    if seen is None:
        seen = set()
    if id(value) in seen:
        return
    seen.add(id(value))
    values = value.values() if isinstance(value, dict) else value
    for child in values:
        yield from _strings(child, seen)


def _rich_table_payload(content: list[str]) -> str:
    """Accept the templates' outer --- pair without accepting multiple documents.

    Replacing only the closing wrapper preserves source offsets. Indented ---
    inside a YAML block scalar is data and must not be stripped.
    """
    nonempty = [index for index, line in enumerate(content) if line.strip()]
    if len(nonempty) < 2:
        return "\n".join(content)
    marker = re.compile(r"^---[ \t]*(?:#.*)?$")
    first, last = nonempty[0], nonempty[-1]
    if not marker.fullmatch(content[first]) or not marker.fullmatch(content[last]):
        return "\n".join(content)
    payload = content.copy()
    payload[last] = ""
    return "\n".join(payload)


def _table_visible_links(data: dict, start: int, context: str,
                         links: list[tuple[str, int, str]]) -> None:
    # IDs, xlsx paths and arbitrary plugin metadata are not Markdown text.
    fields: list[Any] = [data.get("title"), data.get("footer")]
    for column in data.get("columns", []) if isinstance(data.get("columns"), list) else []:
        if isinstance(column, dict):
            fields.append(column.get("name"))
    for row in data.get("rows", []) if isinstance(data.get("rows"), list) else []:
        if isinstance(row, dict):
            fields.append(row.get("cells"))
    for field in fields:
        for value in _strings(field):
            # YAML values may be folded, escaped or on a single JSON line.
            # Locate the sheet, not a guessed source line for a decoded cell.
            links.extend((target, start, context) for target, _ in _links(value, 1))


def _rich_table_sheet(data: dict, start: int, context: str,
                      findings: list[Finding], links: list[tuple[str, int, str]]) -> None:
    if data.get("locked") is not True:
        findings.append(Finding("error", "TABLE_UNLOCKED", start, f"{context}：locked 必须为 true。"))
    if data.get("theme") != "grid":
        findings.append(Finding("error", "TABLE_THEME", start, f"{context}：theme 必须为 grid。"))
    for key in ("columns", "rows"):
        if not isinstance(data.get(key), list) or not data[key]:
            findings.append(Finding("error", "TABLE_STRUCTURE", start,
                                    f"{context}：{key} 必须为非空列表。"))
    _table_visible_links(data, start, context, links)


def _sheet_source_lines(payload: str, start: int) -> list[int]:
    """Return real sheet-node locations after the payload has passed YAML checks."""
    root = yaml.compose(payload, Loader=yaml.SafeLoader)
    if not isinstance(root, yaml.MappingNode):
        return []
    for key, value in root.value:
        if key.value != "sheets" or not isinstance(value, yaml.SequenceNode):
            continue
        return [start + 1 + sheet.start_mark.line for sheet in value.value]
    # A YAML merge may provide sheets indirectly. The owning fence remains an
    # honest fallback location; the message still identifies the sheet.
    return []


def _rich_table(content: list[str], start: int, findings: list[Finding],
                links: list[tuple[str, int, str]]) -> None:
    payload = _rich_table_payload(content)
    data = _load_yaml(payload, start + 1, "TABLE_YAML", findings)
    if data is _YAML_ERROR:
        return
    if not isinstance(data, dict):
        findings.append(Finding("error", "TABLE_TYPE", start, "Rich Table 必须是 YAML 映射。"))
        return
    version = data.get("version")
    if type(version) is not int or version not in (2, 3):
        findings.append(Finding("error", "TABLE_VERSION", start,
                                "Rich Table version 必须为整数 2（单表）或 3（多工作表）。"))
    if version != 3 or type(version) is not int:
        _rich_table_sheet(data, start, "Rich Table", findings, links)
        return
    sheets = data.get("sheets")
    if not isinstance(sheets, list) or not sheets:
        findings.append(Finding("error", "TABLE_STRUCTURE", start,
                                "Rich Table v3 的 sheets 必须为非空列表。"))
        return
    _table_visible_links(data, start, "Rich Table 工作簿", links)
    locations = _sheet_source_lines(payload, start)
    for index, sheet in enumerate(sheets):
        location = locations[index] if index < len(locations) else start
        context = f"Rich Table 工作表 {index + 1}"
        if not isinstance(sheet, dict):
            findings.append(Finding("error", "TABLE_SHEET_TYPE", location,
                                    f"{context}必须是 YAML 映射。"))
            continue
        labels = [f"{key}={sheet[key]}" for key in ("name", "id")
                  if isinstance(sheet.get(key), str) and sheet[key]]
        if labels:
            context += "（" + "，".join(labels) + "）"
        _rich_table_sheet(sheet, location, context, findings, links)


def _object_anchor(lines: list[str], closing: int, prefix: str,
                   findings: list[Finding], required: bool) -> None:
    following = closing + 1
    while following < len(lines) and not lines[following].strip():
        following += 1
    match = _WHOLE_ANCHOR.fullmatch(lines[following]) if following < len(lines) else None
    if not match or not match[1].startswith(prefix):
        if required:
            findings.append(Finding("error", "TABLE_ANCHOR", closing + 1,
                                    "Rich Table 闭合围栏下一行须为顶格独占的 ^tbl-…。"))
        return
    if following != closing + 1:
        findings.append(Finding("error", "ANCHOR_GAP", following + 1,
                                "对象闭合行与其锚点之间不得有空行。"))
    if following + 1 < len(lines) and lines[following + 1].strip():
        findings.append(Finding("error", "ANCHOR_SPACING", following + 1,
                                "对象锚点与后续内容之间须留一空行。"))


def _citation_preview_anchor(anchor: str) -> tuple[str, int] | None:
    match = _CITATION_ANCHOR_COMPACT.fullmatch(anchor) or _CITATION_ANCHOR_NAMESPACED.fullmatch(anchor)
    if match is None:
        return None
    return match[1].lower(), int(match[2])


def _bibliography(lines: list[str], findings: list[Finding]) -> None:
    supplement = False
    bibliography_level: int | None = None
    entry: tuple[int, int, bool] | None = None
    entry_lines: list[str] = []

    def finish() -> None:
        if entry is None:
            return
        number, first, in_si = entry
        anchors = [match[1] for line in entry_lines if (match := _ANCHOR.search(line))]
        if len(anchors) != 1:
            findings.append(Finding("error", "BIBLIOGRAPHY_ANCHOR", first,
                                    "每条书目须有且仅有一个末尾参考文献块 ID。"))
            return
        last = next((line for line in reversed(entry_lines) if line.strip()), "")
        if not _ANCHOR.search(last):
            findings.append(Finding("error", "BIBLIOGRAPHY_ANCHOR", first,
                                    "参考文献块 ID 须放在该书目条目的末尾。"))
            return
        anchor = anchors[0]
        parsed = _citation_preview_anchor(anchor)
        if parsed is None:
            recommended = "sref-N 或 sref-src<source_token>-N" if in_si else "ref-N"
            findings.append(Finding(
                "warning", "BIBLIOGRAPHY_PLUGIN_COMPAT", first,
                f"块 ID ^{anchor} 保留，但 Citation Preview 1.3.0 不识别此形式。"
                f"新建书目采用 {recommended}；不要据此重命名已有锚点或改写链接。",
            ))
            return
        if parsed[1] == number:
            return
        findings.append(Finding(
            "warning", "BIBLIOGRAPHY_NUMBER", first,
            f"书目显示序号 {number} 与已有锚点 ^{anchor} 的数字不同；保留 ID 和链接，"
            "按书目身份及原有映射核对，不自动重编号。",
        ))

    for index, line in enumerate(lines):
        if re.fullmatch(r" {0,3}(?:-{3,}|\*{3,}|_{3,})[ \t]*", line):
            finish()
            entry, entry_lines, bibliography_level = None, [], None
            continue
        heading = _HEADING.match(line)
        if heading:
            level, title = len(heading[1]), heading[2]
            finish()
            entry, entry_lines = None, []
            if level <= 2:
                supplement = bool(_SUPPLEMENT.fullmatch(title))
            if bibliography_level is not None and level <= bibliography_level:
                bibliography_level = None
            if _BIBLIOGRAPHY.fullmatch(title):
                bibliography_level = level
            continue
        if bibliography_level is None:
            continue
        numbered = _ENTRY.match(line)
        if numbered:
            finish()
            entry = (int(numbered[1]), index + 1, supplement)
            entry_lines = [line]
            continue
        if entry is not None:
            entry_lines.append(line)
    finish()


def validate_note(text: str) -> list[Finding]:
    """Return static findings; an empty list is not a whole-note acceptance."""
    lines = text.removeprefix("\ufeff").replace("\r\n", "\n").replace("\r", "\n").split("\n")
    findings: list[Finding] = []
    first = _frontmatter(lines, findings)
    visible = [""] * len(lines)
    links: list[tuple[str, int, str]] = []
    fence: tuple[str, int, str, int] | None = None
    fence_content: list[str] = []
    equation: int | None = None
    comment: str | None = None
    equation_closings: set[int] = set()
    table_closings: set[int] = set()
    code_lines: dict[int, str] = {}
    list_indent: int | None = None

    for index in range(first, len(lines)):
        raw = lines[index]
        if fence:
            marker, length, language, opening = fence
            if re.fullmatch(r" {0,3}" + re.escape(marker) + "{" + str(length) + r",}[ \t]*", raw):
                if language == "rich-table":
                    table_closings.add(index)
                    _rich_table(fence_content, opening + 1, findings, links)
                    _object_anchor(lines, index, "tbl-", findings, required=True)
                fence, fence_content = None, []
                continue
            fence_content.append(raw)
            continue
        if equation is not None:
            if raw.strip() == "$$":
                equation_closings.add(index)
                _object_anchor(lines, index, "eq-", findings, required=False)
                equation = None
            continue
        indentation = len(raw.expandtabs(4)) - len(raw.expandtabs(4).lstrip(" "))
        continuation = list_indent is not None and indentation >= list_indent
        if raw.strip() and not continuation:
            list_indent = None
        if comment is None and indentation >= 4 and not (
            continuation and list_indent is not None and indentation < list_indent + 4
        ):
            # Independent code blocks must not open mathematics or comments.
            continue
        numbered = _ENTRY.match(raw) if comment is None else None
        if numbered:
            list_indent = numbered.end()
        fenced_raw = _FENCE.match(raw) if comment is None else None
        if fenced_raw and not (fenced_raw[1][0] == "`" and "`" in fenced_raw[2]):
            fence = (fenced_raw[1][0], len(fenced_raw[1]), fenced_raw[2].strip(), index)
            list_indent = None
            continue
        if index not in code_lines:
            end = index + 1
            if raw.strip() and not _HEADING.match(raw):
                while end < len(lines) and lines[end].strip() and not (
                    _FENCE.match(lines[end]) or _HEADING.match(lines[end]) or _ENTRY.match(lines[end])
                ):
                    end += 1
            paragraph = _mask_paragraph_code("\n".join(lines[index:end]), comment).split("\n")
            code_lines.update(enumerate(paragraph, start=index))
        code_safe = code_lines[index]
        if comment is None and re.fullmatch(r" {0,3}\$\$.+\$\$[ \t]*", code_safe):
            equation_closings.add(index)
            _object_anchor(lines, index, "eq-", findings, required=False)
            continue
        line, comment = _mask_comments(_mask_delimited(code_safe, "$"), comment)
        fenced = _FENCE.match(line)
        if fenced and not (fenced[1][0] == "`" and "`" in fenced[2]):
            fence = (fenced[1][0], len(fenced[1]), fenced[2].strip(), index)
            continue
        if line.strip() == "$$":
            equation = index
            continue
        visible[index] = line

    if fence:
        findings.append(Finding("error", "FENCE_UNCLOSED", fence[3] + 1, "代码围栏未闭合。"))
    if equation is not None:
        findings.append(Finding("error", "MATH_UNCLOSED", equation + 1, "显示公式未闭合。"))
    if comment:
        findings.append(Finding("warning", "COMMENT_UNCLOSED", len(lines), "注释未闭合；后续内容可能不可见。"))

    anchors: dict[str, int] = {}
    for index, line in enumerate(visible):
        match = _ANCHOR.search(line)
        if match:
            anchor = match[1]
            if anchor in anchors:
                findings.append(Finding("error", "DUPLICATE_ANCHOR", index + 1,
                                        f"块 ID ^{anchor} 重复，首次定义在第 {anchors[anchor]} 行。"))
            else:
                anchors[anchor] = index + 1
            if anchor.startswith("eq-") and index - 1 not in equation_closings:
                findings.append(Finding("error", "EQUATION_ANCHOR", index + 1,
                                        "公式锚点须顶格独占显示公式闭合行的下一行。"))
            elif anchor.startswith("eq-") and not _WHOLE_ANCHOR.fullmatch(line):
                findings.append(Finding("error", "EQUATION_ANCHOR", index + 1,
                                        "公式锚点须顶格独占一行。"))
            if anchor.startswith("tbl-") and (
                index - 1 not in table_closings or not _WHOLE_ANCHOR.fullmatch(line)
            ):
                findings.append(Finding("error", "TABLE_ANCHOR", index + 1,
                                        "表锚须顶格独占 Rich Table 闭合围栏的下一行。"))
        links.extend((target, location, "") for target, location in _links(line, index + 1))
    for target, line, context in links:
        if target not in anchors:
            prefix = f"{context}：" if context else ""
            findings.append(Finding("error", "MISSING_ANCHOR", line,
                                    f"{prefix}本篇链接目标 ^{target} 未定义。"))
    _bibliography(visible, findings)
    return sorted(findings, key=lambda item: (item.line, item.code, item.message))


def validate_file(path: Path) -> list[Finding]:
    """Read one UTF-8 Markdown file without changing it."""
    try:
        text = path.read_text(encoding="utf-8-sig")
    except (OSError, UnicodeError) as error:
        return [Finding("error", "FILE_READ", 1, f"无法读取文件：{error}")]
    return validate_note(text)


def main() -> int:
    if NOTE_PATH is None:
        print("请先在脚本顶部设置 NOTE_PATH，或导入 validate_note(text)。本脚本只读，不修改笔记。")
        return 2
    findings = validate_file(NOTE_PATH)
    for finding in findings:
        print(f"{finding.level.upper()} [{finding.code}] 第 {finding.line} 行：{finding.message}")
    print(f"静态检查完成：{len(findings)} 项发现。原文、翻译、Obsidian 缓存和界面仍须分别验收。")
    return int(any(finding.level == "error" for finding in findings))


if __name__ == "__main__":
    raise SystemExit(main())
