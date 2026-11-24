---
name: literature-header-format
description: 核对并规范文献的原文标题、中文译名、作者、单位、通讯标记和邮箱区域。用于题头构建或修复，不改 YAML 属性及正文论述。
---

# 规范标题、作者和单位

## 必要输入

- 前序候选或既有 MD、item.json、原文题头位置及译名。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

新候选的题头由 [Python 组装入口](../literature-zotero2obsidian/references/deterministic-pipeline.md)的 `header` / `metadata` 单元生成。源题名、作者顺序、上标、单位、邮箱与元信息先对源确认，程序负责转义、分行及间距；不让模型再次手工拼接。既有候选用规范化入口处理已能明确识别的格式，角色不明时保留原文并定位报告。

1. 查看所处理来源的题头页图。
2. 在来源映射中逐项核对题头字段。
3. 排列完整原题及适用的译名。
4. 按源页映射排列作者的单位上标。
5. 逐条排列原文单位。
6. 整理单位后的元信息引用块。
7. 直接读取 Markdown 中的候选题头，与源页逐项比较。

文字层的日期仍为占位、缺字或与页图不同时，按以下顺序核对。全文页图缩略得太小不能解决这个差异。

1. 单独放大来源中的日期区域。
2. 在该字段的来源单元中记录实际可见值。
3. 在该字段的来源单元中记录区域定位。
4. 保留文字层原值。
5. 根据可见值填写候选日期。

若放大后的原刊也确实只有占位，照录该占位。对应记录说明该区域的实际观察。不得从网络补造本来源没有显示的日期。

## 约束

每条约束按其 ID 单独记录执行证据。

- [HDR-C01] 原文与条目不一致时保留差异证据。
- [HDR-C02] 不跨文献版本拼凑题头。
- [HDR-C03] 主全文完整原题使用 Markdown H1。
- [HDR-C04] 不以截断文件名代替原题。
- [HDR-C05] 外文题名的中译紧接原题下一行。
- [HDR-C06] 中文原题不生成同义译名。
- [HDR-C07] 题名区与作者之间留一个空行。
- [HDR-C08] 作者与单位之间留一个空行。
- [HDR-C09] 题头不添加对齐样式。
- [HDR-C10] 不使用 HTML 包装题头。
- [HDR-C11] 作者姓名照录原文。
- [HDR-C12] 转义作者姓名中的字面语法字符。
- [HDR-C13] 所属单位名称照录原文。
- [HDR-C14] 转义单位名称中的字面语法字符。
- [HDR-C15] 作者顺序依原文。
- [HDR-C16] 作者之间使用分号分隔。
- [HDR-C17] 作者邮箱链接使用 `[姓名](mailto:…)` 格式。
- [HDR-C18] 作者邮箱只采用本篇对应邮箱。
- [HDR-C19] 单位编号使用行内 LaTeX 上标，如 `${}^{1}$`。
- [HDR-C20] 通讯星号使用行内 LaTeX 上标，如 `${}^{1,\ast}$`。
- [HDR-C21] 单位行以对应编号起首，如 `${}^{1}$`。
- [HDR-C22] 多个单位逐行列出。
- [HDR-C23] 相邻单位行之间使用行尾两个空格硬换行。
- [HDR-C24] 无证据时不新增题头标记。
- [HDR-C25] 旧标记缺少证据时保留原标记。
- [HDR-C26] 旧标记缺少证据时登记待核。
- [HDR-C27] 不在作者行之外新增独立的通讯作者名单。
- [HDR-C28] 不添加开头引文行。
- [HDR-C29] 单位后的 DOI、日期、版权、基金、通讯说明、地址及引用格式归入同一 `> ` 引用块。
- [HDR-C30] 元信息引用块中的各项信息逐行排列。
- [HDR-C31] 元信息引用块内不留空行。
- [HDR-C32] 元信息引用块前留一空行。
- [HDR-C33] 元信息引用块后留一空行。
- [HDR-C34] 原文没有元信息时省略元信息引用块。
- [HDR-C35] 摘要交正文步骤处理。
- [HDR-C36] 关键词交属性步骤处理。
- [HDR-C37] 对未核实的单位记录待核状态。
- [HDR-C38] 对原文未提供的单位记录缺失状态。
- [HDR-C39] 可辨认的作者单位上标须写入候选作者行。
- [HDR-C40] 每个作者的单位上标只能来自该来源的题头。
- [HDR-C41] 题头中可辨认的每条单位须在候选中完整保留。
- [HDR-C42] 日期字段采用该来源可见页面的实际日期。
- [HDR-C43] 日期记录保留原文的日期类别，如接收、接受或在线发表。
- [HDR-C44] 题名相近的另一附件不能替代本来源的题头核验。
- [HDR-C45] 每条单位后元信息独立登记为来源单元。
- [HDR-C46] 作者行的邮箱链接不能替代来源中的通讯说明。
- [HDR-C47] 每份 SI 的元信息归入该 SI 自己的题头引用块。

