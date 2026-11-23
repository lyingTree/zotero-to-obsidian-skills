"""Exercise the complete entry point with generated, source-bound PDF fixtures.

The small skill is a real four-goal input, not a mock validator. Its explicitly
authored evidence is only about the synthetic documents created below. No paper,
model output, review answer, or live application state is used.
"""
from __future__ import annotations

from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-zotero2obsidian", "scripts", "complete_workflow.py")
SPEC = importlib.util.spec_from_file_location("test_complete_workflow_integration_tools", SCRIPT)
assert SPEC and SPEC.loader
workflow = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = workflow
SPEC.loader.exec_module(workflow)


def read(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def save(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class SyntheticCompleteWorkflow:
    """A complete main-plus-SI fixture passed through the real public helpers."""

    def __init__(self, base: Path):
        self.base = base
        self.base.mkdir(parents=True, exist_ok=True)
        self.skills = base.joinpath("skills")
        self.rule_file = self.skills.joinpath("synthetic-completion", "SKILL.md")
        self.rule_file.parent.mkdir(parents=True)
        self.rule_file.write_text(
            "# Synthetic completion rules\n"
            "- [ ] [FLOW-A01] Each source paragraph has its mapped candidate paragraph. <!-- check-type: offline_outcome -->\n"
            "- [ ] [FLOW-A02] Each recorded local link resolves to its declared anchor. <!-- check-type: offline_outcome -->\n"
            "- [ ] [FLOW-A03] If source figures exist, each final image asset is recorded. <!-- check-type: offline_outcome -->\n"
            "- [ ] [FLOW-A04] A historical opening action was observed. <!-- check-type: execution_process -->\n",
            encoding="utf-8",
        )
        self.source_text = {
            "main": ("Synthetic photon study", "A red marker accompanies the reference beam at a range of 10 km."),
            "si": ("Supplementary fixture", "A blue marker identifies the calibration beam."),
        }
        self.sources = {}
        for identity, (title, paragraph) in self.source_text.items():
            path = base.joinpath("input", identity + ".pdf")
            path.parent.mkdir(parents=True, exist_ok=True)
            with fitz.open() as document:
                page = document.new_page(width=400, height=220)
                page.insert_text((20, 35), title)
                page.insert_text((20, 80), paragraph)
                document.save(path)
            self.sources[identity] = path
        self.source_initial_hashes = {k: digest(v) for k, v in self.sources.items()}
        self.output = base.joinpath("output")
        self.output.mkdir()
        self.candidate = self.output.joinpath("candidate.md")
        self.candidate.write_text(
            "---\n类型: 期刊论文\n年份: 2026\n关键词: []\n期刊名: Synthetic fixture\n---\n"
            "# Synthetic photon study\n\n"
            "## 1 正文\n"
            "A red marker accompanies the reference beam at a range of $10\\,\\text{km}$.\n^main-paragraph\n\n"
            "## 2 补充材料\n\n"
            "### 2.1 Supplementary fixture\n"
            "A blue marker identifies the calibration beam.\n^si-paragraph\n",
            encoding="utf-8",
        )
        candidate_line_count = len(self.candidate.read_text(encoding="utf-8").splitlines())
        self.candidate_initial_hash = digest(self.candidate)
        self.work = base.joinpath("work")
        self.run_path = workflow.initialize_workflow(
            skills_root=self.skills, work_root=self.work,
            sources=self.sources, source_roles={"main": "main", "si": "supplement"},
            candidate_path=self.candidate, resource_root=self.output,
            translation_required=False, progress=lambda *_: None,
        )
        self.run = read(self.run_path)
        self.checks_path = Path(self.run["records"]["checks"])
        self.inventory_path = Path(self.run["records"]["source_inventory"])
        self.manifest_path = Path(self.run["records"]["delivery_manifest"])
        self.check_tools = workflow._module("check_records")
        self.inventory = read(self.inventory_path)
        self.deliveries = []
        self.block_records = []
        self.inventory["units"] = []
        for source in self.inventory["sources"]:
            identity = source["id"]
            title, paragraph = self.source_text[identity]
            blocks = source["pages"][0]["blocks"]
            title_block = next(b for b in blocks if b["text"].strip() == title)
            paragraph_block = next(b for b in blocks if b["text"].strip() == paragraph)
            all_page = {"page": 1, "bbox": [0, 0, 400, 220], "basis": "visual"}
            source["header_fields"] = {
                "title": {"status": "present", "units": [identity + "-title"]},
                **{name: {
                    "status": "absent_in_source",
                    "reason": "This generated source contains only its title and one paragraph; no " + name + " are printed.",
                    "evidence": [all_page],
                } for name in ["authors", "affiliations", "publication_date"]},
            }
            self.inventory["units"].extend([
                {"id": identity + "-title", "source": identity, "kind": "metadata", "field": "title",
                 "evidence": [{**all_page, "blocks": [title_block["id"]], "quote": title}]},
                {"id": identity + "-paragraph", "source": identity, "kind": "paragraph",
                 "evidence": [{"page": 1, "basis": "text", "blocks": [paragraph_block["id"]], "quote": paragraph}]},
            ])
            self.deliveries.extend([
                {"unit": identity + "-title", "target": {"quote": title}},
                {"unit": identity + "-paragraph", "target": {"anchor": identity + "-paragraph"}},
            ])
            for order, block in enumerate([title_block, paragraph_block]):
                self.block_records.append({
                    "block_id": block["id"], "source_id": identity,
                    "source_attachment": self.check_tools.bind_file(self.sources[identity]),
                    "source_locator": {"page": 1, "bbox": block["bbox"]},
                    "order": order, "kind": "title" if order == 0 else "paragraph",
                    "original_sha256": hashlib.sha256(block["text"].encode()).hexdigest(),
                    "decision": "keep", "reason": "This exact generated block is included in the candidate.",
                    "target_unit": identity + ("-title" if order == 0 else "-paragraph"),
                })
        save(self.inventory_path, self.inventory)
        file_bindings = {
            "source_inventory": self.check_tools.bind_file(self.inventory_path),
            "candidate": self.check_tools.bind_file(self.candidate),
            "sources": {k: self.check_tools.bind_file(v) for k, v in self.sources.items()},
        }
        save(Path(self.run["records"]["source_map"]), {
            "schema_version": 1, "bindings": file_bindings,
            "blocks": [r for r in self.block_records if r["source_id"] == "main"],
            "objects": [], "references": [], "citations": [],
        })
        save(Path(self.run["records"]["si_map"]), {
            "schema_version": 1, "bindings": file_bindings,
            "sources": [{"id": "si", "role": "supplement", **self.check_tools.bind_file(self.sources["si"])}],
            "blocks": [r for r in self.block_records if r["source_id"] == "si"], "objects": [],
        })
        save(Path(self.run["records"]["link_map"]), {
            "schema_version": 1, "bindings": file_bindings,
            "anchors": [
                {"id": "main-paragraph", "source_id": "main", "target_id": "main-paragraph", "status": "checked"},
                {"id": "si-paragraph", "source_id": "si", "target_id": "si-paragraph", "status": "checked"},
            ],
            "internal_links": [], "library_matches": [], "unresolved": [],
        })
        self.run["naming"] = {
            "status": "resolved", "original_title": self.source_text["main"][0],
            "author_token": "UnknownAuthor", "author_reason": "No author appears in either synthetic PDF.",
            "year": None, "year_reason": "The synthetic source does not print a publication year.",
            "filename": "UnknownAuthor-无年份-Synthetic photon study.md",
            "transformations": [], "conflicts": [], "conflict_scope": "offline_plan_only",
        }
        save(self.run_path, self.run)
        workflow.bind_candidate(self.run_path)
        evidence = {
            "candidate": self.check_tools.bind_evidence(self.candidate, {"kind": "lines", "start": 7, "end": candidate_line_count}),
            "main-source": self.check_tools.bind_evidence(self.sources["main"], {"kind": "pdf_pages", "start": 1, "end": 1}),
            "si-source": self.check_tools.bind_evidence(self.sources["si"], {"kind": "pdf_pages", "start": 1, "end": 1}),
        }
        workflow.record_check_batch(self.checks_path, [
            {"id": "FLOW-A01", "status": "passed",
             "scope": [{"object_id": "main-paragraph", "locator": {"page": 1}},
                       {"object_id": "si-paragraph", "locator": {"page": 1}}],
             "evidence": ["candidate", "main-source", "si-source"],
             "reason": "The red-marker and blue-marker source paragraphs are separately present at the two exact candidate anchors."},
            {"id": "FLOW-A02", "status": "not_applicable",
             "scope": [{"object_id": "candidate-internal-links", "locator": {"start": 7, "end": candidate_line_count}}],
             "evidence": ["candidate"], "reason": "The candidate has two anchor definitions and no local link calls.",
             "applicability_condition": "The generated candidate contains no local link calls requiring resolution."},
            {"id": "FLOW-A03", "status": "not_applicable",
             "scope": [{"object_id": "main:page1", "locator": {"page": 1}},
                       {"object_id": "si:page1", "locator": {"page": 1}}],
             "evidence": ["main-source", "si-source"], "reason": "Both generated PDFs contain two text blocks and no figures or image resources.",
             "applicability_condition": "Neither synthetic source has a figure requiring a final image asset."},
            {"id": "FLOW-A04", "status": "not_verified",
             "scope": [{"object_id": "historical-open", "locator": {"source": "main", "page": 1}}],
             "evidence": [], "reason": "This test does not provide a historical executor opening event."},
        ], evidence=evidence)
        workflow.write_manifest(self.run_path, self.deliveries)

    def inspect(self):
        return workflow.inspect_workflow(self.run_path)

    def refresh_manifest(self):
        return workflow.write_manifest(self.run_path, self.deliveries)

    def edit_check(self, identity: str, **changes):
        ledger = read(self.checks_path)
        next(row for row in ledger["checks"] if row["id"] == identity).update(changes)
        save(self.checks_path, ledger)
        self.refresh_manifest()

    def current_file_hashes(self):
        return {str(path): digest(path) for path in self.base.rglob("*")
                if path.is_file() and "__pycache__" not in path.parts}


class CompleteWorkflowIntegrationTests(unittest.TestCase):
    artifact_root: Path | None = None

    def setUp(self):
        if self.artifact_root is None:
            temporary = tempfile.TemporaryDirectory()
            self.addCleanup(temporary.cleanup)
            base = Path(temporary.name)
        else:
            base = self.artifact_root.joinpath(self._testMethodName)
        self.fixture = SyntheticCompleteWorkflow(base)
        result = self.fixture.inspect()
        self.assertEqual(result.findings, [], msg=result.as_dict())
        self.assertTrue(result.records_ready)

    def assert_rejected(self, report):
        self.assertFalse(report.records_ready, msg=report.as_dict())
        self.assertTrue(report.findings)
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_complete_main_and_si_records_are_ready_without_semantic_certification(self):
        f = self.fixture
        report = f.inspect()
        self.assertEqual(report.findings, [])
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertIn("do not establish source fidelity", report.as_dict()["boundary"])
        ledger = read(f.checks_path)
        self.assertEqual([r["status"] for r in ledger["checks"]],
                         ["passed", "not_applicable", "not_applicable", "not_verified"])
        self.assertEqual(f.source_initial_hashes, {k: digest(p) for k, p in f.sources.items()})
        self.assertEqual(f.candidate_initial_hash, digest(f.candidate))

    def test_inspection_is_read_only_even_when_the_result_is_ready(self):
        f = self.fixture
        before = f.current_file_hashes()
        self.assertTrue(f.inspect().records_ready)
        self.assertEqual(before, f.current_file_hashes())

    def test_every_required_record_cannot_be_omitted_as_a_whole(self):
        f = self.fixture
        run_before = read(f.run_path)
        for name in ["source_inventory", "source_map", "si_map", "link_map", "checks", "delivery_manifest"]:
            with self.subTest(name=name):
                run = deepcopy(run_before)
                del run["records"][name]
                save(f.run_path, run)
                report = f.inspect()
                self.assert_rejected(report)
                self.assertTrue(any(x.code == "WORKFLOW_RECORD_MISSING" and x.target == name for x in report.findings))
        save(f.run_path, run_before)

    def test_empty_and_nonobject_whole_records_are_rejected(self):
        f = self.fixture
        path = Path(f.run["records"]["source_map"])
        for value in [{}, [], None, "a record was completed"]:
            with self.subTest(value=value):
                save(path, value)
                self.assert_rejected(f.inspect())

    def test_fake_collection_elements_are_not_completed_mapping_records(self):
        f = self.fixture
        save(Path(f.run["records"]["source_map"]), {
            "blocks": [None], "objects": {"invented": False},
            "references": ["not a reference record"], "citations": [17],
        })
        self.assert_rejected(f.inspect())

    def test_empty_maps_cannot_cover_known_source_blocks_and_candidate_anchors(self):
        f = self.fixture
        save(Path(f.run["records"]["source_map"]), {"blocks": [], "objects": [], "references": [], "citations": []})
        save(Path(f.run["records"]["si_map"]), {"sources": [], "blocks": [], "objects": []})
        save(Path(f.run["records"]["link_map"]), {"anchors": [], "internal_links": [], "library_matches": [], "unresolved": []})
        self.assert_rejected(f.inspect())

    def test_mapping_bound_to_another_candidate_is_rejected(self):
        f = self.fixture
        other = f.output.joinpath("another.md")
        other.write_text("# Another complete-looking candidate\n", encoding="utf-8")
        path = Path(f.run["records"]["source_map"])
        data = read(path)
        data["bindings"]["candidate"] = f.check_tools.bind_file(other)
        save(path, data)
        self.assert_rejected(f.inspect())

    def test_mapping_with_an_old_candidate_hash_is_rejected(self):
        f = self.fixture
        path = Path(f.run["records"]["si_map"])
        data = read(path)
        data["bindings"]["candidate"]["sha256"] = "0" * 64
        save(path, data)
        self.assert_rejected(f.inspect())

    def test_manifest_bound_to_another_candidate_is_rejected(self):
        f = self.fixture
        other = f.output.joinpath("other-current.md")
        other.write_bytes(f.candidate.read_bytes())
        manifest = read(f.manifest_path)
        manifest["candidate"] = f.check_tools.bind_file(other)
        save(f.manifest_path, manifest)
        self.assert_rejected(f.inspect())

    def test_candidate_changed_after_binding_is_rejected_without_writing_back(self):
        f = self.fixture
        f.candidate.write_text(f.candidate.read_text(encoding="utf-8") + "\nA later candidate change.\n", encoding="utf-8")
        before = f.current_file_hashes()
        self.assert_rejected(f.inspect())
        self.assertEqual(before, f.current_file_hashes())

    def test_main_block_mislabeled_as_si_is_rejected(self):
        f = self.fixture
        path = Path(f.run["records"]["source_map"])
        data = read(path)
        data["blocks"][0]["source_id"] = "si"
        data["blocks"][0]["source_attachment"] = f.check_tools.bind_file(f.sources["si"])
        save(path, data)
        self.assert_rejected(f.inspect())

    def test_original_run_scope_prevents_removing_a_whole_si_source(self):
        f = self.fixture
        inventory = read(f.inventory_path)
        inventory["sources"] = [s for s in inventory["sources"] if s["id"] == "main"]
        inventory["units"] = [u for u in inventory["units"] if u["source"] == "main"]
        save(f.inventory_path, inventory)
        ledger = read(f.checks_path)
        ledger["bindings"]["sources"] = [s for s in ledger["bindings"]["sources"] if s["id"] == "main"]
        save(f.checks_path, ledger)
        f.deliveries = [d for d in f.deliveries if d["unit"].startswith("main-")]
        f.refresh_manifest()
        run = read(f.run_path)
        del run["records"]["si_map"]
        save(f.run_path, run)
        self.assertEqual(run["scope"]["source_ids"], ["main", "si"])
        self.assert_rejected(f.inspect())

    def test_another_skills_root_cannot_shrink_the_acceptance_catalog(self):
        f = self.fixture
        alternate = f.base.joinpath("alternate-skills")
        skill = alternate.joinpath("one-goal", "SKILL.md")
        skill.parent.mkdir(parents=True)
        skill.write_text("- [ ] [ALT-A01] One unrelated goal is recorded. <!-- check-type: offline_outcome -->\n", encoding="utf-8")
        replacement = f.work.joinpath("replacement-checks.json")
        f.check_tools.prepare_check_records(alternate, replacement, sources=f.sources,
                                           candidates={"candidate": f.candidate}, progress=None)
        workflow.record_check_batch(replacement, [{
            "id": "ALT-A01", "status": "passed", "scope": [{"object_id": "candidate", "locator": {"line": 7}}],
            "evidence": ["text"], "reason": "The unrelated goal has a file-bound record.",
        }], evidence={"text": f.check_tools.bind_evidence(f.candidate, {"kind": "lines", "start": 7, "end": 7})})
        f.checks_path.write_bytes(replacement.read_bytes())
        f.refresh_manifest()
        self.assert_rejected(f.inspect())

    def test_skill_changed_after_initialization_is_rejected(self):
        f = self.fixture
        f.rule_file.write_text(f.rule_file.read_text() + "\nAdditional input text.\n", encoding="utf-8")
        self.assert_rejected(f.inspect())

    def test_missing_atomic_goal_is_rejected(self):
        f = self.fixture
        ledger = read(f.checks_path)
        ledger["checks"] = [r for r in ledger["checks"] if r["id"] != "FLOW-A01"]
        save(f.checks_path, ledger)
        f.refresh_manifest()
        self.assert_rejected(f.inspect())

    def test_offline_goal_cannot_be_hidden_as_a_process_goal(self):
        f = self.fixture
        f.edit_check("FLOW-A01", requirement_type="execution_process", status="not_verified")
        self.assert_rejected(f.inspect())

    def test_not_applicable_requires_a_specific_condition(self):
        f = self.fixture
        f.edit_check("FLOW-A03", applicability_condition="")
        self.assert_rejected(f.inspect())

    def test_not_applicable_cannot_discard_its_evidence_after_batch_recording(self):
        f = self.fixture
        f.edit_check("FLOW-A03", evidence=[])
        self.assert_rejected(f.inspect())

    def test_static_evidence_does_not_resolve_an_unobserved_process(self):
        f = self.fixture
        f.edit_check("FLOW-A04", status="passed", evidence=["candidate"],
                     reason="Only final text exists; it is not a directly observed opening event.")
        self.assert_rejected(f.inspect())

    def test_unbound_presentation_disposition_cannot_clear_a_current_prompt(self):
        f = self.fixture
        text = f.candidate.read_text(encoding="utf-8")
        f.candidate.write_text(text.replace("$10\\,\\text{km}$", "10 km"), encoding="utf-8")
        # Deliberately re-review this fixture revision so only the presentation
        # disposition's missing candidate binding remains at issue.
        ledger = read(f.checks_path)
        ledger["bindings"]["candidates"][0].update(f.check_tools.bind_file(f.candidate))
        ledger["evidence"]["candidate"].update(f.check_tools.bind_file(f.candidate))
        save(f.checks_path, ledger)
        for name in ["source_map", "si_map", "link_map"]:
            path = Path(f.run["records"][name])
            data = read(path)
            data["bindings"]["candidate"] = f.check_tools.bind_file(f.candidate)
            save(path, data)
        f.refresh_manifest()
        prompts = workflow._module("lint_presentation").lint_file(f.candidate)
        self.assertTrue(prompts, "The synthetic unit must produce a real current lint prompt.")
        disposition = f.work.joinpath("presentation-review.json")
        save(disposition, {"items": [{"code": p.code, "line": p.line,
                                     "status": "not_an_issue", "reason": "This reason was copied from an unbound old review."}
                                    for p in prompts]})
        run = read(f.run_path)
        run["records"]["presentation_review"] = str(disposition)
        save(f.run_path, run)
        self.assert_rejected(f.inspect())


if __name__ == "__main__":
    unittest.main()
