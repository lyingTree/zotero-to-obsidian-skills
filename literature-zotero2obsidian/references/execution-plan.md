# 分阶段执行路线

用于从全文新建完整笔记。局部修改仍按用户指定范围执行。先把来源盘点完成，再逐段构建候选；中间检查发现问题时返回对应阶段。阶段名称不是验收结论。

## 路线

| 阶段 | 当前任务 | 留存产物 | 下一阶段入口 |
|---|---|---|---|
| 0 范围 | 确认实际输入与输出范围 | run.json、checks.json 待填骨架 | [项目配置](project-profile.md) |
| 1 来源 | 建立所有来源的逐页材料 | source-inventory.json、页图、原始文字 | [来源索引说明](delivery-checks.md) |
| 2 盘点 | 登记来源中的内容单元 | source-map.json、si-map.json | [全文提取](../../literature-fulltext-extract/SKILL.md) |
| 3 译文 | 按来源块完成中文译稿 | translated.md、si.md | [翻译](../../literature-translate-zh/SKILL.md) |
| 4 结构 | 组装题头与正文对象 | drafts/06 至 08 | [正文排版](../../literature-body-format/SKILL.md) |
| 5 链接 | 按来源作用域连接对象 | drafts/09、link-map.json | [篇内链接](../../literature-internal-links/SKILL.md) |
| 6 交付 | 固定最终候选与资源路径 | final.md、delivery-manifest.json、台账文件绑定 | [自动检查](automated-validation.md) |
| 7 对源 | 逐页核对最终候选 | checks.json 逐验收 ID 的实际结论与证据 | [逐篇验收](acceptance.md) |

## 逐项台账贯穿各阶段

开始时调用 [`check_records.prepare_check_records(...)`](../scripts/check_records.py)，从实际技能文件生成全量 `catalog` 与待填 `checks`。默认待填集合覆盖各技能及 references 中全部 `A` 编号验收；追踪约束时使用 `additional_ids` 明确加入，或选择 `rule_selection="all"`。每个验收 ID 从 `not_verified` 开始，不因阶段结束而整体转为通过。

来源文件从准备时绑定实际哈希。最初没有候选时可暂不传候选路径；在阶段 6 前，把已存在的最终候选用 `bind_file()` 得到的路径和哈希填入 `bindings.candidates`。缺少候选绑定时校验会诊断，但这不禁止继续生成候选。每次候选变化都重新核验受影响记录，不通过更新哈希掩盖旧证据失效。已有台账默认不会被 `prepare_check_records()` 覆盖；需要重新生成时先保留旧版本及其实际结论。

