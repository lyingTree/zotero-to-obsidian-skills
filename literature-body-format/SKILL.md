---
name: literature-body-format
description: 规范 Obsidian 文献正文的标题层级、间距、加粗、段落、图表公式及参考文献编号。保留内容语义和既有锚点，不承担翻译或引用目标匹配。
---

# 规范全文排版

使用已安装的[独立二进制运行器](../literature-zotero2obsidian/references/binary-runtime.md)时，原 Python API 的任务脚本由主技能 bin/z2o 的 run-script 执行；已注册的确定性操作可用 invoke JSON 请求文件。无需系统 Python 或 pip；开发环境才直接运行源码。不得用模型手工替代程序验收。

## 必要输入

- 当前候选稿、模板、对象清单与资源规划。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

机械排版继续使用既有 normalize_note、lint_presentation、validate_note 与 accept_note；[原子文件检查](../literature-zotero2obsidian/references/atomic-stage-checks.md) 区分直接计算与仍需来源语义复核的记录，禁止零提示即全篇通过。

优先按 [Python 组装、规范化与验收](../literature-zotero2obsidian/references/deterministic-pipeline.md)执行：结构化内容由 `render_note()` 序列化，既有候选由 `normalize_note()` 规范化，再由 `accept_note()` 读取实际文件。统一调用 `process_content()` / `process_markdown()` 可同时取得新候选、可重放变换、来源映射和逐环节报告。模型处理译义与异常判断，不手拼已由程序管理的空行、量值包装和对象块。

1. 依原文章序整理候选结构。
2. 按图清单逐幅放置原图。
3. 按表清单逐表构建 Rich Table，包含图内数据表。
4. 按公式清单逐式排版。
5. 直接读取 Markdown 文件，按书目清单逐条核对参考文献。
6. 按本页约束扫描量值与链接格式。
7. 向第 9 步交接受结构变化影响的调用。

书目 ID 的兼容性按第 9 步的语法规则静态核对，保留对应 warning；不以插件运行作为验收条件。

Rich Table 使用 `literature-zotero2obsidian/scripts/format_helpers.py` 的 `build_rich_table_block(table_data, anchor=actual_anchor)` 生成完整数据块。`table_data` 来自已逐格核对的源表；显式提供 v2/v3、每张表的 `locked=True`、`theme="grid"`、完整 `columns` 与 `rows`。函数保留单元格、公式字符串、合并关系和锚 ID，生成块状 YAML 后读回比较；不要改用 `json.dumps()` 输出 flow 数组。表题中的“表N”保留纯文本，正文调用再链接到该表锚。

每幅逻辑图使用[图块组装入口](references/figure-blocks.md)生成图片、图下题注和图锚，包含无编号图及图形摘要。源图只有类别标签时，题注保留该标签，不编造图号或说明。图上方的标题不能替代图下题注；没有正文调用也不省略图锚。

统一入口已调用 `validate_note()` 与 `lint_presentation()` 并区分机械失败与待核提示，不必重复手工记录相同检查。单独处理旧候选时仍可直接调用这两个只读函数。列、行样式和题注自链的诊断分别处理。前置材料位置提示须回到来源核对：原文摘要前的亮点、图形摘要仍置于摘要前；摘要后直接进入正文。普通正文讨论“研究亮点”不据此移动。

处理 `FIGURE_CAPTION_MISSING`、`FIGURE_ANCHOR_MISSING` 和图块间距提示时，先确认实际资源确属文献图。处理 `TRANSLATOR_NOTE_INLINE` 时保留原文及译注内容，将真正的译注单独写为 `> 译注：…`；普通正文讨论“译注”这个词不自动改写。

## 约束

每条约束按其 ID 单独记录执行证据。验收对象为 Markdown 源码、解析后的结构化字段及引用的资源文件。

