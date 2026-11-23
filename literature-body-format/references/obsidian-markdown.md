# Obsidian Markdown 检查表

## 范围索引

本表定位本次任务中实际出现的语法；具体判定使用下方独立 ID。

适用载体：正文、题头、题注、译注、脚注、书目、补充材料、链接显示文字及数据格式定义为 Markdown 的字段。直接读取文件，按对应语法解析后核对；结构化数据先反序列化，再核对字段中的 Markdown。

重点边界：行首、行尾、空行、缩进变化、紧邻中英文或标点、跨行定界符、嵌套及相邻块。

| 解析范围 | 语法对象 |
| --- | --- |
| 段落与块结构 | 空行、软/硬换行、缩进、ATX/Setext 标题、分隔线、有序/无序/任务列表、引用及嵌套 callout |
| 行内格式 | `*`、`_`、`**`、`__`、`~~`、`==`（含颜色标记）、反引号、反斜杠、HTML 实体 |
| 链接、标签及嵌入 | Markdown 行内/引用式链接、自动链接、`[[…]]`、`![[…]]`、`![…](…)`、标题与块引用、别名与媒体尺寸、`#标签/子标签`、URI |
| 注释、脚注及 HTML | `%%` 行内/块注释、`<!-- … -->`、`[^id]`、`^[…]`、HTML 标签及属性 |
| 代码、公式及图表 | 行内代码、围栏或缩进代码块、`$`、`$$`、LaTeX、Mermaid、GFM 表格 |
| 属性与扩展数据 | 起始 YAML、Rich Table、其他实际出现的数据围栏、内联标记及可嵌入格式 |

## 约束

每条约束按其 ID 单独记录执行证据。

- [MD-C01] 按原文和笔记意图区分可见字面文本、预期格式与结构化内容。
- [MD-C02] 仅转义所在解析层会误读的字面定界符。
- [MD-C03] 采用当前解析层能够识别的最小转义写法。
- [MD-C04] 普通文本标识符中的字面 `_` 统一转义。
- [MD-C05] 表格内的字面竖线使用 `\|` 或 `&#124;`。
- [MD-C06] Markdown 表格内的链接别名分隔符按表格层转义。
- [MD-C07] Markdown 表格内的嵌入尺寸分隔符按表格层转义。
- [MD-C08] 代码示例使用足够长的反引号包围。
- [MD-C09] 不确定的语法先用对应格式的文件解析器验证。
- [MD-C10] 保留有意使用的语法。
- [MD-C11] 处理链接显示文字时保留链接目标。
- [MD-C12] 处理链接显示文字时保留路径。
- [MD-C13] 处理链接显示文字时保留查询参数。
- [MD-C14] 处理链接显示文字时保留片段。
- [MD-C15] 处理链接显示文字时保留锚点 ID。
- [MD-C16] 代码内容使用代码层的转义规则。
- [MD-C17] 数学内容使用数学层的转义规则。
- [MD-C18] HTML 内容使用 HTML 层的转义规则。
- [MD-C19] YAML 内容使用 YAML 层的转义规则。
- [MD-C20] 扩展数据按对应数据格式转义。
- [MD-C21] 禁止用全局替换处理定界符。
- [MD-C22] 嵌套解析前确定每层接收的实际字符串。
- [MD-C23] 结构化内容中的普通数据照录。
- [MD-C24] 仅对格式定义为 Markdown 的字段应用 Markdown 层转义。
- [MD-C25] 重新解析序列化后的字段值。
- [MD-C28] 源码检查记录问题位置。
- [MD-C29] 源码检查记录原显示文字。
- [MD-C30] 源码检查记录预计结构。
- [MD-C31] 源码检查记录受保护内容。
- [MD-C32] 对修订稿复跑同一处理。
- [MD-C33] 能复现误触发时保留修复前证据。
- [MD-C34] 能复现误触发时保留修复后证据。
- [MD-C35] 适用时复核行首误触发情形。
- [MD-C36] 适用时复核跨行定界符情形。
- [MD-C37] 适用时复核同段多标识符情形。
- [MD-C38] 适用时复核紧邻中文的情形。
- [MD-C39] 仅对实际覆盖的部分判定通过。
- [MD-C40] 清单外的语法须查对应格式文档。
- [MD-C43] 新增语法样例沿用本清单的验收标准。

### 转义示例

- 标识符紧邻中文：`356-WTs\_E及356-WTs\_W`。
- 字面标签：`\#字面标签`。
- 行首数字与点号：`1\. 开头的文字`。
- 行首数字与右括号：`1\) 开头的文字`。
- 字面注释定界符：`\%\%说明\%\%`。
- 表格内字面竖线：`\|` 或 `&#124;`。

## 验收标准

每项验收按其 ID 单独记录判定结果。

