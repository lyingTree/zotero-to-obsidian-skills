"""Regression tests for blinded input preparation, not model quality tests."""

from dataclasses import replace
from pathlib import Path
import json
import subprocess
import tempfile
import unittest

from evals.model_matrix import MatrixConfig, MODELS, EFFORTS, prepare_matrix, inspect_run


class ModelMatrixTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repository = self.root.joinpath("repo")
        self.repository.mkdir()
        self.git("init", "--quiet")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        files = {
            "literature-zotero2obsidian/SKILL.md": "frozen skill",
            "template/example.md": "generic template",
            "evals/answers.md": "private answer must not leak",
            "README.md": "historical model results must not leak",
        }
        for name, text in files.items():
            path = self.repository.joinpath(name)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "fixture")
        self.inputs = self.root.joinpath("inputs")
        self.inputs.mkdir()
        for name in ("main.pdf", "supplement.docx", "expected.md"):
            self.inputs.joinpath(name).write_bytes(name.encode())
        self.config = MatrixConfig(
            self.repository, "HEAD", self.inputs.joinpath("main.pdf"),
            self.inputs.joinpath("supplement.docx"), self.inputs.joinpath("expected.md"),
            self.root.joinpath("matrix"),
        )

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repository, check=True,
                              capture_output=True).stdout

    def prepare(self):
        path = prepare_matrix(self.config)
        return path, json.loads(path.read_text(encoding="utf-8"))

    def test_all_nine_configs_use_identical_blind_inputs(self):
        _, manifest = self.prepare()
        self.assertEqual({(r["model"], r["reasoning_effort"]) for r in manifest["runs"]},
                         {(m, e) for m in MODELS for e in EFFORTS})
        tasks = set()
        for run in manifest["runs"]:
            root = Path(run["root"])
            tasks.add(root.joinpath("task.txt").read_bytes())
            self.assertFalse(root.joinpath("skills", "evals").exists())
            self.assertFalse(root.joinpath("skills", "README.md").exists())
            self.assertFalse(root.joinpath("input", "expected.md").exists())
            self.assertEqual(root.joinpath("input", "supplement.docx").read_bytes(), b"supplement.docx")
            self.assertEqual(run["status"], "not_started")
        self.assertEqual(len(tasks), 1)
        self.assertIn("no model invocation", manifest["launch_status"])

    def test_dirty_worktree_does_not_change_frozen_skill(self):
        self.repository.joinpath("literature-zotero2obsidian", "SKILL.md").write_text(
            "uncommitted answer", encoding="utf-8")
        _, manifest = self.prepare()
        path = Path(manifest["runs"][0]["root"]).joinpath("skills", "literature-zotero2obsidian", "SKILL.md")
        self.assertEqual(path.read_text(encoding="utf-8"), "frozen skill")

    def test_rerun_does_not_overwrite_prior_trials(self):
        self.prepare()
        with self.assertRaises(FileExistsError):
            prepare_matrix(self.config)

    def test_private_inputs_cannot_be_staged_inside_the_public_repository(self):
        with self.assertRaises(ValueError):
            prepare_matrix(replace(self.config, destination=self.repository.joinpath("private-trials")))

    def test_detects_changed_and_added_protected_files(self):
        manifest_path, manifest = self.prepare()
        root = Path(manifest["runs"][0]["root"])
        root.joinpath("input", "main.pdf").write_bytes(b"changed")
        root.joinpath("skills", "extra.md").write_bytes(b"added")
        root.joinpath("task.txt").write_bytes(b"changed prompt")
        result = inspect_run(root, manifest_path)
        self.assertCountEqual(result["protected_file_changes"],
                              ["input/main.pdf", "skills/extra.md (added)", "task.txt"])
        self.assertEqual(result["quality_status"], "not_assessed_by_integrity_check")

    def test_integrity_without_candidate_is_not_success(self):
        manifest_path, manifest = self.prepare()
        root = Path(manifest["runs"][0]["root"])
        result = inspect_run(root, manifest_path)
        self.assertFalse(result["candidate_exists"])
        self.assertEqual(result["protected_file_changes"], [])
        self.assertEqual(result["quality_status"], "not_assessed_by_integrity_check")

    def test_preserves_and_binds_all_output_artifacts(self):
        manifest_path, manifest = self.prepare()
        root = Path(manifest["runs"][0]["root"])
        root.joinpath("output", "candidate.md").write_bytes(b"candidate")
        root.joinpath("work", "failure.json").write_bytes(b"{}")
        result = inspect_run(root, manifest_path)
        self.assertTrue(result["candidate_exists"])
        self.assertEqual({r["relative"] for r in result["artifacts"]},
                         {"output/candidate.md", "work/failure.json"})


if __name__ == "__main__":
    unittest.main()
