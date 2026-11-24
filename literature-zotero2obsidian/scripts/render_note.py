"""Render a reviewed, ordered unit plan without writing files.

Public functions accept JSON-shaped Python dictionaries. Translation fields are
literal single-line text. Only explicitly registered {{link:name}} and
{{literal:name}} placeholders in paragraph, figure caption and translator note
fields have special meaning; each registration denotes one occurrence. Table
data is the complete upstream-reviewed display workbook, including captions.
Protected inline literals can carry source-verified inline Markdown/math.
"""
from __future__ import annotations

from collections import Counter
from copy import deepcopy
from hashlib import sha256
import json
import importlib.util
from pathlib import Path
import re
from urllib.parse import quote

import yaml

_HELPERS_PATH = Path(__file__).resolve().with_name("format_helpers.py")
_SPEC = importlib.util.spec_from_file_location(
    "_note_format_helpers_" + sha256(str(_HELPERS_PATH).encode()).hexdigest()[:12], _HELPERS_PATH)
_helpers = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_helpers)
build_figure_block = _helpers.build_figure_block
build_rich_table_block = _helpers.build_rich_table_block


FIELDS = {
    "header": ("original_title authors affiliations translated", "notes"),
    "heading": ("level", "numbered"),
    "list": ("style count", ""),
    "metadata": ("notes", ""),
    "paragraph": ("", "links literals"),
    "translator_note": ("", "links literals"),
    "figure": ("images label anchor", "links literals"),
    "formula": ("latex anchor", ""),
    "table": ("data anchor", ""),
    "reference": ("source_text", "anchor"),
    "bibliography": ("source_text", "anchor"),
    "zotero_link": ("uri", ""),
}
IDENT = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*\Z")
TOKEN = re.compile(r"\{\{(link|literal):([A-Za-z0-9][A-Za-z0-9_.-]*)\}\}")
REFERENCES = {"reference", "bibliography"}


class Invalid(ValueError):
    def __init__(self, path, message):
        self.path = path
        super().__init__(message)


def keys(value, path, required="", optional=""):
    if not isinstance(value, dict):
        raise Invalid(path, "expected an object")
    missing = set(required.split()) - value.keys()
    if missing:
        raise Invalid(path + "/" + min(missing), "required field is missing")
    extra = value.keys() - set((required + " " + optional).split())
    if extra:
        raise Invalid(path + "/" + min(extra), "unexpected or protected field")


def line(value, path):
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        raise Invalid(path, "expected nonempty text without outer whitespace")
    if any(ord(c) < 32 or c in "\x7f\x85\u2028\u2029" for c in value):
        raise Invalid(path, "expected one line without controls")
    return value


def digest(value):
    return sha256(value.encode("utf-8")).hexdigest()


def canonical(value, path):
    try:
        encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)
        if json.loads(encoded) != value:
            raise ValueError("requires string keys and JSON arrays")
        return digest(encoded)
    except (TypeError, ValueError, RecursionError) as error:
        raise Invalid(path, f"expected finite UTF-8 JSON data: {error}") from error


def literal(value):
    # Escape only the plain-text channel. Never run this over emitted links,
    # LaTeX, table data or the final Markdown document.
    value = re.sub(r"([\\`*_\[\]<>$~&#^|=])", r"\\\1", value)
    value = re.sub(r"%{2,}", lambda m: r"\%" * len(m[0]), value)
    return re.sub(r"^(?:(\d+)([.)])(?=\s|$)|([-+!])(?=\s|$)|(-)(?=(?:\s*-){2,}\s*$))", lambda m:
                  (m[1] or "") + "\\" + (m[2] or m[3] or m[4]), value)


def template_fields(unit):
    kind = unit["kind"]
    if kind == "header" and unit["translated"]:
        return {"translated_title": ""}
    if kind in {"heading", "paragraph", "translator_note"}:
        return {"text": ""}
    if kind == "list":
        return {"items": [""] * unit["count"]}
    if kind == "figure":
        return {"caption": ""}
    return {}


