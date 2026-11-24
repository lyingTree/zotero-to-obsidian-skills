"""Read-only, deterministic mechanical acceptance of one UTF-8 Markdown note.

Sources are trusted inputs to the contract. Their file digests bind the evidence,
but do not establish that the contract enumerates everything in the paper.
Relative source paths resolve beside the candidate. Embedded local resources
resolve under resource_root (default the candidate parent).
Line spans are one-based inclusive; span digests omit the terminating LF.
"""

from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import asdict
from functools import lru_cache
from hashlib import sha256
import importlib.util
import json
from pathlib import Path, PureWindowsPath
import re
import sys
from urllib.parse import unquote, urlsplit

import yaml


def _module(name):
    path = Path(__file__).resolve().with_name(name + ".py")
    key = "literature_accept_" + name + "_" + sha256(str(path).encode()).hexdigest()[:12]
    if key in sys.modules:
        return sys.modules[key]
    spec = importlib.util.spec_from_file_location(key, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[key] = module
    spec.loader.exec_module(module)
    return module


delivery = _module("validate_delivery")
note = delivery.note_checks
lint = _module("lint_presentation")
normalizer = _module("normalize_note")
HASH = re.compile("[a-f0-9]{64}")
ANCHOR = re.compile("[A-Za-z0-9-]+")
UNIT_KINDS = {"header", "heading", "list", "metadata", "paragraph", "translator_note", "figure",
              "formula", "table", "reference", "bibliography", "zotero_link"}
FIELDS = {"schema_version", "sources", "required_units", "required_anchors",
          "counts", "table_dimensions", "resources", "plan_sha256"}


def _json_hash(value):
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)
    return sha256(raw.encode()).hexdigest()


def _file_hash(path):
    digest = sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _string(value):
    return isinstance(value, str) and bool(value.strip())


def _digest(value):
    return isinstance(value, str) and bool(HASH.fullmatch(value))


def _natural(value):
    return type(value) is int and value >= 0


