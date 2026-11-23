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
