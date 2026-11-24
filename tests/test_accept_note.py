"""End-to-end contract, corruption and confinement regressions for mechanical acceptance."""
from __future__ import annotations

from copy import deepcopy
from hashlib import sha256
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1].joinpath("literature-zotero2obsidian", "scripts")
SPEC = importlib.util.spec_from_file_location("accept_note", SCRIPTS.joinpath("accept_note.py"))
acceptance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(acceptance)
HEAD = "---\ntype: article\n---\n\n"
FIGURE = "![A](<fig%20one.png>)  \n![B](<fig%20one.png>)  \n**图A** 示例\n^fig-A"
TABLE = """```rich-table
version: 2
locked: true
theme: grid
columns:
  - id: a
    name: A
rows:
  - id: r
    cells:
      a: '1'
```
^tbl-A"""
BODY = "\n\n".join((FIGURE, "$$\nx=y\n$$\n^eq-A", TABLE, "## 参考文献\n1. A. ^ref-1"))


def digest(text):
    return sha256(text.encode()).hexdigest()


class TestAccept(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.path = self.root.joinpath("notes", "candidate.md")
        self.path.parent.mkdir()
        self.path.write_text(HEAD + BODY + "\n", encoding="utf-8")
        self.source = self.root.joinpath("paper.bin")
        self.source.write_bytes(b"source")
        self.root.joinpath("fig one.png").write_bytes(b"figure")
        self.contract = dict(schema_version=1, sources={"s": {"path": str(self.source), "sha256": digest("source")}},
                             required_anchors=["fig-A", "eq-A", "tbl-A", "ref-1"],
                             counts=dict(figures=1, equations=1, images=2, tables=1, bibliography=1),
                             table_dimensions={"tbl-A": [{"rows": 1, "columns": 1}]},
                             resources={"fig%20one.png": digest("figure")})

    def audit(self, contract, **kwargs):
        return acceptance.accept_note(self.path, contract, resource_root=self.root, **kwargs)

    def failures(self, result):
        return {item["code"] for item in result["checks"] if item["status"] == "fail"}

    def test_frozen_structural_contract_and_idempotence(self):
        result = self.audit(self.contract)
        self.assertEqual(self.failures(result), set(), result["checks"])
        self.assertEqual(result, self.audit(self.contract))
        self.assertEqual(result["source_coverage"], "verified_against_contract")
        self.assertEqual(result["source_fidelity"], "not_assessed")
        for item in result["checks"]:
            self.assertTrue({"stage", "code", "location", "expected", "actual", "input_sha256", "rules_sha256"} <= item.keys())

    def test_unbound_structure_and_fake_manifest(self):
        r = self.audit(None)
        self.assertEqual((r["source_coverage"], r["source_fidelity"]), ("not_assessed", "not_assessed"))
        r = self.audit({"schema_version": 1, "passed": True, "sources": {}, "required_units": []})
        self.assertTrue({"CONTRACT_FIELDS", "SCHEMA"} <= self.failures(r))

    def test_missing_objects_partial_table_tampered_source_and_resource(self):
        self.path.write_text(HEAD + TABLE, encoding="utf-8")
        self.contract["table_dimensions"]["tbl-A"][0]["columns"] = 2
        self.source.write_bytes(b"new")
        fails = self.failures(self.audit(self.contract))
        self.assertTrue({"OBJECT_COUNT", "TABLE_DIMENSIONS", "REQUIRED_ANCHOR", "SOURCE_HASH", "RESOURCE_EMBEDDED"} <= fails)
        self.path.write_text(HEAD + FIGURE, encoding="utf-8")
        self.root.joinpath("fig one.png").write_bytes(b"changed")
        self.assertIn("RESOURCE_HASH", self.failures(self.audit(self.contract)))

    def test_fake_counts_in_fences_comments_or_unrelated_lists(self):
        self.path.write_text(HEAD + "```txt\n^fig-A\n$$x$$\n## 参考文献\n1. example\n```\n"
                             "<!-- ^tbl-A -->\n1. ordinary\n", encoding="utf-8")
        result = self.audit(self.contract)
        self.assertEqual(set(result["counts"].values()), {0})
        self.assertIn("OBJECT_COUNT", self.failures(result))

    def test_corrupt_yaml_grid_and_anchors(self):
        for text, expected in [
            ("---\nkey: a\nkey: b\n---", "FRONTMATTER_YAML"),
            (HEAD + TABLE.replace("a: '1'", "unknown: '1'"), "TABLE_CELL_COLUMN"),
            (HEAD + TABLE.replace("locked: true", "locked: ["), "TABLE_YAML"),
            (HEAD + "^sample\n\n^sample", "DUPLICATE_ANCHOR"),
            (HEAD + "[[ #^x ]]\n[b](<#^missing>)", "MISSING_ANCHOR"),
            (HEAD + "[go](<#absent>)", "INTERNAL_TARGET"),
        ]:
            with self.subTest(expected=expected):
                self.path.write_text(text, encoding="utf-8")
                self.assertIn(expected, self.failures(self.audit(None)))

    def test_absent_file_traversal_symlink_and_url_punctuation(self):
        outside = self.root.parent.joinpath("outside.png")
        self.root.joinpath("escape.png").symlink_to(outside)
        self.root.joinpath("a#b c%20.png").write_bytes(b"x")
        self.path.write_text(HEAD + "![](<a%23b%20c%2520.png?size=2#view>)", encoding="utf-8")
        self.assertFalse(self.failures(self.audit(None)))
        self.assert_question_mark_resource("a#b?c%20.png", "![](<a%23b%3Fc%2520.png?size=2#view>)")
        for href, code in [("../outside.png", "RESOURCE_BOUNDARY"), ("%2e%2e/x", "RESOURCE_BOUNDARY"),
                           ("escape.png", "RESOURCE_BOUNDARY"), ("missing.png", "RESOURCE_FILE")]:
            with self.subTest(href=href):
                self.path.write_text(HEAD + f"![](<{href}>)", encoding="utf-8")
                self.assertIn(code, self.failures(self.audit(None)))

    def test_real_line_mapping_and_false_claims(self):
        text = HEAD + "first\n\nsecond\n"
        self.path.write_bytes(text.encode("utf-8"))
        units = [dict(unit_id=uid, kind="paragraph", source={"source_id": "s", "locator": uid},
                      scope=scope) for uid, scope in [("p", "main"), ("q", "supplement:B")]]
        contract = {k: self.contract[k] for k in ("schema_version", "sources")}
        contract["required_units"] = units
        mapping = [dict(**unit, start_line=i, end_line=i, sha256=digest(word))
                   for unit, i, word in zip(units, [5, 7], ["first", "second"])]
        rendering = dict(status="rendered", schema_version=1, mapping=mapping, output_sha256=digest(text))
        self.assertFalse(self.failures(self.audit(contract, rendering=rendering)))
        self.assertIn("SCHEMA", self.failures(self.audit(contract)))
        for field, bad, code in [("scope", "main", "UNIT_SCOPE"), ("unit_id", "p", "UNIT_ORDER"),
                                 ("unit_id", "unknown", "UNIT_ORDER"), ("start_line", 5, "UNIT_SPAN"),
                                 ("sha256", digest("other"), "UNIT_HASH")]:
            copy = deepcopy(rendering)
            copy["mapping"][1][field] = bad
            self.assertIn(code, self.failures(self.audit(contract, rendering=copy)))
        self.path.write_bytes(text.replace("first", "forged").encode("utf-8"))
        self.assertTrue({"UNIT_HASH", "OUTPUT_HASH"} <= self.failures(self.audit(contract, rendering=rendering)))
        units[1]["unit_id"] = "p"
        self.assertIn("UNIQUE_UNIT_IDS", self.failures(self.audit(contract, rendering=rendering)))

    def test_unmapped_extra_prose_and_bogus_object_kind(self):
        self.path.write_text(HEAD + "unmapped\npretend figure\n", encoding="utf-8")
        unit = dict(unit_id="fig", kind="figure", source={"source_id": "s", "locator": "p1"})
        contract = dict(schema_version=1, sources=self.contract["sources"], required_units=[unit])
        rendering = dict(schema_version=1, status="rendered", output_sha256=acceptance._file_hash(self.path),
                         mapping=[dict(**unit, start_line=6, end_line=6, sha256=digest("pretend figure"))])
        self.assertTrue({"UNMAPPED_CONTENT", "UNIT_OBJECT"} <= self.failures(self.audit(contract, rendering=rendering)))

    def test_visible_table_caption_footer_and_opaque_cell_literals(self):
        decorated = TABLE.replace("theme: grid", "theme: grid\nfooter: '[more](<#missing>) ![](ghost.png)'")
        self.path.write_text(HEAD + decorated, encoding="utf-8")
        self.assertTrue({"RESOURCE_FILE", "INTERNAL_TARGET"} <= self.failures(self.audit(None)))
        literal = TABLE.replace("a: '1'", "a: '`![](ghost.png)`'")
        self.path.write_text(HEAD + literal, encoding="utf-8")
        self.assertFalse(self.failures(self.audit(None)))

    def test_figures_need_live_image_caption_anchor_and_offsets(self):
        self.path.write_text(HEAD + "ordinary text ^fig-A", encoding="utf-8")
        contract = dict(schema_version=1, sources=self.contract["sources"], counts={"figures": 1})
        self.assertIn("OBJECT_COUNT", self.failures(self.audit(contract)))
        text = HEAD + "```markdown\n![](ghost.png)\n```"
        self.path.write_text(text, encoding="utf-8")
        unit = dict(unit_id="f", kind="figure", source={"source_id": "s", "locator":"p1"})
        contract["required_units"] = [unit]
        rendering = dict(schema_version=1, status="rendered", output_sha256=digest(text),
                         mapping=[dict(**unit, start_line=5, end_line=7, start_offset=3, end_offset=-1,
                                       sha256=digest(text[len(HEAD):]))])
        self.assertTrue({"UNIT_OFFSETS", "UNIT_OBJECT"} <= self.failures(self.audit(contract, rendering=rendering)))
        unit.update(anchor_typo="fig-A", kind="figrue")
        self.assertTrue({"SCHEMA", "UNIT_FIELDS"} <= self.failures(self.audit(contract, rendering=rendering)))

    def assert_question_mark_resource(self, filename, embed):
        self.path.write_text(HEAD + embed, encoding="utf-8")
        if os.name == "nt":
            # Win32 disallows a literal '?' in a filename, even if the URI encodes it.
            self.assertTrue({"RESOURCE_FILE", "RESOURCE_READ"} & self.failures(self.audit(None)))
            return
        self.root.joinpath(filename).write_bytes(b"ok")
        self.assertFalse(self.failures(self.audit(None)))

    def test_wiki_embed_uses_literal_percent_and_question_mark(self):
        self.root.joinpath("panel%20name space.png").write_bytes(b"ok")
        self.path.write_text(HEAD + "![[panel%20name space.png|preview]]", encoding="utf-8")
        self.assertFalse(self.failures(self.audit(None)))
        self.assert_question_mark_resource("panel%20name?.png", "![[panel%20name?.png|preview]]")

    def test_canonical_format_original_and_normalized(self):
        original = HEAD + "# Original title\n\n中文标题\n作者：甲\n\n" + r"持续60min，占31.$2\%$。" + "\n"
        self.path.write_text(original, encoding="utf-8")
        result = self.audit(None)
        failures = [c for c in result["checks"] if c["code"] == "CANONICAL_FORMAT" and c["status"] == "fail"]
        self.assertTrue({"60min", r"31.$2\%$", ""} <= {c["actual"] for c in failures})
        self.assertTrue(all(c["line"] and c["rule"] and c["stage"] for c in failures))
        self.assertEqual(self.path.read_text(encoding="utf-8"), original)
        canonical = acceptance.normalizer.normalize_note(original)["text"]
        self.path.write_text(canonical, encoding="utf-8")
        self.assertFalse(self.failures(self.audit(None)))

    def test_canonical_format_preserves_unclosed_and_ambiguous(self):
        text = HEAD + "正文中未闭合的$2"
        self.path.write_text(text, encoding="utf-8")
        result = self.audit(None)
        self.assertIn("CANONICAL_FORMAT", self.failures(result))
        self.assertTrue(any(code.startswith("UNCLOSED_") for code in self.failures(result)))
        self.assertEqual(self.path.read_text(encoding="utf-8"), text)
        self.path.write_text(HEAD + "# Heading\n\nUnclassified subline\n", encoding="utf-8")
        result = self.audit(None)
        self.assertFalse(self.failures(result))
        self.assertIn("AMBIGUOUS_HEADER", {p["code"] for p in result["review_prompts"]})

    def test_malformed_evidence_reports_failed_check(self):
        for bad in ({"sources": None}, {"counts": {"paragraphs": 10}}, {"required_units": [None]},
                    {"required_units": [{"unit_id": "P", "kind": "body", "source": {"source_id": [], "locator": "p"}}]},
                    {"resources": []}, {"schema_version": True}, {"table_dimensions": {"t": "1x1"}}):
            self.assertEqual(self.audit({**self.contract, **bad})["mechanical_status"], "fail")
        self.path.write_bytes(b"\xff")
        self.assertIn("INPUT_INVALID", self.failures(self.audit(None)))


if __name__ == "__main__":
    unittest.main()
