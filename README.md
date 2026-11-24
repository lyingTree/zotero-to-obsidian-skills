# zotero-to-obsidian-skills

将 Zotero 文献全文处理为 Obsidian 中文文献笔记的 Agent 技能包，含 12 个 `literature-*` 技能，另需两个外部依赖技能。适用于支持 Agent Skills（`SKILL.md`）的宿主。

## 组成

主入口：`literature-zotero2obsidian`（编排完整流程，可按步骤/断点执行）。

| 技能 | 职责 |
|---|---|
| `literature-zotero2obsidian` | 主编排：Zotero → Obsidian 全流程 |
| `literature-inspect-zotero` | 读取条目、集合、语言、附件状态 |
| `literature-fulltext-extract` | 多格式全文提取、按需 OCR、断行修复 |
| `literature-translate-zh` | 按术语表翻译为中文 |
| `literature-write2obsidian` | 目录规划、提交、回读与恢复 |
| `literature-filename` | 文件名规范与冲突处理 |
| `literature-note-properties` | Obsidian YAML 属性 |
| `literature-header-format` | 标题、作者、单位、通讯身份 |
| `literature-body-format` | 正文排版、图表公式、书目编号 |
| `literature-internal-links` | 篇内图/表/公式/引文链接 |
| `literature-library-links` | 参考文献指向库内其他文献的出链 |
| `literature-supplement-materials` | 补充材料（SI）提取与翻译 |

## 依赖

以下两个技能由安装脚本从各自上游仓库自动获取，不随本仓库分发。默认版本锁定在 [dependencies.lock.json](dependencies.lock.json) 中的完整提交 SHA；更新时显式更换锁定版本或指定覆盖 ref：

