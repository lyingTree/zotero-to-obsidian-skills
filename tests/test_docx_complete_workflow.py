"""Mixed PDF/DOCX provenance reaches file completion without invented pages."""
from __future__ import annotations

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz

from test_docx_source_inventory import make_docx, paragraph, image_call, image_relation


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("test_docx_workflow_tools", ROOT.joinpath("literature-zotero2obsidian", "scripts", "complete_workflow.py"))
assert SPEC and SPEC.loader
workflow = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = workflow
SPEC.loader.exec_module(workflow)
prepare = workflow._module("prepare_source_inventory")
delivery = workflow._module("validate_delivery")


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def native_evidence(block, **extra):
    return {"basis": "native", "native_locator": deepcopy(block["locator"]), "blocks": [block["id"]], **extra}


class MixedDocxWorkflowTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.skills = self.base.joinpath("skills")
        skill = self.skills.joinpath("synthetic-skill")
        skill.mkdir(parents=True)
        skill.joinpath("SKILL.md").write_text(
            "# Synthetic\n- [ ] [DOCX-A01] The native paragraph has its own mapped candidate. <!-- check-type: offline_outcome -->\n", encoding="utf-8")
        self.pdf = self.base.joinpath("main.pdf")
        with fitz.open() as document:
            page = document.new_page(width=350, height=180)
            page.insert_text((20, 30), "Synthetic main title")
            page.insert_text((20, 65), "A red marker follows the beam.")
            document.save(self.pdf)
        self.docx = self.base.joinpath("supplement.docx")
        make_docx(self.docx, paragraph("Synthetic supplement title") + paragraph("A blue marker precedes the beam."))
        self.output = self.base.joinpath("output")
        self.output.mkdir()
        self.candidate = self.output.joinpath("candidate.md")
        self.candidate.write_text(
            "---\n类型: 期刊论文\n年份: 2026\n关键词: []\n期刊名: Synthetic\n---\n"
            "# Synthetic main title\n\n## 1 正文\nA red marker follows the beam.\n^main-paragraph\n\n"
            "## 2 补充材料\n\n### 2.1 Synthetic supplement title\nA blue marker precedes the beam.\n^si-paragraph\n", encoding="utf-8")
        self.run_path = workflow.initialize_workflow(
            skills_root=self.skills, work_root=self.base.joinpath("work"),
            sources={"main": self.pdf, "si": self.docx}, source_roles={"main": "main", "si": "supplement"},
            candidate_path=self.candidate, resource_root=self.output,
            translation_required=False, progress=lambda *_: None)
        self.run = read(self.run_path)
        self.inventory_path = Path(self.run["records"]["source_inventory"])
        self.inventory = read(self.inventory_path)
        self.deliveries = []
        for source in self.inventory["sources"]:
            identity = source["id"]
            if identity == "main":
                blocks = source["pages"][0]["blocks"]
                evidences = [{"page": 1, "basis": "visual", "bbox": block["bbox"], "blocks": [block["id"]], "quote": block["text"].strip()} for block in blocks]
            else:
                blocks = source["native_blocks"]
                evidences = [native_evidence(block, quote=block["text"]) for block in blocks]
            source["header_fields"] = {
                "title": {"status": "present", "units": [identity + "-title"]},
                **{key: {"status": "absent_in_source", "reason": "The generated fixture has a title and one sentence only.", "evidence": [evidences[0]]}
                   for key in ("authors", "affiliations", "publication_date")}}
            self.inventory["units"].extend([
                {"id": identity + "-title", "source": identity, "kind": "metadata", "field": "title", "evidence": [evidences[0]]},
                {"id": identity + "-paragraph", "source": identity, "kind": "paragraph", "evidence": [evidences[1]]}])
            self.deliveries.extend([
                {"unit": identity + "-title", "target": {"quote": blocks[0]["text"].strip()}},
                {"unit": identity + "-paragraph", "target": {"anchor": identity + "-paragraph"}}])
        save(self.inventory_path, self.inventory)
        check_tools = workflow._module("check_records")
        self.run["naming"] = workflow._module("format_helpers").build_naming_plan(
            year=None, no_year_reason="The generated main PDF has no year.", author_token="未知作者",
            unknown_author_reason="The generated source names no author.", original_title="Synthetic main title",
            source_evidence={key: {"file": check_tools.bind_file(self.pdf), "locator": {"page": 1}} for key in ("year", "author_token", "original_title")},
            delivery_path=self.candidate)
        save(self.run_path, self.run)
        workflow.bind_candidate(self.run_path)
        workflow.record_check_batch(Path(self.run["records"]["checks"]), [{
            "id": "DOCX-A01", "status": "passed", "scope": [{"object_id": "si-paragraph", "locator": {"native_locator": self.inventory["sources"][1]["native_blocks"][1]["locator"]}}],
            "evidence": ["source", "candidate"], "reason": "The actual generated blue-marker sentence is present at the mapped candidate anchor."}],
            evidence={"source": check_tools.bind_evidence(self.inventory_path, {"kind": "json_pointer", "pointer": "/sources/1/native_blocks/1"}),
                      "candidate": check_tools.bind_evidence(self.candidate, {"kind": "lines", "start": 15, "end": 17})})
        result = workflow.write_maps(self.run_path, self.deliveries)
        self.assertTrue(result.records_ready, msg=result.as_dict())
        workflow.write_manifest(self.run_path, self.deliveries)

    def test_mixed_source_workflow_closes_file_records_without_source_fidelity_claim(self):
        report = workflow.require_completed_records(self.run_path)
        self.assertEqual(report.findings, [])
        self.assertTrue(report.records_ready)
        self.assertEqual(report.source_fidelity, "not_assessed")
        source = self.inventory["sources"][1]
        self.assertEqual(source["pages"], [])
        si_map = read(Path(self.run["records"]["si_map"]))
        self.assertTrue(all("native_locator" in block["source_locator"] for block in si_map["blocks"]))
        self.assertTrue(all("page" not in block["source_locator"] for block in si_map["blocks"]))

    def test_changed_native_inventory_is_rejected_against_original_docx(self):
        self.inventory["sources"][1]["native_blocks"][1]["text"] = "Replaced without source support."
        save(self.inventory_path, self.inventory)
        workflow.write_manifest(self.run_path, self.deliveries)
        report = delivery.validate_delivery(Path(self.run["records"]["delivery_manifest"]))
        self.assertIn("NATIVE_PROVENANCE", {item.code for item in report.findings})

    def test_native_source_cannot_be_relabelled_with_fake_page_geometry(self):
        self.inventory["units"][-1]["evidence"][0].update(page=1, bbox=[0, 0, 10, 10])
        save(self.inventory_path, self.inventory)
        workflow.write_manifest(self.run_path, self.deliveries)
        report = delivery.validate_delivery(Path(self.run["records"]["delivery_manifest"]))
        self.assertIn("NATIVE_EVIDENCE", {item.code for item in report.findings})


class NativeFigureProvenanceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.docx = self.base.joinpath("source.docx")
        pixels = fitz.Pixmap(fitz.csRGB, fitz.IRect(0, 0, 8, 8), False)
        pixels.clear_with(160)
        image_bytes = pixels.tobytes("png")
        make_docx(self.docx, paragraph("Native figure fixture") + image_call("rA") + image_call("rB") + paragraph("Figure S1. Two panels."),
                  relationships=[image_relation("rA", "media/a.png"), image_relation("rB", "media/b.png")],
                  media={"word/media/a.png": image_bytes, "word/media/b.png": image_bytes})
        self.inventory_path = prepare.prepare_sources({"si": self.docx}, self.base.joinpath("evidence"), source_roles={"si": "supplement"}, progress=None)
        self.inventory = read(self.inventory_path)
        source = self.inventory["sources"][0]
        title, first, second, caption = source["native_blocks"]
        title_evidence = native_evidence(title, quote=title["text"])
        source["header_fields"] = {"title": {"status": "present", "units": ["title"]},
                                   **{key: {"status": "absent_in_source", "reason": "The synthetic source has a title and two panels only.", "evidence": [title_evidence]}
                                      for key in ("authors", "affiliations", "publication_date")}}
        figure_evidence = [{"basis": "native", "native_locator": call["locator"], "blocks": [call["block_id"]], "media_call": call["id"]} for call in source["media_calls"]]
        self.inventory["units"] = [
            {"id": "title", "source": "si", "kind": "metadata", "field": "title", "evidence": [title_evidence]},
            {"id": "panels", "source": "si", "kind": "figure", "original_number": "S1", "display_number": "S1", "contains_table": False, "evidence": figure_evidence},
            {"id": "caption", "source": "si", "kind": "paragraph", "evidence": [native_evidence(caption, quote=caption["text"])]}]
        self.assets = []
        for index, media in enumerate(source["native_media"]):
            output = self.base.joinpath(f"panel-{index}.png")
            output.write_bytes(Path(media["file"]["path"]).read_bytes())
            self.assets.append(output)
        self.candidate = self.base.joinpath("candidate.md")
        figure = "![[panel-0.png]]\n![[panel-1.png]]\n**图S1：** Two panels.\n^fig-s1"
        self.candidate.write_text("---\n类型: 期刊论文\n年份: 2026\n关键词: []\n期刊名: Synthetic\n---\n# Native figure fixture\n\n## 1 补充材料\n\n" + figure + "\n", encoding="utf-8")
        self.deliveries = [{"unit": "title", "target": {"quote": "Native figure fixture"}},
                           {"unit": "panels", "target": {"quote": figure}, "assets": [prepare.file_record(path) for path in self.assets]},
                           {"unit": "caption", "target": {"quote": "Two panels."}}]
        self.manifest_path = self.base.joinpath("manifest.json")
        save(self.inventory_path, self.inventory)

    def validate(self):
        delivery.write_delivery_manifest(self.inventory_path, self.candidate, self.deliveries, self.manifest_path, resource_root=self.base)
        return delivery.validate_delivery(self.manifest_path)

    def test_two_native_calls_form_one_logical_figure_without_fake_pagination(self):
        report = self.validate()
        self.assertEqual(report.findings, [], msg=report.to_dict())
        self.assertEqual(report.source_fidelity, "not_assessed")

    def test_unmapped_image_call_remains_a_specific_failure(self):
        self.inventory["units"][1]["evidence"].pop()
        save(self.inventory_path, self.inventory)
        self.assertIn("NATIVE_MEDIA_UNMAPPED", {item.code for item in self.validate().findings})

    def test_figure_reviews_bind_native_media_and_preserve_pending_observations(self):
        review_path = prepare.prepare_figure_reviews(self.inventory_path, {"panels": self.assets}, self.base.joinpath("review"), progress=None)
        ledger = read(review_path)
        locations = ledger["figures"][0]["source_locations"]
        self.assertEqual(len(locations), 2)
        self.assertTrue(all("source_media" in row and "page_render" not in row and "page" not in row for row in locations))
        prepare.record_figure_edges(review_path, "panels", 0, source_location_indices=[0],
                                    observations={"left": {"status": "pass", "observation": "Synthetic comparison exercises the native media binding only."}})
        report = self.validate()
        codes = {item.code for item in report.findings}
        self.assertIn("FIG_REVIEW_PENDING", codes)
        self.assertIn("FIG_REVIEW_EDGE_PENDING", codes)
        self.assertFalse(codes & {"SOURCE_PDF", "FIG_REVIEW_PAGE", "FIG_REVIEW_SOURCE_RENDER", "FIG_REVIEW_EDGE_BINDING"}, msg=report.to_dict())


if __name__ == "__main__":
    unittest.main()
