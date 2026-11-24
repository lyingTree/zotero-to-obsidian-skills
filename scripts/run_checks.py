"""Run the repository checks and isolated regression suite without CLI arguments."""

from pathlib import Path
import sys
import unittest

from validate_repository import validate_repository


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPOSITORY_ROOT))


def main() -> int:
    print("[1/2] Checking skill metadata, relative links and dependency revisions...", flush=True)
    findings = validate_repository(REPOSITORY_ROOT)
    for finding in findings:
        print(f"{finding.path}:{finding.line}: {finding.code}: {finding.message}")
    print(f"Repository errors: {len(findings)}", flush=True)
    print("[2/2] Running isolated regression tests...", flush=True)
    suite = unittest.defaultTestLoader.discover(str(REPOSITORY_ROOT.joinpath("tests")))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return int(bool(findings) or not result.wasSuccessful())


if __name__ == "__main__":
    raise SystemExit(main())
