# Python 组装、规范化与验收

新建全文任务优先使用这一入口。程序负责可确定的结构、格式和文件检查；模型负责来源辨认、必要的顺序判断与译文。模型不用手拼每个锚点、空行、图块或 Rich Table，也不用为程序能检查的规则逐条自填“已通过”。已有 `complete_pdf_candidate_v1` 工作目录仍按旧入口恢复，不迁移或覆盖旧回执。

## 职责与结果

| 环节 | 程序负责 | 仍需对源判断 |
|---|---|---|
| PDF / DOCX 提取 | 现有来源准备脚本保存文本块、页位置、原始 DOCX 对象与资源 | 多栏阅读顺序、扫描识别歧义、对象归属 |
| 来源计划 | 校验唯一 ID、固定顺序、作用域、必需字段；绑定实际源文件哈希 | 从原文确定完整的对象清单，不能从候选稿反推预期清单 |
| 翻译提交 | 按 ID 校验缺项、多项、未知字段和类型；保护源公式、路径和表数据 | 完整翻译及必要的事实性译注 |
| 组装 | 题头、章节编号、列表、图表公式、书目编号、锚点、链接、SI 和译注块 | 引文实际对应哪条书目、表题与单元格的译义 |
| 规范化 | 明确量值与百分比、可确认的空行和可见字符；逐次记录变换并验证幂等 | 有歧义的范围、未识别单位、复杂或损坏语法 |
| 机械验收 | YAML、实际对象、断链、重复锚点、资源存在/哈希、合同数量与映射 | 译义、来源清单是否遗漏、图片裁剪内容是否完整 |

`mechanical_pass` 只表示本次已实现的机械检查及显式来源合同通过。`needs_attention` 表示仍有机器失败、未绑定范围或解析问题；`blocked` 表示无法生成可验候选。三种状态的 `source_fidelity` 均保留 `not_assessed`。不能把字数、相似度、对象数量一致或模型自称完成当作全文语义通过。

## 两种用法

在 Python 编辑器中打开[调用示例](process_note_example.py)，设置顶部路径和模式后运行。无需拼命令行参数。

### 已有 Markdown

调用 `process_markdown(candidate_path, output_root, contract=..., resource_root=...)`。程序保留原稿，再统一规范化并验收。没有来源合同时仍能做结构检查，但来源覆盖保持 `not_assessed`。

`resource_root` 是实际库根或离线资源根。图片路径从该根解析；不是新版本候选所在的临时目录。程序拒绝越出该根的本地资源路径，不修改链接目标或图像字节。

### 新建候选

1. 用 `prepare_source_inventory.py` 提取实际 PDF / DOCX；保留既有来源材料和页/对象定位。
2. 从来源建立 `plan`，一次确认对象顺序、种类、作用域和必要的固定事实。
3. 用 `prepare_content_job(plan, sources, output_root)` 一次保存计划、合同和空模板；其中 `build_contract(plan, sources)` 负责来源绑定与结构预期。不要根据模型交付结果缩小合同。
4. 将 `content-template.json` 另存为 `content.json`，模型填写其中的译文槽位；公式、资源路径、表结构、书目原文等固定数据不交模型重新拼写。单独调用 `content_template(plan)` 也可取得相同模板。
5. 调用 `process_content(plan, content, output_root, contract=..., resource_root=...)`。
6. 程序指出缺失/无效槽位时只补相应单元，重跑得到新版本。程序可以处理的格式继续交给程序。
7. 对译义、图像视觉内容和引文身份做必要的来源核对；翻译措辞差异及有来源依据的译注不以字面不一致判失败。

## 来源计划与译文槽位

`plan` 是 JSON 形状的字典：

```python
plan = {
    "schema_version": 1,
    "properties": {"类型": "期刊论文"},
    "units": [
        {
            "unit_id": "p-method-01",
            "kind": "paragraph",
            "source": {"source_id": "main", "locator": {"page": 2, "block": 4}},
            "scope": "main",
        }
    ],
}
content = {"p-method-01": {"text": "每60 min进行一次测量。"}}
```

每个单元都需要 `unit_id`、`kind`、`source`、`scope`。`source.locator` 是来源定位字符串或对象；程序绑定该声明，不自动证明它语义正确。主文作用域为 `main`，SI 为 `supplement:<稳定标记>`。主文内容排在 SI 前，原条目链接为最后的可选尾部。

