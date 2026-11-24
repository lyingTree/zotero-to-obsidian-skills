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
| macOS | 不支持 Intel Mac | aarch64-apple-darwin |

原生 2.0.0 以这五个目标为准；不构建、不分发、不验收 Intel macOS 包。使用 GitHub 原生对应 runner 运行测试和 release 构建；不是仅靠 cross compile 推断可运行。Linux 以 Ubuntu 22.04 runner 为本轮构建基线，其他发行版/glibc 兼容性需单独验证；Windows 长路径、NTFS junction、大小写及保留名，macOS APFS 大小写和 NFC/NFD 差异需保留实机测试。正式发布还需精确提交 Cargo.lock、审查第三方许可证、签名/公证和安装包哈希。

## 剩余迁移及不可跳过的发布门槛

1. Markdown 源码扫描、组装、规范化、篇内及库内链接；保留 Math、Obsidian 注释、Rich Table 全部工作表、已有 ID 与原子规则语义。
2. 完整原子台账、来源/交付映射、图件库存、断点恢复、独占提交与真实写后回读。复用当前参考样例做每条规则差分，而不是只比较 pass。
3. DOCX：zip/quick-xml，保留原 XML 路径、关系、图片与表格顺序；PDF：评估 pdfium-render + 明确版本的 PDFium，保留页文本、坐标、变换、对象、SMask、裁剪及其来源证据，不把单纯提取字符串等同于原有 PDF 处理。
4. 翻译审计、术语表、Zotero API、Obsidian 文件写入与原生安装器；替换两套外部技能的 Python 调用，用户无需 Python 或 Rust 编译器。
5. 上述五个原生目标实际运行，所有历史回归、冻结模型样例、负面与并发测试、原子验收覆盖核对。不能迁移的语义判断仍为待核，不允许以更宽松规则通过。
6. 仅当以上验收均完成，才切换全部 12 个技能的生产调用、移除用户安装链上的 Python 并构建原生发布包。未迁移的操作在 describe 中必须明确反映；禁止把部分实现发布为完整工具。

开发测试由 .github/workflows/native.yml 执行。scripts/compare_native_reference.py 在参考环境中以独立进程比较实际文件产物、完整命名计划和每项验收状态；Python 是测试预言机，不是二进制依赖。现有 validate.yml 继续保留参考实现的完整回归。

### 组装引擎的迁移验收边界

新增的 Rust 双遍组装先验证来源计划与翻译槽位并分配编号、锚点，再生成实际 Markdown；对未知/重复/缺失占位符、跨对象锚点、重复标题指向和非法源字段直接阻断。映射使用 Unicode 码点而不是 UTF-8 字节位置，并记录真实 SHA-256。YAML 输出经独立回读核对全部 JSON 数据，保守引号策略不承诺任意 PyYAML 格式字节完全相同；复杂标量、浮点数序列化及旧合同兼容性仍需差分覆盖。本地 Markdown 机械处理流水线已实现；完整来源抽取、审阅台账、正式交付、外部集成及原生安装链尚未全部迁移，full_workflow_ready 保持 false。

### 规范化迁移验收

Rust 规范化器使用独立词法扫描与原始字节区间，处理 CRLF/LF、转义、嵌套 Markdown 链接、代码、公式、引用定义和 Rich Table。量值、空格、横线与图块间距的修改形成 Unicode 码点坐标的可重放记录。固定点验证不等于来源语义通过；输出路径采用不覆盖的同目录临时文件发布与真实回读。差分测试直接复用现有 Python 的全部规范化样例，并比较实际全文、逐次修改、行列、待核问题和哈希，而不仅比较状态。新增代码待五个支持平台运行及差分验证，不能视作已验收。

### 静态验收迁移

静态验收使用真正的 YAML 事件节点定位与结构样式检查、独立安全反序列化、Markdown 注释/代码/公式屏蔽、书目命名空间及全部工作表可见链接扫描。Rich Table 的原子检查复用同一份实际扫描记录，避免两个扫描器认定不同的范围。差分执行原始静态验收和 Rich Table 测试类，核对规则 ID、级别、行号、围栏记录及非解析错误文字；解析库原生诊断文本保留差异，待进一步核对复杂 YAML 类型。静态无发现不等于全文交付或译义通过。

### 2.0.0 发布门槛

native/migration-status.json 登记实际存在的 23 个参考模块和 11 项必要发布能力。scripts/check_native_release.py 使用 AST 枚举参考公开函数和公开类方法，核对实际二进制 describe、缺失接口、资格状态、版本及已提交 Cargo.lock。开发分支报告缺项；VERSION 或 tag 进入 2.x 或明确请求发布检查时，存在缺项便返回非零。模块状态只有经过行为差分、五个支持平台实机运行及完整工作流验收后才能改为 qualified。此门槛不以操作注册数量代替等价性，也不自动创建 tag/Release。

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

旧账册的术语表绑定若使用相对路径，查询和草稿重放按账册所在目录解析，并复核原 SHA-256 和实际文件身份；不改写原账册或把变更后的术语表视为旧证据。

### 图件审阅初始化增量