def _plan(plan):
    keys(plan, "/plan", "schema_version units", "properties")
    if type(plan["schema_version"]) is not int or plan["schema_version"] != 1:
        raise Invalid("/plan/schema_version", "expected integer 1")
    if "properties" in plan and not isinstance(plan["properties"], dict):
        raise Invalid("/plan/properties", "expected a YAML property mapping")
    units = plan["units"]
    if not isinstance(units, list) or not units:
        raise Invalid("/plan/units", "expected a nonempty ordered list")
    seen = set()
    supplement = False
    for i, unit in enumerate(units):
        path = f"/plan/units/{i}"
        if not isinstance(unit, dict) or not isinstance(unit.get("kind"), str) or unit["kind"] not in FIELDS:
            raise Invalid(path + "/kind", "unknown or missing unit kind")
        kind = unit["kind"]
        required, optional = FIELDS[kind]
        keys(unit, path, "unit_id kind scope source " + required, optional)
        uid = line(unit["unit_id"], path + "/unit_id")
        if not IDENT.fullmatch(uid) or uid in seen:
            raise Invalid(path + "/unit_id", "invalid or duplicate unit ID")
        seen.add(uid)
        keys(unit["source"], path + "/source", "source_id locator")
        line(unit["source"]["source_id"], path + "/source/source_id")
        locator = unit["source"]["locator"]
        if not (isinstance(locator, str) and locator.strip() or isinstance(locator, dict) and locator):
            raise Invalid(path + "/source/locator", "expected a nonempty locator")
        scope = line(unit["scope"], path + "/scope")
        if not re.fullmatch(r"main|supplement:[A-Za-z0-9][A-Za-z0-9_.-]*", scope):
            raise Invalid(path + "/scope", "invalid source scope")
        if kind == "header" and (i != 0 or scope != "main"):
            raise Invalid(path, "header must be the first main unit")
        if kind == "zotero_link" and (i != len(units) - 1 or scope != "main"):
            raise Invalid(path, "Zotero link must be the final main-scope footer")
        if supplement and scope == "main" and kind != "zotero_link":
            raise Invalid(path + "/scope", "main content must precede supplements")
        supplement |= scope != "main"
        if kind == "list":
            if type(unit["count"]) is not int or unit["count"] < 1:
                raise Invalid(path + "/count", "expected a positive source item count")
            if unit["style"] not in ("ordered", "unordered"):
                raise Invalid(path + "/style", "expected ordered or unordered")
        if "notes" in unit:
            notes = unit["notes"]
            if not isinstance(notes, list) or kind == "metadata" and not notes:
                raise Invalid(path + "/notes", "expected a list of verified metadata lines")
            for j, note in enumerate(notes):
                line(note, f"{path}/notes/{j}")
        if kind == "header" and type(unit["translated"]) is not bool:
            raise Invalid(path + "/translated", "expected a boolean")
    return units


def content_template(plan):
    """Return editable translation slots keyed by unit ID; blank figure caption is optional."""
    canonical(plan, "/plan")
    return {unit["unit_id"]: template_fields(unit) for unit in _plan(plan)}


def _header_item(item, field, path):
    """A literal name, or an explicitly annotated source name."""
    if isinstance(item, str):
        return literal(line(item, path))
    if field == "affiliations":
        keys(item, path, "name number")
        number = line(item["number"], path + "/number")
        if not number.isascii() or not number.isalnum():
            raise Invalid(path + "/number", "expected the source affiliation marker")
        return "${}^{" + number + "}$" + literal(line(item["name"], path + "/name"))
    keys(item, path, "name", "numbers corresponding email")
    name = literal(line(item["name"], path + "/name"))
    markers = item.get("numbers", [])
    if not isinstance(markers, list) or any(not isinstance(n, str) or not n.isascii() or not n.isalnum() for n in markers):
        raise Invalid(path + "/numbers", "expected a list of source affiliation markers")
    if type(item.get("corresponding", False)) is not bool:
        raise Invalid(path + "/corresponding", "expected a boolean")
    if item.get("corresponding"):
        markers = [*markers, r"\ast"]
    if "email" in item:
        email = line(item["email"], path + "/email")
        if not re.fullmatch(r"[^\s<>@]+@[^\s<>@]+", email):
            raise Invalid(path + "/email", "invalid email")
        name = f"[{name}](mailto:{quote(email, safe='@.+-_')})"
    return name + ("${}^{" + ",".join(markers) + "}$" if markers else "")


