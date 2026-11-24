"""Development-only differential tests; the Rust executable never runs Python."""
from __future__ import annotations

import base64
import json
import hashlib
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


class NativeEndToEndComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_process_markdown_and_content_native_receipts(self):
        import test_note_pipeline as suite
        source = self.root.joinpath("paper.txt")
        source.write_bytes(b"source evidence")
        note = self.root.joinpath("candidate.md")
        note.write_text("---\ntype: article\n---\n\n# Example\n\n## 1 方法\n每60 min观测，增加31.$2\\%$。\n", encoding="utf-8")
        contract = {"schema_version": 1, "sources": {"main": {"path": str(source),
            "sha256": hashlib.sha256(source.read_bytes()).hexdigest()}},
            "counts": {"figures": 0, "equations": 0, "tables": 0, "bibliography": 0}}
        for selected in (contract, None):
            expected = suite.pipeline.process_markdown(note, self.root.joinpath("python"),
                contract=selected, resource_root=self.root, progress=None)
            actual = self.invoke("note_pipeline.process_markdown", str(note), str(self.root.joinpath("rust")),
                contract=selected, resource_root=str(self.root))
            self.assertEqual(actual["status"], expected["status"])
            self.assertEqual(actual["source_fidelity"], "not_assessed")
            self.assertEqual(Path(actual["directory"]).joinpath("normalized.md").read_bytes(),
                             Path(expected["directory"]).joinpath("normalized.md").read_bytes())
            self.assertEqual(actual, self.invoke("note_pipeline.process_markdown", str(note),
                str(self.root.joinpath("rust")), contract=selected, resource_root=str(self.root)))
            for name, record in actual["outputs"].items():
                path = Path(record["path"])
                self.assertEqual(path.name, name)
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), record["sha256"])
        plan = {"schema_version": 1, "properties": {"type": "article"}, "units": [
            {"unit_id": "title", "kind": "header", "original_title": "A source", "translated": True,
             "authors": ["Author"], "affiliations": ["Institute"], "scope": "main",
             "source": {"source_id": "main", "locator": "title"}},
            {"unit_id": "p", "kind": "paragraph", "scope": "main",
             "source": {"source_id": "main", "locator": "p"}}]}
        content = {"title": {"translated_title": "来源"}, "p": {"text": "每60 min观测。"}}
        contract = suite.pipeline.build_contract(plan, {"main": source})
        for supplied in (content, {}):
            expected = suite.pipeline.process_content(plan, supplied, self.root.joinpath("python"),
                contract=contract, resource_root=self.root, progress=None)
            actual = self.invoke("note_pipeline.process_content", plan, supplied, str(self.root.joinpath("rust")),
                contract=contract, resource_root=str(self.root))
            self.assertEqual(actual["status"], expected["status"])
            self.assertEqual(actual["source_fidelity"], expected["source_fidelity"])
            if supplied:
                for name in ("normalized.md", "rendering.json", "normalization.json"):
                    self.assertEqual(Path(actual["directory"]).joinpath(name).read_bytes(),
                                     Path(expected["directory"]).joinpath(name).read_bytes())


import test_docx_source_inventory as docx_suite