### 通讯说明的交付位置

作者行解决姓名与邮箱的对应关系；来源中的通讯、材料索取等说明仍按原义交付到单位后的引用块。主文与每份 SI 各自处理。以下仅示意结构，姓名、邮箱及说明取自当前来源：

```markdown
[Dana Example](mailto:dana@example.org)${}^{1,\ast}$; Alex Example${}^{1}$

${}^{1}$Institute One

> 通讯及材料索取：请联系Dana Example（[dana@example.org](mailto:dana@example.org)）。
```

来源若只说明通讯身份，交付该身份说明，不凭示例增加“材料索取”。来源若没有通讯说明，不生成对应行。[来源清单](../literature-zotero2obsidian/references/delivery-checks.md)中的 `header_note` 可分别登记这些说明，每个单元只映射一条元信息。

### 元信息逐项拆分

“一条元信息”按字段含义判断，不按 PDF 的文字块或排版行判断。一块原文同时出现收稿、接受、发表日期时，先建三个来源单元，再生成三条引用行。发表日期使用 `publication_date`；收稿和接受日期各用一个 `header_note` 单元。原文中的开放获取声明、卷期页码、DOI 同样各占自己的单元和引用行。

下面的虚构示例只演示分行方式；没有出现在来源中的字段不生成。

```markdown
> Received: 3 May 2020
> Accepted: 7 June 2020
> Published online: 19 June 2020
> Open access
> Example Journal 4: 28
> DOI: [10.9999/example](https://doi.org/10.9999/example)
```

交付映射的目标片段只选本条字段。多个单元可以引用同一个来源文字块，但不能让一个 `publication_date` 单元包办所有日期。元信息内部不插入空行，整块前后各留一个空行。最终运行 `lint_presentation()`；`HEADER_ITEMS_SAME_LINE` 提示已识别的多日期或 DOI 合行，仍需人工核对开放获取、版权等其他字段。

## 产物

- drafts/07-header.md。

## 验收标准

每项验收按其 ID 单独记录判定结果。

- [ ] [HDR-A01] 每份来源的题头原题只出现一次。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A02] 原题内容完整。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A03] 中文译名位于规定位置。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A04] 作者顺序有原文依据或待核记录。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A05] 作者邮箱有本篇依据或待核记录。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A06] 作者与单位的映射有依据或待核记录。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A07] 通讯标记有依据或待核记录。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A08] 多单位之间的硬换行符合规定。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A09] 元信息归入同一引用块。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A10] 元信息逐行排列。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A11] 元信息引用块内没有空行。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A12] 元信息引用块前恰有一空行。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A13] 元信息引用块后恰有一空行。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A14] 逐段比较确认属性未被改动。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A15] 逐段比较确认正文未被改动。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A17] 每个可辨认的作者单位上标均已交付。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A18] 每个作者的单位编号与该来源一致。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A19] 每条可辨认的单位名称与该来源一致。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A20] 每个日期值与该来源可见页面一致。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A21] 每个日期标签与该来源的日期类别一致。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A22] 来源中的每条通讯说明均在对应来源的题头引用块中。 <!-- check-type: offline_outcome -->
- [ ] [HDR-A23] 每条单位后元信息具有独立交付映射。 <!-- check-type: offline_outcome -->