def _target_fragment(name):
    # Angle destinations permit spaces. Escape literal URL-fragment and
    # Markdown delimiters, while preserving readable Unicode heading text.
    return re.sub(r"[%<>#\\&]", lambda m: quote(m[0], safe=""), name)


def _citation(spec, path, targets, numbers):
    """Render only explicitly named endpoints, preserving the source grouping."""
    keys(spec, path, "citation")
    citation = spec["citation"]
    if not isinstance(citation, list) or not citation or len(citation) % 2 == 0:
        raise Invalid(path, "expected reference IDs alternating with separators")
    parts = []
    for i, part in enumerate(citation):
        if i % 2:
            if part not in (",", "-"):
                raise Invalid(path, "expected ',' or '-' between reference IDs")
            parts.append(part)
            continue
        if not isinstance(part, str) or part not in numbers:
            raise Invalid(path, "citation must resolve to a bibliography unit")
        parts.append(f"[{numbers[part]}](<#{targets[part]}>)")
    return r"<sup>\[" + "".join(parts) + r"\]</sup>"


def _inline(unit, value, path, targets, numbers, ambiguous):
    links, literals = unit.get("links", {}), unit.get("literals", {})
    rules = {}
    for typ, entries in (("link", links), ("literal", literals)):
        if not isinstance(entries, dict):
            raise Invalid(path, f"{typ} registrations must be an object")
        for name, spec in entries.items():
            if not IDENT.fullmatch(name):
                raise Invalid(path, "invalid placeholder name")
            if typ == "literal":
                raw = line(spec, path)
                if re.match(r"^(?:#{1,6}(?:\s|$)|>|[-+*]\s|\d+[.)]\s|[-*_]{3,}\s*$|\^[-A-Za-z0-9]+$|`{3,}|~{3,}|\$\$|<[/!A-Za-z])", raw):
                    raise Invalid(path, "protected inline text contains a block delimiter")
                rules[typ, name] = raw
                continue
            if isinstance(spec, dict) and "citation" in spec:
                rules[typ, name] = _citation(spec, path, targets, numbers)
                continue
            keys(spec, path, "target label")
            target = line(spec["target"], path)
            if target not in targets:
                raise Invalid(path, f"unresolved link target {target}")
            if targets[target] in ambiguous:
                raise Invalid(path, f"link target resolves to multiple headings: {target}")
            label = literal(line(spec["label"], path))
            rules[typ, name] = f"[{label}](<#{targets[target]}>)"
    parts, used, cursor = [], set(), 0
    for m in TOKEN.finditer(value):
        key = m.group(1, 2)
        if key not in rules or key in used:
            raise Invalid(path, f"unregistered or repeated placeholder {m[0]}")
        used.add(key)
        parts.extend((literal(value[cursor:m.start()]), rules[key]))
        cursor = m.end()
    if rules.keys() - used:
        raise Invalid(path, f"missing registered placeholder(s): {sorted(rules.keys() - used)}")
    return "".join(parts) + literal(value[cursor:])


def render_note(plan: dict, content: dict) -> dict:
    """Return text, exact line/character mappings and hashes, or a located finding."""
    report = dict(schema_version=1, status="blocked", text=None, mapping=[], findings=[],
                  plan_sha256=None, content_sha256=None, output_sha256=None, source_fidelity="not_assessed")
    try:
        report["plan_sha256"] = canonical(plan, "/plan")
        report["content_sha256"] = canonical(content, "/content")
        units = _plan(plan)
        expected = {u["unit_id"] for u in units}
        if not isinstance(content, dict):
            raise Invalid("/content", "expected a mapping of unit IDs")
        for ids, message in ((expected - content.keys(), "missing unit"), (content.keys() - expected, "unexpected unit")):
            if ids:
                raise Invalid("/content/" + min(ids), message)
        text, mapping = _render(plan, units, content)
        report.update(status="rendered", text=text, mapping=mapping, output_sha256=digest(text))
    except Invalid as error:
        report["findings"].append(dict(code="RENDER_INVALID", field=error.path, message=str(error)))
    return report


