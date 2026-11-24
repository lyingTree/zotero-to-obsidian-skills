# Rust 原生迁移

本分支直接迁移到 Rust，不以 PyInstaller 或嵌入 CPython 作为交付方式。Python 1.1.0 保留为开发期参考实现及回归测试工具，原生可执行程序不会启动 Python、执行 Python 脚本、自动回退或用模型判断代替程序验收。

当前为迁移候选，不是完整的用户安装包。native/Cargo.toml 的 0.1.0 是原生引擎开发版本，与仓库 VERSION 独立；完成迁移和发布验收前不修改已发布的 1.1.0，不发布 tag/release。

## 当前已实现的操作

使用 native/src/main.rs 构建的 z2o.exe 或 z2o；describe 返回精确注册的操作、平台、引擎版本及 full_workflow_ready。invoke 读取版本化 JSON 文件，输出 JSON 结果；任何未注册操作均以非零退出码和 UNMIGRATED_OPERATION 报告，不允许静默跳过。

示例输入（实际脚本调用时由任务创建 JSON）：

    {
      "schema_version": 1,
      "operation": "deterministic_stage_checks.author_token",
      "args": [["欧阳", "Smith"]],
      "kwargs": {}
    }

路径可用普通 Unicode 路径字符串或 {"$path":"实际路径"}；原始字节使用 {"$bytes":"base64"}。JSON 深层重复键及非有限数被拒绝；程序输出保持 UTF-8，进度使用 stderr。运算成功不等于来源语义通过。

| 既有模块 | 当前原生实现 |
|---|---|
| deterministic_stage_checks | snapshot、verify_binding、inspect_relative_files、author_token、collection_ancestry、split_frontmatter、build_properties、repair_properties、audit_properties、audit_unchanged_properties、audit_write_readback、audit_translation_binding、audit_rich_tables |
| format_helpers | build_naming_plan、validate_naming_plan（当前 schema）、check_local_name_conflicts、new_figure_table_anchor、build_figure_block、plan_supplement_figure_caption |
| source_url_inventory | scan_source_url_occurrences、source_url_coverage（重复出现位置、源位置及一对一匹配） |
| render_note | content_template、render_note：题头/章节/正文/列表/图/表/公式/书目及分来源映射；非法合同返回阻断报告 |
| format_helpers 增量 | build_rich_table_block：JSON 数据的块状 YAML、逐 sheet 校验、动态围栏长度和 LF/CRLF |
| normalize_note | normalize_note、normalize_file：保护语法范围、确定性修改、Unicode 变换记录、幂等性、独占发布与回读 |
| validate_note | validate_note、validate_file、inspect_note：静态 YAML、Rich Table 全工作表、公式、锚点、可见链接与书目检查；inspect_note 返回围栏实际记录 |
| glossary_candidates | 原生术语索引、候选及来源证据 |
| note_pipeline | 原生完整本地机械流水线、来源合同、重放、回读与不可变回执 |
| lint_presentation / accept_note | 排版提示及实际对象、链接、来源与资源哈希、逐单元机械验收 |
| validate_delivery 增量 | Rich Table 合并范围、覆盖值、重叠与来源网格检查；完整交付验收未迁移 |
| docx_source_inventory | 原生 XML、段落/表格/图片关系、嵌入媒体、待核对象及原生位置校验 |
| prepare_source_inventory | DOCX 专用来源准备；PDF 仍阻断 |
| prepare_translation_review | 真实文件绑定、位置扩展、待核出现项/术语及翻译审计初始化 |
| check_records / complete_workflow 增量 | 实际规则目录、证据绑定、全量待核台账、待核队列、台账验证、显式决定批量写回及候选绑定；交付范围派生与完整事务仍待迁移 |
| draft_queue | 冻结清单、页面顺序与对象登记、显式分组/排除、不可变草稿修订、来源问题处置、完整组装与映射 |
| 其他操作 | 以 native/migration-status.json 和实际二进制 describe 为准，尚未全部迁移 |

所有原生实现目前均需通过相应回归，不能以新增函数数量充当迁移完成率。YAML 解析使用 serde-saphyr 的重复键拒绝及解析预算；保守修复不重新序列化原有自定义属性，不确定的复杂 YAML 阻断。Unicode 规范化/完整 casefold 算法必须与目标文件系统约定及参考版本对照，不能假设库的 Unicode 数据版本一致。路径使用本机 Path/PathBuf，文件名仍同时检查 UTF-16 单元及可声明的 UTF-8 字节预算。资源相对路径拒绝穿越、设备名和解析到根外的符号链接；并发文件替换不是本阶段只读扫描所能完全消除的风险。