`prepare_source_inventory.prepare_figure_reviews` 使用实际 PDF 页树、未旋转边界及可达 XObject 资源检查，或使用 DOCX 原生媒体定位，绑定已存在的页面渲染和最终图片，生成 28 个 pending 原子检查及每件资源四个 pending 边缘槽位。原始清单快照和恢复计划先发布，随后更新清单指针；不自动填充视觉结论。PDF 资源存在不等价于实际绘制，更不代表页面渲染与完整图件提取已经迁移。新增真实多页 PDF/XRef 参考差分测试，待 CI 验证。

### 原生工作流初始化与断点续作增量

新增 `complete_workflow.initialize_workflow` 的 DOCX 原生入口：冻结规则/技能文件和原始附件，调用真实来源清单、全量 pending 台账及不可变起草队列生成器，最后独占发布 run.json。PDF 输入在产生工作目录前明确阻断。`resume_workflow` 从真实运行记录和冻结来源只读恢复 setup/draft/assemble 阶段，重新核对角色、原始附件和已写片段哈希；当候选已存在并进入 review 阶段时，完整映射/交付检查未迁移，明确返回 UNMIGRATED_WORKFLOW_INSPECTION，不产生接受结论。新增 DOCX 参考差分和来源变更负例。尚不等价于完整工作流。

### 文本层与可见页冲突记录增量

新增 `handoff_maps.record_text_visual_conflict` 原生写入器。按实际源页和已登记单元、文件哈希、Unicode 码点或行范围、真实页边界验证双方读法、显式选择及观察状态；允许保留未决记录，拒绝伪造来源和重复冲突 ID，协作锁保护清单写入。新增真实 PDF 的参考差分、重复/范围/读法/变更证据负例。完整映射生成及最终冲突状态验收仍待迁移。

### 翻译 writer 原生接口增量

新增 select_source、select_candidate、append_decision。复用冻结准备记录和已有原生选择器，返回实际来源/候选位置、原始字节对应读法、显式解释及可重放选择参数；后续决定重新验证而非信任外部构造的 selection。追加决定之前和之后均复核绑定。正式登记原有 UnitReview.source/candidate 及 ReviewWriter.save/save_draft/from_draft 的原生替代接口。类方法的全部行为、历史正反例及完整工作流尚未全部取得发布资格。

### 原生来源交付核验与映射文件差分增量

`validate_delivery.check_source_provenance` 重新读取实际 OOXML，比较原生块/关系/媒体顺序、图件与 XML 字节、保存的原始文字及实际问题审阅证据。PDF 只检查实际页树、页范围及已保存的文件绑定，显式产生 PDF_FULL_PROVENANCE_UNMIGRATED；本操作始终不宣称完整交付通过。`mapping_rerun.compare_bound_maps` 从不同的实际映射文件读取并绑定精确字节，对稳定身份投影进行独立比较，写入独占证据报告；不会将调用者提供的第二组文件当作程序实际 rerun。完整原生映射生成/验证、PDF 像素/文本来源验证及交付整体验收仍需完成。

### PdfCraft 原生后端（迁移候选）

