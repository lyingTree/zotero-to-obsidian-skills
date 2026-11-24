"""Regression tests for lost dependencies and unsafe documentation references."""

import json
from pathlib import Path
import sys
import tempfile
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPOSITORY_ROOT.joinpath("scripts")))
from validate_repository import DEPENDENCY_URLS, EXPECTED_SKILLS, RULE_PREFIXES, validate_repository


class RepositoryValidationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in EXPECTED_SKILLS:
            self.root.joinpath(name).mkdir()
            prefix = RULE_PREFIXES[name]
            self.root.joinpath(name, "SKILL.md").write_text(
                f"---\nname: {name}\ndescription: Valid test skill.\n---\n# Skill\n"
                f"\n## 约束\n\n- [{prefix}-C01] Preserve the source.\n"
                f"\n## 验收标准\n\n- [ ] [{prefix}-A01] Source is unchanged.\n",
                encoding="utf-8",
            )
        self.root.joinpath("VERSION").write_text("1.0.0\n", encoding="utf-8")
        self.lock = {
            "schema_version": 1,
            "dependencies": {
                name: {
                    "repository": url,
                    "revision": "a" * 40,
                    "skill_path": f"skills/{name}",
                    "extra_paths": {"LICENSE": "LICENSE"},
                }
                for name, url in DEPENDENCY_URLS.items()
            },
        }
        self._save_lock()

    def _save_lock(self):
        self.root.joinpath("dependencies.lock.json").write_text(json.dumps(self.lock), encoding="utf-8")

    def _append(self, text):
        path = self.root.joinpath(EXPECTED_SKILLS[0], "SKILL.md")
        path.write_text(path.read_text(encoding="utf-8") + text, encoding="utf-8")

    def _codes(self):
        return {finding.code for finding in validate_repository(self.root)}

    def test_missing_external_skills_are_explicitly_allowed(self):
        self._append("\n[Obsidian](../obsidian-cli/SKILL.md)\n[Zotero](../zotero-skills/SKILL.md)\n")
        self.assertEqual(validate_repository(self.root), [])

    def test_dependency_exemption_does_not_hide_typo_or_missing_reference(self):
        self._append("\n[wrong](../zotero-skill/SKILL.md)\n[missing](../zotero-skills/references/nonexistent.md)\n")
        self.assertEqual(len(validate_repository(self.root)), 2)
        self.assertEqual(self._codes(), {"broken-link"})

    def test_unpinned_revision_and_path_escape_are_rejected(self):
        self.lock["dependencies"]["zotero-skills"]["revision"] = "main"
        self.lock["dependencies"]["zotero-skills"]["extra_paths"]["../private"] = "scripts"
        self._save_lock()
        self._append("\n[outside](../../outside.md)\n")
        self.assertEqual(self._codes(), {"dependency-revision", "dependency-path", "path-escape"})

    def test_examples_are_not_misread_as_file_dependencies(self):
        self._append("\n`[code](missing.md)`\n````markdown\n```text\n[code](missing.md)\n```\n````\n")
        self.assertEqual(validate_repository(self.root), [])

    def test_duplicate_frontmatter_keys_do_not_silently_pass(self):
        path = self.root.joinpath(EXPECTED_SKILLS[0], "SKILL.md")
        path.write_text("---\nname: wrong\nname: literature-zotero2obsidian\ndescription: Test\n---\n", encoding="utf-8")
        self.assertIn("frontmatter", self._codes())

    def test_declared_skill_must_be_present(self):
        self.root.joinpath(EXPECTED_SKILLS[-1], "SKILL.md").unlink()
        self.assertEqual(self._codes(), {"missing-skill"})

    def test_invalid_yaml_scalar_is_reported_without_crashing(self):
        path = self.root.joinpath(EXPECTED_SKILLS[0], "SKILL.md")
        path.write_text("---\nname: literature-zotero2obsidian\ndescription: 2025-02-30\n---\n", encoding="utf-8")
        self.assertIn("frontmatter", self._codes())

    def test_duplicate_rule_id_cannot_hide_a_separate_check(self):
        self._append("- [ ] [PIPE-A01] Different check with a reused identity.\n")
        self.assertEqual(self._codes(), {"duplicate-rule-id"})

    def test_constraint_and_acceptance_rows_require_their_own_ids(self):
        self._append("- [ ] Unlabelled acceptance.\n\n## 约束\n\n- No rule ID.\n")
        self.assertEqual(self._codes(), {"rule-unit"})

    def test_wrong_skill_prefix_and_wrong_rule_kind_are_rejected(self):
        self._append("- [ ] [ZOT-A02] Another skill's ID.\n- [ ] [PIPE-C02] Constraint in acceptance.\n")
        self.assertEqual(self._codes(), {"rule-id", "rule-checkbox"})

    def test_unknown_skill_is_reported_without_crashing_rule_validation(self):
        path = self.root.joinpath("literature-unregistered", "SKILL.md")
        path.parent.mkdir()
        path.write_text("---\nname: literature-unregistered\ndescription: Unknown test skill.\n---\n", encoding="utf-8")
        self.assertEqual(self._codes(), {"unknown-skill"})


if __name__ == "__main__":
    unittest.main()