## 平台适配

| 系统 | x86-64 | ARM64 |
|---|---|---|
| Windows | x86_64-pc-windows-msvc | aarch64-pc-windows-msvc |
| Linux | x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu |
| macOS | x86_64-apple-darwin | aarch64-apple-darwin |

使用 GitHub 原生对应 runner 运行测试和 release 构建；不是仅靠 cross compile 推断可运行。Linux 以 Ubuntu 22.04 runner 为本轮构建基线，其他发行版/glibc 兼容性需单独验证；Windows 长路径、NTFS junction、大小写及保留名，macOS APFS 大小写和 NFC/NFD 差异需保留实机测试。正式发布还需精确提交 Cargo.lock、审查第三方许可证、签名/公证和安装包哈希。

## 剩余迁移及不可跳过的发布门槛

1. Markdown 源码扫描、组装、规范化、篇内及库内链接；保留 Math、Obsidian 注释、Rich Table 全部工作表、已有 ID 与原子规则语义。
2. 完整原子台账、来源/交付映射、图件库存、断点恢复、独占提交与真实写后回读。复用当前参考样例做每条规则差分，而不是只比较 pass。
3. DOCX：zip/quick-xml，保留原 XML 路径、关系、图片与表格顺序；PDF：评估 pdfium-render + 明确版本的 PDFium，保留页文本、坐标、变换、对象、SMask、裁剪及其来源证据，不把单纯提取字符串等同于原有 PDF 处理。
4. 翻译审计、术语表、Zotero API、Obsidian 文件写入与原生安装器；替换两套外部技能的 Python 调用，用户无需 Python 或 Rust 编译器。
5. Windows/Linux/macOS × x86-64/ARM64 实际运行，所有历史回归、冻结模型样例、负面与并发测试、原子验收覆盖核对。不能迁移的语义判断仍为待核，不允许以更宽松规则通过。
6. 仅当以上验收均完成，才切换全部 12 个技能的生产调用、移除用户安装链上的 Python 并构建原生发布包。未迁移的操作在 describe 中必须明确反映；禁止把部分实现发布为完整工具。

开发测试由 .github/workflows/native.yml 执行。scripts/compare_native_reference.py 在参考环境中以独立进程比较实际文件产物、完整命名计划和每项验收状态；Python 是测试预言机，不是二进制依赖。现有 validate.yml 继续保留参考实现的完整回归。

### 组装引擎的迁移验收边界

新增的 Rust 双遍组装先验证来源计划与翻译槽位并分配编号、锚点，再生成实际 Markdown；对未知/重复/缺失占位符、跨对象锚点、重复标题指向和非法源字段直接阻断。映射使用 Unicode 码点而不是 UTF-8 字节位置，并记录真实 SHA-256。YAML 输出经独立回读核对全部 JSON 数据，保守引号策略不承诺任意 PyYAML 格式字节完全相同；复杂标量、浮点数序列化及旧合同兼容性仍需差分覆盖。本地 Markdown 机械处理流水线已实现；完整来源抽取、审阅台账、正式交付、外部集成及原生安装链尚未全部迁移，full_workflow_ready 保持 false。

### 规范化迁移验收

Rust 规范化器使用独立词法扫描与原始字节区间，处理 CRLF/LF、转义、嵌套 Markdown 链接、代码、公式、引用定义和 Rich Table。量值、空格、横线与图块间距的修改形成 Unicode 码点坐标的可重放记录。固定点验证不等于来源语义通过；输出路径采用不覆盖的同目录临时文件发布与真实回读。差分测试直接复用现有 Python 的全部规范化样例，并比较实际全文、逐次修改、行列、待核问题和哈希，而不仅比较状态。新增代码待六平台运行及差分验证，不能视作已验收。

### 静态验收迁移

静态验收使用真正的 YAML 事件节点定位与结构样式检查、独立安全反序列化、Markdown 注释/代码/公式屏蔽、书目命名空间及全部工作表可见链接扫描。Rich Table 的原子检查复用同一份实际扫描记录，避免两个扫描器认定不同的范围。差分执行原始静态验收和 Rich Table 测试类，核对规则 ID、级别、行号、围栏记录及非解析错误文字；解析库原生诊断文本保留差异，待进一步核对复杂 YAML 类型。静态无发现不等于全文交付或译义通过。

