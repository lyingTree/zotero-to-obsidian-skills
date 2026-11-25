# 完整候选的开始与收尾

本文保留 `complete_pdf_candidate_v1` 旧版工作目录的恢复协议。新建任务优先使用 [Rust 组装、规范化与验收](rust-entrypoints.md)，不再要求模型重填程序可完成的机械检查。局部修复仍使用对应子技能。恢复本旧版协议时，单独的 `validate_note` 或 `validate_delivery` 没有检查整套既有工作记录是否齐全，不能替代这里的收尾检查。

执行顺序是：实际来源材料 → 来源块与阅读顺序 → 完整内容片段 → 候选组装 → 对源核对与记录 → 最终交付。初始化及每次恢复都使用 `resume_workflow(run_path)`。尚有内容未写时，它先返回[逐块产出队列](draft-queue.md)的当前动作；`next_review_task()` 是已有译文的复核入口。

## 结束条件

本地文件记录属于本次候选交付。实际库身份、集合和 URI 未提供时可以如实保留未知；逐 ID 结果、译文核对、来源映射和链接映射则可以在本地完成。

1. 每个阶段完成后填写该阶段已经核对的验收 ID。
2. 收尾报告出现本地缺项时，继续处理该缺项。
3. `records_ready` 为 `false` 时，保留当前工作状态并继续下一批任务。
4. 当前来源能支持的本地缺项清零后，再写最终完成报告。

“未完成全部台账”不等于缺少外部环境；不能用这一表述提前结束仍可完成的任务。遇到实际读取失败或来源不可辨认时，按具体对象记录该阻断。不要把一个对象的阻断扩大到其余可完成对象。

## 开始：先生成清单

在本次工作目录保存 JSON 请求文件，先填写实际路径。以下文件名是示例，不表示实际来源已经读取。

```json
{
  "schema_version": 1,
  "operation": "note_pipeline.process_content",
  "args": [],
  "kwargs": {}
}
```

没有 SI 时从两个映射中同时去掉对应来源。中文原文确实无需翻译时，将 `translation_required` 设为 `False`，并在语言分流记录中保存依据。初始化会保存来源材料、来源哈希、全量验收骨架、产出队列及 `run.json`；`run.records.draft_queue` 指向该实际队列。它不会把任何内容判断填成通过。恢复任务时将已有 `run.json` 传给 `resume_workflow()`，不要重新初始化或覆盖旧记录。

PDF 来源含实际页图和文字块；DOCX 来源含 `native_blocks`、`media_calls`、`native_media` 与 `native_issues`。DOCX 没有固定页码，不向 PDF 解析器传入 DOCX，也不填写虚构的 `page` 或 `bbox`。段落和表格按 XML 顺序定位，媒体按实际 relationship 绑定。一个逻辑图可以对应多次媒体调用；媒体文件名、relationship 编号和图号不是同一概念。

先读取每件媒体调用的所属块及前后文字，再据来源题注登记图号。保留原始图像字节，逐幅查看内容。发现 `native_issues` 时处理该具体对象；不要把未支持对象静默排除，也不要让一件未决对象阻止其他可完成段落。结构来源映射见[交接约定](pipeline-contract.md)。