| kind | 计划中的固定数据 | content 中的字段 |
|---|---|---|
| `header` | `original_title`、`authors`、`affiliations`、`translated`；可选 `notes` | `translated=True` 时填 `translated_title`，否则 `{}` |
| `heading` | `level`（2–6）；可选 `numbered` | `text`，不手填程序管理的节号 |
| `paragraph` | 可选 `links`、`literals` | `text` |
| `list` | `style` 为 `ordered`/`unordered`，`count` | `items`，长度与源列表一致 |
| `metadata` | `notes`，每项一条已核实的题头元信息 | `{}` |
| `figure` | `images`、`label`、`anchor`；可选 `links`、`literals` | 可选 `caption`；纯类别标签的图可留空 |
| `formula` | 原始 `latex`（不带外围 `$$`）、`anchor` | `{}` |
| `table` | 完整 Rich Table `data`、`anchor` | `{}` |
| `reference` / `bibliography` | `source_text`；可选已核实 `anchor` | `{}` |
| `translator_note` | 可选 `links`、`literals` | `text` |
| `zotero_link` | 已核实的 `uri` | `{}` |

普通翻译槽位使用单行文本；一段一个单元，换行列表使用 `list`。普通文字不会被当作新标题、锚点或模板代码执行。需要保留的行内公式等由计划的 `literals` 登记，用 `{{literal:标记}}` 引用。来源中已有引文用 `links` 绑定实际目标单元与显示文字，再在内容中放 `{{link:标记}}`。程序拒绝未登记、重复或漏用的标记。作者—年份制不自动改成数字制。

例如 `links={"cite1": {"target": "ref-example", "label": "Example, 2020"}}`。重复标题不提供猜测的链接目标。数字制合引可显式声明 `{"citation": ["ref-a", ",", "ref-b", "-", "ref-d"]}`；条目身份和范围端点必须来自来源，不由字符串相似度推断。

表格 `data` 是上游已核对的展示工作簿，包含所需中文表题、列名与脚注；序列化器不会自动翻译其中的单元格。v2/v3 的数据结构、标量值与精度均保留。任意既有 Rich Table 默认整体保护，由独立结构检查处理；不把 YAML 当普通正文正则替换。

## 可重复、可恢复、可追查

- 同一输入、路径、合同和引擎版本生成同一运行标识。输出目录按该标识分开；新输入生成新目录。
- 原稿保存为 `original.md`，规范化结果为 `normalized.md`。不原位覆盖候选、源附件或资源。
- `normalization.json` 每次变换含规则、阶段、行列、字符偏移、原值与新值。主入口逐次重放变换，并实际重跑规范化以确认字节幂等。
- `rendering.json` 的单元位置由实际变换重新计算。跨越不可确定边界时报告问题，不能只刷新哈希沿用旧位置。
- `acceptance.json` 记录程序实际执行的检查、期望/实际、位置、规则与输入哈希。独立验收也执行规范化固定点检查：如果仍有已确认安全的格式变换，按具体行列报告 `CANONICAL_FORMAT` 失败；修复后重新检查。未闭合边界明确失败，歧义保持待核。缺少合同的范围不自动通过。
- `phases.json` 分别记录已执行阶段、自动修复、观察到的失败及待核项。问题发现阶段与上游起因分别记录，未观察到的模型操作不会被标成通过。
- `report.md` 提供中文阶段与具体失败定位，`result.json` 绑定实际产物哈希。写入后回读；同一运行重试只接受字节一致的已有文件。
- 原子发布使用同目录临时文件和独占硬链接。中断不会把半写文件当作最终产物；文件系统不支持该能力时明确报错。用户修改或并发冲突保留原文件并报出路径。
- 引擎更新后使用新的 Python 解释器。代码改变时拒绝用旧缓存冒充新引擎，输入或资源变化时重新核验。

未知和歧义情形保留原文并报告。此行为避免静默损失；不表示任意 PDF / DOCX 都能被无误理解。

## 按环节汇报模型问题

每次测试至少分别报告：PDF 文字提取、DOCX 提取、来源单元/阅读顺序、标题识别、作者与单位识别、翻译覆盖与保真、文字/段落拼接、引文身份识别、篇内链接组装、图表与公式、空行格式化。

每项包含模型与推理强度、发现阶段、具体症状、候选位置、来源或执行记录位置、当前处理者和状态。用 `observed_failure`、`repaired_by_python`、`pending_review`、`not_observed` 等明确状态。未全面核对的环节不能仅因暂未报错写为通过。

例如“候选缺少公式”首先是来源对象覆盖失败；只有实际提取记录显示源公式未进入文本，才能进一步归因到 PDF 提取。某引文可跳转只证明链接组装，不证明书目身份匹配正确。实时更新按这些边界报告；完整观察记录仍保留在报告中。
