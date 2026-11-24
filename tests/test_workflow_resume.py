"""A complete task resumes one real drafting action before review paperwork."""
from __future__ import annotations

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
SPEC = importlib.util.spec_from_file_location("test_workflow_resume_tools", SCRIPT)
assert SPEC and SPEC.loader
workflow = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = workflow
SPEC.loader.exec_module(workflow)


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


class WorkflowResumeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.skills = self.base.joinpath("skills")
        skill = self.skills.joinpath("synthetic-skill")
        skill.mkdir(parents=True)
        skill.joinpath("SKILL.md").write_text(
            "# Synthetic\n"
            "- [ ] [TEST-A01] A source sentence has a target. <!-- check-type: offline_outcome -->\n",
            encoding="utf-8",
        )
        self.source = self.base.joinpath("source.pdf")
        with fitz.open() as document:
            for sentence in ("First synthetic paragraph.", "Second synthetic paragraph."):
                page = document.new_page(width=220, height=160)
                page.insert_text((20, 40), sentence)
            document.save(self.source)
        self.work = self.base.joinpath("work")
        self.candidate = self.base.joinpath("output", "candidate.md")
        self.candidate.parent.mkdir()
        self.run_path = self.initialize(self.work)
        self.run = read(self.run_path)
        self.queue_path = Path(self.run["records"]["draft_queue"])
        self.queue_tools = workflow._module("draft_queue")

    def initialize(self, work):
        return workflow.initialize_workflow(
            skills_root=self.skills, work_root=work,
            sources={"main": self.source}, source_roles={"main": "main"},
            candidate_path=self.candidate, resource_root=self.candidate.parent,
            translation_required=False, progress=lambda *_: None,
        )

    def finish_next_page(self):
        state = workflow.resume_workflow(self.run_path)
        task = state["next_action"]
        self.assertEqual(task["stage"], "inventory_page")
        page = task["page"]
        queue = self.queue_tools.DraftQueue.open(self.queue_path)
        queue.review_pdf_page_order("main", page["page"], page["block_ids"],
                                    "The synthetic fixture has one complete paragraph on this page.")
        queue.review_additional_objects("main", page["page"],
                                        "This explicit fixture contains no additional object.")
        self.assertEqual(workflow.resume_workflow(self.run_path)["next_action"]["stage"], "group_or_exclude")
        unit = f"paragraph-{page['page']}"
        queue.group(unit, page["block_ids"], kind="paragraph",
                    reason="These are the complete entries of one synthetic paragraph.")
        self.assertEqual(workflow.resume_workflow(self.run_path)["next_action"]["stage"], "write_unit")
        queue.write_unit(unit, f"合成来源的第 {page['page']} 个完整段落。\n")
        return queue

    def test_initialization_registers_actual_all_pending_queue(self):
        queue = self.queue_tools.DraftQueue.open(self.queue_path)
        self.assertEqual(len(queue.data["blocks"]), 2)
        self.assertEqual(queue.data["units"], {})
        self.assertEqual(queue.data["sources"]["main"]["sha256"], sha256(self.source.read_bytes()).hexdigest())
        self.assertEqual(read(Path(self.run["records"]["checks"]))["checks"][0]["status"], "not_verified")
        self.assertFalse(self.candidate.exists())

    def test_initial_resume_is_read_only_one_real_page_not_acceptance_menu(self):
        paths = [self.run_path, self.queue_path, Path(self.run["records"]["checks"])]
        before = [path.read_bytes() for path in paths]
        state = workflow.resume_workflow(self.run_path)
        self.assertEqual(state["phase"], "draft")
        self.assertEqual(state["next_action"]["stage"], "inventory_page")
        self.assertEqual(state["next_action"]["page"]["page"], 1)
        self.assertEqual(state["next_action"]["source_file"]["sha256"], sha256(self.source.read_bytes()).hexdigest())
        self.assertEqual(state["counts"]["unassigned_entries_count"], 2)
        self.assertNotIn("pending_acceptance", state)
        self.assertNotIn("records_ready", state)
        self.assertEqual(state["source_fidelity"], "not_assessed")
        self.assertEqual([path.read_bytes() for path in paths], before)

    def test_partial_candidate_cannot_skip_unwritten_source(self):
        self.candidate.write_text("# Partial\n\nOnly a conclusion.\n", encoding="utf-8")
        self.finish_next_page()
        state = workflow.resume_workflow(self.run_path)
        self.assertEqual(state["phase"], "draft")
        self.assertFalse(state["draft_ready"])
        self.assertEqual(state["next_action"]["page"]["page"], 2)
        self.assertEqual(state["counts"]["written_unit_count"], 1)

    def test_reopen_keeps_written_fragment_and_returns_next_page(self):
        queue = self.finish_next_page()
        fragment = Path(queue.data["units"]["paragraph-1"]["revisions"][0]["path"])
        original = fragment.read_bytes()
        for _ in range(2):
            state = workflow.resume_workflow(self.run_path)
            self.assertEqual(state["next_action"]["page"]["page"], 2)
        self.assertEqual(fragment.read_bytes(), original)
        self.assertEqual(self.queue_tools.DraftQueue.open(self.queue_path).status()["written_unit_count"], 1)

    def test_all_fragments_then_assembly_then_single_record_action(self):
        self.finish_next_page()
        queue = self.finish_next_page()
        state = workflow.resume_workflow(self.run_path)
        self.assertEqual(state["phase"], "assemble")
        self.assertEqual(state["next_action"]["stage"], "assemble")
        self.assertTrue(state["draft_ready"])
        queue.assemble(self.candidate, progress=None)
        state = workflow.resume_workflow(self.run_path)
        self.assertEqual(state["phase"], "review")
        self.assertEqual(state["next_action"]["stage"], "repair_record")
        self.assertEqual(state["next_action"]["target"], "source_map")
        self.assertEqual(state["next_action"]["record_path"], self.run["records"]["source_map"])
        self.assertFalse(state["records_ready"])
        self.assertEqual(state["source_fidelity"], "not_assessed")
        self.assertNotIn("findings", state)

    def test_legacy_run_requests_registration_without_mutation(self):
        self.run["records"].pop("draft_queue")
        save(self.run_path, self.run)
        before = [self.run_path.read_bytes(), self.queue_path.read_bytes()]
        state = workflow.resume_workflow(self.run_path)
        self.assertEqual(state["phase"], "setup")
        self.assertEqual(state["next_action"]["stage"], "register_draft_queue")
        self.assertEqual([self.run_path.read_bytes(), self.queue_path.read_bytes()], before)

    def test_queue_from_another_inventory_is_not_adopted(self):
        second_run = read(self.initialize(self.base.joinpath("second-work")))
        self.run["records"]["draft_queue"] = second_run["records"]["draft_queue"]
        save(self.run_path, self.run)
        with self.assertRaisesRegex(ValueError, "originate from this workflow"):
            workflow.resume_workflow(self.run_path)

    def test_original_role_and_source_scope_cannot_drift(self):
        self.run["scope"]["source_roles"]["main"] = "supplement"
        save(self.run_path, self.run)
        with self.assertRaisesRegex(ValueError, "workflow source binding"):
            workflow.resume_workflow(self.run_path)
        self.run["scope"]["source_roles"]["main"] = "main"
        for declared in (["main", "main"], [{"id": "main"}], [["main"]], [""], [" "]):
            self.run["scope"]["source_ids"] = declared
            save(self.run_path, self.run)
            with self.subTest(declared=declared), self.assertRaisesRegex(ValueError, "source scope and roles"):
                workflow.resume_workflow(self.run_path)

    def test_changed_original_source_is_not_silently_reinitialized(self):
        before = self.queue_path.read_bytes()
        self.source.write_bytes(self.source.read_bytes() + b"\nchanged-source")
        with self.assertRaisesRegex(ValueError, "Stale source"):
            workflow.resume_workflow(self.run_path)
        self.assertEqual(self.queue_path.read_bytes(), before)

    def test_changed_fragment_cannot_be_hidden_by_existing_candidate(self):
        self.finish_next_page()
        queue = self.finish_next_page()
        queue.assemble(self.candidate, progress=None)
        fragment = Path(queue.data["units"]["paragraph-1"]["revisions"][0]["path"])
        fragment.write_text("changed fragment", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "saved draft fragment or resource changed"):
            workflow.resume_workflow(self.run_path)

    def test_existing_queue_prevents_fresh_initialization_without_overwrite(self):
        other_work = self.base.joinpath("existing-work")
        other_work.mkdir()
        queue = other_work.joinpath("draft-queue.json")
        queue.write_text("existing evidence", encoding="utf-8")
        with self.assertRaises(FileExistsError):
            self.initialize(other_work)
        self.assertEqual(queue.read_text(encoding="utf-8"), "existing evidence")
        self.assertFalse(other_work.joinpath("run.json").exists())
        self.assertFalse(other_work.joinpath("evidence").exists())


if __name__ == "__main__":
    unittest.main()
