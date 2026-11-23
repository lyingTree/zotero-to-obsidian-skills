"""One complete short-form review of wholly invented text.

Call demonstrate(Path("a new example directory")) from a Python script. This
example generates its own inputs and uses the real, unchanged prepare/validate
functions. These decisions apply only to this invented sentence; they are not
default answers for another text. records_ready describes records, not meaning.
"""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys

import fitz


SCRIPTS = Path(__file__).resolve().parents[1].joinpath("scripts")
SOURCE_TEXT = "The signal petal stays above the violet ring."
CANDIDATE_TEXT = "信号瓣片保持在紫色圆环上方。"
UNIT_ID = "invented-position"


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS.joinpath(filename))
    if spec is None or spec.loader is None:
        raise ImportError(filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


prepare = _load("compact_example_prepare", "prepare_translation_review.py")
writer_tools = _load("compact_example_writer", "translation_review_writer.py")
validator = _load("compact_example_validate", "validate_translation_review.py")


def demonstrate(directory: Path) -> dict:
    directory = Path(directory).resolve()
    directory.mkdir(parents=True, exist_ok=False)
    source = directory.joinpath("invented-source.pdf")
    candidate = directory.joinpath("invented-candidate.md")
    glossary = directory.joinpath("invented-glossary.md")
    inventory_path = directory.joinpath("invented-inventory.json")
    prepared = directory.joinpath("review-pending.json")
    draft = directory.joinpath("paragraph-decisions.json")
    output = directory.joinpath("review-complete.json")
    terminology = directory.joinpath("terminology-complete.json")
    with fitz.open() as document:
        page = document.new_page(width=440, height=180)
        page.insert_text((24, 42), SOURCE_TEXT)
        block = page.get_text("blocks")[0]
        box = list(block[:4])
        document.save(source)
    candidate.write_text(CANDIDATE_TEXT + "\n", encoding="utf-8")
    glossary.write_text(
        "| ID | English | 中文 | Context |\n| --- | --- | --- | --- |\n"
        "| INVENTED_PETAL | signal petal | 信号瓣片 | An invented display part. |\n"
        "| INVENTED_RING | ring | 圆环 | A ring-shaped part in this invented display. |\n",
        encoding="utf-8")
    inventory = {
        "schema_version": 1,
        "sources": [{"id": "invented-source", **prepare.bind_review_file(source), "page_count": 1}],
        "units": [{"id": UNIT_ID, "source": "invented-source", "kind": "paragraph",
                   "evidence": [{"page": 1, "bbox": box, "quote": SOURCE_TEXT}]}],
    }
    inventory_path.write_text(json.dumps(inventory, ensure_ascii=False, indent=2), encoding="utf-8")
    prepare.prepare_translation_review(inventory_path, candidate, prepared,
        unit_ids=[UNIT_ID], glossary_path=glossary, progress=None)

    writer = writer_tools.ReviewWriter(prepared, progress=None)
    unit = writer.unit(UNIT_ID, candidate_locations=[{"lines": [1, 1]}])
    source_sentence, translated_sentence = unit.source(SOURCE_TEXT), unit.candidate(CANDIDATE_TEXT)
    unit.occurrence("claims", "position-claim", source_sentence, translated_sentence, "passed",
                    "The petal, ring, above relation and continuing state all remain in this sentence.")
    unit.occurrence("directions", "above-ring", source_sentence, translated_sentence, "passed",
                    "Above locates the petal relative to the ring; 上方 preserves that ordered pair.",
                    source_relation={"subject": "signal petal", "reference": "violet ring", "relation": "above"},
                    candidate_relation={"subject": "信号瓣片", "reference": "紫色圆环", "relation": "上方"})
    unit.occurrence("strength", "continuing-position", source_sentence, translated_sentence, "passed",
                    "Stays describes a continuing position; 保持 preserves it without adding a causal or measured claim.",
                    source_strength="asserted continuing spatial state", candidate_strength="陈述持续的空间状态")
    unit.occurrence("modifiers", "signal-modifier", unit.source("signal petal"), unit.candidate("信号瓣片"), "passed",
                    "Signal belongs to the petal expression; 信号 qualifies 瓣片.",
                    source_candidates=["petal"], candidate_candidates=["瓣片"])
    unit.occurrence("modifiers", "violet-modifier", unit.source("violet ring"), unit.candidate("紫色圆环"), "passed",
                    "Violet modifies the ring; 紫色 qualifies 圆环.",
                    source_candidates=["ring"], candidate_candidates=["圆环"])

    petal = unit.term("petal-term", "signal petal", "concept", unit.source("signal petal"))
    petal.search("full_phrase", "signal petal", "The exact entry names this invented display component.")
    petal.search("family", "petal", "The family query returns the same component; no competing entry appears.")
    petal.search_status("aliases", "not_applicable", "This complete source sentence declares no alias or abbreviation for signal petal.")
    petal.decide("adopted", "信号瓣片", "The entry's invented display-part context matches the component in this sentence.", matches=["INVENTED_PETAL"])
    ring = unit.term("ring-term", "ring", "concept", unit.source("violet ring"))
    ring.search("full_phrase", "ring", "The entry names a ring-shaped part in this invented display.")
    ring.search("family", "ring", "The core family search finds that same part and no alternate sense in this glossary.")
    ring.search_status("aliases", "not_applicable", "This complete sentence provides no alias or abbreviation for the ring.")
    ring.decide("adopted", "圆环", "The glossary's ring-shaped component context fits the reference object; violet remains separately translated as 紫色.", matches=["INVENTED_RING"])

    # These are ten separate decisions for this sentence, not a default loop.
    scans = (
        ("concept", "scanned", ["petal-term", "ring-term"], "Both component concepts were registered and queried individually."),
        ("method", "not_applicable", [], "The complete sentence names two components and a position, with no method."),
        ("estimator", "not_applicable", [], "No estimator or estimated quantity is named in this positional statement."),
        ("statistic", "not_applicable", [], "The complete sentence has no sample statistic or statistical summary."),
        ("metric", "not_applicable", [], "Above is a qualitative relation here; no measurement metric is named."),
        ("procedure", "not_applicable", [], "The sentence describes a state and names no procedure."),
        ("model", "not_applicable", [], "No model or model family is introduced in this sentence."),
        ("parameter", "not_applicable", [], "The two components and their relative position introduce no parameter."),
        ("classification", "not_applicable", [], "The sentence names no classification scheme or class label."),
        ("abbreviation", "not_applicable", [], "Both component expressions are written out and no abbreviation is declared."),
    )
    for category, status, ids, reason in scans:
        unit.scan(category, status, ids, reason)
    checks = (
        ("alignment", "passed", "The one source sentence corresponds to the one complete translated sentence."),
        ("ambiguity", "passed", "Explicit nouns identify the petal as subject and ring as reference; the Chinese relation keeps those roles."),
        ("negation", "not_applicable", "This complete affirmative source sentence contains no negation or exclusion."),
        ("strength", "passed", "The continuing-state strength is compared in continuing-position."),
        ("number_changes", "not_applicable", "There is no numeral or quantified value in this complete sentence."),
        ("terminology", "passed", "The petal and ring decisions retain the matching glossary concepts and the separate violet modifier."),
        ("units", "not_applicable", "No measured quantity or unit occurs in this complete sentence."),
        ("symbols", "not_applicable", "No mathematical symbol or symbolic definition occurs in this complete sentence."),
        ("formulas", "not_applicable", "No inline or displayed formula occurs in this complete sentence."),
        ("citation_labels", "not_applicable", "The sentence has no citation label or reference callout."),
        ("completeness", "passed", "The independent position-claim record compares every proposition in this single-clause sentence."),
        ("directions", "passed", "The sole ordered relation is explicitly compared in above-ring."),
        ("citation_claims", "not_applicable", "No claim is linked to a citation in this source sentence."),
        ("table_text", "not_applicable", "This unit is the actual running-text sentence and contains no table cells."),
        ("conditions", "not_applicable", "The complete statement supplies no condition, restriction or conditional clause."),
        ("antecedents", "not_applicable", "Both objects use explicit noun phrases; no pronoun or backward reference appears."),
        ("modifier_scope", "passed", "Signal and violet have separate modifier records and retain their original attachment."),
    )
    for name, status, reason in checks:
        unit.check(name, status, reason)

    # A real task can checkpoint after this paragraph and continue other units.
    writer.save_draft(draft)
    writer = writer_tools.ReviewWriter.from_draft(draft, progress=None)
    writer.scope("passed", "This invented inventory contains exactly the one requested sentence, which is selected.")
    writer.source_issues_review("passed", "The actual source sentence is readable; its two nouns, modifier attachments and spatial predicate introduce no unresolved source reading.")
    writer.terminology_review("reviewed", covered_units=[UNIT_ID],
                              no_unlisted_reason="Both discovered component terms have context-matched glossary entries; their actual phrase and family queries are recorded.")
    writer.review("reviewed")
    report = writer.save(output, terminology_path=terminology)
    # Independently call the existing validator too, without replacing its gates.
    checked = validator.validate_translation_review(output, inventory_path, candidate)
    if report.to_dict() != checked.to_dict() or not checked.records_ready:
        raise AssertionError(checked.to_dict())
    return {"review_path": output, "terminology_path": terminology, "draft_path": draft,
            "prepared_path": prepared, "inventory_path": inventory_path, "candidate_path": candidate,
            "glossary_path": glossary, "report": checked.to_dict()}