def _inventory(text, audit):
    """Use the delivery masks for fenced examples; keep real table data separate."""
    lines = text.split("\n")
    # Both upstream scanners accept YAML's explicit document-end marker.
    closing = next((i for i, line in enumerate(lines[1:], 1) if line.strip() in {"---", "..."}), 0)
    if lines[0].strip() == "---" and closing and lines[closing].strip() == "...":
        lines[closing] = "---"
    visible = delivery._visible("\n".join(lines), keep_tables=False).split("\n")
    tables = delivery._visible("\n".join(lines), keep_table_fences=True).split("\n")
    counts = dict.fromkeys(("figures", "equations", "bibliography", "tables", "images"), 0)
    anchors, dimensions, resources, targets = {}, {}, [], []
    headings = Counter()
    events = defaultdict(list)
    prior = []  # two most recent visible nonempty lines
    images_on_line = {}
    def event(kind, start, end):
        events[end].append((kind, start))
    fence = None
    equation = False
    bibliography = None

    def scan_links(line, lineno):
        line, _ = note._mask_comments(line, None)
        line = note._mask_delimited(note._mask_delimited(line, "`"), "$")
        for link in delivery.scan_markdown_links(line):
            if link.image:
                resources.append((link.href, lineno, "markdown"))
        for match in re.finditer(r"!\[\[([^\]\n|]+)(?:\|[^\]\n]*)?\]\]", line):
            if not note._escaped(line, match.start()):
                resources.append((match[1], lineno, "wiki"))
        targets.extend(("block", target, lineno) for target, _ in delivery._reference_links(line))
        for link in delivery.scan_markdown_links(line):
            if link.href.startswith("#") and not link.href.startswith("#^") and not link.image:
                targets.append(("heading", unquote(link.href[1:]), lineno))
        for match in re.finditer(r"\[\[#(?!\^)([^|\]\n]+)(?:\|[^\]\n]*)?\]\]", line):
            if not note._escaped(line, match.start()):
                targets.append(("heading", match[1], lineno))

    for idx, (line, table_line) in enumerate(zip(visible, tables)):
        lineno = idx + 1
        if fence:
            marker, opening = fence
            if not re.fullmatch(r" {0,3}" + re.escape(marker[0]) + "{" + str(len(marker)) + r",}[ \t]*", table_line):
                continue
            fence = None
            event("table", opening + 1, lineno)
            data = yaml.load(note._rich_table_payload(lines[opening+1:idx]), Loader=note._UniqueKeyLoader)
            if not isinstance(data, dict):
                continue
            sheets = data.get("sheets", []) if data.get("version") == 3 else [data]
            if data.get("version") == 3:
                for value in note._strings([data.get("title"), data.get("footer")]):
                    scan_links(value, opening + 1)
            sizes = []
            for sheet in sheets if isinstance(sheets, list) else []:
                if not isinstance(sheet, dict):
                    continue
                delivery._Audit.check_merges(audit, f"line {opening+1}", None, sheet)
                fields = [sheet.get("title"), sheet.get("footer")]
                for field, key in (("columns", "name"), ("rows", "cells")):
                    entries = sheet.get(field)
                    if isinstance(entries, list):
                        fields.extend(row.get(key) for row in entries if isinstance(row, dict))
                for value in note._strings(fields):
                    scan_links(value, opening + 1)
                rows, cols = sheet.get("rows"), sheet.get("columns")
                if isinstance(rows, list) and isinstance(cols, list):
                    sizes.append({"rows": len(rows), "columns": len(cols)})
            anchor = note._WHOLE_ANCHOR.fullmatch(visible[idx+1]) if idx+1 < len(visible) else None
            if anchor:
                dimensions[anchor[1]] = sizes
            continue
        fenced = note._FENCE.match(table_line)
        if fenced and fenced[2].strip() == "rich-table":
            counts["tables"] += 1
            fence = fenced[1], idx
            continue
        if line.strip() == "$$":
            if equation:
                counts["equations"] += 1
                event("formula", equation, lineno)
                equation = False
            else:
                equation = lineno
            continue
        if equation:
            continue
        if re.fullmatch(r" {0,3}\$\$.+\$\$[ \t]*", line):
            counts["equations"] += 1
            event("formula", lineno, lineno)
            continue
        line = note._mask_delimited(line, "$")
        if anchor := note._ANCHOR.search(line):
            anchors[anchor[1]] = lineno
            if anchor[1].startswith("fig-"):
                standalone = bool(note._WHOLE_ANCHOR.fullmatch(line))
                caption = prior[-1] if standalone and prior else (lineno, line[:anchor.start()].strip())
                img = prior[-2] if standalone and len(prior) >= 2 else prior[-1] if prior else (0, "")
                if caption[1].strip() and not note._HEADING.match(caption[1]) and img[0] in images_on_line:
                    counts["figures"] += 1
                    event("figure", images_on_line[img[0]], lineno)
        if heading := note._HEADING.match(line):
            event("header" if len(heading[1]) == 1 else "heading", lineno, lineno)
            headings[re.sub(r"\\([^\w\s])", r"\1", heading[2])] += 1
            if bibliography is not None and len(heading[1]) <= bibliography:
                bibliography = None
            if note._BIBLIOGRAPHY.fullmatch(heading[2]):
                bibliography = len(heading[1])
        elif re.fullmatch(r" {0,3}(?:-{3,}|\*{3,}|_{3,})[ \t]*", line):
            bibliography = None
        elif bibliography and note._ENTRY.match(line):
            counts["bibliography"] += 1
        if line.strip():
            if lint._is_standalone_image(line):
                previous_image = prior[-1][0] if prior else 0
                images_on_line[lineno] = images_on_line.get(previous_image, lineno)
            if note._ENTRY.match(line) and anchor and note._citation_preview_anchor(anchor[1]):
                event("reference", lineno, lineno)
                event("bibliography", lineno, lineno)
            if re.match(r"^(?:- |[0-9]+\. )", line):
                event("list", lineno, lineno)
            if line.startswith("> "):
                event("metadata", lineno, lineno)
            if line.startswith("> 译注："):
                event("translator_note", lineno, lineno)
            if re.search(r"^## \[Zotero原条目\]\(zotero://select/", line):
                event("zotero_link", lineno, lineno)
            event("paragraph", lineno, lineno)
            prior = (prior + [(lineno, line)])[-2:]
        scan_links(line, lineno)
    counts["images"] = len(resources)
    return anchors, counts, dimensions, resources, targets, headings, events


