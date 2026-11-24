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

    def test_compact_rows_preserve_separate_decisions_and_unselected_goals(self):
        evidence = self.evidence()
        contexts = {"source-sentence": {
            "scope": [{"object_id": "source:page1", "locator": {"page": 1}}],
            "evidence": ["candidate-text"],
        }}
        before = read(self.checks_path)
        workflow.record_decision_rows(self.checks_path, [
            ("TEST-A01", "passed", "source-sentence", "The actual source sentence is present."),
            ("TEST-A02", "not_applicable", "source-sentence", "This source page contains text only.",
             "The complete synthetic source has no figure."),
        ], contexts=contexts, evidence=evidence)
        after = read(self.checks_path)
        self.assertEqual([row["status"] for row in after["checks"]],
                         ["passed", "not_applicable", "not_verified"])
        for old, new in zip(before["checks"], after["checks"], strict=True):
            self.assertEqual(new["objective"], old["objective"])
            self.assertEqual(new["requirement_type"], old["requirement_type"])
        self.assertEqual(after["checks"][1]["applicability_condition"],
                         "The complete synthetic source has no figure.")
        self.assertEqual(len(after["updates"][-1]["previous"]), 2)

    def test_compact_authoring_rejects_implicit_decisions_without_changing_ledger(self):
        evidence = self.evidence()
        contexts = {"text": {"scope": [{"object_id": "line1", "locator": {"lines": [1, 1]}}],
                             "evidence": ["candidate-text"]}}
        valid = ("TEST-A01", "passed", "text", "This sentence is present.")
        invalid_rows = [
            [valid[:3]],
            [("TEST-A01", "not_applicable", "text", "No object.")],
            [(*valid, "Condition on a non-NA result")],
            [("TEST-A01", "passed", "missing-context", "Compared.")],
            [("TEST-A01", "passed", "text", " ")],
            [valid, valid],
            [("UNKNOWN-A01", "passed", "text", "Compared.")],
        ]
        before = self.checks_path.read_bytes()
        for rows in invalid_rows:
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                workflow.record_decision_rows(self.checks_path, rows, contexts=contexts, evidence=evidence)
            self.assertEqual(self.checks_path.read_bytes(), before)
        for context in ({**contexts["text"], "status": "passed"},
                        {"scope": [], "evidence": ["candidate-text"]}):
            with self.subTest(context=context), self.assertRaises(ValueError):
                workflow.record_decision_rows(self.checks_path, [valid], contexts={"text": context}, evidence=evidence)
            self.assertEqual(self.checks_path.read_bytes(), before)

    def test_compact_pending_view_preserves_exact_goals(self):
        rows = workflow.pending_check_rows(self.checks_path, limit=1)
        self.assertEqual(rows, [["TEST-A01", "A source sentence has a target."]])
        all_rows = workflow.pending_check_rows(self.checks_path)
        self.assertEqual([row[0] for row in all_rows], ["TEST-A01", "TEST-A02"])
        self.assertEqual([row["status"] for row in read(self.checks_path)["checks"]],
                         ["not_verified"] * 3)

    def test_reinitialization_does_not_replace_existing_records(self):
        before = self.checks_path.read_bytes()
        with self.assertRaises(FileExistsError):
            workflow.initialize_workflow(
                skills_root=self.skills, work_root=self.work,
                sources={"main": self.source}, source_roles={"main": "main"},
                candidate_path=self.candidate, resource_root=self.output,
            )
        self.assertEqual(self.checks_path.read_bytes(), before)

    def test_glossary_leads_are_prepared_without_filling_acceptance_decisions(self):
        glossary = self.base.joinpath("source-glossary.md")
        glossary.write_text("| T0201 | optical axis | 光轴 | Geometry, not every optical object. | synthetic |\n", encoding="utf-8")
        run_path = workflow.initialize_workflow(
            skills_root=self.skills, work_root=self.base.joinpath("with-glossary"),
            sources={"main": self.source}, source_roles={"main": "main"},
            candidate_path=self.candidate, resource_root=self.output,
            glossary_path=glossary, progress=lambda *_: None,
        )
        run = read(run_path)
        leads = read(Path(run["records"]["glossary_candidates"]))
        self.assertEqual(leads["blocks"][0]["matches"][0]["entry"]["id"], "T0201")
        self.assertEqual(leads["blocks"][0]["matches"][0]["decision"], "pending")
        self.assertEqual([row["status"] for row in read(Path(run["records"]["checks"]))["checks"]], ["not_verified"] * 3)

    def test_legacy_glossary_formats_initialize_complete_pending_workflows(self):
        for number, text in enumerate((
            "| sample-optical | optical axis | 光轴 |\n",
            "ID | English | Chinese\n--- | --- | ---\nsample-optical | optical axis | 光轴\n",
        )):
            with self.subTest(glossary_format=number):
                glossary = self.base.joinpath(f"legacy-glossary-{number}.md")
                glossary.write_text(text, encoding="utf-8")
                before = glossary.read_bytes()
                run_path = workflow.initialize_workflow(
                    skills_root=self.skills, work_root=self.base.joinpath(f"legacy-run-{number}"),
                    sources={"main": self.source}, source_roles={"main": "main"},
                    candidate_path=self.candidate, resource_root=self.output,
                    glossary_path=glossary, progress=lambda *_: None,
                )
                run = read(run_path)
                leads = read(Path(run["records"]["glossary_candidates"]))
                self.assertEqual(leads["blocks"][0]["matches"][0]["entry"]["id"], "sample-optical")
                self.assertEqual(leads["blocks"][0]["matches"][0]["decision"], "pending")
                self.assertEqual(read(Path(run["records"]["terminology"]))["review_status"], "pending")
                self.assertEqual([row["status"] for row in read(Path(run["records"]["checks"]))["checks"]], ["not_verified"] * 3)
                self.assertEqual(glossary.read_bytes(), before)

    def test_whole_missing_ledger_is_reported(self):
        self.checks_path.unlink()
        report = workflow.inspect_workflow(self.run_path)
        self.assertFalse(report.records_ready)
        self.assertTrue(any(x.code == "WORKFLOW_RECORD_MISSING" and x.target == "checks" for x in report.findings))
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_final_report_gate_requires_the_existing_local_records(self):
        self.candidate_file()
        before = self.checks_path.read_bytes()
        with self.assertRaises(workflow.IncompleteRecordsError) as caught:
            workflow.require_completed_records(self.run_path)
        self.assertFalse(caught.exception.report.records_ready)
        self.assertEqual(caught.exception.report.source_fidelity, "not_assessed")
        self.assertIn("continue the listed work", str(caught.exception))
        self.assertEqual(self.checks_path.read_bytes(), before)

    def test_missing_plan_is_not_hidden_by_fixed_candidate_name(self):
        self.candidate_file()
        report = workflow.inspect_workflow(self.run_path)
        self.assertTrue(any(x.code == "WORKFLOW_NAMING_MISSING" for x in report.findings))

    def test_filename_alone_does_not_hide_the_missing_source_title_mapping(self):
        self.candidate_file()
        run = read(self.run_path)
        run["naming"] = {"status": "planned", "filename": "candidate.md"}
        save(self.run_path, run)
        report = workflow.inspect_workflow(self.run_path)
        self.assertTrue(any(x.code == "NAMING_PLAN_FIELD_MISSING" and
                            x.target == "run.naming/title_mapping" for x in report.findings))

    def test_complete_offline_naming_record_accepts_unknown_library_limits(self):
        self.candidate_file()
        run = read(self.run_path)
        source_evidence = {field: {"file": self.check_tools.bind_file(self.source), "locator": {"page": 1}}
                           for field in ("year", "author_token", "original_title")}
        run["naming"] = workflow._module("format_helpers").build_naming_plan(
            year=None, no_year_reason="The synthetic source prints no publication year.",
            author_token="未知作者", unknown_author_reason="The synthetic source names no author.",
            original_title="Synthetic optical apparatus.", source_evidence=source_evidence,
            delivery_path=self.candidate,
        )
        save(self.run_path, run)
        report = workflow.inspect_workflow(self.run_path)
        self.assertFalse(any(x.code.startswith(("NAMING_PLAN_", "WORKFLOW_NAMING_")) for x in report.findings))
        run["naming"]["title_mapping"]["used"] = "An unrecorded replacement"
        save(self.run_path, run)
        report = workflow.inspect_workflow(self.run_path)
        self.assertTrue(any(x.code == "NAMING_PLAN_MAPPING_MISMATCH" and
                            x.target == "run.naming/title_mapping/used" for x in report.findings))

    def test_pending_batch_preserves_individual_file_goals(self):
        batch = workflow.pending_check_batch(self.checks_path, limit=1)
        self.assertEqual([x["id"] for x in batch], ["TEST-A01"])
        self.assertEqual(len(workflow.pending_check_batch(self.checks_path)), 2)
        with self.assertRaises(ValueError):
            workflow.pending_check_batch(self.checks_path, limit=0)

    def test_pending_group_cannot_hide_other_groups_or_accept_a_typo(self):
        ledger = read(self.checks_path)
        ledger["checks"][1]["id"] = "SECOND-A01"
        save(self.checks_path, ledger)
        self.assertEqual([row["id"] for row in workflow.pending_check_batch(self.checks_path, prefix="TEST")], ["TEST-A01"])
        self.assertEqual([row["id"] for row in workflow.pending_check_batch(self.checks_path, prefix="SECOND")], ["SECOND-A01"])
        self.assertEqual(len(workflow.pending_check_batch(self.checks_path)), 2)
        with self.assertRaises(ValueError):
            workflow.pending_check_batch(self.checks_path, prefix="TYPO")

    def test_summary_limits_display_without_resolving_or_deleting_findings(self):
        findings = [workflow.CompletionFinding("MISSING", str(i), "Required local fact.") for i in range(50)]
        report = workflow.CompletionReport(findings=findings + [findings[0]], pending_acceptance=[{"id": "TEST-A01"}])
        before = report.as_dict()
        summary = report.summary(examples_per_code=2)
        self.assertFalse(summary["records_ready"])
        self.assertEqual(summary["finding_count"], 51)
        self.assertEqual(summary["distinct_finding_count"], 50)
        self.assertEqual(summary["counts_by_code"], {"MISSING": 50})
        self.assertEqual(len(summary["examples_by_code"]["MISSING"]), 2)
        self.assertEqual(summary["pending_file_goal_count"], 1)
        self.assertEqual(report.as_dict(), before)

    def test_summary_rejects_zero_examples(self):
        with self.assertRaises(ValueError):
            workflow.CompletionReport().summary(examples_per_code=0)

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
