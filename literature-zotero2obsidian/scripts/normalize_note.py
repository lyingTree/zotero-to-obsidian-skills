"""Conservative Markdown normalization. Offsets are Unicode code points.

Set the two paths to write a new UTF-8 candidate from an editor. Imported
functions never read source attachments or adjudicate translation fidelity.
"""
from __future__ import annotations

from bisect import bisect_right
from hashlib import sha256
import importlib.util
from pathlib import Path
import re
import sys

INPUT_PATH: Path | None = None
OUTPUT_PATH: Path | None = None

# Use the repository's image/heading, reference-definition and DOI grammars.
_script = Path(__file__).resolve().parent.joinpath("lint_presentation.py")
_spec = importlib.util.spec_from_file_location("literature_normalizer_presentation", _script)
presentation = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = presentation
_spec.loader.exec_module(presentation)

CJK = r"\u3400-\u9fff"
UNITS = (r"小时|分钟|秒钟|千米|公里|厘米|毫米|米|秒|节|年|"
         r"milliseconds?|seconds?|minutes?|hours?|days?|weeks?|months?|years?|"
         r"min|km|cm|mm|ms|kg|mg|kPa|Pa|Hz|m|h|s|g|[%％]")
NUMBER = r"[+−-]?(?:[0-9]+(?:\.[0-9]+)?|\.[0-9]+)"
LEFT = r"(?<![A-Za-z0-9_.:/+−-])"
RIGHT = r"(?![A-Za-z0-9_])"
QUANTITY = re.compile(LEFT + rf"(?P<n>{NUMBER})[ \t]*(?P<u>{UNITS})" + RIGHT)
SPLIT = re.compile(LEFT + r"(?P<a>[+−-]?[0-9]+\.)\$(?P<b>[0-9]+)\\?[%％]\$")
WRAPPED = re.compile(r"\$(?P<n>" + NUMBER + r")\$[ \t]*(?P<u>" + UNITS + ")" + RIGHT)
PREFIX = re.compile(r"(?:第|图|表|公式|方程|章节|文献|Fig\.?|Figure|Table|Eq\.?)\s*$", re.I)
COMPOUND = re.compile(r"(?:[/^²³·]|\s*[×±]|每|/)")
RANGE_LEFT = re.compile(r"[0-9]\s*(?:[-−–—~～至到×±]|[eE][+-]?)\s*$")
TOKEN = re.compile(
    r"(?m)^ {0,3}#{1,6}[ \t]+[^\r\n]*|^ {0,3}>[^\r\n]*|^ {0,3}(?:`{3,}|~{3,})[^\r\n]*|^(?: {4}|\t)[^\r\n]*|"
    r"^ {0,3}\[(?:\\.|[^\]\\\r\n])+\]:[^\r\n]*|"
    r"\\\\[^\s<>]+|<!--|%%|\\[\s\S]|\x60+|\$\$|\$|!?\[|<|"
    r"(?i:(?<![A-Za-z0-9+.-])(?:[a-z][a-z0-9+.-]*://|www\.)[^\s<>]+)|"
    r"(?<![\w.])10\.\d{4,9}/[^\s<>]+|"
    r"(?:[A-Za-z]:[\\/]|(?:\.{0,2}/|~/)|(?<![\w.-])[\w.\-]+[/\\])[^\s<>]*|"
    r"(?i:(?<![\w.—-])[\w.—-]+\.(?:png|jpe?g|svg|gif|webp|pdf|md|csv|docx?|xlsx?|py|txt)\b)|"
    r"\^[\w-]+|(?<![\w.+-])[\w.+-]+@[\w.-]+\.[A-Za-z]+"
)
HTML_CODE_OPEN = re.compile(r"<(code|pre)(?=[\s>])", re.I)
NEXT_UNIT = re.compile(r"[ \t]+(?:min|[kcm]?m|[kmg]?g|s|h|Hz|Pa)(?=[⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻^/·\s.,，。；;]|$)")
QUOTE_LINE = re.compile(r"^ {0,3}>")
EOL = re.compile(r"\r\n|\r|\n")
SPACE = re.compile(rf"(?<=[{CJK}]) +(?=[0-9A-Za-z$])|(?<=[0-9A-Za-z$]) +(?=[{CJK}])")
AMBIG_UNIT = re.compile(r"(?<![\w.])\d+(?:\.\d+)?[ \t]+[A-Za-zµμ°]+\b")
AUTHOR = re.compile(r"^(?:\*\*)?(?:作者|Authors?)\s*[:：]|\(mailto:|\$\{\}\^", re.I)
TITLE = re.compile(r"^(?:\*\*)?(?:中译名|中文译名|中文题名|译名|中文标题|译题)\s*[:：]")
ANCHOR = re.compile(r"\^(?P<kind>fig|eq|tbl|tab)[-\w]*", re.I)
FENCE = presentation.FENCE


