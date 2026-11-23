"""An actual second map generation must retain the bound input identities."""
from __future__ import annotations

from copy import deepcopy
from pathlib import Path
import tempfile
import unittest

from test_handoff_maps import Fixture, binding, load, read, save


reruns = load("mapping_rerun")


class MappingRerunTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.fixture = Fixture(self.root, rich=True)
        result = self.fixture.write()
        self.assertTrue(result.records_ready, result.as_dict())
        self.run_path = self.root.joinpath("run.json")
        self.run = {
            "records": {"source_inventory": str(self.fixture.inventory_path),
                        **{name: str(path) for name, path in self.fixture.paths.items()}},
            "config": {"candidate_path": str(self.fixture.candidate)},
            "identity": {"sources": [{"id": identity, **binding(path)}
                                     for identity, path in self.fixture.source_paths.items()]},
            "scope": {"source_roles": {"main": "main", "si": "supplement"}},
        }
        save(self.run_path, self.run)

    def test_actual_new_generation_preserves_identities_and_prior_files(self):
        old_paths = [self.run_path, self.fixture.inventory_path, self.fixture.candidate,
                     *self.fixture.paths.values(), *self.fixture.source_paths.values()]
        before = {str(path): binding(path) for path in old_paths}
        path = reruns.record_mapping_rerun(self.run_path, self.fixture.deliveries, self.root.joinpath("repeat"))
        result = read(path)
        self.assertTrue(result["mapping_equal"])
        self.assertEqual(result["differences"], [])
        self.assertEqual(result["source_fidelity"], "not_assessed")
        self.assertFalse(result["acceptance_status_written"])
        self.assertEqual(set(result["rerun_maps"]), {"source_map", "si_map", "link_map"})
        for name, record in result["rerun_maps"].items():
            self.assertNotEqual(record["path"], str(self.fixture.paths[name]))
            self.assertEqual(record, binding(Path(record["path"])))
        self.assertEqual(before, {str(path): binding(path) for path in old_paths})

    def test_human_review_status_is_not_replaced_by_generated_pending_status(self):
        path = self.fixture.paths["link_map"]
        original = read(path)
        original["review_status"] = "checked"
        for row in original["anchors"]:
            row["review_status"] = "checked"
            row["review_reason"] = "Explicit synthetic anchor identity compared."
        save(path, original)
        report = read(reruns.record_mapping_rerun(self.run_path, self.fixture.deliveries,
                                                self.root.joinpath("repeat")))
        self.assertTrue(report["mapping_equal"])
        self.assertEqual(read(path), original)
        new_map = read(Path(report["rerun_maps"]["link_map"]["path"]))
        self.assertTrue(all(row["review_status"] == "pending" for row in new_map["anchors"]))

    def test_a_different_registered_delivery_selector_is_reported(self):
        changed = deepcopy(self.fixture.deliveries)
        row = next(row for row in changed if row["unit"] == "main-body")
        row["target"] = {"quote": "The main sample has an orange marker.\n^main-body"}
        result = read(reruns.record_mapping_rerun(self.run_path, changed, self.root.joinpath("repeat")))
        self.assertFalse(result["mapping_equal"])
        self.assertTrue(any(row["collection"] == "units" and row["id"] == "main-body"
                            for row in result["differences"]))

    def test_existing_rerun_evidence_is_never_overwritten(self):
        destination = self.root.joinpath("repeat")
        destination.mkdir()
        marker = destination.joinpath("keep.txt")
        marker.write_text("Previous evidence", encoding="utf-8")
        with self.assertRaises(FileExistsError):
            reruns.record_mapping_rerun(self.run_path, self.fixture.deliveries, destination)
        self.assertEqual(marker.read_text(encoding="utf-8"), "Previous evidence")

    def test_changed_candidate_invalidates_baseline_before_new_files_are_written(self):
        self.fixture.candidate.write_text(self.fixture.candidate.read_text(encoding="utf-8") + "\nChanged.\n", encoding="utf-8")
        destination = self.root.joinpath("repeat")
        with self.assertRaises(ValueError):
            reruns.record_mapping_rerun(self.run_path, self.fixture.deliveries, destination)
        self.assertFalse(destination.exists())

    def test_invalid_original_target_cannot_be_certified_by_a_correct_rerun(self):
        path = self.fixture.paths["link_map"]
        data = read(path)
        data["anchors"][0]["target_unit"] = "invented-source-object"
        save(path, data)
        destination = self.root.joinpath("repeat")
        with self.assertRaises(ValueError):
            reruns.record_mapping_rerun(self.run_path, self.fixture.deliveries, destination)
        self.assertFalse(destination.exists())

    def test_input_change_during_baseline_validation_is_not_recaptured_as_original(self):
        changed = False

        def change_candidate(_done, _total, label):
            nonlocal changed
            if changed or label != "source_map block records":
                return
            self.fixture.candidate.write_bytes(self.fixture.candidate.read_bytes() + b"\n")
            changed = True

        destination = self.root.joinpath("repeat")
        with self.assertRaisesRegex(ValueError, "during baseline validation"):
            reruns.record_mapping_rerun(self.run_path, self.fixture.deliveries, destination,
                                        progress=change_candidate)
        self.assertTrue(changed)
        self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
