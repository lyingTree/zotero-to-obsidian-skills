"""Run the actual formatter, renderer, checker and restart protocol together."""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from copy import deepcopy
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "note_pipeline_under_test", ROOT.joinpath("literature-zotero2obsidian", "scripts", "note_pipeline.py"))
pipeline = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pipeline)


def digest(raw):
    return sha256(raw if isinstance(raw, bytes) else raw.encode("utf-8")).hexdigest()


class PipelineTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root.joinpath("paper.txt")
        self.source.write_bytes(b"A small source for a synthetic assembly fixture.")
        self.original = self.root.joinpath("candidate.md")
        self.original.write_bytes("---\ntype: article\n---\n\n# Example\n\n## 1 方法\n每60 min观测，增加31.$2\\%$。\n".encode())
        self.output = self.root.joinpath("formatted")
        self.contract = {"schema_version": 1, "sources": {
            "main": {"path": str(self.source), "sha256": digest(self.source.read_bytes())}},
            "counts": {"figures": 0, "equations": 0, "tables": 0, "bibliography": 0}}

    def run_markdown(self, **kwargs):
        return pipeline.process_markdown(self.original, self.output, contract=self.contract,
                                         resource_root=self.root, progress=None, **kwargs)

    def test_normalize_replay_restart_and_immutable_original(self):
        before = self.original.read_bytes()
        result = self.run_markdown()
        directory = Path(result["directory"])
        note = directory.joinpath("normalized.md").read_text(encoding="utf-8")
        self.assertIn(r"$60\,\mathrm{min}$", note)
        self.assertIn(r"$31.2\%$", note)
        self.assertEqual(self.original.read_bytes(), before)
        self.assertEqual(directory.joinpath("original.md").read_bytes(), before)
        normalization = pipeline.read_json(directory.joinpath("normalization.json"))
        self.assertEqual(pipeline.replay_changes(before.decode(), normalization["changes"]), note)
        snapshots = {path.name: (path.read_bytes(), path.stat().st_mtime_ns) for path in directory.iterdir()}
        self.assertEqual(self.run_markdown(), result)
        self.assertEqual({path.name: (path.read_bytes(), path.stat().st_mtime_ns)
                          for path in directory.iterdir()}, snapshots)
        self.assertEqual(result["status"], "mechanical_pass")
        self.assertEqual(result["source_fidelity"], "not_assessed")

    def test_no_contract_does_not_award_complete_acceptance(self):
        result = pipeline.process_markdown(self.original, self.output, resource_root=self.root, progress=None)
        self.assertEqual(result["status"], "needs_attention")
        report = pipeline.read_json(Path(result["directory"]).joinpath("acceptance.json"))
        self.assertEqual(report["source_coverage"], "not_assessed")

    def test_changed_original_is_new_run_and_old_result_unchanged(self):
        old = self.run_markdown()
        receipt = Path(old["directory"]).joinpath("result.json").read_bytes()
        self.original.write_bytes(self.original.read_bytes().replace(b"60", b"61"))
        new = self.run_markdown()
        self.assertNotEqual(old["run_id"], new["run_id"])
        self.assertEqual(Path(old["directory"]).joinpath("result.json").read_bytes(), receipt)

    def test_tampered_output_is_never_overwritten(self):
        result = self.run_markdown()
        note = Path(result["directory"]).joinpath("normalized.md")
        note.write_bytes(b"a user edit")
        with self.assertRaises(pipeline.PipelineConflictError):
            self.run_markdown()
        self.assertEqual(note.read_bytes(), b"a user edit")

    def test_source_changed_does_not_reuse_old_acceptance(self):
        result = self.run_markdown()
        receipt = Path(result["directory"]).joinpath("result.json").read_bytes()
        self.source.write_bytes(b"a new source")
        with self.assertRaises(pipeline.PipelineConflictError):
            self.run_markdown()
        self.assertEqual(Path(result["directory"]).joinpath("result.json").read_bytes(), receipt)

    def test_restart_after_interrupted_final_report_writes_missing_artifacts(self):
        result = self.run_markdown()
        root = Path(result["directory"])
        normalization = root.joinpath("normalized.md").read_bytes()
        root.joinpath("report.md").unlink()
        root.joinpath("result.json").unlink()
        self.assertEqual(self.run_markdown(), result)
        self.assertEqual(root.joinpath("normalized.md").read_bytes(), normalization)

    def test_atomic_publication_preserves_conflicts_and_concurrent_identical_files(self):
        path = self.root.joinpath("result.bin")
        with ThreadPoolExecutor(max_workers=4) as pool:
            futures = [pool.submit(pipeline._publish, path, b"complete" * 10000) for _ in range(8)]
            for future in futures:
                future.result()
        self.assertEqual(path.read_bytes(), b"complete" * 10000)
        with self.assertRaises(pipeline.PipelineConflictError):
            pipeline._publish(path, b"different")
        self.assertFalse(list(self.root.glob(".pending-*")))

    def test_invalid_utf8_is_preserved_and_reported_without_final_note(self):
        self.original.write_bytes(b"\xff\xfe\x00")
        result = self.run_markdown()
        root = Path(result["directory"])
        self.assertEqual(result["status"], "blocked")
        self.assertEqual(result["issues"][0]["code"], "INPUT_ENCODING")
        self.assertEqual(root.joinpath("original.md").read_bytes(), b"\xff\xfe\x00")
        self.assertFalse(root.joinpath("normalized.md").exists())

    def test_duplicate_json_and_nonfinite_fields_are_not_silently_lost(self):
        path = self.root.joinpath("input.json")
        for text in ('{"p":1,"p":2}', '{"p":NaN}', '[]'):
            path.write_text(text, encoding="utf-8")
            with self.subTest(text=text), self.assertRaises(ValueError):
                pipeline.read_json(path)

    def structured(self):
        units = [
            dict(unit_id="title", kind="header", original_title="A source", translated=True,
                 authors=["Author"], affiliations=["Institute"]),
            dict(unit_id="method", kind="heading", level=2),
            dict(unit_id="paragraph", kind="paragraph"),
        ]
        for unit in units:
            unit.update(source={"source_id": "main", "locator": unit["unit_id"]}, scope="main")
        plan = {"schema_version": 1, "properties": {"type": "article"}, "units": units}
        content = {"title": {"translated_title": "来源"}, "method": {"text": "方法"},
                   "paragraph": {"text": "每60 min观测，变化7.00%。"}}
        required = [{key: unit[key] for key in ("unit_id", "kind", "source", "scope")} for unit in units]
        contract = {**deepcopy(self.contract), "required_units": required}
        return plan, content, contract

    def test_structured_units_have_correct_mappings_after_actual_normalization(self):
        plan, content, contract = self.structured()
        original = deepcopy((plan, content, contract))
        result = pipeline.process_content(plan, content, self.output, contract=contract,
                                          resource_root=self.root, progress=None)
        root = Path(result["directory"])
        self.assertEqual(result["status"], "mechanical_pass", root.joinpath("acceptance.json").read_text(encoding="utf-8"))
        note = root.joinpath("normalized.md").read_text(encoding="utf-8")
        self.assertIn("# A source\n来源\n\nAuthor", note)
        self.assertIn(r"$60\,\mathrm{min}$", note)
        self.assertIn(r"$7.00\%$", note)
        rendering = pipeline.read_json(root.joinpath("rendering.json"))
        for entry in rendering["mapping"]:
            text = note[entry["start_offset"]:entry["end_offset"]]
            self.assertEqual(digest(text), entry["sha256"])
            self.assertEqual(text, "\n".join(note.split("\n")[entry["start_line"]-1:entry["end_line"]]))
        self.assertEqual((plan, content, contract), original)

    def test_automatic_source_contract_precedes_candidate_fields(self):
        plan, content, _ = self.structured()
        contract = pipeline.build_contract(plan, {"main": self.source})
        self.assertEqual(contract["sources"]["main"]["sha256"], digest(self.source.read_bytes()))
        self.assertEqual([unit["unit_id"] for unit in contract["required_units"]],
                         [unit["unit_id"] for unit in plan["units"]])
        result = pipeline.process_content(plan, content, self.output, contract=contract,
                                          resource_root=self.root, progress=None)
        self.assertEqual(result["status"], "mechanical_pass")
        with self.assertRaises(ValueError):
            pipeline.build_contract(plan, {"different": self.source})

    def test_prepare_job_retries_preserve_model_translation(self):
        plan, content, _ = self.structured()
        first = pipeline.prepare_content_job(plan, {"main": self.source}, self.output)
        filled = Path(first["directory"]).joinpath("content.json")
        filled.write_text(json.dumps(content, ensure_ascii=False), encoding="utf-8")
        original = filled.read_bytes()
        self.assertEqual(first, pipeline.prepare_content_job(plan, {"main": self.source}, self.output))
        self.assertEqual(filled.read_bytes(), original)
        self.source.write_bytes(b"different source")
        second = pipeline.prepare_content_job(plan, {"main": self.source}, self.output)
        self.assertNotEqual(first["job_id"], second["job_id"])

    def test_mapping_artifact_conflict_is_not_downgraded_to_review(self):
        plan, content, contract = self.structured()
        result = pipeline.process_content(plan, content, self.output, contract=contract,
                                          resource_root=self.root, progress=None)
        mapping = Path(result["directory"]).joinpath("rendering.json")
        mapping.write_text("user edited mapping", encoding="utf-8")
        with self.assertRaises(pipeline.PipelineConflictError):
            pipeline.process_content(plan, content, self.output, contract=contract,
                                     resource_root=self.root, progress=None)
        self.assertEqual(mapping.read_text(encoding="utf-8"), "user edited mapping")

    def test_hot_reloaded_version_cannot_label_cached_code_as_new(self):
        changed = deepcopy(pipeline._IMPORTED_ENGINE)
        changed["files"]["normalize_note.py"] = "0" * 64
        with patch.object(pipeline, "_version", return_value=changed):
            with self.assertRaisesRegex(pipeline.PipelineConflictError, "new Python interpreter"):
                self.run_markdown()

    def test_phase_report_does_not_invent_model_root_causes(self):
        result = self.run_markdown()
        report = pipeline.read_json(Path(result["directory"]).joinpath("phases.json"))
        self.assertTrue(report["automatic_repairs"])
        self.assertIn("pdf_text_extraction", report["unobserved_model_processes"])
        self.assertIn("translation", report["unobserved_model_processes"])
        self.assertTrue(all(row["cause_status"] == "not_established"
                            for row in report["observed_failures"]))

    def test_missing_translation_returns_located_block_without_partial_note(self):
        plan, content, contract = self.structured()
        del content["paragraph"]
        result = pipeline.process_content(plan, content, self.output, contract=contract,
                                          resource_root=self.root, progress=None)
        self.assertEqual(result["status"], "blocked")
        self.assertIn("paragraph", result["issues"][0]["field"])
        self.assertFalse(Path(result["directory"]).joinpath("normalized.md").exists())

    def test_progress_reports_real_executed_stages(self):
        events = []
        pipeline.process_markdown(self.original, self.output, contract=self.contract,
                                   resource_root=self.root, progress=lambda *args: events.append(args))
        self.assertEqual([row[0] for row in events], list(range(1, 7)))
        self.assertTrue(all(row[1] == 6 for row in events))
        self.assertIn("空行", events[2][2])

    def test_source_change_after_acceptance_prevents_final_receipt(self):
        def change_after_check(done, total, stage):
            if done == 5:
                self.source.write_bytes(b"changed while processing")
        with self.assertRaisesRegex(pipeline.PipelineConflictError, "changed after acceptance"):
            pipeline.process_markdown(self.original, self.output, contract=self.contract,
                                       resource_root=self.root, progress=change_after_check)
        self.assertFalse(list(self.output.glob("*/result.json")))

    def test_first_parallel_load_uses_complete_modules(self):
        # A fresh interpreter is necessary; earlier tests may already warm caches.
        script = self.root.joinpath("parallel_first_load.py")
        script.write_text(
            "import importlib.util\nfrom concurrent.futures import ThreadPoolExecutor\n"
            f"spec = importlib.util.spec_from_file_location('fresh', {str(SPEC.origin)!r})\n"
            "p = importlib.util.module_from_spec(spec)\nspec.loader.exec_module(p)\n"
            "def load(_):\n    return p._module('normalize_note').normalize_note('每60 min。')['text']\n"
            "with ThreadPoolExecutor(max_workers=8) as pool:\n    outputs = list(pool.map(load, range(16)))\n"
            "assert len(set(outputs)) == 1\nassert '$60' in outputs[0]\n", encoding="utf-8")
        completed = subprocess.run([sys.executable, str(script)], capture_output=True, text=True, check=False)
        self.assertEqual(completed.returncode, 0, completed.stderr)


if __name__ == "__main__":
    unittest.main()
