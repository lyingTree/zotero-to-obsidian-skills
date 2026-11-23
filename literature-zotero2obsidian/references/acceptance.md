# 逐篇验收清单

## 记录方法

每个检查 ID 单独记录一个结果。结果取 `passed`、`failed`、`blocked`、`not_verified` 或 `not_applicable`。

使用 [`check_records.py`](../scripts/check_records.py) 保存 `checks.json`。脚本从实际 `skills_root` 下各子技能的 `SKILL.md` 和 `references/*.md` 提取声明；代码示例、注释和行内引用不算新规则。台账中两个集合用途不同：

- `catalog` 保存全部原子约束和验收 ID，以及规则正文、文件位置和实际哈希。
- `checks` 默认包含所有 `A` 编号验收 ID，包括 references 中的验收项。`rule_selection="all"` 可追踪全部约束与验收；`additional_ids` 可在默认验收集合上加入明确的约束 ID。这两种方式都不能删除默认验收项。

同一 ID 只有一条记录。一个记录的 `scope` 可以列出多页、多段、多张表或多个单元格，但不能把多个 ID 合成一个结果。可按 `skill` 分组填写，或让明确列出的 ID 复用同一证据文件；没有“整项技能通过”操作。执行台账不替代独立评审对全部适用约束和验收规则的核对。

### 单项记录

每条 `checks` 记录包含 `scope`、`status`、`evidence`、`reason`。`scope` 是实际覆盖对象的列表，每个对象写 `object_id` 和 `locator`；例如某来源的第几页、候选的哪个段落或某表的哪些单元格。未分配的范围保留空列表，校验会报告缺项，不用“全文已验”占位。

`evidence` 引用共享证据注册表中的 ID。每个证据项绑定已存在文件的 `path`、实际 `sha256`、`evidence_type` 和 `locator`。支持的定位方式是文本行区间、PDF 页区间、JSON Pointer、图像像素区域或完整文件；脚本检查位置确实存在。多个检查可以引用同一份稳定报告的不同位置，无须复制其正文。

| 状态 | 记录要求 |
|---|---|
| `not_verified` | 默认状态；写明尚未核验的原因。未观察到的执行过程保持此状态。 |
| `blocked` | 写明实际阻断原因，例如真实应用环境不可用。 |
| `not_applicable` | 写明实际 `applicability_condition` 和理由，不能按关键词自动判定。 |
| `passed` / `failed` | 引用至少一处已存在、哈希匹配、位置有效的实际证据；记录者仍须对该单项结论负责。 |

`requirement_type` 明确区分 `execution_process`、`offline_outcome` 和 `live_environment`。未分类时保留 `unclassified`，不得填已决结论。分类来自调用脚本逐 ID 的明确映射，或规则行末的 `check-type` 声明；脚本不根据规则关键词替人判断。

过程结论需要 `execution_trace`，其 `observation` 包含 `observed: true`、`basis: direct_event`、观察者及带时区的实际时间。真实环境结论需要 `environment_observation`，并写环境身份。`environment_kind` 明确选择 `actual_ui`、`native_cache` 或 `actual_cli`，证据的 `basis` 必须一致。阅读视图或交互项目使用 `actual_ui`；原生缓存、CLI、静态文件和离屏结果不能替它通过。观察元数据只是可追查的声明，脚本不能认证观察确实发生。

每个界面验收 ID 都保留自己的记录。没有实际界面时，逐 ID 写 `blocked` 或 `not_verified` 及原因；一句“界面待验”只能作为报告摘要。有范围和原因的待验记录是合法记录，不因缺少界面环境阻止完成离线候选。

### 调用与版本绑定

在工作目录的 Python 调用脚本中设置路径和明确的映射，调用公共函数；不需要命令行参数：

```python
from pathlib import Path
from check_records import prepare_check_records, validate_check_records

CHECKS_PATH = prepare_check_records(
    SKILLS_ROOT,
    WORK_DIR.joinpath("checks.json"),
    sources=SOURCE_FILES,
    candidate_path=CANDIDATE_PATH,
    scopes=SCOPES_BY_ID,
    requirement_types=TYPES_BY_ID,
    environment_kinds=ENVIRONMENT_KINDS_BY_ID,
    inventory_path=WORK_DIR.joinpath("source-inventory.json"),
)
report = validate_check_records(CHECKS_PATH)
```

