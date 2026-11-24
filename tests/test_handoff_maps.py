"""Synthetic handoff-map coverage, provenance and writer-boundary regressions."""

from __future__ import annotations

from copy import deepcopy
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


ROOT = Path(__file__).resolve().parents[1]


def load(name):
    path = ROOT.joinpath("literature-zotero2obsidian", "scripts", name + ".py")
    spec = importlib.util.spec_from_file_location("test_handoff_" + name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


maps = load("handoff_maps")
sources = load("prepare_source_inventory")


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def binding(path):
    return {"path": str(path.resolve()), "sha256": sha256(path.read_bytes()).hexdigest()}


class SourceUnitConstructionTests(unittest.TestCase):
    def setUp(self):
        self.evidence = [{"page": 1, "blocks": ["main-p1-b0"], "quote": "Synthetic source call."}]

    def test_missing_call_display_fails_before_map_generation(self):
        with self.assertRaisesRegex(ValueError, "display_text"):
            maps.make_source_unit("call-one", "main", "citation", self.evidence, references=["ref-one"])

    def test_explicit_call_retains_its_source_text_and_reference_identity(self):
        unit = maps.make_source_unit("call-one", "main", "citation", self.evidence,
                                     references=["ref-one", "ref-two"], display_text="3,7")
        self.assertEqual(unit["display_text"], "3,7")
        self.assertNotIn("status", unit)
        unit["evidence"][0]["blocks"].append("other")
        self.assertEqual(self.evidence[0]["blocks"], ["main-p1-b0"])

    def test_main_objects_need_explicit_original_number_too(self):
        with self.assertRaisesRegex(ValueError, "original_number"):
            maps.make_source_unit("figure-one", "main", "figure", self.evidence, contains_table=False)
        unit = maps.make_source_unit("figure-one", "main", "figure", self.evidence,
                                     original_number="3", contains_table=False)
        self.assertEqual(unit["original_number"], "3")

    def test_unnumbered_formula_has_an_explicit_observed_absence_reason(self):
        with self.assertRaisesRegex(ValueError, "absence"):
            maps.make_source_unit("formula-one", "main", "formula", self.evidence, original_number=None)
        unit = maps.make_source_unit("formula-one", "main", "formula", self.evidence,
                                     original_number=None, original_number_reason="The displayed source formula has no printed label.")
        self.assertIsNone(unit["original_number"])

    def test_cross_reference_cannot_omit_its_actual_target(self):
        with self.assertRaises(ValueError):
            maps.make_source_unit("cross-one", "main", "cross_reference", self.evidence, display_text="Fig. 3")
        with self.assertRaises(ValueError):
            maps.make_source_unit("cross-one", "main", "cross_reference", self.evidence, display_text="Fig. 3", target_unit="")

    def test_reference_fields_are_source_facts_not_inferred_from_an_anchor(self):
        with self.assertRaisesRegex(ValueError, "bibliography_id"):
            maps.make_source_unit("reference-one", "main", "reference", self.evidence, number="3", original_text="Synthetic reference.")

    def test_empty_source_evidence_and_identity_override_are_rejected(self):
        with self.assertRaises(ValueError):
            maps.make_source_unit("paragraph-one", "main", "paragraph", [])
        with self.assertRaises(ValueError):
            maps.make_source_unit("paragraph-one", "main", "paragraph", self.evidence, source="another")

    def test_constructor_uses_the_delivery_inventory_kind_contract(self):
        for kind in ("equation", "video", "unknown"):
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "supported kind"):
                maps.make_source_unit("object-one", "main", kind, self.evidence, original_number="1")

    def test_figure_requires_an_explicit_boolean_table_classification(self):
        for value in (None, "false", 0, []):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "contains_table"):
                maps.make_source_unit("figure-one", "main", "figure", self.evidence,
                                      original_number="1", contains_table=value)

    def test_source_number_is_a_literal_string_or_explained_absence(self):
        for value in ("", "  ", 1, True, [], {}):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "original_number"):
                maps.make_source_unit("formula-one", "main", "formula", self.evidence, original_number=value)

    def test_table_requires_source_shape_and_explicit_merge_list(self):
        for fields in ({}, {"shape": {"rows": True, "columns": 2}, "merges": []},
                       {"shape": {"rows": 2, "columns": 2}},
                       {"shape": {"rows": 2, "columns": 2}, "merges": None}):
            with self.subTest(fields=fields), self.assertRaises(ValueError):
                maps.make_source_unit("table-one", "main", "table", self.evidence, original_number="1", **fields)
        fields = {"shape": {"rows": 2, "columns": 2}, "merges": []}
        unit = maps.make_source_unit("table-one", "main", "table", self.evidence, original_number="1", **fields)
        self.assertEqual(unit["shape"], fields["shape"])
        self.assertEqual(unit["merges"], [])
        self.assertNotIn("status", unit)

    def test_source_merge_bounds_must_fit_recorded_table_shape(self):
        for merge in ({}, {"row_start": 1, "row_end": 3, "column_start": 1, "column_end": 1},
                      {"row_start": True, "row_end": 2, "column_start": 1, "column_end": 1}):
            with self.subTest(merge=merge), self.assertRaises(ValueError):
                maps.make_source_unit("table-one", "main", "table", self.evidence, original_number="1",
                                      shape={"rows": 2, "columns": 2}, merges=[merge])

    def test_present_but_empty_reference_identity_fields_are_rejected(self):
        for key in ("number", "bibliography_id", "original_text"):
            fields = {"number": "1", "bibliography_id": "main-bibliography", "original_text": "Synthetic entry."}
            fields[key] = ""
            with self.subTest(key=key), self.assertRaises(ValueError):
                maps.make_source_unit("ref-one", "main", "reference", self.evidence, **fields)


