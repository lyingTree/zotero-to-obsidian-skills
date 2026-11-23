"""Locate text-layer URL occurrences that still need individual source units.

This is a coverage aid, not a visible-page or URL-identity verdict. It preserves
the raw source token, including extraction defects. Wrapped or damaged text
needs an explicit source_url_occurrence binding after an actual page review.
No function creates source units, edits files, or awards acceptance results.
"""
from __future__ import annotations

from collections import deque
from collections.abc import Mapping
import re


URL_START = re.compile(r"(?<![A-Za-z0-9_])(?:https?://|www\.)", re.IGNORECASE)
OPEN_BRACKETS = {"(": ")", "[": "]", "{": "}"}
CLOSE_BRACKETS = frozenset(OPEN_BRACKETS.values())


def _url_tokens(text: str):
    """Keep balanced URL brackets while stopping at surrounding prose.

    Advancing past each token also keeps a redirect URL inside a query from
    becoming a second source occurrence. No URI-validity judgment is made.
    """
    cursor = 0
    while match := URL_START.search(text, cursor):
        end = match.end()
        brackets = []
        while end < len(text):
            character = text[end]
            if character.isspace() or character in "<>\"'":
                break
            if character in OPEN_BRACKETS:
                brackets.append(OPEN_BRACKETS[character])
            elif character in CLOSE_BRACKETS:
                if not brackets or brackets[-1] != character:
                    break
                brackets.pop()
            end += 1
        yield match.start(), end, text[match.start():end]
        cursor = end


def _token(value: str) -> str:
    value = value.rstrip(".,;:!?")
    for opening, closing in (("(", ")"), ("[", "]"), ("{", "}")):
        while value.endswith(closing) and value.count(closing) > value.count(opening):
            value = value[:-1].rstrip(".,;:!?")
    return value


def scan_source_url_occurrences(inventory: Mapping, *, excluded_blocks=(), excluded_pages=()) -> list[dict]:
    """Return every HTTP(S)/www text occurrence, never a deduplicated URL set."""
    if not isinstance(inventory, Mapping) or not isinstance(inventory.get("sources"), list):
        raise ValueError("Provide an inventory containing its actual sources.")
    ignored_blocks, ignored_pages = set(excluded_blocks), set(excluded_pages)
    result = []
    for source in inventory["sources"]:
        if (not isinstance(source, dict) or not isinstance(source.get("pages"), list)
                or not isinstance(source.get("id"), str) or not source["id"]):
            continue  # The main inventory validator diagnoses malformed source records.
        for page in source["pages"]:
            if not isinstance(page, dict) or type(page.get("page")) is not int or page["page"] < 1:
                continue
            if (source["id"], page["page"]) in ignored_pages:
                continue
            blocks = page.get("blocks")
            if not isinstance(blocks, list):
                continue
            for block in blocks:
                if not isinstance(block, dict) or block.get("id") in ignored_blocks:
                    continue
                text = block.get("text")
                if not isinstance(text, str) or not isinstance(block.get("id"), str):
                    continue
                for number, (start, end, raw) in enumerate(_url_tokens(text), 1):
                    result.append({
                        "id": f"{block['id']}:url:{number}", "source": source.get("id"),
                        "page": page.get("page"), "block": block["id"],
                        "start": start, "end": end,
                        "raw_text": raw, "lookup_text": _token(raw),
                        "boundary": "Text-layer lead; verify the visible source before using a URL.",
                    })
    return result


def _at_location(unit: dict, occurrence: dict) -> bool:
    if unit.get("source") != occurrence["source"]:
        return False
    records = unit.get("evidence")
    if not isinstance(records, list):
        return False
    for record in records:
        if not isinstance(record, dict) or record.get("page") != occurrence["page"]:
            continue
        blocks = record.get("blocks")
        if isinstance(blocks, list) and blocks and occurrence["block"] not in blocks:
            continue
        return True
    return False


