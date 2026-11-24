"""Figure groups and translator-note regressions with invented source labels."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT.joinpath("literature-zotero2obsidian", "scripts")))
from format_helpers import build_figure_block, plan_supplement_figure_caption
from lint_presentation import lint_presentation
from validate_delivery import scan_markdown_links


def relevant(text):
    return [item for item in lint_presentation(text)
            if item.code.startswith("FIGURE_") or item.code == "TRANSLATOR_NOTE_INLINE"]


class FigureBlockTests(unittest.TestCase):
    def test_unnumbered_source_label_keeps_caption_and_anchor(self):
        images = [{"target": "figures/example map.png", "alt": "示意[图]"}]
        before = deepcopy(images)
        block = build_figure_block(images, caption="**概念示意**", anchor="fig-concept")
        self.assertEqual(images, before)
        self.assertEqual(block.splitlines(), ["![示意\\[图\\]](<figures/example map.png>)  ", "**概念示意**", "^fig-concept"])
        self.assertEqual(relevant(block), [])

    def test_multiple_assets_form_one_logical_figure(self):
        block = build_figure_block([
            {"target": "left.png", "alt": "图S8"}, {"target": "right.png", "alt": ""},
        ], caption="**图S8：** 两个来源面板。", anchor="^fig-s8", newline="\r\n")
        self.assertEqual(block.count("\r\n"), 4)
        self.assertEqual(relevant(block), [])

    def test_existing_supplement_caption_with_literal_star_is_accepted(self):
        caption = plan_supplement_figure_caption("7*", "来源题注。")['caption']
        block = build_figure_block([{"target": "figure.png", "alt": "图S7*"}],
                                   caption=caption, anchor="fig-s7")
        self.assertIn("**图S7\\*：**", block)
        self.assertEqual(relevant(block), [])

    def test_literal_resource_names_survive_markdown_and_url_decoding(self):
        for target in ("assets/A&amp;B.png", "assets/A&B.png", "assets/100%20#1?.png",
                       r"assets/literal\name.png", "assets/中文 (two).png"):
            with self.subTest(target=target):
                block = build_figure_block([{"target": target, "alt": "图 &amp; 名"}],
                                           caption="**图9**", anchor="fig-9")
                images = [link for link in scan_markdown_links(block) if link.image]
                self.assertEqual(len(images), 1)
                self.assertEqual(unquote(images[0].href), target)
                self.assertEqual(relevant(block), [])

    def test_builder_requires_explicit_content(self):
        image = [{"target": "figure.png", "alt": ""}]
        for options in ({"images": [], "caption": "**图**", "anchor": "fig-a"},
                        {"images": image, "caption": "", "anchor": "fig-a"},
                        {"images": image, "caption": "**图**", "anchor": ""},
                        {"images": [{"target": "a>bad.png", "alt": ""}], "caption": "**图**", "anchor": "fig-a"},
                        {"images": [{"target": "a.png", "alt": "a\nb"}], "caption": "**图**", "anchor": "fig-a"}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                build_figure_block(**options)

    def test_heading_above_image_does_not_replace_caption_or_anchor(self):
        text = "## 概念示意\n![图](<example.png>)  \n\n## 摘要\n正文。\n"
        findings = relevant(text)
        self.assertEqual({item.code for item in findings}, {"FIGURE_CAPTION_MISSING", "FIGURE_ANCHOR_MISSING"})
        self.assertTrue(all(item.line == 2 for item in findings))

    def test_caption_and_anchor_have_independent_gap_goals(self):
        findings = relevant("![[example.png]]  \n\n**图9**\n\n^fig-9\n")
        self.assertEqual({item.code for item in findings}, {"FIGURE_CAPTION_GAP", "FIGURE_ANCHOR_GAP"})
        self.assertEqual({item.code for item in relevant("![[example.png]]  \n**图9**\n")}, {"FIGURE_ANCHOR_MISSING"})

    def test_code_comments_and_nonimage_wiki_embeds_are_excluded(self):
        text = ("```markdown\n![[example.png]]\n正文〔译注：示例〕\n```\n"
                "<!-- ![[hidden.png]] 译注：隐藏 -->\n"
                "    ![[indented.png]]\n"
                "![[source.pdf]]\n![[source.mp4]]\n![[Note#Section]]\n")
        self.assertEqual(relevant(text), [])

    def test_explicit_inline_translator_note_requires_separate_quote(self):
        text = "来源论断〔译注：此处有源内差异〕。\n\n> 译注：此处有源内差异。\n"
        findings = relevant(text)
        self.assertEqual([(item.code, item.line) for item in findings], [("TRANSLATOR_NOTE_INLINE", 1)])


if __name__ == "__main__":
    unittest.main()