def _balanced(text, pos, opening, closing):
    depth = 0
    for index in range(pos, len(text)):
        char = text[index]
        if char not in (opening, closing) or presentation.note_checks._escaped(text, index):
            continue
        depth += 1 if char == opening else -1
        if depth == 0:
            return index + 1
    return None


def _pass(text):
    lines = text.splitlines(keepends=True)
    starts = [0]
    for line in lines:
        starts.append(starts[-1] + len(line))
    spans, proposals, issues, labels = [], [], [], []
    links = {link.start: link for link in presentation.delivery_checks.scan_markdown_links(text)}
    logical_lines = [s.rstrip("\r\n") for s in lines]

    def issue(code, at, message, severity="review"):
        issues.append(dict(code=code, line=bisect_right(starts, at), start=at, message=message, severity=severity))

    def protect(start, end, kind):
        spans.append((start, end, kind))
        return end

    pos = 0
    if lines and lines[0].lstrip("\ufeff").rstrip("\r\n") == "---":
        end = next((starts[i+1] for i in range(1, len(lines))
                    if lines[i].rstrip("\r\n") in ("---", "...")), None)
        if end is None:
            issue("UNCLOSED_YAML", 0, "前置 YAML 缺少结束标记。")
        pos = protect(0, end or len(text), "yaml")

    while match := TOKEN.search(text, pos):
        a, b = match.span()
        token = match[0]
        line = bisect_right(starts, a) - 1
        kind, end = "literal", b
        fence = FENCE.fullmatch(token) if a == starts[line] else None
        if a == starts[line] and QUOTE_LINE.match(token):
            # A lazy continuation is part of a quoted paragraph even without
            # another '>'. Preserve through the next blank line if unsure.
            last = line
            while last + 1 < len(lines) and logical_lines[last + 1].strip():
                last += 1
            end = starts[last] + len(logical_lines[last])
            kind = "quote"
        elif fence:
            mark, info = fence.groups()
            closer = re.compile(r" {0,3}" + mark[0] + "{"+str(len(mark))+",}[ \t]*")
            end = next((starts[i] + len(lines[i].rstrip("\r\n")) for i in range(line+1,len(lines))
                        if closer.fullmatch(lines[i].rstrip("\r\n"))), None)
            kind = "table" if "rich-table" in info.lower() else "fence"
            if kind == "table":
                issue("RICH_TABLE_PROTECTED", a, "表格按原字节边界保留，结构与字段由表格验收器核对。", "info")
        elif a == starts[line] and presentation.REFERENCE_DEFINITION.match(token):
            last = presentation._reference_definition_end(logical_lines, line)
            end = starts[min(len(lines), (last if last is not None else line) + 1)]
        elif token.startswith("`"):
            close = re.compile(r"(?<!`)`{" + str(len(token)) + r"}(?!`)").search(text,b)
            end = close.end() if close else None
            kind = "code"
        elif token in (r"\(", r"\["):
            closer = r"\)" if token == r"\(" else r"\]"
            closing = text.find(closer,b)
            end = closing+2 if closing >= 0 else None
            kind = "math" if token == r"\(" else "display"
        elif token in ("<!--", "%%", "$$", "$"):
            if token == "$":
                split = re.search(r"[+−-]?[0-9]+\.$", text[max(0,a-128):a])
                repair = SPLIT.match(text, max(0,a-128)+split.start()) if split else None
                if repair and repair.start() >= pos and PREFIX.search(text[max(0, repair.start()-36):repair.start()]):
                    issue("AMBIGUOUS_SPLIT_DECIMAL", a, "编号后紧接百分数，保留原文边界。")
                    trailing = re.compile(r"[ \t]*").match(text, repair.end())
                    pos = protect(repair.start(), trailing.end(), "math")
                    continue
                if repair and repair.start() >= pos:
                    proposals.append((repair.start(),repair.end(),"split_percent", "$"+repair["a"]+repair["b"]+r"\%$"))
                    pos = protect(repair.start(), repair.end(), "math")
                    continue
                if split:
                    issue("AMBIGUOUS_SPLIT_DECIMAL", a, "小数点紧接数学边界，无法唯一确定数值或百分号归属。")
            delimiter = "-->" if token == "<!--" else token
            end = text.find(delimiter, b)
            while end >= 0 and presentation.note_checks._escaped(text,end):
                end = text.find(delimiter, end+len(delimiter))
            end = end+len(delimiter) if end >= 0 else None
            kind = {"$":"math","$$":"display"}.get(token,"comment")
            wrapped = WRAPPED.match(text,a) if token == "$" else None
            if wrapped and _quantity_allowed(text,wrapped):
                end = wrapped.end()
                proposals.append((a,end,"split_quantity",_math(wrapped)))
        elif token.endswith("["):
            opening = b-1
            end = _balanced(text, opening, "[", "]")
            if end and text.startswith("(", end):
                label_end = end - 1
                link = links.get(a)
                end = link.end if link else None
                if end and token == "[":
                    labels.append((b, label_end))
            elif end and text.startswith("[", end):
                end = _balanced(text, end, "[", "]")
            elif end and token == "[" and text.startswith("[[",a):
                alias = text.find("|",b,end)
                if alias >= 0:
                    labels.append((alias+1,end-2))
            kind = "link"
        elif token == "<":
            tag = re.match(r"(?:/?[A-Za-z!]|https?://)",text[b:b+10])
            if tag:
                html = re.compile(r"""(?:[^>"']|"[^"]*"|'[^']*')*>""").match(text,b)
                end = html.end() if html else None
                code_tag = HTML_CODE_OPEN.match(text,a)
                if code_tag and end is not None:
                    closer = re.compile(r"</" + code_tag[1] + r"\s*>", re.I).search(text,end)
                    end = closer.end() if closer else None
                    kind = "html_code"
        if end is None:
            kind = "display" if token == "$$" else kind
            issue("UNCLOSED_"+kind.upper(), a, "未找到可确认的闭合边界，保留余下原文。")
            end = len(text)
        pos = protect(a,end,kind)

    def prose(a,b):
        for m in QUANTITY.finditer(text,a,b):
            if not _quantity_allowed(text,m):
                issue("AMBIGUOUS_QUANTITY", m.start(), "编号、日期、范围或复合单位需核对。")
                continue
            proposals.append((*m.span(),"quantity",_math(m)))
        for m in AMBIG_UNIT.finditer(text,a,b):
            if not QUANTITY.match(text,m.start()):
                issue("UNCLASSIFIED_UNIT",m.start(),"保留未识别的数值与拉丁文字组合。")
        for m in presentation.NON_ASCII_DASH.finditer(text,a,b):
            if re.search(r"\d\s*$",text[max(a,m.start()-8):m.start()]) and re.compile(r"\s*\d").match(text,m.end(),b):
                issue("AMBIGUOUS_RANGE",m.start(),"保留数值之间的横线，待确认是范围还是减法。")
                continue
            proposals.append((*m.span(),"dash","-"))
        for m in SPACE.finditer(text,max(0,a-1),min(len(text),b+1)):
            line_start = starts[bisect_right(starts,m.start())-1]
            if re.compile(r" {0,3}#{1,6} +[0-9]+(?:\.[0-9]+)*").fullmatch(text,line_start,m.start()):
                continue
            if a <= m.start() and m.end() <= b:
                proposals.append((*m.span(),"cjk_space",""))

    cursor = 0
    for a,b,kind in spans:
        if cursor < a:
            prose(cursor,a)
        cursor = b
    if cursor < len(text):
        prose(cursor,len(text))

    for a,b in labels:
        if not re.search(r"[\x60$\[\]<>\\/|]",text[a:b]) and not presentation.URL.search(text[a:b]) and not presentation.DOI.search(text[a:b]):
            prose(a,b)

    _layout(text, lines, starts, spans, proposals, issue)
    proposals.sort(key=lambda edit:(edit[0],-edit[1]))
    parts, changes, cursor, shift = [], [], 0, 0
    line, column = 1, 1
    for a,b,rule,after in proposals:
        if a < cursor or text[a:b] == after:
            continue
        untouched = text[cursor:a]
        line, column = _advance(line, column, untouched)
        parts.extend((untouched,after))
        changes.append(dict(start=a+shift,end=b+shift,before=text[a:b],after=after,
                            line=line,column=column,stage=_stage(rule),rule=rule))
        line,column = _advance(line,column,after)
        cursor = b
        shift += len(after)-(b-a)
    parts.append(text[cursor:])
    return "".join(parts), changes, issues