def _forms(unit: dict) -> set[str]:
    values = {_token(value.strip()) for key in ("display_text", "value")
              if isinstance((value := unit.get(key)), str) and value.strip()}
    if unit.get("field") == "doi":
        values.update(re.sub(r"^doi:\s*", "", value, flags=re.IGNORECASE)
                      for value in tuple(values) if re.match(r"doi:\s*10\.", value, re.IGNORECASE))
        values.update(prefix + value for value in tuple(values) if value.startswith("10.")
                      for prefix in ("https://doi.org/", "http://doi.org/", "https://dx.doi.org/", "http://dx.doi.org/"))
    values.update(re.sub(r"^https?://", "", value, flags=re.IGNORECASE)
                  for value in tuple(values) if re.match(r"https?://www\.", value, re.IGNORECASE))
    return values


def source_url_coverage(inventory: Mapping, *, excluded_blocks=(), excluded_pages=()) -> dict:
    """Match individual leads to distinct, explicitly recorded metadata units.

    Explicit source_url_occurrence IDs support page-reviewed wrapped tokens.
    Otherwise only the recorded source value/display text is matched. One unit
    cannot stand in for repeated occurrences. Metadata semantics stay unassessed.
    """
    occurrences = scan_source_url_occurrences(
        inventory, excluded_blocks=excluded_blocks, excluded_pages=excluded_pages)
    by_id = {row["id"]: row for row in occurrences}
    rows = inventory.get("units")
    if not isinstance(rows, list):
        rows = []  # Malformed units remain an error in the main inventory validator.
    units = {row["id"]: row for row in rows
             if isinstance(row, dict) and isinstance(row.get("id"), str)
             and row.get("kind") == "metadata" and row.get("field") in {"external_url", "doi"}}
    invalid = []
    assigned = {}
    used_units = set()
    explicit_units = set()
    for identity, unit in units.items():
        reference = unit.get("source_url_occurrence")
        if reference is None:
            continue
        explicit_units.add(identity)
        if not isinstance(reference, str) or reference not in by_id:
            invalid.append({"unit": identity, "reason": "source_url_occurrence must identify one actual text-layer occurrence."})
            continue
        if not _at_location(unit, by_id[reference]):
            invalid.append({"unit": identity, "reason": "The explicit occurrence is outside this unit's source location."})
            continue
        if reference in assigned:
            invalid.append({"unit": identity, "reason": "Another source unit already binds this same occurrence."})
            continue
        assigned[reference] = identity
        used_units.add(identity)
    forms = {identity: _forms(unit) for identity, unit in units.items() if identity not in explicit_units}
    # Construct only plausible value matches, then use augmenting paths so a
    # broad page locator cannot steal a narrow block's only usable assignment.
    by_form = {}
    for identity, values in forms.items():
        source = units[identity].get("source")
        records = units[identity].get("evidence")
        if not isinstance(source, str) or not isinstance(records, list):
            continue
        pages = {record["page"] for record in records if isinstance(record, dict)
                 and type(record.get("page")) is int and record["page"] > 0}
        for page in pages:
            for value in values:
                by_form.setdefault((source, page, value), []).append(identity)
    edges = {row["id"]: [identity for identity in by_form.get((row["source"], row["page"], row["lookup_text"]), [])
                         if identity not in used_units and _at_location(units[identity], row)]
             for row in occurrences if row["id"] not in assigned}
    unit_to_lead = {}
    for initial in edges:
        queue = deque([initial])
        seen_leads, parent_unit = {initial}, {}
        free_unit = None
        while queue and free_unit is None:
            lead = queue.popleft()
            for identity in edges[lead]:
                if identity in parent_unit:
                    continue
                parent_unit[identity] = lead
                previous = unit_to_lead.get(identity)
                if previous is None:
                    free_unit = identity
                    break
                if previous not in seen_leads:
                    seen_leads.add(previous)
                    queue.append(previous)
        while free_unit is not None:
            lead = parent_unit[free_unit]
            previous_unit = assigned.get(lead)
            assigned[lead] = free_unit
            unit_to_lead[free_unit] = lead
            free_unit = previous_unit
    return {
        "occurrences": occurrences,
        "recorded": [{"occurrence": row["id"], "unit": assigned[row["id"]]}
                     for row in occurrences if row["id"] in assigned],
        "unmapped": [row for row in occurrences if row["id"] not in assigned],
        "invalid_bindings": invalid,
        "source_fidelity": "not_assessed",
    }