直接引入 [pdfcraft-render](https://github.com/storytold/pdfcraft/tree/6e910675c68dd5092e889065cfe8d0274c1b2742/crates/render) 的 headless 核心库，锁定上游 commit `6e910675c68dd5092e889065cfe8d0274c1b2742` 与对应 craft-libs 定制 PDF 依赖 `0a6e42b80bfd586aee6ee1144aa98cf1ea37a844`。不引入 PdfCraft GUI、MCP 服务或外部进程。按上游要求把 Rust 开发工具链提升到 1.95；最终用户仍仅运行 z2o 二进制。上游依赖 workspace 的 patch 不会自动传递，因此显式保留必要的 PDF-only patch。

新增原生 `pdf_image_objects.render_pdfcraft_page`、`render_pdfcraft_region`、`extract_pdfcraft_text` 及 `prepare_source_inventory.prepare_pdfcraft_evidence`，对同一份原始字节实施真实页面渲染、区域裁剪、字形坐标与结构 XRef 证据的绑定。坐标采用未旋转 PDF 点；核对 PdfCraft 与既有结构解析器的页数、页尺寸及旋转，再建立变换。拒绝越界、错误 DPI、静默降分辨率、哈希变化及冲突输出。现阶段不把 PdfCraft 字形/行分组伪造为原 PyMuPDF 文本块 ID，不把页面像素或文本结果当作语义和图件验收。PdfCraft renderer 的部分解码/字体诊断可能不会出现在返回 warnings 中，必须通过独立历史差分和视觉回归检查。保留现有 lopdf 0.36 结构定位，后续核对 COS 替换。

此次引入新的 git 依赖，原 Cargo.lock 不再代表完整图谱。候选 CI 单次实际 Cargo 解析生成锁文件，保存日志及 artifact，五个支持平台使用同一份生成的锁文件做 --locked 构建。发布门禁拒绝未提交或与 HEAD 不同的锁文件，版本/tag 发布不允许自动重解析。待真实 lockfile 回收提交后恢复直接 locked 构建。当前不能声明 PdfCraft 的编译、字体覆盖、像素等价和分发许可证审查已通过。

### CPU-first OCR 后端（候选）

直接复用 [ppocr-rs 0.8.5](https://github.com/dariofinardi/df-ppocr-rs/tree/b87550b775206655acf84d7fc421ca5bdf745fd3)，固定 Git revision，`ort = 2.0.0-rc.13` + API 28 + load-dynamic。只有实际 OCR 才加载本地 CPU ONNX Runtime 1.28.x；普通 PDF、DOCX、Markdown 操作不要求 GPU 或 OCR 模型。上游按平台启用了可选 EP API，本实现只注册 CPU，不尝试 CUDA、DirectML 或 CoreML。上游 Clipper C++ FFI 需要构建工具（发布用户无需编译器）。GPU 加速是后续可选扩展，不宣称已实现 GPU→CPU 回退。

新增 `prepare_source_inventory.ocr_hardware_plan`、`inspect_ocr_package`、`recognize_ocr_png`。按总内存和可用内存选择已安装 tiny/small/medium 模型，限制 det/rec 线程、检测输入尺寸和单页像素；layout 按需加载，orientation 单独记录原始方向和逆变换。辅助模型的线程限制、低端 CPU 兼容及内存峰值仍需实测。模型包 schema 1 为 `{"engine":"ppocr-v6","runtime":{"platform":"linux","architecture":"x86_64","version":"1.28.2","file":{"path":"runtime/libonnxruntime.so","sha256":"<actual>"}}, "profiles":{"tiny":{"detector":{...},"recognizer":{...},"dictionary":{...}}},"auxiliary":{"orientation":{...},"layout":{...}},"schema_version":1}`。所有资源记录均为包根目录相对路径+真实 SHA-256，manifest SHA-256 须来自独立可信安装配置。拒绝不匹配平台、目录穿越、链接及修改后的资产；不自动下载、执行或信任未经批准的远程模型。识别字典必须从对应模型的 inference.yml 提取，保留空行和 token 顺序，不可混用通用训练字典。包内 ONNX 应为自包含权重，安装器需验证发行来源和外部数据引用。

输出保留逐行、逐词检测/识别分数、原始图像坐标、方向校正后的坐标、可选版面区域及全部 OCR 候选；未知/越界框和低分候选只能触发复核。分数不是正确概率；不得自动通过验收。MiniCPM-OCR/Jev、表格结构重建、可信资源分发、实际 CPU 推理五个支持平台测试及完整扫描 PDF 工作流均待继续实现/验证。现已回收上轮 PdfCraft 实际依赖锁文件，加入 OCR 后须由 CI 重新解析并提交最终 lockfile。

### PDF 逐页 OCR 证据与恢复

新增 `prepare_source_inventory.prepare_pdf_ocr_evidence`。默认 auto 策略按实际页文字层与图像调用分类：正常文字候选不加载 OCR；无文字/异常文字层或占页大图伴随文字层时整页 OCR；含局部图像的混合页保留原文字与页面图像，提示显式区域复核。可按 `regions={"3":[[x0,y0,x1,y1]]}` 配合未旋转 PDF 点坐标执行区域 OCR，也可 mode=all。完整扫描页、原文字层、OCR 文字/方向/置信分数、源图和反算 PDF 坐标分别保存，不自动覆盖或合并。job.json 冻结原 PDF、模型包、DPI、策略和版本；成功页单独发布、恢复时重验文件，失败尝试保留并允许同输入重试。报告始终 source_inventory_ready=false、acceptance_ready=false；不冒充完整的 prepare_sources。尚需真实模型/平台测试、模型安装分发、表格/公式、GPU 扩展、最终内容映射与语义验收。布局会使用内存预算控制的 CPU 会话，方向分类器的线程/峰值预算仍需独立测试。

### 锁定依赖与真实 CPU 验收

回收成功的 `resolve-lock` job（run 45406085584, job 138487297262）生成的完整 Cargo.lock，含固定 PdfCraft 和 ppocr-rs Git revisions、ort 2.0.0-rc.13；CI 恢复 `--locked`，不得静默重解析。新增 Ubuntu x86_64 独立 CPU OCR smoke：从 Microsoft 官方 release v1.28.3 读取发布校验值，从 PaddlePaddle Hugging Face 模型 metadata 确定实际 revision 并校验 LFS SHA-256/Git blob，使用模型配置中的字典，不信任无元数据下载。只提取指定常规运行库文件、不跟随 archive 链接。CI 生成虚构中英文扫描版 PDF（无文字层），通过真实 z2o 二进制，在空 PATH、禁用 GPU 条件下执行并验证英文字串、中文、数值、实际坐标与源文件哈希。CI Python 仅用于生成测试夹具及资源验证，不是用户运行依赖。首次真实测试尚未执行完；成功后仍需其余五个 target 的实际模型、内存/低端 CPU 及字体、可信发布模型包的资格审查。此测试资源清单不是已签名或已批准的生产安装目录，不能将其直接标记为合格安装包。

CI 修复：OCR smoke 的工作目录改为执行步骤从 RUNNER_TEMP 写入 GITHUB_ENV，避免在 job 级 env 中引用不可用的 runner context 导致整个 native workflow 无法创建 job。官方 ONNX Runtime 1.28.3 包含 1.28.2 之后的模型校验与内存安全修复，测试优先使用该补丁版；真实包仍须通过 release digest 核验。

### 原生离线 OCR 资源安装

新增 `prepare_source_inventory.install_ocr_package(package_manifest, expected_package_sha256=..., store=...)`。独立可信 SHA-256 绑定清单；所有声明的 tiny/small/medium、运行库及依赖、辅助模型逐一检查，包括当前机器不会加载的模型。仅复制被声明的常规文件，拒绝目录穿越、链接、冲突及额外文件；暂存目录校验完后发布到 `store/<manifest SHA-256>/package.json`，同包可复用，不覆盖或悄悄修复旧资源。运行时依赖列表 `runtime.dependencies` 纳入哈希核验。支持已有离线可信包，不包含自动下载、签名发布、GPU 探测或合格模型目录。用户无需 Python 执行本安装接口。新增真实文件的安装、复用、篡改与未选模型负例。

### 可选表格结构及单元格证据

新增 `recognize_ocr_table_png` 和 `recognize_pdf_table_region`。显式传入表格截图或原始 PDF 的未旋转点坐标，使用经绑定的 `auxiliary.table_structure = {"architecture":"SLANet_plus","model":{path,sha256},"dictionary":{path,sha256}}`；可选择 `cell_mode="wired"|"wireless"` 使用独立 RT-DETR-L 单元格检测模型（`auxiliary.table_cells_wired` / `table_cells_wireless`）。默认 cell_mode=off，不自动加载表格模型。原始 HTML 仅作为数据，不执行或写入最终 Markdown；有限语法解析记录明确 rowspan/colspan、空位及上游可能省略的终止容器标签。缺失行/单元格、越界、非法属性、重叠 span 均阻断，不猜补。上游可能产生数量或位置不匹配的结构框，始终保留独立框数组，不按索引与结构单元格强配。RT-DETR-L 框只与完全且唯一包含的 OCR 行形成几何候选，跨格/未归属读法全部保留。PDF 接口同时映射独立结构框、检测框及正文到原始未旋转坐标。没有自动 Rich Table 输出、合并格匹配验收或真实模型测试通过的声明；表格模型本身须单独安装、实测和复核。

### OCR 编译修复、独立复核与公式候选

修正 PP-DocLayoutV3 的上游真实 API：`LayoutBox.class.semantic()`，同时保留原始 25 类标签和简化语义类。旧 CI run 45408421711 的失败原因是 `semantic_class()` 不存在，以及 Microsoft v1.28.3 GitHub release API 返回 404。CPU smoke 改用 Microsoft 官方 PyPI `onnxruntime-1.32.0-cp312-cp312-manylinux_2_28_x86_64.whl`，独立固定 PyPI 公布的 SHA-256 `99e1074c4b2197accd5073052a465361be17b89c093b8ffb6e6bd8f986c52df8`，仅提取经校验的原生 CPU .so，不安装/执行 Python wheel。允许明确列出的 1.28.3 或 1.32.0 CPU 候选运行库，Rust `ort` 仍请求 API 28；实际兼容性需要测试，不把候选称为已合格运行时。

新增 `audit_ocr_evidence(primary_record, expected_primary_sha256=..., comparison_record=..., expected_comparison_sha256=...)`：重新读取并绑定原 PNG、源 PDF（存在时）、模型包及实际权重；仅比较同一实际栅格的候选，双向唯一几何匹配保留原始文字、分数、坐标，记录数值、希腊字母、正负号、上下标及低分，所有文本和特殊版面区域均生成待复核任务；不因两份结果一致而验收通过，不假装调用 Jev，未匹配文本不丢弃。当前接受原生 OCR 证据格式，独立增强模型还需要统一输出适配。

新增显式 `recognize_ocr_formula_png`、`recognize_pdf_formula_region`，复用同一固定 ppocr-rs 的实验性 `PP-FormulaNet_plus-L` 识别器（`auxiliary.formula={architecture,model,tokenizer}`，架构值为 `PP-FormulaNet_plus-L`）。仅在用户/调用方显式选择一处公式时运行，不随正文自动加载；CPU 内存保护暂定总内存 16 GiB、可用内存 6 GiB，限制模型、词表及裁剪尺寸，要求真实 tokenizer 并禁止隐式 fallback。原始公式图、PDF 坐标、候选 LaTeX、模型分数及资源哈希分别记录；上游 decoder 最多 2560 步，终止完整性、线程限制、实际模型精度和运行时上限仍未验证，不执行或自动写入 LaTeX。该轻量集成是明确的可选备选方案，不替代尚待集成的 MiniCPM-OCR/Jev。

### 原生图像导出与裁剪边界候选

新增 `pdf_image_objects.export_image_object`、`inspect_crop_boundaries`、`render_page_region`。复用同一已锁定 craft-pdf filters 严格解码、限制实际像素和工作量；实际调用 XRef/generation、页面矩阵、原文件哈希、原 JPEG 字节、8-bit RGB/Gray 样本及同尺寸灰度 SMask（含反转 Decode）均留证。未知色彩空间、特殊显示状态、显式 Mask/Matte/Decode、旋转、非均匀缩放及未支持编码要求页面渲染。边界检查使用实际 PdfCraft glyph 行和图像调用几何，输出独立原始上下文与红/绿标注，建议裁剪框不自动应用，四边待审。glyph 几何不等于 painted ink，不能声称与旧 PyMuPDF 完全一致。新增原始字节、Alpha、错误扩展名、未调用 XRef、覆盖及实际 crop 负例。锁文件复用现有 craft-pdf 节点，仅加入 root 依赖边，仍以 CI --locked 验证。

1364743：真实 Linux x86_64 CPU OCR（中英文和数字）通过，7.22 秒；原生五平台通过，macOS ARM64 被取消；差分 264/265 通过，唯一错误是 debug process_markdown 超时。改用 release 二进制差分与发布门禁，保留 45 秒超时及完整原用例。新图像候选尚未获得编译和实际回归资格。MiniCPM/Jev、GPU 回退和完整自动下载仍待实现。

### PDF/DOCX 混合来源清单与真实原生重放

`prepare_source_inventory.prepare_sources` 与 `complete_workflow.initialize_workflow` 现支持 PDF/DOCX 混合来源，可选 `ocr={package_manifest,expected_package_sha256,profile,layout}`，正常文字页不会启动 OCR。每页保留 PNG、RGB 像素哈希、实际 PdfCraft glyph JSON、原生行块、结构索引、来源绑定及待审的 OCR 候选。扫描页无可信模型时记录 `ocr_resources_required`，仍保留完整实际页面和图像调用，必须通过逐页补充对象复核。原生行 ID 明确 `-pc-line-`，不冒充 PyMuPDF 块编号；OCR 候选不混进原生文字块。实际调用 XRef 以 generation 区分，同一对象在来源内保存一次编码流；仅严格支持的独立对象导出 PNG/JPEG，否则显式要求区域渲染。草稿队列冻结新增证据，在当前页面复核/完整草稿恢复时验证绑定。`prepare_docx_sources` 仍为原有 DOCX-only API。

新增 `validate_delivery.replay_native_pdf_inventory`，由同一固定 PdfCraft 实际重新提取文字、glyph/line、重新渲染页面与比较 RGB 像素/PNG 哈希、重新解析图像调用及重新导出独立对象样本；检验 OCR 候选的实际栅格、原 PDF 与模型资源绑定，不虚构 OCR 正确性。`check_source_provenance` 对新原生 PDF 自动使用重放子集，仍保留独立视觉验证和完整交付门禁。重放不是跨引擎准确率验证。新增 PDF 双页与扫描页、篡改、未定位字形和图像对象证据测试。当前完整工作流仍缺 post-draft handoff generation、实际 mapping rerun 和 final validation；不提高任何 release qualification 状态。

### 原生 handoff maps 与实际 mapping rerun

实际 rerun 可读取未写 `schema_version` 的旧版任务清单；显式不受支持的版本仍拒绝。原运行清单、清单内来源、候选 Markdown 和基线映射在验证前就绑定到具体文件和 SHA-256，同一文件在验证期间被重新保存也不能被当作原始基线。差异依主文献、补充材料、链接以及对应记录组顺序输出，组内按身份排序。只读取两个现成文件集的 `compare_bound_maps` 仍不视为实际 rerun。

新增 `handoff_maps.write_handoff_maps`、`handoff_maps.validate_handoff_maps`、`mapping_rerun.record_mapping_rerun` 原生候选实现。真实绑定来源清单、候选 Markdown、显式 delivery selector 及源块身份，生成主文献/补充材料/链接视图；检查来源集合、块覆盖、排除、引用目标及资源哈希。支持 anchor、唯一 LF quote、LF 行范围 SHA-256；输出原始 Unicode code-point、CRLF 和精确 snippet 哈希。枚举 Markdown、Wiki、reference、HTML、autolink、裸 URL 和 Rich Table v2/v3 scalar links；表格保留 JSON pointer 和 value SHA-256，不能唯一恢复序列化位置时要求明确完整 fence 归属。未支持的多行引用定义、HTML 媒体等显式阻断。生成记录保持 pending/extracted，无真实库匹配或观察推断。校验重新从实际绑定输入与保留的 delivery 推导预期记录并检查全部分类集合；旧版手工精简/合并块表示及复杂 locator 尚未全部迁移。实际 rerun 创建新目录、重新生成并校验映射，再比较稳定字段，保留原映射和实际输入绑定，不写验收通过。新增合成 PDF、CRLF/Unicode、覆盖缺失、篡改、真正 rerun 与 Python 主/补充材料稳定投影差分测试。尚待编译及历史回归，全部正式发布门禁保持。

### 原生草稿后工作流审查与交付验收候选

新增 `complete_workflow.write_maps`、`inspect_workflow`、`require_completed_records`、`validate_delivery.validate_delivery`。工作流审查核对原始 skill 文件集合/哈希、来源身份/角色/范围、必需记录、命名、原子检查账本、交付清单绑定、映射、翻译及版式 disposition。resume 在草稿完成且候选存在时返回实际首个待修记录；缺失记录按原依赖顺序处理。交付验证调用已迁移的来源重放/OOXML 证据、实际候选目标和映射推导，检查来源页/块/媒体覆盖、元数据有无、来源 URL、Rich Table v2/v3 worksheet/尺寸/merge、公式/图像/引用/标题目标、实际本地图像解码与资产绑定、术语、翻译、原子检查账本，并检查图件快照、来源位置、28 项原子目标及逐资产四边记录。现阶段较严格要求显式映射契约；未支持的旧版手工精简映射及一些复杂候选布局仍需补齐。`NATIVE_DELIVERY_PARITY_PENDING` 仍明确阻断完成；即使 subset 无误也不标记最终交付通过。新增缺失记录、来源空集及篡改清单的原生回归测试，所有新代码等待 CI。上一轮 27c653a 的 266/266 差分、真实 CPU OCR 和安装器检查已通过。

### 补齐公开生产接口（候选）

新增 ReviewWriter 的 scope/exclude/source_issues_review/terminology_review/review/unit，UnitReview 的 check/scan/occurrence/term/issue，TermReview 的 search/search_status/decide 共 14 个独立 native 操作。接口名保留原类名，调用方式为 prepared_path、已有 decisions 和各方法显式 kwargs；从冻结准备记录重放并真实执行，不接受伪造的可变 review ledger。单元 handle 重复获取保持幂等；实际 search 返回 method_result 中的真实命中，并保留完整新决策日志。新增 render_note.keys/line/digest/canonical/literal/template_fields、glossary_candidates.GlossaryIndex.search/suggest、check_records.AtomicRule.catalog_entry/location 和 prepare_source_inventory.pdf_page_rect/valid_pdf_bbox。所有接口复用当前 native 核心实现，不使用 Python。新增公开渲染辅助函数和实际旋转 PDF 几何差分测试、拒绝缺参和保护字段测试。CLI/progress、报告对象访问器、ImageObjectIndexer 的带对象目录完整行为，以及复杂 JSON 浮点规范化等仍需审查；不改正式资格和发布门禁。

### PDF 对象索引器与逐方法回归

新增 `pdf_image_objects.ImageObjectIndexer.index_page` 原生接口：明确 PDF 与实际页码，可选 object_directory，按源文件 SHA-256 隔离缓存；真实间接调用 XRef+generation 对象与 SMask/Mask 编码流独立绑定，重复相同字节安全复用，未支持的 mask/codec/显示效果保留明确待审原因，严禁覆盖不同旧证据。并保留严格支持的 JPEG/PNG 独立导出候选；不声称等价于旧版每个 raw_asset 格式或可见像素验收。新增实际 JPEG/Flate/SMask 夹具的逐页索引与复用测试。新增 14 个原生翻译复核方法的参考决策逐条执行差分（真实查询、单元绑定幂等、对照 Python 决策日志和数据）。上一轮 58ad068 CPU smoke 已通过；其余任务因新提交取消，等待本提交的完整 CI，保持全部发布门禁。

### 全公开接口覆盖与原生离线候选安装

修复 5322d80 在 CI 中的两个真实 E0599 编译错误（render_note.keys 缺少错误传播）。剩余报告对象、进度与脚本入口共 27 个公开符号新增真实 native 调用；报告方法根据当前实际文件重新执行相应校验器并投影，不接受调用方伪造的通过报告。旧 main 的可编辑脚本全局变量改为显式 JSON 参数，退出语义作为 exit_code 字段返回；不运行 Python。进度条输出 stderr，保持 stdout JSON 协议。此轮针对上次 AST 报告列出的全部缺口，最终以新编译二进制的门禁结果为准。

新增 native_runtime.package / inspect_bundle / install 及 z2o package、z2o install 原生入口。从实际仓库 12 个技能目录与当前预编译程序构建独立候选目录，包清单逐文件 SHA-256、大小及 Unix 可执行位，绑定 OS/CPU 架构、当前运行器哈希及独立可信清单哈希。拒绝链接/特殊文件、路径穿越、大小写/Unicode 冲突、额外或缺失文件；先完整暂存、复核，再提交目录，保留本地旧技能的原始备份和事务记录；重复同一包跳过。安装后的每个 SKILL.md 在 YAML 题头后插入明确 native-only 守则；旧 .py 仅在开发仓库和既有安装的事务备份中保留，不再复制到原生候选技能包；不能作为执行路径。实际外部 Zotero/Obsidian、OCR 资源自动分发、第三方许可证和正式发布仍待验收，不宣称已替代外部技能。CI 五个支持平台新增预编译实际仓库候选包、空 PATH 安装、安装后 describe，以及合成夹具的升级备份、重复安装、篡改拒绝和实际 invoke。避免同一 PR push/PR 双跑，并不取消已开始的五个支持平台验收。当前仅保存安装回执，不发布二进制安装包或 2.0.0。

### 原生 Zotero 读取、库内身份索引与交付缺口

新增 `zotero_local.inspect_item(library_type, library_id, item_key, storage_root=..., attachment_paths=..., output_path=...)`，与 `zotero_local.catalog(library_type, library_id, output_path=...)`。按 Zotero 官方 Local API v3 文档（https://www.zotero.org/support/dev/web_api/v3/local_api），仅访问本机 loopback 23119 的 GET 请求，拒绝重定向、异常帧、重复分页、身份变化和超预算响应。不执行附件下载或写入。保留原始条目、直接子项、附件子项、集合父链、声明 MIME/语言、本地明确附件字节与 PDF/DOCX 解析状态；语言和主全文版本仍待人工/模型核对。0 仅作为本地用户路由，不当成跨设备身份。新增实际 loopback HTTP 模拟服务测试，覆盖分页、父链和无下载，不声称连接真实 Zotero 实机。

新增 `vault_library.index(vault_root, literature_relative, excluded_relative=[], personal_library_id=..., output_path=...)` 及 `vault_library.match_reference(index_path, reference, expected_index_sha256=..., catalog_path=..., expected_catalog_sha256=...)`（JSON kwargs 或显式命名输入）。扫描已确认文献根下真实笔记，排除模板/附件/工作稿等目录并保留无法读取记录；仅使用末尾原条目 URI，不将正文引用中的 DOI 当作笔记身份。结合实际绑定 Zotero catalog、完整题名、全部作者、年份、文献类型及 DOI；同 DOI 冲突、重名、重复笔记保持歧义。个人库映射需明确实际库 ID，唯一候选仍须独立版本确认。空结果只针对已绑定快照，不证明当前实时库无文献。真实笔记读取和篡改负例测试。两项均不修改真实库，正式桌面集成和提交链仍需资格验证。

补齐 author affiliation_numbers / affiliation_units、未编号及缺失理由、重复关系、同源归属，以及实际 bibliography call 的独立 citation unit 和重复参考锚点约束。当前 `NATIVE_DELIVERY_PARITY_PENDING` 等最终阻断仍保留。442f0a0 实际 CI 编译后二进制覆盖 23 个参考模块、0 缺失公开符号，差分和 CPU OCR 通过；本次新增功能仍须 CI 验收。

### 实际 CPU OCR 平台资源门禁

`native/ocr-cpu-assets.json` 固定 Microsoft 官方 PyPI 1.32.0 CPU wheel 发布页的实际文件名和 SHA-256：Linux x86-64/ARM64、Windows x86-64/ARM64、macOS ARM64。仅提取明确定名的独立原生库及依赖，若 wheel 缺少独立库则测试失败，不安装或导入 wheel。新增四个真实目标的 CPU inference CI，复用成功的 Linux x86-64 扫描版 PDF、PNG、原始模型 revision 与完整资源哈希，使用同一个中英文验收测试及空 PATH、禁用 GPU 的真实程序。该资源目录与五个支持目标完全一致，不包含 Intel macOS 或 Rosetta 变体。所有额外 OCR 平台是待验收候选，实际完整模型包、生产分发与许可证尚未资格确认。来源：https://pypi.org/project/onnxruntime/1.32.0/#files 。

### 原生写库预检与逐模块验收报告

新增 `vault_publish.plan(run_path, expected_run_sha256=..., vault_root=..., literature_relative=..., assets_relative=..., note_relative=..., expected_existing_sha256=..., inspect_workflow=true)`。使用明确真实 vault 根及原生 run.json，逐级检查实际路径、Unicode/case 冲突、链接和设备名，绑定工作区、候选 MD、源附件、实际嵌入资源和既有目标字节；工作区及候选资源须位于库外。仅生成只读计划，不创建任何 vault 目录或覆盖文件。相同字节资源仍需归属核验，冲突必须规划新路径。写前调用实际完整工作流检查，不以预检替代用户授权或最终验收；尚未实现正式 vault 提交事务、回滚与 Zotero note 写入。`scripts/check_native_release.py --report-path` 直接保存真实编译二进制和参考 AST 对照的 23 模块完整 JSON，CI 上传 native-module-progress 回执，所有原有资格/版本门禁不变。

### Markdown 原生提交和回滚

新增 `vault_publish.commit_markdown` 与 `vault_publish.rollback`。明确授权、候选及目标实际 SHA-256（null 表示新建）、嵌入资源和库外备份目录。资源先独占创建，已有资源不覆盖；Markdown 同文件系统暂存、原子替换，原字节备份、事务日志与实际回读均保留。回滚只恢复确认仍与本次写入一致的 MD，保留后续修改及可能共享的资源。native 锁只协调共享备份根的协作进程；中断且无完成记录时保留待核。standalone_markdown 不替代完整科研交付，complete_workflow 仍严格执行既有门禁。无需 CLI 或 Python。新增真实文件回归测试，尚待 CI。

### MiniCPM-OCR、Jev 与隔离的 GPU / CPU 回退

固定上游 OpenBMB/MiniCPM-OCR c21a3ca524263ef8166cb6d350864b10c3056b61，按模型固定 llama.cpp 发布版。`prepare_source_inventory.recognize_minicpm_region` / `recognize_minicpm_pdf_region` 直接使用官方 GGUF（main + mmproj；Q8 可选匹配 DSpark draft），由 Rust 启动哈希绑定的本机 llama-server，使用 loopback `/v1/chat/completions` 的实际 Text/Table/Formula Recognition prompt。`score_jev_region` 使用同一原图、实际候选文本文件及单独绑定的候选渲染图，直接调用 `/v1/systemone`，按类别名读取 0/1 概率，不再 softmax。不需要官方 Python SDK；Jev 候选内容渲染需要独立实际图，当前不内置公式/表格渲染器。`inspect_minicpm_package` 核对 manifest 和全部运行库/模型，不启动推理。MiniCPM package schema 1 包含 engine=llama.cpp, release=b11901（minicpm-ocr）或 b11928（jev）, model_kind=minicpm-ocr/jev, platform, architecture, cpu/gpu={directory,executable,variant,files:[{path,sha256}]}, model/projector/draft={path,sha256}，cpu 必需、gpu 可选。GPU 尝试独立进程，要求实际启动日志中正数层 offload，失败/缺资源回到同权重 CPU；CPU 显式禁 GPU/投影 offload。GPU node placement/实际提速不由报告自证。

PP-OCRv6 `recognize_ocr_png_accelerated` 使用独立 z2o worker；CPU 与可选 GPU package 明确绑定同一 detector/recognizer/dictionary/aux hashes 和 profile，GPU manifest runtime.execution_provider 可为 Linux CUDAExecutionProvider、Windows DmlExecutionProvider、macOS CoreMLExecutionProvider。实际 EP availability / registration error-on-failure；GPU worker 失败/崩溃/超时重新启动 CPU worker，不伪造节点放置。CI 真双语 OCR 增加故意无效动态库的 GPU worker 实际失败再走同权重 CPU 识别测试。已有默认 PDF/OCR 路由仍为 CPU；显式区域增强接口待完整工作流集成。新增官方 b11901 五个支持平台 CPU 及 Vulkan/Metal archive SHA-256 目录和实际 native server 启动矩阵。Windows 原生编译成功后的安装失败为哈希参数异常，改用 stdin SHA-256 并校验格式。MiniCPM/Jev 的实际模型测试仍分别在 Linux x86-64 和 Apple Silicon macOS 上运行。真实 MiniCPM/Jev 权重推理和真实 GPU 加速仍需单独验收。用户资源自动下载未完成。

Jev 运行器单独固定为 llama.cpp b11928（提交 7eed6b7685d2abc32182c0df330812e3848b862f），普通 MiniCPM-OCR 仍为 b11901。包清单按模型种类校验 release，交叉声明明确拒绝。native/jev-runtime-assets.json 固定 Linux x64/ARM64、Windows x64/ARM64、Apple Silicon 五种 CPU 压缩包的 SHA-256 和大小，不为 Intel Mac 提供 Jev 候选。Linux x64 已用同一官方 Q4 模型及投影器对照：旧版返回 unsupported decision model type，新版三份官方样例通过；其他平台的哈希核对不是实际推理验收。Linux ARM64 压缩包仍依赖 GLIBC_2.38。启动失败报告只保留限量日志识别的固定错误类别与退出码，不保留原始日志、令牌或启动参数。

### 原生候选包排除旧 Python 文件

候选包生成和读取均强制排除旧 Python 文件（`*.py`、`*.pyw`、`*.pyc`、`*.pyo`、`*.pyz`、`*.pyd`、`*.pyi`、`__pycache__`，路径按 ASCII 大小写无关匹配）。包清单新增 `legacy_python_files_included=false`；即使重新计算合法清单哈希，含旧 Python 路径的清单仍拒绝安装。保留非 Python 资料及旧安装的完整事务备份；源仓库 Python 仅用于历史差分。旧候选包须用新版二进制重新生成。该变化只解决交付载荷残留，不代表历史行为等价、旧文档示例全部转换或完整发布资格通过。

### Remaining seven runtime modules: additional native implementation

The native implementation now additionally supports historical merged source-block partitions with exact source locators, joiners, original text digests, kind and target-unit coverage; link origins resolve through those partitions. Mapping reruns validate the actual original source paths, declared IDs/roles and independently regenerated evidence, and include the block partition in stable comparisons.

PDF extraction additionally handles packed 1/2/4-bit grayscale/RGB sample rows and explicit identity/inverting Decode arrays in the supported native extraction subset. The inventory preserves separately bound indirect mask stream evidence and replays it against the original PDF, including when standalone extraction is not appropriate. Source preparation rejects ambiguous roles, duplicate canonical attachments and case-conflicting source IDs before writing output.

Delivery and workflow verification bind the same manifest/map target and worksheet selectors, retain the original resource root, reject escaped or stale assets, and recheck evidence across stages. Translation decision drafts may be restored with an explicit `expected_draft_sha256`; input bytes and review publication are checked again before returning validation results.

These changes cover all seven previously partial module areas, but are implementation increments, not evidence of complete historical behavior parity. Keep the existing release gates and module qualification states until native compilation, reference comparisons, mixed-source acceptance and the required five-platform execution results establish them. Unsupported PDF display/codec cases remain explicit, rather than silently falling back to Python or certifying semantic correctness.

### Linux ARM64 MiniCPM 兼容性边界

Ubuntu 22.04 ARM64 的运行验收不可用 Ubuntu 24.04 替代。上游 b11901 的 Ubuntu ARM64 CPU 发布档由 24.04 ARM64/GCC 14 构建，在 22.04 上实测缺少 `GLIBC_2.38`、`GLIBCXX_3.4.32` 与 `CXXABI_1.3.15`。CI 为 22.04 ARM64 单独从该版本固定源码 `5320c87ff0d994845b44ce7976f10055ec697447` 构建通用 ARMv8-A CPU 后端，并在原生 22.04 上检查 ELF 版本需求、无额外动态库路径的 `--version` 启动及产物哈希，不回退到官方档、其他版本或其他运行器。若 CI 通过，仅说明本次尝试的源码构建产物可启动；真实 MiniCPM/Jev 推理、GPU 后端、安装与再分发仍需独立核验。

### Bounded local SVG delivery validation

Figure delivery recognizes local `.svg` files case-insensitively and binds the exact bytes it validates. The native vector validator uses pinned resvg 0.50.0 without default font, raster-image, or SVGZ features. It draws a bounded in-memory raster to require a real drawable image; it does not replace the SVG or certify visual/source fidelity. Standard static shapes, groups, transforms, local gradients and clips, and direct local shape reuse are supported. Fonts/text, stylesheet elements, images, filters, masks, animation, foreign elements, DTD and custom entities, processing instructions, and external or data references fail explicitly rather than disappearing during rendering. Compact inline presentation styles are permitted within the supported shape subset.

The file stays inside the manifest resource root and the rasterizer has no image resource resolver. Budgets limit bytes per SVG and delivery, number of distinct SVG assets, XML nodes and depth, geometry and `use` expansion, effect nesting, intrinsic canvas size, and actual render dimensions. Existing native delivery release and independent-review gates are unchanged.