class _Audit:
    def __init__(self, path, root):
        self.path = path
        self.root = root
        self.checks = []
        self.prompts = []
        self.hash_file = lru_cache(maxsize=None)(_file_hash)

    def check(self, code, location, expected, actual, passed=None):
        if passed is None:
            passed = expected == actual
        self.checks.append(dict(code=code, location=location, expected=expected, actual=actual,
                                status="pass" if passed else "fail"))
        return passed

    def fail(self, code, location, message):
        self.check(code, location, "valid Rich Table grid", message, False)

    def shape(self, ok, location, expected):
        if not ok:
            self.check("SCHEMA", location, expected, "missing or invalid", False)
        return ok

    def canonical_format(self, text):
        result = normalizer.normalize_note(text)
        blocked = False
        for issue in result["unresolved"]:
            code = issue["code"]
            if code.startswith("UNCLOSED_") or code == "INTERACTING_RULES":
                blocked = True
                self.check(code, f"line {issue['line']}", "complete, stable parse", issue["message"], False)
            elif issue.get("severity") != "info":
                self.prompts.append({**issue, "status": "pending"})
        changes = result["changes"]
        if not changes:
            self.check("CANONICAL_FORMAT", "candidate", "verified fixed point",
                       "parse incomplete" if blocked else "verified fixed point",
                       not blocked and result["idempotent"])
            return
        for change in changes:
            loc = f"line {change['line']}, column {change['column']}"
            self.check("CANONICAL_FORMAT", loc, change["after"], change["before"], False)
            self.checks[-1].update(rule=change["rule"], stage=change["stage"],
                                   line=change["line"], column=change["column"])

    def resource(self, href, line, syntax, hashes):
        location = f"line {line}: {href}"
        try:
            # Wikilink targets are literal vault names. Their heading/block
            # suffix is separated by #; percent signs and ? are filename data.
            parsed = urlsplit(href) if syntax == "markdown" else None
            if parsed and parsed.scheme.lower() in {"http", "https"} and parsed.netloc:
                self.prompts.append(dict(code="REMOTE_RESOURCE", location=location, status="pending"))
                if href in hashes:
                    self.check("RESOURCE_HASH", location, hashes[href], "remote resource", False)
                return
            name = unquote(parsed.path, errors="strict") if parsed else href.partition("#")[0]
            path = Path(name)
            valid = bool(name) and not ((parsed and (parsed.scheme or parsed.netloc)) or path.is_absolute()
                                        or PureWindowsPath(name).drive or "\\" in name)
            resolved = self.root.joinpath(path).resolve() if valid else None
            if not self.check("RESOURCE_BOUNDARY", location, "within resource root", str(resolved),
                              resolved is not None and resolved.is_relative_to(self.root)):
                return
            if not self.check("RESOURCE_FILE", location, True, resolved.is_file()):
                return
            actual = self.hash_file(resolved)
            if href in hashes:
                self.check("RESOURCE_HASH", location, hashes[href], actual)
            else:
                self.check("RESOURCE_FINGERPRINT", location, "readable SHA-256", actual, True)
        except (OSError, ValueError, RuntimeError) as error:
            self.check("RESOURCE_READ", location, "readable path", str(error), False)

    def sources(self, sources):
        if not self.shape(isinstance(sources, dict) and bool(sources), "sources", "nonempty source mapping"):
            return {}
        for identity, source in sources.items():
            location = f"sources.{identity}"
            if not self.shape(_string(identity) and isinstance(source, dict)
                              and _string(source.get("path")) and _digest(source.get("sha256")),
                              location, "source ID, file path and SHA-256"):
                continue
            try:
                observed = self.hash_file(self.path.parent.joinpath(source["path"]).resolve())
            except (ValueError, OSError, RuntimeError) as error:
                observed = str(error)
            self.check("SOURCE_HASH", location, source["sha256"], observed)
        return sources

    def units(self, units, sources, rendering, raw, contract, events, anchors):
        if not self.shape(isinstance(units, list), "required_units", "array"):
            return
        expected = []
        for index, unit in enumerate(units):
            loc = f"required_units[{index}]"
            if not self.shape(isinstance(unit, dict) and _string(unit.get("unit_id")), loc, "unit with ID"):
                continue
            self.check("UNIT_FIELDS", loc, [], sorted(set(unit) - {"unit_id", "kind", "source", "scope", "anchor"}))
            if "scope" in unit:
                scope = unit["scope"]
                self.shape(isinstance(scope, str) and bool(re.fullmatch(r"main|supplement:[A-Za-z0-9][A-Za-z0-9_.-]*", scope)), loc, "main or named supplement scope")
            source = unit.get("source")
            self.shape(isinstance(unit.get("kind"), str) and unit["kind"] in UNIT_KINDS and isinstance(source, dict)
                       and isinstance(source.get("source_id"), str) and source["source_id"] in sources
                       and (_string(source.get("locator")) or isinstance(source.get("locator"), dict) and bool(source["locator"])), loc, "kind and locator in known source")
            expected.append(unit["unit_id"])
        self.check("UNIQUE_UNIT_IDS", "required_units", [], sorted(k for k,v in Counter(expected).items() if v > 1))
        if rendering is None and not units:
            return
        if not self.shape(isinstance(rendering, dict) and isinstance(rendering.get("mapping"), list),
                          "rendering", "rendered unit mapping"):
            return
        self.check("RENDER_STATUS", "rendering.status", "rendered", rendering.get("status"))
        self.check("RENDER_SCHEMA", "rendering.schema_version", 1, rendering.get("schema_version"),
                   type(rendering.get("schema_version")) is int and rendering["schema_version"] == 1)
        self.check("OUTPUT_HASH", "rendering.output_sha256", sha256(raw).hexdigest(), rendering.get("output_sha256"))
        if "plan_sha256" in contract:
            self.check("PLAN_HASH", "rendering.plan_sha256", contract["plan_sha256"], rendering.get("plan_sha256"))
        mapping = rendering["mapping"]
        ids = [entry.get("unit_id") if isinstance(entry, dict) else None for entry in mapping]
        self.check("UNIT_ORDER", "rendering.mapping", expected, ids)
        lines = raw.decode("utf-8").split("\n")
        offsets = [0]
        for line in lines[:-1]:
            offsets.append(offsets[-1] + len(line) + 1)
        previous = 0
        # Renderer-owned YAML precedes the first source-unit span.
        if lines[0].strip() == "---" and mapping and isinstance(mapping[0], dict):
            close = next((i for i, line in enumerate(lines[1:], 1) if line.strip() in {"---", "..."}), None)
            if close is not None and mapping[0].get("start_line") != 1:
                previous = close + 1
        for index, entry in enumerate(mapping):
            loc = f"rendering.mapping[{index}]"
            if not self.shape(isinstance(entry, dict), loc, "mapping entry"):
                continue
            if index < len(units) and isinstance(units[index], dict):
                unit = units[index]
                for field in ("source", "kind", "scope"):
                    if field in unit:
                        self.check("UNIT_" + field.upper(), loc, unit[field], entry.get(field))
            start, end = entry.get("start_line"), entry.get("end_line")
            if not self.check("UNIT_SPAN", loc, f"{previous} < start <= end <= {len(lines)}",
                              [start, end], type(start) is int and type(end) is int
                              and previous < start <= end <= len(lines)):
                continue
            gap = lines[previous:start-1]
            self.check("UNMAPPED_CONTENT", loc, False, any(line.strip() for line in gap))
            previous = end
            snippet = "\n".join(lines[start-1:end])
            self.check("UNIT_NONEMPTY", loc, True, bool(snippet.strip()))
            if index < len(units) and isinstance(units[index], dict):
                unit = units[index]
                kind = unit.get("kind")
                if "anchor" in unit and self.shape(_string(unit["anchor"]), loc, "block anchor"):
                    anchor = unit["anchor"].removeprefix("^")
                    actual = anchors.get(anchor)
                    self.check("UNIT_ANCHOR", loc, f"{anchor} on lines {start}..{end}", actual,
                               actual is not None and start <= actual <= end)
                if isinstance(kind, str) and kind in UNIT_KINDS:
                    has_object = any(actual_kind == kind and begin >= start for i in range(start, end+1)
                                     for actual_kind, begin in events.get(i, ()))
                    self.check("UNIT_OBJECT", loc, kind, "matching active syntax" if has_object else "missing syntax", has_object)
            if "start_offset" in entry or "end_offset" in entry:
                expected = [offsets[start-1], offsets[end-1] + len(lines[end-1])]
                actual = [entry.get("start_offset"), entry.get("end_offset")]
                self.check("UNIT_OFFSETS", loc, expected, actual, all(type(i) is int for i in actual) and expected == actual)
            self.check("UNIT_HASH", loc, sha256(snippet.encode()).hexdigest(), entry.get("sha256"))
        self.check("UNMAPPED_CONTENT", "after mapping", False, any(line.strip() for line in lines[previous:]))

    def expectations(self, contract, anchors, counts, dimensions, resources):
        required = contract.get("required_anchors", [])
        if self.shape(isinstance(required, list), "required_anchors", "array of bare IDs"):
            for index, key in enumerate(required):
                if self.shape(isinstance(key, str) and bool(ANCHOR.fullmatch(key)),
                              f"required_anchors[{index}]", "bare anchor ID"):
                    self.check("REQUIRED_ANCHOR", key, True, key in anchors)
        expected_counts = contract.get("counts", {})
        if self.shape(isinstance(expected_counts, dict), "counts", "object counts"):
            for key, expected in expected_counts.items():
                if self.shape(key in counts and _natural(expected), f"counts.{key}", "supported nonnegative count"):
                    self.check("OBJECT_COUNT", f"counts.{key}", expected, counts[key])
        sizes = contract.get("table_dimensions", {})
        if self.shape(isinstance(sizes, dict), "table_dimensions", "anchor to sheet dimensions"):
            for key, expected in sizes.items():
                valid = isinstance(expected, list) and bool(expected) and all(
                    isinstance(sheet, dict) and set(sheet) == {"rows", "columns"}
                    and all(_natural(x) for x in sheet.values()) for sheet in expected)
                if self.shape(valid, f"table_dimensions.{key}", "nonempty sheet dimensions"):
                    self.check("TABLE_DIMENSIONS", key, expected, dimensions.get(key))
        hashes = contract.get("resources", {})
        if not self.shape(isinstance(hashes, dict), "resources", "href to SHA-256"):
            return {}
        emitted = {href for href, _, _ in resources}
        for href, digest in hashes.items():
            self.shape(_string(href) and _digest(digest), f"resources.{href}", "href and SHA-256")
            self.check("RESOURCE_EMBEDDED", href, True, href in emitted)
        return hashes


