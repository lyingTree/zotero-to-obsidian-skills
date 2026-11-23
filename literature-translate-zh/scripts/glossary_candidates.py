"""Retrieve glossary evidence from source text without choosing a translation.

Exact phrases and uncommon shared English tokens are search leads only. The
caller still decides whether a hit describes this source concept. No candidate
Markdown, glossary row, acceptance status, or terminology decision is changed.
"""
from __future__ import annotations

from collections import defaultdict
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import re
from typing import Callable


TOKEN = re.compile(r"[A-Za-z][A-Za-z0-9]*")
IDENTIFIER = re.compile(r"[A-Za-z][A-Za-z0-9_-]*")
TABLE_SEPARATOR = re.compile(r":?-{3,}:?")
ALIAS = re.compile(r"[;；]\s*别名(?:/缩写)?\s*[:：]\s*")
Progress = Callable[[int, int, str], None]


def _binding(path: Path) -> dict:
    path = Path(path).resolve(strict=True)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return {"path": str(path), "sha256": digest.hexdigest()}


def _tokens(text: str) -> tuple[str, ...]:
    return tuple(match[0].casefold() for match in TOKEN.finditer(text))


def _text_snapshot(path: Path) -> tuple[str, dict]:
    """Decode and bind the same bytes, even if the file changes after reading."""
    path = Path(path).resolve(strict=True)
    raw = path.read_bytes()
    return raw.decode("utf-8-sig"), {"path": str(path), "sha256": hashlib.sha256(raw).hexdigest()}


def _table_cells(raw: str) -> list[str]:
    cells = [cell.strip().replace(r"\|", "|") for cell in re.split(r"(?<!\\)\|", raw)]
    if cells and not cells[0]:
        cells = cells[1:]
    if cells and not cells[-1]:
        cells = cells[:-1]
    return cells


def _table_separator(cells: list[str]) -> bool:
    return len(cells) >= 3 and all(TABLE_SEPARATOR.fullmatch(cell) for cell in cells)


class GlossaryIndex:
    """Build reusable phrase/token indexes from the actual Markdown table."""

    def __init__(self, path: Path):
        text, self.binding = _text_snapshot(path)
        self.entries = {}
        self.token_entries = defaultdict(set)
        self.first_token_phrases = defaultdict(list)
        self.entry_phrases = defaultdict(list)
        lines = text.splitlines()
        for line_number, raw in enumerate(lines, 1):
            cells = _table_cells(raw)
            if len(cells) < 3 or _table_separator(cells):
                continue
            following = _table_cells(lines[line_number]) if line_number < len(lines) else []
            if len(following) == len(cells) and _table_separator(following):
                continue  # A Markdown header is identified by its delimiter row.
            if not IDENTIFIER.fullmatch(cells[0]):
                continue
            identity = cells[0]
            if identity in self.entries:
                raise ValueError("Duplicate glossary entry: " + identity)
            english, chinese = cells[1:3]
            if not english or not chinese:
                raise ValueError("A glossary entry needs its source phrase and translation: " + identity)
            parts = ALIAS.split(english, maxsplit=1)
            phrases = [parts[0]]
            if len(parts) == 2:
                phrases.extend(item.strip() for item in re.split(r"[;；、]", parts[1]) if item.strip())
            entry = {"id": identity, "english": english, "translation": chinese,
                     "context": cells[3] if len(cells) > 3 else "", "line": line_number, "raw_line": raw,
                     "evidence": {**self.binding, "locator": {"kind": "lines", "start": line_number, "end": line_number}}}
            self.entries[identity] = entry
            seen = set()
            for phrase in phrases:
                tokens = _tokens(phrase)
                if not tokens or tokens in seen:
                    continue
                seen.add(tokens)
                self.first_token_phrases[tokens[0]].append((tokens, identity, phrase))
                self.entry_phrases[identity].append(tokens)
                for token in set(tokens):
                    self.token_entries[token].add(identity)
        if not self.entries:
            raise ValueError("The glossary has no identifiable term rows.")

    def search(self, query: str) -> list[dict]:
        """Return actual English-token phrase matches, including zero hits."""
        if not isinstance(query, str) or not query.strip():
            raise ValueError("Provide the actual nonempty search query.")
        tokens = _tokens(query)
        if not tokens:
            return [deepcopy(entry) for entry in self.entries.values() if query.casefold() in entry["raw_line"].casefold()]
        postings = [self.token_entries.get(token, set()) for token in set(tokens)]
        if any(not values for values in postings):
            return []
        candidates = set.intersection(*sorted(postings, key=len))
        found = set()
        for identity in candidates:
            for phrase in self.entry_phrases[identity]:
                if any(phrase[index:index + len(tokens)] == tokens for index in range(len(phrase) - len(tokens) + 1)):
                    found.add(identity)
        return [deepcopy(self.entries[identity]) for identity in sorted(found)]

    def suggest(self, text: str, *, max_family_entries: int = 8) -> dict:
        """Return all exact phrases and bounded-frequency token-family leads.

        A family lead does not assert synonymy, a domain category, exhaustive
        term discovery, or the correctness of applying the whole Chinese row.
        The frequency bound controls retrieval noise, not acceptance scope.
        """
        if not isinstance(text, str):
            raise ValueError("Source text must be a string.")
        if type(max_family_entries) is not int or max_family_entries < 1:
            raise ValueError("max_family_entries must be a positive integer.")
        spans = list(TOKEN.finditer(text))
        tokens = tuple(span[0].casefold() for span in spans)
        matches = {}

        def add(identity: str, start: int, stop: int, mode: str, query: str) -> None:
            key = (identity, start, stop)
            if key in matches:
                matches[key]["match_types"].append(mode)
                return
            matches[key] = {"entry": deepcopy(self.entries[identity]), "match_types": [mode],
                            "query": query, "source_span": {"start": start, "end": stop, "quote": text[start:stop]},
                            "decision": "pending"}

        for index, span in enumerate(spans):
            for phrase, identity, raw_phrase in self.first_token_phrases.get(tokens[index], ()):
                end = index + len(phrase)
                if tokens[index:end] == phrase:
                    add(identity, span.start(), spans[end - 1].end(), "exact_token_phrase", raw_phrase)
            identities = self.token_entries.get(tokens[index], ())
            if len(tokens[index]) < 4 or not 0 < len(identities) <= max_family_entries:
                continue
            for identity in sorted(identities):
                add(identity, span.start(), span.end(), "shared_token_family", span[0])
        return {
            "glossary": dict(self.binding), "source_text_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
            "matches": sorted(matches.values(), key=lambda row: (row["source_span"]["start"], row["entry"]["id"], row["source_span"]["end"])),
            "source_fidelity": "not_assessed",
            "boundary": "Search leads only. Read each hit in its source context; a shared token is not an equivalent concept or an adopted translation. Unmatched terms still require manual discovery.",
        }


