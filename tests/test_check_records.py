"""Atomic-check ledgers tested against small generated skills, PDFs and text."""

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
from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-zotero2obsidian", "scripts", "check_records.py")
SPEC = importlib.util.spec_from_file_location("test_atomic_check_records", SCRIPT)
assert SPEC and SPEC.loader
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)


class CheckRecordsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.skills = self.base.joinpath("skills")
        first = self.skills.joinpath("sample-extract")
        second = self.skills.joinpath("sample-format")
        first.joinpath("references").mkdir(parents=True)
        second.mkdir(parents=True)
        self.first_skill = first.joinpath("SKILL.md")
        self.first_skill.write_text(
            "---\nname: sample-extract\n---\n# Extraction\n"
            "- [EX-C01] Retain the source paragraph.\n"
            "- [ ] [EX-A01] Observe extraction before editing. <!-- check-type: execution_process -->\n"
            "- [ ] [EX-A02] Inspect the heading in the application. <!-- check-type: live_environment -->\n"
            "- [ ] [EX-A03] Click the citation in the application. <!-- check-type: live_environment -->\n"
            "Mention [FAKE-C01] is a reference, not a declaration.\n"
            "- [FAKE-C02](elsewhere.md) is only a linked reference.\n"
            "```markdown\n- [FAKE-C03] Code is not a rule.\n```\n"
            "<!--\n- [FAKE-C04] A commented example is not a rule.\n-->\n",
            encoding="utf-8")
        self.acceptance = first.joinpath("references", "acceptance.md")
        self.acceptance.write_text(
            "# Acceptance\n- [AC-C01] Record the actual scope.\n"
            "- [ ] [AC-A01] A conditional feature has a result.\n",
            encoding="utf-8")
        second.joinpath("SKILL.md").write_text(
            "# Formatting\n- [FMT-C01] Preserve the visible equation.\n"
            "- [ ] [FMT-A01] Each table cell has evidence.\n", encoding="utf-8")
        # A nested archive is not part of the immediate skill installation.
        archive = self.skills.joinpath("archive", "sample-extract")
        archive.mkdir(parents=True)
        archive.joinpath("SKILL.md").write_text("- [OLD-C01] Old rule.\n", encoding="utf-8")
        self.ids = {"EX-C01", "EX-A01", "EX-A02", "EX-A03", "AC-C01", "AC-A01", "FMT-C01", "FMT-A01"}
        self.pdf = self.base.joinpath("source.pdf")
        with fitz.open() as document:
            page = document.new_page(width=240, height=320)
            page.insert_text((25, 45), "A small synthetic source paragraph.")
            page.insert_text((25, 75), "A single-cell table: 7.")
            document.save(self.pdf)
        self.candidate = self.base.joinpath("candidate.md")
        self.candidate.write_text("# Candidate\n\nA source paragraph.\n\n| Value |\n|---|\n| 7 |\n", encoding="utf-8")
        self.observations = self.base.joinpath("observations.json")
        self.observations.write_text(json.dumps({"events": [{"event": "inspect", "target": "synthetic fixture"}],
                                                "table": {"row/one": {"~value": 7}}}), encoding="utf-8")
        self.image = self.base.joinpath("image.png")
        Image.new("RGB", (16, 12), "white").save(self.image)
        self.inventory = self.base.joinpath("inventory.json")
        self.inventory.write_text('{"sources": ["source-a"]}\n', encoding="utf-8")
        self.path = self.base.joinpath("work", "checks.json")
        self.scopes = {identity: [{"object_id": "source-a:p1", "locator": "PDF page 1 / candidate paragraph"}]
                       for identity in self.ids}
        self.types = {identity: "offline_outcome" for identity in self.ids}
        self.types.update({"EX-A01": "execution_process", "EX-A02": "live_environment", "EX-A03": "live_environment"})
        self.prepare()

    def prepare(self, **options):
        arguments = {"sources": {"source-a": self.pdf}, "candidate_path": self.candidate,
                     "scopes": self.scopes, "requirement_types": self.types, "rule_selection": "all", "progress": None}
        arguments.update(options)
        self.path.unlink(missing_ok=True)
        checker.prepare_check_records(self.skills, self.path, **arguments)
        self.ledger = json.loads(self.path.read_text(encoding="utf-8"))

    def save(self):
        self.path.write_text(json.dumps(self.ledger, ensure_ascii=False, indent=2), encoding="utf-8")

    def report(self):
        self.save()
        return checker.validate_check_records(self.path)

    def codes(self):
        return {finding.code for finding in self.report().findings}

    def record(self, identity):
        return next(item for item in self.ledger["checks"] if item["id"] == identity)

    def bind(self, identity="text", path=None, locator=None, **options):
        self.ledger["evidence"][identity] = checker.bind_evidence(
            path or self.candidate, locator or {"kind": "lines", "start": 1, "end": 3}, **options)
        return identity

    def resolve(self, identity="EX-C01", *, status="passed", evidence=None):
        entry = self.record(identity)
        entry.update(status=status, evidence=evidence or [self.bind()], reason="Observed the stated synthetic object.")
        return entry

    def observation(self, *, ui=False):
        value = {"observed": True, "basis": "actual_ui" if ui else "direct_event",
                 "observer": "synthetic-test-observer", "observed_at": "2026-01-01T10:00:00+00:00"}
        if ui:
            value["environment"] = "Synthetic UI fixture; no real application acceptance claimed"
        return value

    def test_skeleton_discovers_all_atomic_ids_without_resolving_any(self):
        records = self.ledger["checks"]
        self.assertEqual({item["id"] for item in records}, self.ids)
        self.assertEqual(len(records), len(self.ids))
        self.assertEqual({item["status"] for item in records}, {"not_verified"})
        self.assertTrue(all(item["evidence"] == [] for item in records))
        self.assertEqual({self.record(identity)["status"] for identity in ("EX-A02", "EX-A03")}, {"not_verified"})
        report = self.report()
        self.assertEqual(report.findings, [])
        self.assertTrue(report.structurally_consistent)
        self.assertFalse(report.recorded_checks_complete)
        self.assertFalse(report.offline_records_ready)
        self.assertEqual(set(report.pending_ids), self.ids)

    def test_offline_completion_is_separate_from_record_shape(self):
        evidence = self.bind()
        offline = [entry for entry in self.ledger["checks"] if entry["requirement_type"] == "offline_outcome"]
        for entry in offline:
            self.resolve(entry["id"], evidence=[evidence])
        report = self.report()
        self.assertTrue(report.structurally_consistent)
        self.assertTrue(report.offline_records_ready)
        self.assertFalse(report.recorded_checks_complete)
        self.assertEqual(report.process_unverified_ids, ["EX-A01"])
        self.assertEqual(set(report.live_unverified_ids), {"EX-A02", "EX-A03"})
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_each_unresolved_offline_status_blocks_readiness(self):
        evidence = self.bind()
        for entry in self.ledger["checks"]:
            if entry["requirement_type"] == "offline_outcome":
                self.resolve(entry["id"], evidence=[evidence])
        for status in ("not_verified", "blocked", "failed"):
            with self.subTest(status=status):
                self.record("FMT-A01").update(status=status, reason="A concrete unresolved synthetic cell.")
                report = self.report()
                self.assertTrue(report.structurally_consistent)
                self.assertFalse(report.offline_records_ready)
                self.assertEqual(report.offline_unresolved_ids, ["FMT-A01"])
                self.assertEqual(report.offline_failed_ids, ["FMT-A01"] if status == "failed" else [])

    def test_unclassified_pending_record_cannot_escape_offline_readiness(self):
        evidence = self.bind()
        for entry in self.ledger["checks"]:
            if entry["requirement_type"] == "offline_outcome":
                self.resolve(entry["id"], evidence=[evidence])
        self.record("FMT-A01").update(requirement_type="unclassified", status="not_verified", evidence=[],
                                     reason="Its boundary has not been decided.")
        report = self.report()
        self.assertTrue(report.structurally_consistent)
        self.assertEqual(report.offline_unresolved_ids, [])
        self.assertFalse(report.offline_records_ready)

    def test_explicit_scopes_keep_each_table_source_and_target_separate(self):
        with fitz.open(self.pdf) as document:
            document.new_page(width=240, height=320).insert_text((25, 75), "A second table: 8.")
            document.saveIncr()
        self.candidate.write_text(
            "# Candidate\n\nFirst table. ^tbl-1\n\nSecond table. ^tbl-2\n", encoding="utf-8")
        inventory = {"sources": [{"id": "source-a", "page_count": 2}], "units": [
            {"id": "table-one", "source": "source-a", "evidence": [{"page": 1, "quote": "A single-cell table: 7."}]},
            {"id": "table-two", "source": "source-a", "evidence": [{"page": 2, "quote": "A second table: 8."}]},
        ]}
        self.inventory.write_text(json.dumps(inventory), encoding="utf-8")
        deliveries = [{"unit": "table-one", "target": {"anchor": "tbl-1"}},
                      {"unit": "table-two", "target": {"anchor": "tbl-2"}}]
        scopes = checker.scopes_from_deliveries(self.inventory, self.candidate, deliveries,
                                                unit_ids=["table-two", "table-one"])
        self.assertEqual([row["object_id"] for row in scopes], ["table-two", "table-one"])
        self.assertEqual([row["locator"]["source_locations"][0]["page"] for row in scopes], [2, 1])
        self.assertEqual([row["locator"]["candidate_target"]["anchor"] for row in scopes], ["tbl-2", "tbl-1"])
        self.assertTrue(all("status" not in row and "evidence" not in row for row in scopes))
        with self.assertRaisesRegex(ValueError, "lack source units"):
            checker.scopes_from_deliveries(self.inventory, self.candidate, deliveries, unit_ids=["missing"])
        with self.assertRaisesRegex(ValueError, "explicit source unit IDs"):
            checker.scopes_from_deliveries(self.inventory, self.candidate, deliveries, unit_ids=[])
        with self.assertRaisesRegex(ValueError, "unique nonempty"):
            checker.scopes_from_deliveries(self.inventory, self.candidate, deliveries + deliveries, unit_ids=["table-one"])

    def scope_fixture(self):
        self.candidate.write_text("# Candidate\n\nA source paragraph. ^present\n", encoding="utf-8")
        inventory = {"sources": [{"id": "source-a", "page_count": 1}], "units": [
            {"id": "paragraph", "source": "source-a",
             "evidence": [{"page": 1, "quote": "A small synthetic source paragraph."}]},
        ]}
        return inventory, [{"unit": "paragraph", "target": {"anchor": "present"}}]

    def selected_scope(self, inventory, deliveries):
        self.inventory.write_text(json.dumps(inventory), encoding="utf-8")
        return checker.scopes_from_deliveries(self.inventory, self.candidate, deliveries, unit_ids=["paragraph"])

    def test_scopes_reject_missing_hidden_or_duplicate_candidate_anchor(self):
        inventory, deliveries = self.scope_fixture()
        candidates = [
            "# Candidate\n\nNo definition.\n",
            "---\nvalue: text ^present\n---\n\nBody.\n",
            "# Candidate\n\n```markdown\nCode ^present\n```\n",
            "# Candidate\n\n```rich-table\ntext: ^present\n```\n",
            "# Candidate\n\n    Code ^present\n",
            "# Candidate\n\n<!--\nComment ^present\n-->\n",
            "# Candidate\n\n%%\nComment ^present\n%%\n",
            "# Candidate\n\n`Multiline code\ntext ^present\n`\n",
            "# Candidate\n\n$$\nx ^present\n$$\n",
            "# Candidate\n\nEscaped \\^present\n",
            "# Candidate\n\nFirst ^present\n\nSecond ^present\n",
        ]
        for text in candidates:
            with self.subTest(candidate=text):
                self.candidate.write_text(text, encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "one visible block definition"):
                    self.selected_scope(inventory, deliveries)
        self.candidate.write_text("```markdown\nFake ^present\n```\n\nActual content. ^present\n", encoding="utf-8")
        self.assertEqual(self.selected_scope(inventory, deliveries)[0]["object_id"], "paragraph")

    def test_scopes_require_bounded_candidate_lines_and_their_actual_hash(self):
        inventory, deliveries = self.scope_fixture()
        for bounds in ([999, 999], [0, 1], [3, 2], [True, 2], [1]):
            with self.subTest(bounds=bounds):
                deliveries[0]["target"] = {"lines": bounds}
                with self.assertRaisesRegex(ValueError, "one-based inclusive range"):
                    self.selected_scope(inventory, deliveries)
        deliveries[0]["target"] = {"lines": [3, 3], "sha256": "0" * 64}
        with self.assertRaisesRegex(ValueError, "line-range sha256"):
            self.selected_scope(inventory, deliveries)
        text = self.candidate.read_text(encoding="utf-8").splitlines(keepends=True)[2]
        deliveries[0]["target"]["sha256"] = hashlib.sha256(text.encode("utf-8")).hexdigest()
        self.assertEqual(self.selected_scope(inventory, deliveries)[0]["locator"]["candidate_target"],
                         deliveries[0]["target"])

    def test_scopes_reject_unknown_or_ambiguous_candidate_selector(self):
        inventory, deliveries = self.scope_fixture()
        for target in ({"phantom": "missing"}, {}, None, {"anchor": "present", "quote": "A source paragraph."}):
            with self.subTest(target=target):
                deliveries[0]["target"] = target
                with self.assertRaisesRegex(ValueError, "exactly one candidate target"):
                    self.selected_scope(inventory, deliveries)

    def test_scope_quote_and_line_hash_use_delivery_bom_and_newline_normalization(self):
        inventory, deliveries = self.scope_fixture()
        normalized = "# Candidate\n\nA source paragraph. ^present\n"
        self.candidate.write_bytes(("\ufeff" + normalized.replace("\n", "\r\n")).encode("utf-8"))
        targets = [
            {"quote": "# Candidate\n\nA source paragraph."},
            {"lines": [1, 3], "sha256": hashlib.sha256(normalized.encode("utf-8")).hexdigest()},
        ]
        for target in targets:
            with self.subTest(target=target):
                deliveries[0]["target"] = target
                self.assertEqual(self.selected_scope(inventory, deliveries)[0]["locator"]["candidate_target"], target)

    def test_scopes_reject_source_pages_outside_the_inventory(self):
        inventory, deliveries = self.scope_fixture()
        location = inventory["units"][0]["evidence"][0]
        for page in (999, 0, True, "1"):
            with self.subTest(page=page):
                location["page"] = page
                with self.assertRaisesRegex(ValueError, "within inventory page_count"):
                    self.selected_scope(inventory, deliveries)

    def test_scopes_require_finite_nonempty_source_bbox(self):
        inventory, deliveries = self.scope_fixture()
        location = inventory["units"][0]["evidence"][0]
        del location["quote"]
        for box in ([], None, [0, 0, 0, 1], [1, 0, 0, 1], [0, 0, 1, float("inf")],
                    [0, float("nan"), 1, 1], [False, 0, 1, 1], [0, 0, "1", 1]):
            with self.subTest(bbox=box):
                location["bbox"] = box
                with self.assertRaisesRegex(ValueError, "finite nonempty rectangle"):
                    self.selected_scope(inventory, deliveries)
        location["bbox"] = [0, 0, 120, 80]
        self.assertEqual(self.selected_scope(inventory, deliveries)[0]["locator"]["source_locations"][0], location)

    def test_scopes_require_nonempty_unique_candidate_quote(self):
        inventory, deliveries = self.scope_fixture()
        for quote in (None, "", " ", "absent", "a"):
            with self.subTest(quote=quote):
                deliveries[0]["target"] = {"quote": quote}
                with self.assertRaisesRegex(ValueError, "nonempty and unique"):
                    self.selected_scope(inventory, deliveries)
        deliveries[0]["target"] = {"quote": "A source paragraph."}
        self.assertEqual(self.selected_scope(inventory, deliveries)[0]["object_id"], "paragraph")
        self.candidate.write_text("aaaa\n", encoding="utf-8")
        deliveries[0]["target"] = {"quote": "aaa"}
        with self.assertRaisesRegex(ValueError, "nonempty and unique"):
            self.selected_scope(inventory, deliveries)

    def test_scopes_validate_declared_page_count_quote_and_image_xref(self):
        inventory, deliveries = self.scope_fixture()
        source = inventory["sources"][0]
        for page_count in (None, 0, True, "1"):
            with self.subTest(page_count=page_count):
                source["page_count"] = page_count
                with self.assertRaisesRegex(ValueError, "positive inventory page_count"):
                    self.selected_scope(inventory, deliveries)
        source["page_count"] = 1
        location = inventory["units"][0]["evidence"][0]
        for quote in (None, "", " ", 3):
            with self.subTest(quote=quote):
                location["quote"] = quote
                with self.assertRaisesRegex(ValueError, "source quote"):
                    self.selected_scope(inventory, deliveries)
        del location["quote"]
        for xref in (None, 0, -1, True, "4"):
            with self.subTest(xref=xref):
                location["image_xref"] = xref
                with self.assertRaisesRegex(ValueError, "positive integer"):
                    self.selected_scope(inventory, deliveries)
        location["image_xref"] = 4
        self.assertEqual(self.selected_scope(inventory, deliveries)[0]["locator"]["source_locations"][0], location)

    def test_default_checks_include_all_acceptance_ids_and_catalog_keeps_constraints(self):
        self.path.unlink()
        checker.prepare_check_records(self.skills, self.path, sources={"source-a": self.pdf},
                                      candidate_path=self.candidate, scopes=self.scopes, requirement_types=self.types)
        self.ledger = json.loads(self.path.read_text(encoding="utf-8"))
        self.assertEqual({item["id"] for item in self.ledger["catalog"]}, self.ids)
        self.assertEqual({item["id"] for item in self.ledger["checks"]},
                         {"EX-A01", "EX-A02", "EX-A03", "AC-A01", "FMT-A01"})
        self.assertEqual(self.ledger["rule_selection"], {"mode": "acceptance", "additional_ids": []})
        self.assertTrue(self.report().structurally_consistent)

    def test_explicit_constraints_extend_but_cannot_remove_acceptance_ids(self):
        self.prepare(rule_selection="acceptance", additional_ids=["FMT-C01"])
        self.assertEqual({item["id"] for item in self.ledger["checks"]},
                         {"EX-A01", "EX-A02", "EX-A03", "AC-A01", "FMT-A01", "FMT-C01"})
        self.ledger["checks"] = [item for item in self.ledger["checks"] if item["id"] != "EX-A02"]
        self.assertIn("CHECK_ID_MISSING", self.codes())
        with self.assertRaisesRegex(ValueError, "Unknown additional"):
            self.prepare(additional_ids=["UNKNOWN-C01"])

    def test_full_catalog_cannot_be_replaced_by_only_selected_records(self):
        self.prepare(rule_selection="acceptance")
        self.ledger["catalog"] = [item for item in self.ledger["catalog"] if "-A" in item["id"]]
        self.assertIn("CHECK_CATALOG_ID_MISSING", self.codes())

    def test_no_keyword_automatically_sets_a_type_scope_pass_or_na(self):
        self.prepare(scopes={}, requirement_types={})
        self.assertEqual(self.record("EX-C01")["requirement_type"], "unclassified")
        self.assertEqual(self.record("AC-A01")["status"], "not_verified")
        self.assertEqual(self.record("AC-A01")["scope"], [])
        report = self.report()
        self.assertEqual({finding.code for finding in report.findings}, {"CHECK_SCOPE_EMPTY"})
        self.assertEqual(len(report.findings), len(self.ids))

    def test_hashes_bind_actual_source_candidate_and_rule_files(self):
        for group in self.ledger["bindings"].values():
            for entry in group:
                self.assertEqual(entry["sha256"], hashlib.sha256(Path(entry["path"]).read_bytes()).hexdigest())
        for entry in self.ledger["rule_files"]:
            self.assertEqual(entry["sha256"], hashlib.sha256(Path(entry["path"]).read_bytes()).hexdigest())

    def test_inventory_writeback_does_not_create_an_inventory_hash_cycle(self):
        self.prepare(inventory_path=self.inventory)
        updated = json.loads(self.inventory.read_text(encoding="utf-8"))
        self.assertEqual(Path(updated["checks_path"]).resolve(), self.path.resolve())
        self.assertNotIn(str(self.inventory), json.dumps(self.ledger))
        updated["later_annotation"] = "Mutable inventory may be completed later."
        self.inventory.write_text(json.dumps(updated), encoding="utf-8")
        self.assertTrue(self.report().structurally_consistent)

    def test_inventory_cannot_also_be_a_hash_bound_input(self):
        with self.assertRaisesRegex(ValueError, "mutable inventory"):
            self.prepare(sources={"inventory": self.inventory}, inventory_path=self.inventory)

    def test_prepare_preserves_an_existing_ledger(self):
        before = self.path.read_bytes()
        with self.assertRaises(FileExistsError):
            checker.prepare_check_records(self.skills, self.path, sources={"source-a": self.pdf})
        self.assertEqual(before, self.path.read_bytes())

    def test_prepare_cannot_overwrite_an_input(self):
        before = self.candidate.read_bytes()
        with self.assertRaisesRegex(ValueError, "overwrite a bound input"):
            checker.prepare_check_records(self.skills, self.candidate, sources={"source-a": self.pdf},
                                          candidate_path=self.candidate, overwrite=True)
        self.assertEqual(before, self.candidate.read_bytes())

    def test_unknown_explicit_ids_and_type_conflicts_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "Unknown rule IDs"):
            self.prepare(requirement_types={"NO-C01": "offline_outcome"})
        with self.assertRaisesRegex(ValueError, "conflicts"):
            self.prepare(requirement_types={"EX-A02": "offline_outcome"})

    def test_duplicate_declarations_in_actual_skills_are_rejected(self):
        self.acceptance.write_text(self.acceptance.read_text(encoding="utf-8") + "- [EX-C01] Duplicate.\n", encoding="utf-8")
        self.assertIn("CHECK_RULE_CATALOG_INVALID", self.codes())
        with self.assertRaisesRegex(ValueError, "Duplicate atomic ID EX-C01"):
            self.prepare()

    def test_missing_duplicate_unknown_ids_are_all_diagnosed_without_short_circuit(self):
        self.ledger["checks"] = [item for item in self.ledger["checks"] if item["id"] != "EX-C01"]
        duplicate = deepcopy(self.record("FMT-C01"))
        self.ledger["checks"].append(duplicate)
        unknown = deepcopy(duplicate)
        unknown["id"] = "WHOLE-SKILL-A01"
        self.ledger["checks"].append(unknown)
        codes = self.codes()
        self.assertTrue({"CHECK_ID_MISSING", "CHECK_ID_DUPLICATE", "CHECK_ID_UNKNOWN"} <= codes)

    def test_rule_file_additions_and_rule_text_changes_are_detected(self):
        self.acceptance.write_text(self.acceptance.read_text(encoding="utf-8").replace("actual scope", "precise scope"), encoding="utf-8")
        extra = self.skills.joinpath("sample-extract", "references", "extra.md")
        extra.write_text("- [NEW-C01] A new atomic rule.\n", encoding="utf-8")
        self.assertTrue({"CHECK_HASH_CHANGED", "CHECK_RULE_CHANGED", "CHECK_RULE_FILES_CHANGED", "CHECK_ID_MISSING"} <= self.codes())

    def test_source_and_candidate_hash_changes_are_both_detected(self):
        self.pdf.write_bytes(self.pdf.read_bytes() + b"\n% changed synthetic source\n")
        self.candidate.write_text("Changed candidate.\n", encoding="utf-8")
        findings = self.report().findings
        changed = {item.object_id for item in findings if item.code == "CHECK_HASH_CHANGED"}
        self.assertEqual(changed, {"sources:source-a", "candidates:candidate"})

    def test_empty_scope_and_reasons_are_diagnosed_per_id(self):
        self.record("EX-C01").update(scope=[], reason="")
        self.record("FMT-C01").update(status="blocked", reason=" ")
        findings = self.report().findings
        self.assertIn(("CHECK_SCOPE_EMPTY", "EX-C01"), {(item.code, item.object_id) for item in findings})
        self.assertEqual({item.object_id for item in findings if item.code == "CHECK_REASON_MISSING"}, {"EX-C01", "FMT-C01"})

    def test_honest_blocked_ui_records_are_structurally_valid(self):
        for identity in ("EX-A02", "EX-A03"):
            self.record(identity).update(status="blocked", reason="The actual application is unavailable; this UI object remains unobserved.")
        report = self.report()
        self.assertTrue(report.structurally_consistent)
        self.assertFalse(report.recorded_checks_complete)
        self.assertEqual(set(report.blocked_ids), {"EX-A02", "EX-A03"})

    def test_passed_and_failed_need_actual_evidence(self):
        for identity, status in (("EX-C01", "passed"), ("FMT-C01", "failed")):
            self.record(identity).update(status=status, evidence=[], reason="A conclusion without a location.")
        findings = self.report().findings
        self.assertEqual({item.object_id for item in findings if item.code == "CHECK_RESULT_EVIDENCE_MISSING"}, {"EX-C01", "FMT-C01"})

    def test_na_needs_an_actual_condition_and_reason(self):
        self.record("AC-A01").update(status="not_applicable", reason="", applicability_condition="")
        self.assertTrue({"CHECK_NA_CONDITION_MISSING", "CHECK_REASON_MISSING"} <= self.codes())
        self.record("AC-A01").update(reason="The source contains no video.", applicability_condition="This rule only applies to a delivered video; none is in the declared source scope.")
        self.assertTrue(self.report().structurally_consistent)

    def test_evidence_can_be_shared_without_collapsing_atomic_records(self):
        evidence_id = self.bind()
        for identity in ("EX-C01", "FMT-C01", "FMT-A01"):
            self.resolve(identity, evidence=[evidence_id])
        self.record("FMT-A01")["scope"].append({"object_id": "table-a:r1c1", "locator": "source page 1 / target row 1 cell 1"})
        self.assertTrue(self.report().structurally_consistent)
        self.assertEqual(len(self.ledger["evidence"]), 1)
        self.assertEqual(len(self.ledger["checks"]), len(self.ids))
        self.assertNotIn("A source paragraph.", json.dumps(self.ledger["evidence"]))

    def test_evidence_missing_file_locator_and_hash_changes_are_detected(self):
        evidence_id = self.bind(path=self.observations, locator={"kind": "json_pointer", "pointer": "/events/0"})
        self.resolve(evidence=[evidence_id])
        self.ledger["evidence"][evidence_id]["locator"] = {}
        self.assertIn("CHECK_EVIDENCE_LOCATION_INVALID", self.codes())
        self.ledger["evidence"][evidence_id]["locator"] = {"kind": "json_pointer", "pointer": "/events/0"}
        self.observations.write_text('{"events": []}\n', encoding="utf-8")
        self.assertIn("CHECK_HASH_CHANGED", self.codes())
        self.observations.unlink()
        self.assertIn("CHECK_FILE_MISSING", self.codes())

    def test_actual_line_pdf_json_and_image_locations_are_checked(self):
        cases = [
            (self.candidate, {"kind": "lines", "start": 1, "end": 3}, {"kind": "lines", "start": 1, "end": 999}),
            (self.pdf, {"kind": "pdf_pages", "start": 1, "end": 1}, {"kind": "pdf_pages", "start": 1, "end": 2}),
            (self.observations, {"kind": "json_pointer", "pointer": "/table/row~1one/~0value"}, {"kind": "json_pointer", "pointer": "/events/4"}),
            (self.image, {"kind": "image_region", "bbox": [0, 0, 8, 8]}, {"kind": "image_region", "bbox": [0, 0, 99, 99]}),
        ]
        for path, valid, invalid in cases:
            with self.subTest(path=path.name):
                self.bind(path=path, locator=valid)
                self.resolve(evidence=["text"])
                self.assertTrue(self.report().structurally_consistent)
                self.ledger["evidence"]["text"]["locator"] = invalid
                self.assertIn("CHECK_EVIDENCE_LOCATION_INVALID", self.codes())

    def test_static_results_cannot_claim_process_or_ui_acceptance(self):
        self.resolve("EX-A01")
        self.resolve("EX-A02")
        self.assertTrue({"CHECK_PROCESS_NOT_OBSERVED", "CHECK_UI_NOT_OBSERVED"} <= self.codes())

    def test_self_report_and_offscreen_observation_do_not_replace_actual_ui(self):
        observation = self.observation(ui=True)
        observation["basis"] = "offscreen_render"
        identity = self.bind("ui", self.image, {"kind": "whole_file"},
                             evidence_type="environment_observation", observation=observation)
        self.resolve("EX-A02", evidence=[identity])
        self.assertIn("CHECK_UI_NOT_OBSERVED", self.codes())
        self.ledger["evidence"][identity]["observation"]["basis"] = "self_report"
        self.assertIn("CHECK_UI_NOT_OBSERVED", self.codes())

    def test_native_cache_evidence_is_separate_from_actual_ui_evidence(self):
        types = {**self.types, "FMT-A01": "live_environment"}
        self.prepare(requirement_types=types, environment_kinds={"FMT-A01": "native_cache"})
        observation = self.observation(ui=True)
        observation["basis"] = "native_cache"
        evidence_id = self.bind("cache", self.observations, {"kind": "json_pointer", "pointer": "/events/0"},
                                evidence_type="environment_observation", observation=observation)
        self.resolve("FMT-A01", evidence=[evidence_id])
        self.assertTrue(self.report().structurally_consistent)
        self.resolve("EX-A02", evidence=[evidence_id])
        self.assertIn("CHECK_UI_NOT_OBSERVED", self.codes())
        self.record("EX-A02")["environment_kind"] = "native_cache"
        self.assertIn("CHECK_ENVIRONMENT_CONFLICT", self.codes())

    def test_malformed_status_type_or_evidence_kind_is_diagnosed_without_crashing(self):
        self.record("EX-C01")["status"] = {"passed": True}
        self.record("FMT-A01")["requirement_type"] = ["offline_outcome"]
        evidence_id = self.bind()
        self.ledger["evidence"][evidence_id]["evidence_type"] = ["offline_result"]
        self.assertTrue({"CHECK_STATUS_INVALID", "CHECK_REQUIREMENT_TYPE_INVALID", "CHECK_EVIDENCE_TYPE_INVALID"} <= self.codes())

    def test_explicit_observation_metadata_is_not_a_semantic_or_process_certificate(self):
        process = self.bind("process", self.observations, {"kind": "json_pointer", "pointer": "/events/0"},
                            evidence_type="execution_trace", observation=self.observation())
        ui = self.bind("ui", self.image, {"kind": "whole_file"}, evidence_type="environment_observation",
                       observation=self.observation(ui=True))
        offline = self.bind()
        for identity in self.ids:
            evidence = [process] if identity == "EX-A01" else [ui] if identity in {"EX-A02", "EX-A03"} else [offline]
            self.resolve(identity, evidence=evidence)
        report = self.report()
        self.assertTrue(report.structurally_consistent)
        self.assertTrue(report.recorded_checks_complete)
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertEqual(report.execution_truth, "not_assessed")
        self.assertEqual(report.live_behavior, "not_assessed")
        self.assertTrue(report.independent_review_required)
        self.assertIn("Independent comparison", report.evidence_boundary)
        self.assertEqual(report.to_dict()["findings"], [])

    def test_unclassified_rule_cannot_be_resolved_or_reclassify_declared_ui(self):
        self.resolve()
        self.record("EX-C01")["requirement_type"] = "unclassified"
        self.record("EX-A02")["requirement_type"] = "offline_outcome"
        self.assertTrue({"CHECK_TYPE_UNCLASSIFIED", "CHECK_TYPE_CONFLICT"} <= self.codes())

    def test_unknown_evidence_and_duplicate_json_keys_are_rejected(self):
        self.resolve(evidence=["not-an-evidence-id"])
        self.assertIn("CHECK_EVIDENCE_UNKNOWN", self.codes())
        self.path.write_text('{"checks": [], "checks": []}', encoding="utf-8")
        report = checker.validate_check_records(self.path)
        self.assertEqual(report.findings[0].code, "CHECK_LEDGER_UNREADABLE")

    def test_progress_callback_and_report_finding_contract(self):
        events = []
        self.record("EX-C01")["scope"] = []
        self.save()
        report = checker.validate_check_records(self.path, progress=lambda current, total, stage: events.append((current, total, stage)))
        self.assertIn((len(self.ids), len(self.ids), "checks"), events)
        finding = report.findings[0]
        self.assertEqual(set(finding.to_dict()), {"code", "object_id", "message", "level"})
        self.assertIn(finding.level, {"error", "warning"})


if __name__ == "__main__":
    unittest.main()