def _stage(code, location):
    if code == "SOURCE_FIDELITY":
        return "translation_review"
    if "GAP" in code or "SPACING" in code or "HARD_BREAK" in code:
        return "spacing"
    if code.startswith(("RESOURCE", "REMOTE_RESOURCE")):
        return "resources"
    if code.startswith(("BIBLIOGRAPHY", "CITATION")):
        return "citations"
    if "ANCHOR" in code or code == "INTERNAL_TARGET":
        return "internal_links"
    if any(term in code for term in ("QUANTITY", "PERCENT", "MATH", "FORMULA", "DEGREE", "EQUATION")):
        return "math_formatting"
    if "FIGURE" in code or "CROP" in code:
        return "figures"
    if "TABLE" in code or "RICH" in code:
        return "tables"
    if code.startswith("HEADING"):
        return "headings"
    if code.startswith(("FRONTMATTER", "HEADER", "AUTHOR", "AFFILIATION")):
        return "metadata"
    if code.startswith(("UNIT_", "RENDER", "OUTPUT_HASH", "UNMAPPED", "UNBOUND")):
        return "assembly"
    if code.startswith(("SOURCE", "CONTRACT", "REQUIRED", "PLAN_HASH", "OBJECT_COUNT", "SCHEMA", "UNIQUE_UNIT")):
        return "source_coverage"
    return "markdown_structure"