class NativeDocxComparison(docx_suite.NativeDocxTests):
    def setUp(self):
        super().setUp()
        from unittest.mock import patch
        module = docx_suite.native
        read_reference = module.read_docx
        prepare_reference = module.prepare_docx_source
        native_document = module.NativeDocument
        errors_reference = module.native_evidence_errors

        def invoke(operation, *args):
            with tempfile.TemporaryDirectory() as folder:
                request = Path(folder).joinpath("request.json")
                request.write_text(json.dumps({"schema_version": 1,
                    "operation": "docx_source_inventory." + operation,
                    "args": args}, ensure_ascii=False), encoding="utf-8")
                completed = subprocess.run([str(BINARY), "invoke", str(request)],
                    capture_output=True, encoding="utf-8", check=True, timeout=90)
                return json.loads(completed.stdout)["result"]

        def read(path, source_id, *, progress=None):
            try:
                expected = read_reference(path, source_id, progress=progress)
            except ValueError:
                with self.assertRaises(subprocess.CalledProcessError):
                    invoke("read_docx", str(path), source_id)
                raise
            actual = invoke("read_docx", str(path), source_id)
            self.assertEqual(actual, expected)
            return actual

        def prepare(path, source_id, directory, *, progress=None):
            with tempfile.TemporaryDirectory() as folder:
                expected_dir = Path(folder).joinpath("reference")
                expected = prepare_reference(path, source_id, expected_dir, progress=progress)
                if Path(directory).joinpath("document.xml").exists():
                    with self.assertRaises(subprocess.CalledProcessError):
                        invoke("prepare_docx_source", str(path), source_id, str(directory))
                    raise FileExistsError(str(directory))
                actual = invoke("prepare_docx_source", str(path), source_id, str(directory))
                def compare_records(a, b):
                    if isinstance(a, dict) and isinstance(b, dict):
                        self.assertEqual(set(a), set(b))
                        for key in a:
                            if key == "path":
                                self.assertEqual(Path(a[key]).name, Path(b[key]).name)
                                self.assertEqual(Path(a[key]).read_bytes(), Path(b[key]).read_bytes())
                            else:
                                compare_records(a[key], b[key])
                    elif isinstance(a, list) and isinstance(b, list):
                        self.assertEqual(len(a), len(b))
                        for x, y in zip(a, b):
                            compare_records(x, y)
                    else:
                        self.assertEqual(a, b)
                compare_records(actual, expected)
                return actual

        def errors(source, location):
            actual = invoke("native_evidence_errors", source, location)
            self.assertEqual(actual, errors_reference(source, location))
            return actual

        test = self
        class NativeDocument:
            def __init__(self, path):
                self.path = path
                self.reference = native_document(path)
                self.inventory = invoke("read_docx", str(path), "native")
                test.assertEqual(self.inventory, self.reference.inventory)
            def resolve_native_locator(self, locator):
                try:
                    expected = self.reference.resolve_native_locator(locator)
                except ValueError:
                    with test.assertRaises(subprocess.CalledProcessError):
                        invoke("resolve_native_locator", str(self.path), locator)
                    raise
                actual = invoke("resolve_native_locator", str(self.path), locator)
                test.assertEqual(actual, expected)
                return actual

        for name, replacement in (("read_docx", read), ("prepare_docx_source", prepare),
            ("native_evidence_errors", errors), ("NativeDocument", NativeDocument)):
            patcher = patch.object(module, name, replacement)
            patcher.start()
            self.addCleanup(patcher.stop)


class NativePreparationComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_docx_source_inventory_uses_real_files_and_pending_terms(self):
        from prepare_source_inventory import prepare_sources
        from test_docx_source_inventory import make_docx, paragraph, image_call, image_relation, PNG
        source = self.root.joinpath("论文.docx")
        make_docx(source, paragraph("First source.") + image_call("image"),
                  relationships=[image_relation("image", "media/one.png")],
                  media={"word/media/one.png": PNG})
        glossary = self.root.joinpath("术语.md")
        glossary.write_text("test", encoding="utf-8")
        py_root = self.root.joinpath("python")
        rs_root = self.root.joinpath("rust")
        expected_path = prepare_sources({"main": source}, py_root, source_roles={"main": "main"},
                                        glossary_path=glossary, progress=None)
        actual_path = Path(self.invoke("prepare_source_inventory.prepare_docx_sources", {"main": str(source)},
                                        str(rs_root), source_roles={"main": "main"}, glossary_path=str(glossary)))
        def compare(a, b, key=None):
            if isinstance(a, dict):
                self.assertEqual(set(a), set(b))
                for k in a:
                    compare(a[k], b[k], k)
            elif isinstance(a, list):
                self.assertEqual(len(a), len(b))
                for x, y in zip(a, b):
                    compare(x, y)
            elif key == "path":
                self.assertEqual(Path(a).name, Path(b).name)
                self.assertEqual(Path(a).read_bytes(), Path(b).read_bytes())
            elif key == "terminology_path":
                compare(json.loads(Path(a).read_text(encoding="utf-8")),
                        json.loads(Path(b).read_text(encoding="utf-8")))
            else:
                self.assertEqual(a, b, key)
        compare(json.loads(actual_path.read_text(encoding="utf-8")),
                json.loads(expected_path.read_text(encoding="utf-8")))

    def test_pending_translation_scaffold_source_binding_and_negative_cases(self):
        import test_translation_review as suite
        ref = suite.review
        source = self.root.joinpath("source.pdf")
        source.write_bytes(b"fixture source")
        candidate = self.root.joinpath("candidate.md")
        candidate.write_text("翻译。", encoding="utf-8")
        inventory = {"sources": [{"id":"s","path":source.name,
            "sha256":hashlib.sha256(source.read_bytes()).hexdigest(),
            "pages":[{"page":1,"blocks":[{"id":"b1","bbox":[0,1,3,4],"text":"Source 1"},
                                        {"id":"b2","bbox":[2,5,9,10],"text":"Source 2"}]}]}],
            "units":[{"id":"p1","source":"s","kind":"paragraph",
                      "evidence":[{"basis":"text_blocks","page":1,"blocks":["b2","b1"]}]},
                     {"id":"p2","source":"s","kind":"paragraph","evidence":[{"page":1,"quote":"Source 2"}]}]}
        inv = self.root.joinpath("inventory.json")
        inv.write_text(json.dumps(inventory), encoding="utf-8")
        py_out, rs_out = self.root.joinpath("python.json"), self.root.joinpath("rust.json")
        ref.prepare_translation_review(inv,candidate,py_out,unit_ids=["p1"],progress=None)
        self.invoke("prepare_translation_review.prepare_translation_review",str(inv),str(candidate),str(rs_out),
                    unit_ids=["p1"])
        self.assertEqual(json.loads(py_out.read_text(encoding="utf-8")),json.loads(rs_out.read_text(encoding="utf-8")))
        for unit_ids in ([],["p1","p1"],["unknown"]):
            with self.assertRaises(subprocess.CalledProcessError):
                self.invoke("prepare_translation_review.prepare_translation_review",str(inv),str(candidate),
                            str(self.root.joinpath("invalid.json")),unit_ids=unit_ids)
        self.assertFalse(self.root.joinpath("invalid.json").exists())
        loc={"page":1,"quote":"12"}
        self.assertEqual(self.invoke("prepare_translation_review.new_occurrence","num","numbers",source_span=loc),
                         ref.new_occurrence("num","numbers",source_span=loc))
        self.assertEqual(self.invoke("prepare_translation_review.new_term_candidate","t","ocean","concept",
                                     source_excerpt="ocean",source_locator=loc),
                         ref.new_term_candidate("t","ocean","concept",source_excerpt="ocean",source_locator=loc))


class NativeAtomicPreparationComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_actual_repository_rule_catalog(self):
        expected = reference.coverage_catalog(ROOT)
        actual = self.invoke("deterministic_stage_checks.coverage_catalog", str(ROOT))
        self.assertEqual(actual, expected)

    def test_atomic_ledger_creation_and_pending_queries(self):
        import test_check_records as suite
        fixture = suite.CheckRecordsTests(methodName="test_skeleton_discovers_all_atomic_ids_without_resolving_any")
        fixture.setUp()
        try:
            expected = fixture.ledger
            output = fixture.base.joinpath("rust-checks.json")
            self.invoke("check_records.prepare_check_records", str(fixture.skills), str(output),
                        sources={"source-a": str(fixture.pdf)}, candidate_path=str(fixture.candidate),
                        scopes=fixture.scopes, requirement_types=fixture.types, rule_selection="all")
            actual = json.loads(output.read_text(encoding="utf-8"))
            # UTC timestamps describe distinct executions. Compare every other
            # field, including complete rule declarations, selection and hashes.
            expected = dict(expected)
            actual.pop("created_at")
            expected.pop("created_at")
            self.assertEqual(actual, expected)
            from complete_workflow import pending_check_batch, pending_check_rows
            for prefix in (None, "EX", "FMT"):
                self.assertEqual(self.invoke("complete_workflow.pending_check_batch", str(output), prefix=prefix, limit=5),
                                 pending_check_batch(output, prefix=prefix, limit=5))
                self.assertEqual(self.invoke("complete_workflow.pending_check_rows", str(output), prefix=prefix, limit=2),
                                 pending_check_rows(output, prefix=prefix, limit=2))
            locator = {"kind": "lines", "start": 1, "end": 1}
            self.assertEqual(self.invoke("check_records.bind_evidence", str(fixture.candidate), locator),
                             suite.checker.bind_evidence(fixture.candidate, locator))
            with self.assertRaises(subprocess.CalledProcessError):
                self.invoke("check_records.prepare_check_records", str(fixture.skills), str(output),
                            sources={"source-a": str(fixture.pdf)})
        finally:
            fixture.doCleanups()


import test_check_records as check_records_suite

class NativeAtomicValidatorComparison(check_records_suite.CheckRecordsTests):
    """Reuse original adversarial ledger fixtures; compare native diagnostics
    and status counts for every validator call, not just a final pass boolean."""
    def setUp(self):
        super().setUp()
        from unittest.mock import patch
        from collections import Counter
        original = check_records_suite.checker.validate_check_records

        def validate(path, *, progress=None):
            expected = original(path, progress=progress)
            with tempfile.TemporaryDirectory() as directory:
                request = Path(directory).joinpath("request.json")
                request.write_text(json.dumps({"schema_version": 1,
                    "operation": "check_records.validate_check_records", "args": [str(path)]},
                    ensure_ascii=False), encoding="utf-8")
                result = subprocess.run([str(BINARY), "invoke", str(request)], check=True,
                                        capture_output=True, encoding="utf-8", timeout=90)
                actual = json.loads(result.stdout)["result"]
            expected_value = expected.to_dict()
            # Parser diagnostics may differ across libraries, but every
            # independent rule ID, severity, count and readiness gate must match.
            def diagnostics(v):
                return Counter((r["code"], r["object_id"], r["level"]) for r in v["findings"])
            self.assertEqual(diagnostics(actual), diagnostics(expected_value), actual["findings"])
            for key, value in expected_value.items():
                if key != "findings":
                    self.assertEqual(actual[key], value, key)
            return expected
        patcher = patch.object(check_records_suite.checker, "validate_check_records", validate)
        patcher.start()
        self.addCleanup(patcher.stop)

        original_scopes = check_records_suite.checker.scopes_from_deliveries

        def scopes(inventory_path, candidate_path, deliveries, *, unit_ids):
            payload = {"schema_version": 1, "operation": "check_records.scopes_from_deliveries",
                       "args": [str(inventory_path), str(candidate_path), deliveries],
                       "kwargs": {"unit_ids": unit_ids}}
            with tempfile.TemporaryDirectory() as directory:
                request = Path(directory).joinpath("scope.json")
                request.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
                actual = subprocess.run([str(BINARY), "invoke", str(request)], capture_output=True,
                                        encoding="utf-8", timeout=90)
                try:
                    expected = original_scopes(inventory_path, candidate_path, deliveries, unit_ids=unit_ids)
                except ValueError:
                    self.assertNotEqual(actual.returncode, 0, actual.stdout)
                    raise
                self.assertEqual(actual.returncode, 0, actual.stderr)
                self.assertEqual(json.loads(actual.stdout)["result"], expected)
                return expected

        scope_patch = patch.object(check_records_suite.checker, "scopes_from_deliveries", scopes)
        scope_patch.start()
        self.addCleanup(scope_patch.stop)

class NativeAtomicWriterComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_explicit_batch_preserves_previous_decisions_and_evidence(self):
        from complete_workflow import record_check_batch, record_decision_rows
        fixture = check_records_suite.CheckRecordsTests(methodName="test_skeleton_discovers_all_atomic_ids_without_resolving_any")
        fixture.setUp()
        try:
            a = fixture.path
            b = fixture.base.joinpath("native-checks.json")
            b.write_bytes(a.read_bytes())
            evidence = {"result": check_records_suite.checker.bind_evidence(fixture.candidate, {"kind":"lines","start":1,"end":1})}
            scope = fixture.scopes["EX-C01"]
            rows = [{"id":"EX-C01","status":"passed","scope":scope,"reason":"Actual check.","evidence":["result"]}]
            record_check_batch(a, rows, evidence=evidence)
            self.invoke("complete_workflow.record_check_batch",str(b),rows,evidence=evidence)
            contexts = {"table": {"scope":fixture.scopes["FMT-A01"],"evidence":["result"]}}
            decisions = [["FMT-A01","failed","table","Missing cell."]]
            record_decision_rows(a,decisions,contexts=contexts,evidence={})
            self.invoke("complete_workflow.record_decision_rows",str(b),decisions,contexts=contexts,evidence={})
            expected = json.loads(a.read_text(encoding="utf-8"))
            actual = json.loads(b.read_text(encoding="utf-8"))
            for report in (actual,expected):
                for update in report["updates"]:
                    update.pop("at")
            self.assertEqual(actual,expected)
            before = b.read_bytes()
            with self.assertRaises(subprocess.CalledProcessError):
                self.invoke("complete_workflow.record_check_batch",str(b),[dict(rows[0],objective="forged")],evidence={})
            self.assertEqual(b.read_bytes(),before)
        finally:
            fixture.doCleanups()



    def test_prepare_with_inventory_and_explicit_overwrite(self):
        fixture = check_records_suite.CheckRecordsTests(methodName="test_skeleton_discovers_all_atomic_ids_without_resolving_any")
        fixture.setUp()
        try:
            b = fixture.base.joinpath("native-checks.json")
            inventory = fixture.base.joinpath("native-inventory.json")
            inventory.write_bytes(fixture.inventory.read_bytes())
            kwargs = dict(sources={"source-a":str(fixture.pdf)},candidate_path=str(fixture.candidate),
                          scopes=fixture.scopes,requirement_types=fixture.types,rule_selection="all")
            self.invoke("check_records.prepare_check_records",str(fixture.skills),str(b),
                        inventory_path=str(inventory),**kwargs)
            actual=json.loads(b.read_text(encoding="utf-8"))
            expected=dict(fixture.ledger)
            actual.pop("created_at")
            expected.pop("created_at")
            self.assertEqual(actual,expected)
            self.assertEqual(Path(json.loads(inventory.read_text(encoding="utf-8"))["checks_path"]),b.resolve())
            original=b.read_bytes()
            self.invoke("check_records.prepare_check_records",str(fixture.skills),str(b),
                        inventory_path=str(inventory),overwrite=True,**kwargs)
            backups=list(b.parent.glob(".z2o-check-transaction-*/ledger.before"))
            self.assertIn(original,[p.read_bytes() for p in backups])
            self.assertTrue(all(p.parent.joinpath("completed.json").is_file() for p in backups))
            before=fixture.candidate.read_bytes()
            with self.assertRaises(subprocess.CalledProcessError):
                self.invoke("check_records.prepare_check_records",str(fixture.skills),str(fixture.candidate),
                            overwrite=True,**kwargs)
            self.assertEqual(fixture.candidate.read_bytes(),before)
        finally:
            fixture.doCleanups()



import test_format_helpers as format_suite