### 2.0.0 发布门槛

native/migration-status.json 登记实际存在的 23 个参考模块和 11 项必要发布能力。scripts/check_native_release.py 使用 AST 枚举参考公开函数和公开类方法，核对实际二进制 describe、缺失接口、资格状态、版本及已提交 Cargo.lock。开发分支报告缺项；VERSION 或 tag 进入 2.x 或明确请求发布检查时，存在缺项便返回非零。模块状态只有经过行为差分、六平台实机运行及完整工作流验收后才能改为 qualified。此门槛不以操作注册数量代替等价性，也不自动创建 tag/Release。

### 来源准备与翻译审计初始化

DOCX 原生适配器保留真实 XML 和媒体，禁止假造页号与裁剪结果。新增 prepare_source_inventory.prepare_docx_sources 可为一组 DOCX 生成统一来源清单及待核术语记录；在创建工作目录前阻断 PDF（无 Python 回退）。JSON 对象按键顺序处理，跨文档阅读顺序必须由独立来源计划确认。新增 prepare_translation_review 的原生脚手架在发布前复核源、清单、候选和术语表文件哈希，保留显式选中和未选中单元、各类待核检查与出现项；不推断任何语义结论。新增实际文件比较及负例测试，仍待 CI 验证。尚未移植 PDF 渲染/图片对象链、翻译审计 writer/validator、完整验收台账/映射及原生安装器和 API。

原子规则目录迁移保留实际 SKILL.md 和 references 的字节哈希、行号、声明类型与完整独立 ID；注释、代码和嵌套归档不计入。新建台账只生成 not_verified，不产生内容或执行通过结论。台账校验、记录写回、范围派生及 inventory 写回已在后续增量中实现；完整工作流及发布资格仍需验收。

### 原生验收台账验证与写回

新增 check_records.validate_check_records，按当前真实规则文件、原子声明、每条状态/范围、证据文件字节和定位逐项检查。PDF 页数由 lopdf 实际读取页树，图像像素区域由 image 读取实际尺寸，文本与 JSON 指针使用原文件内容并缓存；这不等价于 PDF 完整来源几何或视觉核验。原有历史台账反例复用作差分测试。新增 complete_workflow.record_check_batch、record_decision_rows、bind_candidate 及 deterministic_stage_checks.record_file_checks，使用同目录独占协作锁、旧哈希比较、临时文件同步与原子替换；保留每条之前的记录，拒绝篡改规则/类型、复用变更的证据 ID、同名不同字节报告和候选静默重绑定。程序不自动授予语义、执行或现场观察通过。协作锁保护本工具的写入者，不能替代外部编辑器的协作锁或保证跨多个文件的原子事务。完整工作流及跨文档事务恢复仍需验收。

新增 check_records.scopes_from_deliveries 直接复用 Rust 静态验收词法函数，检查候选稿真实可见锚点的唯一性、行范围哈希及唯一引文，保留用户明确选择的来源对象和顺序。对 DOCX 额外支持真实原生 XML 位置，不虚构页码；PDF 页号仍仅按清单声明的页范围校验，需后续真实来源交付验证。原有范围正反例纳入差分测试。

新建台账支持原有 inventory_path 和显式 overwrite 参数；实际规则、来源和候选文件始终受保护。两文件更新按稳定路径顺序获取原生协作锁，在发布前保存完整旧/新字节和独立恢复计划；库存写回失败时仅回滚仍与本次输出字节一致的台账，保留全部恢复数据。成功覆盖也保留旧台账。两个独立文件的更新不能宣称单次文件系统原子操作，崩溃后需依据恢复计划确认实际哈希。check_records 与 deterministic_stage_checks 现在为实现候选，不代表已取得发布资格。

格式辅助模块已补齐 starts_with_bold_source_label 与 apply_visible_text_edits。后者按原始 Unicode 码点范围一次性组装显式可见文本修订，拒绝过期文件哈希、重复/重叠位置及未观察决定，保留 CRLF 和未选择的连字符、减号、拼写，不修改原始文件，不认证观察或语义。原有完整 VisibleTextEditTests 用于差分验证。format_helpers 的公开操作已全部注册，仍需全面行为验收。