- [BODY-C01] 沿用第 7 步生成的题头。
- [BODY-C02] 保留唯一的原题 H1。
- [BODY-C03] 原文有摘要时使用独立一行的 `## 摘要` 标题。
- [BODY-C04] 摘要节位于题头之后。
- [BODY-C256] 摘要节位于正文之前。
- [BODY-C05] 摘要首段紧接摘要标题下一行。
- [BODY-C06] 保留摘要完整内容。
- [BODY-C07] 保留摘要原有分段。
- [BODY-C08] 移除摘要段首残留的旧摘要标签。
- [BODY-C09] 原文没有摘要时省略摘要节。
- [BODY-C10] 摘要后直接接正文。
- [BODY-C11] 正文首个编号 H2 从 `1` 开始。
- [BODY-C12] 参与编号的标题按 `1`、`1.1`、`1.1.1` 逐级连续编号，包含附录、参考文献和补充材料。
- [BODY-C13] 摘要标题不参与编号。
- [BODY-C14] 「Zotero原条目」标题不参与编号。
- [BODY-C15] 保留原文章节层级。
- [BODY-C16] 保留原文章节顺序。
- [BODY-C17] 记录新旧标题映射。
- [BODY-C18] 题注按模板加粗。
- [BODY-C19] 关键词只保存在 YAML 属性中。
- [BODY-C20] 仅移除已确认重复的目录。
- [BODY-C21] 仅移除已确认重复的封面出版说明。
- [BODY-C22] 仅移除已确认的人工页码标记。
- [BODY-C23] 为每处删除记录来源依据。
- [BODY-C24] 保留有效脚注。
- [BODY-C25] 保留有效页码引用。
- [BODY-C26] 保留资助信息。
- [BODY-C27] 保留致谢。
- [BODY-C28] 保留附录。
- [BODY-C29] 保留原刊正文脚注编号。
- [BODY-C30] 保留原刊正文脚注调用位置。
- [BODY-C31] 保留完整脚注注文。
- [BODY-C32] 正文脚注调用使用 `<sup>1</sup>` 格式。
- [BODY-C33] 脚注注文就近另起引用块。
- [BODY-C34] 脚注引用块首行使用 `> <sup>1</sup> 注文` 格式。
- [BODY-C35] 脚注引用块续行加 `> `。
- [BODY-C36] 仅转换已核实的脚注标记。
- [BODY-C37] 脚注处理不得改动数学上标。
- [BODY-C38] 脚注处理不得改动型号。
- [BODY-C39] 已有 Obsidian 脚注链接保留跳转能力。
- [BODY-C40] 译注独立写为 `> 译注：…`。
- [BODY-C41] 按 [Obsidian Markdown 检查表](references/obsidian-markdown.md)确定字面文本与预期语法的解析环境。
- [BODY-C42] Markdown 扫描覆盖本次修改范围及相邻块。
- [BODY-C43] Markdown 扫描涵盖行首、跨行、嵌套及紧邻中英文和标点的边界组合。
- [BODY-C44] 逐处修正误触发语法的定界符。
- [BODY-C45] 普通文本标识符中的字面 `_` 写作 `\_`。
- [BODY-C46] 转义处理保持幂等。
- [BODY-C47] 转义处理保留预期格式。
- [BODY-C48] 转义处理保留链接目标。
- [BODY-C49] 转义处理保留锚点。
- [BODY-C50] 转义处理保留结构化数据。
- [BODY-C51] 结构化字段按对应数据格式转义。
- [BODY-C52] 扫描可见文本中的裸网址，包括 `www.` 形式。
- [BODY-C53] 建立网址链接前核对目标。
- [BODY-C54] 核实后的裸网址写为 `[原显示文字](<完整URL>)`。
- [BODY-C55] 链接边界不得包含周围括号、标点或相邻正文。
- [BODY-C56] 非 DOI 解析链接保留 URL 的原协议。
- [BODY-C57] 保留 URL 的原路径。
- [BODY-C58] 保留 URL 的查询参数。
- [BODY-C59] 保留 URL 的片段。
- [BODY-C60] 仅有域名时核实或注明采用的协议。
- [BODY-C61] 保留边界正确的已有网址链接。
- [BODY-C62] 网址转换跳过代码。
- [BODY-C63] 网址转换跳过公式。
- [BODY-C64] 网址转换跳过 YAML 属性。
- [BODY-C65] 网址转换跳过资源路径。
- [BODY-C66] 可见正文段落之间恰留一空行。
- [BODY-C67] 各级标题前留一空行。
- [BODY-C68] 标题后的普通正文块紧接标题下一行。
- [BODY-C69] 删除中文与数字、拉丁字母或行内数学之间的多余空格。
- [BODY-C70] 保留英文词间空格。
- [BODY-C71] 保留数值与拉丁单位之间的空格。
- [BODY-C72] 保留语法所需空格。
- [BODY-C73] 空白修改只作用于已定位的目标。
- [BODY-C74] 空白修改保留结构缩进。
- [BODY-C75] 空白修改保留脚注结构。
- [BODY-C76] 空白修改保留结构化数据块。
- [BODY-C77] 空白修改保留代码结构。
- [BODY-C78] 显示公式单独成块。
- [BODY-C79] 显示公式与前面的引导句、标题或其他块之间恰留一空行。
- [BODY-C80] 已有公式锚位于闭合 `$$` 的紧接下一行。
- [BODY-C81] 已有公式锚顶格书写。
- [BODY-C82] 已有公式锚独占一行。
- [BODY-C83] 显示公式块与后续内容之间恰留一空行；有锚时从锚后计算。
- [BODY-C84] 原图与中文题注就近排列。
- [BODY-C85] 新增或复用的 PDF 图像均须符合[裁剪与验收规则](../literature-fulltext-extract/references/figure-crop.md)。
- [BODY-C86] 不得用整页截图代替 PDF 图件。
- [BODY-C87] 图内文字保持原样。
- [BODY-C88] 图内色标保持原样。
- [BODY-C89] 不另作非表格类图内文字的转录。
- [BODY-C90] 不添加图内内容解释。
- [BODY-C91] 正文图和补充图均先嵌入图片。
- [BODY-C92] 图片行尾使用两个空格硬换行。
- [BODY-C93] 图片题注紧接图片的下一行。
- [BODY-C94] 图片 alt 只使用简短图号或留空。
- [BODY-C95] 正文图片题注使用 `**图N：** 题注正文` 格式。
- [BODY-C96] 补充图片题注使用 `**图SN：** 题注正文` 格式。
- [BODY-C97] 题注只作直译。
- [BODY-C98] 原文面板问题的说明放在锚点之后。
- [BODY-C99] 原文面板问题的说明使用独立一句事实性译注。
- [BODY-C100] 行内图表调用保留正文形式。
- [BODY-C101] 表格转换前核对原文版面。
- [BODY-C102] 仅原文存在明确行列对应关系的数据表时转为 Rich Table，包括图内数据表。
- [BODY-C103] 原文段落保留段落结构。
- [BODY-C104] 原文列表保留列表结构。
- [BODY-C105] 既有笔记自行制表时按原文恢复对应内容。
- [BODY-C106] 原文独立表格的表题使用 `**表N：**` 格式。
- [BODY-C107] 表题写入所属表的 `title`。
- [BODY-C108] 表下注写入所属表的 `footer`。
- [BODY-C109] 表格脚注写入所属表的 `footer`。
- [BODY-C110] 表内符号释义写入所属表的 `footer`。
- [BODY-C255] 存在 `footer` 时，其值使用 YAML 字符串。
- [BODY-C111] v2 的 `footer` 位于数据块顶层。
- [BODY-C112] v3 的 `footer` 位于对应的 `sheets` 成员中。
- [BODY-C113] 多段表注使用 `footer: |-`。
- [BODY-C114] 表注保留原段落顺序。
- [BODY-C115] 数据块外不重复题注。
- [BODY-C116] 数据块外不重复表注。
- [BODY-C117] 表注不计入 `rows`。
- [BODY-C122] 表格使用 rich-table 数据块。
- [BODY-C123] Rich Table 版本限定为 v2 单表或 v3 多工作表。
- [BODY-C124] v2 顶层包含 columns。
- [BODY-C125] v2 顶层包含 rows。
- [BODY-C126] v2 顶层 theme 为 grid。
- [BODY-C127] v2 顶层 locked 为 true。
- [BODY-C128] v3 的 sheets 为非空列表。
- [BODY-C129] v3 的每张工作表包含 columns。
- [BODY-C130] v3 的每张工作表包含 rows。
- [BODY-C131] v3 的每张工作表 theme 为 grid。
- [BODY-C132] v3 的每张工作表 locked 为 true。
- [BODY-C133] 保留已有 Rich Table 版本，不将 v3 降为 v2。
- [BODY-C134] 保留已有工作表结构。
- [BODY-C135] 接受模板生成的成对 `---` 外包裹。
- [BODY-C136] 接受不含外包裹的 Rich Table 数据。
- [BODY-C137] Rich Table 数据须可作为 YAML/JSON 解析。
- [BODY-C254] Rich Table 数据只包含一份文档。
- [BODY-C138] 每张表的 columns 使用逐条展开的块状 YAML，不使用 flow 式数组。
- [BODY-C139] 每张表的 rows 使用逐条展开的块状 YAML，不使用 flow 式数组。
- [BODY-C140] 保留列 id。
- [BODY-C141] 保留列 name。
- [BODY-C142] 保留行 id。
- [BODY-C143] 保留行 cells。
- [BODY-C144] 逐表回读确认 columns 非空。
- [BODY-C145] 逐表回读确认 rows 非空。
- [BODY-C146] 逐表回读确认行数等于源表。
- [BODY-C147] v3 回读覆盖全部工作表，包括非活动工作表。
- [BODY-C148] 检查每张工作表 title 中的链接。
- [BODY-C149] 检查每张工作表 footer 中的链接。
- [BODY-C150] 检查每张工作表列名中的链接。
- [BODY-C151] 检查每张工作表单元格中的链接。
- [BODY-C152] 不得用管道表替代 Rich Table。
- [BODY-C153] 不得用 HTML 表替代 Rich Table。
- [BODY-C154] 不得用图片表替代 Rich Table。
- [BODY-C155] 表格单独成块。
- [BODY-C156] 已有表锚 `^tbl-…` 位于闭合围栏的紧接下一行。
- [BODY-C157] 已有表锚顶格书写。
- [BODY-C158] 已有表锚独占一行。
- [BODY-C159] 表锚与后续内容之间留一空行。
- [BODY-C160] 保留已有表锚 ID。
- [BODY-C161] 缺失的表锚交第 9 步处理。
- [BODY-C162] 归属不明的表锚交第 9 步处理。
- [BODY-C163] 正文、题注、补充材料及表格中的数学表达式整体使用 LaTeX。
- [BODY-C164] 保留数学表达式的上标。
- [BODY-C165] 保留数学表达式的下标。
- [BODY-C166] 保留数学表达式的重音。
- [BODY-C167] 保留数学表达式的运算符。
- [BODY-C168] 保留数学表达式的原编号。
- [BODY-C169] 带单位的量整体置于同一数学环境，包括时长与时间间隔，如“每$8\,\mathrm{h}$一次”。
- [BODY-C170] 数学环境中的单位使用直立体。
- [BODY-C171] 数值与直立单位之间使用细空格，如 `$28\,\mathrm{m\,s^{-1}}$`。
- [BODY-C172] 笔记可见文本中的连字符统一为 ASCII `-`（U+002D），覆盖题头、正文、题注、表格、书目、SI 和链接显示文字。
- [BODY-C173] 笔记可见文本中的范围连接线统一为 ASCII `-`。
- [BODY-C174] 笔记可见文本中的表格缺项标记统一为 ASCII `-`。
- [BODY-C175] 字符规范化不得改动 URL。
- [BODY-C176] 字符规范化不得改动路径。
- [BODY-C177] 字符规范化不得改动锚点 ID。
- [BODY-C178] 字符规范化不得改动原图。
- [BODY-C179] Rich Table 缺项保存为字符串 `'-'`。
- [BODY-C180] 量值范围随量排入同一数学环境，如 `$2\text{-}3\,\mathrm{km}$`。
- [BODY-C181] 量值正负号随量排入公式。
- [BODY-C182] 量值乘积随量排入公式。
- [BODY-C183] 角度随量排入公式。
- [BODY-C184] 百分号随量排入同一数学环境，如 `$25\%$`。
- [BODY-C185] 公式中的范围连接线使用 `\text{-}`。
- [BODY-C186] 公式减号保留 LaTeX 运算语义。
- [BODY-C187] 表头已给单位时只格式化对应量值。
- [BODY-C188] 保留表头列单位。
- [BODY-C189] 保留量值原精度。
- [BODY-C190] 保留算例编号。
- [BODY-C191] 量值公式化排除日期。
- [BODY-C192] 量值公式化排除 UTC 时刻。
- [BODY-C193] 量值公式化排除文献编号。
- [BODY-C194] 量值公式化排除图表编号。
- [BODY-C195] 量值公式化排除 DOI。
- [BODY-C196] 量值公式化排除链接目标。
- [BODY-C197] 量值公式化排除路径。
- [BODY-C198] 已有正确公式直接复用。
- [BODY-C199] 显示公式主体与 `\tag{…}` 之间只保留必要的 LaTeX 结构命令和空白。
- [BODY-C200] 无编号公式检查闭合分隔符前的内容。
- [BODY-C201] 多行公式检查最后一个公式行及闭合命令前后的内容。
- [BODY-C202] 移除公式末尾冗余标点。
- [BODY-C203] 移除公式末尾重复编号。
- [BODY-C204] 移除公式末尾误混入的文字。
- [BODY-C205] 正文所需标点归入正文。
- [BODY-C206] 保留数学分隔符、小数点、省略号、`\left.` / `\right.` 等有效数学语法。
- [BODY-C207] 保留原有 `\tag{…}`。
- [BODY-C208] 公式末尾字符用途不明时对照原文上下文。
- [BODY-C209] 公式原编号位于公式同行右侧。
- [BODY-C210] 保留已核实的公式 tag。
- [BODY-C211] 无编号公式不新增编号。
- [BODY-C212] 保留稳定锚点。
- [BODY-C213] 新建导航交第 9 步处理。
- [BODY-C214] 主文书目标题写作 `## N 参考文献`，N 为实际 H2 节序号。
- [BODY-C215] 补充书目标题写作 `### M.K 参考文献`，M 为补充材料章号、K 为实际 H3 节序号。
- [BODY-C216] 书目节序号与标题之间使用一个半角空格，如 `## 10 参考文献`、`### 11.3 参考文献`。
- [BODY-C217] 补充材料章名统一为「补充材料」。
- [BODY-C218] 补充材料整章位于主文全部内容之后，包括主文参考文献。
- [BODY-C219] 补充材料整章位于「Zotero原条目」之前。
- [BODY-C220] 补充材料按最终章序编号。
- [BODY-C221] 迁移补充材料重编号影响的章节链接。
- [BODY-C222] 参考文献保留原文顺序。
- [BODY-C223] 每条参考文献独立为一个列表项。
- [BODY-C224] 参考文献列表使用 `1. …` 编号。
- [BODY-C225] 首条参考文献紧接书目标题。
- [BODY-C226] 书目条目之间不留空行。
- [BODY-C227] 书目条目内部不留空行。
- [BODY-C228] 书目列表与其后标题之间留一空行。
- [BODY-C229] 逐条登记 bibliography_id。
- [BODY-C230] 逐条登记书目顺序。
- [BODY-C231] 逐条登记书目原标签。
- [BODY-C232] 逐条登记现有块 ID。
- [BODY-C233] 保留作者—年份引文制。
- [BODY-C234] 新建主文书目锚点使用 `^ref-N`。
- [BODY-C235] 新建单份 SI 书目锚点使用 `^sref-N`。
- [BODY-C236] 新建多份 SI 书目锚点使用 `^sref-src<source_token>-N`。
- [BODY-C237] 书目锚点分配遵循[第 9 步](../literature-internal-links/SKILL.md#约束)。
- [BODY-C238] 已有书目 ID 逐字保留，包括其他前缀或与当前列表序号不同的数字后缀。
- [BODY-C239] 已有书目链接目标逐字保留。
- [BODY-C240] 插件兼容性问题单独登记。
- [BODY-C241] 书目缺锚交接第 9 步。
- [BODY-C242] 书目锚点重复定义交接第 9 步。
- [BODY-C243] 书目目标错配交接第 9 步。
- [BODY-C244] 受影响的书目调用交接第 9 步。
- [BODY-C245] 不得将 SI 编号套入主文书目作用域。
- [BODY-C246] 旧书目脚注转列表时保留完整文本。
- [BODY-C247] 旧书目脚注转列表时保留 ID 映射。
- [BODY-C248] 真实脚注不转为书目列表。
- [BODY-C249] 只移除有证据的附加题名中译。
- [BODY-C250] 保留中文书目。
- [BODY-C251] 保留书目必要括注。
- [BODY-C252] 书目缺锚、重复、目标错配或调用受影响时，提交前须通过第 9 步书目锚点验收。
- [BODY-C253] 既有 ID 仅因插件不识别或格式不同产生 warning 时，不要求迁移。
- [BODY-C257] 新建 DOI 解析链接使用 `https://doi.org/<DOI标识符>`。
- [BODY-C258] DOI 链接中的标识符须与来源一致。
- [BODY-C259] 已有 DOI 链接未获迁移授权时保留原目标。
- [BODY-C260] 可见文本中的已核实邮箱写为 `[原显示文字](mailto:完整邮箱)`。
- [BODY-C261] 每个图内数据表另建独立的中文可编辑 Rich Table。
- [BODY-C262] 图内数据表的可编辑副本放在所属图块之后。
- [BODY-C263] 图内数据表副本的标题使用“图N内表格”标识来源。
- [BODY-C264] 图内数据表副本不得占用原文独立表格的编号。
- [BODY-C265] 图内数据表转换不得改动原图。
- [BODY-C266] 按仓库模板完成 Rich Table 源码。
- [BODY-C267] 图内数据表副本的每个单元格须与来源逐格核对。
- [BODY-C268] 量值格式扫描覆盖表格的所有可见文本字段。
- [BODY-C269] 可见文本中已核实的裸 DOI 标识符须建立 DOI 解析链接。
- [BODY-C270] 源表合并关系按[合并字段说明](references/rich-table-merges.md)写入对应表的 `merges`。
- [BODY-C271] 每个合并范围的 `anchor` 指向源范围的左上单元格。
- [BODY-C272] 每个合并范围的 `end` 指向源范围的右下单元格。
- [BODY-C273] 合并范围内被覆盖单元格的内容为空或省略。
- [BODY-C274] 不得用重复组标签替代原有合并关系。
- [BODY-C276] 相邻父子标题之间留一空行。
- [BODY-C277] 每处来源中的裸网址独立登记为一个来源单元。

### 最后一遍格式核对

自动排版提示均为 `review` 级待核位置。提示不认证来源含义。引用式链接识别覆盖单行标签；复杂多行标签和深层容器嵌套不属于这项启发式的保证范围。对应结构仍按实际 Markdown 源码核对。

标题后直接出现下级标题时，按下级标题前的间距留一空行；首块为显示公式时按公式前间距处理。普通正文直接接标题下一行。此处分别适用 BODY-C276、BODY-C79 与 BODY-C68。

量值范围与减法按原意区分。例如，来源表示“五至七年”时写为 `$5\text{-}7\,\text{年}$`；表示差值时保留减号。中文数词表达的时长同样属于带单位量，例如“十二年的资料”写为“$12\,\text{年}$的资料”。近似限定仍依原文保留，例如“数十年”可保留为 `$\text{数十}\,\text{年}$`，不得只替换其中的“十”而变成精确值。公历年份仍按日期处理。

复核量值时同时看闭合 `$` 右侧：`$12$年` 仍然把单位留在数学环境外，应为 `$12\,\text{年}$`。同样检查题注、表头、单元格及每份 SI 的正文。`QUANTITY_SPLIT_MATH` 提示专门列出这种边界，不能仅因数值已带 `$` 就结束核对。

英文文献题名中的实际时长也逐处核对，例如 `a 7-year survey` 中的时长可写为 `a $7\,\mathrm{year}$ survey`。保留原题词义。发表年份仍按日期处理。`QUANTITY_OUTSIDE_MATH` 会提示英文长写单位。提示本身不决定这处文字的含义。

Rich Table 的每张工作表分别检查 `title`。独立表标签使用 `**表N：**`。图内表使用下述“内表格”标签。粗体必须闭合。图题标签后保留模板空格，例如 `**图S7：** 题注正文`。

来源清单若使用 `a)`、`b)` 等字母项，单纯在各行末尾加两个空格仍然只是普通段落。用 Markdown 列表标记保持列表块，例如 `1. a) 第一项` 与 `2. b) 第二项`。原字母标签保留。`LETTER_LIST_NOT_MARKDOWN` 只是结构提示。是否属于清单应以来源为准。

从图中另建的可编辑表格，标题示例为 `**图8内表格。** 原图中表格的说明`；保留“内表格”三个字，图编号取实际父图。该表的独立锚点仍按篇内链接技能生成。

来源若直接显示 `https://example.org/data/`，建立 `[https://example.org/data/](https://example.org/data/)`，保留原网址显示文字。来源原本带有命名链接时保留其显示文字。不要自行把裸网址的显示文字换成机构名称。来源清单可用 `external_url` 记录每处链接的目标和原显示文字。

## 产物

- drafts/08-body.md。

## 验收标准

每项验收按其 ID 单独记录判定结果。

- [ ] [BODY-A01] 文本变化均有来源依据。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A02] 数字变化均有来源依据。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A03] 对象数量变化均有来源依据。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A04] 书目数量变化均有来源依据。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A05] 原文有摘要时，源码恰有一处 `## 摘要`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A07] 摘要首段紧接标题下一行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A08] 摘要段首没有旧摘要标签残留。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A09] 摘要内容完整。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A10] 摘要分段完整。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A11] 原文无摘要时，候选稿没有摘要节。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A12] H1 内容为原题。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A13] 正文首个编号 H2 为 `1`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A14] 参与编号的标题逐级连续。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A15] 普通内容紧接标题下一行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A16] 标题后首块为显示公式时留一空行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A17] 受保护的语法块保持完整。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A18] 主文书目标题源码符合 `## N 参考文献`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A19] 补充书目标题源码符合 `### M.K 参考文献`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A21] 书目节序号在同级连续。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A22] 首条书目紧接标题。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A23] 书目标题没有附加文字。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A24] 受影响的章节链接已按第 9 步核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A25] 补充材料位于主文全部内容之后，包括主文参考文献。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A26] 最后一个 H2 为「Zotero原条目」链接标题，按[篇内链接验收](../literature-internal-links/SKILL.md#验收标准)核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A27] 已按检查表复核源码差异。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A28] 转义处理经复跑保持幂等。 <!-- check-type: execution_process -->
- [ ] [BODY-A29] 受影响的解析结构已核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A35] 外链的源码目标已核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A37] 各项检查均有证据记录。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A38] 各项检查均有实际覆盖范围记录。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A40] 待验检查均注明位置。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A41] 每个数值均已对照输入核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A42] 每个单位均已对照输入核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A43] 每个幂次均已对照输入核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A44] 每个数学符号均已对照输入核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A45] 每个数值范围均已对照输入核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A46] 没有遗漏的裸单位。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A47] 带单位的量没有被拆入不同数学环境。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A48] 单行显示公式源码已逐块检查。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A49] 多行显示公式源码已逐块检查。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A50] 显示公式前恰有一空行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A51] 已有公式锚紧接闭合 `$$`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A52] 显示公式块后有内容时恰有一空行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A53] 公式锚点范围已按[篇内链接验收](../literature-internal-links/SKILL.md#验收标准)核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A54] 有编号显示公式末尾没有冗余字符。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A55] 无编号显示公式末尾没有冗余字符。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A56] 多行显示公式末尾没有冗余字符。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A57] 公式数学含义已对照差异核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A58] 公式编号已对照差异核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A59] 公式锚点已对照差异核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A62] 公式未决项已记录。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A63] 可见文本连字符均为 ASCII `-`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A64] 可见文本范围连接线均为 ASCII `-`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A65] 可见文本缺项标记均为 ASCII `-`。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A66] 可见文本没有 U+2013 残留。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A67] 公式范围语义正确。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A68] 公式减号语义正确。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A69] 每幅图已对照原文核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A70] 每个表已对照原文核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A71] 每个公式已对照原文核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A72] 每幅最终嵌入图与[裁剪视觉验收](../literature-fulltext-extract/references/figure-crop.md)记录一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A73] 图片位于对应题注上方。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A74] 图片题注紧接图片下一行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A75] 正文脚注调用的 `<sup>` 格式正确。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A76] 正文脚注注文的引用块格式正确。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A77] 正文脚注编号配对正确。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A78] 正文脚注链接目标正确。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A81] 数学上标未被脚注处理改动。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A82] 型号未被脚注处理改动。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A83] 每个 Rich Table 数据块按实际外包裹形式成功解析 YAML/JSON。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A84] v2 单表或 v3 每张工作表的 locked 均为 true。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A85] v2 单表或 v3 每张工作表的 theme 均为 grid。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A86] 每张工作表的 columns 非空。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A87] 每张工作表的 rows 非空。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A88] 每张表的表注内容完整。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A111] 已有 `footer` 的值是 YAML 字符串。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A89] 每张表的表注仅位于所属表的 footer。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A90] 正文没有重复表注。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A91] 每张表的数据行数与源表一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A92] 每张表的题注链接已核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A93] 每张表的表注链接已核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A94] 每张表的列名链接已核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A95] 每张表的单元格链接已核对。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A99] 每张表的实际行数已有记录；v3 逐工作表记录。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A100] 每张表的源表行数已有记录；v3 逐工作表记录。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A101] 整个表格数据块的闭合围栏与表锚之间零空行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A102] 每个表锚顶格书写。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A103] 每个表锚独占一行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A104] 表锚后有内容时恰留一空行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A105] 表格缺锚已修复；可交第 9 步完成修复。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A112] 所有表格缺锚修复均已在提交前复验。 <!-- check-type: execution_process -->
- [ ] [BODY-A106] 表锚错绑已修复；可交第 9 步完成修复。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A113] 所有表锚错绑修复均已在提交前复验。 <!-- check-type: execution_process -->
- [ ] [BODY-A107] 重复表锚 ID 已修复；可交第 9 步完成修复。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A114] 所有重复表锚 ID 修复均已在提交前复验。 <!-- check-type: execution_process -->
- [ ] [BODY-A108] 既有调用已保留或完成已授权的迁移。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A115] 每个图内数据表均有独立 Rich Table 副本。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A116] 每个图内数据表副本的数据行数与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A117] 每个图内数据表副本的列数与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A118] 每个图内数据表副本的类别代号与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A119] 每个图内数据表副本的数值与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A120] 每个新建 DOI 解析链接均使用规定的规范目标。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A121] 每个可辨认的可见邮箱均有正确的 mailto 目标。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A122] 每个时长量值已整体置于数学环境。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A123] 每个量值范围已整体置于数学环境。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A124] 每个百分数量值已整体置于数学环境。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A125] 离线候选中的每个表格均已完成 Rich Table 源码构建。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A126] 每个可见 DOI 标识符均有对应的解析链接。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A127] 每个源合并范围均有对应的 `merges` 项。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A128] 每个合并范围的左上端点与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A129] 每个合并范围的右下端点与来源一致。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A130] 每个合并范围的被覆盖单元格为空或省略。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A131] 经过模板处理的既有合并范围未被改动。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A133] 相邻父子标题之间恰有一空行。 <!-- check-type: offline_outcome -->
- [ ] [BODY-A134] 来源裸网址的链接显示文字与来源一致。 <!-- check-type: offline_outcome -->