class NativeVisibleTextEditComparison(format_suite.VisibleTextEditTests):
    def setUp(self):
        super().setUp()
        from unittest.mock import patch
        reference_apply = format_suite.helpers.apply_visible_text_edits

        def apply(*, text_file, edits):
            with tempfile.TemporaryDirectory() as directory:
                request = Path(directory).joinpath("request.json")
                request.write_text(json.dumps({"schema_version":1,
                    "operation":"format_helpers.apply_visible_text_edits",
                    "kwargs":{"text_file":text_file,"edits":edits}}, ensure_ascii=False),
                    encoding="utf-8")
                result = subprocess.run([str(BINARY),"invoke",str(request)],
                    capture_output=True, encoding="utf-8", timeout=90)
                try:
                    expected = reference_apply(text_file=text_file, edits=edits)
                except ValueError:
                    self.assertNotEqual(result.returncode,0,result.stdout)
                    raise
                self.assertEqual(result.returncode,0,result.stderr)
                actual=json.loads(result.stdout)["result"]
                self.assertEqual(actual,expected)
                return expected
        patcher=patch.object(format_suite.helpers,"apply_visible_text_edits",apply)
        patcher.start()
        self.addCleanup(patcher.stop)



import test_draft_queue as draft_suite

class NativeDraftQueueComparison(draft_suite.DraftQueueTests):
    """Drive the existing queue fixtures through the actual Rust executable.
    The adapter holds only the latest explicit queue hash, not a Python engine."""
    def setUp(self):
        super().setUp()
        from unittest.mock import patch
        original_prepare = draft_suite.queue_tools.prepare_draft_queue

        def encode(value):
            if isinstance(value, Path):
                return str(value)
            if isinstance(value, (tuple,list)):
                return [encode(v) for v in value]
            if isinstance(value, dict):
                return {k:encode(v) for k,v in value.items()}
            return value

        def invoke(operation, *args, **kwargs):
            with tempfile.TemporaryDirectory() as directory:
                request = Path(directory).joinpath("request.json")
                request.write_text(json.dumps({"schema_version":1,"operation":operation,
                    "args":encode(args),"kwargs":encode(kwargs)},ensure_ascii=False),encoding="utf-8")
                completed = subprocess.run([str(BINARY),"invoke",str(request)],capture_output=True,
                                            encoding="utf-8",timeout=90)
                if completed.returncode != 0:
                    msg = completed.stderr
                    if any(phrase in msg for phrase in ("Resume the existing queue", "Assemble a new version", "Keep existing output")):
                        raise FileExistsError(msg)
                    if "another script" in msg:
                        raise RuntimeError(msg)
                    raise ValueError(msg)
                return json.loads(completed.stdout)["result"]

        def prepare(inventory,queue,*,progress=None):
            # Independently prepare a reference queue with a distinct path.
            # Its absolute snapshot/parts paths are expected to differ.
            reference = Path(queue).with_name("reference-" + Path(queue).name)
            result = invoke("draft_queue.prepare_draft_queue",inventory,queue)
            if not reference.exists():
                original_prepare(inventory,reference,progress=progress)
                a=json.loads(Path(result).read_text(encoding="utf-8"))
                b=json.loads(reference.read_text(encoding="utf-8"))
                a.pop("parts_root");b.pop("parts_root")
                x=a.pop("inventory_snapshot");y=b.pop("inventory_snapshot")
                self.assertEqual(x["sha256"],y["sha256"])
                self.assertEqual(Path(x["path"]).read_bytes(),Path(y["path"]).read_bytes())
                self.assertEqual(a,b)
            return Path(result)

        class NativeQueue:
            def __init__(self,p):
                self.path=Path(p)
                self._reload()
            @classmethod
            def open(cls,p):
                return cls(p)
            def _reload(self):
                opened=invoke("draft_queue.DraftQueue.open",self.path)
                self.data=opened["data"];self.sha=opened["sha256"]
            def _call(self,operation,*args,mutate=False,**kwargs):
                result=invoke("draft_queue.DraftQueue."+operation,self.path,*args,expected_sha256=self.sha,**kwargs)
                if mutate:
                    self._reload()
                return result
            def status(self):
                return self._call("status")
            def next_task(self):
                return self._call("next_task")
            def review_pdf_page_order(self,source_id,page,ordered_block_ids,reason):
                return self._call("review_pdf_page_order",source_id,page,ordered_block_ids,reason,mutate=True)
            def review_additional_objects(self,source_id,page,reason):
                return self._call("review_additional_objects",source_id,page,reason,mutate=True)
            def add_page_object(self,block_id,source_id,page,**kwargs):
                return self._call("add_page_object",block_id,source_id,page,mutate=True,**kwargs)
            def group(self,unit_id,block_ids,**kwargs):
                return self._call("group",unit_id,block_ids,mutate=True,**kwargs)
            def exclude(self,block_id,reason):
                return self._call("exclude",block_id,reason,mutate=True)
            def write_unit(self,unit_id,markdown,**kwargs):
                return self._call("write_unit",unit_id,markdown,mutate=True,**kwargs)
            def record_issue_decision(self,issue_id,**kwargs):
                return self._call("record_issue_decision",issue_id,mutate=True,**kwargs)
            def assemble(self,output_path,*,progress=None):
                return self._call("assemble",output_path)

        for name,replacement in (("prepare_draft_queue",prepare),("DraftQueue",NativeQueue)):
            patcher=patch.object(draft_suite.queue_tools,name,replacement)
            patcher.start()
            self.addCleanup(patcher.stop)



class NativeDeliveryPreparationComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_manifest_and_source_unit_against_reference(self):
        import validate_delivery as delivery
        import handoff_maps as maps
        inventory=self.root.joinpath("inventory.json")
        candidate=self.root.joinpath("candidate.md")
        asset=self.root.joinpath("figure.png")
        inventory.write_text('{"schema_version":1,"sources":[],"units":[]}',encoding="utf-8")
        candidate.write_text("图注。",encoding="utf-8")
        asset.write_bytes(b"fixture")
        evidence=[{"page":1,"bbox":[0,0,10,10]}]
        fields={"original_number":"S1","contains_table":False}
        expected=maps.make_source_unit("figure","main","figure",evidence,**fields)
        actual=self.invoke("handoff_maps.make_source_unit","figure","main","figure",evidence,**fields)
        self.assertEqual(actual,expected)
        rows=[{"unit":"figure","target":{"anchor":"fig-1"},"assets":[{"path":"figure.png"}]}]
        py=self.root.joinpath("python.json");rs=self.root.joinpath("rust.json")
        delivery.write_delivery_manifest(inventory,candidate,rows,py,resource_root=self.root,figure_edge_reviews_required=True)
        self.invoke("validate_delivery.write_delivery_manifest",str(inventory),str(candidate),rows,str(rs),resource_root=str(self.root),figure_edge_reviews_required=True)
        self.assertEqual(json.loads(py.read_text(encoding="utf-8")),json.loads(rs.read_text(encoding="utf-8")))
        from dataclasses import asdict
        text="中文 [标签](<a b.md> \"title\") ![图](image.png)"
        self.assertEqual(self.invoke("validate_delivery.scan_markdown_links",text),
                         [asdict(v) for v in delivery.scan_markdown_links(text)])



