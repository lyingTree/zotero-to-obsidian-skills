---
name: literature-supplement-materials
version: v2.0.3
description: 提取并翻译 Zotero 条目的补充材料（SI 文本、图、表、视频），合并为正文后的补充材料章节；在第 8 步排版与第 9 步链接之前执行。
---

# 处理补充材料

机械操作统一通过 [Rust 运行入口](../literature-zotero2obsidian/references/rust-entrypoints.md) 的 `z2o describe` / `z2o invoke` 执行。未通过实际验收的能力保持阻断或待核。

## 必要输入

- item.json 中的 supplement 附件、第 3 步选定主稿及术语记录。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

对明确列出的本地资源运行 [确定性阶段辅助](../literature-zotero2obsidian/references/atomic-stage-checks.md) 的 inspect_relative_files，实际读取文件并检查根边界及哈希；仍须按来源库存核对完整性、实际视频容器、编解码和首帧。

1. 复用第 1 步清单核对每份 SI 附件的身份。
2. 查看每份 SI 自己的题头；PDF 使用题头页图，DOCX 使用结构段落并按实际版面需要渲染。
3. 按[来源命名空间](../literature-zotero2obsidian/references/pipeline-contract.md#补充材料与书目命名空间)建立该份 SI 的来源记录。
4. 按[题头规则](../literature-header-format/SKILL.md)构建该份 SI 的题头。
5. 提取该份 SI 的完整内容。
6. 对照清单检查该份 SI 的遗漏对象。
7. 将核对后的 SI 内容并入候选一次。

DOCX 补充材料直接进入[DOCX 结构证据入口](../literature-zotero2obsidian/references/delivery-checks.md#docx-结构证据)。先核对每个 `media_call` 的关系目标和前后原文，再建立图号与题注对应。一个题注前有两个媒体调用时，先核对是否为同一图的两个面板；不要据此补造第二个图号。最终检查会提示未映射的实际媒体调用，目录中存在图片不能替代笔记中的正确嵌入。

## 约束

每条约束按其 ID 单独记录执行证据。

- [SI-C01] SI 来源独立于主全文登记。
- [SI-C02] SI 编号独立于主全文登记。
- [SI-C03] 在来源映射中保留各份 SI 的可见原号，包括重复的 S1、参考条目1或公式号。
- [SI-C04] SI 内容块以来源命名空间消歧。
- [SI-C05] SI 资源以来源命名空间消歧。
- [SI-C06] SI 链接以来源命名空间消歧。
- [SI-C07] 不得按当前附件顺序重排 SI 原号。
- [SI-C08] 继续使用已核实的 SI 映射。
- [SI-C09] 不可读附件按实际可用性登记 remote_only、missing 或其他状态。
- [SI-C10] PDF 优先使用文本层。
- [SI-C11] PDF 扫描页按需 OCR。
- [SI-C12] SI 提取沿用[全文提取规则](../literature-fulltext-extract/SKILL.md)。
- [SI-C13] 保留 Text 的原 S 编号。
- [SI-C14] 保留 Figure 的原 S 编号。
- [SI-C15] 保留 Table 的原 S 编号。
- [SI-C16] 保留 Movie 的原 S 编号。
- [SI-C17] 保留 SI 对象的原顺序。
- [SI-C18] 来源原编号字段不得补入原文没有的对象编号。
- [SI-C19] PDF 补充图执行[PDF 图像裁剪与验收规则](../literature-fulltext-extract/references/figure-crop.md)。
- [SI-C20] PDF 一页仅一幅补充图时仍须定位图框。
- [SI-C21] 保留 SI 表格的行结构。
- [SI-C98] 保留 SI 表格的列结构。
- [SI-C22] 保留 SI 表格合并关系。
- [SI-C23] 保留 SI 表格单位。
- [SI-C24] 保留 SI 表格表注。
- [SI-C25] 保留 SI 公式 LaTeX。
- [SI-C26] 保留 SI 公式原号。
- [SI-C27] 逐个探测视频实际容器。
- [SI-C28] 逐个探测视频编码。
- [SI-C29] 逐个探测视频时长。
- [SI-C30] 逐个探测视频分辨率。
- [SI-C31] 逐个探测视频音轨。
- [SI-C32] Obsidian 视频嵌入仅使用 `.mkv`、`.mov`、`.mp4`、`.ogv` 或 `.webm`。
- [SI-C33] 其他格式或文件解码失败的视频自动转为 H.264 MP4。
- [SI-C34] 转码时保留视频原件。
- [SI-C35] 转码时保留已有音轨。
- [SI-C36] 记录视频原件哈希。
- [SI-C37] 记录视频转码件哈希。
- [SI-C38] SI 资源先保存在工作目录。
- [SI-C39] SI 图片、视频及文档按项目配置归入所属文献的 `attachments/<年份>/<第一作者>/<资源桶>/`。
- [SI-C40] 沿用已核实的 title 哈希资源目录。
- [SI-C41] 资源目录按已核实身份或版本执行必要消歧。
- [SI-C42] 资源哈希不同时不得覆盖旧文件。
- [SI-C43] SI 资源引用使用从库根起算的完整路径。
- [SI-C44] SI 资源由第 4 步提交。
- [SI-C45] 按 SI 实际语种复用[翻译规则](../literature-translate-zh/SKILL.md)。
- [SI-C46] SI 翻译复用主稿术语。
- [SI-C47] 补充图题注直译。
- [SI-C48] 补充表题注直译。
- [SI-C49] 补充视频题注直译。
- [SI-C50] 不改绘视频画面。
- [SI-C51] 新建或修复时，SI 章名统一为「补充材料」。
- [SI-C52] 补充材料整章位于主文全部内容之后，包括附录、声明和主文参考文献。
- [SI-C53] 补充材料整章位于「Zotero原条目」之前。
- [SI-C54] 补充材料章号交第 8 步处理。
- [SI-C55] 补充书目保留在补充材料章内。
- [SI-C56] 补充书目标题使用 H3。
- [SI-C57] 补充书目节序号交第 8 步按同级次序确定，如 `### 11.3 参考文献`。
- [SI-C58] 沿用补充书目顺序。
- [SI-C59] 沿用已核实的补充书目锚点映射，包括 `^ref-s<token>-N` 等格式。
- [SI-C60] 仅新建时，单份 SI 书目锚点默认使用 `^sref-N`。
- [SI-C61] 仅新建时，多份 SI 书目锚点使用 `^sref-src<source_token>-N`。
- [SI-C62] 补充书目 ID 不得与主文或其他书目冲突。
- [SI-C63] 保留已有补充书目调用。
- [SI-C64] 旧 `^ref-sN` 记录 Citation Preview 兼容 warning。
- [SI-C65] 不自动重命名旧 `^ref-sN`。
- [SI-C66] 有来源 S 编号的 SI 对象子标题使用对应的“文本 S…”、“图 S…”、“表 S…”或“视频 S…”标题文字。
- [SI-C67] SI 子标题依原顺序排列。
- [SI-C68] 补充图块以图片起首。
- [SI-C69] 补充图题注位于图片下方。
- [SI-C70] 补充图锚位于题注后方。
- [SI-C96] 补充图锚独占一行。
- [SI-C71] 补充图块各相邻行之间不留空行。
- [SI-C72] 新建单份 SI 图锚默认使用 `^fig-sN`。
- [SI-C73] 多份 SI 图锚按来源 token 消歧。
- [SI-C74] 补充表沿用第 8 步 Rich Table 规则。
- [SI-C75] 补充表锚紧接闭合围栏的下一行。
- [SI-C76] 补充表锚顶格书写。
- [SI-C77] 补充表锚独占一行。
- [SI-C78] 新建单份 SI 表锚默认使用 `^tbl-sN`。
- [SI-C79] 多份 SI 表锚按来源 token 消歧。
- [SI-C80] 补充表锚后有内容时留一空行。
- [SI-C81] 补充视频就近排列题注。
- [SI-C82] 补充视频嵌入独占一行。
- [SI-C83] 补充视频使用 Obsidian 内置嵌入 `![[从库根起算的视频完整路径]]`。
- [SI-C84] 单份 SI 视频锚写在嵌入行尾。
- [SI-C97] 单份 SI 视频锚使用 `^mov-sN` 格式。
- [SI-C85] 多份 SI 视频锚按来源 token 消歧。
- [SI-C86] 沿用已有视频锚点。
- [SI-C87] 沿用已有视频调用。
- [SI-C88] 视频仅远程可用或转码失败时登记实际状态。
- [SI-C89] 视频仅远程可用或转码失败时记录原来源。
- [SI-C90] 正文 SI 调用的显示号遵循对应对象的显示号规则。
- [SI-C91] 正文 SI 调用交第 9 步绑定现有目标。
- [SI-C92] SI 缺页按项目配置判级。
- [SI-C93] SI 缺图按项目配置判级。
- [SI-C94] SI 缺表按项目配置判级。
- [SI-C95] SI 缺视频按项目配置判级。
- [SI-C99] 来源图号未带 S 前缀时，补充图显示号仅在原编号前加 S。
- [SI-C100] 补充图显示号与来源原号不同时，在题注中注明来源原号。
- [SI-C101] 来源未编号的 SI 文本小节使用原小节标题的译文。
- [SI-C102] SI 文本小节的导航序号采用本篇标题层级序号。
- [SI-C103] 每份 SI 的完整原题须在补充材料章节中保留。
- [SI-C104] 每份 SI 的作者行须依据该份 SI 单独构建。
- [SI-C105] 每份 SI 的单位行须依据该份 SI 单独构建。
- [SI-C106] 每份 SI 的作者单位映射须依据该份 SI 单独构建。
- [SI-C107] 不能因题名相近而将 SI 题头判定为主文题头的重复。
- [SI-C108] SI 题头不新增 H1。
- [SI-C109] SI 中可辨认的无编号显示公式须完整交付。
- [SI-C110] SI 中独立存在的书目须完整交付。
- [SI-C111] 每个补充图的显示号单独记录为 `display_number`。

### 原号、显示号与既有链接

在每个 SI 图或表的来源映射单元中分别保存 `original_number` 与 `display_number`。`original_number` 是源可见编号的原样字符串，保留字母、前缀和编号标点；它不是源对象 ID，也不是从资源名或锚点中拆出的数字。`display_number` 是本次 Markdown 使用的非空显示号。书目仍使用其独立列表的原号记录，不从图号推断。

补充图有原号但没有 S 前缀时，仅在原号前加 S；已有 S 前缀时逐字保留，不再追加 S。题注中按 SI-C100 保留原号说明。新建的正文补充图调用采用同一显示号；显示文字的调整不重命名已核实锚点，也不改变既有链接目标。

新建补充图题注时可调用 `format_helpers.plan_supplement_figure_caption(original_number, translated_caption)`。输入原号来自实际源页。函数返回独立的原号、显示号、显示号来源及 `caption`。显示号改变时，可见题注内带出原号说明。函数只返回题注计划。原号本来含 S 时保留原样。来源无编号时，原号明确传入 JSON `null`。无原号时，另传入已规划的显示号。无原号时，另提供实际无号理由。不从图件顺序猜原号。

以下为虚构接口示例；实际调用改用已读源页事实。

```json
{
  "schema_version": 1,
  "operation": "format_helpers.plan_supplement_figure_caption",
  "args": ["7c", "棱镜的响应。"],
  "kwargs": {}
}
```

返回 `result.caption` 为 `**图S7c：** 棱镜的响应。（来源原号：7c）`。

以下都是虚构例子。实际来源位置与文件引用沿用本次来源映射：

```json
[
  {
    "id": "supplement-a-figure-window",
    "source": "supplement-a",
    "kind": "figure",
    "original_number": "7c",
    "display_number": "S7c",
    "display_number_origin": "normalized_source",
    "anchor_id": "fig-existing-window",
    "evidence": [{"page": 4, "basis": "visual", "quote": "Figure 7c. Response of the prism."}]
  },
  {
    "id": "supplement-a-figure-lens",
    "source": "supplement-a",
    "kind": "figure",
    "original_number": "S8b",
    "display_number": "S8b",
    "display_number_origin": "source_literal",
    "anchor_id": "fig-existing-lens",
    "evidence": [{"page": 5, "basis": "visual", "quote": "Figure S8b. Layout of the lens."}]
  },
  {
    "id": "supplement-a-figure-unnumbered",
    "source": "supplement-a",
    "kind": "figure",
    "original_number": null,
    "original_number_reason": "此来源题注没有可见编号；所给页图位置只显示图题。",
    "display_number": "Saux-a",
    "display_number_origin": "generated_navigation",
    "anchor_id": "fig-existing-aux",
    "evidence": [{"page": 6, "basis": "visual", "quote": "Assembly of the shutter."}]
  }
]
```

第一条可显示为“图S7c（原图7c）”。第二条显示为“图S8b”。第三条的显示文字明确为“补充图 Saux-a（导航标识，原文未编号）”；`Saux-a` 只是新建导航示例，有已核实显示映射时继续使用该映射。无可见原号时使用 `null` 与 `original_number_reason`，不把生成导航号写回 `original_number`。没有原号的表同样不补造来源号。

## 产物

- si.md、si-map.json、assets/si/、drafts/11-supplement.md。

## 验收标准

每项验收按其 ID 单独记录判定结果。

- [ ] [SI-A01] 每一页均有覆盖决定。 <!-- check-type: offline_outcome -->
- [ ] [SI-A02] 每幅图均有覆盖决定。 <!-- check-type: offline_outcome -->
- [ ] [SI-A03] 每个表均有覆盖决定。 <!-- check-type: offline_outcome -->
- [ ] [SI-A04] 每个视频均有覆盖决定。 <!-- check-type: offline_outcome -->
- [ ] [SI-A05] 每个应译文本块均有对应译块或未决定位。 <!-- check-type: offline_outcome -->
- [ ] [SI-A06] 每项本地资源符合附件三级目录规则。 <!-- check-type: offline_outcome -->
- [ ] [SI-A07] 每项本地资源使用库根引用。 <!-- check-type: offline_outcome -->
- [ ] [SI-A08] 每项本地资源的最终路径已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A09] 每项本地资源的哈希已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A10] SI 资源引用能从候选资源根解析到可读文件。 <!-- check-type: offline_outcome -->
- [ ] [SI-A11] 每幅补充图已通过[裁剪视觉验收](../literature-fulltext-extract/references/figure-crop.md)。 <!-- check-type: offline_outcome -->
- [ ] [SI-A12] 每个视频使用内置嵌入。 <!-- check-type: offline_outcome -->
- [ ] [SI-A13] 每个视频的实际容器已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A14] 每个视频的编解码器已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A15] 每个视频的首帧解码已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A16] 每个视频所属 `^mov-s…` 落点已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A17] 视频转码件的视频编码为 H.264。 <!-- check-type: offline_outcome -->
- [ ] [SI-A45] 视频转码件的容器为 MP4。 <!-- check-type: offline_outcome -->
- [ ] [SI-A18] 视频转码件时长已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A19] 视频转码件分辨率已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A20] 视频转码件音轨已核对。 <!-- check-type: offline_outcome -->
- [ ] [SI-A21] 视频检查范围已记录。 <!-- check-type: offline_outcome -->
- [ ] [SI-A22] 视频未通过项已记录。 <!-- check-type: offline_outcome -->
- [ ] [SI-A23] SI 内容块 ID 全篇唯一。 <!-- check-type: offline_outcome -->
- [ ] [SI-A24] SI 图锚全篇唯一。 <!-- check-type: offline_outcome -->
- [ ] [SI-A25] SI 表锚全篇唯一。 <!-- check-type: offline_outcome -->
- [ ] [SI-A26] SI 公式锚全篇唯一。 <!-- check-type: offline_outcome -->
- [ ] [SI-A27] SI 视频锚全篇唯一。 <!-- check-type: offline_outcome -->
- [ ] [SI-A28] SI 书目锚全篇唯一。 <!-- check-type: offline_outcome -->
- [ ] [SI-A29] 每个 SI 对象的来源记录与实际对象一致。 <!-- check-type: offline_outcome -->
- [ ] [SI-A30] 同号不同 SI 的对象没有混配。 <!-- check-type: offline_outcome -->
- [ ] [SI-A31] 新增来源没有破坏已有调用。 <!-- check-type: offline_outcome -->
- [ ] [SI-A32] 新建书目锚点采用插件支持的前缀。 <!-- check-type: offline_outcome -->
- [ ] [SI-A33] 既有书目 ID 已保留。 <!-- check-type: offline_outcome -->
- [ ] [SI-A34] 既有书目调用已保留。 <!-- check-type: offline_outcome -->
- [ ] [SI-A35] 既有书目来源映射已保留。 <!-- check-type: offline_outcome -->
- [ ] [SI-A36] 插件不兼容的旧书目 ID 已登记警告。 <!-- check-type: offline_outcome -->
- [ ] [SI-A37] 数字后缀与当前序号不同的旧书目 ID 已登记警告。 <!-- check-type: offline_outcome -->
- [ ] [SI-A38] 旧书目 ID 未因新增、减少或重排 SI 而重命名。 <!-- check-type: offline_outcome -->
- [ ] [SI-A39] 主稿仅有一个「补充材料」章节。 <!-- check-type: offline_outcome -->
- [ ] [SI-A40] 补充材料整章位于主文全部内容之后。 <!-- check-type: offline_outcome -->
- [ ] [SI-A41] 补充材料整章位于「Zotero原条目」之前。 <!-- check-type: offline_outcome -->
- [ ] [SI-A42] 补充书目仍位于补充材料章内。 <!-- check-type: offline_outcome -->
- [ ] [SI-A43] 补充材料最终章号已交第 8 步核对。 <!-- check-type: execution_process -->
- [ ] [SI-A44] 补充材料小节序号已交第 8 步核对。 <!-- check-type: execution_process -->
- [ ] [SI-A46] 每份 SI 的完整原题与自身来源一致。 <!-- check-type: offline_outcome -->
- [ ] [SI-A47] 每份 SI 的作者姓名与自身来源一致。 <!-- check-type: offline_outcome -->
- [ ] [SI-A48] 每份 SI 的单位名称与自身来源一致。 <!-- check-type: offline_outcome -->
- [ ] [SI-A49] 每份 SI 的作者单位映射与自身来源一致。 <!-- check-type: offline_outcome -->
- [ ] [SI-A50] 每个补充图显示号均可回溯到来源原号。 <!-- check-type: offline_outcome -->
- [ ] [SI-A51] 原文未编号的 SI 文本小节没有被标为原有 S 编号。 <!-- check-type: offline_outcome -->
- [ ] [SI-A52] 每个可辨认的 SI 显示公式均已交付。 <!-- check-type: offline_outcome -->
- [ ] [SI-A53] 每份 SI 的书目条目均已交付。 <!-- check-type: offline_outcome -->
- [ ] [SI-A54] 有原号的补充图显示号符合 S 前缀规则。 <!-- check-type: offline_outcome -->
