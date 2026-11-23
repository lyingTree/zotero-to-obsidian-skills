"""Synthetic script example: record one checked fact, keep all other goals pending.

Call demonstrate(Path("an unused example directory")) from a Python script.
The two sentences below are invented for this example, not taken from a paper.
The explicit judgments apply only to this pair and must not be copied to a task.
"""
from __future__ import annotations

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile

import fitz


SKILL = Path(__file__).resolve().parents[1]


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, SKILL.joinpath("scripts", filename))
    if spec is None or spec.loader is None:
        raise ImportError(filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


prepare = _load("example_translation_prepare", "prepare_translation_review.py")
validate = _load("example_translation_validate", "validate_translation_review.py")


def _write_new(path: Path, value) -> None:
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def demonstrate(work_root: Path) -> dict:
    work = Path(work_root).resolve()
    work.mkdir(parents=True, exist_ok=False)
    source = work.joinpath("fictional-source.pdf")
    candidate = work.joinpath("fictional-translation.md")
    glossary = work.joinpath("fictional-glossary.md")
    source_sentence = "The blue tile lies inward of the amber rim."
    candidate_sentence = "蓝色方片位于琥珀色边框的内侧。"
    glossary.write_text("| example-inward | inward of | 位于……内侧 |\n", encoding="utf-8")
    glossary_binding = prepare.bind_review_file(glossary)
    with fitz.open() as document:
        page = document.new_page(width=480, height=160)
        page.insert_text((24, 48), source_sentence)
        document.save(source)
    with fitz.open(source) as document:
        block = document[0].get_text("blocks")[0]
    candidate.write_text(candidate_sentence + "\n", encoding="utf-8")
    inventory = {
        "schema_version": 1,
        "sources": [{"id": "fictional-source", **prepare.bind_review_file(source), "page_count": 1,
                     "pages": [{"page": 1, "blocks": [{"id": "fictional-block", "bbox": list(block[:4]), "text": block[4]}]}]}],
        "units": [{"id": "fictional-paragraph", "source": "fictional-source", "kind": "paragraph",
                   "evidence": [{"page": 1, "basis": "text_blocks", "blocks": ["fictional-block"]}]}],
    }
    inventory_path = work.joinpath("source-inventory.json")
    _write_new(inventory_path, inventory)
    pending_path = prepare.prepare_translation_review(
        inventory_path, candidate, work.joinpath("translation-review.pending.json"),
        unit_ids=["fictional-paragraph"], glossary_path=glossary,
        candidate_locations={"fictional-paragraph": [{"lines": [1, 1]}]}, progress=None)
    ledger = json.loads(pending_path.read_text(encoding="utf-8"))
    assert ledger["bindings"]["glossary"] == glossary_binding

    # These two invented sentences were explicitly compared: blue tile is the
    # subject, amber rim the reference, and inward of is preserved as 内侧.
    evidence = [prepare.bind_review_evidence(source, {"page": 1, "quote": source_sentence}),
                prepare.bind_review_evidence(candidate, {"lines": [1, 1], "quote": candidate_sentence})]
    occurrence = prepare.new_occurrence("direction-one", "directions",
                                        source_span={"page": 1, "quote": source_sentence})
    occurrence.update({
        "candidate_span": {"lines": [1, 1], "quote": candidate_sentence},
        "source_relation": {"subject": "blue tile", "reference": "amber rim", "relation": "inward of"},
        "candidate_relation": {"subject": "蓝色方片", "reference": "琥珀色边框", "relation": "位于……内侧"},
        "status": "passed", "reason": "In this invented pair, the subject, reference and inward relation are preserved.",
        "evidence": evidence,
    })
    unit = ledger["units"]["fictional-paragraph"]
    unit["occurrences"]["directions"] = [occurrence]
    unit["checks"]["directions"] = {
        "status": "passed", "reason": "The one spatial relation in this example was explicitly compared.",
        "evidence": deepcopy(evidence),
    }
    reviewed_path = work.joinpath("translation-review.partial.json")
    _write_new(reviewed_path, ledger)

    # This is an illustration of two explicit atomic rows, not a replacement
    # checks.json. Bind this fact in the real atomic ledger after its own scope
    # is established; all other IDs require their own decisions.
    fact = prepare.bind_review_evidence(reviewed_path, {
        "kind": "json_pointer", "pointer": "/units/fictional-paragraph/occurrences/directions/0"})
    atomic_links = {
        "evidence": {"direction-fact": fact},
        "checks": [
            {"id": "TRN-C68", "object_id": "fictional-paragraph", "status": "passed",
             "reason": "The single subject/reference/direction relation was translated according to the invented source.",
             "evidence": ["direction-fact"]},
            {"id": "TRN-A14", "object_id": "fictional-paragraph", "status": "passed",
             "reason": "The explicitly compared direction occurrence matches the invented source relation.",
             "evidence": ["direction-fact"]},
        ],
        "boundary": "Illustrative fragment only; do not mark any other atomic ID passed from these two rows.",
    }
    _write_new(work.joinpath("example-atomic-links.json"), atomic_links)
    report = validate.validate_translation_review(reviewed_path, inventory_path, candidate).to_dict()
    _write_new(work.joinpath("translation-record-validation.json"), report)
    assert not report["records_ready"]  # Other checks, terms and scope remain pending.
    assert report["source_fidelity"] == "not_assessed"
    return {"review_path": str(reviewed_path), "report": report}


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as temporary:
        result = demonstrate(Path(temporary).joinpath("example"))
        print(json.dumps({key: result["report"][key] for key in
                          ("records_ready", "source_fidelity", "pending_ids")}, ensure_ascii=False, indent=2))
