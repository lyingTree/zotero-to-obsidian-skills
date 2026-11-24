"""Portable tests for the Jammy ARM64 release-smoke policy, not inference."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "qualify_llama_jammy_arm64.py"
SPEC = importlib.util.spec_from_file_location("qualify_llama_jammy_arm64", SCRIPT)
smoke = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(smoke)


class JammyArm64SmokeTests(unittest.TestCase):
    def test_architecture_and_minimum_abi(self):
        smoke.require_baseline({"ID": "ubuntu", "VERSION_ID": "22.04"}, "aarch64", "glibc 2.35")
        for version, machine, glibc in (("24.04", "aarch64", "glibc 2.39"),
                                         ("22.04", "x86_64", "glibc 2.35")):
            with self.assertRaisesRegex(RuntimeError, "no fallback"):
                smoke.require_baseline({"ID": "ubuntu", "VERSION_ID": version}, machine, glibc)
        header = "ELF Header:\n  Machine:                           AArch64\n"
        requirements = "GLIBC_2.4 GLIBC_2.35 GLIBCXX_3.4.29 CXXABI_1.3.13"
        self.assertEqual(smoke.inspect_elf(header, requirements)["glibc"], ["2.4", "2.35"])
        for incompatible in ("GLIBC_2.38", "GLIBC_PRIVATE"):
            with self.assertRaisesRegex(RuntimeError, "ABI"):
                smoke.inspect_elf(header, requirements + " " + incompatible)
        with self.assertRaisesRegex(RuntimeError, "AArch64"):
            smoke.inspect_elf(header.replace("AArch64", "x86-64"), requirements)

    def test_pinned_build_receipt_does_not_claim_inference_or_redistribution(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "receipt"
            build = target / "build"
            engine = build / "bin" / "llama-server"
            payload = b"stand-in for the built executable"

            def commands(args, **kwargs):
                if args[0] == "git":
                    return (smoke.SOURCE_COMMIT + "\n", "") if "rev-parse" in args else ("", "")
                if args[:2] == ["readelf", "--file-header"]:
                    return "  Machine: AArch64\n", ""
                if args[0] == "readelf":
                    return "  Name: GLIBC_2.35\n", ""
                if args[0] == str(engine):
                    self.assertEqual(args[1:], ["--version"])
                    self.assertEqual(kwargs["env"], {"PATH": "", "CUDA_VISIBLE_DEVICES": "", "HIP_VISIBLE_DEVICES": ""})
                    return "", "version: 11901 (5320c87f)\n"
                return args[0] + " mock version\n", ""

            def builder(args, **kwargs):
                self.assertTrue(kwargs["check"])
                if "--build" in args:
                    self.assertIn("llama-server", args)
                    engine.parent.mkdir(parents=True)
                    engine.write_bytes(payload)
                else:
                    self.assertIn("-DBUILD_SHARED_LIBS=OFF", args)
                    self.assertIn("-DGGML_CPU_ARM_ARCH=armv8-a", args)

            with patch.object(smoke, "require_baseline") as baseline, patch.object(
                    smoke.os, "confstr", return_value="glibc 2.35", create=True), patch.object(
                    smoke.platform, "freedesktop_os_release", return_value={"ID": "ubuntu"}, create=True), patch.object(
                    smoke, "output", side_effect=commands), patch.object(smoke.subprocess, "run", side_effect=builder):
                smoke.qualify(root / "source", target)
            baseline.assert_called_once()
            evidence = json.loads((target / "runtime-evidence.json").read_text())
            self.assertEqual(evidence["source"]["commit"], smoke.SOURCE_COMMIT)
            self.assertEqual(evidence["engine"]["sha256"], hashlib.sha256(payload).hexdigest())
            self.assertTrue(evidence["launch_smoke_passed"])
            self.assertFalse(evidence["official_archive_qualified_on_this_host"])
            self.assertFalse(evidence["inference_qualified"])
            self.assertFalse(evidence["redistribution_qualified"])


if __name__ == "__main__":
    unittest.main()
