"""Development-only differential tests; the Rust executable never runs Python."""
from __future__ import annotations

import base64
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT.joinpath("literature-zotero2obsidian", "scripts")))
import deterministic_stage_checks as reference
import format_helpers as formatting
import source_url_inventory as urls
import render_note as renderer
sys.path.insert(0, str(ROOT.joinpath('tests')))
from test_render_note import fixture as render_fixture
import test_normalize_note as normalize_suite

BINARY = ROOT.joinpath("native", "target", "debug", "z2o.exe" if sys.platform == "win32" else "z2o")


def encoded(data: bytes) -> dict:
    return {"$bytes": base64.b64encode(data).decode("ascii")}


def statuses(report: dict) -> dict:
    return {key: row["status"] for key, row in report["checks"].items()}


class ReferenceComparison(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def invoke(self, operation, *args, **kwargs):
        request = self.root.joinpath("request.json")
        request.write_text(json.dumps({"schema_version": 1, "operation": operation,
                                      "args": args, "kwargs": kwargs}, ensure_ascii=False), encoding="utf-8")
        completed = subprocess.run([str(BINARY), "invoke", str(request)], check=True,
                                   capture_output=True, encoding="utf-8", timeout=45)
        return json.loads(completed.stdout)["result"]

    def test_author_and_collection(self):
        for markers in (["欧阳"], ["van der Waals", "Smith"], ["联合国", "A", "B"]):
            self.assertEqual(self.invoke("deterministic_stage_checks.author_token", markers),
                             reference.author_token(markers))
        library = {"type": "group", "id": 123}
        item = {"library": library, "data": {"collections": ["B"]}}
        records = [
            {"key": "A", "library": library, "data": {"key": "A", "name": "Root", "parentCollection": False}},
            {"key": "B", "library": library, "data": {"key": "B", "name": "Child", "parentCollection": "A"}},
        ]
        self.assertEqual(self.invoke("deterministic_stage_checks.collection_ancestry", item, records,
                                     pagination_complete=True),
                         reference.collection_ancestry(item, records, pagination_complete=True))

    def test_file_binding(self):
        source = self.root.joinpath("论文.md")
        source.write_bytes(b"abc")
        self.assertEqual(self.invoke("deterministic_stage_checks.snapshot", str(source)),
                         reference.snapshot(source))
        self.assertEqual(self.invoke("deterministic_stage_checks.verify_binding", reference.snapshot(source),
                                     str(source), base=str(self.root)),
                         reference.verify_binding(reference.snapshot(source), source, base=self.root))

    def test_properties_byte_output_and_audit(self):
        original = b"\xef\xbb\xbf# Title\r\n\r\n---\r\nText\r\n"
        fields = {"类型": "期刊论文", "年份": 2024, "关键词": ["a: b", "a # b", "2020"], "期刊名": "J: X"}
        expected = reference.build_properties(original, item_type="journalArticle", confirmed=fields)
        self.assertEqual(self.invoke("deterministic_stage_checks.build_properties", encoded(original),
                                     item_type="journalArticle", confirmed=fields), encoded(expected))
        old = ("---\r\n类型: '期刊论文'\r\n库内参考文献:\r\n"
               "  - 'old'\r\ncustom: \"2020\"\r\n"
               "nested:\r\n  库内参考文献: retain\r\n---\r\n# Title\r\n").encode()
        self.assertEqual(self.invoke("deterministic_stage_checks.repair_properties", encoded(old)),
                         encoded(reference.repair_properties(old)))
        before, after = self.root.joinpath("before.md"), self.root.joinpath("after.md")
        before.write_bytes(original)
        after.write_bytes(expected)
        self.assertEqual(statuses(self.invoke("deterministic_stage_checks.audit_properties",
                                            str(before), str(after), mode="new", item_type="journalArticle")),
                         statuses(reference.audit_properties(before, after, mode="new", item_type="journalArticle")))

    def test_naming_plan_complete_mapping(self):
        source = self.root.joinpath("source.txt")
        source.write_text("Synthetic source", encoding="utf-8")
        file = reference.snapshot(source)
        evidence = {key: {"file": file, "locator": "line 1"}
                    for key in ("year", "author_token", "original_title")}
        fields = dict(year=2024, author_token="Smith", original_title="a:🌊🌊🌊",
                      source_evidence=evidence, max_utf16_units=17)
        expected = formatting.build_naming_plan(**fields, target_directory=self.root)
        actual = self.invoke("format_helpers.build_naming_plan", **fields, target_directory=str(self.root))
        self.assertEqual(actual, expected)

    def test_figure_caption_and_anchor(self):
        images = [{"target": "a#b.png", "alt": "[A]"}]
        fields = dict(caption="**图1：** 内容", anchor="fig-1", newline="\r\n")
        self.assertEqual(self.invoke("format_helpers.build_figure_block", images, **fields),
                         formatting.build_figure_block(images, **fields))
        self.assertEqual(self.invoke("format_helpers.plan_supplement_figure_caption", "7c", "内容"),
                         formatting.plan_supplement_figure_caption("7c", "内容"))
        self.assertEqual(self.invoke("format_helpers.new_figure_table_anchor",
                                     source_token="ab12", figure_number_token="S7c"),
                         formatting.new_figure_table_anchor(source_token="ab12", figure_number_token="S7c"))

    def test_table_checks(self):
        fence = chr(96) * 3
        note = self.root.joinpath("table.md")
        note.write_text(fence + "rich-table\nversion: 3\nsheets:\n"
                        "  - locked: true\n    theme: grid\n    columns: [a]\n    rows: [b]\n"
                        "  - locked: false\n    theme: grid\n    columns: [a]\n    rows: [b]\n"
                        "    footer: 123\n" + fence + "\n^tbl-1\n", encoding="utf-8")
        self.assertEqual(statuses(self.invoke("deterministic_stage_checks.audit_rich_tables", str(note))),
                         statuses(reference.audit_rich_tables(note)))


    def test_source_url_occurrence_assignment(self):
        value = "https://example.org/Fn_(x)"
        inventory = {"sources": [{"id": "main", "pages": [{"page": 1, "blocks": [
            {"id": "b1", "text": "中文 🌊 (" + value + ")."},
            {"id": "b2", "text": value},
        ]}]}], "units": [
            {"id": "broad", "kind": "metadata", "field": "external_url", "source": "main",
             "value": value, "evidence": [{"page": 1}]},
            {"id": "narrow", "kind": "metadata", "field": "external_url", "source": "main",
             "value": value, "evidence": [{"page": 1, "blocks": ["b1"]}]},
        ]}
        self.assertEqual(self.invoke("source_url_inventory.source_url_coverage", inventory),
                         urls.source_url_coverage(inventory))
        self.assertEqual(self.invoke("source_url_inventory.scan_source_url_occurrences", inventory,
                                     excluded_blocks=["b2"]),
                         urls.scan_source_url_occurrences(inventory, excluded_blocks=["b2"]))


    def test_full_render_reference_bytes_and_source_mapping(self):
        plan, content = render_fixture()
        self.assertEqual(self.invoke("render_note.content_template", plan), renderer.content_template(plan))
        self.assertEqual(self.invoke("render_note.render_note", plan, content),
                         renderer.render_note(plan, content))
        for mutate in (
            lambda p, c: c.pop("eq"),
            lambda p, c: c["eq"].update(latex="tamper"),
            lambda p, c: p["units"][4].update(anchor="fig-1"),
            lambda p, c: p["units"][4].update(latex="a\n$$\n## injected"),
            lambda p, c: p["units"][2]["literals"].update(scale="# forged"),
            lambda p, c: c["body"].update(text="{{link:unknown}}"),
            lambda p, c: p["units"][1].update(level=4),
        ):
            plan, content = render_fixture()
            mutate(plan, content)
            expected = renderer.render_note(plan, content)
            actual = self.invoke("render_note.render_note", plan, content)
            self.assertEqual(actual["status"], expected["status"])
            self.assertEqual(actual["text"], expected["text"])
            self.assertEqual(actual["mapping"], expected["mapping"])
            self.assertEqual(actual["findings"][0]["field"], expected["findings"][0]["field"])



class NativeNormalizationComparison(normalize_suite.NoteNormalization):
    """Run every frozen reference fixture, then compare the entire native report."""
    def check(self, source):
        expected = super().check(source)
        with tempfile.TemporaryDirectory() as folder:
            request = Path(folder).joinpath("request.json")
            request.write_text(json.dumps({"schema_version": 1,
                "operation": "normalize_note.normalize_note", "args": [source]},
                ensure_ascii=False), encoding="utf-8")
            completed = subprocess.run([str(BINARY), "invoke", str(request)], check=True,
                capture_output=True, encoding="utf-8", timeout=45)
            actual = json.loads(completed.stdout)["result"]
        self.assertEqual(actual, expected)
        return actual

    def test_exclusive_destination(self):
        super().test_exclusive_destination()
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source, destination = root.joinpath("原稿.md"), root.joinpath("规范.md")
            source.write_bytes(b"60 min\r\n")
            request = root.joinpath("request.json")
            request.write_text(json.dumps({"schema_version": 1,
                "operation": "normalize_note.normalize_file",
                "args": [str(source), str(destination)]}, ensure_ascii=False), encoding="utf-8")
            result = subprocess.run([str(BINARY), "invoke", str(request)], check=True,
                capture_output=True, encoding="utf-8", timeout=45)
            report = json.loads(result.stdout)["result"]
            self.assertEqual(destination.read_bytes(), report["text"].encode())
            repeated = subprocess.run([str(BINARY), "invoke", str(request)],
                capture_output=True, encoding="utf-8", timeout=45)
            self.assertNotEqual(repeated.returncode, 0)
            self.assertEqual(source.read_bytes(), b"60 min\r\n")



class NativeValidatorMixin:
    """Run the original validation suites unchanged using native findings."""
    validator_module = None

    def setUp(self):
        super().setUp()
        import dataclasses
        from unittest.mock import patch

        module = self.validator_module
        reference_note = module.validate_note
        reference_file = module.validate_file
        reference_inspect = reference_note

        def compare(actual, expected):
            expected_rows = [dataclasses.asdict(row) for row in expected]
            self.assertEqual([(r["level"], r["code"], r["line"]) for r in actual],
                             [(r["level"], r["code"], r["line"]) for r in expected_rows])
            for a, b in zip(actual, expected_rows):
                if a["code"] not in {"FRONTMATTER_YAML", "TABLE_YAML", "FILE_READ"}:
                    self.assertEqual(a["message"], b["message"])
            return [module.Finding(**row) for row in actual]

        def native(operation, value):
            with tempfile.TemporaryDirectory() as folder:
                request = Path(folder).joinpath("request.json")
                request.write_text(json.dumps({"schema_version": 1, "operation": operation,
                    "args": [value]}, ensure_ascii=False), encoding="utf-8")
                process = subprocess.run([str(BINARY), "invoke", str(request)],
                    capture_output=True, encoding="utf-8", check=True, timeout=45)
                return json.loads(process.stdout)["result"]

        def validate_note(text, **kwargs):
            expected_records = []
            expected = reference_inspect(text, table_records=expected_records)
            actual = native("validate_note.inspect_note", text)
            self.assertEqual(actual["table_records"], expected_records)
            if kwargs.get("table_records") is not None:
                kwargs["table_records"].extend(actual["table_records"])
            return compare(actual["findings"], expected)

        def validate_file(path):
            expected = reference_file(path)
            actual = native("validate_note.validate_file", str(path))
            return compare(actual, expected)

        for name, replacement in (("validate_note", validate_note), ("validate_file", validate_file)):
            patcher = patch.object(module, name, replacement)
            patcher.start()
            self.addCleanup(patcher.stop)


import test_validate_note as validate_suite
import test_rich_tables as table_suite


class NativeStaticValidationComparison(NativeValidatorMixin, validate_suite.ValidateNoteTests):
    validator_module = validate_suite.validator


class NativeRichTableComparison(NativeValidatorMixin, table_suite.RichTableTests):
    validator_module = table_suite.validator


class NativeGlossaryComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_glossary_retrieval_and_saved_evidence(self):
        import test_glossary_candidates as suite
        glossary = self.root.joinpath("术语表.md")
        glossary.write_bytes(("\ufeff| ID | English | Chinese | Context |\r\n"
            "| --- | --- | --- | --- |\r\n"
            "| T1 | radial velocity | 径向速度 | centre |\r\n"
            "| T2 | root mean square；别名/缩写：RMS | 均方根 | statistic |\r\n"
            "| T3 | tangential velocity | 切向速度 | tangent |\r\n").encode())
        index = suite.glossary_tools.GlossaryIndex(glossary)
        for query in ("RADIAL", "RMS", "切向", "rad", "unlisted estimator"):
            self.assertEqual(self.invoke("glossary_candidates.search", str(glossary), query),
                             index.search(query))
        for text in ("🌊 Radial-velocity and RMS were recorded.", "The radial spacing changed.",
                     "radially spaced", "Tangential velocity", ""):
            for limit in (1, 8):
                self.assertEqual(self.invoke("glossary_candidates.suggest", str(glossary), text,
                                             max_family_entries=limit),
                                 index.suggest(text, max_family_entries=limit))
        source = self.root.joinpath("source.pdf")
        source.write_bytes(b"source identity")
        import hashlib
        inventory = self.root.joinpath("inventory.json")
        inventory.write_text(json.dumps({"sources": [{"id": "main", "path": source.name,
             "sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "pages": [{"page": 1,
             "blocks": [{"id": "b1", "bbox": [0, 0, 20, 30], "text": "Radial spacing."}]}]}]}),
             encoding="utf-8")
        expected = self.root.joinpath("expected.json")
        actual = self.root.joinpath("actual.json")
        suite.glossary_tools.prepare_glossary_candidates(inventory, glossary, expected)
        self.assertEqual(self.invoke("glossary_candidates.prepare_glossary_candidates",
                                     str(inventory), str(glossary), str(actual)), str(actual.resolve()))
        self.assertEqual(json.loads(actual.read_text(encoding="utf-8")),
                         json.loads(expected.read_text(encoding="utf-8")))


class NativePipelineComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_plan_contract_replay_and_mapping(self):
        import test_note_pipeline as suite
        source = self.root.joinpath("source.txt")
        source.write_bytes(b"source")
        plan = {"schema_version": 1, "properties": {"type": "article"}, "units": [
            {"unit_id": "title", "kind": "header", "original_title": "A source", "translated": True,
             "authors": ["Author"], "affiliations": ["Institute"],
             "source": {"source_id": "main", "locator": "title"}, "scope": "main"},
            {"unit_id": "p", "kind": "paragraph", "source": {"source_id": "main", "locator": "p1"}, "scope": "main"}]}
        content = {"title": {"translated_title": "来源"}, "p": {"text": "每60 min观测，变化7.00%。"}}
        expected = suite.pipeline.build_contract(plan, {"main": source})
        actual = self.invoke("note_pipeline.build_contract", plan, {"main": str(source)})
        self.assertEqual(actual, expected)
        rendered = self.invoke("render_note.render_note", plan, content)
        normalized = self.invoke("normalize_note.normalize_note", rendered["text"])
        self.assertEqual(self.invoke("note_pipeline.replay_changes", rendered["text"], normalized["changes"]),
                         suite.pipeline.replay_changes(rendered["text"], normalized["changes"]))
        self.assertEqual(self.invoke("note_pipeline.rebase_rendering", rendered, normalized),
                         suite.pipeline.rebase_rendering(rendered, normalized))
        job = self.invoke("note_pipeline.prepare_content_job", plan, {"main": str(source)}, str(self.root.joinpath("jobs")))
        d = Path(job["directory"])
        self.assertEqual(json.loads(d.joinpath("contract.json").read_text(encoding="utf-8")), expected)
        self.assertEqual(json.loads(d.joinpath("content-template.json").read_text(encoding="utf-8")),
                         renderer.content_template(plan))
        self.assertEqual(self.invoke("note_pipeline.read_json", str(d.joinpath("request.json")))["engine"]["engine"],
                         "rust-native")