### 原生逐来源起草队列

新增 draft_queue.prepare_draft_queue 及 DraftQueue 的全部公开状态操作，原生入口按每次调用重新加载冻结来源、保留源对象与 DOCX 顺序、绑定原始哈希、逐对象登记和分组、保存不可变片段及附件、独占组装候选与真实行范围映射。调用 open 返回实际队列 SHA-256，修改时可传 expected_sha256 保护旧会话；协作锁和旧字节比较拒绝并发覆盖。PDF 页边界优先使用已冻结图件清单，缺失时从实际 PDF 页树读取未旋转媒体/裁切范围及 UserUnit；该几何候选仍需专门差分，完整 PDF 图件引擎不是本模块的能力。已接入原有 DraftQueueTests 通过实际二进制执行及原生负例测试。开发态进度函数不构成发布资格，完整编排、来源交付及翻译审计仍未完成。

### PDF 与交付准备增量

新增真实 PDF 页树、未旋转坐标、Do 调用及嵌套 Form 遍历、原始流哈希、遮罩引用与图形状态记录；仅生成结构证据，不能替代原来的页面图像/文本清单。index_page_structure 与 prepare_pdf_structure_evidence 明示 not_rendered，不授予原始 ImageObjectIndexer 或完整提取资格。prepare_sources 对 DOCX 保持现有实现，对 PDF 仍阻断。新增真实文件绑定的交付清单、Markdown 链接位置、来源单元结构和稳定投影对照；对照不是实际 rerun，保留 original record_mapping_rerun 阻断，直到真实地图生成与验证迁移完成。候选实现待完整差分与跨平台 CI。

### 翻译审计生命周期增量

原生 validate_translation_review 按真实源文件/候选/术语表哈希、原始 PDF 页范围、DOCX XML 定位、候选行及唯一引文、逐处出现项、逐类别术语检索、术语指针、范围排除和事件证据生成逐项记录诊断。原生 writer 采用无状态 JSON 短命令重放：replay_decisions、save_decision_draft、restore_decision_draft、save_review，执行实际 glossary 查询而非接受伪造 hit；源/候选原始范围和声明的可见/转义读法分开，保存新文件和原始绑定。task 模块据同一验证器按 scope→unit→global 的顺序提供任务，最终预览只写临时文件。未验证的旧类接口映射保留未资格化，不以新增接口数量替代行为等价；原有完整正例及多类反例用于差分测试。发布门禁仍保持关闭。

### 图件审阅初始化增量

`prepare_source_inventory.prepare_figure_reviews` 使用实际 PDF 页树、未旋转边界及可达 XObject 资源检查，或使用 DOCX 原生媒体定位，绑定已存在的页面渲染和最终图片，生成 28 个 pending 原子检查及每件资源四个 pending 边缘槽位。原始清单快照和恢复计划先发布，随后更新清单指针；不自动填充视觉结论。PDF 资源存在不等价于实际绘制，更不代表页面渲染与完整图件提取已经迁移。新增真实多页 PDF/XRef 参考差分测试，待 CI 验证。

### 原生工作流初始化与断点续作增量

新增 `complete_workflow.initialize_workflow` 的 DOCX 原生入口：冻结规则/技能文件和原始附件，调用真实来源清单、全量 pending 台账及不可变起草队列生成器，最后独占发布 run.json。PDF 输入在产生工作目录前明确阻断。`resume_workflow` 从真实运行记录和冻结来源只读恢复 setup/draft/assemble 阶段，重新核对角色、原始附件和已写片段哈希；当候选已存在并进入 review 阶段时，完整映射/交付检查未迁移，明确返回 UNMIGRATED_WORKFLOW_INSPECTION，不产生接受结论。新增 DOCX 参考差分和来源变更负例。尚不等价于完整工作流。

### 文本层与可见页冲突记录增量

新增 `handoff_maps.record_text_visual_conflict` 原生写入器。按实际源页和已登记单元、文件哈希、Unicode 码点或行范围、真实页边界验证双方读法、显式选择及观察状态；允许保留未决记录，拒绝伪造来源和重复冲突 ID，协作锁保护清单写入。新增真实 PDF 的参考差分、重复/范围/读法/变更证据负例。完整映射生成及最终冲突状态验收仍待迁移。