class Fixture:
    def __init__(self, base: Path, *, rich: bool = False, candidate_eol: str | None = None):
        self.base = base
        self.specs = {
            "main": [
                {"id": "main-title", "kind": "metadata", "field": "title", "text": "Synthetic projection notes"},
                {"id": "main-body", "kind": "paragraph", "text": "The main sample has an orange marker."},
            ],
            "si": [
                {"id": "si-title", "kind": "metadata", "field": "title", "text": "Synthetic supplement"},
                {"id": "si-body", "kind": "paragraph", "text": "The extra sample has a green marker."},
            ],
        }
        self.candidate = base.joinpath("output", "candidate.md")
        self.candidate.parent.mkdir(parents=True)
        candidate = ("---\nkind: synthetic\n---\n# Synthetic projection notes\n\n"
                     "The main sample has an orange marker.\n^main-body\n\n"
                     "## Synthetic supplement\n\nThe extra sample has a green marker.\n^si-body\n")
        self.deliveries = [
            {"unit": "main-title", "target": {"quote": "# Synthetic projection notes"}},
            {"unit": "main-body", "target": {"anchor": "main-body"}},
            {"unit": "si-title", "target": {"quote": "## Synthetic supplement"}},
            {"unit": "si-body", "target": {"anchor": "si-body"}},
        ]
        if rich:
            candidate += self.rich_content()
        if candidate_eol is None:
            self.candidate.write_text(candidate, encoding="utf-8")
        else:
            self.candidate.write_bytes(candidate.replace("\n", candidate_eol).encode("utf-8"))
        self.source_paths = {}
        for source_id, specs in self.specs.items():
            path = base.joinpath("input", source_id + ".pdf")
            path.parent.mkdir(parents=True, exist_ok=True)
            with fitz.open() as document:
                page = document.new_page(width=600, height=900)
                for index, spec in enumerate(specs):
                    page.insert_text((25, 35 + index * 34), spec["text"], fontsize=10)
                document.save(path)
            self.source_paths[source_id] = path
        self.inventory_path = sources.prepare_sources(self.source_paths, base.joinpath("evidence"),
                                                     source_roles={"main": "main", "si": "supplement"}, progress=None)
        self.inventory = read(self.inventory_path)
        self.inventory["units"] = []
        for source in self.inventory["sources"]:
            for spec, block in zip(self.specs[source["id"]], source["pages"][0]["blocks"], strict=True):
                record = {key: value for key, value in spec.items() if key != "text"}
                record.update({"source": source["id"], "evidence": [{"page": 1, "basis": "text", "bbox": block["bbox"],
                                "quote": spec["text"], "blocks": [block["id"]]}]})
                self.inventory["units"].append(record)
        save(self.inventory_path, self.inventory)
        self.paths = {name: base.joinpath("maps", name.replace("_", "-") + ".json")
                      for name in ("source_map", "si_map", "link_map")}
        self.expected = {identity: {**binding(path), "role": "main" if identity == "main" else "supplement"}
                         for identity, path in self.source_paths.items()}

    def rich_content(self):
        calls = [
            ("call-first", "First [One](#^ref-a)", "One"),
            ("call-again", "Again [One](#^ref-a)", "One"),
            ("call-caption", "[Caption](#^ref-a)", "Caption"),
            ("call-cell", "[Cell](#^ref-a)", "Cell"),
            ("call-footer", "[[#^ref-a|Footer]]", "Footer"),
        ]
        self.specs["main"].extend([
            {"id": "ref-unit", "kind": "reference", "number": 1, "bibliography_id": "synthetic-list",
             "original_text": "Vale. Synthetic entry. 2032.", "text": "Vale. Synthetic entry. 2032."},
            {"id": "table-unit", "kind": "table", "original_number": "2", "shape": [1, 1], "merges": [],
             "text": "Table 2 contains the registered synthetic links."},
            {"id": "cross-table", "kind": "cross_reference", "target_unit": "table-unit", "display_text": "Table 2",
             "text": "See Table 2 for the synthetic labels."},
            {"id": "data-url", "kind": "metadata", "field": "external_url", "value": "https://example.invalid/data",
             "display_text": "Data", "text": "Data https://example.invalid/data"},
            {"id": "library-origin", "kind": "paragraph", "display_text": "Notebook", "text": "The source mentions Notebook."},
        ])
        for identity, target, display in calls:
            self.specs["main"].append({"id": identity, "kind": "citation", "references": ["ref-unit"],
                                       "display_text": display, "text": f"Source call {identity}: {display}."})
            self.deliveries.append({"unit": identity, "target": {"quote": target}})
        workbook = {"version": 3, "title": "[Caption](#^ref-a)", "sheets": [
            {"id": "first", "name": "first", "columns": [{"id": "a", "name": "Value"}],
             "rows": [{"id": "r1", "cells": {"a": "plain"}}], "locked": True, "theme": "grid"},
            {"id": "second", "name": "second", "columns": [{"id": "b", "name": "Value"}],
             "rows": [{"id": "r1", "cells": {"b": "[Cell](#^ref-a)"}}],
             "footer": "[[#^ref-a|Footer]]", "locked": True, "theme": "grid"},
        ]}
        self.deliveries.extend([
            {"unit": "ref-unit", "target": {"anchor": "ref-a"}},
            {"unit": "table-unit", "target": {"anchor": "tbl-a"}},
            {"unit": "cross-table", "target": {"quote": "[Table](#^tbl-a)"}},
            {"unit": "data-url", "target": {"quote": "[Data](https://example.invalid/data)"}},
            {"unit": "library-origin", "target": {"quote": "[Notebook](notes/another.md)"}},
        ])
        return ("\nFirst [One](#^ref-a). Again [One](#^ref-a).\n\n[Table](#^tbl-a)\n\n"
                "```rich-table\n" + json.dumps(workbook, ensure_ascii=False) + "\n```\n^tbl-a\n\n"
                "Vale. Synthetic entry. 2032. ^ref-a\n\n"
                "[Data](https://example.invalid/data) and [Notebook](notes/another.md)\n\n"
                "`[hidden](#^missing)`\n\n```text\n[example](#^missing)\n^fake-anchor\n```\n")

    def write(self, **changes):
        arguments = {"expected_source_bindings": self.expected, "progress": None}
        arguments.update(changes)
        return maps.write_handoff_maps(self.inventory_path, self.candidate, self.deliveries, self.paths, **arguments)

    def validate(self, **changes):
        arguments = {"expected_source_bindings": self.expected, "progress": None}
        arguments.update(changes)
        return maps.validate_handoff_maps(self.inventory_path, self.candidate, self.paths, **arguments)


class HandoffMapTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.f = Fixture(self.base)
        report = self.f.write()
        self.assertTrue(report.records_ready, report.as_dict())

    def codes(self):
        report = self.f.validate()
        self.assertFalse(report.records_ready, report.as_dict())
        self.assertEqual(report.source_fidelity, "not_assessed")
        return {row.code for row in report.findings}

    def test_round_trip_preserves_source_facts_and_pending_status(self):
        report = self.f.validate()
        self.assertTrue(report.records_ready, report.as_dict())
        self.assertEqual(report.as_dict()["source_fidelity"], "not_assessed")
        source_map = read(self.f.paths["source_map"])
        self.assertEqual(source_map["review_status"], "pending")
        self.assertEqual(source_map["metadata"][0]["registered_unit"], self.f.inventory["units"][0])
        for block in source_map["blocks"]:
            self.assertEqual(block["original_sha256"], sha256(block["original_text"].encode()).hexdigest())
            self.assertEqual(block["status"], "extracted")
        self.assertFalse(any(row.get("status") == "passed" for row in source_map["units"]))

    def test_minimal_source_bound_block_contract_remains_compatible(self):
        for name in ("source_map", "si_map"):
            value = read(self.f.paths[name])
            for field in ("units", "metadata"):
                del value[field]
            if name == "source_map":
                del value["sources"]
            for block in value["blocks"]:
                for field in ("original_text", "source_blocks", "target_units", "status", "review_status"):
                    block.pop(field, None)
            save(self.f.paths[name], value)
        links = read(self.f.paths["link_map"])
        links["anchors"] = [{"id": row["target_unit"], "source_id": row["source_id"], "target_id": row["target_id"], "status": "checked"}
                            for row in links["anchors"]]
        save(self.f.paths["link_map"], links)
        self.assertTrue(self.f.validate().records_ready)

    def test_validation_never_writes_back(self):
        before = {path: path.read_bytes() for path in self.f.paths.values()}
        self.assertTrue(self.f.validate().records_ready)
        self.assertEqual(before, {path: path.read_bytes() for path in self.f.paths.values()})

    def test_empty_maps_cannot_discard_registered_blocks_units_or_anchors(self):
        for name, fields in (("source_map", ("blocks", "units", "metadata", "objects", "references", "citations")),
                             ("si_map", ("sources", "blocks", "units", "metadata", "objects", "references", "citations")),
                             ("link_map", ("anchors",))):
            value = read(self.f.paths[name])
            for field in fields:
                value[field] = []
            save(self.f.paths[name], value)
        codes = self.codes()
        self.assertTrue({"MAP_BLOCK_MISSING", "MAP_UNIT_MISSING", "MAP_ANCHOR_MISSING", "MAP_ROLE_SOURCE_SET"} <= codes)

    def test_fake_collection_elements_are_rejected_without_becoming_records(self):
        value = read(self.f.paths["source_map"])
        value.update({"blocks": [None, 7, "block"], "objects": {"fake": False},
                      "references": ["reference"], "citations": [17]})
        save(self.f.paths["source_map"], value)
        self.assertIn("MAP_COLLECTION_ELEMENT", self.codes())

    def test_current_candidate_path_and_hash_are_both_required(self):
        path = self.f.paths["source_map"]
        original = read(path)
        other = self.base.joinpath("same-bytes.md")
        other.write_bytes(self.f.candidate.read_bytes())
        value = deepcopy(original)
        value["bindings"]["candidate"] = binding(other)
        save(path, value)
        self.assertIn("MAP_BINDING_TARGET", self.codes())
        value = deepcopy(original)
        value["bindings"]["candidate"]["sha256"] = "0" * 64
        save(path, value)
        self.assertIn("MAP_BINDING_HASH", self.codes())

    def test_inventory_and_source_bindings_cannot_be_swapped_or_stale(self):
        value = read(self.f.paths["si_map"])
        value["bindings"]["source_inventory"]["sha256"] = "0" * 64
        value["bindings"]["sources"]["main"] = binding(self.f.source_paths["si"])
        save(self.f.paths["si_map"], value)
        codes = self.codes()
        self.assertIn("MAP_BINDING_HASH", codes)
        self.assertIn("MAP_BINDING_TARGET", codes)

    def test_block_identity_role_location_hash_and_target_are_checked(self):
        path = self.f.paths["source_map"]
        original = read(path)
        changes = [
            ("source_id", "si", "MAP_BLOCK_SOURCE"),
            ("source_locator", {"page": 9, "bbox": [0, 0, 2, 2]}, "MAP_BLOCK_LOCATOR"),
            ("original_sha256", "0" * 64, "MAP_BLOCK_ORIGINAL_HASH"),
            ("target_units", ["si-body"], "MAP_BLOCK_TARGET_SET"),
        ]
        for key, replacement, expected in changes:
            with self.subTest(key=key):
                value = deepcopy(original)
                value["blocks"][0][key] = replacement
                save(path, value)
                self.assertIn(expected, self.codes())
        save(path, original)

    def test_duplicate_ids_and_unknown_unit_references_fail(self):
        value = read(self.f.paths["source_map"])
        value["blocks"].append(deepcopy(value["blocks"][0]))
        value["units"][0]["unit"] = "missing-unit"
        save(self.f.paths["source_map"], value)
        codes = self.codes()
        self.assertIn("MAP_DUPLICATE_ID", codes)
        self.assertIn("MAP_TARGET_UNIT_UNKNOWN", codes)

    def test_original_scope_blocks_removal_of_a_whole_source(self):
        inventory = read(self.f.inventory_path)
        inventory["sources"] = [row for row in inventory["sources"] if row["id"] == "main"]
        inventory["units"] = [row for row in inventory["units"] if row["source"] == "main"]
        save(self.f.inventory_path, inventory)
        self.assertIn("MAP_SOURCE_SET", self.codes())

    def test_existing_maps_are_preserved_as_a_group(self):
        old = {path: path.read_bytes() for path in self.f.paths.values()}
        report = self.f.write()
        self.assertFalse(report.records_ready)
        self.assertEqual({row.code for row in report.findings}, {"MAP_OUTPUT_EXISTS"})
        self.assertEqual(report.written_paths, {})
        self.assertEqual(old, {path: path.read_bytes() for path in self.f.paths.values()})
        self.f.paths = {name: path.with_name(path.stem + "-v2.json") for name, path in self.f.paths.items()}
        self.assertTrue(self.f.write().records_ready)
        self.assertEqual(old, {path: path.read_bytes() for path in old})

    def test_missing_delivery_returns_actionable_findings_without_partial_files(self):
        self.f.paths = {name: path.with_name(path.stem + "-incomplete.json") for name, path in self.f.paths.items()}
        self.f.deliveries.pop()
        report = self.f.write()
        self.assertIn("MAP_DELIVERY_MISSING", {row.code for row in report.findings})
        self.assertFalse(any(path.exists() for path in self.f.paths.values()))

    def test_progress_callback_covers_registered_units_and_output(self):
        events = []
        self.assertTrue(self.f.validate(progress=lambda *event: events.append(event)).records_ready)
        self.assertTrue(events)
        self.assertTrue(any("source units" in event[2] for event in events))

    def test_malformed_identity_fields_report_findings_instead_of_crashing(self):
        path = self.f.paths["source_map"]
        original = read(path)
        probes = [
            ("blocks", "source_id"), ("blocks", "source_blocks"), ("blocks", "target_units"),
            ("blocks", "target_unit"), ("blocks", "source_attachment"), ("blocks", "source_locator"),
            ("units", "id"), ("units", "unit"), ("units", "source_id"), ("units", "kind"),
        ]
        for collection, key in probes:
            for value in (None, [], {}, 19, False):
                with self.subTest(collection=collection, key=key, value=value):
                    modified = deepcopy(original)
                    modified[collection][0][key] = value
                    save(path, modified)
                    self.assertFalse(self.f.validate().records_ready)
        save(path, original)

    def test_malformed_inventory_fields_are_not_silently_ignored(self):
        original = read(self.f.inventory_path)
        for key in ("source", "kind", "evidence", "parent_id"):
            with self.subTest(key=key):
                value = deepcopy(original)
                value["units"][0][key] = ["not-an-identity"]
                save(self.f.inventory_path, value)
                self.assertFalse(self.f.validate().records_ready)
        save(self.f.inventory_path, original)

    def test_conflicting_single_and_plural_target_fields_are_rejected(self):
        value = read(self.f.paths["source_map"])
        value["blocks"][0]["target_unit"] = "si-title"
        save(self.f.paths["source_map"], value)
        self.assertIn("MAP_BLOCK_TARGET_SET", self.codes())

    def test_stale_unit_snippet_and_anchor_locators_are_rejected(self):
        value = read(self.f.paths["source_map"])
        value["units"][0]["candidate_locator"]["sha256"] = "0" * 64
        save(self.f.paths["source_map"], value)
        links = read(self.f.paths["link_map"])
        links["anchors"][0]["candidate_locator"]["start"] += 1
        save(self.f.paths["link_map"], links)
        codes = self.codes()
        self.assertIn("MAP_UNIT_CANDIDATE_LOCATOR", codes)
        self.assertIn("MAP_ANCHOR_LOCATOR", codes)

    def test_expected_binding_list_and_roles_protect_initial_scope(self):
        expected = [{"id": key, **value} for key, value in self.f.expected.items()]
        self.assertTrue(self.f.validate(expected_source_bindings=expected).records_ready)
        expected[0]["role"] = "supplement"
        self.assertIn("MAP_SOURCE_ROLE", {row.code for row in self.f.validate(expected_source_bindings=expected).findings})

    def test_duplicate_json_keys_and_map_path_aliases_are_rejected(self):
        path = self.f.paths["source_map"]
        text = path.read_text(encoding="utf-8")
        path.write_text(text.replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1', 1), encoding="utf-8")
        self.assertIn("MAP_JSON_READ", self.codes())
        self.f.paths["si_map"] = self.f.paths["link_map"]
        self.assertIn("MAP_PATH_COLLISION", self.codes())

    def test_dangling_existing_map_symlink_is_preserved(self):
        self.f.paths = {name: path.with_name(path.stem + "-next.json") for name, path in self.f.paths.items()}
        target = self.base.joinpath("uncreated-map-target.json")
        self.f.paths["source_map"].symlink_to(target)
        report = self.f.write()
        self.assertFalse(report.records_ready)
        self.assertIn("MAP_OUTPUT_EXISTS", {row.code for row in report.findings})
        self.assertTrue(self.f.paths["source_map"].is_symlink())
        self.assertFalse(target.exists())
        self.assertFalse(self.f.paths["link_map"].exists())

    def test_explicit_resource_binding_is_rechecked_before_write(self):
        asset = self.base.joinpath("synthetic-resource.txt")
        asset.write_text("initial synthetic bytes", encoding="utf-8")
        self.f.deliveries[1]["assets"] = [binding(asset)]
        self.f.paths = {name: path.with_name(path.stem + "-resource.json") for name, path in self.f.paths.items()}

        def change_after_resource_binding(_done, _total, label):
            if label == "source_map block records":
                asset.write_text("changed synthetic bytes", encoding="utf-8")

        report = self.f.write(progress=change_after_resource_binding)
        self.assertIn("MAP_INPUT_CHANGED", {row.code for row in report.findings})
        self.assertFalse(any(path.exists() for path in self.f.paths.values()))


class TextVisualConflictTests(unittest.TestCase):
    """Caller-supplied synthetic readings test records, never OCR correctness."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.f = Fixture(self.base)

    def arguments(self, unit_id="main-title", *, pending=False):
        unit = next(row for row in self.f.inventory["units"] if row["id"] == unit_id)
        source = next(row for row in self.f.inventory["sources"] if row["id"] == unit["source"])
        original = unit["evidence"][0]["quote"]
        text = Path(source["pages"][0]["text"]["path"]).read_bytes().decode("utf-8")
        start = text.index(original)
        visible = original.replace("Synthetic", "Fictional")
        return {
            "conflict_id": unit_id + "-reading", "page_number": 1,
            "text_locator": {"kind": "characters", "unit": "unicode_code_points", "start": start,
                             "end": start + len(original), "end_exclusive": True},
            "text_layer_text": original, "visible_text": None if pending else visible,
            "bbox": unit["evidence"][0]["bbox"],
            "decision": {"status": "pending", "reason": "Synthetic observation has not been supplied."} if pending else {
                "status": "resolved", "selected_text": visible,
                "reason": "Synthetic caller-declared reading used to exercise provenance checks.",
                "observed": True, "reviewer": "synthetic reviewer"},
        }

    def record(self, unit_id="main-title", **changes):
        arguments = self.arguments(unit_id)
        arguments.update(changes)
        result = maps.record_text_visual_conflict(self.f.inventory_path, unit_id, **arguments)
        self.f.inventory = read(self.f.inventory_path)
        return result

    def test_one_record_reaches_main_and_si_views_without_retyping(self):
        expected = {identity: self.record(identity) for identity in ("main-title", "si-title")}
        result = self.f.write()
        self.assertTrue(result.records_ready, result.as_dict())
        for name, unit_id in (("source_map", "main-title"), ("si_map", "si-title")):
            document = read(self.f.paths[name])
            for collection in ("units", "metadata"):
                row = next(item for item in document[collection] if item["unit"] == unit_id)
                self.assertEqual(row["text_visual_conflicts"], [expected[unit_id]])
                self.assertEqual(row["registered_unit"]["text_visual_conflicts"], [expected[unit_id]])
            self.assertEqual(document["source_fidelity"], "not_assessed")
        self.assertTrue(self.f.validate().records_ready)

    def test_pending_decision_is_saved_projected_and_blocks_final_validation(self):
        arguments = self.arguments(pending=True)
        maps.record_text_visual_conflict(self.f.inventory_path, "main-title", **arguments)
        written = self.f.write()
        self.assertTrue(written.records_ready, written.as_dict())
        row = read(self.f.paths["source_map"])["metadata"][0]
        self.assertEqual(row["text_visual_conflicts"][0]["decision"]["status"], "pending")
        self.assertIsNone(row["text_visual_conflicts"][0]["visible_page"]["text"])
        self.assertIn("MAP_CONFLICT_UNRESOLVED", {item.code for item in self.f.validate().findings})

    def test_summary_without_conflict_fields_does_not_replace_the_decision(self):
        self.record()
        self.assertTrue(self.f.write().records_ready)
        document = read(self.f.paths["source_map"])
        document["metadata"][0].pop("text_visual_conflicts")
        document["metadata"][0]["summary"] = "The conflict was reviewed."
        save(self.f.paths["source_map"], document)
        self.assertIn("MAP_UNIT_FIELD", {item.code for item in self.f.validate().findings})

    def test_minimal_legacy_map_cannot_discard_a_registered_conflict(self):
        self.record()
        self.assertTrue(self.f.write().records_ready)
        document = read(self.f.paths["source_map"])
        document.pop("units")
        document.pop("metadata")
        save(self.f.paths["source_map"], document)
        self.assertIn("MAP_CONFLICT_MISSING", {item.code for item in self.f.validate().findings})

    def test_duplicate_record_is_rejected_without_changing_inventory(self):
        self.record()
        before = self.f.inventory_path.read_bytes()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.record()
        self.assertEqual(self.f.inventory_path.read_bytes(), before)

    def test_invalid_human_decision_or_source_locator_is_not_saved(self):
        valid = self.arguments()
        cases = [
            {"text_layer_text": "A string absent from the actual text file."},
            {"visible_text": None},
            {"decision": {"status": "resolved", "selected_text": valid["visible_text"], "reason": ""}},
            {"decision": {"status": "resolved", "selected_text": "unrecorded third reading", "reason": "Synthetic reason."}},
            {"decision": {"status": "resolved", "selected_text": valid["visible_text"], "reason": "Synthetic reason.", "observed": False}},
            {"page_number": True}, {"page_number": 2}, {"bbox": [0, 0, 9999, 9999]},
        ]
        before = self.f.inventory_path.read_bytes()
        for changes in cases:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                maps.record_text_visual_conflict(self.f.inventory_path, "main-title", **{**valid, **changes})
            self.assertEqual(self.f.inventory_path.read_bytes(), before)

    def test_line_locator_can_retain_an_explicit_reading_from_the_actual_line(self):
        record = self.record(text_locator={"kind": "lines", "start": 1, "end": 1})
        self.assertEqual(record["text_layer"]["locator"]["kind"], "lines")
        self.assertTrue(self.f.write().records_ready)
        self.assertTrue(self.f.validate().records_ready)

    def test_changed_page_text_and_render_bindings_are_rechecked(self):
        record = self.record()
        self.assertTrue(self.f.write().records_ready)
        for name in ("text_layer", "visible_page"):
            with self.subTest(name=name):
                path = Path(record[name]["file"]["path"])
                original = path.read_bytes()
                path.write_bytes(original + b"synthetic change")
                self.assertIn("MAP_BINDING_HASH", {item.code for item in self.f.validate().findings})
                path.write_bytes(original)

    def test_conflict_page_file_cannot_point_to_another_actual_source(self):
        self.record()
        record = self.f.inventory["units"][0]["text_visual_conflicts"][0]
        other = next(row for row in self.f.inventory["sources"] if row["id"] == "si")
        record["visible_page"]["file"] = binding(Path(other["pages"][0]["render"]["path"]))
        save(self.f.inventory_path, self.f.inventory)
        result = self.f.write()
        self.assertIn("MAP_BINDING_TARGET", {item.code for item in result.findings})
        self.assertFalse(any(path.exists() for path in self.f.paths.values()))

    def test_apply_visible_text_edits_records_flow_without_a_second_manual_entry(self):
        arguments = self.arguments()
        unit = self.f.inventory["units"][0]
        page = self.f.inventory["sources"][0]["pages"][0]
        result = maps._module("format_helpers").apply_visible_text_edits(
            text_file=page["text"], edits=[{
                "id": arguments["conflict_id"], "start": arguments["text_locator"]["start"],
                "end": arguments["text_locator"]["end"], "expected_text": arguments["text_layer_text"],
                "visible_page": {"file": binding(Path(page["render"]["path"])), "page": 1,
                                 "bbox": arguments["bbox"], "coordinate_system": sources.PDF_BBOX_COORDINATES,
                                 "text": arguments["visible_text"]}, "decision": arguments["decision"],
            }])
        unit["text_visual_conflicts"] = result["text_visual_conflicts"]
        save(self.f.inventory_path, self.f.inventory)
        self.assertTrue(self.f.write().records_ready)
        self.assertTrue(self.f.validate().records_ready)
        actual = read(self.f.paths["source_map"])["metadata"][0]["text_visual_conflicts"]
        self.assertEqual(actual, result["text_visual_conflicts"])


class RichHandoffTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.f = Fixture(Path(temporary.name), rich=True)
        report = self.f.write()
        self.assertTrue(report.records_ready, report.as_dict())

    def test_each_repeated_and_table_link_has_its_own_record(self):
        report = self.f.validate()
        self.assertTrue(report.records_ready, report.as_dict())
        links = read(self.f.paths["link_map"])
        self.assertEqual(len(links["internal_links"]), 6)
        table_links = [row for row in links["internal_links"] if row["candidate_locator"]["kind"] == "rich_table_text"]
        self.assertEqual(len(table_links), 3)
        self.assertTrue(any("/sheets/1/rows/0/cells/b" == row["candidate_locator"]["json_pointer"] for row in table_links))
        self.assertEqual(sum(row["display_text"] == "One" for row in links["internal_links"]), 2)
        self.assertEqual(len(links["external_links"]), 1)
        self.assertEqual(links["library_matches"][0]["match_status"], "unavailable")
        self.assertEqual(links["unresolved"][0]["status"], "unavailable")

    def test_empty_category_cannot_hide_registered_objects_references_or_citations(self):
        path = self.f.paths["source_map"]
        value = read(path)
        value.update({"objects": [], "references": [], "citations": []})
        save(path, value)
        report = self.f.validate()
        self.assertFalse(report.records_ready)
        self.assertIn("MAP_UNIT_MISSING", {row.code for row in report.findings})

    def test_missing_one_occurrence_and_wrong_registered_target_fail(self):
        path = self.f.paths["link_map"]
        original = read(path)
        value = deepcopy(original)
        value["internal_links"].pop()
        save(path, value)
        self.assertIn("MAP_LINK_COVERAGE", {row.code for row in self.f.validate().findings})
        value = deepcopy(original)
        value["internal_links"][0]["target_unit"] = "table-unit"
        save(path, value)
        self.assertIn("MAP_LINK_TARGET_UNIT", {row.code for row in self.f.validate().findings})

    def test_unknown_reference_cannot_be_fixed_by_a_plausible_anchor(self):
        value = read(self.f.inventory_path)
        next(row for row in value["units"] if row["kind"] == "citation")["references"] = ["unregistered-reference"]
        save(self.f.inventory_path, value)
        self.assertIn("MAP_REFERENCE_UNKNOWN", {row.code for row in self.f.validate().findings})

    def test_each_known_call_requires_its_complete_registered_target_set(self):
        value = read(self.f.inventory_path)
        call = next(row for row in value["units"] if row["id"] == "call-first")
        original_reference = next(row for row in value["units"] if row["id"] == "ref-unit")
        additional = deepcopy(original_reference)
        additional.update({"id": "ref-second", "number": 2})
        value["units"].append(additional)
        call["references"].append("ref-second")
        save(self.f.inventory_path, value)
        self.assertIn("MAP_CITATION_COVERAGE", {row.code for row in self.f.validate().findings})

    def test_duplicate_occurrence_and_wrong_table_pointer_fail(self):
        path = self.f.paths["link_map"]
        original = read(path)
        value = deepcopy(original)
        copied = deepcopy(value["internal_links"][0])
        copied["id"] = "another-record-for-one-occurrence"
        value["internal_links"].append(copied)
        save(path, value)
        self.assertIn("MAP_LINK_COVERAGE", {row.code for row in self.f.validate().findings})
        value = deepcopy(original)
        table = next(row for row in value["internal_links"] if row["candidate_locator"]["kind"] == "rich_table_text")
        table["candidate_locator"]["json_pointer"] = "/sheets/9/rows/9/cells/missing"
        save(path, value)
        self.assertIn("MAP_LINK_LOCATOR", {row.code for row in self.f.validate().findings})

    def test_malformed_link_origin_and_unresolved_ids_do_not_crash(self):
        path = self.f.paths["link_map"]
        original = read(path)
        for key in ("origin_unit", "origin_block", "target_unit"):
            with self.subTest(key=key):
                value = deepcopy(original)
                value["internal_links"][0][key] = []
                save(path, value)
                self.assertFalse(self.f.validate().records_ready)
        value = deepcopy(original)
        value["unresolved"][0]["unit"] = []
        value["unresolved"][0]["occurrence_id"] = {}
        value["unresolved"][0]["status"] = []
        save(path, value)
        self.assertFalse(self.f.validate().records_ready)

    def test_missing_source_number_or_display_is_a_clear_writer_requirement(self):
        value = read(self.f.inventory_path)
        next(row for row in value["units"] if row["id"] == "table-unit").pop("original_number")
        next(row for row in value["units"] if row["id"] == "call-first").pop("display_text")
        save(self.f.inventory_path, value)
        self.f.paths = {name: path.with_name(path.stem + "-new.json") for name, path in self.f.paths.items()}
        report = self.f.write()
        self.assertFalse(report.records_ready)
        self.assertIn("MAP_SOURCE_FIELD_MISSING", {row.code for row in report.findings})
        self.assertIn("MAP_LINK_SOURCE_DISPLAY_MISSING", {row.code for row in report.findings})
        self.assertFalse(any(path.exists() for path in self.f.paths.values()))


class CandidateNewlineTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)

    def assert_raw_round_trip(self, fixture):
        original = fixture.candidate.read_bytes()
        text = original.decode("utf-8")
        self.assertIn(b"\r\n", original)
        report = fixture.write()
        self.assertTrue(report.records_ready, report.as_dict())
        self.assertTrue(fixture.validate().records_ready)
        self.assertEqual(fixture.candidate.read_bytes(), original)
        self.assertEqual(report.checked_files["candidate"]["sha256"], sha256(original).hexdigest())
        for name in ("source_map", "si_map"):
            for row in read(fixture.paths[name])["units"]:
                location = row["candidate_locator"]
                value = text[location["start"]:location["end"]]
                self.assertEqual(location["sha256"], sha256(value.encode("utf-8")).hexdigest())
                if "anchor" in row["target"]:
                    self.assertTrue(value.endswith("^" + row["target"]["anchor"]))
        for row in read(fixture.paths["link_map"])["anchors"]:
            location = row["candidate_locator"]
            value = text[location["start"]:location["end"]]
            self.assertEqual(value, "^" + row["target_id"])
            self.assertTrue(location["start"] == 0 or text[location["start"] - 1].isspace())
            self.assertTrue(text[location["end"]:].startswith(("\r\n", "\n")))
            self.assertEqual(location["sha256"], sha256(value.encode("utf-8")).hexdigest())

    def test_raw_crlf_candidate_keeps_hashes_and_character_offsets(self):
        self.assert_raw_round_trip(Fixture(self.base, candidate_eol="\r\n"))

    def test_raw_crlf_rich_tables_close_before_following_links_and_code(self):
        fixture = Fixture(self.base, rich=True, candidate_eol="\r\n")
        self.assert_raw_round_trip(fixture)
        links = read(fixture.paths["link_map"])
        self.assertEqual(len(links["internal_links"]), 6)
        self.assertEqual(len(links["external_links"]), 1)
        self.assertEqual(len(links["library_matches"]), 1)
        self.assertEqual(sum(row["candidate_locator"]["kind"] == "rich_table_text"
                             for row in links["internal_links"]), 3)
        self.assertNotIn("fake-anchor", {row["target_id"] for row in links["anchors"]})

    def test_mixed_endings_line_selector_uses_lf_hash_and_raw_offsets(self):
        fixture = Fixture(self.base, candidate_eol="\n")
        lines = fixture.candidate.read_bytes().decode("utf-8").splitlines()
        text = "".join(line + ("\r\n" if index % 2 else "\n") for index, line in enumerate(lines))
        fixture.candidate.write_bytes(text.encode("utf-8"))
        raw_lines = text.splitlines(keepends=True)
        first = next(index for index, line in enumerate(raw_lines) if line.startswith("The main sample"))
        selected = "".join(raw_lines[first:first + 2])
        self.assertIn("\r\n", selected)
        delivery = next(row for row in fixture.deliveries if row["unit"] == "main-body")
        delivery["target"] = {"lines": [first + 1, first + 2],
                              "sha256": sha256(selected.encode("utf-8")).hexdigest()}
        invalid = fixture.write()
        self.assertFalse(invalid.records_ready)
        self.assertIn("MAP_TARGET_LINES_HASH", {row.code for row in invalid.findings})
        self.assertFalse(any(path.exists() for path in fixture.paths.values()))
        delivery["target"]["sha256"] = sha256(selected.replace("\r\n", "\n").encode("utf-8")).hexdigest()
        self.assert_raw_round_trip(fixture)
        mapped = next(row for row in read(fixture.paths["source_map"])["units"] if row["unit"] == "main-body")
        location = mapped["candidate_locator"]
        self.assertEqual(text[location["start"]:location["end"]], selected)
        self.assertNotEqual(location["sha256"], mapped["target"]["sha256"])

    def test_lf_multiline_quote_resolves_to_original_crlf_characters(self):
        fixture = Fixture(self.base, candidate_eol="\r\n")
        quote = "The main sample has an orange marker.\n^main-body"
        delivery = next(row for row in fixture.deliveries if row["unit"] == "main-body")
        delivery["target"] = {"quote": quote}
        self.assert_raw_round_trip(fixture)
        text = fixture.candidate.read_bytes().decode("utf-8")
        mapped = next(row for row in read(fixture.paths["source_map"])["units"] if row["unit"] == "main-body")
        location = mapped["candidate_locator"]
        self.assertEqual(mapped["target"]["quote"], quote)
        self.assertEqual(text[location["start"]:location["end"]], quote.replace("\n", "\r\n"))


if __name__ == "__main__":
    unittest.main(argv=[__file__])
