"""Shared delivery selectors use LF; file and link bindings preserve raw CRLF.

Only generated PDFs and an explicitly authored synthetic Markdown fixture are
used. The same deliveries object reaches both complete-workflow public writers.
"""
from __future__ import annotations

from copy import deepcopy
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-zotero2obsidian", "scripts", "complete_workflow.py")


def load_workflow(path: Path = SCRIPT):
    name = "handoff_delivery_contract_" + sha256(str(path).encode()).hexdigest()[:12]
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def read(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def save(path: Path, value) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def text_hash(text: str) -> str:
    return sha256(text.encode("utf-8")).hexdigest()


class SelectorFixture:
    def __init__(self, base: Path, workflow, eol_mode: str, selector_kind: str):
        self.base, self.workflow = base, workflow
        base.mkdir(parents=True, exist_ok=True)
        self.pdf = base.joinpath("source.pdf")
        specs = [
            ("title", "Synthetic selector contract"),
            ("body", "Line alpha points to detail label.\nLine beta is retained."),
            ("details", "The detail paragraph remains stable."),
        ]
        with fitz.open() as document:
            page = document.new_page(width=500, height=300)
            for y, (_identity, text) in zip([35, 85, 160], specs, strict=True):
                page.insert_text((20, y), text, fontsize=10)
            document.save(self.pdf)
        self.pdf_before = self.pdf.read_bytes()
        self.lines = [
            "# Synthetic selector contract", "", "Line alpha points to [detail",
            "label](<#^detail-section>). Line beta is retained.", "^body", "",
            "The detail paragraph remains stable.", "^detail-section",
        ]
        self.lf_text = "\n".join(self.lines) + "\n"
        self.raw_text = "".join(
            line + ("\r\n" if eol_mode == "crlf" or index % 2 == 0 else "\n")
            for index, line in enumerate(self.lines)
        )
        self.candidate = base.joinpath("output", "candidate.md")
        self.candidate.parent.mkdir()
        self.candidate.write_bytes(self.raw_text.encode("utf-8"))
        self.candidate_before = self.candidate.read_bytes()
        skills = base.joinpath("skills", "synthetic-contract")
        skills.mkdir(parents=True)
        skills.joinpath("SKILL.md").write_text(
            "- [ ] [CON-A01] The synthetic source and candidate identities are bound. <!-- check-type: offline_outcome -->\n",
            encoding="utf-8",
        )
        self.run_path = workflow.initialize_workflow(
            skills_root=skills.parent, work_root=base.joinpath("work"),
            sources={"main": self.pdf}, source_roles={"main": "main"},
            candidate_path=self.candidate, resource_root=self.candidate.parent,
            translation_required=False, progress=lambda *_: None,
        )
        run = read(self.run_path)
        self.inventory_path = Path(run["records"]["source_inventory"])
        self.checks_path = Path(run["records"]["checks"])
        self.map_paths = {name: Path(run["records"][name]) for name in ["source_map", "link_map"]}
        inventory = read(self.inventory_path)
        source = inventory["sources"][0]
        blocks = source["pages"][0]["blocks"]
        assert len(blocks) == len(specs) == 3
        units = []
        for (identity, original), block in zip(specs, blocks, strict=True):
            assert block["text"].strip() == original
            evidence = {"page": 1, "bbox": block["bbox"], "blocks": [block["id"]],
                        "basis": "visual" if identity == "title" else "text", "quote": original}
            unit = {"id": "main-" + identity, "source": "main", "evidence": [evidence]}
            if identity == "title":
                unit.update(kind="metadata", field="title")
            elif identity == "body":
                unit.update(kind="cross_reference", target_unit="main-details", display_text="detail label")
            else:
                unit.update(kind="paragraph")
            units.append(unit)
        inventory["units"] = units
        source["header_fields"] = {
            "title": {"status": "present", "units": ["main-title"]},
            **{field: {"status": "absent_in_source",
                       "reason": "The generated source prints only a title and two short passages, without " + field + ".",
                       "evidence": [{"page": 1, "bbox": [0, 0, 500, 300], "basis": "visual"}]}
               for field in ["authors", "affiliations", "publication_date"]},
        }
        save(self.inventory_path, inventory)
        normalized_body_with_eol = "\n".join(self.lines[2:5]) + "\n"
        if selector_kind == "lines":
            body_target = {"lines": [3, 5], "sha256": text_hash(normalized_body_with_eol)}
        elif selector_kind == "quote":
            body_target = {"quote": "\n".join(self.lines[2:5])}
        elif selector_kind == "anchor":
            body_target = {"anchor": "body"}
        else:
            raise ValueError(selector_kind)
        self.deliveries = [
            {"unit": "main-title", "target": {"quote": self.lines[0]}},
            {"unit": "main-body", "target": body_target},
            {"unit": "main-details", "target": {"anchor": "detail-section"}},
        ]
        tools = workflow._module("check_records")
        workflow.bind_candidate(self.run_path)
        workflow.record_check_batch(self.checks_path, [{
            "id": "CON-A01", "status": "passed",
            "scope": [{"object_id": "main", "locator": {"page": 1}}],
            "evidence": ["source", "candidate"],
            "reason": "The generated PDF and exact candidate bytes have explicit current file bindings.",
        }], evidence={
            "source": tools.bind_evidence(self.pdf, {"kind": "pdf_pages", "start": 1, "end": 1}),
            "candidate": tools.bind_evidence(self.candidate, {"kind": "lines", "start": 1, "end": 8}),
        })
        self.expected_body_start = self.raw_text.index(self.lines[2])
        self.expected_body_end = self.raw_text.index("^body") + len("^body")
        if selector_kind == "lines":
            self.expected_body_end += 2  # Line five is CRLF in both declared fixtures.
        self.expected_link_start = self.raw_text.index("[detail")
        self.expected_link_end = self.raw_text.index(">)", self.expected_link_start) + 2


class HandoffDeliveryContractTests(unittest.TestCase):
    workflow = load_workflow()
    artifact_root: Path | None = None

    def setUp(self):
        if self.artifact_root is None:
            temporary = tempfile.TemporaryDirectory()
            self.addCleanup(temporary.cleanup)
            self.base = Path(temporary.name)
        else:
            self.base = self.artifact_root.joinpath(self._testMethodName)
        self.observation = {}

    def check_contract(self, eol_mode: str, selector_kind: str):
        fixture = SelectorFixture(self.base, self.workflow, eol_mode, selector_kind)
        self.fixture = fixture
        original_deliveries = deepcopy(fixture.deliveries)
        map_report = self.workflow.write_maps(fixture.run_path, fixture.deliveries)
        manifest_path = self.workflow.write_manifest(fixture.run_path, fixture.deliveries)
        delivery_report = self.workflow._module("validate_delivery").validate_delivery(manifest_path)
        self.observation = {
            "eol_mode": eol_mode, "selector_kind": selector_kind,
            "map_report": map_report.as_dict(),
            "delivery_findings": [vars(row) for row in delivery_report.findings],
            "delivery_structurally_consistent": delivery_report.structurally_consistent,
            "candidate_sha256": sha256(fixture.candidate_before).hexdigest(),
            "lf_text_sha256": text_hash(fixture.lf_text),
        }
        self.assertEqual(fixture.deliveries, original_deliveries)
        self.assertEqual(fixture.pdf.read_bytes(), fixture.pdf_before)
        self.assertEqual(fixture.candidate.read_bytes(), fixture.candidate_before)
        self.assertNotEqual(text_hash(fixture.raw_text), text_hash(fixture.lf_text))
        self.assertTrue(delivery_report.structurally_consistent, self.observation)
        self.assertEqual(delivery_report.source_fidelity, "not_assessed")
        self.assertTrue(map_report.records_ready, self.observation)
        self.assertEqual(map_report.source_fidelity, "not_assessed")
        maps = {name: read(path) for name, path in fixture.map_paths.items()}
        manifest = read(manifest_path)
        for binding in [manifest["candidate"], *(value["bindings"]["candidate"] for value in maps.values())]:
            self.assertEqual(binding["path"], str(fixture.candidate.resolve()))
            self.assertEqual(binding["sha256"], sha256(fixture.candidate_before).hexdigest())
        body = next(row for row in maps["source_map"]["units"] if row["id"] == "main-body")
        self.assertEqual(body["target"], original_deliveries[1]["target"])
        self.assertEqual(body["candidate_locator"], {
            "kind": "characters", "unit": "unicode_code_points",
            "start": fixture.expected_body_start, "end": fixture.expected_body_end,
            "sha256": text_hash(fixture.raw_text[fixture.expected_body_start:fixture.expected_body_end]),
        })
        links = maps["link_map"]["internal_links"]
        self.assertEqual(len(links), 1)
        raw_link = fixture.raw_text[fixture.expected_link_start:fixture.expected_link_end]
        self.assertIn("\r\n", raw_link)
        self.assertNotEqual(text_hash(raw_link), text_hash(raw_link.replace("\r\n", "\n")))
        self.assertEqual(links[0]["candidate_locator"], {
            "kind": "characters", "unit": "unicode_code_points",
            "start": fixture.expected_link_start, "end": fixture.expected_link_end,
            "sha256": text_hash(raw_link),
        })
        self.assertEqual(links[0]["origin_unit"], "main-body")
        self.assertEqual(links[0]["target_unit"], "main-details")
        before_validation = {path: path.read_bytes() for path in fixture.map_paths.values()}
        result = self.workflow._module("handoff_maps").validate_handoff_maps(
            fixture.inventory_path, fixture.candidate, fixture.map_paths, progress=None)
        self.assertTrue(result.records_ready, result.as_dict())
        self.assertEqual(before_validation, {path: path.read_bytes() for path in fixture.map_paths.values()})
        self.assertEqual(fixture.pdf.read_bytes(), fixture.pdf_before)
        self.assertEqual(fixture.candidate.read_bytes(), fixture.candidate_before)

    def test_crlf_lines_use_the_shared_lf_hash_contract(self):
        self.check_contract("crlf", "lines")

    def test_crlf_multiline_quote_maps_back_to_raw_characters(self):
        self.check_contract("crlf", "quote")

    def test_crlf_anchor_retains_raw_link_and_file_hashes(self):
        self.check_contract("crlf", "anchor")

    def test_mixed_lines_use_the_shared_lf_hash_contract(self):
        self.check_contract("mixed", "lines")

    def test_mixed_multiline_quote_maps_back_to_raw_characters(self):
        self.check_contract("mixed", "quote")

    def test_mixed_anchor_retains_raw_link_and_file_hashes(self):
        self.check_contract("mixed", "anchor")


if __name__ == "__main__":
    unittest.main()
