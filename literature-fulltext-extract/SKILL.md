---
name: literature-fulltext-extract
version: v2.0.4
description: 将已确定的文献全文提取为原语言 Markdown，支持 PDF 按页 OCR 及 DOCX、HTML、EPUB、文本等格式，修复版面断行并保留来源定位。不负责翻译或写入 Obsidian。
---

# 提取全文与段落

机械操作统一通过 [Rust 运行入口](../literature-zotero2obsidian/references/rust-entrypoints.md) 的 `z2o describe` / `z2o invoke` 执行。未通过实际验收的能力保持阻断或待核。

## 必要输入

- item.json 已确认的主全文及附件哈希。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。
- 如需了解 PDF 质量预检、内置 PP-OCRv6 与零乱码视觉核对，读取[PDF 质量预检、内置 PP-OCRv6 与零乱码视觉核对指南](../literature-zotero2obsidian/references/ocr-and-minicpm.md)。

## 执行步骤

继续以现有逐页/结构清单与来源交付检查为机械证据；实际文件绑定使用 [确定性阶段辅助](../literature-zotero2obsidian/references/atomic-stage-checks.md)。页图内容是否正确与源单元是否完整仍须核对。

1. 核对每份附件的来源身份。
2. 优先调用复合入口 `prepare_source_inventory.prepare_paper`（传 `work_dir`、`pdf_path`、`tex_target`、`glossary_path`），一站完成 arXiv TeX 获取（自动转码非 UTF-8 源码）、`pdf-inspector` + `lopdf` 字体 CMap 质量预检、选择性 PDF 版面聚类、行末连字符愈合、内置 PP-OCRv6 乱码自动修复、生成 `clean-layout.md`、`ocr-verify-sheet.png`（如有 `<!-- VISUAL_REQUIRED: ... -->` 区域）、`figure_candidates` 与本篇 `matched-glossary.md`（若独立运行 PDF 阶段可调用 `prepare_pdf_layout`，默认 `render_pages: "selective"`；TeX 已成功时严禁全量渲染 `render_pages: "all"`）。
3. 先在 `source-map.json`（或 `layout-blocks.json` / `tex-structure.json`）登记原文章节及对象清单；若 `clean-layout.md` 中存在 `<!-- VISUAL_REQUIRED: ... -->` 标记，必须对照 `ocr-verify-sheet.png` 或对应页图逐处核实并替换为核定原文，禁止残留该标记进入发布校验。
4. 对照 `figure_candidates` 调用 `pdf_image_objects.render_page_regions_batch` 批量裁剪多个图件（支持 `auto_trim` 与 `contact_sheet_destination` 拼图）；仅在异常区域或边界有疑问时再单独调用 `render_page_region` / `inspect_crop_boundaries` 微调。
5. 对照页图或 TeX 源码复核需要精确转录的字段与公式。
6. 回查清单中尚无提取结果的对象。

## DOCX 与多格式结构入口