def _advance(line, column, fragment):
    last = None
    for last in EOL.finditer(fragment):
        line += 1
    if last is None:
        return line, column + len(fragment)
    return line, 1 + len(fragment) - last.end()


def _stage(rule):
    if rule in {"split_percent", "split_quantity", "quantity"}:
        return "math_formatting"
    if rule in {"block_spacing", "cjk_space", "image_hard_break"}:
        return "whitespace_formatting"
    return "punctuation_formatting"


def _quantity_allowed(text,m):
    before, after = text[max(0,m.start()-36):m.start()], text[m.end():m.end()+32]
    if (PREFIX.search(before) or RANGE_LEFT.search(before) or COMPOUND.match(after) or NEXT_UNIT.match(after)
            or before.rstrip().endswith(("×","±","·","/"))
            or m["u"] == "年" and after.startswith("级")
            or re.search(r"(?:UTC\s*|[0-9][,，]|[±×])$",before,re.I)
            or re.match(r"\s*UTC\b",after,re.I)):
        return False
    if m["u"] == "节" and re.match(r"(?:课|目|点|能|选|[、.．])",after):
        return False
    if m["u"] == "年" and len(m["n"]) == 4 and m["n"].isdigit():
        return not presentation._calendar_year(m["n"], m["u"])
    return True


