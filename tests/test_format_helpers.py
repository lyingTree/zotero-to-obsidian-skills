"""Offline format regressions using invented metadata and glyph snapshots only.

The visible-reading declarations below are synthetic inputs. These tests check
binding and edit mechanics; they do not certify an actual visual review.
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


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "format_helpers_under_test",
    ROOT.joinpath("literature-zotero2obsidian", "scripts", "format_helpers.py"),
)
assert SPEC and SPEC.loader
helpers = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = helpers
SPEC.loader.exec_module(helpers)


def file_record(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path.read_bytes()).hexdigest()}


class _NamingPlanFixture:
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.source = self.base.joinpath("synthetic-fields.json")
        self.source.write_text(json.dumps({
            "year": 2031, "author_token": "Vale", "original_title": "Model measurements",
            "item_key": "KEY123", "purpose": "synthetic binding fixture",
        }), encoding="utf-8")
        self.evidence = {
            field: {"file": file_record(self.source), "locator": {"json_pointer": f"/{field}"}}
            for field in ("year", "author_token", "original_title")
        }

    def plan(self, **changes):
        arguments = {
            "year": 2031, "author_token": "Vale", "original_title": "Model measurements",
            "source_evidence": self.evidence,
        }
        arguments.update(changes)
        return helpers.build_naming_plan(**arguments)


class NamingPlanTests(_NamingPlanFixture, unittest.TestCase):
    def test_plan_is_deterministic_and_serializable_without_semantic_acceptance(self):
        plan = self.plan()
        self.assertEqual(plan, self.plan())
        self.assertEqual(plan["filename"], "2031-Vale-Model measurements.md")
        self.assertEqual(json.loads(json.dumps(plan)), plan)
        self.assertEqual(plan["semantic_review"]["status"], "not_assessed")
        self.assertEqual(plan["identity_review"]["status"], "not_assessed")
        self.assertEqual(plan["library_conflicts"]["status"], "not_checked")
        self.assertFalse(plan["overwrite_authorized"])

    def test_fixed_delivery_path_preserves_separate_name_without_writing(self):
        candidate = self.base.joinpath("candidate.md")
        candidate.write_text("Synthetic delivery remains unchanged.\n", encoding="utf-8")
        before = candidate.read_bytes()
        target = self.base.joinpath("uncreated-target")
        plan = self.plan(delivery_path=candidate, target_directory=target)
        self.assertEqual(plan["delivery_path"], str(candidate.resolve()))
        self.assertEqual(Path(plan["target_path"]).name, plan["filename"])
        self.assertNotEqual(plan["filename"], candidate.name)
        self.assertEqual(candidate.read_bytes(), before)
        self.assertFalse(target.exists())

    def test_source_evidence_missing_or_stale_is_rejected(self):
        missing = deepcopy(self.evidence)
        del missing["year"]
        with self.assertRaisesRegex(ValueError, "each source field"):
            self.plan(source_evidence=missing)
        stale = deepcopy(self.evidence)
        stale["author_token"]["file"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            self.plan(source_evidence=stale)
        self.source.unlink()
        with self.assertRaisesRegex(ValueError, "does not exist"):
            self.plan()

    def test_source_locator_is_required_for_each_field(self):
        for locator in (None, {}, "", "   "):
            with self.subTest(locator=locator):
                evidence = deepcopy(self.evidence)
                evidence["original_title"]["locator"] = locator
                with self.assertRaisesRegex(ValueError, "locator"):
                    self.plan(source_evidence=evidence)

    def test_unknown_year_and_author_need_explicit_reasons(self):
        with self.assertRaisesRegex(ValueError, "no_year_reason"):
            self.plan(year=None)
        with self.assertRaisesRegex(ValueError, "unknown_author_reason"):
            self.plan(author_token="未知作者")
        plan = self.plan(year=None, no_year_reason="Synthetic source lacks a reliable date.",
                         author_token="未知作者", unknown_author_reason="Synthetic source has no attributable author.")
        self.assertEqual(plan["filename"], "无年份-未知作者-Model measurements.md")
        self.assertIsNone(plan["year"])

    def test_year_is_explicit_publication_year_not_a_coerced_value(self):
        for year in (True, "2031", 0, 10000):
            with self.subTest(year=year), self.assertRaises(ValueError):
                self.plan(year=year)

    def test_author_token_is_not_split_or_inferred(self):
        for token in ("North Ridge Collective", "de Vale and Xu", "许知远", "Vale et al"):
            with self.subTest(token=token):
                plan = self.plan(author_token=token)
                self.assertEqual(plan["author_token"], token)
                self.assertTrue(plan["filename"].startswith(f"2031-{token}-"))

    def test_illegal_characters_have_exact_reversible_mappings(self):
        plan = self.plan(author_token="de/Vale", original_title='A:B?C\x00D|E\\F<>"*')
        self.assertEqual(plan["filename"], "2031-de_Vale-A_B_C_D_E_F____.md")
        changes = plan["transformations"]
        self.assertTrue(any(row["reason"] == "control_character" for row in changes))
        self.assertEqual(plan["character_checks"]["illegal_characters"], [])
        self.assertEqual(plan["character_checks"]["control_characters"], [])
        rebuilt = list(plan["title_mapping"]["cleaned"])
        for row in changes:
            if row["field"] != "original_title":
                continue
            rebuilt[row["start"]:row["end"]] = row["before"]
        self.assertEqual("".join(rebuilt), plan["original_title"])

    def test_legal_punctuation_and_internal_trailing_title_spaces_are_preserved(self):
        title = "A [model] #2 ^note, CON. "
        plan = self.plan(original_title=title)
        self.assertEqual(plan["title_mapping"]["used"], title)
        self.assertEqual(plan["transformations"], [])
        self.assertEqual(plan["character_checks"]["link_ambiguity_characters"], ["#", "[", "]", "^"])
        self.assertFalse(plan["character_checks"]["trailing_dot"])
        self.assertFalse(plan["character_checks"]["trailing_space"])
        self.assertFalse(plan["character_checks"]["reserved_name"])
        self.assertEqual(self.plan(original_title="CON")["filename"], "2031-Vale-CON.md")

    def test_utf16_limit_truncates_title_only_without_splitting_astral_character(self):
        author = "North Ridge Collective et al"
        title = "🧭" * 180
        plan = self.plan(author_token=author, original_title=title)
        self.assertTrue(plan["filename"].startswith(f"2031-{author}-"))
        self.assertLessEqual(len(plan["filename"].encode("utf-16-le")) // 2, 250)
        self.assertEqual(plan["title_mapping"]["original"], title)
        self.assertTrue(plan["title_mapping"]["truncated"])
        used = plan["title_mapping"]["used"]
        self.assertEqual(used, "🧭" * len(used))
        self.assertGreater(len((f"2031-{author}-{used}🧭.md").encode("utf-16-le")) // 2, 250)

    def test_overlong_author_prefix_blocks_instead_of_shortening_author(self):
        author = "Q" * 260
        plan = self.plan(author_token=author)
        self.assertEqual(plan["status"], "blocked")
        self.assertIsNone(plan["filename"])
        self.assertEqual(plan["author_mapping"]["cleaned"], author)
        self.assertFalse(any(row["field"] == "author_token" for row in plan["transformations"]))

    def test_no_complete_title_character_fits(self):
        fixed_units = len("2031-Vale-.md")
        plan = self.plan(original_title="🧭", max_utf16_units=fixed_units + 1)
        self.assertEqual(plan["status"], "blocked")
        self.assertIsNone(plan["filename"])

    def test_lone_unicode_surrogates_and_excessive_limit_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "surrogate"):
            self.plan(original_title="invalid\ud800")
        with self.assertRaisesRegex(ValueError, "250"):
            self.plan(max_utf16_units=251)

    def test_explicit_byte_limit_also_truncates_only_the_title(self):
        fixed_bytes = len("2031-Vale-.md".encode("utf-8"))
        plan = self.plan(original_title="样" * 20, filename_max_utf8_bytes=fixed_bytes + 6)
        self.assertEqual(plan["filename"], "2031-Vale-样样.md")
        self.assertEqual(plan["length"]["filename_utf8_bytes"], fixed_bytes + 6)

    def test_stable_suffix_is_explicit_evidenced_and_included_in_length(self):
        with self.assertRaisesRegex(ValueError, "source field"):
            self.plan(disambiguation_suffix="KEY123")
        suffix_evidence = {"file": file_record(self.source), "locator": {"json_pointer": "/item_key"}}
        plan = self.plan(original_title="z" * 300, disambiguation_suffix="group9-KEY123", suffix_evidence=suffix_evidence)
        self.assertTrue(plan["filename"].endswith("-group9-KEY123.md"))
        self.assertEqual(plan["length"]["filename_utf16_units"], 250)
        self.assertEqual(plan["identity_review"]["status"], "not_assessed")
        with self.assertRaisesRegex(ValueError, "alphanumeric"):
            self.plan(disambiguation_suffix="../KEY123", suffix_evidence=suffix_evidence)

    def test_path_limits_are_unknown_until_explicitly_supplied(self):
        target = self.base.joinpath("target")
        self.assertEqual(self.plan(target_directory=target)["path_check"]["status"], "not_assessed")
        short = self.plan(target_directory=target, full_path_max_utf16_units=10)
        self.assertEqual(short["status"], "blocked")
        self.assertFalse(short["path_check"]["within_declared_limit"])
        enough = self.plan(target_directory=target, full_path_max_utf16_units=10000)
        self.assertTrue(enough["path_check"]["within_declared_limit"])
        self.assertEqual(enough["library_conflicts"]["status"], "not_checked")
        with self.assertRaisesRegex(ValueError, "target_directory"):
            self.plan(full_path_max_utf16_units=10000)


class NamingPlanValidationTests(_NamingPlanFixture, unittest.TestCase):
    def validate(self, plan, **kwargs):
        before = deepcopy(plan)
        source_before = self.source.read_bytes() if self.source.exists() else None
        report = helpers.validate_naming_plan(plan, **kwargs)
        self.assertEqual(plan, before)
        if source_before is not None:
            self.assertEqual(self.source.read_bytes(), source_before)
        return report

    def assert_valid(self, plan, **kwargs):
        report = self.validate(plan, **kwargs)
        self.assertEqual(report["findings"], [])
        self.assertTrue(report["structurally_consistent"])
        self.assertTrue(report["recomputed"])
        self.assertEqual(report["semantic_review"]["status"], "not_assessed")
        self.assertEqual(report["library_conflicts"]["status"], "not_assessed")
        self.assertFalse(report["authorization_granted"])
        return report

    def test_complete_plan_is_recomputed_and_never_writes_the_fixed_delivery(self):
        candidate = self.base.joinpath("candidate.md")
        candidate.write_text("Synthetic existing delivery.", encoding="utf-8")
        before = candidate.read_bytes()
        plan = self.plan(delivery_path=candidate)
        report = self.assert_valid(json.loads(json.dumps(plan)), delivery_path=candidate)
        self.assertEqual(report["validated_source_fields"], ["year", "author_token", "original_title"])
        self.assertEqual(candidate.read_bytes(), before)
        self.assertFalse(self.base.joinpath(plan["filename"]).exists())

    def test_filename_only_record_cannot_stand_in_for_a_plan(self):
        report = self.validate({"status": "planned", "filename": "2031-Vale-Model measurements.md"})
        self.assertFalse(report["structurally_consistent"])
        missing = {row["field"] for row in report["findings"] if row["code"] == "NAMING_PLAN_FIELD_MISSING"}
        self.assertTrue({"/original_title", "/title_mapping", "/transformations", "/source_evidence", "/length", "/character_checks"} <= missing)
        self.assertFalse(report["recomputed"])

    def test_missing_mapping_member_is_reported_at_that_field(self):
        plan = self.plan()
        del plan["title_mapping"]["cleaned"]
        report = self.validate(plan)
        self.assertEqual([(row["code"], row["field"]) for row in report["findings"]],
                         [("NAMING_PLAN_FIELD_MISSING", "/title_mapping/cleaned")])

    def test_consistently_forged_used_title_and_filename_are_rejected(self):
        plan = self.plan(original_title="Synthetic extended title")
        plan["title_mapping"]["used"] = "Synthetic"
        plan["title_mapping"]["truncated"] = True
        plan["filename"] = "2031-Vale-Synthetic.md"
        fields = {row["field"] for row in self.validate(plan)["findings"]}
        self.assertTrue({"/title_mapping/used", "/title_mapping/truncated", "/filename"} <= fields)

    def test_illegal_character_change_must_be_recorded_with_exact_transformation(self):
        plan = self.plan(original_title="A:B", author_token="de/Vale")
        self.assert_valid(plan)
        plan["transformations"][0]["start"] = 99
        report = self.validate(plan)
        self.assertEqual([(row["code"], row["field"]) for row in report["findings"]],
                         [("NAMING_PLAN_TRANSFORMATION_MISMATCH", "/transformations")])

    def test_forged_length_and_character_results_are_not_trusted(self):
        plan = self.plan(original_title="Model [variant]")
        plan["length"]["filename_utf16_units"] = 1
        plan["character_checks"]["link_ambiguity_characters"] = []
        report = self.validate(plan)
        fields = {row["field"] for row in report["findings"]}
        self.assertEqual(fields, {"/length/filename_utf16_units", "/character_checks/link_ambiguity_characters"})

    def test_boolean_and_integer_records_are_not_interchangeable(self):
        plan = self.plan()
        plan["title_mapping"]["truncated"] = 0
        report = self.validate(plan)
        self.assertEqual(report["findings"][0]["field"], "/title_mapping/truncated")

    def test_each_field_binding_is_checked_even_when_another_is_invalid(self):
        plan = self.plan()
        plan["source_evidence"]["year"]["file"]["sha256"] = "0" * 64
        del plan["source_evidence"]["author_token"]["locator"]
        report = self.validate(plan)
        self.assertEqual({row["field"] for row in report["findings"]},
                         {"/source_evidence/year", "/source_evidence/author_token"})
        self.assertEqual(report["validated_source_fields"], ["original_title"])
        self.assertFalse(report["recomputed"])

    def test_changed_source_file_invalidates_all_bound_fields(self):
        plan = self.plan()
        self.source.write_text("Changed invented metadata.", encoding="utf-8")
        report = self.validate(plan)
        self.assertEqual({row["field"] for row in report["findings"]},
                         {"/source_evidence/year", "/source_evidence/author_token", "/source_evidence/original_title"})
        self.assertTrue(all("SHA-256 mismatch" in row["message"] for row in report["findings"]))

    def test_unknown_fields_retain_explicit_reasons(self):
        plan = self.plan(year=None, no_year_reason="Invented source has no year.",
                         author_token="未知作者", unknown_author_reason="Invented source has no author.")
        self.assert_valid(plan)
        plan["no_year_reason"] = ""
        report = self.validate(plan)
        self.assertEqual(report["findings"][0]["field"], "/no_year_reason")

    def test_unicode_truncation_byte_budget_and_evidenced_suffix_remain_valid(self):
        evidence = {"file": file_record(self.source), "locator": {"json_pointer": "/item_key"}}
        plan = self.plan(original_title="🧭样" * 100, disambiguation_suffix="group9-KEY123",
                         suffix_evidence=evidence, filename_max_utf8_bytes=160)
        self.assertTrue(plan["title_mapping"]["truncated"])
        self.assert_valid(plan)
        plan["suffix_evidence"]["file"]["sha256"] = "0" * 64
        report = self.validate(plan)
        self.assertEqual(report["findings"][0]["field"], "/suffix_evidence")

    def test_blocked_record_is_consistent_but_never_ready_by_implication(self):
        plans = [self.plan(author_token="Q" * 260),
                 self.plan(target_directory=self.base.joinpath("target"), full_path_max_utf16_units=10)]
        for plan in plans:
            with self.subTest(filename=plan["filename"]):
                report = self.assert_valid(plan)
                self.assertEqual(report["plan_status"], "blocked")
                plan["status"] = "planned"
                self.assertIn("NAMING_PLAN_STATUS_MISMATCH", {row["code"] for row in self.validate(plan)["findings"]})

    def test_legacy_version_one_plans_without_target_directory_remain_valid(self):
        for arguments in ({}, {"target_directory": self.base.joinpath("target")},
                          {"target_directory": self.base.joinpath("target"), "author_token": "Q" * 260,
                           "full_path_max_utf16_units": 1000}):
            with self.subTest(arguments=arguments):
                plan = self.plan(**arguments)
                del plan["target_directory"]
                self.assert_valid(plan)

    def test_unknown_path_limits_stay_unknown_even_without_a_vault(self):
        for target in (None, self.base.joinpath("not-created")):
            with self.subTest(target=target):
                plan = self.plan(target_directory=target)
                self.assert_valid(plan)
                self.assertEqual(plan["path_check"]["status"], "not_assessed")
                plan["path_check"]["within_declared_limit"] = True
                report = self.validate(plan)
                self.assertEqual(report["findings"][0]["field"], "/path_check/within_declared_limit")

    def test_workflow_delivery_binding_is_independent_of_proposed_name(self):
        delivery = self.base.joinpath("fixed.md")
        plan = self.plan(delivery_path=delivery)
        self.assert_valid(plan, delivery_path=delivery)
        report = self.validate(plan, delivery_path=self.base.joinpath("another.md"))
        self.assertEqual(report["findings"][0]["code"], "NAMING_PLAN_DELIVERY_MISMATCH")
        self.assertFalse(delivery.exists())

    def test_target_filename_cannot_be_silently_replaced(self):
        plan = self.plan(target_directory=self.base.joinpath("target"))
        plan["target_path"] = str(self.base.joinpath("target", "other.md"))
        report = self.validate(plan)
        self.assertEqual(report["findings"][0]["field"], "/target_path")

    def test_malformed_inputs_return_diagnostics_instead_of_crashing(self):
        for value in (None, "filename.md", [], 7):
            with self.subTest(value=value):
                report = helpers.validate_naming_plan(value)
                self.assertEqual(report["findings"][0]["code"], "NAMING_PLAN_SHAPE")
        for field, value in (("length", []), ("path_check", None), ("original_title", 7),
                             ("delivery_path", []), ("author_mapping", []), ("schema_version", True)):
            with self.subTest(field=field):
                plan = self.plan()
                plan[field] = value
                self.assertFalse(self.validate(plan)["structurally_consistent"])

    def test_consistency_does_not_authenticate_metadata_semantics_or_library_claims(self):
        plan = self.plan(year=2044, original_title="A different invented title")
        plan["library_conflicts"] = {"status": "caller_claims_clear"}
        plan["semantic_review"] = {"status": "caller_claims_passed"}
        report = self.assert_valid(plan)
        self.assertEqual(report["semantic_review"]["status"], "not_assessed")
        self.assertEqual(report["library_conflicts"]["status"], "not_assessed")


class LocalConflictTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)

    def compare(self, filename, **changes):
        arguments = {"filename": filename, "directory": self.base, "case_sensitive": True,
                     "unicode_normalization": "none"}
        arguments.update(changes)
        return helpers.check_local_name_conflicts(**arguments)

    def test_case_policy_is_explicit_and_conflict_does_not_resolve_identity(self):
        path = self.base.joinpath("Entry.md")
        path.write_text("synthetic existing item", encoding="utf-8")
        self.assertEqual(self.compare("entry.md")["status"], "clear_in_directory_snapshot")
        collision = self.compare("entry.md", case_sensitive=False)
        self.assertEqual(collision["status"], "name_collision")
        self.assertEqual(collision["matches"], [{"name": path.name, "path": str(path.resolve())}])
        self.assertEqual(collision["identity_review"]["status"], "not_assessed")
        self.assertEqual(collision["library_conflicts"]["status"], "not_checked")
        self.assertFalse(collision["overwrite_authorized"])
        self.assertEqual(path.read_text(encoding="utf-8"), "synthetic existing item")

    def test_unicode_normalization_is_declared_not_inferred_from_host(self):
        self.base.joinpath("A\u030a-model.md").write_text("synthetic", encoding="utf-8")
        self.assertEqual(self.compare("Å-model.md")["status"], "clear_in_directory_snapshot")
        for normalization in ("NFC", "NFD"):
            with self.subTest(normalization=normalization):
                self.assertEqual(self.compare("Å-model.md", unicode_normalization=normalization)["status"], "name_collision")

    def test_snapshot_scope_does_not_include_subdirectories_or_later_files(self):
        child = self.base.joinpath("child")
        child.mkdir()
        child.joinpath("target.md").write_text("synthetic", encoding="utf-8")
        report = self.compare("target.md")
        self.assertEqual(report["status"], "clear_in_directory_snapshot")
        self.assertEqual(report["entries"], ["child"])
        digest = sha256(json.dumps(report["entries"], ensure_ascii=False, separators=(",", ":")).encode("utf-8")).hexdigest()
        self.assertEqual(report["entry_names_sha256"], digest)
        self.base.joinpath("target.md").mkdir()
        self.assertEqual(self.compare("target.md")["status"], "name_collision")
        self.assertEqual(report["status"], "clear_in_directory_snapshot")

    def test_missing_directory_or_undeclared_policy_cannot_claim_clear(self):
        with self.assertRaisesRegex(ValueError, "does not exist"):
            self.compare("entry.md", directory=self.base.joinpath("absent"))
        with self.assertRaisesRegex(ValueError, "explicitly"):
            self.compare("entry.md", case_sensitive="false")
        with self.assertRaisesRegex(ValueError, "normalization"):
            self.compare("entry.md", unicode_normalization="guess")
        with self.assertRaisesRegex(ValueError, "directory-entry"):
            self.compare("../entry.md")


class FigureTableAnchorTests(unittest.TestCase):
    def test_new_anchors_follow_required_order_and_keep_source_tokens(self):
        self.assertEqual(helpers.new_figure_table_anchor(source_token="main", figure_number_token="7"), "^tbl-fig-main-7")
        self.assertEqual(helpers.new_figure_table_anchor(source_token="a92c17e84b60", figure_number_token="S7"),
                         "^tbl-fig-a92c17e84b60-S7")

    def test_occupied_anchor_is_rejected_without_migration_or_suffix(self):
        for occupied in ({"^tbl-fig-main-7"}, {"tbl-fig-main-7"}):
            before = occupied.copy()
            with self.subTest(occupied=occupied), self.assertRaisesRegex(ValueError, "occupied"):
                helpers.new_figure_table_anchor(source_token="main", figure_number_token="7", occupied_ids=occupied)
            self.assertEqual(occupied, before)

    def test_existing_anchor_is_reused_only_by_the_caller(self):
        old_anchor = "^legacy-data-42"
        occupied = {old_anchor}
        anchor = old_anchor
        if anchor is None:
            anchor = helpers.new_figure_table_anchor(source_token="main", figure_number_token="7", occupied_ids=occupied)
        self.assertEqual(anchor, old_anchor)
        self.assertEqual(occupied, {old_anchor})

    def test_unsafe_or_unregistered_token_forms_are_not_silently_normalized(self):
        for token in ("", " S7", "图7", "7.2", "old-token", "7/8", "^S7"):
            with self.subTest(token=token), self.assertRaisesRegex(ValueError, "registered"):
                helpers.new_figure_table_anchor(source_token="main", figure_number_token=token)
        for occupied in ("legacy", None, [1]):
            with self.subTest(occupied=occupied), self.assertRaisesRegex(ValueError, "collection"):
                helpers.new_figure_table_anchor(source_token="main", figure_number_token="7", occupied_ids=occupied)


class VisibleTextEditTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.text_path = self.base.joinpath("synthetic-text.txt")
        self.cluster = "-\u00ad\u2010"
        self.original = f"A micro{self.cluster}mesh B\r\nC micro{self.cluster}mesh D; alpha--beta; 6−2; 3–5.\r\nTypo: teh."
        self.text_path.write_bytes(self.original.encode("utf-8"))
        self.page_path = self.base.joinpath("synthetic-visible-page.svg")
        self.page_path.write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" width="500" height="160">'
            '<text x="10" y="30">micro-mesh / alpha--beta / 6−2 / 3–5 / teh</text></svg>',
            encoding="utf-8",
        )

    def edit(self, *, start=None, identifier="cluster-a", expected=None, selected="-"):
        expected = self.cluster if expected is None else expected
        start = self.original.index(expected) if start is None else start
        return {
            "id": identifier, "start": start, "end": start + len(expected), "expected_text": expected,
            "visible_page": {"file": file_record(self.page_path), "page": 1, "bbox": [10, 10, 490, 40],
                             "coordinate_system": "rendered_image_pixels", "text": selected},
            "decision": {"status": "resolved", "selected_text": selected,
                         "reason": "Synthetic fixture declares this exact transcription.",
                         "reviewer": "synthetic-test-declaration", "observed": True},
        }

    def apply(self, edits, **changes):
        arguments = {"text_file": file_record(self.text_path), "edits": edits}
        arguments.update(changes)
        return helpers.apply_visible_text_edits(**arguments)

    def test_cluster_is_replaced_once_with_one_hyphen_and_full_source_record(self):
        edit = self.edit()
        result = self.apply([edit])
        expected = self.original[:edit["start"]] + "-" + self.original[edit["end"]:]
        self.assertEqual(result["text"], expected)
        self.assertIn("A micro-mesh B", result["text"])
        self.assertIn(f"C micro{self.cluster}mesh D", result["text"])
        self.assertEqual(self.text_path.read_bytes(), self.original.encode("utf-8"))
        conflict = result["text_visual_conflicts"][0]
        self.assertEqual(conflict["text_layer"]["text"], self.cluster)
        self.assertEqual(conflict["visible_page"]["file"], file_record(self.page_path.resolve()))
        self.assertEqual(conflict["text_layer"]["locator"]["start"], edit["start"])
        self.assertEqual(conflict["decision"]["declaration_origin"], "caller")
        self.assertFalse(result["observation_certified"])
        self.assertEqual(result["semantic_review"]["status"], "not_assessed")

    def test_no_global_cleanup_of_hyphens_minus_ranges_or_spelling(self):
        result = self.apply([])
        self.assertEqual(result["text"], self.original)
        self.assertEqual(result["text_visual_conflicts"], [])
        changed = self.apply([self.edit()])["text"]
        self.assertIn("alpha--beta; 6−2; 3–5.", changed)
        self.assertTrue(changed.endswith("Typo: teh."))

    def test_multiple_edits_use_original_offsets_even_when_lengths_change(self):
        first = self.edit()
        second_start = self.original.index(self.cluster, first["end"])
        second = self.edit(start=second_start, identifier="cluster-b")
        result = self.apply([second, first])
        self.assertEqual(result["text"], self.original.replace(self.cluster, "-"))
        self.assertEqual([row["id"] for row in result["text_visual_conflicts"]], ["cluster-a", "cluster-b"])

    def test_visible_double_hyphen_is_preserved_when_explicitly_recorded(self):
        page = self.page_path.read_text(encoding="utf-8")
        self.page_path.write_text(page.replace("micro-mesh", "micro--mesh"), encoding="utf-8")
        result = self.apply([self.edit(selected="--")])
        self.assertIn("A micro--mesh B", result["text"])
        self.assertIn("alpha--beta", result["text"])

    def test_disagreement_between_selected_and_recorded_reading_is_rejected(self):
        edit = self.edit()
        edit["decision"]["selected_text"] = "--"
        with self.assertRaisesRegex(ValueError, "recorded visible reading"):
            self.apply([edit])

    def test_explicit_empty_transcription_is_not_global_soft_hyphen_removal(self):
        self.original = "inter\u00adlock and inter\u00adlock"
        self.text_path.write_bytes(self.original.encode("utf-8"))
        page = self.page_path.read_text(encoding="utf-8")
        self.page_path.write_text(page.replace("micro-mesh", "interlock"), encoding="utf-8")
        edit = self.edit(expected="\u00ad", selected="")
        result = self.apply([edit])
        self.assertEqual(result["text"].count("\u00ad"), 1)
        self.assertEqual(result["text_visual_conflicts"][0]["decision"]["selected_text"], "")

    def test_pending_or_unobserved_decisions_are_not_applied(self):
        for key, value in (("status", "pending"), ("observed", False), ("observed", "true"), ("reviewer", ""), ("reason", "")):
            with self.subTest(key=key, value=value):
                edit = self.edit()
                edit["decision"][key] = value
                with self.assertRaises(ValueError):
                    self.apply([edit])

    def test_missing_or_changed_visible_file_is_rejected(self):
        missing = self.edit()
        del missing["visible_page"]
        with self.assertRaisesRegex(ValueError, "visible-page"):
            self.apply([missing])
        stale = self.edit()
        self.page_path.write_text("changed synthetic snapshot", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            self.apply([stale])

    def test_stale_text_binding_or_expected_span_is_rejected(self):
        stale = file_record(self.text_path)
        stale["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            self.apply([self.edit()], text_file=stale)
        edit = self.edit()
        edit["expected_text"] = "invented"
        with self.assertRaisesRegex(ValueError, "stale or incorrect"):
            self.apply([edit])

    def test_overlapping_ranges_and_duplicate_ids_are_rejected(self):
        first = self.edit()
        second = self.edit(identifier="overlap")
        with self.assertRaisesRegex(ValueError, "overlap"):
            self.apply([first, second])
        with self.assertRaisesRegex(ValueError, "duplicate edit ID"):
            self.apply([first, deepcopy(first)])

    def test_page_bbox_and_coordinate_system_are_explicit(self):
        for key, value in (("page", 0), ("bbox", [0, 0, 0, 3]), ("bbox", [0, 0, float("nan"), 3]),
                           ("coordinate_system", None), ("coordinate_system", "   ")):
            with self.subTest(key=key, value=value):
                edit = self.edit()
                edit["visible_page"][key] = value
                with self.assertRaises(ValueError):
                    self.apply([edit])


if __name__ == "__main__":
    unittest.main(argv=[__file__])
