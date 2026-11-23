"""Installer regression tests; all writes and Git fetches stay in temporary folders."""

from __future__ import annotations

import contextlib
import io
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path, PurePosixPath
from unittest.mock import patch

import install as installer


class InstallerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="z2o-install-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root.joinpath("source with spaces")
        self.source.mkdir()
        self.target = self.root.joinpath("安装 skills")
        for name in installer.LITERATURE_SKILLS:
            self.write_skill(self.source.joinpath(name), name, "new")
        self.source.joinpath("literature-zotero2obsidian", "requirements.txt").write_text("PyYAML>=6.0,<7\n", encoding="utf-8")
        self.source.joinpath("VERSION").write_text("1.0.0\n", encoding="utf-8")
        shutil.copy2(installer.REPOSITORY_ROOT.joinpath("dependencies.lock.json"), self.source.joinpath("dependencies.lock.json"))
        self.config = installer.InstallConfig(
            skills_root=self.target,
            source_root=self.source,
            install_python_dependencies=False,
        )

    @staticmethod
    def write_skill(path: Path, name: str, content: str) -> None:
        path.mkdir(parents=True, exist_ok=True)
        path.joinpath("SKILL.md").write_text(
            f"---\nname: {name}\ndescription: A test skill.\n---\n\n{content}\n", encoding="utf-8"
        )

    @staticmethod
    def fake_fetch(dependency: installer.Dependency, destination: Path, _git: str) -> str:
        skill = destination.joinpath(*dependency.skill_path.parts)
        InstallerTests.write_skill(skill, dependency.name, "new dependency")
        destination.joinpath("LICENSE").write_text("MIT fixture license\n", encoding="utf-8")
        if dependency.name == "zotero-skills":
            scripts = destination.joinpath("scripts")
            scripts.mkdir()
            scripts.joinpath("zotero_client.py").write_text("# fixture\n", encoding="utf-8")
            destination.joinpath("requirements.txt").write_text("pyzotero>=1.5\n", encoding="utf-8")
        return dependency.revision

    def perform_install(self, **kwargs: object) -> installer.InstallResult:
        return installer.install(self.config, fetcher=kwargs.get("fetcher", self.fake_fetch), progress=lambda _: None)

    def existing_skills(self) -> dict[str, str]:
        for name in (*installer.LITERATURE_SKILLS, *installer.DEPENDENCY_NAMES):
            target = self.target.joinpath(name)
            self.write_skill(target, name, "old user content")
            target.joinpath("custom.txt").write_text(f"custom {name}\n", encoding="utf-8")
        return self.fingerprints()

    def fingerprints(self) -> dict[str, str]:
        return {
            name: installer.tree_fingerprint(self.target.joinpath(name))
            for name in (*installer.LITERATURE_SKILLS, *installer.DEPENDENCY_NAMES)
            if self.target.joinpath(name).exists()
        }

    def test_new_target_installs_all_skills_licenses_and_scripts(self) -> None:
        result = self.perform_install()
        self.assertEqual(len(result.installed), 14)
        self.assertEqual(set(result.dependency_revisions), set(installer.DEPENDENCY_NAMES))
        for name in installer.DEPENDENCY_NAMES:
            self.assertEqual(self.target.joinpath(name, "LICENSE").read_text(encoding="utf-8"), "MIT fixture license\n")
        self.assertTrue(self.target.joinpath("zotero-skills", "scripts", "zotero_client.py").is_file())
        self.assertFalse(self.target.joinpath(".z2o-install.lock").exists())
        self.assertEqual(list(self.target.glob(".z2o-staging-*")), [])

    def test_existing_customizations_are_backed_up(self) -> None:
        before = self.existing_skills()
        self.target.joinpath("unrelated-folder").mkdir()
        result = self.perform_install()
        self.assertIsNotNone(result.backup_root)
        for name, fingerprint in before.items():
            self.assertEqual(installer.tree_fingerprint(result.backup_root.joinpath("previous", name)), fingerprint)
        self.assertTrue(self.target.joinpath("unrelated-folder").is_dir())
        receipt = json.loads(result.backup_root.joinpath("installation.json").read_text(encoding="utf-8"))
        self.assertEqual(receipt["status"], "completed")
        self.assertEqual(receipt["source_version"], "1.0.0")
        self.assertEqual(receipt["python_dependencies"], "skipped")

    def test_identical_install_keeps_files_and_avoids_another_backup(self) -> None:
        first = self.perform_install()
        note = self.target.joinpath("literature-filename", "SKILL.md")
        before_mtime = note.stat().st_mtime_ns
        before = self.fingerprints()
        second = self.perform_install()
        self.assertEqual(second.installed, ())
        self.assertEqual(len(second.skipped), 14)
        self.assertIsNone(second.backup_root)
        self.assertEqual(before, self.fingerprints())
        self.assertEqual(note.stat().st_mtime_ns, before_mtime)
        self.assertEqual(len(list(first.backup_root.parent.iterdir())), 1)

    def test_repository_destination_skips_own_literature_directories(self) -> None:
        config = installer.InstallConfig(skills_root=self.source, source_root=self.source, install_python_dependencies=False)
        before = {name: installer.tree_fingerprint(self.source.joinpath(name)) for name in installer.LITERATURE_SKILLS}
        result = installer.install(config, fetcher=self.fake_fetch, progress=lambda _: None)
        self.assertEqual(set(result.installed), set(installer.DEPENDENCY_NAMES))
        for name in installer.LITERATURE_SKILLS:
            self.assertEqual(installer.tree_fingerprint(self.source.joinpath(name)), before[name])

    def test_nested_destination_is_rejected_before_it_is_created(self) -> None:
        target = self.source.joinpath("literature-zotero2obsidian", "nested-skills")
        config = installer.InstallConfig(skills_root=target, source_root=self.source, install_python_dependencies=False)
        with self.assertRaisesRegex(installer.InstallationError, "inside a source skill"):
            installer.install(config, fetcher=self.fake_fetch, progress=lambda _: None)
        self.assertFalse(target.exists())

    def test_source_inside_replacement_target_is_rejected(self) -> None:
        nested_source = self.target.joinpath("literature-zotero2obsidian", "source")
        nested_source.mkdir(parents=True)
        config = installer.InstallConfig(skills_root=self.target, source_root=nested_source, install_python_dependencies=False)
        with self.assertRaisesRegex(installer.InstallationError, "inside a skill being replaced"):
            installer.install(config, fetcher=self.fake_fetch, progress=lambda _: None)

    def test_python_caches_are_not_installed_or_treated_as_source_changes(self) -> None:
        skill = self.source.joinpath("literature-zotero2obsidian")
        cache = skill.joinpath("__pycache__")
        cache.mkdir()
        cache.joinpath("fixture.cpython-310.pyc").write_bytes(b"cache")
        skill.joinpath("legacy.pyc").write_bytes(b"cache")
        self.perform_install()
        self.assertFalse(self.target.joinpath("literature-zotero2obsidian", "__pycache__").exists())
        self.assertFalse(self.target.joinpath("literature-zotero2obsidian", "legacy.pyc").exists())
        installed_cache = self.target.joinpath("literature-zotero2obsidian", "__pycache__")
        installed_cache.mkdir()
        installed_cache.joinpath("fresh.pyc").write_bytes(b"cache created after running the skill")
        result = self.perform_install()
        self.assertEqual(result.installed, ())
        self.assertTrue(installed_cache.joinpath("fresh.pyc").exists())

    def test_missing_upstream_license_preserves_all_existing_skills(self) -> None:
        before = self.existing_skills()

        def broken_fetch(dependency: installer.Dependency, destination: Path, git: str) -> str:
            revision = self.fake_fetch(dependency, destination, git)
            if dependency.name == "zotero-skills":
                destination.joinpath("LICENSE").unlink()
            return revision

        with self.assertRaises(installer.InstallationError):
            self.perform_install(fetcher=broken_fetch)
        self.assertEqual(self.fingerprints(), before)
        self.assertFalse(self.target.joinpath(".z2o-backups").exists())

    def test_missing_local_skill_stops_before_fetch(self) -> None:
        shutil.rmtree(self.source.joinpath("literature-filename"))
        with patch.object(installer, "fetch_repository") as fetch:
            with self.assertRaises(installer.InstallationError):
                self.perform_install(fetcher=fetch)
            fetch.assert_not_called()

    def test_pip_failure_keeps_existing_skills_and_no_success_is_reported(self) -> None:
        before = self.existing_skills()
        config = installer.InstallConfig(skills_root=self.target, source_root=self.source)
        failure = subprocess.CalledProcessError(71, ["python", "-m", "pip"])
        with patch.object(installer, "_run", side_effect=failure):
            with self.assertRaises(subprocess.CalledProcessError):
                installer.install(config, fetcher=self.fake_fetch, progress=lambda _: None)
        self.assertEqual(self.fingerprints(), before)
        self.assertFalse(self.target.joinpath(".z2o-backups").exists())
        output = io.StringIO()
        with patch.object(installer, "install", side_effect=failure), contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            self.assertEqual(installer.main(), 1)
        self.assertNotIn("Done:", output.getvalue())

    def test_pip_receives_both_requirements_files(self) -> None:
        config = installer.InstallConfig(skills_root=self.target, source_root=self.source)
        with patch.object(installer, "_run") as run:
            installer.install(config, fetcher=self.fake_fetch, progress=lambda _: None)
        command = run.call_args.args[0]
        self.assertEqual(command[:4], [config.python_executable, "-m", "pip", "install"])
        self.assertEqual(command.count("-r"), 2)
        self.assertEqual(Path(command[5]), self.source.joinpath("literature-zotero2obsidian", "requirements.txt"))

    def test_mid_publish_failure_rolls_back_every_skill(self) -> None:
        before = self.existing_skills()
        real_move = installer._move_path

        def interrupted_move(source: Path, target: Path) -> None:
            if source.parent.name == "packages" and source.name == "literature-filename":
                raise OSError("simulated replace failure")
            real_move(source, target)

        with patch.object(installer, "_move_path", side_effect=interrupted_move):
            with self.assertRaisesRegex(installer.InstallationError, "Previous skill directories restored"):
                self.perform_install()
        self.assertEqual(before, self.fingerprints())
        receipt_paths = list(self.target.joinpath(".z2o-backups").glob("*/installation.json"))
        self.assertEqual(len(receipt_paths), 1)
        receipt = json.loads(receipt_paths[0].read_text(encoding="utf-8"))
        self.assertEqual(receipt["status"], "rolled_back")

    def test_mid_publish_failure_removes_new_skills_from_live_target(self) -> None:
        real_move = installer._move_path

        def interrupted_move(source: Path, target: Path) -> None:
            if source.parent.name == "packages" and source.name == "literature-filename":
                raise OSError("simulated replace failure")
            real_move(source, target)

        with patch.object(installer, "_move_path", side_effect=interrupted_move):
            with self.assertRaises(installer.InstallationError):
                self.perform_install()
        self.assertEqual(self.fingerprints(), {})

    def test_rollback_preserves_edits_made_to_a_published_skill(self) -> None:
        before = self.existing_skills()
        edited_name = "literature-zotero2obsidian"
        marker = self.target.joinpath(edited_name, "human-edit.txt")
        real_move = installer._move_path

        def interrupted_move(source: Path, target: Path) -> None:
            if source.parent.name == "packages" and source.name == "literature-filename":
                marker.write_text("human edit while installing", encoding="utf-8")
                raise OSError("simulated replace failure")
            real_move(source, target)

        with patch.object(installer, "_move_path", side_effect=interrupted_move):
            with self.assertRaisesRegex(installer.InstallationError, "destination changed"):
                self.perform_install()
        self.assertEqual(marker.read_text(encoding="utf-8"), "human edit while installing")
        receipt_path = next(self.target.joinpath(".z2o-backups").glob("*/installation.json"))
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        self.assertEqual(receipt["status"], "rollback_incomplete")
        self.assertEqual(installer.tree_fingerprint(receipt_path.parent.joinpath("previous", edited_name)), before[edited_name])
        after = self.fingerprints()
        for name, fingerprint in before.items():
            if name == edited_name:
                continue
            self.assertEqual(after[name], fingerprint)

    def test_unknown_target_directory_is_preserved(self) -> None:
        target = self.target.joinpath("obsidian-cli")
        target.mkdir(parents=True)
        target.joinpath("important.txt").write_text("must survive", encoding="utf-8")
        with self.assertRaisesRegex(installer.InstallationError, "without SKILL.md"):
            self.perform_install()
        self.assertEqual(target.joinpath("important.txt").read_text(encoding="utf-8"), "must survive")

    def test_target_changes_during_preparation_are_preserved(self) -> None:
        self.existing_skills()
        marker = self.target.joinpath("obsidian-cli", "custom.txt")

        def concurrent_fetch(dependency: installer.Dependency, destination: Path, git: str) -> str:
            marker.write_text("new human edit", encoding="utf-8")
            return self.fake_fetch(dependency, destination, git)

        with self.assertRaisesRegex(installer.InstallationError, "Target changed"):
            self.perform_install(fetcher=concurrent_fetch)
        self.assertEqual(marker.read_text(encoding="utf-8"), "new human edit")
        self.assertFalse(self.target.joinpath(".z2o-backups").exists())

    def test_symbolic_link_target_is_rejected(self) -> None:
        self.target.mkdir()
        outside = self.root.joinpath("outside")
        self.write_skill(outside, "obsidian-cli", "keep this")
        link = self.target.joinpath("obsidian-cli")
        try:
            link.symlink_to(outside, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"Creating a symlink is unavailable: {error}")
        with self.assertRaisesRegex(installer.InstallationError, "symbolic-link"):
            self.perform_install()
        self.assertTrue(link.is_symlink())
        self.assertIn("keep this", outside.joinpath("SKILL.md").read_text(encoding="utf-8"))

    def test_existing_install_lock_is_not_removed(self) -> None:
        self.target.mkdir()
        lock = self.target.joinpath(".z2o-install.lock")
        lock.write_text('{"pid": 123}', encoding="utf-8")
        with self.assertRaisesRegex(installer.InstallationError, "install lock"):
            self.perform_install()
        self.assertEqual(lock.read_text(encoding="utf-8"), '{"pid": 123}')

    def test_full_commit_sha_branch_and_tag_are_fetchable(self) -> None:
        repository = self.root.joinpath("local git repository")
        repository.mkdir()
        installer._run(["git", "init", "--quiet", "--initial-branch=main", str(repository)])
        installer._run(["git", "config", "user.name", "Installer test"], cwd=repository)
        installer._run(["git", "config", "user.email", "installer-test@example.invalid"], cwd=repository)
        repository.joinpath("fixture.txt").write_text("fixture\n", encoding="utf-8")
        installer._run(["git", "add", "fixture.txt"], cwd=repository)
        installer._run(["git", "commit", "--quiet", "-m", "fixture"], cwd=repository)
        installer._run(["git", "tag", "v1"], cwd=repository)
        sha = installer._run(["git", "rev-parse", "HEAD"], cwd=repository, capture=True)
        for index, ref in enumerate((sha, "main", "v1")):
            with self.subTest(ref=ref):
                dependency = installer.Dependency("test", str(repository), sha, ref, PurePosixPath("skill"), ())
                actual = installer.fetch_repository(dependency, self.root.joinpath(f"checkout-{index}"), "git")
                self.assertEqual(actual, sha)

    def test_lock_requires_full_sha_and_preserves_license(self) -> None:
        path = self.source.joinpath("dependencies.lock.json")
        lock = json.loads(path.read_text(encoding="utf-8"))
        lock["dependencies"]["obsidian-cli"]["revision"] = "main"
        path.write_text(json.dumps(lock), encoding="utf-8")
        with self.assertRaisesRegex(installer.InstallationError, "full commit SHA"):
            self.perform_install()


if __name__ == "__main__":
    unittest.main()