执行中把每项的实际对象写入 `scope`，把稳定证据文件与精确 `locator` 登入共享证据表，再由对应检查引用证据 ID。相同证据文件可被多个明确 ID 使用；每个 ID 的适用范围、状态与理由仍分别填写。范围包含多页、多对象时列出这些实际对象，不能把某子技能整体通过当作逐项记录。详见[单项台账契约](acceptance.md#单项记录)。

过程与文件结果分别登记。只看到最终文件不能补写“已经执行”的过程结论；没有观察到的过程保持 `not_verified`。文件内容则由当前执行者逐项对源核对。只生成候选时，实际写库步骤按具体条件记为不适用。此流程不安排应用界面审查。

阶段 6 固定稳定的源页、对象记录和检查报告，再引用其哈希。`inventory_path` 可让准备函数回写 `checks_path`；inventory、manifest、台账自身仍可能改写，不能互相作为正在变化的哈希证据造成循环。最终由主交付校验器绑定台账文件。阶段 7 先完成执行者自己的逐项对源核对，再检查 `offline_records_ready`；不能只确认台账格式合法就结束。

## 阶段约束

- [EXEC-C01] 处理当前阶段前读取该阶段对应的子技能。
- [EXEC-C02] 跨阶段只复用有来源依据的工作值。
- [EXEC-C03] 每个来源使用独立的内容身份。
- [EXEC-C04] PDF 提取前运行 `prepare_source_inventory.prepare_sources()`。
- [EXEC-C05] 使用可查看图像的能力逐页查看生成的页图。
- [EXEC-C06] 页图内容不清楚时重新渲染该区域。
- [EXEC-C07] 文字层与页图不一致时以可见来源核定该字段。
- [EXEC-C08] 每页保留实际核对记录。
- [EXEC-C09] 原始文字层不被后续通顺化覆盖。
- [EXEC-C10] 按实际阅读顺序建立来源段落清单。
- [EXEC-C11] 每个来源独立建立题头字段清单。
- [EXEC-C12] 来源未显示的题头字段登记为未提供。
- [EXEC-C13] 每幅图登记其实际构成。
- [EXEC-C14] 每幅图登记是否含有真正的表格。
- [EXEC-C15] 图内真正表格单独进入表格清单。
- [EXEC-C16] 每张表登记原始行结构。
- [EXEC-C17] 每个显示公式登记来源位置。
- [EXEC-C18] 行内公式登记到所属来源段落。
- [EXEC-C19] 每份书目独立登记完整条目。
- [EXEC-C20] 每次文内引文登记对应的来源书目。
- [EXEC-C21] 盘点完成后再决定各对象的交付位置。
- [EXEC-C22] 每个被排除的来源块登记具体依据。
- [EXEC-C23] 全文整理不得以摘要替代来源段落。
- [EXEC-C24] 翻译按来源块顺序推进。
- [EXEC-C25] 译稿保留来源块映射。
- [EXEC-C26] 各来源的元信息分别进入对应题头。
- [EXEC-C27] 表格数据通过序列化器生成 Rich Table。
- [EXEC-C28] 表格生成后重新解析其数据。
- [EXEC-C29] 每个表格单元格对照可见来源核对。
- [EXEC-C30] 数学表达式对照可见来源核对。
- [EXEC-C31] 原始图件按[图像规则](../../literature-fulltext-extract/references/figure-crop.md)提取。
- [EXEC-C32] 每次引文调用按对应来源书目分配目标。
- [EXEC-C33] 章节调用指向它实际提到的小节。
- [EXEC-C34] 最终资源复制到候选实际解析根内。
- [EXEC-C35] 最终候选建立 `delivery-manifest.json`。
- [EXEC-C36] 运行 `validate_note.validate_file()` 检查最终源码。
- [EXEC-C37] 运行 `validate_delivery` 检查最终交付索引。
- [EXEC-C38] 运行 `lint_presentation` 检查易漏的可见排版。
- [EXEC-C39] 修正自动检查确认的当前范围错误。
- [EXEC-C40] 对提示的非问题项记录逐条解释。
- [EXEC-C41] 每次候选修改后重验受影响对象。
- [EXEC-C42] 内容核对从来源逐页走向候选。
- [EXEC-C43] 完整生成任务检查每个适用目标。
- [EXEC-C44] 发现一处错误后继续核对其余适用目标。
- [EXEC-C45] 内容核对记录引用当前候选版本。
- [EXEC-C46] 真实环境不可用时继续完成来源支持的候选内容。
- [EXEC-C48] 工作报告不得把文字层读取写成可见页面核验。
- [EXEC-C49] 工作报告不得把自填清单写成独立评审。
- [EXEC-C51] 文献处理过程中保留技能源码不变。
- [EXEC-C52] 本次处理参数写入工作目录中的调用脚本。
- [EXEC-C53] 书目中的疑似拼写错误保留原文。
- [EXEC-C54] 书目字段边界不明时保留对应的完整原文片段。
- [EXEC-C55] 不凭模型记忆补全书目字段。
- [EXEC-C50] 每张表登记原始列结构。

## 对源顺序

按下列顺序核对，可减少在长稿中反复寻找对象。每行只给出一个核对目标；实际规则 ID 仍取相应子技能。

| 顺序 | 单项目标 | 核对方式 |
|---|---|---|
| 1 | 每个来源的题名正确 | 对照该来源标题页 |
| 2 | 每个来源的作者身份正确 | 逐名对照署名区 |
| 3 | 每个来源的单位内容完整 | 逐条对照单位区 |
| 4 | 作者单位映射正确 | 对照上标标记 |
| 5 | 日期结论有可见来源依据 | 查看包含日期的页图 |
| 6 | 每个来源段落都有交付 | 从首段到末段逐段回查 |
| 7 | 每个译文段落忠于来源 | 对照该段原文 |
| 8 | 每幅图的内容完整 | 对照源图与最终文件 |
| 9 | 每张表的单元格正确 | 按行逐格核对 |
| 10 | 每个公式的符号关系正确 | 对照可见公式 |
| 11 | 每条书目身份正确 | 逐条对照原书目 |
| 12 | 每次引文实际指向正确 | 对照该句来源与目标书目 |
| 13 | 每处量值排版符合规则 | 检查数学环境边界 |
| 14 | 每个链接目标对应正确对象 | 回查对象清单 |
| 15 | 每条完成声明有对应证据 | 回查执行记录 |

## 阶段验收

- [ ] [EXEC-A01] 每份来源的页数来自实际文件。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A02] 每页来源有对应渲染材料。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A03] 每页来源有实际查看记录。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A04] 每份书目的边界已确定。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A05] 每个来源段落都有交付映射。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A06] 每个图内真正表格都有结构化交付。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A07] 每个范围内公式都有交付映射。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A08] 每条来源书目都有交付映射。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A09] 每次来源引文都有核对记录。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A10] 每个本地嵌入资源可实际读取。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A11] 自动检查发现均有处置记录。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A12] 每项内容结论有独立证据位置。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A13] 报告中的剩余问题与实际记录一致。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A14] 本次技能源码哈希与输入基线一致。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A15] 书目原文疑点均有工作记录。 <!-- check-type: offline_outcome -->
- [ ] [EXEC-A16] 每个书目字段均有来源依据。 <!-- check-type: offline_outcome -->