- [ ] [MD-A01] 段落与块结构的块类型符合意图。 <!-- check-type: offline_outcome -->
- [ ] [MD-A02] 段落与块结构的层级符合意图。 <!-- check-type: offline_outcome -->
- [ ] [MD-A03] 段落与块结构的边界符合意图。 <!-- check-type: offline_outcome -->
- [ ] [MD-A04] 软换行或硬换行的解析结果符合意图。 <!-- check-type: offline_outcome -->
- [ ] [MD-A06] 数字后的 `.` 没有误触发列表。 <!-- check-type: offline_outcome -->
- [ ] [MD-A07] 数字后的 `)` 没有误触发列表。 <!-- check-type: offline_outcome -->
- [ ] [MD-A08] 下划线行按其预期语义解析。 <!-- check-type: offline_outcome -->
- [ ] [MD-A09] `---` 按其所在解析环境的预期语义解析。 <!-- check-type: offline_outcome -->
- [ ] [MD-A10] 文件解析后的行内字面符号完整。 <!-- check-type: offline_outcome -->
- [ ] [MD-A11] 文件解析后的行内文字没有多余反斜杠。 <!-- check-type: offline_outcome -->
- [ ] [MD-A12] 预期行内格式的作用范围准确。 <!-- check-type: offline_outcome -->
- [ ] [MD-A13] 行内格式的相邻正文文字没有被定界符吞并。 <!-- check-type: offline_outcome -->
- [ ] [MD-A14] 行内格式的相邻文字没有被误加格式。 <!-- check-type: offline_outcome -->
- [ ] [MD-A15] 链接显示文字已单独核对。 <!-- check-type: offline_outcome -->
- [ ] [MD-A16] 链接源码中的目标已单独核对。 <!-- check-type: offline_outcome -->
- [ ] [MD-A17] 邻接标点位于链接之外。 <!-- check-type: offline_outcome -->
- [ ] [MD-A23] 注释定界符正确配对。 <!-- check-type: offline_outcome -->
- [ ] [MD-A24] 注释隐藏范围正确。 <!-- check-type: offline_outcome -->
- [ ] [MD-A25] 脚注引用与定义对应。 <!-- check-type: offline_outcome -->
- [ ] [MD-A26] 脚注续行归属正确。 <!-- check-type: offline_outcome -->
- [ ] [MD-A27] HTML 块与 Markdown 的解析边界正确。 <!-- check-type: offline_outcome -->
- [ ] [MD-A29] 代码和公式等结构的定界符正确配对。 <!-- check-type: offline_outcome -->
- [ ] [MD-A30] 定界符没有吞并后续文本。 <!-- check-type: offline_outcome -->
- [ ] [MD-A31] 代码内部内容保持原意。 <!-- check-type: offline_outcome -->
- [ ] [MD-A32] 数学上标保持原意。 <!-- check-type: offline_outcome -->
- [ ] [MD-A33] 数学下标保持原意。 <!-- check-type: offline_outcome -->
- [ ] [MD-A34] 反斜杠保持所在解析层中的原意。 <!-- check-type: offline_outcome -->
- [ ] [MD-A39] 属性与扩展数据的类型经文件解析器核对。 <!-- check-type: offline_outcome -->
- [ ] [MD-A40] 属性与扩展数据的值经文件解析器核对。 <!-- check-type: offline_outcome -->
- [ ] [MD-A41] 属性与扩展数据标记的边界经文件解析器核对。 <!-- check-type: offline_outcome -->
- [ ] [MD-A42] 普通数据未经非预期改写。 <!-- check-type: offline_outcome -->
- [ ] [MD-A43] 序列化后重新解析的结构化字段值符合预期。 <!-- check-type: offline_outcome -->
- [ ] [MD-A44] 同一处理复跑后没有新增源码差异。 <!-- check-type: execution_process -->
- [ ] [MD-A45] 改动处及相邻结构的正文文字完整。 <!-- check-type: offline_outcome -->
- [ ] [MD-A46] 改动处及相邻结构的内容顺序正确。 <!-- check-type: offline_outcome -->
- [ ] [MD-A47] 改动处及相邻结构的格式起点正确。 <!-- check-type: offline_outcome -->
- [ ] [MD-A48] 改动处及相邻结构的格式终点正确。 <!-- check-type: offline_outcome -->
- [ ] [MD-A49] 改动处及相邻结构的对象完整。 <!-- check-type: offline_outcome -->
- [ ] [MD-A50] 配对的另一端定界符仍按预期解析。 <!-- check-type: offline_outcome -->
- [ ] [MD-A51] 源码检查证据有独立记录。 <!-- check-type: offline_outcome -->
- [ ] [MD-A56] 通过结论没有超出实际覆盖范围。 <!-- check-type: offline_outcome -->

## 参考

- [基础格式](https://obsidian.md/help/syntax) · [进阶格式](https://obsidian.md/help/advanced-syntax) · [Obsidian Flavored Markdown](https://obsidian.md/help/obsidian-flavored-markdown) · [CommonMark](https://spec.commonmark.org/) · [GFM](https://github.github.com/gfm/)