提供术语表时，初始化另存 `run.json.records.glossary_candidates` 指向的逐块检索线索。翻译每块前读取它的完整短语命中和核心词线索。词族行只提供原术语表的解释，不代表整个命中短语就是本次概念；按[术语检索步骤](../../literature-translate-zh/references/translation-review.md#自动取得检索线索)决定是否采用。原术语表保持不变。

## 中间阶段：按对象保存实际判断

`resume_workflow()` 为只读入口，返回 `phase`、分项计数和一个 `next_action`。`draft` 阶段直接展示当前页或当前来源单元；不会在正文尚未写出时先展示全量收尾诊断。全部片段就绪而候选尚不存在时，返回 `assemble`；候选存在后返回现有完整检查中的一个具体文件目标。每次实际保存后重新调用，直到当前阶段没有可继续的本地缺项。完整诊断仍由 `inspect_workflow()` 保存，不删减任何验收目标。

旧 `run.json` 没有队列指针时，恢复入口返回 `register_draft_queue`。先查看实际工作目录：存在旧队列时登记其真实路径，不存在时才从本次来源库存准备，再登记 `records.draft_queue`。入口不自动迁移、覆盖或清空旧队列。队列来源身份不符、源文件变化或片段被改写时保留实际异常并处理该文件，不能另建空队列掩盖问题。

先按[逐块产出队列](draft-queue.md)确认阅读顺序并写内容片段。所有实际来源块都要登记到明确单元，或逐块说明排除理由；跨页的同一段可以合并，来源块身份仍各自保留。每个单元保存完整译文、原有引文和对应对象。下一项有尚未写入的正文时，继续该正文；摘要、结论、补充材料已经写出不表示整篇内容完成。

候选组装前检查队列。队列未完成时读取其首个实际缺项，而不是反复运行末尾的几百条待核记录。队列完成后仍需核对段内是否遗漏信息；非空片段和全覆盖计数不证明翻译忠实。来源事实可保存在稳定的逐片段观察记录里，最终候选固定后引用其实际行号和哈希。

沿[分阶段路线](execution-plan.md)完成来源盘点、翻译、排版和链接。`run.json.records` 给出本次必需记录的计划路径；尚未生成的文件应在对应阶段补齐。译文记录按[逐段待办](../../literature-translate-zh/references/translation-review.md#逐段待办与范围计划)推进：先说明全部来源范围，每次闭合当前块，再处理下一块；不要把每段仅有一项判断的半成品留给最终全量收尾。

| 记录 | 在哪个阶段完成 | 核对目标 |
|---|---|---|
| `run.json.naming` | 来源题头核定后 | 拟用文件名有明确来源与变换依据 |
| `source-map.json` | 来源盘点、候选和交付位置均存在后 | 主文每个块能回到来源及实际候选 |
| `si-map.json` | SI 盘点、候选和交付位置均存在后 | SI 每个对象能回到独立来源及实际候选 |
| `link-map.json` | 链接组装后 | 每处调用能回到实际目标 |
| `translation-review.json` | 逐段自核后 | 每个译文源块有独立核对记录 |
| `checks.json` | 各阶段持续填写 | 每个验收 ID 有自己的判断 |

记录的字段沿用[交接约定](pipeline-contract.md)。同一个事实可以存一次，再由多个明确 ID 引用其精确位置；不能把整个技能视作一个已通过目标。译文复核使用[翻译短入口](../../literature-translate-zh/references/translation-review.md#逐段短入口与增量保存)。首次填写前阅读[短调用的完整单段示例](../../literature-translate-zh/references/compact_review_example.py)，理解各目标如何分别闭合；该示例的内容判断只适用于它自带的虚构材料。需要查看展开后全部字段时再读[原始完整示例](../../literature-translate-zh/references/complete_review_example.py)。

完整入口读取以下集合字段；集合可以用列表保存记录，也可以用对象 ID 到记录的映射保存。没有对应对象时保留空集合，并在该验收 ID 记录实际不适用依据：`source-map.json` 的 `blocks`、`objects`、`references`、`citations`；`si-map.json` 的 `sources`、`blocks`、`objects`；`link-map.json` 的 `anchors`、`internal_links`、`library_matches`、`unresolved`。集合存在只说明结构齐全，不证明内容已经核对。

来源单元和交付位置登记后，可调用 `write_maps(RUN_PATH, deliveries)`，由实际 inventory 和明确的交付定位生成这三份映射。返回报告会列出不能可靠派生的字段；补齐实际来源依据后再生成，不能填猜测值绕过诊断。该函数保留 `extracted`／`pending` 状态，既不声称已经看过来源，也不替代逐单项目标的实际核对。首次生成不覆盖旧文件；需要重新生成时在 `run.json.records` 指定新版本路径，保留原文件。

发现文字层与页图读法不同时，先用[冲突登记入口](pipeline-contract.md#一次登记文字层与页图冲突)把本次观察写入该来源单元，再建立最终库存快照和映射。`basis: visual` 只标明来源类型；单独写在执行报告或译文疑点中的解释不会自动进入 `source-map.json` 或 `si-map.json`。主文与每份补充材料分别检查自己的实际冲突记录。

在翻译前按[来源单元字段](delivery-checks.md)使用 `make_source_unit()` 登记对象和调用，能提前发现漏填的来源编号或调用文字。所有候选链接都要有来源显示文字；正文调用可复用对应单元的 `display_text`，邮箱、DOI 和图片嵌入等仍按各自实际来源登记。源页没有文字显示标签时，明确记录空字符串及原因，不从候选生成标签冒充源显示文字。

登记正文中的裸网址时，先查看 `source_url_inventory.scan_source_url_occurrences(inventory)` 的逐次线索，再对源页补齐实际单元。同一个网址重复出现时分别登记。该入口只找提取文字中的位置，不决定网址在可见源页中的实际写法。

某条链接的所属单元或源块仍不唯一时，在该单元的 `delivery.link_bindings` 为这一次链接补充 `candidate_locator`、`source_display_text` 和 `origin_block`。`candidate_locator` 取诊断中给出的实际位置；源字段取该来源页面。逐条补完报告中的缺项后再生成三份映射，不能把生成失败仅写进最终报告后结束。

三份映射的 `bindings` 分别绑定当前 `source_inventory`、`candidate` 及按来源 ID 登记的 `sources`。生成以后检查每个对象的源位置、目标位置和对应关系，再把实际比较结果写入相应对象记录及 `checks.json`。当前稿与旧稿的行号相同不代表证据仍然有效。

### 内容观察与文件映射分别记录

映射生成器保留的 `review_status: pending` 没有表达内容已经核对。来源和候选定位可以直接复用；实际比较结果由执行者按下列单一对象保存，随后供有关验收 ID 引用：

| 记录对象 | 本条记录的目标 | 最小实际定位 |
|---|---|---|
| 一个来源页 | 说明实际查看的页内容 | 来源 ID、物理页码、该页 PNG 文件绑定、实际观察文字 |
| 一个表格 | 说明该表源码核对结果 | 表单元 ID、候选表块行段、工作表 ID（若有） |
| 一个表格单元格 | 说明本格与来源的比较 | 表单元 ID、源网格坐标、候选行列 ID |
| 一条书目 | 说明该条身份字段比较结果 | 该来源的书目条目 ID、源页位置、候选锚 |
| 一次引文调用 | 说明本次调用的对应关系 | 调用 ID、来源句位置、候选调用位置、实际书目目标 |
| 一个未决问题 | 定位仍未核实的对象 | 实际单元 ID、源位置或候选位置、具体未核原因 |

这些记录可以存放在同一个工作 JSON 的独立对象中；每个对象各写自己的结果和理由。一次引文的判断不能自动覆盖同一书目的其他调用。没有文献库访问时，库匹配问题仍能定位到本篇具体书目和候选范围。图像查看记录直接引用实际页图，不能把读取文字层写成已经查看页面。

先完成所需的实际观察，再用 `check_records.bind_evidence()` 绑定已保存记录的精确 JSON pointer；同一事实可以供多个明确验收 ID 引用。不要引用一份仍在改写的 JSON 的旧哈希。没有对应对象的目标各自填写 `not_applicable` 与条件依据；没有真实提交任务时，提交和落库回读目标也逐 ID 记录该条件。

### 同一输入的映射复跑

三份映射稳定后，用相同 inventory、候选和 `deliveries` 实际再生成一次，保留原映射。以下入口把第二次生成写到新目录，并比较来源单元目标、锚和逐次链接的身份字段：

```json
{
  "schema_version": 1,
  "operation": "note_pipeline.process_content",
  "args": [],
  "kwargs": {}
}
```

查看报告的 `mapping_equal` 和每个 `differences` 位置，再为 `AC-A75` 写实际结果。报告绑定前后映射和本次输入，复跑前后的原候选与源文件必须相同。它只比较稳定映射字段，不复制原映射里的人工核对结果，也不替执行者填写验收状态。修订候选后产生的新映射不是同输入复跑。

`write_maps()` 与 `write_manifest()` 共用同一份 `deliveries`。其中 `target` 沿用[交付定位约定](delivery-checks.md)：行段片段按 UTF-8、LF 和原有行尾换行计算哈希，跨行 `quote` 同样用 LF 表示。映射中的 `candidate_locator` 则记录原始候选的 Unicode 字符位置与对应原始片段哈希；工具负责转换位置，不将这两种定位混用。整个候选仍绑定实际文件字节的哈希，解析换行不改写文件。

最终文件即使被用户指定为 `candidate.md`，也在 `run.json.naming` 单独保存[离线命名计划](../../literature-filename/SKILL.md)。指定交付路径不会自动产生文献的拟用名称，也不能据此宣称真实库冲突已经检查。

### 图件输出的调用顺序

此段调用适用于 PDF。DOCX 的内嵌媒体直接使用来源清单中 `native_media[].file` 指向的原始文件；按 `media_calls` 对照所属段落和题注，使用对应 `native_locator` 登记来源。不要为它们构造 PDF 页码、xref 或裁框。

先从完整源页核定图件范围。只有已确定整幅图就是该原始图片对象时，才走直接导出分支。复合图直接使用页面范围渲染。以下变量均取实际图件清单，不从示例猜页码或对象号。

```json
{
  "schema_version": 1,
  "operation": "note_pipeline.process_content",
  "args": [],
  "kwargs": {}
}
```

将返回记录保存到工作目录。渲染记录包含每边实际安全边距；页边限制导致的边距不足可从记录定位。随后按[图件规则](../../literature-fulltext-extract/references/figure-crop.md)分别核对四边。`pending` 记录不会自动转为通过。直接调用 `get_pixmap(clip=bbox)` 只保留原框，不会自动加入安全边距。

## 最终候选：先固定版本，再核对

候选内容完成后，调用 `bind_candidate(RUN_PATH)`。它把当前实际候选的路径与哈希写入验收骨架，不填写结果。随后对这一版本逐项核对。候选又有变化时，保留旧记录并重新核验受影响目标；该函数拒绝静默刷新已有候选绑定。

`pending_check_batch(CHECKS_PATH, limit=20)` 列出下一批尚未完成的文件目标。每条带原规则文字和文件位置，直接读取对应规则、查实际对象、保存证据并填写该 ID。完成当前批次后再取下一批。批次大小只影响展示，不改变目标集合。

可按当前阶段指定 `prefix="HDR"`、`prefix="BODY"` 或其他清单中实际存在的规则前缀。前缀只缩小这一批的展示；某个前缀返回空列表不表示整篇完成。最后用不带前缀的调用确认所有文件目标都已处理。不要到全文结束才首次填写整套台账。

`record_check_batch(CHECKS_PATH, results, evidence=EVIDENCE)` 接收明确的逐 ID 结果。`results` 中每条独立给出 `id`、`status`、`scope`、`evidence`、`reason`；不适用项另给实际 `applicability_condition`。共享证据通过 `check_records.bind_evidence()` 绑定已存在文件。函数不接受整技能通过、默认通过、按关键词推断不适用或改变目标类型。填写格式见[单项记录](acceptance.md#单项记录)。

### 用短行填写每个判断

日常填写可用同一模块的两个短入口：

```json
{
  "schema_version": 1,
  "operation": "note_pipeline.process_content",
  "args": [],
  "kwargs": {}
}
```

每行四列依次为 ID、状态、上下文名、该项目标的实际比较理由。`not_applicable` 行加第五列，单独写实际适用条件。示例变量须来自本次实际核对；不能把规则原句改成“已通过”作为比较理由。

上下文只复用对象范围和证据 ID，不带状态。比如两项都涉及同一组作者时，可以引用同一上下文，但仍各写一行判断。函数仅展开记录字段，调用原有 `record_check_batch()`；未列出的 ID 保持原状态。不能在上下文内放一个默认 `passed`，也不能把多个 ID 写成一行。

对一个来源块的核对内容可以先写在工作目录中的调用脚本，再在最终候选稳定后绑定实际位置；不要先把全部译文写完再开始回忆各段的核对结果。候选发生变化时，重新检查受影响内容。没有改变的源事实可复用其实际证据。

首次最终绑定前先完成会改动 inventory 的指针登记。翻译复核与图件复核按各自说明绑定稳定版本，避免相互引用仍在变化的文件造成哈希循环。使用 `write_manifest(RUN_PATH, deliveries)` 生成交付索引；它复用同一运行的实际路径，并绑定已完成的翻译复核文件。

## 收尾：按缺项继续，不在零语法错误处结束

把以下代码放入本次调用脚本。报告写在工作目录，不写入文献正文。

```json
{
  "schema_version": 1,
  "operation": "note_pipeline.process_content",
  "args": [],
  "kwargs": {}
}
```

写最终完成报告的脚本先调用下面这一行。若本地记录尚未完成，函数抛出 `IncompleteRecordsError`，并附 `report` 与有限长度的缺项摘要；回到这些实际目标继续工作，不吞掉异常后填写完成结论。

```json
{
  "schema_version": 1,
  "operation": "note_pipeline.process_content",
  "args": [],
  "kwargs": {}
}
```

这一入口使用与 `inspect_workflow()` 相同的验收范围。它不会补写任何状态，也不替代实际对源比较。

`resume_workflow()` 提供下一项行动，`require_completed_records()` 拒绝给未齐全的记录返回就绪结果；两者都不能拦截助手的聊天最终答复。若宿主要强制续跑，必须另行实现任务派发和终止控制，不能把这里的 程序异常称为宿主硬门。

`WORKFLOW_RECORD_MISSING` 表示需要补对应记录。`WORKFLOW_CHECK_PENDING` 表示需要完成该单项目标。`WORKFLOW_MANIFEST_BINDING` 表示报告引用了另一份文件或旧版本。翻译记录中的 `TRN_PENDING`、`TRN_UNRESOLVED` 等逐项提示同样保留其具体目标位置，可据此回到对应段落或术语记录。先处理实际缺项，再运行收尾检查。

完整报告可能很长；保存在文件中，控制台只看 `summary()` 与当前批次。摘要仅合并显示重复诊断，不删除待验目标，也不会把任何目标转为通过。处理顺序为：来源单元字段 → 对应交接映射 → 逐段译文记录 → 逐 ID 结果 → 最终索引绑定 → 完成检查。对一个阶段的每个剩余对象完成记录后再进入下一阶段。译文阶段使用 `next_review_task()` 与 `require_unit_records()`；它们只显示当前单元及其真实缺项，仍调用原有检查逻辑，不能替代最终实际文件的检查。

排版提示仍出现时，回到原规则判定。带单位的近似时长同样适用数学排版，例如“数十年”可写为 `$\text{数十}\,\text{年}$`；非精确不构成忽略提示的理由。真正的非问题项保存为 `presentation_review` 指向的 JSON。该文件的 `bindings.candidate` 用 `bind_file(CANDIDATE_PATH)` 绑定当前实际文件；`items` 每项写当前 `code`、`line`、`status: "not_an_issue"` 和具体 `reason`。候选变化后重新对照提示，不复用旧行号或旧理由。

第一次自动检查就保存该次报告与被检查的候选版本。每次修复后，为旧报告的每个发现保留一条处置：原发现的 code 和位置、原文件绑定、实际处理理由、修改后文件绑定与复核定位。修复项用 `fixed` 描述实际修正；确属非问题项用 `not_an_issue` 解释依据。将这些历史处置与当前候选的非问题项分开保存。最终零提示只能说明当前扫描没有发现，不能替代已出现问题的处置记录。

`records_ready` 只表示当前检查所需记录与源码条件已满足。`source_fidelity` 始终为 `not_assessed`，程序不认证译义、原文身份、图件完整性或记录者声明的真实性。执行者完成自己的实际对源核对后，才据各项证据报告文件结果。独立评审是复核，不承担补做执行者尚能完成的文件目标。
