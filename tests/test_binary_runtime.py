"""Source-mode regressions for the same structured front end used in the binary."""
from __future__ import annotations
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT.joinpath("runtime", "z2o.py")


class BinaryRuntimeTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.request = self.root.joinpath("request.json")

    def run_runtime(self, *args, expect_success=True):
        result = subprocess.run([sys.executable, str(RUNTIME), *map(str, args)],
                                cwd=self.root, capture_output=True, encoding="utf-8",
                                timeout=90)
        self.assertEqual(result.returncode == 0, expect_success, result.stderr)
        return result

    def invoke(self, value, expect_success=True):
        self.request.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")
        result = self.run_runtime("invoke", self.request, expect_success=expect_success)
        return json.loads(result.stdout if expect_success else result.stderr)

    def test_describe_actual_signatures(self):
        result = json.loads(self.run_runtime("describe").stdout)
        self.assertIn("note_pipeline.process_content", result["operations"])
        self.assertIn("resource_root", result["operations"]["accept_note.accept_note"])

    def test_explicit_operations_and_unicode(self):
        result = self.invoke({"schema_version": 1, "operation": "deterministic_stage_checks.author_token",
                              "args": [["欧阳", "Smith"]]})
        self.assertEqual(result["result"], "欧阳 and Smith")
        result = self.invoke({"schema_version": 1, "operation": "os.system"}, expect_success=False)
        self.assertEqual(result["error"], "ValueError")

    def test_path_and_bytes_roundtrip(self):
        source = self.root.joinpath("中文.md")
        source.write_bytes(b"# test\n")
        result = self.invoke({"schema_version": 1, "operation": "deterministic_stage_checks.snapshot",
                              "args": [{"$path": str(source)}]})["result"]
        self.assertEqual(Path(result["path"]), source.resolve())
        self.assertEqual(result["size"], len(source.read_bytes()))
        value = self.invoke({"schema_version": 1, "operation": "deterministic_stage_checks.split_frontmatter",
                              "args": [{"$bytes": "IyB0ZXN0Cg=="}]})["result"]
        self.assertEqual(value[1], {"$bytes": "IyB0ZXN0Cg=="})

    def test_duplicate_json_and_nonfinite_rejected(self):
        for text in ('{"schema_version":1,"schema_version":1,"operation":"x"}',
                     '{"schema_version":1,"operation":"x","args":[NaN]}'):
            self.request.write_text(text, encoding="utf-8")
            result = self.run_runtime("invoke", self.request, expect_success=False)
            self.assertEqual(json.loads(result.stderr)["error"], "ValueError")

    def test_trusted_script_compatibility(self):
        script = self.root.joinpath("task.py")
        script.write_text("from deterministic_stage_checks import author_token\n"
                          "import sys\n"
                          "print(author_token([sys.argv[1]]))\n", encoding="utf-8")
        self.assertEqual(self.run_runtime("run-script", script, "Example").stdout.strip(), "Example")


if __name__ == "__main__":
    unittest.main()