class NativeTranslationLifecycleComparison(unittest.TestCase):
    setUp = ReferenceComparison.setUp
    invoke = ReferenceComparison.invoke

    def test_reference_completed_and_defective_ledgers(self):
        import test_translation_review as suite
        fixture=suite.TranslationRecordValidationTests(methodName="test_complete_synthetic_record_is_ready_without_semantic_certification")
        fixture.setUp()
        try:
            from collections import Counter
            from copy import deepcopy
            ledger=fixture.completed()
            def compare(data):
                fixture.save_ledger(data)
                expected=fixture.validate().to_dict()
                actual=self.invoke("validate_translation_review.validate_translation_review",str(fixture.path))
                self.assertEqual(actual["records_ready"],expected["records_ready"],actual["findings"])
                self.assertEqual(Counter((x["code"],x["object_id"],x["level"]) for x in actual["findings"]),
                                 Counter((x["code"],x["object_id"],x["level"]) for x in expected["findings"]))
                for field in ("status_counts","inventory_unit_count","selected_unit_count","recorded_unit_count","excluded_unit_count","review_status","source_fidelity","execution_truth"):
                    self.assertEqual(actual[field],expected[field],field)
                for field in ("pending_ids","failed_ids","unresolved_ids"):
                    self.assertEqual(sorted(actual[field]),sorted(expected[field]),field)
            compare(ledger)
            mutations=[
                lambda v:v["units"]["paragraph-a"]["occurrences"].update(claims=[]),
                lambda v:v["units"]["paragraph-a"]["checks"].pop("directions"),
                lambda v:v["units"]["paragraph-a"]["candidate_locations"].append({"lines":[99,99]}),
                lambda v:v["units"]["paragraph-a"]["term_candidates"][0]["searches"]["family"].update(status="not_applicable",queries=[]),
                lambda v:v["units"]["paragraph-a"]["term_candidates"][0].update(terminology_pointer="/adopted/999"),
                lambda v:v["units"]["paragraph-a"]["occurrences"]["claims"][0].update(candidate_span={"lines":[2,2]}),
                lambda v:v["bindings"]["candidate"].update(sha256="0"*64),
                lambda v:v["scope"].update(selected_units=["paragraph-a"]),
            ]
            for mutate in mutations:
                data=deepcopy(ledger);mutate(data);compare(data)
        finally:
            fixture.doCleanups()

    def test_replay_real_reference_writer_draft_and_final_files(self):
        import test_translation_review_writer as suite
        fixture=suite.CompactReviewTests(methodName="test_compact_draft_resumes_exact_decisions_and_keeps_remaining_targets")
        fixture.setUp()
        try:
            writer,unit=fixture.active()
            writer.scope("passed","Explicit fixture scope decision.")
            writer.exclude("two","This independent fixture task selects one paragraph.","Second paragraph is outside the selected scope.")
            unit.check("alignment","passed","Actual complete pair was compared.")
            unit.occurrence("claims","claim-a",unit.source(fixture.source_text),unit.candidate(fixture.candidate_text),
                            "failed","Retain explicit failed comparison.")
            term=unit.term("badge","cobalt badge","concept",unit.source("cobalt badge"))
            term.search("full_phrase","cobalt badge","Actual exact search.")
            term.search("family","badge","Actual family search.")
            term.search_status("aliases","not_applicable","No alias in the selected source.")
            term.decide("adopted","钴蓝色徽片","Explicit fixture decision.",matches=["BADGE"])
            unit.scan("concept","scanned",["badge"],"One actual candidate.")
            draft=writer.save_draft(fixture.base.joinpath("reference-draft.json"))
            commands=json.loads(draft.read_text(encoding="utf-8"))["decisions"]
            restored=self.invoke("translation_review_writer.restore_decision_draft",str(draft))
            self.assertEqual(restored["data"],writer.data)
            replay=self.invoke("translation_review_writer.replay_decisions",str(fixture.prepared),commands)
            self.assertEqual(replay["data"],writer.data)
            task=self.invoke("translation_review_tasks.next_review_task",str(fixture.prepared),commands)
            self.assertEqual(task["stage"],"unit")
            actual=self.invoke("translation_review_writer.save_review",str(fixture.prepared),commands,
                               output_path=str(fixture.base.joinpath("rust-review.json")),
                               terminology_path=str(fixture.base.joinpath("rust-terms.json")))
            expected=writer.save(fixture.output,terminology_path=fixture.term_output).to_dict()
            self.assertFalse(actual["records_ready"])
            self.assertEqual(sorted(actual["pending_ids"]),sorted(expected["pending_ids"]))
            self.assertEqual(sorted(actual["failed_ids"]),sorted(expected["failed_ids"]))
            self.assertEqual(json.loads(fixture.base.joinpath("rust-terms.json").read_text(encoding="utf-8")),
                             json.loads(fixture.term_output.read_text(encoding="utf-8")))
        finally:
            fixture.doCleanups()


if __name__ == "__main__":
    unittest.main(verbosity=2)