def _math(m):
    number, unit = m["n"],m["u"]
    if unit in "%％":
        return "$"+number+r"\%$"
    command = r"\text" if re.search(rf"[{CJK}]",unit) else r"\mathrm"
    return "$"+number+r"\,"+command+"{"+unit+"}$"


def _layout(text, lines, starts, spans, proposals, issue):
    protected, roles = set(), {}
    for a,b,kind in spans:
        first,last = bisect_right(starts,a)-1,bisect_right(starts,b-1)-1
        if kind in ("yaml","table","fence","display") or first != last:
            protected.update(range(first,last+1))
            roles[first] = kind+"_start"
            roles[last] = kind+"_end"
            if first == last and kind == "display":
                whole = text[starts[first]:starts[first+1]].strip() == text[a:b]
                roles[first] = "display_whole" if whole else "protected"
    occupied = [i for i,line in enumerate(lines) if line.strip()]
    def role(i):
        if i in protected:
            return roles.get(i,"protected")
        s = lines[i].rstrip("\r\n")
        if s.startswith("# "):
            return "h1"
        if presentation.HEADING.match(s):
            return "heading"
        if presentation._is_standalone_image(s):
            return "image"
        if ANCHOR.fullmatch(s):
            return "anchor_"+ANCHOR.fullmatch(s)["kind"].lower()
        if re.match(r"^\*\*(?:图|Fig(?:ure)?\.?)\s*[\w.-]*[：:.]",s,re.I):
            return "caption"
        if s.startswith((" ","\t",">","|")):
            return "other"
        return "text"
    for j,(i,k) in enumerate(zip(occupied,occupied[1:])):
        left,right = role(i),role(k)
        if (i in protected and not left.endswith(("_end","_whole"))
                or k in protected and not right.endswith(("_start","_whole"))):
            continue
        if (right == "text" and left == "image" and j+2 < len(occupied)
                and role(occupied[j+2]) == "anchor_fig"
                and presentation.formatting.starts_with_bold_source_label(lines[k].strip())):
            right = "caption"
        if (left == "text" and right == "anchor_fig" and j > 0
                and role(occupied[j-1]) == "image"
                and presentation.formatting.starts_with_bold_source_label(lines[i].strip())):
            left = "caption"
        if left == "image" and right in {"image", "caption"}:
            raw = lines[i].rstrip("\r\n")
            stripped = raw.rstrip(" \t")
            if not stripped.endswith("\\") and raw != stripped + "  ":
                proposals.append((starts[i]+len(stripped), starts[i]+len(raw),
                                  "image_hard_break", "  "))
        blank = None
        if right in ("heading","h1") or left.startswith("anchor_"):
            blank = True
        elif left == "h1" and right=="text":
            if TITLE.search(lines[k]):
                blank = False
            elif AUTHOR.search(lines[k]):
                blank = True
            elif j+2<len(occupied) and AUTHOR.search(lines[occupied[j+2]]) and re.search(rf"[{CJK}]",lines[k]):
                blank = False
            else:
                issue("AMBIGUOUS_HEADER",starts[k],"题名之后行的角色不明，保留间距。")
        elif left=="text" and right=="text" and j and role(occupied[j-1])=="h1" and AUTHOR.search(lines[k]):
            blank = True
        elif left=="image" and right in ("image","caption") or (left,right) in {
            ("caption","anchor_fig"),("display_end","anchor_eq"),("display_whole","anchor_eq"),("table_end","anchor_tab"),("table_end","anchor_tbl")
        }:
            blank = False
        elif right in ("image","display_start","display_whole") or left in ("display_end","display_whole"):
            blank = True
        elif left=="heading" and right=="text":
            blank = False
        if blank is None:
            continue
        eol = EOL.search(lines[i])
        if eol:
            proposals.append((starts[i+1],starts[k],"block_spacing",eol[0] if blank else ""))