正文或 SI 为 `.docx`、`.html`/`.xhtml`、`.epub`、`.odt`、`.txt`/`.md` 时，将原文件直接传给 `prepare_source_inventory.prepare_sources()` 或总入口 `initialize_workflow()`；同一调用可以混合 PDF 与上述格式，非 UTF-8 文本自动通过 `encoding_rs`/`chardetng` 无乱码解码。DOCX 读取返回清单的 `native_blocks`、`media_calls` 和 `native_media`，具体字段见[DOCX 结构证据](../literature-zotero2obsidian/references/delivery-checks.md#docx-结构证据)。结构定位使用 XML 部件及子元素路径，没有推定页码。

先按 `native_blocks` 的顺序提取正文与表格。再按 `media_calls` 的实际 relationship 找原图，并结合前后题注确认逻辑图号。同一图可以对应多个媒体调用；ZIP 中的图片文件名和媒体序号不代表图号。原图文件绑定位于 `native_media[].file`。

`native_issues` 列出 OMML 公式、浮动图形、裁剪变换、其他部件等尚需核对的对象。保留这些原始条目，在 `native_issue_reviews` 另记实际处置及证据。需要看排版时实际渲染 DOCX，再记录该次渲染的文件和定位；不能将段落序号写成 PDF 页码，也不能用生成清单代替内容核验。

## 预印本 TeX 源辅助

条目为 `preprint` 或出现 arXiv ID / `arxiv.org` 链接时默认启用 TeX 辅助，不需要再次询问。直接调用 `prepare_source_inventory.fetch_arxiv_tex`（自动探测 ID、下载 e-print、展开 `.bbl` 与 `.gz`/`.tar` 并写出 `tex-structure.json` 与 `clean-source.md`）；若工作区或附件已有 `.tex`、`.zip`、`.tar`、`.tar.gz`、`.gz`，也可调用 `prepare_source_inventory.inspect_tex_source` 或 `prepare_source_inventory.prepare_tex_source`，读取 `blocks`、`equations`、`tables`、`citations`、`graphics` 与 `clean-source.md`。

TeX 用于核对乱码文字层难以辨认的公式、表格和书目；PDF 页图仍是页面顺序、可见版面和图件边界的依据。两者冲突时同时保留并登记裁决，不用 TeX 静默覆盖 PDF。下载或解析失败时记录原因并回退 `prepare_pdf_layout` 继续 PDF 路径。

## 透明背景图

遵循档案 `transparent_figure: preserve_original` 偏好：透明背景插图默认保留原图透明通道（alpha），禁止主动合成纯白底（`#FFFFFF`）。当 `figure_candidates[].single_object_reusable` 为 `true` 时可调用 `export_image_object`（直接保留原编码与 SMask alpha）；其余需按页裁剪或存在矢量文字/坐标轴叠加的图件调用 `pdf_image_objects.render_page_regions_batch` / `render_page_region` / `render_pdfcraft_region`（`transparent` 默认 `true`，不要设 `transparent: false`）。若源图本身有实色白底则保持原样；仅在需要记录像素诊断对照时临时使用合成视图，不覆盖最终交付 PNG。记录 `transparent_background`、`alpha_min`、`alpha_max` 与最终文件哈希。

## 约束

每条约束按其 ID 单独记录执行证据。

- [EXT-C01] 逐页检查 PDF 文本层是否存在乱码。
- [EXT-C02] 逐页检查 PDF 文本层是否存在缺字。
- [EXT-C03] 逐页检查 PDF 文本层是否存在意外空白。
- [EXT-C04] 逐页检查 PDF 文本层是否存在串栏。
- [EXT-C05] PDF 仅对存在文本层问题的页面或区域执行 OCR。
- [EXT-C06] OCR 使用所处理文本的对应语言。
- [EXT-C07] DOCX/ODT 提取保留原段落边界。
- [EXT-C08] DOCX/ODT 提取保留图片与正文的对应关系。
- [EXT-C71] DOCX/ODT 提取保留表格与正文的对应关系。
- [EXT-C09] DOCX/ODT 提取保留公式与正文的对应关系。
- [EXT-C10] HTML/EPUB 按文档阅读顺序解析。
- [EXT-C11] TXT/MD 核对文本编码。
- [EXT-C12] 图像序列按已核实的页序执行 OCR。
- [EXT-C13] 旧 DOC/RTF 在提取前验证转换结果。
- [EXT-C14] 依据文字坐标确定分栏的阅读顺序。
- [EXT-C15] 依据文字坐标识别侧栏内容。
- [EXT-C16] 依据文字坐标识别跨栏题注。
- [EXT-C17] 依据文字方向识别旋转文字。
- [EXT-C18] 提取裁框不得截掉正文字符。
- [EXT-C19] 保留侧栏要点。
- [EXT-C20] 保留作者贡献。
- [EXT-C21] 保留致谢。
- [EXT-C22] 仅在确认是重复版面元素后移除页眉。
- [EXT-C23] 仅在确认是重复版面元素后移除页脚。
- [EXT-C24] 仅在确认是重复版面元素后移除页码。
- [EXT-C25] 将人工页码保存在来源映射中。
- [EXT-C26] 只合并同一段落内的版面断行。
- [EXT-C27] 跨页合并前核对上下页的语义衔接。
- [EXT-C28] 保留英文词间空格。
- [EXT-C29] 删除中文文本中的无意义空格。
- [EXT-C30] 将版面断词与原词中的连字符区别处理。
- [EXT-C31] 保留复合词自身的连字符。
- [EXT-C32] 将负号按数学符号处理。
- [EXT-C33] 将范围连接线按范围语义处理。
- [EXT-C34] 段落之间保留空行。
- [EXT-C35] 列表保持为独立结构。
- [EXT-C36] 表格保持为独立结构。
- [EXT-C37] 数学内容保持为独立结构。
- [EXT-C38] 代码保持为独立结构。
- [EXT-C39] 依据悬挂缩进重建书目条目边界。
- [EXT-C40] 逐条保留书目原文。
- [EXT-C41] 跨块分式对照源页重建。
- [EXT-C42] 跨块上标对照源页重建。
- [EXT-C72] 跨块下标对照源页重建。
- [EXT-C43] 逐处核对低质量 OCR 中的数值。
- [EXT-C44] 逐处核对低质量 OCR 中的单位。
- [EXT-C45] 逐处核对低质量 OCR 中的公式。
- [EXT-C46] 为 OCR 不确定处保留候选文本。
- [EXT-C47] 为 OCR 不确定处记录来源定位。
- [EXT-C48] PDF 图像按[裁剪与验收规则](references/figure-crop.md)逐图提取。
- [EXT-C49] 保留图框内的完整图件。
- [EXT-C50] 将图像题注单独保存。
- [EXT-C51] 能完整保留显示内容时优先保留图像的原编码。
- [EXT-C52] 图像扩展名须与实际编码一致。
- [EXT-C53] 图像二进制保存为独立资源文件。
- [EXT-C54] 在 JSON 图像记录中保存路径。
- [EXT-C68] 在 JSON 图像记录中保存哈希。
- [EXT-C69] JSON 图像记录不内嵌二进制。
- [EXT-C70] JSON 中的图像文件引用对象仅含 `path` 与 `sha256`。
- [EXT-C55] 保留表格的行结构。
- [EXT-C73] 保留表格的列结构。
- [EXT-C56] 保留表格单元格合并关系。
- [EXT-C57] 保留表格单位。
- [EXT-C58] 保留表格注释。
- [EXT-C59] 复杂表格另存结构化数据。
- [EXT-C60] 复杂表格另存核对图。
- [EXT-C61] 保留公式原编号。
- [EXT-C62] 对照源文核实公式 LaTeX。
- [EXT-C63] 按附件角色区分主全文与 SI。
- [EXT-C64] 将 SI 附件交第 11 步处理。
- [EXT-C65] 保留原始附件。
- [EXT-C66] 保留原始解析结果。
- [EXT-C67] 后续步骤使用候选稿。
- [EXT-C74] 每份 PDF 的题头页须生成可读页图。
- [EXT-C75] 每个公式所在的 PDF 页面须生成可读页图。
- [EXT-C76] 每个表格所在的 PDF 页面须生成可读页图。
- [EXT-C77] 页图内容清晰时，以可见字形裁决其与文字层的差异。
- [EXT-C78] 在来源映射中登记文字层与页图不一致的位置。
- [EXT-C79] 文字层出现日期占位符时，查看对应页图后再判定日期是否缺失。
- [EXT-C80] 未实际查看页图时，不得把文字层结果表述为“原页可见”。
- [EXT-C81] 将图中具有明确行列对应关系的数据表登记为独立 table 对象。
- [EXT-C82] 将无编号显示公式纳入公式清单。
- [EXT-C83] 提取结果保留每个可辨认的无编号显示公式。
- [EXT-C84] 逐单元格核对表格中的类别代号。
- [EXT-C85] 逐单元格核对表格中的数值。
- [EXT-C86] 逐公式核对分母的实际作用范围。
- [EXT-C87] 逐公式核对根号的实际作用范围。
- [EXT-C88] 逐公式核对指数的实际作用范围。
- [EXT-C89] 每份来源单独登记作者到单位编号的映射。

图像文件引用与来源定位分开表示。例如 `{"asset":{"path":"assets/figure.png","sha256":"实际哈希"},"source":{"id":"main","page":2,"bbox":[40,80,300,400]}}` 中，`asset` 是最小文件引用对象，`source` 是定位记录。来源页渲染记录的 DPI、像素哈希和宽高属于诊断证据，不属于交接稿中的图像文件引用对象。不得因最小文件引用规则删除裁剪或对源证据。

PDF 图像先使用 `prepare_sources(...)` 生成的 `image_index` 选择实际原图对象，再由 `export_image_object(...)` 导出；出现显示效果限制时使用 `render_page_region(...)`。完整调用及逐边核对见[原图对象索引与导出](references/figure-crop.md#原图对象索引与导出)。原图对象不自动等同于整幅 figure，最终图片仍须与源页文件核对。

### 来源映射记录示例

文字层与页图冲突直接写入对应来源单元的 `text_visual_conflicts`；报告可引用该位置，但不能替代映射记录。下例仅为虚构字段示例，路径、哈希与定位须换成本次实际值。`basis: visual` 只说明证据类型，本身没有记录冲突的两个读法。

```json
{
  "id": "source-a-affiliation-b",
  "source": "source-a",
  "kind": "metadata",
  "field": "affiliation",
  "value": "Reid Laboratory",
  "text_visual_conflicts": [
    {
      "id": "conflict-lab-name",
      "text_layer": {
        "file": {"path": "work/page-text.txt", "sha256": "实际文本文件SHA256"},
        "locator": {"kind": "lines", "start": 9, "end": 9},
        "text": "Reed Laboratory"
      },
      "visible_page": {
        "file": {"path": "work/page-render.png", "sha256": "实际页图文件SHA256"},
        "page": 2,
        "bbox": [60, 90, 280, 112],
        "text": "Reid Laboratory"
      },
      "decision": {
        "status": "resolved",
        "selected_text": "Reid Laboratory",
        "reason": "实际查看所定位的页图后，该字母可辨为i。"
      }
    }
  ],
  "evidence": [{"page": 2, "basis": "visual", "bbox": [60, 90, 280, 112]}]
}
```

尚未查看页图时，冲突决定保持 `pending`；实际字形仍不可辨时保留候选读法与未决原因。不要为了填满示例伪造可见文字或查看记录。

**连字符与排印差异的对源处理。** 文字层出现软连字符 U+00AD、可见连字符或相邻排印符号时，先定位原始文字片段及对应页图区域，再辨明是版面断词、原词连字符、数学负号还是范围连接线。保留原始解析文件；以可见页为依据确定转写，并把原文字层、页图读法和裁决原因登记到该来源单元。原刊自身可辨的拼写或排印疑点保留原样；字形不清时保留未决，不靠词典、另一篇书目或全局替换补正。

软连字符与可见连字符同处一个片段时，把整个待核片段一次替换为已核定的读法。不能先把软连字符转换成 `-`，再叠加一个可见连字符。原页有两个可见字符也不能机械去重。涉及跨行断词时，证据区域覆盖词的两部分，裁决原因写明为何保留或去掉该处连字符；不将这一处裁决扩散到全文同形片段。

Rust 操作 `format_helpers.apply_visible_text_edits` 只应用调用方已经指定的修订。每条 `resolved_edits` 记录包含 `id`、原始文本的 Unicode 码点区间 `start` / `end`（右端不含）、`expected_text`、`visible_page` 和 `decision`。`visible_page` 保存实际页图的最小文件引用、页码、`bbox`、坐标系及核定转写 `text`；`decision` 保存 `status="resolved"`、相同的 `selected_text`、原因、查看人和该查看人提供的 `observed` 声明。调用方负责真实查看行为与读法正确性，函数不认证这两项。

将实际 `raw_text_file_record` 与 `resolved_edits` 分别作为 `kwargs.text_file` 和 `kwargs.edits`，调用 `format_helpers.apply_visible_text_edits`。将返回 `result.text_visual_conflicts` 追加到实际来源单元的 `text_visual_conflicts`，用 `result.text` 作为候选文字；原始文字文件不变。

函数核对证据文件哈希、原片段及区间不重叠，再基于同一原始快照一次组装结果；它不写回原始文件，不按词典生成修订，也不自动删除软连字符。缺少页图证据、待决读法或未声明实际查看时不应用修改，保留来源映射中的待核记录。以上帮助复用 EXT-C30–C33、EXT-C65–C66、EXT-C77–C80 与 EXT-A22 的逐项验收。

作者单元使用 `affiliation_numbers` 保存可见单位编号，用 `affiliation_units` 保存对应的单位单元 ID。两个数组按同一顺序对应；各单位单元属于同一实际来源。整行作者框与作者姓名不能代替这一关系。下例中的人名和单位均为虚构：

```json
[
  {
    "id": "source-a-author-mira",
    "source": "source-a",
    "kind": "metadata",
    "field": "author",
    "value": "Mira Vale",
    "affiliation_numbers": ["B", "D"],
    "affiliation_units": ["source-a-affiliation-b", "source-a-affiliation-d"],
    "evidence": [{"page": 2, "basis": "visual", "quote": "Mira Valeᴮ,ᴰ"}]
  },
  {
    "id": "source-a-affiliation-b",
    "source": "source-a",
    "kind": "metadata",
    "field": "affiliation",
    "value": "Reid Laboratory",
    "evidence": [{"page": 2, "basis": "visual", "quote": "B Reid Laboratory"}]
  },
  {
    "id": "source-a-affiliation-d",
    "source": "source-a",
    "kind": "metadata",
    "field": "affiliation",
    "value": "Aster Institute",
    "evidence": [{"page": 2, "basis": "visual", "quote": "D Aster Institute"}]
  }
]
```

来源没有编号但关系可由源页明确判定时，对应编号用 `null`，在 `affiliation_reason` 写出来源依据。无法判定作者所属单位时，两数组留空，填写 `affiliation_reason`，保留源页定位；不从姓名、作者排序或另一份来源推断。主文与每份 SI 各自建立关系。

## 产物

- source.raw.jsonl、source.md、source-map.json、assets/。

## 验收标准

每项验收按其 ID 单独记录判定结果。

- [ ] [EXT-A01] 每页或章节均有覆盖记录。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A02] 每项排除均有来源定位。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A03] 每项排除均有原因。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A04] 每项提取失败均有来源定位。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A05] 每项提取失败均有原因。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A06] 每处漏页均已处理或列为未决。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A07] 每处重复段均已处理或列为未决。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A08] 每处串栏均已处理或列为未决。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A09] 每个图可回溯至源文。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A10] 每个表可回溯至源文。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A11] 每个公式可回溯至源文。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A12] 每个脚注可回溯至源文。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A13] 每个书目条目可回溯至源文。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A14] 每幅 PDF 图像已通过[裁剪视觉验收](references/figure-crop.md)。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A15] 每幅 PDF 图像均有源页记录。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A16] 每幅 PDF 图像均有裁剪范围记录。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A17] 每幅 PDF 图像均有裁剪验收证据。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A18] 正文语言与原判断冲突时已反馈第 1 步。 <!-- check-type: execution_process -->
- [ ] [EXT-A19] 关键内容不可辨时已保留工作稿。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A20] 关键内容不可辨时已标记阻断。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A21] 每份 PDF 的题头页均有实际查看页图的记录。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A22] 每处文字层与页图的冲突均有裁决依据。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A23] 每个图内数据表均有独立对象记录。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A24] 每个可辨认的显示公式均有提取结果。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A25] 每个表格类别代号与页图一致。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A26] 每个表格数值与页图一致。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A27] 每个公式的分母范围与页图一致。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A28] 每个公式的根号范围与页图一致。 <!-- check-type: offline_outcome -->
- [ ] [EXT-A29] 每个公式的指数范围与页图一致。 <!-- check-type: offline_outcome -->
