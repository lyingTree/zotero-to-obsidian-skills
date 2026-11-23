# 逐篇验收清单

本清单验收 Markdown 文件及其引用资源。用户提供参考 Markdown 时逐项核对其内容和格式要求；文献事实仍回查原始来源。应用界面、点击交互、阅读视图、插件显示和原生缓存均不属于验收步骤。

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
| `blocked` | 写明实际阻断原因，例如本次必需的来源文件无法读取。 |
| `not_applicable` | 写明实际 `applicability_condition` 和理由，不能按关键词自动判定。 |
| `passed` / `failed` | 引用至少一处已存在、哈希匹配、位置有效的实际证据；记录者仍须对该单项结论负责。 |

本流程的 `requirement_type` 使用 `execution_process` 或 `offline_outcome`。未分类时保留 `unclassified`，不得填已决结论。分类来自调用脚本逐 ID 的明确映射，或规则行末的 `check-type` 声明；脚本不根据规则关键词替人判断。

过程结论需要 `execution_trace`，其 `observation` 包含 `observed: true`、`basis: direct_event`、观察者及带时区的实际时间。观察元数据只是可追查的声明，脚本不能认证观察确实发生。最终文件检查使用 `offline_result` 或 `source_location` 证据。

没有对应对象时，逐 ID 填 `not_applicable`，说明具体条件。例如来源没有视频、没有既有锚点或本次不写库。存在对象而未核对时保留 `not_verified` 并继续核对；不能把这两种情形统一写为阻断。

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
    inventory_path=WORK_DIR.joinpath("source-inventory.json"),
)
report = validate_check_records(CHECKS_PATH)
```

`SOURCE_FILES` 是来源 ID 到实际 `Path` 的映射；多个候选可使用 `candidates` 映射代替 `candidate_path`。`scopes` 和类型映射可分阶段填写；没有提供的范围不会自动补全，也没有任何检查被自动置为通过。可选的 `progress(current, total, stage)` 回调可接入进度条。`inventory_path` 只回写 `checks_path` 指针，不把仍在改写的 inventory 自身哈希写入台账。主交付校验器通过该指针绑定台账文件并核对来源、候选版本。

`bind_file(path)` 返回实际路径与哈希；`bind_evidence(path, locator, evidence_type=..., observation=...)` 创建可复用的证据项，均不改变检查状态。不要把仍在改写的 inventory、manifest 或 `checks.json` 自身用作哈希绑定证据；引用稳定的源页、对象记录或已固定的检查报告。来源、候选或规则改变后保留旧版本记录，重新核验受影响项；不得仅刷新哈希而沿用旧通过结论。

`validate_check_records(path)` 返回带 `findings` 的报告对象，支持 `to_dict()`。缺 ID、重复或未知 ID、空范围、无效位置、文件变化、缺证据结论和缺原因均逐项诊断。`structurally_consistent` 只表示这些结构与绑定检查无错误；`recorded_checks_complete` 只表示记录已填写为已决状态。`offline_records_ready` 另检查所有文件层目标是否已有已决记录且没有报告失败；未分类 ID 也使此门槛为 false。`offline_unresolved_ids` 列出需要继续处理的文件目标。

上述布尔值都不自动证明译义正确。检查器的 `source_fidelity: not_assessed` 表示程序没有授予语义结论，执行者仍须亲自对源核对，再依据实际证据填写单项结果。不能因为自己不是独立评审者，就把能核对的文件内容全部留给后来者。

### 完成文件层验收的操作

1. 读取本次台账中当前技能的各个验收目标。
2. 为每个目标选择实际对象范围。
3. 对照该目标所需的源文件或参考 Markdown。
4. 将实际比较结果写入稳定的证据文件。
5. 为该 ID 引用证据的精确位置。
6. 依据该项目标的比较结果填写状态。
7. 对 `offline_unresolved_ids` 中每个 ID 继续处理。
8. 对 `unclassified_ids` 中每个 ID 明确证据边界。

`scopes_from_deliveries(inventory_path, candidate_path, deliveries, unit_ids=[...])` 可复制显式选中对象的源位置和交付位置，只生成范围，不生成证据或通过结论。例如两张表的源码检查要列出两个表单元；不得用“正文链接”这一整篇范围代替每张表的位置。每条规则仍单独填结果，可复用同一份逐表报告中的相关位置。

该辅助函数检查候选定位是否存在，并检查来源定位的结构与声明页数边界；它不重新读取源 PDF。来源定位真实性由 `validate_delivery` 和实际对源核对负责，来源覆盖完整性与语义仍须逐项比较。

执行者自核完成后，独立评审可再对同一版本复核。主交付检查发现 `CHECK_OFFLINE_INCOMPLETE` 或 `CHECK_BOUNDARY_UNCLASSIFIED` 时，返回对应目标继续处理；不能因其他结构检查零错误而宣称全文完成。

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
- [AC-C24] 文件层验收仍有 `not_verified` 时不宣称文件验收完成。
- [AC-C25] 文件层验收仍有 `blocked` 时不宣称文件验收完成。
- [AC-C26] 文件层验收仍有 `failed` 时不宣称文件验收通过。
- [AC-C27] 验收记录仍有 `unclassified` 时不宣称文件验收完成。

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

- [ ] [AC-A01] 范围内的关键内容阻断项已解决。 <!-- check-type: offline_outcome -->
- [ ] [AC-A02] 条目身份阻断项已解决。 <!-- check-type: offline_outcome -->
- [ ] [AC-A03] 文献版本阻断项已解决。 <!-- check-type: offline_outcome -->
- [ ] [AC-A04] 预检记录早于内容处理记录。 <!-- check-type: execution_process -->
- [ ] [AC-A05] 工作根位于 vault 外。 <!-- check-type: offline_outcome -->
- [ ] [AC-A06] 备份根位于 vault 外。 <!-- check-type: offline_outcome -->
- [ ] [AC-A07] 各来源的对象 ID 没有冲突。 <!-- check-type: offline_outcome -->
- [ ] [AC-A08] 各书目的锚点 ID 没有冲突。 <!-- check-type: offline_outcome -->
- [ ] [AC-A09] 未知年份处理沿用已核实的命名映射。 <!-- check-type: offline_outcome -->
- [ ] [AC-A10] 资源冲突处理沿用已核实的路径映射。 <!-- check-type: offline_outcome -->
- [ ] [AC-A11] final.md 与最后候选的内容差异仅包含已登记工作标记的移除及最终资源路径的替换。 <!-- check-type: offline_outcome -->
- [ ] [AC-A12] 最终链接组装后的语法有效。 <!-- check-type: offline_outcome -->
- [ ] [AC-A13] 资源路径替换后的引用语法有效。 <!-- check-type: offline_outcome -->
- [ ] [AC-A14] 工作标记清理后的相邻块边界有效。 <!-- check-type: offline_outcome -->
- [ ] [AC-A15] 每幅范围内 PDF 图像具有[裁剪视觉验收](../../literature-fulltext-extract/references/figure-crop.md)的逐项记录。 <!-- check-type: offline_outcome -->
- [ ] [AC-A16] 最终嵌入图片的哈希与已验资源一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A17] 补充材料位于主文全部内容之后。 <!-- check-type: offline_outcome -->
- [ ] [AC-A18] 「Zotero原条目」位于补充材料之后；无 SI 时位于主文之后。 <!-- check-type: offline_outcome -->
- [ ] [AC-A19] 书目标题采用实际节序号。 <!-- check-type: offline_outcome -->
- [ ] [AC-A20] 补充书目处于补充材料章节内。 <!-- check-type: offline_outcome -->
- [ ] [AC-A21] 新建主文书目锚点采用 `^ref-N`。 <!-- check-type: offline_outcome -->
- [ ] [AC-A22] 新建单份 SI 书目锚点采用 `^sref-N`。 <!-- check-type: offline_outcome -->
- [ ] [AC-A23] 新建多份 SI 书目锚点采用 `^sref-src<source_token>-N`。 <!-- check-type: offline_outcome -->
- [ ] [AC-A24] 既有列表补锚沿用已核实且插件支持的命名空间。 <!-- check-type: offline_outcome -->
- [ ] [AC-A25] 既有锚点 ID 与输入基线一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A26] 既有链接目标与输入基线一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A27] 旧 ID 的插件兼容性问题已单列 warning。 <!-- check-type: offline_outcome -->
- [ ] [AC-A28] 旧 ID 的数字后缀与显示序号差异已单列 warning。 <!-- check-type: offline_outcome -->
- [ ] [AC-A29] 范围内缺失的书目锚点已按来源补齐。 <!-- check-type: offline_outcome -->
- [ ] [AC-A30] 范围内重复定义的锚点已解决。 <!-- check-type: offline_outcome -->
- [ ] [AC-A31] 范围内实际断链已解决。 <!-- check-type: offline_outcome -->
- [ ] [AC-A32] 范围内书目作用域错配已解决。 <!-- check-type: offline_outcome -->
- [ ] [AC-A33] Rich Table 的成对 `---` 外包裹可按模板格式解析。 <!-- check-type: offline_outcome -->
- [ ] [AC-A34] v2 单表或 v3 的每张工作表均为 `locked: true`。 <!-- check-type: offline_outcome -->
- [ ] [AC-A35] v2 单表或 v3 的每张工作表均为 `theme: grid`。 <!-- check-type: offline_outcome -->
- [ ] [AC-A36] 每张工作表的 columns 非空。 <!-- check-type: offline_outcome -->
- [ ] [AC-A37] 每张工作表的 rows 非空。 <!-- check-type: offline_outcome -->
- [ ] [AC-A38] 每张工作表的题注链接均可解析。 <!-- check-type: offline_outcome -->
- [ ] [AC-A39] 每张工作表的表注链接均可解析。 <!-- check-type: offline_outcome -->
- [ ] [AC-A40] 每张工作表的列名链接均可解析。 <!-- check-type: offline_outcome -->
- [ ] [AC-A41] 每张工作表的单元格链接均可解析。 <!-- check-type: offline_outcome -->
- [ ] [AC-A42] 工作簿顶层状态没有代替工作表检查记录。 <!-- check-type: offline_outcome -->
- [ ] [AC-A43] 本地资源位于所属文献的附件三级目录。 <!-- check-type: offline_outcome -->
- [ ] [AC-A44] 本地资源引用可按从库根起算的路径解析。 <!-- check-type: offline_outcome -->
- [ ] [AC-A45] 本地资源哈希与资源清单一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A46] 补充视频的原生嵌入有对应验收记录。 <!-- check-type: offline_outcome -->
- [ ] [AC-A48] 补充视频的转码结果有对应验收记录。 <!-- check-type: offline_outcome -->
- [ ] [AC-A49] 待提交哈希已记录。 <!-- check-type: offline_outcome -->
- [ ] [AC-A115] 范围内来源页均进入来源清单。 <!-- check-type: offline_outcome -->
- [ ] [AC-A116] 范围内来源单元均有交付位置。 <!-- check-type: offline_outcome -->
- [ ] [AC-A117] 被排除的来源块均有具体理由。 <!-- check-type: offline_outcome -->
- [ ] [AC-A118] 候选中的本地资源在实际解析根下存在。 <!-- check-type: offline_outcome -->
- [ ] [AC-A119] 图内真实表格有对应的可编辑结构。 <!-- check-type: offline_outcome -->
- [ ] [AC-A120] 每条自动排版提示有处置依据。 <!-- check-type: offline_outcome -->

## 文件写入与回读
- [AC-C16] 不要求写库时将提交检查记为不适用。
- [AC-C17] 不要求写库时将落库回读检查记为不适用。

- [ ] [AC-A50] commit 对应实际落库证据。 <!-- check-type: offline_outcome -->
- [ ] [AC-A51] 图片位于所属题注上方。 <!-- check-type: offline_outcome -->
- [ ] [AC-A52] 图锚位于所属题注后方。 <!-- check-type: offline_outcome -->
- [ ] [AC-A55] 最终稿公式前间距符合正文规则。 <!-- check-type: offline_outcome -->
- [ ] [AC-A56] 最终稿公式闭合行与其锚点相邻。 <!-- check-type: offline_outcome -->
- [ ] [AC-A57] 最终稿公式块后间距符合正文规则。 <!-- check-type: offline_outcome -->
- [ ] [AC-A61] 书目标题的同级节序号连续。 <!-- check-type: offline_outcome -->
- [ ] [AC-A62] 首条书目紧接书目标题。 <!-- check-type: offline_outcome -->
- [ ] [AC-A63] 改名后的书目章节链接落点正确。 <!-- check-type: offline_outcome -->
- [ ] [AC-A64] 「Zotero原条目」标题只出现一次。 <!-- check-type: offline_outcome -->
- [ ] [AC-A65] 原条目链接写在最后一个 H2 内。 <!-- check-type: offline_outcome -->
- [ ] [AC-A66] 原条目标题行是文件最后的非空内容。 <!-- check-type: offline_outcome -->
- [ ] [AC-A68] 原条目链接的显示文字正确。 <!-- check-type: offline_outcome -->
- [ ] [AC-A69] 原条目链接的目标 URI 正确。 <!-- check-type: offline_outcome -->
- [ ] [AC-A75] 复跑结果与已有稳定映射一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A76] 作者—年份引文保留原引文制。 <!-- check-type: offline_outcome -->
- [ ] [AC-A77] 公式数学内容与输入一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A78] 书目页码与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A79] 原刊脚注内容完整。 <!-- check-type: offline_outcome -->
- [ ] [AC-A80] 正文脚注上标与引用块编号配对。 <!-- check-type: offline_outcome -->
- [ ] [AC-A81] 表注位于所属表的 footer。 <!-- check-type: offline_outcome -->
- [ ] [AC-A82] 表块外没有重复表注。 <!-- check-type: offline_outcome -->

## 结论

- [ ] [AC-A93] report.md 单独报告内容覆盖结果。 <!-- check-type: offline_outcome -->
- [ ] [AC-A94] report.md 单独报告结构检查结果。 <!-- check-type: offline_outcome -->
- [ ] [AC-A95] report.md 单独报告写入结果。 <!-- check-type: offline_outcome -->
- [ ] [AC-A96] report.md 单独报告回读结果。 <!-- check-type: offline_outcome -->
- [ ] [AC-A98] report.md 单独报告遗留问题。 <!-- check-type: offline_outcome -->
- [ ] [AC-A99] 静态源码结论具有对应证据。 <!-- check-type: offline_outcome -->
- [ ] [AC-A100] 原文内容结论具有对应证据。 <!-- check-type: offline_outcome -->
- [ ] [AC-A101] CLI 结论具有对应证据。 <!-- check-type: offline_outcome -->
- [ ] [AC-A105] 整篇完成结论没有仅依据静态通过。 <!-- check-type: offline_outcome -->
- [ ] [AC-A106] 翻译通过结论没有仅依据静态通过。 <!-- check-type: offline_outcome -->
- [ ] [AC-A109] 内容阻断项标为“异常待处理”。 <!-- check-type: offline_outcome -->
- [ ] [AC-A110] 明确要求保存的不完整稿标为 partial。 <!-- check-type: offline_outcome -->
- [ ] [AC-A111] 工作稿状态与对应证据一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A112] 单项通过状态与对应证据一致。 <!-- check-type: offline_outcome -->
- [ ] [AC-A113] 整篇完成状态与对应证据一致。 <!-- check-type: offline_outcome -->
