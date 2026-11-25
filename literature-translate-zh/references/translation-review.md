# 逐段翻译复核操作

## 目录

- [范围与证据边界](#范围与证据边界)
- [准备记录](#准备记录)
- [逐段待办与范围计划](#逐段待办与范围计划)
- [逐段短入口与增量保存](#逐段短入口与增量保存)
- [展开清单中的 block 定位](#展开清单中的-block-定位)
- [逐段操作顺序](#逐段操作顺序)
- [术语类别与词族检索](#术语类别与词族检索)
- [自动取得检索线索](#自动取得检索线索)
- [关系与表达式的逐处记录](#关系与表达式的逐处记录)
- [状态与不适用条件](#状态与不适用条件)
- [检查记录并处理诊断](#检查记录并处理诊断)
- [完整单段示例](#完整单段示例)
- [一个事实对应多个单目标](#一个事实对应多个单目标)
- [验收证据位置](#验收证据位置)

## 范围与证据边界

一次处理一个完整来源块。正文段、题名、题注、表格应译文本和独立说明分别纳入范围。跨页同段保存多个来源位置；拆分译段保存多个候选位置。每处数值、符号、单位、行内公式和显示公式在所属段内另设记录。段落存在只证明有对应文本，不能证明信息完整或译义准确。

将本次完整来源清单与选择范围对照。`scope.selected_units` 保存应译块；`scope.excluded_units` 保存本次不译的块及具体理由。生成器的 `unselected_inventory_units` 只是待判范围，不能自动成为“不适用”。只有局部修订授权时，才以该授权明确排除范围外单元。

记录只引用本次真实文件。证据的文件引用使用 `path` 与实际 SHA-256；定位另放 `locator`。源 PDF 使用页码和原文片段或边界框；Markdown 使用行段或唯一原样片段；记录文件使用 JSON Pointer。不可把一个整段的 `has_math: true` 当作每个表达式的证据。

`translation-review.json` 是自核台账。`validate_translation_review()` 检查实际文件哈希、来源和候选身份、清单单元是否全部被选中或明确排除、每块的必填目标、状态、逐处记录、术语扫描与检索、记录之间的引用及可解析定位。它不判断清单本身是否漏掉原 PDF 内容，不判断段内命题是否被穷举，不决定不适用理由是否成立，也不判断科学译义。源 PDF 的可见原文仍须实际阅读；文字层损坏时不会用文本检索结果否定可见页。空列表、术语命中、文件存在和执行者自述均不产生语义通过。实际过程与离线结果分别填写；没有可回读的过程证据时，不由正确结果反推核对动作已发生。

## 准备记录

实际翻译时，每完成一段就可以阅读来源、核对译句并保存这一段的明确决定；不必等全文完成才开始复核。下面的准备步骤用于绑定某一个实际候选版本。先写入本段的短调用，候选稳定后再编译完整台账，见[逐段短入口与增量保存](#逐段短入口与增量保存)。

按以下顺序准备，避免文件绑定刚生成就过期：

1. 在翻译前登记实际术语表指纹。
2. 完成来源单元清单。
3. 完成会写入清单的 `terminology_path`、`checks_path` 等工具指针。
4. 保存本次待复核的完整译稿版本。
5. 调用 `prepare_translation_review()`。
6. 逐块填写复核结果。
7. 保存最终 `terminology.json`。
8. 将其文件引用写入复核记录的 `terminology_record`。
9. 调用 `validate_translation_review()`，读取每条诊断的 `object_id`，只修正对应的实际缺项或错误绑定。
10. 回读修订后的记录并再次检查；未完成项继续保持原来的待核、失败或未决状态。
11. 按本文末表为每个单目标 ID 绑定具体记录，不用检查器结果代填通过。
12. 将最终复核记录作为 `translation_review_path` 显式交给交付清单生成器。

准备函数不修改来源清单，不添加 inventory 指针，不覆盖旧复核记录。来源清单、术语表或候选改变后保留旧记录，新建修订文件；按变化重新核对受影响目标。仅更新哈希不能使旧结论继续有效。后续排版改动行号时，重新绑定实际最终 Markdown 中的对应片段。

在工作目录的 JSON 请求文件中填写本次 `run.json` 对应的实际路径并调用 `prepare_translation_review.prepare_translation_review`。`unit_ids` 由本次完整来源盘点确定，不按关键词自动选段。完整 PDF 流程从已有 `run.json` 读取实际路径：初始化通常把清单和术语记录放在 `work/evidence/`，候选位置由本次配置指定，不从固定文件名推断。下面的运行记录路径、来源块 ID 和候选片段须替换为本次实际值：

```json
{
  "schema_version": 1,
  "operation": "prepare_translation_review.prepare_translation_review",
  "args": [
    "<实际 run.records.source_inventory>",
    "<实际 run.config.candidate_path>",
    "<实际 run.records.translation_review>"
  ],
  "kwargs": {
    "unit_ids": ["<source-a-paragraph-k>"],
    "glossary_path": "<实际 run.config.glossary_path 或 null>",
    "candidate_locations": {
      "<source-a-paragraph-k>": [
        {"quote": "<本次译稿中可唯一定位的完整原样片段>"}
      ]
    }
  }
}
```

在上述流程中，最终术语采用记录保存到 `terminology_path`，再用它的实际文件引用填写 `terminology_record`。若运行记录尚未登记所需术语记录，先按实际任务补齐计划和 inventory 指针，再固定清单生成复核骨架；不在另一目录另建同名文件代替。

独立翻译任务也可以直接调用这些函数，无需创建 `run.json`。在自己的调用脚本中明确设置 `scripts`、`work`、`inventory_path`、`candidate_path`、`review_path`、`glossary_path` 与 `terminology_path` 的实际值，再复用下面的操作；所有路径以本次真实文件和计划为准。

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
| `/scope/excluded_units` | 每条写 `id`、`applicability_condition`、`reason`、`evidence`；必须逐一覆盖未选择的清单单元。 |
| `/units/<id>/source_locations` | 源清单中该块的实际位置；block 定位保留原字段并展开真实元数据。 |
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

未选择的单元不能只从 `selected_units` 删掉。保留其 `unselected_inventory_units` 条目，并在 `excluded_units` 说明本次实际语种、非应译对象或局部修订授权条件，附真实定位证据。若仍属于本次应译范围，把它加入生成器的 `unit_ids`，为它建立待核记录。检查器不会按 `kind`、关键词或候选有没有中文自动排除单元。

## 逐段待办与范围计划

准备清单后先用 `translation_review_tasks.next_review_task` 查看实际待办。它按“范围 → 当前单元 → 全局记录”的顺序读取现有台账，当前单元未就绪时不会跳到后面的单元。它只显示已有目标，不判断语义，不填写状态，也不按对象类型自动排除内容。

在本次 JSON 请求文件里使用：

```json
{
  "schema_version": 1,
  "operation": "translation_review_tasks.next_review_task",
  "args": ["<实际 prepared_path>", []],
  "kwargs": {
    "scope_limit": 8
  }
}
```

| `stage` | 当前目标 |
|---|---|
| `scope` | 确认每个来源单元的处理范围 |
| `unit` | 完成当前单元的全部记录 |
| `global_records` | 处理报告指出的全局记录缺项 |
| `save_final_records` | 保存实际正式复核文件 |

`scope` 每次显示少量未说明范围的原单元及其实际源位置。仍需翻译的单元应在第一次逐段判断之前加入准备范围。实际不译的单元用 `writer.exclude(unit_id, condition, reason)` 各写自己的条件。再明确填写 `writer.scope(status, reason)`。类型标签、关键词或未被选中本身均不是排除依据。

逐一核对题名、小节标题、正文、题注、表头、注释和声明中的实际应译内容。短标题也建立自己的单元。短标题的中文译文也要复核，不能因为它字数少、原题另外保留或不是正文段落而跳过。书目等明确照录原文的对象，则按实际适用规则给出排除理由。

`unit` 给出完整原位置、实际候选片段、17个独立检查问题、10类扫描状态和14组逐处记录字段。原文片段与候选片段不会被静默截断；显示中省略的重复绑定仍在完整台账内。数值检查不能代替单位检查；用文字表达的时间尺度同样具有单位。完整性检查落实到实际命题，不能用概括性动作替代来源明确的动作。术语查询仍由实际读取和取舍产生。

按下一节短入口完成本段后，再依次校验当前单元记录完整性、保存新草稿并查看下一待办：

```json
{
  "schema_version": 1,
  "operation": "translation_review_tasks.require_unit_records",
  "args": ["<实际 prepared_path>", "<累计 decisions 数组>"],
  "kwargs": {
    "unit_id": "<当前 unit_id>"
  }
}
```

```json
{
  "schema_version": 1,
  "operation": "translation_review_writer.save_decision_draft",
  "args": ["<实际 prepared_path>", "<累计 decisions 数组>"],
  "kwargs": {
    "output_path": "<actual_new_checkpoint_path>"
  }
}
```

若本段仍有缺项，`IncompleteUnitRecordsError.result` 保存该段的完整实际诊断；继续处理本段。`unit_record_report(writer, unit_id)` 可只读检查任一已选单元。没有做完时也可以保存新草稿，但保存草稿不表示本段就绪。下一次用 `ReviewWriter.from_draft(actual_checkpoint_path)` 恢复原输入仍有效的决定，再查看待办。

只有 `unit` 阶段才读取 `task["unit_id"]`。本单元尚无候选定位时，先用 `writer.unit(unit_id, candidate_locations=actual_complete_candidate_locations)` 绑定实际完整范围，再填写检查；其他阶段按上表处理自己的目标。

每条实质命题使用自己的 `claims` 记录；只填 `completeness: passed` 而没有命题记录会被拒绝。各类实际不存在时分别说明该类的缺省条件，不能把所有空组统一改成不适用。一个单元的判断不传播到下一单元。

待办中的诊断显示有数量上限，`diagnostic_totals` 保留完整数量。显示上限不删除目标，也不影响原检查器判断。当前单元检查调用原样的最终校验器逻辑；完整就绪仍由正式文件的 `validate_translation_review()` 判断。

全部单元就绪后，工具用临时预览文件检查全局记录。它保留源文疑点、术语覆盖、整体状态等实际未决项。这些临时文件不是交付物。`save_final_records` 只表示可以保存正式记录。随后调用 `writer.save(new_review_path, terminology_path=new_terminology_path)` 保存新文件。检查这次保存返回的实际报告。将正式复核文件接入完整工作流。将正式术语文件接入完整工作流。工具不为任何技能验收 ID 自动填写通过。工具不认证译义正确。

## 逐段短入口与增量保存

`translation_review_writer.*` 入口延续原 `ReviewWriter` 短调用协议。先用上节的真实 `prepare_translation_review.prepare_translation_review` 建立台账，再以该路径和当前 `decisions` 数组逐步推进。来源身份和全部来源位置从准备台账读取；每段只绑定一次完整候选位置。之后短调用里的状态、理由和独立比较仍由实际复核者逐项填写，文件路径、哈希、重复 evidence 和术语总表指针由入口展开。17 个检查、10 类扫描和 14 组 occurrence 不变；没有调用的目标继续是 `pending`。

下面只是接口形状；变量须来自本段真实阅读与选择，不是可直接套用的结论。完整方法列表及增量保存方式见 [Rust 运行入口](../../literature-zotero2obsidian/references/rust-entrypoints.md#翻译复核)：

```json
[
  {
    "schema_version": 1,
    "operation": "translation_review_writer.ReviewWriter.unit",
    "args": ["<实际 prepared_path>", "<累计 decisions 数组>"],
    "kwargs": {"unit_id": "<unit_id>", "candidate_locations": "<actual_complete_candidate_locations>"}
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.select_source",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {"unit_id": "<unit_id>", "selector": {"quote": "<exact_source_excerpt>"}}
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.select_candidate",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {"unit_id": "<unit_id>", "selector": {"quote": "<exact_candidate_excerpt>"}}
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.UnitReview.occurrence",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {
      "unit_id": "<unit_id>",
      "kind": "claims",
      "occurrence_id": "<claim_id>",
      "source": "<select_source 返回的 selector>",
      "candidate": "<select_candidate 返回的 selector>",
      "status": "<claim_status>",
      "reason": "<claim_reason>"
    }
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.UnitReview.check",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {"unit_id": "<unit_id>", "name": "completeness", "status": "<completeness_status>", "reason": "<completeness_reason>"}
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.UnitReview.term",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {"unit_id": "<unit_id>", "term_id": "<term_id>", "original": "<original_term>", "category": "<primary_category>", "source": "<source selector>", "aliases": ["<actual_aliases>"]}
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.TermReview.search",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {"unit_id": "<unit_id>", "term_id": "<term_id>", "mode": "full_phrase", "query": "<actual_query>", "interpretation": "<query_interpretation>"}
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.TermReview.decide",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {"unit_id": "<unit_id>", "term_id": "<term_id>", "decision": "<term_decision>", "translation": "<chosen_translation>", "reason": "<contextual_reason>", "matches": ["<explicitly_chosen_hit_ids>"]}
  },
  {
    "schema_version": 1,
    "operation": "translation_review_writer.UnitReview.scan",
    "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
    "kwargs": {"unit_id": "<unit_id>", "category": "<primary_category>", "status": "<category_status>", "candidate_ids": ["<term_id>"], "reason": "<category_reason>"}
  }
]
```

上面每步按顺序单独保存并执行 `z2o invoke`（若无别名改用 `TermReview.search_status`），最后用 `translation_review_writer.save_decision_draft` 写入新草稿文件（如 `work/translation-decisions-paragraph-01.json`）；草稿不冒充完成台账。

`source` 应覆盖待比较的实际术语上下文；`original_term` 必须出现在所选片段里。同一个已注册候选可以被多个类别明确引用，不必为每一类复制候选；其主类别也必须实际包含该候选。`search()` 执行真实 `GlossaryIndex` 查询，保存原样行号、词条 ID、命中原文和调用方的解释；它记录 `searched`，不决定同义或采用。`decide()` 的 `matches` 只接受本候选真实查询返回的 ID，保存时会重跑查询核对。完整短语和词族不能用未查询的“不适用”代替。未收录术语的 `unlisted` 决定另外要求明确 `resolution_basis`；`not_a_term`、未决与不采用理由同样由复核者填写。

同一候选版本下继续：

1. 用 `ReviewWriter.from_draft(saved_draft_path)` 恢复已保存的短决定。入口重新核对准备台账及其实际来源、候选、清单与词表绑定。
2. 用 `next_review_task(writer)` 确认当前段。用 `writer.unit(unit_id, candidate_locations=...)` 完成该段自己的检查、术语扫描及逐次 occurrence，再调用 `require_unit_records()`；尚未完成时继续本段，不能把上一段的结论复制给下一段。
3. 完成当前批次后写另一个新草稿。也可以随时 `writer.save(new_partial_review_path, terminology_path=new_partial_terminology_path)`，保存可检查的部分台账并查看实际 `pending_ids`；保存成功不表示已经就绪。
4. 所有实际应译单元完成后，明确填写 `writer.source_issues_review(status, reason)`；范围和排除条件已经在逐段判断前完成。发现源文疑点时，用 `unit.issue(issue_id, source, description, candidate_readings, disposition, status, reason)` 保留各个读法与处理，不用总括结论代替疑点记录。
5. 明确调用 `translation_review_writer.ReviewWriter.terminology_review`（`status: "reviewed"`, `covered_units`, 可选 `no_unlisted_reason`）和 `translation_review_writer.ReviewWriter.review`（`status: "reviewed"`），再保存新的正式台账与术语文件：

```json
{
  "schema_version": 1,
  "operation": "translation_review_writer.save_review",
  "args": ["<实际 prepared_path>", "<累计 decisions 数组>"],
  "kwargs": {
    "output_path": "<实际 final_review_path>",
    "terminology_path": "<实际 final_terminology_path>"
  }
}
```

`save_review` 返回原有 `validate_translation_review` 的报告，不返回文件路径，也不替任何目标填写通过。已存在的输出文件会被拒绝，原准备台账不会被改写。完整流程中，先在实际 `run`／inventory 计划里登记本次新的复核与术语路径，再固定清单和准备骨架；不要用覆盖初始化台账的方式绕过保护。生成的术语总表沿用交付检查器的契约，采用项的 `glossary_locator` 是实际词表绝对路径加 `#L行号`，逐候选的查询与命中仍保存结构化定位。

**候选后续有改动时保留旧版本。** 可把已经完成的本段短调用保存在自己的工作目录 JSON 决定序列里，边译边记录；每次需要绑定结果时针对实际候选版本准备新台账。最终版本稳定后，在新台账里重新选择实际片段、核对受改动影响的判断，再重放已明确核对的短调用。`translation_review_writer.restore_decision_draft` 只恢复原绑定仍有效的草稿；它不换哈希、不跨版本迁移旧结论。旧候选追加了一段、行号变化、来源清单或词表变化，都会要求重新准备并明确处理，不能靠重新保存旧决定获得新版本结论。这样既可以从第一段开始实际复核，也不会把早期草稿的通过状态自动带到最终稿。

### 位置、重复片段与可见转录

`unit.source(quote, page=..., block_id=..., occurrence=...)` 从本单元的真实来源位置选择片段。重复文字必须用页、block、边界框或从 1 开始的明确 occurrence 选择；不知道属于哪个位置时保留待查。`unit.candidate(raw_quote, lines=[start, end], occurrence=...)` 选择实际 Markdown 源码。候选片段必须在本段已经绑定的位置内，同一行重复也必须明确选择；实际字符范围会写入定位。入口不从关键词推断哪段是本单元。

图内文字、表格或损坏文字层可使用本单元内的实际源页与边界框，不要求 PDF 文字层命中：

```json
{
  "schema_version": 1,
  "operation": "translation_review_writer.select_source",
  "args": ["<实际 prepared_path>", "<累计 decisions 数组>"],
  "kwargs": {
    "unit_id": "<实际 unit_id>",
    "selector": {
      "text": "<visible_transcription>",
      "page": "<actual_page>",
      "bbox": "<actual_box>",
      "observation_basis": "<actual_observation_basis>"
    }
  }
}
```

这会分别记录可见转录及调用方提供的观察依据，并检查真实页号、边界框和所属单元。它不会自动声明已经查看页图。若清单只有无边界框的片段、无法核定该图内位置属于本单元，先补足真实来源位置并重新准备，不能随意指向同页其他区域。

Rich Table 的译文可能位于 YAML 转义字符串中。选择实际源码片段，再明确给出用于比较的解释文字及依据；不要把解码后的文字当成原文件子串：

```json
{
  "schema_version": 1,
  "operation": "translation_review_writer.select_candidate",
  "args": ["<实际 prepared_path>", "<累计 decisions 数组>"],
  "kwargs": {
    "unit_id": "<实际 unit_id>",
    "selector": {
      "quote": "<actual_yaml_source_fragment>",
      "lines": "<actual_source_lines>",
      "comparison_text": "<explicit_decoded_cell_text>",
      "interpretation_basis": "<actual_decoding_or_rendering_basis>"
    }
  }
}
```

然后以 `UnitReview.occurrence` 记录 `table_cells`，并在 `comparison` 中传入单元格归属：

```json
{
  "schema_version": 1,
  "operation": "translation_review_writer.UnitReview.occurrence",
  "args": ["<实际 prepared_path>", "<上一步返回的 decisions>"],
  "kwargs": {
    "unit_id": "<实际 unit_id>",
    "kind": "table_cells",
    "occurrence_id": "<cell_id>",
    "source": "<source selector>",
    "candidate": "<select_candidate 返回的 selector>",
    "status": "<cell_status>",
    "reason": "<cell_reason>",
    "comparison": {
      "source_cell": {"table_id": "<source_table_id>", "row": "<source_row>", "column": "<source_column>"},
      "candidate_cell": {"table_id": "<candidate_table_id>", "row": "<candidate_row>", "column": "<candidate_column>"}
    }
  }
}
```

单元格文本从明确选择的比较文字填入，表身份、行列和判断仍单独提供。记录同时保留 `raw_text`、`comparison_text`、解释依据与原文件定位，是否解码正确仍需实际核对。

位置不存在、跨单元片段、缺少独立比较字段、未知类别、旧哈希或假词表命中会产生带 `code`、`object_id` 的 `ReviewWriterError`。按该具体目标修正。空 occurrence 组不会让对应检查自动通过，源码检查零错误也不会写入语义结论。`records_ready=True` 仍只表示原校验器认可记录结构与绑定；`source_fidelity`、`execution_truth` 保持 `not_assessed`，仍须完成本次实际逐段自核及最终独立验收。

## 展开清单中的 block 定位

`prepare_sources` 的合法来源定位可以只有 `page`、`basis`、`blocks`。`blocks` 是同一 `inventory.sources` 中该来源、该页 `pages[].blocks[]` 的真实 ID 列表，不是任意行号。准备函数自动调用 `expand_source_locations(unit, source)`：

1. 用 `unit.source` 选择同一来源。
2. 用 `evidence.page` 选择实际页索引。
3. 按原 `blocks` 顺序逐个查找 ID。
4. 保留原来的 `page`、`basis`、`blocks`。
5. 在 `block_locations` 保存每个 `block_id`、`page`、原始 `bbox` 和已有的原始文字 `quote`。
6. 缺少整块位置时，仅用所列 block 的实际 bbox 并集和原始文字生成 `bbox`／`quote`。

未知 ID、错页 ID、重复 ID、缺少实际 bbox 会报错，不会猜测替代块。没有 block 文字时只保留真实 bbox，不伪造 OCR。空 `blocks` 不能单独提供定位；已有独立 bbox／quote 的视觉对象可继续使用该实际定位。此展开不改变来源清单；准备函数随后核验实际 PDF 与清单 SHA-256。它不判断所选 block 的阅读顺序、语义或是否构成完整段落，来源盘点者仍须实际确认。

需要查看展开结果时，可直接调用 `prepare_translation_review.expand_source_locations`（再由人确认这些实际 block 是否覆盖了完整应译段）：

```json
{
  "schema_version": 1,
  "operation": "prepare_translation_review.expand_source_locations",
  "args": ["<source-inventory.json 中本次 unit 对象>", "<source-inventory.json 中对应 source 对象>"],
  "kwargs": {}
}
```

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
11. **核对每次数值。** 在 `occurrences.numbers` 保存该处原值与译稿中的对应值。
12. **核对每次单位。** 在 `occurrences.units` 保存该处单位所修饰的量。
13. **核对每次符号。** 在 `occurrences.symbols` 保存该处符号的对应关系。
14. **核对每个公式。** 在 `occurrences.formulas` 保存该处完整表达式的比较。
15. **核对每次引文标签。** 记录该次原显示标签；同号重复出现仍分别核对。
16. **核对每次引文身份。** 确认该次调用指向的原书目条目。
17. **核对引文支持范围。** 在 `occurrences.citation_claims` 比较该次引文所支持的命题。
18. **判断段落对应关系。** `checks.alignment` 只判断本源块与候选位置是否对应。
19. **判断信息完整性。** `checks.completeness` 逐一回查原命题是否保留；不能由“段落有中文”推导通过。
20. **回查本段待核项。** 已检查且没有该类对象的项目填写具体不适用条件；其余继续保留 `pending`、失败或未决。

命题清单按可独立遗漏的信息拆分，不按句号数量计数。例如虚构原句 `The cracked seal prevents immersion, although trained staff may still use the instrument indoors.` 包含以下各自可核对的信息：

| 一个核对目标 | 需要保留的信息 |
|---|---|
| 浸没限制的理由 | 密封件开裂造成限制。 |
| 被限制的操作 | 仪器不能浸没。 |
| 使用许可的强度 | 其余使用仍是许可，不是已发生的使用。 |
| 使用者范围 | 原句指定经过训练的人员。 |
| 使用地点范围 | 原句指定室内。 |

只译出主要结论会漏掉这些独立信息。核对时逐项找候选对应片段；没有对应片段的目标继续保持未完成，不能用整段位置代替。

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

## 自动取得检索线索

完整流程初始化会从实际术语表建立索引，并在 `run.json.records.glossary_candidates` 保存原始来源各文字块的命中行。该文件绑定初查时的清单、来源文件和术语表；后续登记来源单元不会回写这一初查记录。它不是最终术语决定或复核状态。

1. 按当前源块 ID 找到对应检索线索。
2. 阅读命中条目的原英文。
3. 阅读命中条目的中文解释。
4. 判断命中条目与本段概念的关系。
5. 按判断填写本段术语决定。
6. 继续查找自动线索中没有出现的专业术语。

完整短语匹配采用英文词边界。`shared_token_family` 只表示核心词相同，例如来源的 `radial spacing` 可能检索出术语表中的 `radial velocity`；这条线索有助于核对方向，但不能把整个“径向速度”直接替换到间距语境。程序没有判断同义关系、专业类别或译义，也不宣布已经穷举术语。

独立翻译任务可在自己的 JSON 请求文件中使用同一术语索引（分别按完整术语调用 `GlossaryIndex.search`，按原段调用 `GlossaryIndex.suggest` 取得词族线索）：

```json
{
  "schema_version": 1,
  "operation": "glossary_candidates.GlossaryIndex.search",
  "args": ["<实际 glossary_path>", "<actual_source_term>"],
  "kwargs": {}
}
```

```json
{
  "schema_version": 1,
  "operation": "glossary_candidates.GlossaryIndex.suggest",
  "args": ["<实际 glossary_path>", "<original_paragraph>"],
  "kwargs": {"max_family_entries": 8}
}
```

保存实际查询与命中行，包括零命中。索引返回的每条命中含真实行号和文件哈希，可供后续证据绑定复用；仍须自行填写取舍理由。自动线索的频率阈值只用于减少常见词的检索噪声，不改变任何术语扫描目标。

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

`glossary_matches` 使用与 `hits` 相同的 `entry_id`／`glossary_locator`／`excerpt` 结构。在 `decision_reason` 解释命中项是同一概念、相关词族还是不同义项；`adopted` 至少有一个实际命中。`terminology_pointer` 使用 `/adopted/<索引>` 或 `/unlisted/<索引>`，所指条目的原词、拟译和来源单元须与本候选一致。术语表中的一行与本文中的一次术语出现不是同一对象，两种定位都要保留。

所有类别扫描完成后才填写该块的 `checks.terminology`。`term_scan` 中已扫描类别使用 `scanned`，没有此类概念的类别使用 `not_applicable` 并写本段证据。未扫描仍为 `pending`。`covered_units` 只从这些实际完成记录中登记，不按候选全文汉字数或已有术语条数推导。

## 关系与表达式的逐处记录

`occurrences` 中每个实例有独立 `id`、`source_span`、`candidate_span`、`status`、`reason`、`evidence`。`source_span` 和 `candidate_span` 各保存实际原样片段及其定位；包含同一词的两句话必须有两条实例，不能只记一个词的全篇命中。

调用 `new_occurrence(id, kind, source_span=实际来源位置)` 可生成该类别的必填字段；所有比较值初始为 `null`，状态仍为 `pending`。每条已填写记录附真实证据，例如 `bind_review_evidence(source_path, {"page": 2, "quote": 原句})` 与 `bind_review_evidence(candidate_path, {"lines": [起行, 止行]})`。文件引用的哈希由函数读取实际字节，不手写占位哈希。候选单独 `quote` 必须唯一；重复文字改用明确行段，必要时带上原样片段进一步定位。

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
| `claims` | `source_claim`、`candidate_claim`：该次可独立遗漏的陈述。 |
| `directions` | `source_relation`、`candidate_relation`，各有 `subject`、`reference`、`relation`。 |
| `negations` | `source_scope`、`candidate_scope`：否定的谓语、对象或量词范围。 |
| `conditions` | `source_condition`、`candidate_condition`：条件与它限定的命题。 |
| `antecedents` | `source_candidates`、`candidate_candidates`：原文允许的指代对象集合。 |
| `modifiers` | `source_candidates`、`candidate_candidates`：原文允许的修饰对象集合。 |
| `strength` | `source_strength`、`candidate_strength`：可能性、因果或证据性质。 |
| `numbers` | `source_expression`、`candidate_expression`、`difference_type`：数值及表示差异。 |
| `units` | `source_expression`、`candidate_expression`、`source_quantity`、`candidate_quantity`。 |
| `symbols` | `source_expression`、`candidate_expression`、`source_definition`、`candidate_definition`。 |
| `formulas` | `source_expression`、`candidate_expression`、`source_structure`、`candidate_structure`。结构分别写 `denominators`、`radicals`、`exponents`、`brackets` 四个字符串列表；本式没有某结构时明确留空列表。 |
| `citation_labels` | `source_label`、`candidate_label`、`source_target`、`candidate_target`：可见标签及实际书目身份。 |
| `citation_claims` | `source_claim`、`candidate_claim`、`source_target`、`candidate_target`：被支持命题及该次目标。 |
| `table_cells` | `source_cell`、`candidate_cell`，各有 `table_id`、从 1 起算的 `row`／`column`、`text`。 |

否定与边界分开核对。虚构句 `The detector reports no stable trace beyond the guarded chamber.` 中，“没有稳定轨迹”是否定内容，“防护腔体之外”是其空间适用范围；只译成“腔体内可报告轨迹”没有明确保留原句的带外否定。虚构句 `The update is used only while the latch remains closed.` 要保留更新操作受锁扣状态约束的关系，不能删除 `only`。

原文歧义也属于要保留的信息。虚构句 `The operator told the assistant that she should recalibrate the sensor.` 未唯一决定 `she` 指谁；不要擅改成“操作员应……”或“助手应……”。先记录两种可能对象，再选择保留原指代空间的译法。虚构句 `She examined the specimen with a lens.` 中末尾短语的作用需要上下文；不能无依据强行固定修饰范围。读完实际相邻句仍不能消歧时，保留未决记录；必要说明放在独立译注，不混入作者结论。

数值、单位、符号和公式按每次出现定位。虚构表达式 `q = a/(b + c)` 的记录要覆盖完整分母 `b + c`，候选也定位到该完整式；不能只以变量 `q` 命中证明转录一致。段中有多个公式时分开编号，行内式不能被省略。数值表达式未改变时 `difference_type` 为 `unchanged`。Unicode 字形、显示月份或行内/显示排版变化可记录为 `representation_only`，但须给出等价理由；有数值或作用范围变化使用 `value_changed` 或 `structure_changed`，不确定则 `unresolved`。

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

有未决术语时，`unresolved_terms` 每条写 `unit_id`、`candidate_id`、`reason`，并与该候选及 `terminology.json` 的未决状态对应。有来源疑点时，`source_issues` 每条写 `id`、`unit_id`、`source_locator`、`description`、`candidate_readings`、`disposition` 及常规的 `status`／`reason`／`evidence`；未解决时不得改为已通过。没有来源疑点仍需填写 `source_issues_review` 的实际范围与依据。

## 检查记录并处理诊断

在同一工作流中读取实际文件并执行复核台账校验，不通过命令行拼接记录：

```json
{
  "schema_version": 1,
  "operation": "validate_translation_review.validate_translation_review",
  "args": [
    "<实际 review_path>",
    "<实际 inventory_path>",
    "<实际 candidate_path>"
  ],
  "kwargs": {}
}
```

`findings` 每项都有 `code`、`object_id`、`message`、`level`。找出它指向的单元或字段，回到实际来源和候选处理；不把诊断批量改成通过。文件缺失、JSON 无法解析、重复 JSON 键、嵌套类型错误也会返回诊断。

完整诊断已保存在文件中；工作时只展开当前来源块的项目，或先看前若干条。显示截断不改变检查范围，不删除尚未显示的诊断。按逐段操作顺序完成后重新运行，直至所有实际问题均有处理或如实保留为未完成。

| 诊断类别 | 操作 |
|---|---|
| `TRN_SCOPE_UNACCOUNTED`／`TRN_UNIT_MISSING` | 检查本次授权范围；增加应译块记录，或为确实不适用的块写具体条件与证据。 |
| `TRN_TARGET_MISSING` | 恢复骨架中的该一个目标；先保留 `pending`，再实际检查。 |
| `TRN_PENDING`／`TRN_UNRESOLVED`／`TRN_RECORDED_FAILURE` | 处理该条尚未完成或自报失败的问题；不能为了清空诊断改写状态。 |
| `TRN_HASH_CHANGED`／`TRN_BINDING_IDENTITY` | 找出文件版本差异；保留旧记录，对实际新版本重新核对受影响项，再新建绑定。 |
| `TRN_LOCATION_INVALID`／`TRN_SOURCE_LOCATIONS_CHANGED` | 回读实际页／行／JSON Pointer；恢复对应原块，不能换成仅存在但无关的位置。 |
| `TRN_OCCURRENCES_MISSING` | 为实际出现的关系或表达式逐处填写；若本段确实没有该类对象，写有证据的不适用理由。 |
| `TRN_TERM_QUERIES_MISSING`／`TRN_TERM_SEARCH_REQUIRED` | 完成并记录该词的具体检索；完整短语和词族两项都需要实际查询，无命中也记录。 |
| `TRN_TERM_MAPPING`／`TRN_TERMINOLOGY_CANDIDATE_MISSING` | 对齐该块术语候选与最终术语条目的原词、拟译、来源 ID 和精确指针。 |

`records_ready=True` 只说明记录通过结构、绑定和定位检查，且没有自报的待核、失败、阻断或未决项。它不说明这些申报结果为真。报告的 `source_fidelity` 与 `execution_truth` 始终为 `not_assessed`，`independent_review_required` 始终为 `true`。一份自报通过但把“蓝色”误译为“红色”的完整记录仍可能通过文件检查；独立对源审阅必须发现该错误。

失败、阻断和未决记录允许完整保存，但会使 `records_ready=False`。已实际检查过这些项目时，`review_status` 可以仍为 `reviewed`；不能据此推导交付合格。空 `events` 不阻断离线文件目标，程序也不因出现时间戳而认证执行动作。

## 完整单段示例

先阅读[Rust 运行入口](../../literature-zotero2obsidian/references/rust-entrypoints.md#翻译复核)，了解单段记录的完整检查目标、短调用流程及最终校验器的验证方式；填写真实任务时每一条理由均须来自本次实际来源与候选的比较。

示例分别给出 17 个检查目标的判断。每个不适用项都有对应的具体条件。10 个术语类别分别保存扫描结果。实际出现的命题、方向、否定、条件、指代、修饰、强度、数值和单位分别定位。三个术语候选保留真实查询，包括零命中、缩写命中和仅相关词族命中的区别。

成功运行时，`records_ready` 为 `true`；`source_fidelity` 和 `execution_truth` 仍为 `not_assessed`。这演示文件记录的闭合，不认证语义，也不将示例中的判断变成本次论文的默认结果。填写真实任务时只复用结构；每一条理由来自本次实际来源与候选的比较。

完整示例还保留最初的待核骨架，便于逐字段比较。候选变化或必填目标被删除后，原校验器会拒绝沿用完成结论。按当前来源块完成后，再处理下一块；不要以示例只含一块为由缩小整篇的应译范围。

## 一个事实对应多个单目标

若在同一位置已通过人工核对确认了真实来源事实（如方位关系或结构对照），应在 `checks.json` 中为对应条目分别引用该精确事实记录的 JSON Pointer。

同一已经实际核对的事实可以同时支持 `TRN-C68` 和 `TRN-A14`，两条目标分别记录自己的 ID、范围、理由和结果，并引用同一事实记录的精确 JSON Pointer。不要为复用证据重新编造一次动作；也不要把这两个目标的通过扩大到整节、整份技能或全部 `TRN-*`。示例的 `example-atomic-links.json` 只是两个目标的说明片段，不能代替完整 `checks.json`。

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