- `obsidian-cli` — 通过 `obsidian` CLI 操作库；来源 [obsidianmd/obsidian-skills](https://github.com/obsidianmd/obsidian-skills)（`plugins/obsidian/skills/obsidian-cli`）
- `zotero-skills` — Zotero 条目与库管理；来源 [WenyuChiou/zotero-skills](https://github.com/WenyuChiou/zotero-skills)（`skills/zotero-skills` + 根 `scripts/`）

## 模板与术语表

`template/` 提供文献笔记的默认资源，复制到你的文献根 `template/` 目录使用：

- `期刊论文.md`、`学位论文.md`、`会议论文.md`、`预印本.md` — Templater 模板，按 Zotero 条目类型创建文献笔记（依赖 Templater、Rich Table 插件，书目从 Zotero 本地 API 读取）
- `术语表.md` — 文献翻译用的中英术语表与译名核对说明

## 安装

### Rust 原生迁移候选

参见 [Rust 原生迁移](literature-zotero2obsidian/references/rust-native.md)。直接迁移至原生 Rust，不嵌入 Python、不自动回退。此分支仅完成部分操作及六种平台/架构的测试配置，尚未达到全流程发布门槛；现有安装说明仍为 1.1.0 参考实现。开发者可运行 native crate 的测试和 release 构建；未迁移的操作明确失败。

**原生 2.0.0 的目标平台**：Windows 与 Linux 各支持 x86-64 和 ARM64；macOS 仅支持 Apple Silicon（ARM64）。Intel Mac 不在原生安装包、OCR 或验收范围内。以下候选状态与旧版 Python 安装方式不受此范围调整影响。

原生迁移分支新增仅用于候选验收的离线安装器：预编译 z2o 可构建含 12 个技能的哈希绑定候选目录、核验清单并安装，保留原技能备份，不需要 Python 或 Git 执行安装。实际全流程、外部 Zotero/Obsidian、OCR 分发、平台与许可证资格仍未通过；不是正式用户安装包。

安装后可用**安装目录内**的 z2o（Windows 为 z2o.exe）执行 `verify-install <package.json> <可信清单SHA-256> <skills目录>`，只读复核本包的 12 个技能和可执行文件，无需保留解包后的资源目录。核验失败不会覆盖本地修改；核验成功也不表示该候选已具备正式发布资格。

1.1.0 参考安装方式仍需要 Python 3.10+ 和 Git。推荐在 Python 编辑器中打开 [install.py](install.py)，设置顶部配置后运行脚本，无需拼接命令行参数：

```python
SKILLS_ROOT = Path(r"D:\YourAgent\skills")
INSTALL_PYTHON_DEPENDENCIES = True
# OBSIDIAN_SKILLS_REF / ZOTERO_SKILLS_REF 留空时使用锁定的提交。
```

一次安装本仓库的 12 个技能和两个外部依赖，全部位于目标 Skills 目录的同一层。目标目录不存在时自动创建；默认目标为仓库目录时，已有的本仓库技能原地保留，只安装依赖。运行中的进度会显示在输出中。

安装器先下载、检查全部源文件并在目标文件系统暂存，再提交目录变更。已有技能会保存在 `.z2o-backups/<安装ID>/previous/`；发生可捕获的提交异常时按相反顺序回滚。安装记录保留实际依赖 SHA、目录及备份位置。拒绝覆盖符号链接或非技能目录；不会删除备份中的本地修改。

Python 依赖在发布技能目录前安装。pip 失败会返回失败，不显示安装成功。pip 对当前 Python 环境的包变更不能随文件回滚，建议在所用 Agent 的专用 Python 环境中运行。已经管理好依赖时可将 `INSTALL_PYTHON_DEPENDENCIES` 设为 `False`，相应状态会明确记录。

兼容入口仍保留：[install.ps1](install.ps1) 用于 Windows，[install.sh](install.sh) 用于 macOS/Linux。原有目标目录和版本参数转交同一个 Python 安装核心；ref 可指定分支、标签或完整提交 SHA。

从真实 Zotero 文献库读取时，需要已核实的 Zotero 读取接口；写入现有笔记时，需要目标 vault 的文件访问能力。已有 PDF 的离线整理直接生成 Markdown 与资源文件。验收比较这些文件与来源、参考 Markdown，不要求启动应用或审查应用界面。安装技能本身不会安装桌面应用。Zotero 写入所需凭据从环境变量或本机配置读取，绝不写入仓库。

## 使用

对 Agent 说"把某篇 Zotero 文献整理进 Obsidian"，主入口 `literature-zotero2obsidian` 会按流程调用其余技能。也可单独触发某个子技能（如只改文件名、只建链接）。

完整流程先检查身份和环境，完成命名与目录预规划，再提取、翻译和处理补充材料，随后构建属性、题头、正文及链接，验收后提交并回读。不同条目可以有界并行；同一条目的候选链按依赖顺序执行，写入同一目标时保持独占。

从全文新建笔记时，优先使用 [Python 组装、规范化与验收](literature-zotero2obsidian/references/deterministic-pipeline.md)。来源清单固定内容单元，模型填写译文，Python 统一组装标题、段落、图表公式、书目和链接，并自动规范化、机械验收、保存变换及阶段报告。模型只处理来源理解和程序无法可靠判断的事项，不再手工拼每个空行或重复填写格式自检表。主文与每份 SI 分别核对题名、署名和单位，来源语义结果仍单独报告。

在编辑器打开 [process_note_example.py](literature-zotero2obsidian/references/process_note_example.py)，设置路径即可运行。既有 Markdown 也可经同一套程序规范化；保留原稿，每次变换可重放，重复执行会验证已有文件。恢复旧版工作目录仍使用[原完整入口](literature-zotero2obsidian/references/complete-workflow.md)，不覆盖旧回执。

## 检查与维护

- **程序主入口**：[note_pipeline.py](literature-zotero2obsidian/scripts/note_pipeline.py) 提供任务准备、结构化组装和既有稿规范化。输出 `normalization.json`、`acceptance.json`、`phases.json` 和中文报告；原稿不覆盖，重跑不修改一致的已有产物，冲突明确报出位置。
- **结构化渲染**：[render_note.py](literature-zotero2obsidian/scripts/render_note.py) 校验必需单元与可译字段，固定对象顺序、编号、锚点和源数据。**规范化**：[normalize_note.py](literature-zotero2obsidian/scripts/normalize_note.py) 只改明确可处理的格式，保护代码、链接目标、公式和复杂结构；未知情形保留并定位。**机械验收**：[accept_note.py](literature-zotero2obsidian/scripts/accept_note.py) 直接读文件，核对实际对象、来源合同、单元映射及资源，区分确定失败与启发式提示。
- **完整任务检查**：[complete_workflow.py](literature-zotero2obsidian/scripts/complete_workflow.py) 初始化来源与全量待核清单，收尾时检查必需记录、原始输入范围、实际版本绑定和未完成目标。整份台账未生成也会报出缺项。`records_ready` 不等于来源语义已被程序认证。
- **候选笔记检查**：在 [validate_note.py](literature-zotero2obsidian/scripts/validate_note.py) 中设置 `NOTE_PATH` 后运行。只读检查 YAML、块 ID、本篇断链、Citation Preview 1.3.0 锚点兼容性、Rich Table 和公式锚间距。支持模板的 `---` 外包裹、v2 单表、v3 全部工作表；逐表检查锁定、主题及表内链接，结果附行号和工作表信息。完整范围见[自动检查说明](literature-zotero2obsidian/references/automated-validation.md)。
- **来源交付检查**：按[来源交付索引](literature-zotero2obsidian/references/delivery-checks.md)运行来源准备脚本与 `validate_delivery.py`，检查实际源页、文件哈希、已盘点对象的交付位置及资源文件。来源映射通过不代表译文语义通过。
- **逐项记录**：[逐篇验收](literature-zotero2obsidian/references/acceptance.md)说明自动机械证据和旧版单 ID 台账的边界。程序已执行的检查不要求模型重复自填；译义和图像内容仍保留实际来源核对记录。
- **排版提示**：[lint_presentation.py](literature-zotero2obsidian/scripts/lint_presentation.py) 提示裸邮箱、裸 DOI、常见量值、间距、同图多面板调用、公式尾标点和末尾分隔线问题；逐条核对后修正或保留有依据的例外，不自动全局替换。
- **仓库检查**：在编辑器运行 [scripts/run_checks.py](scripts/run_checks.py)，检查 12 个技能的元数据、相对引用、依赖锁定，并运行隔离回归测试。Python 验证依赖见 [requirements-dev.txt](requirements-dev.txt)；完整回归另需 Node.js 24 执行四份真实 Templater 模板，无 npm 依赖。模板回归使用内存中的应用/API 替身，不连接真实文献库；仅运行候选笔记校验器不需要 Node.js。
- **持续集成**：[GitHub Actions](.github/workflows/validate.yml) 在 Windows/Linux、Python 3.10/3.13、Node.js 24 上运行上述检查，并检查安装包装脚本的语法。

静态检查不能证明全文提取、翻译或图片裁剪正确。机械任务交程序，来源判断和实际译文核对保留明确范围。模型测试按 PDF 提取、翻译、拼接、引文身份、链接、题头、作者单位、图表公式、空行等环节报告具体症状和位置；输出问题不自动归因为某个未观察到的上游过程。版本和回执按[交接约定](literature-zotero2obsidian/references/pipeline-contract.md)保留。

## 规则粒度与模型执行测试

12 个技能的约束与验收均采用独立 ID：`前缀-C编号` 对应约束，`前缀-A编号` 对应验收。每条只表达一个可独立判断的目标。运行记录按 ID 保存结果与证据；路径、哈希、锁定状态、主题等可分别失败的检查各占一条。

仓库检查会报告缺失或重复的规则 ID，并检查其所属技能和复选框结构。语义是否单一仍需逐项审阅。模型执行测试采用冻结的技能与 PDF 输入，在独立子代理中生成候选，再由评审者对照未向执行代理公开的验收样例核验；方法见[技能独立执行测试](evals/README.md)。脚本 CI 与模型实际执行的结果分别报告。

[2026-11-23 的独立执行记录](evals/2026-11-23-low-model.md)：`gpt-5.6-luna / low` 在拆分前后各执行一次，两轮都未达到验收样例。规则拆分与代码测试通过不代表全文、图件、表格和引文的实际交付已经通过。

[同日的模型与推理强度对照](evals/2026-11-23-model-matrix.md)完成8个配置的独立执行。部分high配置的来源内容更完整，但尚未建立全部目标通过的最低门槛；DeepSeek V4.1 Flash因当前接入缺失尚未执行。

[PDF + DOCX 九配置协议](evals/2026-11-24-hu2024-protocol.md)固定比较 `gpt-6-luna`、`gpt-6.1-terra`、`gpt-6.2-sol` 的 low、medium、high。[准备脚本](evals/model_matrix.py)从指定 Git 提交建立独立输入并绑定哈希；[文件检查工具](evals/inspect_markdown.py)保存可复查的结构与资源证据。两个脚本均不代替真实模型调用或对源验收。

[Hu 2024 九组实际结果](evals/2026-11-24-hu2024-model-matrix.md)已完成：冻结版本 `f410b04` 下没有完整通过配置。Sol三档均交齐主要图表书目，但存在明确的源码格式、前置材料位置或题注自链问题；Luna、Terra另有正文或来源保真缺项。两组中断后续接的条件单独保留，过程台账和文件质量分别报告。

## 说明

- 项目根、vault 与文献根在运行时确认后写入 `run.json`。
- 新建主文锚点用 `^ref-N`，单份 SI 用 `^sref-N`，多份 SI 用 `^sref-src<source_token>-N`，其中来源标记按[交接约定](literature-zotero2obsidian/references/pipeline-contract.md#补充材料与书目命名空间)固定，保留可见原号。既有 ID 与链接不因新规则而改写；旧 `^ref-sN` 仅提示插件兼容 warning，真实缺锚、重复和断链仍报告错误。
- 附件采用年份、作者和题名哈希目录；身份或版本冲突时稳定消歧，禁止以同名覆盖不同内容。
- 两个外部依赖按各自上游许可分发，安装时随目录保留各自的 `LICENSE`。
