"""End-to-end serialization with fabricated source units."""
from copy import deepcopy
from hashlib import sha256
from pathlib import Path
import sys
import unittest

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parents[1].joinpath("literature-zotero2obsidian", "scripts")))
from render_note import content_template, render_note


def unit(uid, kind, scope="main", **fields):
    return dict(unit_id=uid, kind=kind, source=dict(source_id="article", locator=f"box/{uid}"), scope=scope, **fields)


def fixture():
    units = [
        unit("h", "header", original_title="Alpha_7", authors=["Example A", "Example B"],
             affiliations=["Institute A"], translated=True),
        unit("section", "heading", level=2),
        unit("body", "paragraph", links={"first": {"target": "r", "label": "Example"}},
             literals={"scale": "$7.00$"}),
        unit("f", "figure", anchor="fig-1", label="图1。", images=[{"target": "a#b.png", "alt": "x[y]"}]),
        unit("eq", "formula", latex=r"a_b = 7.00\tag{1}", anchor="eq-1"),
        unit("tab", "table", data={"version": 2, "locked": True, "theme": "grid", "columns": ["a"],
             "rows": [["07.00"]]}, anchor="tbl-1"),
        unit("r", "reference", source_text="First entry_7.00"),
        unit("note", "translator_note", "supplement:A"),
        unit("sr", "bibliography", "supplement:A", source_text="Supplement 1."),
        unit("end", "zotero_link", uri="zotero://select/library/items/ABCDEFGH"),
    ]
    plan = dict(schema_version=1, units=units, properties={"tags": ["example"], "year": 2020})
    content = content_template(plan)
    content["h"]["translated_title"] = "甲"
    content["section"]["text"] = "引言"
    content["body"]["text"] = "含{{literal:scale}}，见{{link:first}}；保留{not_a_template}。"
    content["f"]["caption"] = "图注。"
    content["note"]["text"] = "图中未标出。"
    return plan, content


