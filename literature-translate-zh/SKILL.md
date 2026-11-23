---
name: literature-translate-zh
description: 根据已核实的文献语言和项目术语表，将原文 Markdown 完整译成中文；中文文献复用原稿，混合文献按内容块分流。用于全文翻译或已有译文的指定部分修订。
---

# 按术语表翻译

## 必要输入

- item.json、source.md、source-map.json、术语表。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。
- 开始翻译前完整读取[逐段翻译复核操作](references/translation-review.md)。

## 执行步骤

1. 核实语种，按源块 ID 列出本次应译范围。
2. 绑定实际术语表指纹。
3. 按来源块逐段翻译。
4. 在清单与候选固定后生成默认待核的 `translation-review.json`。
5. 按[逐段操作顺序](references/translation-review.md#逐段操作顺序)逐项回查完整来源段。
6. 将每个专业术语的检索与取舍记录写入对应来源块。
7. 将术语采用决定汇入 `terminology.json`。
8. 运行[记录检查函数](scripts/validate_translation_review.py)，按每条诊断补齐本次实际记录。
9. 按[验收证据位置](references/translation-review.md#验收证据位置)填写各 ID 的实际结果。

## 约束

每条约束按其 ID 单独记录执行证据。

- [TRN-C01] 语种为 zh 时复用 source.md。
- [TRN-C02] 中文文献中的外文题名不触发整篇重译。
- [TRN-C03] 中文文献中的外文摘要不触发整篇重译。
- [TRN-C04] 语种为 non-zh 时翻译应译正文。
- [TRN-C05] 语种为 non-zh 时翻译应译题注。
- [TRN-C06] 语种为 non-zh 时翻译应译表头。
- [TRN-C07] 语种为 non-zh 时翻译应译注释。
- [TRN-C08] 语种为 mixed 时优先复用已有的完整中文对应块。
- [TRN-C09] 为复用的中文对应块记录映射。
- [TRN-C10] 语种为 unknown 或判断冲突时交第 1 步复核。
- [TRN-C11] 记录所用术语表路径。
- [TRN-C12] 记录所用术语表哈希。
- [TRN-C13] 术语表缺失时将译稿标为待核。
- [TRN-C14] 同义概念按语境统一译词。
- [TRN-C15] 多义词按每处语境确定译词。
- [TRN-C16] 重叠上下文只输出一次。
- [TRN-C17] 保留原文论证顺序。
- [TRN-C18] 保留原文否定含义。
- [TRN-C19] 保留原文适用条件。
- [TRN-C20] 保留原文因果强度。
- [TRN-C21] 保留原文不确定性。
- [TRN-C22] 保留模式结果原有的证据性质。
- [TRN-C23] 保留反演结果原有的证据性质。
- [TRN-C24] 保留作者推断原有的证据性质。
- [TRN-C25] 不添加原文没有的解释。
- [TRN-C26] 不添加原文没有的结论。
- [TRN-C27] 数值与原文对应。
- [TRN-C28] 单位与原文对应。
- [TRN-C29] 符号与原文对应。
- [TRN-C30] 公式与原文对应。
- [TRN-C31] 对象 ID 与原文对应。
- [TRN-C32] 编号与原文对应。
- [TRN-C33] DOI 与原文对应。
- [TRN-C34] URL 与原文对应。
- [TRN-C35] 人名照录原文。
- [TRN-C36] 所属单位照录原文。
- [TRN-C37] 作者—年份引文中的 `et al.` 照录原文，不译为“等”。
- [TRN-C38] 作者—年份引文中的双作者连接词 `and` 照录原文，不译为“和”。
- [TRN-C39] 中文文献原引文中的“等”照录原文。
- [TRN-C40] 中文文献原引文中的“和”照录原文。
- [TRN-C41] 原刊引文中的 `&` 等其他连接形式照录原文。
- [TRN-C42] 表格翻译只作用于文本。
- [TRN-C43] 保留表格单元格中非应译文本的原始值。
- [TRN-C44] 保留表格单元格的合并关系。
- [TRN-C45] 原始图像中的文字保留原样。
- [TRN-C46] 编号公式引用统一为「公式3b」等形式。
- [TRN-C47] 保留公式引用的编号前缀。
- [TRN-C48] 保留公式引用的编号后缀。
- [TRN-C49] 保留公式引用的编号范围。
- [TRN-C50] 方程术语依原义翻译。
- [TRN-C51] 外文题名另产中文译名。
- [TRN-C52] 保留完整原题。
- [TRN-C53] 书目保留发表时原文，包括中文条目。
- [TRN-C54] 不为书目附加题名中译。
- [TRN-C55] 为未收录术语记录原词。
- [TRN-C56] 为未收录术语记录拟译。
- [TRN-C57] 为未收录术语记录语境。
- [TRN-C58] 为未收录术语记录待核原因。
- [TRN-C59] 不自动修改共享术语表。
- [TRN-C60] 为原文疑误记录来源定位。
- [TRN-C65] 对照来源复核原文疑误。
- [TRN-C61] 为 OCR 歧义记录来源定位。
- [TRN-C66] 对照来源复核 OCR 歧义。
- [TRN-C62] 不猜补数值。
- [TRN-C63] 不猜补定义。
- [TRN-C64] 必要译注独立写为 `> 译注：…`。
- [TRN-C67] 图内数据表的独立可编辑副本按表格文本翻译。
- [TRN-C68] 每处方位关系按原文对应关系翻译。
- [TRN-C69] 每处数字引文跟随其在原文中支持的陈述。
- [TRN-C70] 译文不得用章节摘要替代应译正文段落。
- [TRN-C71] 翻译开始前在 `terminology.json` 登记实际术语表指纹。
- [TRN-C72] 每个未收录专业术语独立登记一条记录。
- [TRN-C73] 已解决的未收录术语记录其译法依据。
- [TRN-C74] 每个应译来源块建立独立翻译复核记录。
- [TRN-C75] 每个应译块保存逐类术语扫描记录。
- [TRN-C76] 每条术语候选保存完整短语的检索记录。
- [TRN-C77] 每条术语候选保存词族检索记录。
- [TRN-C78] 有别名的术语候选保存别名检索记录。
- [TRN-C79] 每处方位关系建立独立对照记录。
- [TRN-C80] 每处否定范围建立独立对照记录。
- [TRN-C81] 每处适用边界建立独立对照记录。
- [TRN-C82] 译文保留原文未消歧的指代关系。
- [TRN-C83] 译文保留原文未消歧的修饰范围。
- [TRN-C84] 未完成核对的复核项保持 `pending`。
- [TRN-C85] 复核结论仅支持其绑定的译稿版本。
- [TRN-C86] 每个数值表达式建立独立对照记录。
- [TRN-C87] 每个单位表达式建立独立对照记录。
- [TRN-C88] 每个符号表达式建立独立对照记录。
- [TRN-C89] 每个公式表达式建立独立对照记录。

### 术语记录的具体做法

完整 PDF 流程调用 `prepare_sources(..., glossary_path=实际术语表路径)`，自动建立含真实路径和 SHA-256 的 `terminology.json`。生成时 `review_status` 为 `pending`；翻译者仍需实际查看术语表并填写记录。独立翻译任务也按同一结构建立记录，不改共享术语表。

逐来源块按[术语类别与词族检索](references/translation-review.md#术语类别与词族检索)穷举候选，不设术语数量上限。方法、估计器、统计量、度量和流程名称分别进入扫描；完整短语未命中后继续查词族，不能据此直接宣告未收录。术语表已有同概念条目时写入 `adopted`，记录 `original`、`translation`、`context`、`source_units` 和 `glossary_locator`。仅有相关词族条目时记录参考依据，不能冒称完整术语已收录。未收录时写入 `unlisted`，每条分别填写：

| 字段 | 单一记录目标 |
|---|---|
| `original` | 原词。 |
| `translation` | 本次拟采用的中文。 |
| `context` | 来源语境。 |
| `source_units` | 实际包含该词的来源单元 ID。 |
| `reason` | 需要核对的原因。 |
| `status` | `resolved` 或 `unresolved`。 |
| `resolution_basis` | 决定该译法的依据。 |

“术语表未收录”是待核原因，不能据此声称译法已有标准依据。可根据来源定义与当前语境作出有依据的译法决定；仍不能消歧时保留 `unresolved`。不要把本次译法写回共享术语表。

`covered_units` 登记已完成术语核对的正文、章节、图题注、表格、题名和题头说明单元。全部相应单元完成后将 `review_status` 改为 `reviewed`。若没有未收录术语，用 `no_unlisted_reason` 记录实际检索依据，不以空列表代替核对。交付检查会绑定本次记录的文件哈希，但不会自动判定术语译义正确。

### 翻译复核记录

使用[准备函数](scripts/prepare_translation_review.py)生成来源块骨架，或按同一结构手工记录。骨架不自动选择应译范围，不自动提取术语，不填写语义通过。完整操作、逐表达式定位、状态条件及各验收 ID 的 JSON 位置见[逐段翻译复核操作](references/translation-review.md)。

每次只填一个来源块的一项目标。准备函数会把清单内合法的 `page/basis/blocks` 定位展开为同一页真实 block 的原始文字和边界框，保留原 ID；它不判断这些 block 是否覆盖完整段落。完成填写后调用 `validate_translation_review(path, inventory_path=None, candidate_path=None)`，依次处理其 `findings` 中的具体单元与目标。`records_ready` 仅检查记录是否齐备且无尚未处理的申报状态，`source_fidelity` 始终为 `not_assessed`。不要把检查器无发现改写为译义通过，也不要用它生成不适用理由。

需要填写范例时读取[单事实脚本示例](references/fill_review_example.py)。示例只为两句虚构文本记录一个已核方位关系；其余项目保持待核。同一事实可以被多个明确单目标 ID 引用，每个 ID 仍分别写范围、理由和结果。

## 产物

- run.json.content_input、terminology.json、translation-review.json；需要翻译时生成 translated.md。

## 验收标准

每项验收按其 ID 单独记录判定结果。

- [ ] [TRN-A01] 每个应译块均有对应译块或未译原因。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A02] 每个应译块的翻译歧义均有对照记录。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A03] 每个应译块的否定核对结果均有记录。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A04] 每个应译块的结论强度均有对照记录。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A05] 翻译复核记录注明实际覆盖范围。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A06] 每项数值差异均有来源支持的判定记录。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A07] 每条采用术语均有对源核对依据。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A08] 每处单位表达式与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A09] 每处公式表达式与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A10] 每处引文标签与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A11] 未决术语已登记。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A12] 原文疑点已登记。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A13] 每个应译正文块均保留原块的完整信息。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A14] 每处方位关系与原文一致。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A15] 每处数字引文仍对应原文所支持的陈述。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A16] 每个图内数据表的可编辑副本已完成应译文本的翻译。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A17] 术语记录中的文件哈希与实际术语表一致。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A18] 每个未收录术语均有本次采用译法的依据或未决记录。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A19] 每个应译块的术语类别扫描结果已记录。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A20] 每条术语候选的词族检索结果已记录。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A21] 每处否定范围与原文一致。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A22] 每处适用边界与原文一致。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A23] 译文保留原文未消歧的指代关系。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A24] 译文保留原文未消歧的修饰范围。 <!-- check-type: offline_outcome -->
- [ ] [TRN-A25] 翻译复核记录绑定当前译稿的文件哈希。 <!-- check-type: offline_outcome -->
