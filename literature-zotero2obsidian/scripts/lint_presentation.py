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
STANDALONE_IMAGE = re.compile(
    r" {0,3}(?:!\[\[(?P<wiki>[^\]\r\n]+)\]\]|!\[(?:\\.|[^\]\\\r\n])*\]"
    r"\((?P<markdown><[^>\r\n]*>|[^()\r\n]*)\))(?:\\)?[ \t]*"
)
IMAGE_SUFFIXES = frozenset({".avif", ".bmp", ".gif", ".jpeg", ".jpg", ".png", ".svg", ".webp"})
PERCENT = re.compile(r"\d+(?:\.\d+)?\s*[%％]")
NON_ASCII_DASH = re.compile(r"[\u2010-\u2015]")
SPACE = re.compile(r"[\u3400-\u9fff] +(?=[0-9A-Za-zⓜ])|[0-9A-Za-zⓜ] +(?=[\u3400-\u9fff])")
NUMBERED_HEADING_PREFIX = re.compile(r"^ {0,3}#{1,6} +[0-9]+(?:\.[0-9]+)* +")
HEADING = re.compile(r"^ {0,3}#{1,6} +\S")
PANEL_LABEL = re.compile(r"(?P<prefix>图)?\s*(?P<object>S?\d+)(?P<panel>[a-z])", re.IGNORECASE)
PANEL_JOIN = re.compile(r"(?:\s|[,，、和及与\-\u2010-\u2015]|以及)*")
MATH_NUMBER_RANGE = re.compile(r"(?<![0-9A-Za-z_.])\d+(?:\.\d+)?\s*-\s*\d+(?:\.\d+)?(?![0-9.])")
BARE_DEGREE_COMMAND = re.compile(r"(?<!\\)\^\s*(?:\{\s*)?circ\b")
QUANTITY = re.compile(
    r"(?<![0-9A-Za-z_.])(?P<number>\d+(?:\.\d+)?(?:\s*[-\u2010-\u2015]\s*\d+(?:\.\d+)?)?)"
    r"\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年|km|cm|mm|m|h|s)(?![A-Za-z])"
)
ENGLISH_DURATION_UNIT = r"(?i:seconds?|minutes?|hours?|days?|weeks?|months?|years?|decades?|century|centuries)"
ENGLISH_DURATION = re.compile(
    r"(?<![0-9A-Za-z_.])(?P<number>\d+(?:\.\d+)?(?:\s*[-\u2010-\u2015]\s*\d+(?:\.\d+)?)?)"
    r"\s*(?:[-\u2010-\u2015]\s*)?(?P<unit>" + ENGLISH_DURATION_UNIT + r")(?![A-Za-z])"
)
ADJACENT_ENGLISH_DURATION = re.compile(
    r"\s*(?:[-\u2010-\u2015]\s*)?(?P<unit>" + ENGLISH_DURATION_UNIT + r")(?![A-Za-z])"
)
LETTER_LIST_ITEM = re.compile(
    r"^ {0,3}(?:\*\*|__)?(?:\((?P<parenthesized>[a-z])\)|(?P<plain>[a-z])[).、])(?:\*\*|__)?\s+\S",
    re.IGNORECASE,
)
FIGURE_CAPTION_NO_SPACE = re.compile(
    r"^ {0,3}\*\*图(?P<number>[A-Za-z0-9](?:\\.|[^\r\n：*])*)：\*\*(?P<body>\S)"
)
REFERENCE_DEFINITION = re.compile(r"^ {0,3}\[(?P<label>(?:\\.|[^\]\\\r\n])+)\]:[ \t]*(?P<payload>.*)$")
CHINESE_QUANTITY = re.compile(
    r"(?P<number>[数几]?[零〇一二三四五六七八九十百千万亿两半]+(?:[-\u2010-\u2015至到][数几]?[零〇一二三四五六七八九十百千万亿两半]+)?)"
    r"\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年)"
)
CENTURY_DURATION = re.compile(
    r"(?<![第0-9零〇一二三四五六七八九十百千万亿两半数几])"
    r"(?P<number>(?:[数几]?(?:[零〇一二三四五六七八九十百千万亿两半]+|\d+)个(?:多|余)?|[数几]个?))"
    r"\s*(?P<unit>世纪)"
)
PROSE_NUMBER_RANGE = re.compile(
    r"(?<![0-9A-Za-z_.:])(?P<first>[+-]?\d+(?:\.\d+)?)\s*(?:至|到)\s*"
    r"(?P<last>[+-]?\d+(?:\.\d+)?)(?![0-9.:])"
)
CHINESE_YEAR_DIGITS = str.maketrans("零〇一二三四五六七八九", "00123456789")
ADJACENT_UNIT = re.compile(r"\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年|km|cm|mm|m|h|s)(?![A-Za-z])")
HEADER_LABEL = re.compile(
    r"(?<![A-Za-z])(?:Received|Accepted|Published(?:\s+online)?|Publication\s+date|DOI)\s*[:：]"
    r"|(?:收稿|接收|接受|录用|在线发表|发表|出版)(?:日期)?\s*[:：]", re.IGNORECASE
)
DATE_LABEL = re.compile(
    r"(?<![A-Za-z])(?:Received|Accepted|Published(?:\s+online)?|Publication\s+date)\s*[:：]"
    r"|(?:收稿|接收|接受|录用|在线发表|发表|出版)(?:日期)?\s*[:：]", re.IGNORECASE
)
DATE_PLACEHOLDER = re.compile(
    r"(?<![A-Za-z])(?:x{1,4}(?:[\s./-]+x{1,4}){1,2}|"
    r"(?:dd|yyyy)[\s./-]+(?:mm|month)[\s./-]+(?:yyyy|dd))(?![A-Za-z])", re.IGNORECASE
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
        parts.append(link.label if keep_labels and not link.image else "")
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
    findings.extend(
        Prompt("BODY-C183", line, "DEGREE_COMMAND_MISSING_BACKSLASH", match[0],
               r"核对来源角度；^circ 会被解析为字母幂，角度单位应使用 ^\circ 或 ^{\circ}。此提示不自动改写来源。")
        for match in BARE_DEGREE_COMMAND.finditer(text)
    )
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
    comment = None
    visible_lines = []
    for line in value.splitlines():
        line, comment = note_checks._mask_comments(line, comment)
        visible_lines.append(line)
    value = "\n".join(visible_lines)
    value = _replace_links(value, keep_labels=True)
    # An embed's target/size is not its visible caption. A regular wiki link's
    # label is visible and remains eligible for a presentation prompt.
    value = WIKI.sub(lambda match: "" if match.start() and value[match.start() - 1] == "!"
                    and not note_checks._escaped(value, match.start() - 1)
                    else match[1].split("|", 1)[-1], value)
    value = re.sub(r"<[^>]*>", "", value)
    return value


def _reference_definition_end(lines: list[str], start: int) -> int | None:
    """Recognize a hidden link definition without hiding malformed prose.

    This is deliberately limited to a single-line label. Destinations and a
    quoted title may continue on following nonblank lines. Footnote definitions
    are visible content and are not treated as link definitions.
    """
    match = REFERENCE_DEFINITION.match(lines[start])
    if match is None or not match["label"].strip() or match["label"].startswith("^"):
        return None
    end = start
    payload = match["payload"].lstrip()
    if not payload and end + 1 < len(lines):
        end += 1
        payload = lines[end].lstrip()
    destination = re.match(r"<(?:\\.|[^\\<>\r\n])*>|(?:\\.|[^\s])+", payload)
    if destination is None:
        return None
    target = destination[0]
    synthetic = "[reference](" + target + ")"
    links = delivery_checks.scan_markdown_links(synthetic)
    if len(links) != 1 or links[0].end != len(synthetic):
        return None
    tail = payload[destination.end():].lstrip()
    same_line_title = bool(tail)
    destination_end = end
    if not tail and end + 1 < len(lines):
        tail = lines[end + 1].lstrip()
        end += 1
    if not tail or tail[0] not in {"'", '"', "("}:
        return None if same_line_title else destination_end
    closing = ")" if tail[0] == "(" else tail[0]
    title_parts = []
    cursor = 1
    while True:
        for index in range(cursor, len(tail)):
            if tail[index] != closing or note_checks._escaped(tail, index):
                continue
            if tail[index + 1:].strip():
                return None if same_line_title else destination_end
            title_parts.append(tail[:index + 1])
            title = "\n".join(title_parts)
            synthetic = "[reference](" + target + " " + title + ")"
            links = delivery_checks.scan_markdown_links(synthetic)
            if len(links) == 1 and links[0].end == len(synthetic):
                return end
            return None if same_line_title else destination_end
        title_parts.append(tail)
        end += 1
        if end >= len(lines) or not lines[end].strip():
            return None if same_line_title else destination_end
        tail, cursor = lines[end], 0


def _calendar_year(number: str, unit: str) -> bool:
    if unit != "年":
        return False
    number = number.replace(r"\text{-}", "-").replace(r"\mathrm{-}", "-")
    years = re.split(r"\s*[-\u2010-\u2015至到]\s*", number.translate(CHINESE_YEAR_DIGITS).strip())
    return all(year.isdigit() and 1500 <= int(year) <= 2199 for year in years)


def _scan_split_quantity(text: str, line: int) -> list[Prompt]:
    findings = []
    for math in INLINE_MATH.finditer(text):
        unit = ADJACENT_UNIT.match(text, math.end()) or ADJACENT_ENGLISH_DURATION.match(text, math.end())
        if unit is None:
            continue
        number = math[0][1:-1]
        if _calendar_year(number, unit["unit"]):
            continue
        findings.append(Prompt("BODY-C169", line, "QUANTITY_SPLIT_MATH", text[math.start():unit.end()],
                               "数值已在数学环境内而单位仍在外；核对量的含义后将完整量值写入同一数学环境。"))
    return findings


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
    math_visible = _visible(text)
    for math in INLINE_MATH.finditer(math_visible):
        findings.extend(_scan_math(math[0][1:-1], line, display=False))
    findings.extend(_scan_split_quantity(math_visible, line))
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
    if re.match(r"^ {0,3}>\s", visible) and len(HEADER_LABEL.findall(visible)) > 1:
        findings.append(Prompt("HDR-C30", line, "HEADER_ITEMS_SAME_LINE", visible.strip(),
                               "题头元信息的一行包含多个带标签的项目；每个日期类别与 DOI 分别占一条引用行。"))
    if re.match(r"^ {0,3}>\s", visible) and DATE_LABEL.search(visible):
        for match in DATE_PLACEHOLDER.finditer(visible):
            findings.append(Prompt("HDR-C42", line, "HEADER_DATE_PLACEHOLDER", match[0],
                                   "日期仍是占位文本；放大实际源页的日期区域核对可见值。若可见来源确实只有占位，保存该区域与具体观察后记录非问题依据。"))
    for match in NON_ASCII_DASH.finditer(visible):
        findings.append(Prompt("BODY-C173", line, "VISIBLE_DASH", match[0], "核对该可见连接线是否应使用 ASCII -；不得修改链接目标。"))
    prose = INLINE_MATH.sub("ⓜ", visible)
    for match in PERCENT.finditer(prose):
        findings.append(Prompt("BODY-C184", line, "PERCENT_OUTSIDE_MATH", match[0], "百分量整体置于数学环境，百分号写作 \\%。"))
    for match in (*QUANTITY.finditer(prose), *ENGLISH_DURATION.finditer(prose),
                  *CHINESE_QUANTITY.finditer(prose), *CENTURY_DURATION.finditer(prose)):
        number, unit = match["number"], match["unit"]
        if _calendar_year(number, unit):
            continue  # Calendar years and ranges are dates, not durations.
        findings.append(Prompt("BODY-C169", line, "QUANTITY_OUTSIDE_MATH", match[0], "按完整原句核对这是量值而非日期/编号；适用时数值与单位放入同一数学环境，保留原有近似限定。"))
    for match in PROSE_NUMBER_RANGE.finditer(prose):
        before = prose[:match.start()].rstrip()
        if before.endswith(("第", "图", "表", "式", "节", "章", "页")):
            continue
        after = prose[match.end():].lstrip()
        if after.startswith("年") and _calendar_year(match["first"] + "-" + match["last"], "年"):
            continue
        findings.append(Prompt("BODY-C180", line, "NUMERIC_RANGE_OUTSIDE_MATH", match[0],
                               r"核对这是否为量值范围；量值范围整体放入数学环境，并以 \text{-} 保留范围关系。日期、UTC时刻和文献/对象编号按各自规则保留。"))
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


def _scan_table_titles(value: object, line: int) -> list[Prompt]:
    if not isinstance(value, dict):
        return []
    sheets = value.get("sheets") if value.get("version") == 3 else [value]
    if not isinstance(sheets, list):
        return []
    findings = []
    for index, sheet in enumerate(sheets, 1):
        if not isinstance(sheet, dict) or not isinstance(sheet.get("title"), str):
            continue
        location = f"第{index}张工作表"
        title = _visible(sheet["title"]).strip()
        if not title:
            continue
        plain_title = re.sub(r"(?<!\\)(?:\*\*|__)", "", title).lstrip()
        label = (re.match(r"(?P<kind>表)(?P<number>[^：:\r\n]+)(?=[:：])", plain_title)
                 or re.match(r"(?P<kind>图)(?P<number>[^\r\n]+?)(?=内表格)", plain_title)
                 or re.match(r"(?P<kind>表|图)(?P<number>[A-Za-z0-9](?:\\.|[A-Za-z0-9.()_ -])*)", plain_title))
        number = label["number"].rstrip() if label is not None else ""
        required_label = label["kind"] + number if label is not None else ""
        if label is not None and label["kind"] == "图" and plain_title[label.end():].startswith("内表格"):
            required_label += "内表格"
        strong = re.match(r"^(?P<marker>\*\*|__)(?P<label>.+?)(?P=marker)", title)
        if label is None:
            if strong is None:
                findings.append(Prompt("BODY-C18", line, "TABLE_TITLE_BOLD", title,
                                       location + "的 title 未见完整粗体题注标签；先对照来源确认标签范围。"))
            findings.append(Prompt("BODY-C106", line, "TABLE_TITLE_LABEL", title,
                                   location + "的 title 未识别表号或图内表格标记；先对照来源确定对象类型与原编号，不自动补造或重排编号。"))
            continue
        if label is not None and (strong is None or not strong["label"].startswith(required_label)):
            findings.append(Prompt("BODY-C18", line, "TABLE_TITLE_BOLD", title,
                                   location + "的 title 题注标签按模板使用闭合的粗体。"))
        if label is not None and label["kind"] == "表" and not re.match(
            r"^\*\*表" + re.escape(number) + r"：\*\*", title
        ):
            findings.append(Prompt("BODY-C106", line, "TABLE_TITLE_LABEL", title,
                                   location + "的独立表 title 使用 **表N：** 标签；N 保留实际表号。"))
        if label is not None and label["kind"] == "图" and not plain_title[label.end():].startswith("内表格"):
            findings.append(Prompt("BODY-C263", line, "EMBEDDED_TABLE_TITLE", title,
                                   location + "的图内数据表可编辑副本用“图N内表格”标明来源；N 取原图编号。"))
    return findings


def _letter_list_structure(events: list[tuple[str, int, str]]) -> list[Prompt]:
    """Prompt on adjacent alphabetic labels that are ordinary Markdown text.

    This does not infer the source's block semantics. Genuine panel prose can be
    explicitly reviewed as a non-issue; code and real Markdown list markers are
    excluded from this heuristic.
    """
    findings = []
    group = []

    def finish():
        if len(group) >= 2:
            findings.append(Prompt("BODY-C104", group[0][0], "LETTER_LIST_NOT_MARKDOWN", group[0][1],
                                   "这些字母标签仍是普通段落；对照来源确认列表后使用 Markdown 列表标记，并保留原字母标签。硬换行不产生列表结构。"))
        group.clear()

    for kind, line, text in events:
        if kind == "blank":
            continue
        if kind != "text" or not LETTER_LIST_ITEM.match(_visible(text)):
            finish()
            continue
        group.append((line, text))
    finish()
    return findings


def _paragraph_spacing(events: list[tuple[str, int, str]]) -> list[Prompt]:
    findings = []
    start = 0
    while start < len(events):
        if events[start][0] != "blank":
            start += 1
            continue
        end = start + 1
        while end < len(events) and events[end][0] == "blank":
            end += 1
        if (end - start > 1 and start > 0 and end < len(events)
                and events[start - 1][0] == events[end][0] == "text"):
            findings.append(Prompt("BODY-C66", events[start][1], "PARAGRAPH_GAP_EXTRA", events[end][2][:80],
                                   "相邻可见正文段落之间存在连续空行；核对块边界后仅保留一空行。"))
        start = end
    return findings


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
    reference_end = -1
    for position in range(start, len(lines)):
        if position <= reference_end:
            continue
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
                findings.extend(_scan_table_titles(table, first_line))
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
        if comment is None and (not events or events[-1][0] != "text"):
            definition_end = _reference_definition_end(lines, position)
            if definition_end is not None:
                reference_end = definition_end
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
        caption = FIGURE_CAPTION_NO_SPACE.match(_visible(line))
        if caption:
            rule = "BODY-C96" if caption["number"].lower().startswith("s") else "BODY-C95"
            findings.append(Prompt(rule, position + 1, "FIGURE_CAPTION_SPACE", line,
                                   "粗体图号标签闭合后保留模板中的空格，再接题注正文。"))
        image_match = STANDALONE_IMAGE.fullmatch(line)
        # Wiki embeds can target PDFs, videos or Markdown blocks. Only an
        # actual image resource belongs to the standalone-figure rule.
        image_target = image_match["wiki"].split("|", 1)[0].split("#", 1)[0] if image_match and image_match["wiki"] else ""
        is_image = image_match and (image_match["markdown"] is not None
                                   or Path(image_target).suffix.lower() in IMAGE_SUFFIXES)
        if is_image and not raw.endswith("  "):
            findings.append(Prompt("BODY-C92", position + 1, "IMAGE_HARD_BREAK", line.strip(),
                                   "图片独占行的末尾使用两个空格硬换行；所属题注紧接下一行。"))
        if re.fullmatch(r" {0,3}##\s+Zotero原条目(?:\s+#+)?\s*", _visible(line)) and last_visible != ("text", "---"):
            findings.append(Prompt("LINK-C120", position + 1, "ZOTERO_SEPARATOR_MISSING", line,
                                   "文献内容与末尾 Zotero 原条目标题之间应有独立的 --- 分隔线。"))
        findings.extend(_scan(line, position + 1))
        last_visible = (kind, line.strip())
    findings.extend(_heading_spacing(events))
    findings.extend(_paragraph_spacing(events))
    findings.extend(_letter_list_structure(events))
    return list(dict.fromkeys(findings))


def lint_file(path: Path) -> list[Prompt]:
    return lint_presentation(path.read_text(encoding="utf-8-sig"))


def main() -> None:
    findings = lint_file(NOTE_PATH)
    print(json.dumps({"prompts": [asdict(item) for item in findings], "source_fidelity": "not_verified"}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
