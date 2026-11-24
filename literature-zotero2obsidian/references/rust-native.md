# Rust 原生迁移

本分支直接迁移到 Rust，不以 PyInstaller 或嵌入 CPython 作为交付方式。Python 1.1.0 保留为开发期参考实现及回归测试工具，原生可执行程序不会启动 Python、执行 Python 脚本、自动回退或用模型判断代替程序验收。

当前为迁移候选，不是完整的用户安装包。native/Cargo.toml 的 0.1.0 是原生引擎开发版本，与仓库 VERSION 独立；完成迁移和发布验收前不修改已发布的 1.1.0，不发布 tag/release。

## 已实现的第一批操作

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
| 其余 18 个生产 Python 模块及上述模块的其他操作 | 尚未完成等价迁移 |

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

新增的 Rust 双遍组装先验证来源计划与翻译槽位并分配编号、锚点，再生成实际 Markdown；对未知/重复/缺失占位符、跨对象锚点、重复标题指向和非法源字段直接阻断。映射使用 Unicode 码点而不是 UTF-8 字节位置，并记录真实 SHA-256。YAML 输出经独立回读核对全部 JSON 数据，保守引号策略不承诺任意 PyYAML 格式字节完全相同；复杂标量、浮点数序列化及旧合同兼容性仍需差分覆盖。完整正文规范化、最终文件验收与写入流水线未迁移，full_workflow_ready 保持 false。

### 规范化迁移验收

Rust 规范化器使用独立词法扫描与原始字节区间，处理 CRLF/LF、转义、嵌套 Markdown 链接、代码、公式、引用定义和 Rich Table。量值、空格、横线与图块间距的修改形成 Unicode 码点坐标的可重放记录。固定点验证不等于来源语义通过；输出路径采用不覆盖的同目录临时文件发布与真实回读。差分测试直接复用现有 Python 的全部规范化样例，并比较实际全文、逐次修改、行列、待核问题和哈希，而不仅比较状态。新增代码待六平台运行及差分验证，不能视作已验收。
