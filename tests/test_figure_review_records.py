"""Figure-review evidence regressions using only synthetic PDFs and images."""

from __future__ import annotations

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-zotero2obsidian", "scripts", "validate_delivery.py")
SPEC = importlib.util.spec_from_file_location("test_figure_review_validator", SCRIPT)
assert SPEC and SPEC.loader
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)
prepare = checker.source_tools


class FigureReviewTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.pdf = self.base.joinpath("synthetic.pdf")
        self.boxes = [[20, 60, 220, 160], [20, 20, 220, 120], [20, 200, 220, 350]]
        with fitz.open() as document:
            page = document.new_page(width=300, height=400)
            page.insert_text((20, 25), "Synthetic figure review")
            page.draw_rect(fitz.Rect(self.boxes[0]), color=(0, 0, 1))
            page.insert_text((25, 80), "(a) first panel")
            page.insert_text((20, 185), "Figure 1. Two panels across pages.")
            page = document.new_page(width=300, height=400)
            page.draw_rect(fitz.Rect(self.boxes[1]), color=(0, 0, 1))
            page.insert_text((25, 40), "(b) second panel")
            page.insert_text((20, 145), "Figure 1 continued.")
            page.draw_rect(fitz.Rect(self.boxes[2]), color=(1, 0, 0))
            icon = fitz.Pixmap(fitz.csRGB, fitz.IRect(0, 0, 12, 12), False)
            icon.clear_with(200)
            self.image_xref = page.insert_image(fitz.Rect(50, 220, 90, 260), stream=icon.tobytes("png"))
            page.insert_text((20, 375), "Figure 2. A simple icon.")
            document.save(self.pdf)
        self.inventory_path = prepare.prepare_sources(
            {"main": self.pdf}, self.base.joinpath("work"), source_roles={"main": "main"}, progress=None)
        self.inventory = json.loads(self.inventory_path.read_text(encoding="utf-8"))
        source = self.inventory["sources"][0]
        pages = source["pages"]
        title_evidence = {"page": 1, "basis": "visual", "bbox": [0, 0, 300, 45],
                          "blocks": [block["id"] for block in pages[0]["blocks"] if "Synthetic" in block["text"]]}
        figure_1_evidence = []
        for number, box in enumerate(self.boxes[:2], 1):
            figure_1_evidence.append({
                "page": number, "basis": "visual", "bbox": box,
                "coordinate_system": dict(prepare.PDF_BBOX_COORDINATES),
                "blocks": [block["id"] for block in pages[number - 1]["blocks"]
                           if "Figure 1" in block["text"] or "panel" in block["text"]],
            })
        figure_2_evidence = {
            "page": 2, "basis": "visual", "bbox": self.boxes[2],
            "coordinate_system": dict(prepare.PDF_BBOX_COORDINATES),
            "quote": "Figure 2. A simple icon.",
            "blocks": [block["id"] for block in pages[1]["blocks"] if "Figure 2" in block["text"]],
        }
        self.inventory["units"] = [
            {"id": "title", "source": "main", "kind": "metadata", "field": "title", "evidence": [title_evidence]},
            {"id": "figure-1", "source": "main", "kind": "figure", "contains_table": False, "evidence": figure_1_evidence},
            {"id": "figure-2", "source": "main", "kind": "figure", "contains_table": False, "evidence": [figure_2_evidence]},
        ]
        source["header_fields"] = {
            "title": {"status": "present", "units": ["title"]},
            **{field: {"status": "absent_in_source", "reason": "The synthetic PDF contains only a title and figure objects.",
                       "evidence": [title_evidence]} for field in ("authors", "affiliations", "publication_date")},
        }
        self.assets = {"figure-1": [self.base.joinpath("panel-a.png"), self.base.joinpath("panel-b.png")],
                       "figure-2": [self.base.joinpath("icon.png")]}
        with fitz.open(self.pdf) as document:
            for page_number, box, path in zip([0, 1, 1], self.boxes, [*self.assets["figure-1"], *self.assets["figure-2"]]):
                document[page_number].get_pixmap(dpi=144, clip=fitz.Rect(box), alpha=False).save(path)
        first = "![[panel-a.png]]\n![[panel-b.png]]\n图1：跨页的两个面板。\n^fig-1"
        second = "![[icon.png]]\n图2：简单图标。\n^fig-2"
        self.note = self.base.joinpath("candidate.md")
        self.note.write_text("---\n类型: 期刊论文\n年份: 2025\n关键词: []\n期刊名: Synthetic\n---\n"
                             "# Synthetic figure review\n\n## 1 图件\n\n" + first + "\n\n" + second + "\n", encoding="utf-8")
        self.deliveries = [
            {"unit": "title", "target": {"quote": "Synthetic figure review"}},
            {"unit": "figure-1", "target": {"quote": first}, "assets": [{"path": path.name} for path in self.assets["figure-1"]]},
            {"unit": "figure-2", "target": {"quote": second}, "assets": [{"path": path.name} for path in self.assets["figure-2"]]},
        ]
        self.manifest = self.base.joinpath("delivery-manifest.json")
        self.review_path = self.base.joinpath("review", "figure-reviews.json")
        self.save_inventory()

    def save_inventory(self):
        self.inventory_path.write_text(json.dumps(self.inventory, ensure_ascii=False), encoding="utf-8")

    def generate(self):
        self.review_path = prepare.prepare_figure_reviews(self.inventory_path, self.assets, self.review_path.parent,
                                                          resource_root=self.base, progress=None)
        self.inventory = json.loads(self.inventory_path.read_text(encoding="utf-8"))
        self.ledger = json.loads(self.review_path.read_text(encoding="utf-8"))

    def save_ledger(self):
        self.review_path.write_text(json.dumps(self.ledger, ensure_ascii=False), encoding="utf-8")

    def bind(self):
        checker.write_delivery_manifest(self.inventory_path, self.note, self.deliveries, self.manifest,
                                         resource_root=self.base)

    def report(self):
        self.save_ledger()
        self.bind()
        return checker.validate_delivery(self.manifest)

    def codes(self):
        return {finding.code for finding in self.report().findings}

    def recorded_decisions(self):
        """Synthetic self-reports exercise schema checks, not visual truth."""
        for row in self.ledger["figures"]:
            for check in row["checks"]:
                check.update({
                    "status": "pass", "reason": "Synthetic recorded decision used only to exercise the evidence schema.",
                    "evidence": [
                        {"kind": "source_page", "index": 0, "observation": "Synthetic source-page record."},
                        {"kind": "final_asset", "index": 0, "observation": "Synthetic final-image record."},
                    ],
                })

    def ready(self):
        self.generate()
        self.recorded_decisions()

    def test_old_manifest_without_a_ledger_remains_compatible(self):
        self.bind()
        report = checker.validate_delivery(self.manifest)
        self.assertEqual(report.findings, [])
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_generated_rows_cover_only_actual_figures_and_all_atomic_ids(self):
        self.generate()
        self.assertEqual([row["unit"] for row in self.ledger["figures"]], ["figure-1", "figure-2"])
        expected = [f"FIG-A{number:02d}" for number in range(1, 29)]
        for row in self.ledger["figures"]:
            self.assertEqual([check["id"] for check in row["checks"]], expected)
            self.assertTrue(all(check["status"] == "pending" for check in row["checks"]))
            self.assertTrue(all(check["reason"] == "" and check["evidence"] == [] for check in row["checks"]))

    def test_generated_pending_rows_block_completion(self):
        self.generate()
        report = self.report()
        self.assertEqual(sum(item.code == "FIG_REVIEW_PENDING" for item in report.findings), 56)
        self.assertFalse(report.structurally_consistent)

    def test_cross_page_locations_preserve_order_and_explicit_coordinates(self):
        self.generate()
        locations = self.ledger["figures"][0]["source_locations"]
        self.assertEqual([item["page"] for item in locations], [1, 2])
        self.assertTrue(all(item["coordinate_system"] == prepare.PDF_BBOX_COORDINATES for item in locations))

    def test_file_references_are_minimal_and_locations_are_separate(self):
        self.generate()
        for row in self.ledger["figures"]:
            for record in [row["source_file"], *row["final_assets"], *(item["page_render"] for item in row["source_locations"])]:
                self.assertEqual(set(record), {"path", "sha256"})
            self.assertTrue(row["source_locations"])

    def test_manifest_binds_the_finished_ledger_hash(self):
        self.ready()
        self.assertEqual(self.codes(), set())
        manifest = json.loads(self.manifest.read_text(encoding="utf-8"))
        self.assertEqual(manifest["figure_reviews"], prepare.file_record(self.review_path))

    def test_ledger_binds_a_real_separate_inventory_snapshot(self):
        self.generate()
        snapshot = self.review_path.parent.joinpath("figure-inventory-snapshot.json")
        self.assertTrue(snapshot.is_file())
        self.assertEqual(self.ledger["inventory"], prepare.file_record(snapshot))
        self.assertEqual(set(self.ledger["inventory"]), {"path", "sha256"})
        self.assertEqual(self.ledger["inventory_path"], str(self.inventory_path.resolve()))

    def test_recorded_passes_never_claim_independent_visual_acceptance(self):
        self.ready()
        report = self.report()
        self.assertTrue(report.structurally_consistent, report.findings)
        self.assertEqual(report.source_fidelity, "not_assessed")
        self.assertTrue(report.independent_review_required)

    def test_ledger_is_never_overwritten(self):
        self.generate()
        before = self.review_path.read_bytes()
        with self.assertRaises(FileExistsError):
            prepare.prepare_figure_reviews(self.inventory_path, self.assets, self.review_path.parent, progress=None)
        self.assertEqual(self.review_path.read_bytes(), before)

    def test_existing_inventory_snapshot_is_never_overwritten(self):
        snapshot = self.review_path.parent.joinpath("figure-inventory-snapshot.json")
        snapshot.parent.mkdir(parents=True)
        snapshot.write_text("Existing snapshot must remain intact.\n", encoding="utf-8")
        before = self.inventory_path.read_bytes()
        with self.assertRaises(FileExistsError):
            self.generate()
        self.assertEqual(snapshot.read_text(encoding="utf-8"), "Existing snapshot must remain intact.\n")
        self.assertEqual(self.inventory_path.read_bytes(), before)

    def test_missing_asset_mapping_is_rejected_before_inventory_mutation(self):
        original = self.inventory_path.read_bytes()
        with self.assertRaises(ValueError):
            prepare.prepare_figure_reviews(self.inventory_path, {"figure-1": self.assets["figure-1"]}, self.review_path.parent, progress=None)
        self.assertEqual(self.inventory_path.read_bytes(), original)
        self.assertFalse(self.review_path.exists())

    def test_missing_coordinate_declaration_is_not_inferred(self):
        self.inventory["units"][1]["evidence"][0].pop("coordinate_system")
        self.save_inventory()
        with self.assertRaisesRegex(ValueError, "PDF points"):
            self.generate()

    def test_pixel_coordinate_declaration_is_rejected(self):
        self.inventory["units"][1]["evidence"][0]["coordinate_system"]["unit"] = "px"
        self.save_inventory()
        with self.assertRaisesRegex(ValueError, "PDF points"):
            self.generate()

    def test_out_of_page_pixel_sized_rectangle_is_rejected(self):
        self.inventory["units"][1]["evidence"][0]["bbox"] = [40, 120, 440, 320]
        self.save_inventory()
        with self.assertRaisesRegex(ValueError, "inside its page"):
            self.generate()

    def test_native_image_object_locator_is_supported(self):
        source = self.inventory["units"][2]["evidence"][0]
        source.pop("bbox")
        source.pop("coordinate_system")
        source.pop("quote")
        source["image_xref"] = self.image_xref
        self.save_inventory()
        self.ready()
        check = self.ledger["figures"][1]["checks"][21]
        check.update({"status": "not_applicable", "reason": "This figure is located by its original image object.",
                      "applicability_condition": "No bbox is used for this image_xref-only figure location."})
        self.assertEqual(self.codes(), set())

    def test_rotated_page_uses_unrotated_pdf_point_bounds(self):
        with fitz.open(self.pdf) as document:
            for page in document:
                page.set_rotation(90)
            document.saveIncr()
        path = prepare.prepare_sources({"main": self.pdf}, self.base.joinpath("rotated-work"),
                                       source_roles={"main": "main"}, progress=None)
        rotated = json.loads(path.read_text(encoding="utf-8"))
        rotated["units"] = deepcopy(self.inventory["units"])
        rotated["sources"][0]["header_fields"] = deepcopy(self.inventory["sources"][0]["header_fields"])
        self.inventory, self.inventory_path = rotated, path
        self.save_inventory()
        self.ready()
        self.assertEqual(self.codes(), set())

    def test_missing_check_id_is_reported(self):
        self.ready()
        self.ledger["figures"][0]["checks"].pop()
        self.assertIn("FIG_REVIEW_CHECK_MISSING", self.codes())

    def test_duplicate_check_id_is_reported(self):
        self.ready()
        checks = self.ledger["figures"][0]["checks"]
        checks.append(deepcopy(checks[0]))
        self.assertIn("ID_DUPLICATE", self.codes())

    def test_unknown_check_id_is_reported(self):
        self.ready()
        self.ledger["figures"][0]["checks"][0]["id"] = "FIG-A99"
        self.assertIn("FIG_REVIEW_CHECK_UNKNOWN", self.codes())

    def test_missing_figure_id_is_reported(self):
        self.ready()
        self.ledger["figures"].pop()
        self.assertIn("FIG_REVIEW_UNIT_MISSING", self.codes())

    def test_duplicate_figure_id_is_reported(self):
        self.ready()
        self.ledger["figures"].append(deepcopy(self.ledger["figures"][0]))
        self.assertIn("ID_DUPLICATE", self.codes())

    def test_unknown_figure_id_is_reported(self):
        self.ready()
        self.ledger["figures"][0]["unit"] = "title"
        self.assertIn("FIG_REVIEW_UNIT_UNKNOWN", self.codes())

    def test_not_applicable_requires_its_concrete_condition(self):
        self.ready()
        check = self.ledger["figures"][1]["checks"][3]
        check.update({"status": "not_applicable", "reason": "The source icon has no coordinate axes."})
        self.assertIn("FIG_REVIEW_APPLICABILITY", self.codes())
        check["applicability_condition"] = "The second figure is an icon without axes in the source PDF."
        self.assertNotIn("FIG_REVIEW_APPLICABILITY", self.codes())

    def test_each_unresolved_status_blocks_completion(self):
        self.ready()
        for status in ("pending", "blocked", "not_tested", "ambiguous"):
            with self.subTest(status=status):
                self.ledger["figures"][0]["checks"][0]["status"] = status
                self.assertIn("FIG_REVIEW_PENDING", self.codes())

    def test_failed_atomic_result_blocks_completion(self):
        self.ready()
        self.ledger["figures"][0]["checks"][0]["status"] = "fail"
        self.assertIn("FIG_REVIEW_FAILED", self.codes())

    def test_decision_without_reason_is_reported(self):
        self.ready()
        self.ledger["figures"][0]["checks"][0]["reason"] = ""
        self.assertIn("FIG_REVIEW_REASON", self.codes())

    def test_decision_without_evidence_is_reported(self):
        self.ready()
        self.ledger["figures"][0]["checks"][0]["evidence"] = []
        self.assertIn("FIG_REVIEW_EVIDENCE", self.codes())

    def test_visual_pass_requires_the_source_and_final_image(self):
        self.ready()
        self.ledger["figures"][0]["checks"][0]["evidence"].pop()
        self.assertIn("FIG_REVIEW_COMPARISON", self.codes())

    def test_unknown_evidence_pointer_is_reported(self):
        self.ready()
        self.ledger["figures"][0]["checks"][0]["evidence"][0]["index"] = 99
        self.assertIn("FIG_REVIEW_EVIDENCE_TARGET", self.codes())

    def test_changed_final_image_fails_even_after_manifest_rebinding(self):
        self.ready()
        image = self.assets["figure-2"][0]
        with fitz.open(self.pdf) as document:
            document[0].get_pixmap().save(image)
        self.assertIn("FILE_HASH_MISMATCH", self.codes())

    def test_changed_ledger_requires_manifest_rebinding(self):
        self.ready()
        self.report()
        self.review_path.write_text(self.review_path.read_text(encoding="utf-8") + "\n", encoding="utf-8")
        report = checker.validate_delivery(self.manifest)
        self.assertIn("FILE_HASH_MISMATCH", {item.code for item in report.findings})

    def test_changed_inventory_invalidates_the_existing_review_binding(self):
        self.ready()
        self.inventory["units"][1]["evidence"][0]["bbox"][0] += 1
        self.save_inventory()
        self.assertIn("FIG_REVIEW_INVENTORY_CHANGED", self.codes())

    def test_adding_checks_path_after_figure_reviews_does_not_invalidate_them(self):
        self.ready()
        skills = self.base.joinpath("synthetic-skills")
        skill = skills.joinpath("sample-review")
        skill.mkdir(parents=True)
        skill.joinpath("SKILL.md").write_text(
            "# Synthetic acceptance\n- [ ] [SYN-A01] Record a synthetic result. <!-- check-type: offline_outcome -->\n",
            encoding="utf-8")
        checks_path = self.base.joinpath("checks.json")
        records = checker._load_sibling("check_records")
        records.prepare_check_records(skills, checks_path, sources={"main": self.pdf}, candidate_path=self.note,
                                      inventory_path=self.inventory_path)
        self.inventory = json.loads(self.inventory_path.read_text(encoding="utf-8"))
        self.assertEqual(Path(self.inventory["checks_path"]).resolve(), checks_path.resolve())
        codes = self.codes()
        self.assertFalse({code for code in codes if code.startswith("FIG_REVIEW_")}, codes)
        self.assertNotIn("FILE_HASH_MISMATCH", codes)

    def test_other_inventory_content_changes_are_not_exempt(self):
        self.ready()
        self.inventory["new_unbound_pointer"] = "new.json"
        self.save_inventory()
        self.assertIn("FIG_REVIEW_INVENTORY_CHANGED", self.codes())

    def test_nested_tool_pointer_name_is_not_exempt(self):
        self.ready()
        self.inventory["units"][1]["checks_path"] = "nested-change.json"
        self.save_inventory()
        self.assertIn("FIG_REVIEW_INVENTORY_CHANGED", self.codes())

    def test_inventory_snapshot_comparison_preserves_scalar_types(self):
        self.ready()
        self.inventory["units"][1]["contains_table"] = 0
        self.save_inventory()
        self.assertIn("FIG_REVIEW_INVENTORY_CHANGED", self.codes())

    def test_inventory_identity_cannot_be_replaced_by_an_identical_copy(self):
        self.ready()
        copy_path = self.base.joinpath("copied-inventory.json")
        copy_path.write_bytes(self.inventory_path.read_bytes())
        self.ledger["inventory_path"] = str(copy_path)
        self.assertIn("FIG_REVIEW_INVENTORY_PATH", self.codes())

    def test_live_inventory_cannot_substitute_for_the_snapshot(self):
        self.ready()
        self.ledger["inventory"] = prepare.file_record(self.inventory_path)
        self.assertIn("FIG_REVIEW_INVENTORY_SNAPSHOT", self.codes())

    def test_snapshot_bytes_cannot_change_after_binding(self):
        self.ready()
        snapshot = Path(self.ledger["inventory"]["path"])
        snapshot.write_bytes(snapshot.read_bytes() + b"\n")
        self.assertIn("FILE_HASH_MISMATCH", self.codes())

    def test_missing_ledger_binding_is_reported(self):
        self.ready()
        self.report()
        manifest = json.loads(self.manifest.read_text(encoding="utf-8"))
        manifest.pop("figure_reviews")
        self.manifest.write_text(json.dumps(manifest), encoding="utf-8")
        report = checker.validate_delivery(self.manifest)
        self.assertIn("FIG_REVIEW_FILE_REFERENCE", {item.code for item in report.findings})

    def test_final_image_reference_cannot_contain_source_bbox_metadata(self):
        self.ready()
        self.ledger["figures"][0]["final_assets"][0]["bbox"] = self.boxes[0]
        self.assertIn("FIG_REVIEW_FILE_REFERENCE", self.codes())

    def test_delivery_image_reference_cannot_contain_source_bbox_metadata(self):
        self.ready()
        self.deliveries[1]["assets"][0]["bbox"] = self.boxes[0]
        self.assertIn("FIG_REVIEW_FILE_REFERENCE", self.codes())

    def test_mismatched_source_page_image_is_reported(self):
        self.ready()
        locations = self.ledger["figures"][0]["source_locations"]
        locations[0]["page_render"] = deepcopy(locations[1]["page_render"])
        self.assertIn("FIG_REVIEW_SOURCE_RENDER", self.codes())

    def test_cross_page_location_order_cannot_be_reversed(self):
        self.ready()
        self.ledger["figures"][0]["source_locations"].reverse()
        self.assertIn("FIG_REVIEW_LOCATION_MAPPING", self.codes())

    def test_coordinate_schema_rejects_each_wrong_axis_or_unit(self):
        self.ready()
        location = self.ledger["figures"][0]["source_locations"][0]
        for field, value in (("space", "pixels"), ("unit", "px"), ("origin", "bottom_left"),
                             ("x_axis", "left"), ("y_axis", "up"), ("rotation", "rendered")):
            with self.subTest(field=field):
                location["coordinate_system"] = {**prepare.PDF_BBOX_COORDINATES, field: value}
                self.assertIn("FIG_REVIEW_COORDINATES", self.codes())

    def test_malformed_ledger_types_are_diagnostics_never_success(self):
        self.ready()
        valid = deepcopy(self.ledger)
        mutations = [
            lambda row: row.update(schema_version=True),
            lambda row: row.update(schema_version="1"),
            lambda row: row.pop("inventory_path"),
            lambda row: row.update(inventory_path=[]),
            lambda row: row.update(figures={}),
            lambda row: row["figures"][0].update(checks=None),
            lambda row: row["figures"][0].update(source_locations="page-1"),
            lambda row: row["figures"][0].update(source_locations=[None]),
            lambda row: row["figures"][0].update(final_assets=[None]),
            lambda row: row["figures"][0]["checks"][0].update(status=[]),
            lambda row: row["figures"][0]["checks"][0].update(reason=7),
            lambda row: row["figures"][0]["checks"][0].update(evidence=[True]),
            lambda row: row["figures"][0]["checks"][0]["evidence"][0].update(index=True),
            lambda row: row["figures"][0]["checks"][0]["evidence"][0].update(kind=[]),
            lambda row: row["figures"][0]["source_locations"][0].update(page=[]),
            lambda row: row["figures"][0]["source_locations"][0].update(bbox=[0, 0, float("inf"), 1]),
            lambda row: row["figures"][0]["source_locations"][0].update(bbox=[0, 0, 10 ** 1000, 1]),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                self.ledger = deepcopy(valid)
                mutate(self.ledger)
                report = self.report()
                self.assertFalse(report.structurally_consistent)
                self.assertTrue(report.findings)

    def test_non_object_ledger_is_a_diagnostic(self):
        self.generate()
        self.ledger = []
        self.assertIn("JSON_OBJECT", self.codes())

    def test_duplicate_json_keys_are_rejected(self):
        self.ready()
        self.save_ledger()
        text = self.review_path.read_text(encoding="utf-8").replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1', 1)
        self.review_path.write_text(text, encoding="utf-8")
        self.bind()
        report = checker.validate_delivery(self.manifest)
        self.assertIn("JSON_READ", {item.code for item in report.findings})


if __name__ == "__main__":
    unittest.main(verbosity=2)