`SOURCE_FILES` 是来源 ID 到实际 `Path` 的映射；多个候选可使用 `candidates` 映射代替 `candidate_path`。`scopes`、类型和环境映射可分阶段填写；没有提供的范围不会自动补全，也没有任何检查被自动置为通过。可选的 `progress(current, total, stage)` 回调可接入进度条。`inventory_path` 只回写 `checks_path` 指针，不把仍在改写的 inventory 自身哈希写入台账。主交付校验器通过该指针绑定台账文件并核对来源、候选版本。

`bind_file(path)` 返回实际路径与哈希；`bind_evidence(path, locator, evidence_type=..., observation=...)` 创建可复用的证据项，均不改变检查状态。不要把仍在改写的 inventory、manifest 或 `checks.json` 自身用作哈希绑定证据；引用稳定的源页、对象记录或已固定的检查报告。来源、候选或规则改变后保留旧版本记录，重新核验受影响项；不得仅刷新哈希而沿用旧通过结论。

`validate_check_records(path)` 返回带 `findings` 的报告对象，支持 `to_dict()`。缺 ID、重复或未知 ID、空范围、无效位置、文件变化、缺证据结论和缺原因均逐项诊断。`structurally_consistent` 只表示这些结构与绑定检查无错误；`recorded_checks_complete` 只表示记录已填写为已决状态。两者都不证明内容、过程或界面真实通过。报告始终保留 `source_fidelity`、`execution_truth`、`live_behavior` 为 `not_assessed`，仍须独立对源和观察真实环境。

- [AC-C01] 为每个检查 ID 记录适用范围。
- [AC-C02] 为每个检查 ID 记录证据位置。
- [AC-C03] 对未执行的检查记录原因。
- [AC-C04] 对不适用的检查记录原因。
- [AC-C05] 仅复用与当前输入版本对应的检查记录。
- [AC-C06] 仅复用与当前产物版本对应的检查记录。
- [AC-C07] 对缺少证据的适用检查补验。
- [AC-C08] 抽查记录明确列出已检查的页、块或对象。
- [AC-C09] 宣称逐字核验时提供逐块证据。
- [AC-C10] [静态校验](automated-validation.md)仅报告实际覆盖的规则。
- [AC-C11] 将扫描发现的范围外既有问题单独登记。
- [AC-C12] 不因全文件扫描而静默修改范围外内容。
- [AC-C18] 完整生成任务逐页核对内容覆盖。
- [AC-C19] 完整生成任务逐条核对来源书目。
- [AC-C20] 完整生成任务逐次核对文内引文目标。
- [AC-C21] 完整生成任务逐个核对数学表达式。
- [AC-C22] 完整生成任务逐格核对表格内容。
- [AC-C23] 完整生成任务逐幅核对图件。

## 子技能检查入口

逐条引用下列技能的适用验收 ID；不以“某技能整体通过”代替各 ID 的结果。

