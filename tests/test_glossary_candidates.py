"""Search retrieval must remain source-bound and must never adopt a translation."""
from contextlib import contextmanager
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT.joinpath("literature-translate-zh", "scripts", "glossary_candidates.py")
SPEC = importlib.util.spec_from_file_location("test_glossary_candidates_tools", SCRIPT)
assert SPEC and SPEC.loader
glossary_tools = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = glossary_tools
SPEC.loader.exec_module(glossary_tools)


class GlossaryCandidateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.glossary = self.root.joinpath("glossary.md")
        self.glossary.write_text(
            "| ID | English | Chinese | Context | Source |\n"
            "| --- | --- | --- | --- | --- |\n"
            "| T0101 | radial velocity | 径向速度 | Relative to the stated centre. | example |\n"
            "| T0102 | tangential velocity | 切向速度 | Along the tangent. | example |\n"
            "| T0103 | root mean square；别名/缩写：RMS | 均方根 | A statistic. | example |\n",
            encoding="utf-8",
        )
        self.index = glossary_tools.GlossaryIndex(self.glossary)

    def test_exact_phrase_and_alias_are_retrieved_at_source_positions(self):
        text = "Radial-velocity and RMS were recorded."
        matches = self.index.suggest(text)["matches"]
        exact = [item for item in matches if "exact_token_phrase" in item["match_types"]]
        self.assertEqual({item["entry"]["id"] for item in exact}, {"T0101", "T0103"})
        self.assertTrue(all(text[item["source_span"]["start"]:item["source_span"]["end"]] == item["source_span"]["quote"] for item in exact))

    def test_shared_core_word_is_a_lead_not_an_adopted_whole_term(self):
        result = self.index.suggest("The radial spacing changed.")
        self.assertEqual(len(result["matches"]), 1)
        match = result["matches"][0]
        self.assertEqual(match["entry"]["id"], "T0101")
        self.assertEqual(match["match_types"], ["shared_token_family"])
        self.assertEqual(match["decision"], "pending")
        self.assertEqual(result["source_fidelity"], "not_assessed")

    def test_frequency_bound_limits_family_noise_without_losing_exact_phrase(self):
        result = self.index.suggest("Tangential velocity", max_family_entries=1)
        exact = [row for row in result["matches"] if "exact_token_phrase" in row["match_types"]]
        self.assertEqual([row["entry"]["id"] for row in exact], ["T0102"])
        self.assertFalse(any(row["query"] == "velocity" for row in result["matches"]))

    def test_word_boundary_does_not_match_an_embedded_character_substring(self):
        self.assertEqual(self.index.suggest("radially spaced")['matches'], [])
        self.assertEqual(self.index.search("rad"), [])

    def test_queries_return_real_rows_and_zero_hits(self):
        self.assertEqual([row["id"] for row in self.index.search("RADIAL")], ["T0101"])
        self.assertEqual([row["id"] for row in self.index.search("RMS")], ["T0103"])
        self.assertEqual(self.index.search("unlisted estimator"), [])
        self.assertEqual([row["id"] for row in self.index.search("切向")], ["T0102"])

    def test_query_copy_cannot_change_later_evidence_bindings(self):
        first = self.index.search("radial")[0]
        first["evidence"]["sha256"] = "wrong"
        self.assertEqual(self.index.search("radial")[0]["evidence"]["sha256"], hashlib.sha256(self.glossary.read_bytes()).hexdigest())

    def test_invalid_queries_and_duplicate_ids_fail_explicitly(self):
        with self.assertRaises(ValueError):
            self.index.search(" ")
        with self.assertRaises(ValueError):
            self.index.suggest("radial", max_family_entries=0)
        self.glossary.write_text(self.glossary.read_text(encoding="utf-8") + "| T0101 | another word | 另一词 | other | example |\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            glossary_tools.GlossaryIndex(self.glossary)

    def test_legacy_three_column_rows_and_non_numeric_ids_remain_searchable(self):
        self.glossary.write_text("| sample-radial | radial spacing | 径向间距 |\n", encoding="utf-8")
        hit = glossary_tools.GlossaryIndex(self.glossary).search("radial spacing")[0]
        self.assertEqual(hit["id"], "sample-radial")
        self.assertEqual(hit["translation"], "径向间距")
        self.assertEqual(hit["context"], "")
        self.assertEqual(hit["line"], 1)
        self.assertEqual(hit["evidence"]["sha256"], hashlib.sha256(self.glossary.read_bytes()).hexdigest())

    def test_headers_separators_and_optional_edge_pipes_are_not_terms(self):
        self.glossary.write_text(
            "ID | English | Chinese\n"
            ":--- | ---: | :---:\n"
            "sample-radial | radial spacing | 径向间距\n"
            "ID | identifier label | 身份标签\n"
            "\n"
            "| ID | English | Chinese | Context |\n"
            "| --- | --- | --- | --- |\n"
            "| sample-offset | radial offset | 径向偏移量 | Fictional geometry. |\n",
            encoding="utf-8",
        )
        index = glossary_tools.GlossaryIndex(self.glossary)
        self.assertEqual(set(index.entries), {"sample-radial", "ID", "sample-offset"})
        self.assertEqual(index.search("English"), [])
        self.assertEqual(index.search("identifier label")[0]["id"], "ID")
        self.assertEqual(index.search("radial offset")[0]["line"], 8)

    def test_duplicate_non_numeric_entry_ids_are_rejected(self):
        self.glossary.write_text(
            "| sample-radial | radial spacing | 径向间距 |\n"
            "| sample-radial | radial offset | 径向偏移量 |\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "Duplicate glossary entry: sample-radial"):
            glossary_tools.GlossaryIndex(self.glossary)

    def test_glossary_rows_and_hash_use_the_same_read_snapshot(self):
        alias_parent = self.root.joinpath("alias-directory")
        alias_parent.mkdir()
        self.glossary = alias_parent.joinpath("..", self.glossary.name)
        actual_path = self.glossary.resolve(strict=True)
        original = self.glossary.read_bytes()
        replacement = original.replace("径向速度".encode("utf-8"), "变更后的译名".encode("utf-8"))
        open_file = Path.open
        replaced = False

        @contextmanager
        def replace_after_read(path, mode="r", *args, **kwargs):
            nonlocal replaced
            with open_file(path, mode, *args, **kwargs) as stream:
                yield stream
            if mode != "rb" or replaced or path.resolve(strict=True) != actual_path:
                return
            replaced = True
            with open_file(path, "wb") as stream:
                stream.write(replacement)

        with patch.object(Path, "open", replace_after_read):
            index = glossary_tools.GlossaryIndex(self.glossary)
        hit = index.search("radial velocity")[0]
        self.assertTrue(replaced)
        self.assertEqual(self.glossary.read_bytes(), replacement)
        self.assertEqual(hit["translation"], "径向速度")
        self.assertEqual(hit["evidence"]["sha256"], hashlib.sha256(original).hexdigest())

    def test_bom_and_crlf_glossary_keep_actual_byte_binding(self):
        raw = "| ID | English | Chinese |\r\n| --- | --- | --- |\r\n| sample-radial | radial spacing | 径向间距 |\r\n"
        actual = b"\xef\xbb\xbf" + raw.encode("utf-8")
        self.glossary.write_bytes(actual)
        hit = glossary_tools.GlossaryIndex(self.glossary).search("radial spacing")[0]
        self.assertEqual(hit["line"], 3)
        self.assertEqual(hit["evidence"]["sha256"], hashlib.sha256(actual).hexdigest())

    def inventory(self):
        source = self.root.joinpath("source.pdf")
        source.write_bytes(b"Synthetic source binding; this retrieval helper never interprets PDF bytes.")
        inventory = self.root.joinpath("inventory.json")
        inventory.write_text(json.dumps({"sources": [{"id": "main", "path": str(source),
            "sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "pages": [{"page": 1,
            "blocks": [{"id": "main-p1-b0", "bbox": [0, 0, 80, 30], "text": "Radial spacing."}]}]}]}), encoding="utf-8")
        return inventory, source

    def test_saved_leads_bind_actual_inputs_without_changing_them(self):
        inventory, source = self.inventory()
        originals = {path: path.read_bytes() for path in (inventory, source, self.glossary)}
        output = self.root.joinpath("leads.json")
        progress = []
        glossary_tools.prepare_glossary_candidates(inventory, self.glossary, output, progress=lambda *args: progress.append(args))
        data = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(data["blocks"][0]["matches"][0]["decision"], "pending")
        self.assertEqual(data["bindings"]["source_inventory"]["sha256"], hashlib.sha256(inventory.read_bytes()).hexdigest())
        self.assertEqual(len(progress), 1)
        self.assertTrue(all(path.read_bytes() == raw for path, raw in originals.items()))
        with self.assertRaises(FileExistsError):
            glossary_tools.prepare_glossary_candidates(inventory, self.glossary, output)

    def test_changed_source_bytes_cannot_be_given_old_inventory_identity(self):
        inventory, source = self.inventory()
        source.write_bytes(b"Changed")
        output = self.root.joinpath("leads.json")
        with self.assertRaises(ValueError):
            glossary_tools.prepare_glossary_candidates(inventory, self.glossary, output)
        self.assertFalse(output.exists())

    def test_inputs_changed_during_retrieval_are_rejected_before_output(self):
        for changed_input in ("inventory", "glossary", "source"):
            with self.subTest(changed_input=changed_input):
                inventory, source = self.inventory()
                output = self.root.joinpath("changed-" + changed_input + ".json")
                changed_path = {"inventory": inventory, "glossary": self.glossary, "source": source}[changed_input]

                def change_input(*_args):
                    if changed_input == "inventory":
                        data = json.loads(inventory.read_text(encoding="utf-8"))
                        data["sources"][0]["pages"][0]["blocks"][0]["text"] = "Tangential spacing."
                        inventory.write_text(json.dumps(data), encoding="utf-8")
                        return
                    changed_path.write_bytes(changed_path.read_bytes() + b"\n")

                with self.assertRaisesRegex(ValueError, "Input changed during glossary search"):
                    glossary_tools.prepare_glossary_candidates(inventory, self.glossary, output, progress=change_input)
                self.assertFalse(output.exists())

    def test_output_created_during_retrieval_is_never_overwritten(self):
        inventory, _source = self.inventory()
        output = self.root.joinpath("existing-during-search.json")
        previous = b"An existing output must be retained.\n"

        def create_output(*_args):
            output.write_bytes(previous)

        with self.assertRaises(FileExistsError):
            glossary_tools.prepare_glossary_candidates(inventory, self.glossary, output, progress=create_output)
        self.assertEqual(output.read_bytes(), previous)


if __name__ == "__main__":
    unittest.main()