def prepare_glossary_candidates(
    inventory_path: Path, glossary_path: Path, output_path: Path, *, progress: Progress | None = None,
) -> Path:
    """Save source-block search leads with actual inventory/glossary bindings."""
    output_path = Path(output_path).resolve()
    if output_path.exists():
        raise FileExistsError("Keep existing glossary search evidence: " + str(output_path))
    inventory_text, inventory_binding = _text_snapshot(inventory_path)
    inventory_path = Path(inventory_binding["path"])
    inventory = json.loads(inventory_text)
    index = GlossaryIndex(glossary_path)
    blocks = [(source["id"], page["page"], block)
              for source in inventory["sources"] for page in source["pages"] for block in page["blocks"]]
    source_bindings = {}
    for source in inventory["sources"]:
        path = Path(source["path"])
        path = path if path.is_absolute() else inventory_path.parent.joinpath(path)
        actual = _binding(path)
        if actual["sha256"] != source["sha256"]:
            raise ValueError("Source bytes differ from inventory: " + source["id"])
        source_bindings[source["id"]] = actual
    rows = []
    for number, (source_id, page_number, block) in enumerate(blocks, 1):
        result = index.suggest(block["text"])
        rows.append({"block_id": block["id"], "source_id": source_id, "page": page_number,
                     "bbox": block["bbox"], "source_text_sha256": result["source_text_sha256"],
                     "matches": result["matches"]})
        if progress:
            progress(number, len(blocks), "source glossary search leads")
    for binding in (inventory_binding, index.binding, *source_bindings.values()):
        if _binding(Path(binding["path"])) != binding:
            raise ValueError("Input changed during glossary search: " + binding["path"])
    payload = {
        "schema_version": 1, "bindings": {"source_inventory": inventory_binding, "glossary": index.binding,
                                           "sources": source_bindings},
        "blocks": rows, "source_fidelity": "not_assessed",
        "boundary": "Automatic retrieval only; no term is adopted or excluded, and no semantic review is completed by this file.",
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("x", encoding="utf-8") as stream:
        json.dump(payload, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    return output_path