def _stable_pass(text):
    candidate, changes, issues = _pass(text)
    for _ in range(3):
        verified, more, _ = _pass(candidate)
        if verified == candidate:
            break
        changes.extend(more)
        candidate = verified
    else:
        # The same input always takes the same bounded path and returns itself.
        candidate, changes = text, []
        issues.append(dict(code="INTERACTING_RULES",line=1,start=0,message="规则互相影响，保留整份输入。"))
    return candidate, changes, issues


def normalize_note(text: str) -> dict:
    candidate, changes, issues = _stable_pass(text)
    verified, _, _ = _stable_pass(candidate)
    def digest(value):
        return sha256(value.encode("utf-8")).hexdigest()
    return dict(schema_version=1,engine="normalize_note",text=candidate,changes=changes,
                unresolved=issues,input_sha256=digest(text),output_sha256=digest(candidate),
                idempotent=(candidate == verified),source_fidelity="not_assessed")


def normalize_file(source: Path, destination: Path) -> dict:
    if source.resolve() == destination.resolve():
        raise ValueError("Use a new candidate path.")
    result = normalize_note(source.read_bytes().decode("utf-8"))
    with destination.open("xb") as handle:
        handle.write(result["text"].encode("utf-8"))
    return result


if __name__ == "__main__" and INPUT_PATH is not None and OUTPUT_PATH is not None:
    normalize_file(INPUT_PATH, OUTPUT_PATH)
