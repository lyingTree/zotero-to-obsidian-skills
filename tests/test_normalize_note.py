from hashlib import sha256
import importlib.util
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest


SCRIPT = Path(__file__).resolve().parents[1].joinpath(
    "literature-zotero2obsidian", "scripts", "normalize_note.py")
spec = importlib.util.spec_from_file_location("normalizer_test_subject",SCRIPT)
normalizer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(normalizer)


class NoteNormalization(unittest.TestCase):
    def check(self, source):
        r = normalizer.normalize_note(source)
        replay = source
        for change in r["changes"]:
            a,b = change["start"],change["end"]
            self.assertEqual(replay[a:b],change["before"])
            prefix = replay[:a].replace("\r\n","\n").replace("\r","\n")
            self.assertEqual((change["line"],change["column"]),
                             (prefix.count("\n")+1,len(prefix.split("\n")[-1])+1))
            replay = replay[:a]+change["after"]+replay[b:]
        self.assertEqual(replay,r["text"])
        self.assertEqual(r,normalizer.normalize_note(source))
        self.assertFalse(normalizer.normalize_note(replay)["changes"])
        self.assertTrue(r["idempotent"])
        self.assertEqual(r["input_sha256"],sha256(source.encode()).hexdigest())
        self.assertEqual(r["output_sha256"],sha256(replay.encode()).hexdigest())
        self.assertEqual(r["source_fidelity"],"not_assessed")
        return r

    def test_signed_precision_and_percent_boundaries(self):
        source = r"持续60 min，速度+81.860节，变化−0.0040%，占31.$2\%$。另有$12$分钟。"
        self.assertEqual(self.check(source)["text"],
            r"持续$60\,\mathrm{min}$，速度$+81.860\,\text{节}$，变化$−0.0040\%$，占$31.2\%$。另有$12\,\text{分钟}$。")

    def test_nonquantities_compounds_and_ambiguous_uses(self):
        source = "2024年；12:45 UTC；UTC 1200 h；第3节；图2秒；A3m；1,250 kg；3±2 m；5–7 km；60 km/h；1.3e-4 m。"
        self.assertEqual(self.check(source)["text"],source)

    def test_preserves_opaque_text_and_link_targets(self):
        literals = [
            "---\r\ntitle: 中 A—B 10 min\r\n---",
            "`a 60 min` 和 ``含内部 ` 81节 — 中 A``",
            "````python\n```\n5 min\n````",
            r"$60\,\mathrm{min}$，\(25 min\) 和 \[20 min\]",
            "https://host/中—A/60min DOI 10.1022/ab—6min",
            "![caption—x](<assets/4bd7/plot—01.png>)",
            '[x](<folder/part) 90 min—b> "caption) 40 min—x")',
            "graph—3.png",
            "[id]: <目标/60 min—ab>\n  \"测试 中 A\"",
            "> ```\n> 30 min\n> ```",
            "    30 min and A—B",
        ]
        for source in literals:
            with self.subTest(source=source):
                self.assertEqual(self.check(source)["text"],source)
        self.assertEqual(self.check("[持续60 min](https://x/a(b)—c) [[图—1|值25%]]")["text"],
            r"[持续$60\,\mathrm{min}$](https://x/a(b)—c) [[图—1|值$25\%$]]")

    def test_malformed_tail_and_rich_tables(self):
        for opening in ("---\n", "``open ", "$$", "[a](missing(", "<!--", "```rich-table\n"):
            tail = opening+"中 A — 30 min\n\n## 3 段\n\n尾"
            r = self.check(tail)
            self.assertEqual(r["text"],tail)
            self.assertTrue(r["unresolved"])
        source = "```rich-table\ngrid: 3 min\n\n\n\n```\n\n^tbl-3\n正文"
        r = self.check(source)
        self.assertEqual(r["text"],source.replace("```\n\n^","```\n^").replace("^tbl-3\n","^tbl-3\n\n"))
        self.assertIn("RICH_TABLE_PROTECTED",{u["code"] for u in r["unresolved"]})

    def test_structural_spacing_keeps_original_line_endings(self):
        source = ("# Original title\n\n中文标题\n作者：甲\n## 2 实验\n\n正文\n"
                  "![源](path/abc123.png)\n\n**图1：** 描述\n\n^fig-1\n"
                  "$$\nx=1\n$$\n\n^eq-1\n下一段").replace("\n","\r\n")
        expected = ("# Original title\n中文标题\n\n作者：甲\n\n## 2 实验\n正文\n\n"
                    "![源](path/abc123.png)  \n**图1：** 描述\n^fig-1\n\n"
                    "$$\nx=1\n$$\n^eq-1\n\n下一段").replace("\n","\r\n")
        self.assertEqual(self.check(source)["text"],expected)

    def test_preserve_resolved_heading_identity_and_verbatim_context(self):
        source = "## 1 海况 8 m\n正文。\n\n[海况](<#1 海况 8 m>)"
        result = self.check(source)["text"]
        target = normalizer.presentation.delivery_checks.scan_markdown_links(result)[0].href[1:]
        self.assertEqual([line[3:] for line in result.splitlines() if line.startswith("## ")], [target])
        for source in ("<code>speed = 5 m</code>", "<PrE lang='txt'>\n5 m\n</pre>",
                       "> 原文摘录\n其长度为 5 m。", r"图3.$15\%$ 是标记。", "测得10 m s⁻¹。",
                       "测得3 m × 4 m。", "学生在3年级读完课程。"):
            with self.subTest(source=source):
                self.assertEqual(self.check(source)["text"],source)
        self.assertEqual(self.check("![图](x.png)\n**图1：** 附图\n^fig-1")["text"],
                         "![图](x.png)  \n**图1：** 附图\n^fig-1")

    def test_exclusive_destination(self):
        with TemporaryDirectory() as folder:
            src,dst = (Path(folder).joinpath(n) for n in ("input.md","output.md"))
            src.write_bytes(b"60 min\r\n")
            normalizer.normalize_file(src,dst)
            with self.assertRaises(FileExistsError):
                normalizer.normalize_file(src,dst)
            with self.assertRaises(ValueError):
                normalizer.normalize_file(src,src)
            self.assertEqual(src.read_bytes(),b"60 min\r\n")
            self.assertTrue(dst.read_bytes().endswith(b"\r\n"))