def _render(plan, units, content):
    targets, headings, numbers = {}, {}, {}
    counts = Counter()
    levels = []
    occupied = set()
    multi_si = len({u["scope"] for u in units if u["scope"] != "main"}) > 1
    for i, u in enumerate(units):
        uid, kind, scope = u["unit_id"], u["kind"], u["scope"]
        path, cp = f"/plan/units/{i}", "/content/" + uid
        slots = {"items": None} if kind == "list" else template_fields(u)
        keys(content[uid], cp, "" if kind == "figure" else " ".join(slots), "caption" if kind == "figure" else "")
        for field, value in content[uid].items():
            if kind == "list":
                if not isinstance(value, list) or len(value) != u["count"]:
                    raise Invalid(cp + "/items", "supply exactly one translation per source list item")
                for j, item in enumerate(value):
                    line(item, f"{cp}/items/{j}")
                continue
            if kind != "figure" or value != "":
                line(value, cp + "/" + field)
        if kind == "heading":
            level, numbered = u["level"], u.get("numbered", True)
            if type(level) is not int or not 2 <= level <= 6 or type(numbered) is not bool:
                raise Invalid(path, "heading requires level 2–6 and boolean numbered")
            name = content[uid]["text"]
            if numbered:
                depth = level - 1
                if depth > len(levels) + 1:
                    raise Invalid(path + "/level", "numbered heading skips a parent")
                levels = levels[:depth]
                if len(levels) < depth:
                    levels.append(0)
                levels[-1] += 1
                name = ".".join(map(str, levels)) + " " + name
            headings[uid] = "#" * level + " " + literal(name)
            targets[uid] = _target_fragment(name)
            continue
        anchor = u.get("anchor")
        if kind in REFERENCES:
            counts[scope] += 1
            numbers[uid] = counts[scope]
            prefix = "ref" if scope == "main" else ("sref-src" + scope.split(":",1)[1] if multi_si else "sref")
            anchor = u.get("anchor", f"{prefix}-{counts[scope]}")
        if "anchor" in u or kind in REFERENCES:
            anchor = line(anchor, path + "/anchor").removeprefix("^")
            if not re.fullmatch(r"[A-Za-z0-9-]+", anchor) or anchor in occupied:
                raise Invalid(path + "/anchor", "invalid or duplicate block ID")
            prefix = anchor.partition("-")[0]
            ns = {"fig": "figure", "eq": "formula", "tbl": "table", "ref": "reference", "sref": "reference"}
            if prefix in ns and ns[prefix] != ("reference" if kind in REFERENCES else kind):
                raise Invalid(path + "/anchor", f"{prefix}- belongs to a different object kind")
            occupied.add(anchor)
            targets[uid] = "^" + anchor

    heading_names = Counter(targets[uid] for uid in headings)
    ambiguous = {name for name, count in heading_names.items() if count > 1}
    chunks, mapping = [], []
    offset, current_line = 0, 1
    previous = None
    if "properties" in plan:
        props = yaml.safe_dump(json.loads(json.dumps(plan["properties"], sort_keys=True)), allow_unicode=True, sort_keys=True)
        chunks.append("---\n" + props + "---\n\n")
        offset, current_line = len(chunks[0]), 1 + chunks[0].count("\n")
    for i, u in enumerate(units):
        uid, kind = u["unit_id"], u["kind"]
        path, cp, values = f"/plan/units/{i}", "/content/" + uid, content[uid]
        if kind == "header":
            blocks = ["# " + literal(line(u["original_title"], path + "/original_title"))]
            if u["translated"]:
                blocks[0] += "\n" + literal(values["translated_title"])
            for field, sep in (("authors", "; "), ("affiliations", "  \n")):
                items = u[field]
                if not isinstance(items, list):
                    raise Invalid(path + "/" + field, "expected a list of source lines")
                if items:
                    blocks.append(sep.join(_header_item(item, field, f"{path}/{field}/{j}")
                                           for j, item in enumerate(items)))
            if u.get("notes"):
                blocks.append("\n".join("> " + literal(note) for note in u["notes"]))
            block = "\n\n".join(blocks)
        elif kind == "metadata":
            block = "\n".join("> " + literal(note) for note in u["notes"])
        elif kind == "list":
            prefix = lambda n: f"{n}. " if u["style"] == "ordered" else "- "
            block = "\n".join(prefix(j) + literal(item) for j, item in enumerate(values["items"], 1))
        elif kind == "heading":
            block = headings[uid]
        elif kind in {"paragraph", "translator_note"}:
            block = _inline(u, values["text"], cp + "/text", targets, numbers, ambiguous)
            if kind == "translator_note":
                block = "> 译注：" + block
        elif kind == "formula":
            latex = u["latex"]
            if not isinstance(latex, str) or not latex.strip() or "$$" in latex or any(
                    (ord(c) < 32 and c not in "\n\t") or c in "\x7f\x85\u2028\u2029" for c in latex):
                raise Invalid(path + "/latex", "expected raw LF LaTeX without display delimiters")
            block = "$$\n" + latex + "\n$$\n" + targets[uid]
        elif kind in {"figure", "table"}:
            try:
                if kind == "table":
                    data = json.loads(json.dumps(u["data"], sort_keys=True))
                    block = build_rich_table_block(data, anchor=targets[uid]).removesuffix("\n")
                else:
                    images = u["images"]
                    if not isinstance(images, list) or not images:
                        raise Invalid(path + "/images", "expected source image list")
                    for j, image in enumerate(images):
                        ip = f"{path}/images/{j}"
                        keys(image, ip, "target alt")
                        line(image["target"], ip + "/target")
                        if image["alt"] != "":
                            line(image["alt"], ip + "/alt")
                    cap = _inline(u, values.get("caption", ""), cp + "/caption", targets, numbers, ambiguous)
                    label = literal(line(u["label"], path + "/label"))
                    caption = "**" + label + "**" + (" " + cap if cap else "")
                    block = build_figure_block(u["images"], caption=caption, anchor=targets[uid]).removesuffix("\n")
            except (ValueError, TypeError) as error:
                if isinstance(error, Invalid):
                    raise
                raise Invalid(path, str(error)) from error
        elif kind in REFERENCES:
            block = f"{numbers[uid]}. " + literal(line(u["source_text"], path + "/source_text")) + " " + targets[uid]
        else:
            uri = line(u["uri"], path + "/uri")
            if not re.fullmatch(r"zotero://select/(?:library|groups/[1-9][0-9]*)/items/[A-Za-z0-9]+", uri):
                raise Invalid(path + "/uri", "expected a Zotero item selection URI")
            block = f"---\n\n## [Zotero原条目]({uri})"
        sep = "" if previous is None else "\n\n"
        follows_heading = previous == "heading" and kind in {"paragraph", "translator_note", "metadata", "list", *REFERENCES}
        same_list = previous in REFERENCES and kind in REFERENCES
        if follows_heading or same_list or previous == kind == "metadata":
            sep = "\n"
        chunks.extend((sep, block))
        offset += len(sep)
        current_line += sep.count("\n")
        mapping.append(dict(unit_id=uid, kind=kind, scope=u["scope"], source=deepcopy(u["source"]),
                            start_line=current_line, end_line=current_line + block.count("\n"),
                            start_offset=offset, end_offset=offset + len(block), sha256=digest(block)))
        current_line += block.count("\n")
        offset += len(block)
        previous = kind
    return "".join(chunks) + "\n", mapping
