"""A complete task cannot omit whole ledgers to obtain an empty diagnostic."""
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
SPEC = importlib.util.spec_from_file_location("test_complete_workflow_tools", SCRIPT)
assert SPEC and SPEC.loader
workflow = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = workflow
SPEC.loader.exec_module(workflow)


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")


class CompleteWorkflowTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.skills = self.base.joinpath("skills")
        skill = self.skills.joinpath("synthetic-skill")
        skill.mkdir(parents=True)
        skill.joinpath("SKILL.md").write_text(
            "# Synthetic\n"
            "- [ ] [TEST-A01] A source sentence has a target. <!-- check-type: offline_outcome -->\n"
            "- [ ] [TEST-A02] A figure condition has evidence. <!-- check-type: offline_outcome -->\n"
            "- [ ] [TEST-A03] A historical action was observed. <!-- check-type: execution_process -->\n",
            encoding="utf-8",
        )
        self.source = self.base.joinpath("source.pdf")
        with fitz.open() as document:
            page = document.new_page(width=220, height=160)
            page.insert_text((20, 40), "Synthetic optical apparatus.")
            document.save(self.source)
        self.work = self.base.joinpath("work")
        self.output = self.base.joinpath("output")
        self.output.mkdir()
        self.candidate = self.output.joinpath("candidate.md")
        self.run_path = workflow.initialize_workflow(
            skills_root=self.skills, work_root=self.work,
            sources={"main": self.source}, source_roles={"main": "main"},
            candidate_path=self.candidate, resource_root=self.output,
            progress=lambda *_: None,
        )
        self.checks_path = self.work.joinpath("checks.json")
        self.check_tools = workflow._module("check_records")

    def candidate_file(self):
        self.candidate.write_text("# Synthetic\n\nSynthetic optical apparatus.\n", encoding="utf-8")

    def result(self, identity="TEST-A01", **changes):
        value = {
            "id": identity, "status": "passed",
            "scope": [{"object_id": "main:page1", "locator": {"page": 1}}],
            "evidence": ["candidate-text"], "reason": "This synthetic sentence matches the specified target.",
        }
        value.update(changes)
        return value

    def evidence(self):
        self.candidate_file()
        return {"candidate-text": self.check_tools.bind_evidence(
            self.candidate, {"kind": "lines", "start": 1, "end": 3})}

    def test_initialization_retains_all_pending_goals(self):
        ledger = read(self.checks_path)
        self.assertEqual([x["status"] for x in ledger["checks"]], ["not_verified"] * 3)
        self.assertEqual(ledger["bindings"]["candidates"], [])
        run = read(self.run_path)
        self.assertEqual(run["scope"]["acceptance"], "markdown_and_referenced_resources")
        self.assertIn("translation_review", run["records"])
        self.assertNotIn("si_map", run["records"])
        self.assertEqual(ledger["bindings"]["sources"][0]["sha256"], hashlib.sha256(self.source.read_bytes()).hexdigest())

    def test_reinitialization_does_not_replace_existing_records(self):
        before = self.checks_path.read_bytes()
        with self.assertRaises(FileExistsError):
            workflow.initialize_workflow(
                skills_root=self.skills, work_root=self.work,
                sources={"main": self.source}, source_roles={"main": "main"},
                candidate_path=self.candidate, resource_root=self.output,
            )
        self.assertEqual(self.checks_path.read_bytes(), before)

    def test_whole_missing_ledger_is_reported(self):
        self.checks_path.unlink()
        report = workflow.inspect_workflow(self.run_path)
        self.assertFalse(report.records_ready)
        self.assertTrue(any(x.code == "WORKFLOW_RECORD_MISSING" and x.target == "checks" for x in report.findings))
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_missing_plan_is_not_hidden_by_fixed_candidate_name(self):
        self.candidate_file()
        report = workflow.inspect_workflow(self.run_path)
        self.assertTrue(any(x.code == "WORKFLOW_NAMING_MISSING" for x in report.findings))

    def test_pending_batch_preserves_individual_file_goals(self):
        batch = workflow.pending_check_batch(self.checks_path, limit=1)
        self.assertEqual([x["id"] for x in batch], ["TEST-A01"])
        self.assertEqual(len(workflow.pending_check_batch(self.checks_path)), 2)
        with self.assertRaises(ValueError):
            workflow.pending_check_batch(self.checks_path, limit=0)

    def test_explicit_batch_does_not_resolve_other_goals(self):
        workflow.record_check_batch(self.checks_path, [self.result()], evidence=self.evidence())
        rows = read(self.checks_path)["checks"]
        self.assertEqual([x["status"] for x in rows], ["passed", "not_verified", "not_verified"])
        self.assertEqual(read(self.checks_path)["updates"][0]["previous"][0]["status"], "not_verified")

    def test_unknown_goal_is_rejected_without_mutation(self):
        before = self.checks_path.read_bytes()
        with self.assertRaises(ValueError):
            workflow.record_check_batch(self.checks_path, [self.result("TEST-A99")], evidence=self.evidence())
        self.assertEqual(self.checks_path.read_bytes(), before)

    def test_duplicate_goal_is_rejected(self):
        with self.assertRaises(ValueError):
            workflow.record_check_batch(self.checks_path, [self.result(), self.result()], evidence=self.evidence())

    def test_type_override_is_rejected(self):
        with self.assertRaises(ValueError):
            workflow.record_check_batch(self.checks_path, [self.result(requirement_type="execution_process")], evidence=self.evidence())

    def test_pass_needs_explicit_scope(self):
        with self.assertRaises(ValueError):
            workflow.record_check_batch(self.checks_path, [self.result(scope=[])], evidence=self.evidence())

    def test_inapplicable_needs_condition(self):
        with self.assertRaises(ValueError):
            workflow.record_check_batch(self.checks_path, [self.result(status="not_applicable")], evidence=self.evidence())

    def test_changed_evidence_cannot_be_bound_with_an_old_hash(self):
        evidence = self.evidence()
        self.candidate.write_text("Changed candidate.\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            workflow.record_check_batch(self.checks_path, [self.result()], evidence=evidence)

    def test_changed_evidence_id_is_not_reused(self):
        evidence = self.evidence()
        workflow.record_check_batch(self.checks_path, [self.result()], evidence=evidence)
        changed = deepcopy(evidence)
        changed["candidate-text"]["locator"] = {"kind": "lines", "start": 1, "end": 1}
        with self.assertRaises(ValueError):
            workflow.record_check_batch(self.checks_path, [self.result("TEST-A02")], evidence=changed)

    def test_candidate_rebinding_rejects_an_unreviewed_revision(self):
        self.candidate_file()
        first = workflow.bind_candidate(self.run_path)
        self.assertEqual(workflow.bind_candidate(self.run_path), first)
        self.candidate.write_text("A revised file.\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            workflow.bind_candidate(self.run_path)
        self.assertEqual(read(self.checks_path)["bindings"]["candidates"][0]["sha256"], first["sha256"])

    def test_translation_record_cannot_be_omitted_at_manifest_write(self):
        self.candidate_file()
        with self.assertRaises(ValueError):
            workflow.write_manifest(self.run_path, [])
        self.assertFalse(self.work.joinpath("delivery-manifest.json").exists())


if __name__ == "__main__":
    unittest.main()