@lru_cache(maxsize=1)
def _rules_hash():
    scripts = Path(__file__).resolve().parent
    names = ("accept_note", "normalize_note", "validate_delivery", "validate_note", "lint_presentation", "format_helpers")
    return _json_hash({name: _file_hash(scripts.joinpath(name + ".py")) for name in names})


def accept_note(candidate_path: Path, contract: dict | None = None, *,
                resource_root: Path | None = None, rendering: dict | None = None) -> dict:
    """Check actual candidate bytes, files and declared structural expectations.

    A verified coverage result is limited to the source-bound contract. For a
    legacy note, counts and anchors can be checked without a renderer mapping.
    Scope and source metadata are compared whenever supplied by the unit plan.
    """
    path = Path(candidate_path)
    audit = _Audit(path, resource_root if resource_root is not None else path.parent)
    fingerprints = {"candidate_sha256": None, "contract_sha256": None, "rendering_sha256": None}
    coverage = "not_assessed"
    counts = {}
    try:
        for key, value in (("contract_sha256", contract), ("rendering_sha256", rendering)):
            if value is not None:
                fingerprints[key] = _json_hash(value)
        raw = path.read_bytes()
        fingerprints["candidate_sha256"] = sha256(raw).hexdigest()
        text = raw.decode("utf-8-sig").replace("\r\n", "\n").replace("\r", "\n")
        audit.canonical_format(raw.decode("utf-8"))
        audit.root = Path(audit.root).resolve()
        first = text.split("\n", 1)[0].strip()
        audit.check("FRONTMATTER_PRESENT", "line 1", "---", first)
        findings = note.validate_note(text)
        if first == "---" and not any(item.code.startswith("FRONTMATTER") for item in findings):
            lines = text.split("\n")
            end = next((i for i, line in enumerate(lines[1:], 1) if line.strip() in {"---", "..."}), None)
            if end is not None:
                metadata = yaml.load("\n".join(lines[1:end]), Loader=note._UniqueKeyLoader)
                audit.check("FRONTMATTER_MAPPING", "line 1", "mapping", type(metadata).__name__,
                            isinstance(metadata, dict))
        audit.check("NOTE_STRUCTURE", "candidate", 0, sum(item.level == "error" for item in findings))
        for item in findings:
            if item.level == "error":
                audit.check(item.code, f"line {item.line}", "satisfies structural rule", item.message, False)
            else:
                audit.prompts.append({**asdict(item), "status": "pending"})
        audit.prompts.extend({**asdict(item), "status": "pending"} for item in lint.lint_presentation(text))
        anchors, counts, dimensions, resources, links, headings, events = _inventory(text, audit)
        for kind, target, line in links:
            hits = int(target in anchors) if kind == "block" else headings[target]
            audit.check("INTERNAL_TARGET", f"line {line}: {target}", 1, hits)
        hashes = {}
        if contract is not None and audit.shape(isinstance(contract, dict), "contract", "mapping"):
            audit.check("CONTRACT_FIELDS", "contract", [], sorted(set(contract) - FIELDS))
            version = contract.get("schema_version")
            audit.check("CONTRACT_VERSION", "contract", 1, version, type(version) is int and version == 1)
            sources = audit.sources(contract.get("sources"))
            if "plan_sha256" in contract:
                audit.shape(_digest(contract["plan_sha256"]), "plan_sha256", "SHA-256")
                audit.shape(rendering is not None, "rendering", "mapping bound to plan digest")
            units = contract.get("required_units", [])
            audit.units(units, sources, rendering, raw, contract, events, anchors)
            hashes = audit.expectations(contract, anchors, counts, dimensions, resources)
            if any(contract.get(key) for key in ("required_units", "required_anchors", "counts",
                                                 "table_dimensions", "resources")):
                coverage = "verified_against_contract"
        elif rendering is not None:
            audit.check("UNBOUND_MAPPING", "rendering", "source-bound required units", "no contract", False)
        for href, line, syntax in resources:
            audit.resource(href, line, syntax, hashes)
    except (ValueError, TypeError, OSError, RuntimeError, yaml.YAMLError, KeyError, IndexError, AttributeError) as error:
        audit.check("INPUT_INVALID", "candidate or supplied evidence", "readable supported data",
                    f"{type(error).__name__}: {error}", False)
    failed = any(item["status"] == "fail" for item in audit.checks)
    if contract is not None and failed:
        coverage = "failed"
    rules = _rules_hash()
    fingerprints["resource_root"] = str(audit.root)
    evidence = [(row["code"], row["location"], row["actual"]) for row in audit.checks
                if row["code"].startswith(("RESOURCE_", "SOURCE_HASH"))]
    fingerprints["file_evidence_sha256"] = _json_hash(evidence)
    input_hash = _json_hash(fingerprints)
    audit.prompts.append(dict(code="SOURCE_FIDELITY", status="pending",
                              message="Compare meaning, translation, object identities and source inventory with originals."))
    for item in (*audit.checks, *audit.prompts):
        item.setdefault("stage", _stage(item["code"], item.get("location", "")))
        item.update(input_sha256=input_hash, rules_sha256=rules)
    return dict(schema_version=1, mechanical_status="fail" if failed else "pass", source_coverage=coverage,
                source_fidelity="not_assessed", inputs=fingerprints, input_sha256=input_hash, rules_sha256=rules,
                counts=counts, checks=audit.checks, review_prompts=audit.prompts)
