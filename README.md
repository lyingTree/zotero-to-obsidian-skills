# Zotero to Obsidian Skills

将 Zotero 文献全文整理为 Obsidian 文献笔记。外文默认译为中文，指定“保留原文”时跳过翻译。主技能负责从条目和附件到正式笔记的完整流程，专项技能可单独处理全文提取、翻译、排版、笔记属性或链接。整套技能共用 Rust 运行器 `z2o`。

## 项目特色

- **覆盖全文处理全流程。** 从 Zotero 条目和附件开始，完成全文提取、按需翻译、图表与公式整理、引文链接、Obsidian 写入和回读验收。
- **专用库解析文档。** PDF 使用 `pdf-inspector`、`lopdf` 和 PdfCraft 处理文本、字体映射、版面和页面渲染。DOCX、EPUB、ODT 使用 `zip` 读取文档容器，按格式处理内部 XML 或 HTML。扫描页和文本异常区域按需使用 OCR。
- **Rust 原生批处理。** `z2o` 在本地批量处理文档、裁剪图件、规范化格式、匹配链接和校验文件，减少逐项调用和中间数据传递。
- **支持自定义术语表。** 筛选本篇文献涉及的词条，供翻译和审校统一专业译名。
- **严格验收。** 对照来源复核译义、图表、公式与引文身份，写入前检查笔记和引用资源，写入后回读并核对 SHA-256。
- **节省 token。** 文档解析由 `z2o` 内置的专用库在本地完成，Agent 直接使用提取结果、处理摘要和待核位置。专项规则按任务加载，术语表按篇筛选。

## 技术栈与处理流程

**Zotero 条目与附件 → 全文与版面解析 → 按需 OCR 和图表提取 → 保留原文或按术语表翻译 → Markdown 规范化与链接匹配 → 写入 Obsidian → 回读验收**

| 环节 | 技术与职责 |
|---|---|
| 任务编排 | Agent Skills（`SKILL.md`）定义处理顺序和验收要求。Agent 按用户要求选择是否翻译，核对需要语义判断的内容，并调用 `z2o` 执行本地操作。 |
| 文档解析 | Rust 使用 `pdf-inspector` 和 `lopdf` 提取与检查 PDF 文本及字体映射，使用 PdfCraft 渲染及裁剪页面。DOCX、EPUB、ODT 使用 `zip` 读取容器，DOCX 等 XML 内容使用 `roxmltree` 解析。文本编码处理使用 `encoding_rs` 和 `chardetng`。支持 HTML、TXT、Markdown，符合条件的 arXiv 文献可使用 TeX 源码辅助。 |
| OCR 与视觉复核 | `ppocr-rs` 通过 CPU ONNX Runtime 运行 PP-OCRv6 及方向、版面、表格模型，可生成拼图供 Agent 核实疑难区域。 |
| 格式与验收 | Rust 批量处理 Markdown、图表公式编号和链接，执行发布前检查、文件写入以及 SHA-256 回读校验。 |

### 依赖说明

| 依赖 | 用途 |
|---|---|
| Zotero Desktop 本地 API | `z2o` 直接读取条目、集合和附件信息。 |
| Obsidian 库目录 | `z2o` 直接写入并回读核验笔记和资源。 |
| `pdf-inspector`、`lopdf`、PdfCraft | PDF 文本与字体映射检查、版面处理及页面渲染。 |
| `ppocr-rs`、`ort` | 通过 ONNX Runtime 运行 PP-OCRv6 及方向、版面和表格模型。 |
| `zip`、`roxmltree`、`encoding_rs`、`chardetng` | 文档容器读取、XML 解析和文本编码处理。 |

Rust 库随预编译运行器提供，ONNX Runtime 和模型由包含 OCR 的发布包提供。[Rust 依赖](rust/Cargo.toml)

## 安装

预编译包支持 Windows x64 / ARM64、Linux x86-64 / ARM64（GNU）和 macOS Apple Silicon。将示例路径换成 Agent 实际读取的 Skills 根目录。

**Windows**

```powershell
.\install.ps1 -SkillsRoot "D:\YourAgent\skills"
```

**Linux / macOS**

```bash
./install.sh "/path/to/your/agent/skills"
```

脚本下载并校验平台包，安装技能、共享运行器和随包 OCR 资源。中国大陆出口 IP 使用 CNB，其他地区或探测失败时使用 GitHub。可通过环境变量 `Z2O_NETWORK_REGION` 手动指定地区，例如 `CN`。每次启动任一技能都会核对该技能声明的版本与本地 z2o 版本。缺失或不符时，自动下载、校验并切换到对应版本。[CNB 发布包](https://cnb.cool/lyingTree/zotero-to-obsidian-skills/-/releases) · [GitHub 发布包](https://github.com/lyingTree/zotero-to-obsidian-skills/releases)

## 模板与术语表

默认位置为 **Obsidian 库根目录下的 `template/`**，默认术语表为该目录下的 `glossary.md`。Agent 首次配置时会显示实际绝对路径，请用户选择默认路径或分别指定模板目录与术语表文件。已有文件会被复用，默认资源只补齐缺失项。确认的路径保存在主技能的 `user/profile.json`。

| 资源 | 用途 |
|---|---|
| [期刊论文模板](literature-zotero2obsidian/template/journal-article.md) | 期刊论文笔记 |
| [学位论文模板](literature-zotero2obsidian/template/thesis.md) | 学位论文笔记 |
| [会议论文模板](literature-zotero2obsidian/template/conference-paper.md) | 会议论文笔记 |
| [预印本模板](literature-zotero2obsidian/template/preprint.md) | 预印本文献笔记 |
| [中英术语表](literature-zotero2obsidian/template/glossary.md) | 专业术语、约定译名和核对说明 |

## 使用

整理整篇文献时，向 Agent 提供 Zotero 条目标题或条目键和目标 Obsidian 库，使用 `literature-zotero2obsidian`。如需保留原文，在请求中注明“不翻译”。只需完成某一项处理时，直接调用对应专项技能。

如需将笔记同步为 Zotero 条目的子笔记，在请求中明确说明。此功能需要 Zotero 10 或更新版本、启用本地 API，并在 Zotero Desktop 中确认写入授权。

| 技能 | 职责 |
|---|---|
| `literature-zotero2obsidian` | 编排完整流程，支持分步处理与断点续做 |
| `literature-inspect-zotero` | 读取 Zotero 条目、集合和附件状态 |
| `literature-fulltext-extract` | 解析全文，按需执行 OCR 和断行修复 |
| `literature-translate-zh` | 按术语表翻译和复核全文 |
| `literature-write2obsidian` | 规划目录、写入、回读与恢复 |
| `literature-filename` | 规范文件名与处理命名冲突 |
| `literature-note-properties` | 整理 Obsidian YAML 属性 |
| `literature-header-format` | 整理标题、作者、单位和通讯作者信息 |
| `literature-body-format` | 整理正文、图表、公式和书目编号 |
| `literature-internal-links` | 建立图表、公式和引文的篇内链接 |
| `literature-library-links` | 将参考文献链接到库内已有笔记 |
| `literature-supplement-materials` | 提取、整理和翻译补充材料 |
