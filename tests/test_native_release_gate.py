"""Test the release policy independently of compiled binaries."""
from copy import deepcopy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("native_release_gate", ROOT.joinpath("scripts", "check_native_release.py"))
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class ReleaseGateTests(unittest.TestCase):
    def test_exact_five_native_target_sets_match_ci_and_catalogs(self):
        expected = {
            "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
        }
        manifest = json.loads(gate.MANIFEST.read_text(encoding="utf-8"))
        self.assertEqual(sorted(manifest["required_targets"]), sorted(expected))
        self.assertEqual(gate.TARGETS, expected)
        for file in ("ocr-cpu-assets.json", "minicpm-runtime-assets.json", "jev-runtime-assets.json"):
            data = json.loads(ROOT.joinpath("native", file).read_text(encoding="utf-8"))
            self.assertEqual(set(data["targets"]), expected, msg=file)
        workflow = yaml.load(ROOT.joinpath(".github", "workflows", "native.yml").read_text(encoding="utf-8"),
                             Loader=yaml.BaseLoader)
        jobs = workflow["jobs"]

        def matrix(job):
            rows = jobs[job]["strategy"]["matrix"]["include"]
            self.assertEqual(len(rows), len({row["target"] for row in rows}), msg=job)
            self.assertNotIn("macos-15-intel", {row["runner"] for row in rows})
            mac = [row for row in rows if row["target"].endswith("-apple-darwin")]
            self.assertEqual(mac, [{"runner": "macos-15", "target": "aarch64-apple-darwin"}], msg=job)
            return {row["target"] for row in rows}

        self.assertEqual(matrix("native"), expected)
        self.assertEqual(matrix("minicpm-native-runtime"), expected)
        self.assertEqual(matrix("ocr-cpu-platforms"), expected - {"x86_64-unknown-linux-gnu"})
        self.assertEqual(jobs["ocr-cpu-smoke"]["runs-on"], "ubuntu-22.04")
        self.assertIn("actual_cpu_ocr_scan_with_verified_package", str(jobs["ocr-cpu-smoke"]["steps"]))
        self.assertIn("actual_cpu_ocr_scan_with_verified_package", str(jobs["ocr-cpu-platforms"]["steps"]))
        self.assertEqual(matrix("minicpm-model-smoke"), {"x86_64-unknown-linux-gnu", "aarch64-apple-darwin"})
        self.assertIn("qualify_minicpm_models.py", str(jobs["minicpm-model-smoke"]["steps"]))
        self.assertIn("Z2O_LLAMA_PREPARE_JEV", str(jobs["minicpm-model-smoke"]["steps"]))
        self.assertNotIn("ocr-intel-macos-resource-gate", jobs)

    def test_unsupported_intel_target_refused_before_fixture_side_effects(self):
        for script, prefix in (("prepare_ocr_ci_fixture.py", "OCR"), ("prepare_minicpm_ci.py", "LLAMA")):
            with self.subTest(script=script), tempfile.TemporaryDirectory() as td:
                output = Path(td) / "must-not-create"
                env = os.environ.copy()
                env.update({f"Z2O_{prefix}_TARGET": "x86_64-apple-darwin",
                            f"Z2O_{prefix}_FIXTURE_DIR": str(output)})
                verified = subprocess.run([sys.executable, str(ROOT / "scripts" / script)],
                                          env=env, capture_output=True, text=True, timeout=10)
                self.assertNotEqual(verified.returncode, 0)
                self.assertIn("Unsupported native", verified.stderr)
                self.assertFalse(output.exists())

    def test_actual_reference_inventory(self):
        manifest = json.loads(gate.MANIFEST.read_text(encoding="utf-8"))
        ref = gate.reference_inventory(ROOT)
        self.assertEqual(set(manifest["module_status"]), set(ref))
        self.assertIn("note_pipeline.process_content", ref["note_pipeline"])

    def test_release_requires_qualified_modules_and_capabilities(self):
        manifest = json.loads(gate.MANIFEST.read_text(encoding="utf-8"))
        ref = {m: {f"{m}.example"} for m in manifest["module_status"]}
        manifest["api_replacements"] = {}
        manifest["module_status"] = dict.fromkeys(manifest["module_status"], "implementation_candidate")
        for cap in manifest["capabilities"].values():
            cap["qualified"] = False
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
        for targets in (manifest["required_targets"][:-1],
                        manifest["required_targets"] + ["x86_64-apple-darwin"],
                        manifest["required_targets"] + [manifest["required_targets"][0]]):
            wrong = dict(manifest, required_targets=targets)
            wrong_result = gate.evaluate(wrong, description, ref, "2.0.0", "2.0.0", True)
            self.assertFalse(wrong_result["ready_for_release"])
            self.assertTrue(any("exactly the five" in err for err in wrong_result["configuration_errors"]))
        self.assertFalse(gate.evaluate(manifest, description, ref, "2.0.0", "2.0.0", False)["ready_for_release"])
        missing = deepcopy(description)
        missing["operations"].pop()
        self.assertFalse(gate.evaluate(manifest, missing, ref, "2.0.0", "2.0.0", True)["ready_for_release"])
        manifest["capabilities"].pop("native_installation_and_external_integration")
        self.assertTrue(gate.evaluate(manifest, description, ref, "2.0.0", "2.0.0", True)["configuration_errors"])


if __name__ == "__main__":
    unittest.main()