| 前缀 | 检查入口 |
|---|---|
| ZOT | [条目信息](../../literature-inspect-zotero/SKILL.md#验收标准) |
| EXT | [全文提取](../../literature-fulltext-extract/SKILL.md#验收标准) |
| TRN | [翻译](../../literature-translate-zh/SKILL.md#验收标准) |
| SI | [补充材料](../../literature-supplement-materials/SKILL.md#验收标准) |
| NAME | [文件名](../../literature-filename/SKILL.md#验收标准) |
| WRITE | [目录与提交](../../literature-write2obsidian/SKILL.md#验收标准) |
| PROP | [属性](../../literature-note-properties/SKILL.md#验收标准) |
| HDR | [题头](../../literature-header-format/SKILL.md#验收标准) |
| BODY | [正文排版](../../literature-body-format/SKILL.md#验收标准) |
| LINK | [篇内链接](../../literature-internal-links/SKILL.md#验收标准) |
| LIB | [库内出链](../../literature-library-links/SKILL.md#验收标准) |

## 候选稿

- [ ] [AC-A01] 范围内的关键内容阻断项已解决。
- [ ] [AC-A02] 条目身份阻断项已解决。
- [ ] [AC-A03] 文献版本阻断项已解决。
- [ ] [AC-A04] 预检记录早于内容处理记录。
- [ ] [AC-A05] 工作根位于 vault 外。
- [ ] [AC-A06] 备份根位于 vault 外。
- [ ] [AC-A07] 各来源的对象 ID 没有冲突。
- [ ] [AC-A08] 各书目的锚点 ID 没有冲突。
- [ ] [AC-A09] 未知年份处理沿用已核实的命名映射。
- [ ] [AC-A10] 资源冲突处理沿用已核实的路径映射。
- [ ] [AC-A11] final.md 与最后候选的内容差异仅包含已登记工作标记的移除及最终资源路径的替换。
- [ ] [AC-A12] 最终链接组装后的语法有效。
- [ ] [AC-A13] 资源路径替换后的引用语法有效。
- [ ] [AC-A14] 工作标记清理后的相邻块边界有效。
- [ ] [AC-A15] 每幅范围内 PDF 图像具有[裁剪视觉验收](../../literature-fulltext-extract/references/figure-crop.md)的逐项记录。
- [ ] [AC-A16] 最终嵌入图片的哈希与已验资源一致。
- [ ] [AC-A17] 补充材料位于主文全部内容之后。
- [ ] [AC-A18] 「Zotero原条目」位于补充材料之后；无 SI 时位于主文之后。
- [ ] [AC-A19] 书目标题采用实际节序号。
- [ ] [AC-A20] 补充书目处于补充材料章节内。
- [ ] [AC-A21] 新建主文书目锚点采用 `^ref-N`。
- [ ] [AC-A22] 新建单份 SI 书目锚点采用 `^sref-N`。
- [ ] [AC-A23] 新建多份 SI 书目锚点采用 `^sref-src<source_token>-N`。
- [ ] [AC-A24] 既有列表补锚沿用已核实且插件支持的命名空间。
- [ ] [AC-A25] 既有锚点 ID 与输入基线一致。
- [ ] [AC-A26] 既有链接目标与输入基线一致。
- [ ] [AC-A27] 旧 ID 的插件兼容性问题已单列 warning。
- [ ] [AC-A28] 旧 ID 的数字后缀与显示序号差异已单列 warning。
- [ ] [AC-A29] 范围内缺失的书目锚点已按来源补齐。
- [ ] [AC-A30] 范围内重复定义的锚点已解决。
- [ ] [AC-A31] 范围内实际断链已解决。
- [ ] [AC-A32] 范围内书目作用域错配已解决。
- [ ] [AC-A33] Rich Table 的成对 `---` 外包裹可按模板格式解析。
- [ ] [AC-A34] v2 单表或 v3 的每张工作表均为 `locked: true`。
- [ ] [AC-A35] v2 单表或 v3 的每张工作表均为 `theme: grid`。
- [ ] [AC-A36] 每张工作表的 columns 非空。
- [ ] [AC-A37] 每张工作表的 rows 非空。
- [ ] [AC-A38] 每张工作表的题注链接均可解析。
- [ ] [AC-A39] 每张工作表的表注链接均可解析。
- [ ] [AC-A40] 每张工作表的列名链接均可解析。
- [ ] [AC-A41] 每张工作表的单元格链接均可解析。
- [ ] [AC-A42] 工作簿顶层状态没有代替工作表检查记录。
- [ ] [AC-A43] 本地资源位于所属文献的附件三级目录。
- [ ] [AC-A44] 本地资源引用可按从库根起算的路径解析。
- [ ] [AC-A45] 本地资源哈希与资源清单一致。
- [ ] [AC-A46] 补充视频的原生嵌入有对应验收记录。
- [ ] [AC-A47] 补充视频的播放兼容性有对应验收记录。
- [ ] [AC-A48] 补充视频的转码结果有对应验收记录。
- [ ] [AC-A49] 待提交哈希已记录。
- [ ] [AC-A115] 范围内来源页均进入来源清单。
- [ ] [AC-A116] 范围内来源单元均有交付位置。
- [ ] [AC-A117] 被排除的来源块均有具体理由。
- [ ] [AC-A118] 候选中的本地资源在实际解析根下存在。
- [ ] [AC-A119] 图内真实表格有对应的可编辑结构。
- [ ] [AC-A120] 每条自动排版提示有处置依据。

## 写入与界面

- [AC-C13] 默认由用户审查实际界面。
- [AC-C14] 未取得界面确认时记录“界面未验收”。
- [AC-C15] 不默认调用 computer use。
- [AC-C16] 不要求写库时将提交检查记为不适用。
- [AC-C17] 不要求写库时将落库回读检查记为不适用。

- [ ] [AC-A50] commit 对应实际落库证据。
- [ ] [AC-A51] 图片位于所属题注上方。
- [ ] [AC-A52] 图锚位于所属题注后方。
- [ ] [AC-A53] 原生缓存中的图块范围覆盖图片。
- [ ] [AC-A114] 原生缓存中的图块范围覆盖所属题注。
- [ ] [AC-A54] 原生缓存中的表锚范围覆盖完整表格。
- [ ] [AC-A55] 最终稿公式前间距符合正文规则。
- [ ] [AC-A56] 最终稿公式闭合行与其锚点相邻。
- [ ] [AC-A57] 最终稿公式块后间距符合正文规则。
- [ ] [AC-A58] 原生缓存中的公式锚点落在对应公式块。
- [ ] [AC-A59] 原生缓存将主文书目标题识别为 H2。
- [ ] [AC-A60] 原生缓存将补充书目标题识别为 H3。
- [ ] [AC-A61] 书目标题的同级节序号连续。
- [ ] [AC-A62] 首条书目紧接书目标题。
- [ ] [AC-A63] 改名后的书目章节链接落点正确。
- [ ] [AC-A64] 「Zotero原条目」标题只出现一次。
- [ ] [AC-A65] 原条目链接写在最后一个 H2 内。
- [ ] [AC-A66] 原条目标题行是文件最后的非空内容。
- [ ] [AC-A67] 原生缓存将原条目标题识别为 H2。
- [ ] [AC-A68] 原条目链接的显示文字正确。
- [ ] [AC-A69] 原条目链接的目标 URI 正确。
- [ ] [AC-A70] 其他编号标题的实际层级符合正文结构。
- [ ] [AC-A71] 章节链接的实际落点正确。
- [ ] [AC-A72] 数字引文上标实际呈现正确。
- [ ] [AC-A73] 引文范围的连接线实际呈现正确。
- [ ] [AC-A74] 引文点击后落到对应书目。
- [ ] [AC-A75] 复跑结果与已有稳定映射一致。
- [ ] [AC-A76] 作者—年份引文保留原引文制。
- [ ] [AC-A77] 公式数学内容与输入一致。
- [ ] [AC-A78] 书目页码与来源一致。
- [ ] [AC-A79] 原刊脚注内容完整。
- [ ] [AC-A80] 正文脚注上标与引用块编号配对。
- [ ] [AC-A81] 表注位于所属表的 footer。
- [ ] [AC-A82] 表块外没有重复表注。
- [ ] [AC-A83] 阅读视图中的 H1 呈现正确。
- [ ] [AC-A84] 阅读视图中的邮箱链接呈现正确。
- [ ] [AC-A85] 阅读视图中的作者单位上标呈现正确。
- [ ] [AC-A86] 阅读视图中的多单位分行呈现正确。
- [ ] [AC-A87] 阅读视图中的引文标点呈现正确。
- [ ] [AC-A88] 阅读视图中的行内数学呈现正确。
- [ ] [AC-A89] Rich Table 每张工作表的锁定状态实际生效。
- [ ] [AC-A90] Rich Table 每张工作表的主题实际生效。
- [ ] [AC-A91] Rich Table 每张工作表中的链接实际可用。
- [ ] [AC-A92] 本次未核验的界面项目已按各自验收 ID 登记待验记录。

## 结论

- [ ] [AC-A93] report.md 单独报告内容覆盖结果。
- [ ] [AC-A94] report.md 单独报告结构检查结果。
- [ ] [AC-A95] report.md 单独报告写入结果。
- [ ] [AC-A96] report.md 单独报告回读结果。
- [ ] [AC-A97] report.md 单独报告界面验收结果。
- [ ] [AC-A98] report.md 单独报告遗留问题。
- [ ] [AC-A99] 静态源码结论具有对应证据。
- [ ] [AC-A100] 原文内容结论具有对应证据。
- [ ] [AC-A101] CLI 结论具有对应证据。
- [ ] [AC-A102] 原生缓存结论具有对应证据。
- [ ] [AC-A103] 离屏渲染结论具有对应证据。
- [ ] [AC-A104] 阅读视图结论具有对应证据。
- [ ] [AC-A105] 整篇完成结论没有仅依据静态通过。
- [ ] [AC-A106] 翻译通过结论没有仅依据静态通过。
- [ ] [AC-A107] 界面通过结论没有仅依据静态通过。
- [ ] [AC-A108] 未完成的界面检查标为“待验收”。
- [ ] [AC-A109] 内容阻断项标为“异常待处理”。
- [ ] [AC-A110] 明确要求保存的不完整稿标为 partial。
- [ ] [AC-A111] 工作稿状态与对应证据一致。
- [ ] [AC-A112] 单项通过状态与对应证据一致。
- [ ] [AC-A113] 整篇完成状态与对应证据一致。