class RenderTests(unittest.TestCase):
    def test_render_structure_and_roundtrip(self):
        plan, content = fixture()
        original = deepcopy((plan, content))
        r = render_note(plan, content)
        self.assertEqual(r["status"], "rendered", r["findings"])
        text = r["text"]
        for fragment in (r"# Alpha\_7" + "\n甲\n\nExample A; Example B\n\nInstitute A",
                         "## 1 引言\n含$7.00$，见[Example](<#^ref-1>)",
                         "a%23b.png>)  \n**图1。** 图注。\n^fig-1",
                         "\n\n$$\na_b = 7.00\\tag{1}\n$$\n^eq-1",
                         "```\n^tbl-1", r"1. First entry\_7.00 ^ref-1",
                         "> 译注：图中未标出。", "1. Supplement 1. ^sref-1"):
            self.assertIn(fragment, text)
        self.assertEqual("not_assessed", r["source_fidelity"])
        self.assertEqual((plan, content), original)
        self.assertEqual(r, render_note(plan, dict(reversed(list(content.items())))))
        lines = text.split("\n")
        for row in r["mapping"]:
            chunk = "\n".join(lines[row["start_line"]-1:row["end_line"]])
            self.assertEqual(chunk, text[row["start_offset"]:row["end_offset"]])
            self.assertEqual(sha256(chunk.encode()).hexdigest(), row["sha256"])
        tab = r["mapping"][5]
        table_lines = lines[tab["start_line"]-1:tab["end_line"]]
        self.assertEqual(yaml.safe_load("\n".join(table_lines[2:-3])), plan["units"][5]["data"])

    def test_block_invalid_without_partial_text(self):
        cases = [
            lambda p,c: c.pop("eq"), lambda p,c: c.update(ghost={}),
            lambda p,c: p["units"].append(p["units"][0]),
            lambda p,c: p["units"][2].update(unit_id="section"),
            lambda p,c: c["eq"].update(latex="tamper"),
            lambda p,c: c["tab"].update(data={}), lambda p,c: c["h"].update(authors=["bad"]),
            lambda p,c: p["units"][4].update(anchor=None), lambda p,c: p["units"][4].update(anchor="fig-1"),
            lambda p,c: p["units"][1].update(level=4), lambda p,c: p.update(schema_version=True),
            lambda p,c: p["units"][0].update(authors="text"), lambda p,c: c["body"].update(text="one\ntwo"),
            lambda p,c: p["units"][4].update(latex="a\n$$\n## injected"),
            lambda p,c: c["body"].update(text="{{link:first}}"), lambda p,c: c["body"].update(text="{{link:unknown}}"),
            lambda p,c: p["units"][2]["links"]["first"].update(target="absent"),
            lambda p,c: p["units"][8].update(scope="main"), lambda p,c: p["units"][9].update(uri="javascript:x"),
            lambda p,c: p["units"][4]["source"].update(locator=[]), lambda p,c: c.update(body=42),
        ]
        for case in cases:
            plan, content = fixture()
            case(plan, content)
            with self.subTest(case=case):
                report = render_note(plan, content)
                self.assertEqual(report["status"], "blocked")
                self.assertIsNone(report["text"])
                self.assertFalse(report["mapping"])
                self.assertTrue(report["findings"][0]["field"].startswith("/"))

    def test_semantically_equal_objects_have_identical_bytes(self):
        plan, content = fixture()
        table = plan["units"][5]["data"]
        before = render_note(plan, content)
        plan["units"][5]["data"] = dict(reversed(list(table.items())))
        self.assertEqual(before, render_note(plan, content))

    def test_sparse_plan_does_not_invent_sections(self):
        plan = {"schema_version": 1, "units": [unit("p", "paragraph")]}
        r = render_note(plan, {"p":{"text": "# <tag> [link] _ *"}})
        self.assertEqual(r["text"], r"\# \<tag\> \[link\] \_ \*" + "\n")
        self.assertEqual(render_note(plan, {"p":{"text": "1. list"}})["text"], "1\\. list\n")

    def test_source_headers_metadata_and_lists(self):
        plan = {"schema_version": 1, "units":[
            unit("h", "header", original_title="原题", translated=False,
                 authors=[{"name": "A_B", "numbers": ["1"], "email":"a@test.example","corresponding":True}],
                 affiliations=[{"number":"1","name":"Institute"}], notes=["Received: 2020"]),
            unit("meta", "metadata", notes=["DOI: x"]),
            unit("meta2", "metadata", notes=["Published: 2021"]),
            unit("l", "list", style="ordered", count=2),
        ]}
        content = content_template(plan)
        self.assertEqual(content["l"], {"items":["", ""]})
        content["l"]["items"] = ["7.00%","%%abc%%"]
        rendered = render_note(plan, content)
        self.assertEqual(rendered["status"], "rendered", rendered["findings"])
        self.assertIn(r"[A\_B](mailto:a@test.example)${}^{1,\ast}$", rendered["text"])
        self.assertIn("${}^{1}$Institute\n\n> Received: 2020", rendered["text"])
        self.assertIn("> DOI: x\n> Published: 2021", rendered["text"])
        self.assertIn("1. 7.00%\n2. " + r"\%\%abc\%\%", rendered["text"])
        content["l"]["items"].pop()
        self.assertEqual("blocked", render_note(plan, content)["status"])

    def test_grouped_citation_retains_endpoints_scopes_and_separators(self):
        units = [unit("p", "paragraph", links={"call":{"citation":["r1", ",", "r3", "-", "r4"]}})]
        units.extend(unit("r" + str(i), "reference", source_text=f"Source {i}") for i in range(1,5))
        plan = dict(schema_version=1, units=units)
        content = content_template(plan)
        content["p"]["text"] = "{{link:call}}"
        note = render_note(plan, content)
        self.assertIn(r"<sup>\[[1](<#^ref-1>),[3](<#^ref-3>)-[4](<#^ref-4>)\]</sup>", note["text"])
        units[0]["links"]["call"]["citation"] = ["r1", ",", "p"]
        self.assertEqual("blocked", render_note(plan, content)["status"])

    def test_block_delimiters_namespace_and_ambiguous_heading(self):
        for mutate in (
            lambda p,c: p["units"][2]["literals"].update(scale="# forged"),
            lambda p,c: p["units"][3]["images"][0].update(alt="a\u2028b"),
            lambda p,c: p["units"][4].update(latex="a\u2029b"),
            lambda p,c: p["units"][4].update(anchor="fig-99"),
        ):
            plan, content = fixture()
            mutate(plan, content)
            self.assertEqual(render_note(plan, content)["status"], "blocked")
        units = [unit("a", "heading", level=2, numbered=False), unit("b", "heading", level=2, numbered=False),
                 unit("c", "paragraph", links={"s": {"target":"b", "label":"here"}})]
        result = render_note(dict(schema_version=1, units=units),
                             {"a":{"text":"标题"}, "b":{"text":"标题"}, "c":{"text":"{{link:s}}"}})
        self.assertEqual("blocked", result["status"])
        self.assertIn("multiple headings", result["findings"][0]["message"])