import test_delivery_presentation as presentation_suite

class NativePresentationComparison(presentation_suite.PresentationTests):
    def setUp(self):
        super().setUp()
        from dataclasses import asdict
        from unittest.mock import patch
        reference_lint = presentation_suite.lint_presentation

        def native(text):
            expected = reference_lint(text)
            with tempfile.TemporaryDirectory() as folder:
                request = Path(folder).joinpath("request.json")
                request.write_text(json.dumps({"schema_version": 1,
                    "operation": "lint_presentation.lint_presentation", "args": [text]},
                    ensure_ascii=False), encoding="utf-8")
                completed = subprocess.run([str(BINARY), "invoke", str(request)], check=True,
                    capture_output=True, encoding="utf-8", timeout=45)
                actual = json.loads(completed.stdout)["result"]
            # Preserve each original prompt, not just pass/fail or code sets.
            self.assertEqual(actual, [asdict(row) for row in expected])
            return expected

        patcher = patch.object(presentation_suite, "lint_presentation", native)
        patcher.start()
        self.addCleanup(patcher.stop)


import test_accept_note as acceptance_suite

class NativeAcceptanceComparison(acceptance_suite.TestAccept):
    def setUp(self):
        super().setUp()
        from unittest.mock import patch
        reference = acceptance_suite.acceptance.accept_note
        def native(candidate_path, contract=None, *, resource_root=None, rendering=None):
            expected = reference(candidate_path, contract, resource_root=resource_root, rendering=rendering)
            with tempfile.TemporaryDirectory() as folder:
                request = Path(folder).joinpath("request.json")
                request.write_text(json.dumps({"schema_version": 1, "operation": "accept_note.accept_note",
                    "args": [str(candidate_path)], "kwargs": {"contract": contract,
                    "resource_root": str(resource_root) if resource_root is not None else None, "rendering": rendering}},
                    ensure_ascii=False), encoding="utf-8")
                completed = subprocess.run([str(BINARY), "invoke", str(request)], check=True,
                    capture_output=True, encoding="utf-8", timeout=90)
                actual = json.loads(completed.stdout)["result"]
            for key in ("mechanical_status", "source_coverage", "source_fidelity"):
                self.assertEqual(actual[key], expected[key], key)
            self.assertEqual({r["code"] for r in actual["checks"] if r["status"] == "fail"},
                             {r["code"] for r in expected["checks"] if r["status"] == "fail"})
            if not any(r["code"] == "INPUT_INVALID" for r in expected["checks"]):
                self.assertEqual(actual["counts"], expected["counts"])
                def rows(report):
                    return sorted((r["code"], r["location"], r["status"]) for r in report["checks"])
                self.assertEqual(rows(actual), rows(expected))
            # Native rules hash binds the native executable, not the old Python files.
            self.assertEqual(actual["inputs"]["candidate_sha256"], expected["inputs"]["candidate_sha256"])
            return actual
        patcher = patch.object(acceptance_suite.acceptance, "accept_note", native)
        patcher.start()
        self.addCleanup(patcher.stop)


if __name__ == "__main__":
    unittest.main(verbosity=2)
