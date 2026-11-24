"""Test the release policy independently of compiled binaries."""
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("native_release_gate", ROOT.joinpath("scripts", "check_native_release.py"))
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class ReleaseGateTests(unittest.TestCase):
    def test_actual_reference_inventory(self):
        manifest = json.loads(gate.MANIFEST.read_text(encoding="utf-8"))
        ref = gate.reference_inventory(ROOT)
        self.assertEqual(set(manifest["module_status"]), set(ref))
        self.assertIn("note_pipeline.process_content", ref["note_pipeline"])

    def test_release_requires_qualified_modules_and_capabilities(self):
        manifest = json.loads(gate.MANIFEST.read_text(encoding="utf-8"))
        ref = {m: {f"{m}.example"} for m in manifest["module_status"]}
        manifest["api_replacements"] = {}
        description = {"engine": "rust-native", "engine_version": "2.0.0", "reference_version": "1.1.0",
                       "system_python_required": False, "python_fallback": False, "full_workflow_ready": False,
                       "operations": sorted(set().union(*ref.values()))}
        self.assertFalse(gate.evaluate(manifest, description, ref, "2.0.0", "2.0.0", True)["ready_for_release"])
        description["full_workflow_ready"] = True
        self.assertTrue(gate.evaluate(manifest, description, ref, "2.0.0", "2.0.0", True)["configuration_errors"])
        manifest["module_status"] = dict.fromkeys(manifest["module_status"], "qualified")
        for cap in manifest["capabilities"].values():
            cap["qualified"] = True
        result = gate.evaluate(manifest, description, ref, "2.0.0", "2.0.0", True, "refs/tags/v2.0.0")
        self.assertTrue(result["ready_for_release"], result)
        self.assertFalse(gate.evaluate(manifest, description, ref, "2.0.0", "2.0.0", False)["ready_for_release"])
        missing = deepcopy(description)
        missing["operations"].pop()
        self.assertFalse(gate.evaluate(manifest, missing, ref, "2.0.0", "2.0.0", True)["ready_for_release"])
        manifest["capabilities"].pop("native_installation_and_external_integration")
        self.assertTrue(gate.evaluate(manifest, description, ref, "2.0.0", "2.0.0", True)["configuration_errors"])


if __name__ == "__main__":
    unittest.main()
