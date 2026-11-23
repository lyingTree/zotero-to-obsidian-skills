# 逐段翻译复核操作

## 目录

- [范围与证据边界](#范围与证据边界)
- [准备记录](#准备记录)
- [逐段操作顺序](#逐段操作顺序)
- [术语类别与词族检索](#术语类别与词族检索)
- [关系与表达式的逐处记录](#关系与表达式的逐处记录)
- [状态与不适用条件](#状态与不适用条件)
- [验收证据位置](#验收证据位置)

## 范围与证据边界

一次处理一个完整来源块。正文段、题名、题注、表格应译文本和独立说明分别纳入范围。跨页同段保存多个来源位置；拆分译段保存多个候选位置。每处数值、符号、单位、行内公式和显示公式在所属段内另设记录。段落存在只证明有对应文本，不能证明信息完整或译义准确。

将本次完整来源清单与选择范围对照。`scope.selected_units` 保存应译块；`scope.excluded_units` 保存本次不译的块及具体理由。生成器的 `unselected_inventory_units` 只是待判范围，不能自动成为“不适用”。只有局部修订授权时，才以该授权明确排除范围外单元。

记录只引用本次真实文件。证据的文件引用使用 `path` 与实际 SHA-256；定位另放 `locator`。源 PDF 使用页码和原文片段或边界框；Markdown 使用行段或唯一原样片段；记录文件使用 JSON Pointer。不可把一个整段的 `has_math: true` 当作每个表达式的证据。

`translation-review.json` 是自核台账。交付检查器对此文件只核验文件绑定的真实性、候选与清单的身份，以及已登记来源块的来源关系。它不核验段落覆盖是否完整，不逐条验证台账内的来源或候选定位，也不判断科学译义。空列表、术语命中、文件存在和执行者自述均不产生语义通过。实际过程与离线结果分别填写；没有可回读的过程证据时，不由正确结果反推核对动作已发生。

## 准备记录

按以下顺序准备，避免文件绑定刚生成就过期：

1. 在翻译前登记实际术语表指纹。
2. 完成来源单元清单。
3. 完成会写入清单的 `terminology_path`、`checks_path` 等工具指针。
4. 保存本次待复核的完整译稿版本。
5. 调用 `prepare_translation_review()`。
6. 逐块填写复核结果。
7. 保存最终 `terminology.json`。
8. 将其文件引用写入复核记录的 `terminology_record`。
9. 将最终复核记录作为 `translation_review_path` 显式交给交付清单生成器。

准备函数不修改来源清单，不添加 inventory 指针，不覆盖旧复核记录。来源清单、术语表或候选改变后保留旧记录，新建修订文件；按变化重新核对受影响目标。仅更新哈希不能使旧结论继续有效。后续排版改动行号时，重新绑定实际最终 Markdown 中的对应片段。

在工作目录的 Python 调用脚本中导入技能函数。`unit_ids` 由本次完整来源盘点确定，不按关键词自动选段。下面只示范调用形式，路径与 ID 均须替换为本次实际值：

```python
from pathlib import Path
from prepare_translation_review import prepare_translation_review, new_term_candidate

run_root = Path("本次运行根")
work = run_root.joinpath("work")
review_path = prepare_translation_review(
    work.joinpath("source-inventory.json"),
    work.joinpath("translated.md"),
    work.joinpath("translation-review.json"),
    unit_ids=["source-a-paragraph-k"],
    glossary_path=run_root.joinpath("skills", "template", "术语表.md"),
    candidate_locations={
        "source-a-paragraph-k": [{"quote": "本次译稿中可唯一定位的完整原样片段"}]
    },
)
```

生成结构的精确定义如下。映射的键为实际来源块 ID；JSON Pointer 中的 `~` 转义为 `~0`，`/` 转义为 `~1`。

| 位置 | 保存内容 |
|---|---|
| `/schema_version` | 当前为 `1`。 |
| `/review_status` | 初始 `pending`；完成记录范围的实际自核后才可为 `reviewed`。 |
| `/bindings/source_inventory` | 最后一次固定清单的文件引用。 |
| `/bindings/sources/<source_id>` | 每份实际来源文件的引用。 |
| `/bindings/candidate` | 本次核对的实际 Markdown 文件引用。 |
| `/bindings/glossary` | 实际术语表引用；缺失时为 `null`。 |
| `/scope/selected_units` | 本次选择的来源块 ID。 |
| `/scope/selection_review` | 本次选择是否覆盖授权范围的核对记录。 |
| `/scope/excluded_units` | 不译块的 ID、适用条件和来源证据。 |
| `/units/<id>/source_locations` | 源清单中该块的实际位置。 |
| `/units/<id>/candidate_locations` | 该块在本次 Markdown 中的位置。 |
| `/units/<id>/checks/<check>` | `status`、`reason`、`evidence`。 |
| `/units/<id>/occurrences/<kind>` | 该类每次出现的独立对照记录。 |
| `/units/<id>/term_scan/<category>` | 该术语类别的扫描状态、候选 ID、理由和证据。 |
| `/units/<id>/term_candidates` | 该块所有专业术语候选的逐词检索与取舍。 |
| `/terminology_record` | 最终 `terminology.json` 的真实文件引用。 |
| `/unresolved_terms` | 未决术语及其来源块、候选 ID、原因。 |
| `/source_issues` | 源文疑点及其来源位置、候选读法和处理。 |
| `/source_issues_review` | 对源文疑点的实际检查范围与结论。 |
| `/events` | 实际自核动作的范围、时间和可回读过程证据；初始为空。 |

`checks` 的键为 `alignment`、`ambiguity`、`negation`、`strength`、`number_changes`、`terminology`、`units`、`symbols`、`formulas`、`citation_labels`、`completeness`、`directions`、`citation_claims`、`table_text`、`conditions`、`antecedents`、`modifier_scope`。每个键初始为 `{"status":"pending","reason":null,"evidence":[]}`。生成器不代填任一通过状态。

## 逐段操作顺序

每次取一个 `/units/<id>`，按下面顺序完成后再处理下一块。

1. **打开完整来源段。** 阅读相邻句以确定本段边界；需要可见字形时打开源页图。保存源页位置。
2. **打开完整候选段。** 记录对应 Markdown 位置。缺译时写具体未译原因，保留待核状态。
3. **列出原句命题。** 在 `occurrences.claims` 为每个可独立遗漏的陈述记录主语、关系、对象和原句位置。逐一指出候选对应句；分别判断遗漏与误译。
4. **逐类扫描专业术语。** 按下节全部类别扫完本段。每个候选词进入记录；不因达到某个词数而停止。
5. **核对方位关系。** 为每处空间方向建立独立记录。对照“什么对象相对哪个参照物处于何种方向”，再选译词。
6. **核对否定范围。** 找出否定词限定的谓语或数量；对照译文中的同一范围。
7. **核对适用边界。** 单独列出 only、unless、except、inside/outside、while 等建立的条件；检查条件是否仍约束原来的命题。
8. **核对指代。** 为代词、this/that、前者/后者等记录可指对象。原句未消歧时不要补成单一对象。
9. **核对修饰范围。** 为后置短语、同位说明、并列结构中的限定语记录可修饰对象。原句有多种可读范围时保留该范围。
10. **核对结论强度。** 逐处对照可能、推测、归因、观测、模式输出和反演结果的证据性质。
11. **逐表达式核对。** 分开记录数值、单位、符号和公式的每次出现；操作方法见下一节。
12. **逐次核对引文。** 记录该次原显示标签、所支持命题及目标书目身份；同号重复出现仍分别核对。
13. **完成两种结论。** `checks.alignment` 只判断对应关系；`checks.completeness` 判断每条原命题是否完整保留。译义细项各自填写，不能由“段落有中文”一并置为通过。
14. **回查本段待核项。** 已检查且没有该类对象的项目填写具体不适用条件；其余继续保留 `pending`、失败或未决。

## 术语类别与词族检索

`term_scan` 使用以下固定类别。类别可以重叠；同一个候选允许被多类扫描引用，但候选本身只建一次。同词异义或不同语境需要分别记录出现位置和取舍。

| 类别键 | 本段要查找什么 |
|---|---|
| `concept` | 专业现象、性质、关系或对象名称。 |
| `method` | 分析、拟合、优化或计算方法。 |
| `estimator` | 估计规则、估计量或估计器名称。 |
| `statistic` | 统计量、分布摘要或检验量名称。 |
| `metric` | 距离、误差、相似度、损失或评价度量。 |
| `procedure` | 重采样、筛选、归一化、校准等处理流程。 |
| `model` | 模型、模型族或求解器名称。 |
| `parameter` | 有专门含义的参数、系数或状态量名称。 |
| `classification` | 类别名称、分组规则或分类编码说明。 |
| `abbreviation` | 专业缩写及其本段含义。 |

虚构来源句：`The radial offset was estimated by trimmed regression before mesh relaxation.` 此句至少触发参数/度量、方法/估计器、流程三个方向的扫描。不能只查较长的方法名而忽略其余专业概念。

对每个候选按固定次序检索实际术语表，并将查询本身写入记录：

1. **完整短语**：查询来源原词；仅处理大小写、复数、断词和连字符等检索变体。
2. **别名与缩写**：查询来源明示或已核实属于同一概念的形式。没有别名时为这一检索类别写具体不适用理由。
3. **词族**：查询决定方向或语义的核心词及已核实派生词。例如虚构句中的 `radial offset` 还查 `radial`、`radius`；只搜整串未命中不足以判未收录。
4. **阅读命中行**：记录条目 ID、行号及足以理解用法的上下文。短语子串命中不是概念同一性的证明。
5. **判断本次语境**：区分“同一概念”“相关词族”“不同义项”。记录选用或不选用该条目的原因。
6. **登记采用决定**：同概念条目进入 `adopted`；完整术语未收录则进入 `unlisted`，即使其译法参考了已收录词族。

完整短语和词族都可能有命中；两类记录均保留。没有可用派生词时，核心词本身仍可作为词族查询，说明原因。不要仅为形成命中而把任意相似词当作同族。

`new_term_candidate()` 为已从来源读出的候选创建待核记录。它不会挖掘术语或搜索词表。其 `searches.full_phrase`、`searches.aliases`、`searches.family` 都含独立状态和 `queries`。每次实际查询写入如下结构，示例是虚构检索，不能照抄为本次结果：

```json
{
  "query": "radial",
  "hits": [
    {"glossary_locator": {"lines": [52, 52]}, "entry_id": "fictional-radial-entry", "excerpt": "radial displacement | 径向位移"}
  ],
  "interpretation": "该条是相关方向词族；是否等同完整术语还需本句定义。"
}
```

没有命中也保存 `query` 与空 `hits`。查询完成后才将该检索状态改为 `searched`。`decision` 使用 `adopted`、`unlisted`、`unresolved` 或 `not_a_term`；排除候选时在 `decision_reason` 写明其只是普通语言而非本段专业术语的理由。`translation` 保存本次译词，`terminology_pointer` 指向最终采用记录的精确 JSON 位置。

所有类别扫描完成后才填写该块的 `checks.terminology`。`term_scan` 中已扫描类别使用 `scanned`，没有此类概念的类别使用 `not_applicable` 并写本段证据。未扫描仍为 `pending`。`covered_units` 只从这些实际完成记录中登记，不按候选全文汉字数或已有术语条数推导。

## 关系与表达式的逐处记录

`occurrences` 中每个实例有独立 `id`、`source_span`、`candidate_span`、`status`、`reason`、`evidence`。`source_span` 和 `candidate_span` 各保存实际原样片段及其定位；包含同一词的两句话必须有两条实例，不能只记一个词的全篇命中。

虚构方位句：`The blue tile lies inward of the amber rim.` 记录该关系的主体、参照物和方向，而非只记 `inward` 的译词：

```json
{
  "id": "direction-k",
  "source_span": {"page": 4, "quote": "The blue tile lies inward of the amber rim."},
  "candidate_span": null,
  "source_relation": {"subject": "blue tile", "reference": "amber rim", "relation": "inward of"},
  "candidate_relation": null,
  "status": "pending",
  "reason": null,
  "evidence": []
}
```

填入本次真实候选后比较两边的关系。方向词需要结合局部坐标系、参照物和原句作用对象，不能只凭词表的第一条中文释义决定。

不同语义项使用下表中的专有字段，其余公共字段不变。每条记录只判断该次一个关系或表达式。

| `occurrences` 类别 | 附加字段与比较目标 |
|---|---|
| `negations` | `source_scope`、`candidate_scope`：否定的谓语、对象或量词范围。 |
| `conditions` | `source_condition`、`candidate_condition`：条件与它限定的命题。 |
| `antecedents` | `source_candidates`、`candidate_candidates`：原文允许的指代对象集合。 |
| `modifiers` | `source_candidates`、`candidate_candidates`：原文允许的修饰对象集合。 |
| `strength` | `source_strength`、`candidate_strength`：可能性、因果或证据性质。 |
| `numbers` | `source_expression`、`candidate_expression`、`difference_type`：数值及表示差异。 |
| `units` | 两边的单位表达式及其作用的量。 |
| `symbols` | 两边的符号及其本句定义。 |
| `formulas` | 两边完整表达式；另列分母、根号、指数、括号的作用范围。 |
| `citation_labels` | 两边可见标签及实际书目身份。 |
| `citation_claims` | 两边被支持的具体命题及该次引文目标。 |
| `table_cells` | 原表行列位置、原文单元格和候选可编辑单元格。 |

否定与边界分开核对。虚构句 `The detector reports no stable trace beyond the guarded chamber.` 中，“没有稳定轨迹”是否定内容，“防护腔体之外”是其空间适用范围；只译成“腔体内可报告轨迹”没有明确保留原句的带外否定。虚构句 `The update is used only while the latch remains closed.` 要保留更新操作受锁扣状态约束的关系，不能删除 `only`。

原文歧义也属于要保留的信息。虚构句 `The operator told the assistant that she should recalibrate the sensor.` 未唯一决定 `she` 指谁；不要擅改成“操作员应……”或“助手应……”。先记录两种可能对象，再选择保留原指代空间的译法。虚构句 `She examined the specimen with a lens.` 中末尾短语的作用需要上下文；不能无依据强行固定修饰范围。读完实际相邻句仍不能消歧时，保留未决记录；必要说明放在独立译注，不混入作者结论。

数值、单位、符号和公式按每次出现定位。虚构表达式 `q = a/(b + c)` 的记录要覆盖完整分母 `b + c`，候选也定位到该完整式；不能只以变量 `q` 命中证明转录一致。段中有多个公式时分开编号，行内式不能被省略。Unicode 字形、显示月份或行内/显示排版变化可记录为 `representation_only`，但须给出等价理由；有数值或作用范围变化使用 `value_changed` 或 `structure_changed`，不确定则 `unresolved`。

## 状态与不适用条件

核对项的状态使用 `pending`、`passed`、`failed`、`blocked`、`not_applicable` 或 `unresolved`。检索项另用 `searched`，术语类别扫描另用 `scanned`。这些动作状态不能代替语义项的判定。

| 情况 | 记录方法 |
|---|---|
| 尚未检查 | 保留 `pending`。 |
| 来源读不清 | 写 `blocked` 和具体来源位置。 |
| 有两种有据读法 | 写 `unresolved` 和候选读法。 |
| 本段没有方位关系 | 在阅读完整段后写 `not_applicable`，理由指出本段内容与检查范围。 |
| 本段没有数值、单位或公式 | 分别对对应类别写具体不适用条件，不能用一次空列表覆盖所有类别。 |
| 本块无需翻译 | 记录实际语种或局部任务范围；译义检查不适用，来源对齐仍可核对。 |
| 未收录术语全部已解决 | 记录其采用依据后，才可说明没有未决术语。 |
| 没有源文疑点 | 在 `source_issues_review` 写明实际检查的来源范围和依据。 |

`review_status: reviewed` 只表示记录声明的范围已经检查，不表示所有项通过。不能把失败或未决统一改成已通过。对外的 `checks.json` 不支持 `pending`、`unresolved` 时均保留 `not_verified`，并附具体原因；需要环境或来源不可读时使用 `blocked`。

实际自核动作可在 `events` 中记录 `unit_ids`、`check`、`performed_at`、`action` 与 `trace_evidence`。时间必须是实际动作时间。`trace_evidence` 引用已存在的工具记录或操作记录。没有独立轨迹时说明这是自述；不要创建一个带时间的 JSON 就声称取得了独立过程证据。

## 验收证据位置

下表给出各 TRN-A ID 的记录入口。`<id>` 和数组索引均须换成实际位置；对每个适用来源块逐一列出指针。不要只绑定整个记录文件就宣称所有细项通过。

| ID | 具体记录位置 | 需要支持的单一判断 |
|---|---|---|
| TRN-A01 | `/units/<id>/candidate_locations`、`/units/<id>/checks/alignment` | 译块对应或未译原因。 |
| TRN-A02 | `/units/<id>/checks/ambiguity`、相应指代/修饰实例 | 歧义对照记录。 |
| TRN-A03 | `/units/<id>/occurrences/negations/<n>`、`/units/<id>/checks/negation` | 否定核对结果记录。 |
| TRN-A04 | `/units/<id>/occurrences/strength/<n>`、`/units/<id>/checks/strength` | 结论强度对照记录。 |
| TRN-A05 | `/scope`、`/units/<id>/checks` | 披露实际覆盖范围。 |
| TRN-A06 | `/units/<id>/occurrences/numbers/<n>` | 数值差异的有据判定。 |
| TRN-A07 | `/units/<id>/term_candidates/<n>`、实际术语采用记录 | 采用术语的对源依据。 |
| TRN-A08 | `/units/<id>/occurrences/units/<n>` | 单位表达式一致。 |
| TRN-A09 | `/units/<id>/occurrences/formulas/<n>` | 公式表达式一致。 |
| TRN-A10 | `/units/<id>/occurrences/citation_labels/<n>` | 引文标签一致。 |
| TRN-A11 | `/unresolved_terms`、相应术语候选位置 | 未决术语登记。 |
| TRN-A12 | `/source_issues`、`/source_issues_review` | 源文疑点登记。 |
| TRN-A13 | `/units/<id>/occurrences/claims/<n>`、`/units/<id>/checks/completeness` | 原块完整信息。 |
| TRN-A14 | `/units/<id>/occurrences/directions/<n>` | 方位关系一致。 |
| TRN-A15 | `/units/<id>/occurrences/citation_claims/<n>` | 引文支持命题一致。 |
| TRN-A16 | `/units/<id>/occurrences/table_cells/<n>`、`/units/<id>/checks/table_text` | 图内表应译文本完整。 |
| TRN-A17 | `/bindings/glossary`、`/terminology_record` 及所引文件 `/glossary` | 实际术语表哈希一致。 |
| TRN-A18 | 每个 `unlisted` 候选及其 `terminology_pointer` | 采用依据或未决记录。 |
| TRN-A19 | `/units/<id>/term_scan/<category>` | 类别扫描结果登记。 |
| TRN-A20 | `/units/<id>/term_candidates/<n>/searches/family` | 词族检索结果登记。 |
| TRN-A21 | `/units/<id>/occurrences/negations/<n>` | 否定范围一致。 |
| TRN-A22 | `/units/<id>/occurrences/conditions/<n>` | 适用边界一致。 |
| TRN-A23 | `/units/<id>/occurrences/antecedents/<n>` | 未消歧指代得到保留。 |
| TRN-A24 | `/units/<id>/occurrences/modifiers/<n>` | 未消歧修饰范围得到保留。 |
| TRN-A25 | `/bindings/candidate` | 记录绑定当前实际译稿。 |

使用 `check_records.bind_evidence(review_path, {"kind":"json_pointer","pointer":"/units/实际块ID/checks/completeness"})` 可绑定真实文件并写入具体定位。将该证据放入 `checks.json` 后，`validate_check_records()` 才会检查这个 JSON Pointer 是否实际存在；定位存在不代表记录内容足够或译义正确。另附源文件与候选的实际位置才能评估语义。所有 ID 都要各自给出范围、理由及证据，不能用一个“全文已读”事件填满全部过程项。

本表的 TRN-A 目标均按现存文件与来源核验，使用 `offline_outcome`。`events` 不是这些文件目标的通过前提。翻译前登记、改稿后复核等真正时序约束仍另行记录；独立评审无法观察的过程保持未验证，不能由文件目标通过反推执行次序。
