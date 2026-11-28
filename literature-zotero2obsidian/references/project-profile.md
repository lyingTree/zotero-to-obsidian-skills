# 项目配置与运行时发现

项目路径在运行时确认。技能级配置保存于主技能的 `user/profile.json`，任务记录保存本次实际使用的路径。

## Magpie 首次运行

宿主若仍载有旧入口，先确认磁盘上当前的 `SKILL.md` 仍有首次运行标记。清理标记只适用于 `.agent/skills` 或 `.agents/skills` 里的已安装技能。

1. 从当前 `SKILL.md` 的实际目录找同级主技能，兼容 `.agent/skills` 和 `.agents/skills`。按 frontmatter 版本运行 `scripts/ensure-runtime.ps1 -Version <version>`（Windows）或 `bash scripts/ensure-runtime.sh <version>`（Linux/macOS）。脚本校验并安装独立 z2o，启动 OCR 后台下载后立即返回可执行文件路径。直接继续预检、来源准备和撰稿。脚本失败时报告错误，保留引导段供重试。
2. 按下节发现实际 Obsidian 库。调用返回的 `z2o` 通过 `user_profile.load` 读取已保存的配置，显式传入当前主技能绝对路径作为 `skill_root`。按「模板与术语表配置」确认并保存路径。只有本次任务确实需要模板或术语表时，才因路径尚未确认而暂停；独立任务可继续，引导段留待配置完成。
3. 运行器和路径配置均成功后，由 Agent 在同一技能根目录内逐个清理本组 12 个 `SKILL.md`。仅删除从 `<!-- z2o:first-run:start -->` 到 `<!-- z2o:first-run:end -->` 的完整区段及其紧随的空行。每个文件在同目录写入临时文件并原子替换，保留其余内容和换行，跳过没有标记的文件。仅处理本组技能目录内的实际文件。完成后继续原任务；若宿主仍缓存旧入口，以磁盘上的区段和已保存配置为准。

脚本按直连出口 IP 选择 Release。中国大陆使用 CNB，其他地区或探测失败时使用 GitHub。可用 `Z2O_NETWORK_REGION` 手动指定地区。独立运行器、OCR 和 SHA-256 清单取自同一个发布源，后台工作沿用前台选择。下载支持断点续传和无进展超时，日志位于 `user/downloads/<version>/<target>/<cnb-or-github>/`。确需 OCR 时先完成可独立执行的环节，再检查日志和 `runtime/ocr/package.json`。重新运行引导命令可续传。[CNB Release](https://cnb.cool/lyingTree/zotero-to-obsidian-skills/-/releases)

## 模板与术语表配置

1. 对已确认的 vault，优先复用已配置且可用的 `paths.template_root` 和 `paths.glossary_path`。首次配置时，向用户显示实际默认路径并提问：“模板默认放在 `<vault_root>/template`，术语表默认用该目录下的 `glossary.md`。使用默认位置，还是分别指定模板目录和术语表文件？”等待用户确认后初始化。
2. 默认文件位于主技能的 `template/`。在用户选定位置只补缺失文件，保留已有模板和术语表。模板映射使用 `journalArticle` → `journal-article.md`、`thesis` → `thesis.md`、`conferencePaper` → `conference-paper.md`、`preprint` → `preprint.md`，保留用户已有映射。
3. 用 `user_profile.patch` 保存 `paths.obsidian_vault_root`、`paths.template_root`、`paths.glossary_path` 和 `templates`。如果切换 vault，将离开的库的模板与术语表路径及模板映射保存在 `preferences.vault_templates`，以已核实的 vault 绝对路径为键。切回时先恢复该库的配置；对未配置的库重新提问。切换时按新库核对文献根、附件和工作目录，避免继续使用上一个库的路径。

## 配置来源

按顺序取第一个可核实值，不按目录名猜测：

1. 用户本次明确给出的路径、库名或笔记位置。
2. 当前工作目录及其向上最近的 `.obsidian` 目录（vault 根）。
3. 主技能配置中已核实且仍可用的 vault。
4. 如已配置 Obsidian 客户端 CLI，可通过实际命令核实 vault 列表。
5. 仍不确定时向用户确认一次。

确认后记录 `project_root`、`vault_root`、`literature_root`、`templates_root`、`glossary_path`、`assets_resource_dir`、`work_root`，存绝对路径并同时保留相对 vault 根的版本。路径别名、符号链接与 OneDrive 重定向先解析再比较，不同路径不得因名称相似视为同一位置。

## 技能内持久化配置

用户级路径与偏好只写入当前技能目录：

`<skills_root>/literature-zotero2obsidian/user/profile.json`

使用 `user_profile.preflight`、`user_profile.locate`、`user_profile.load`、`user_profile.save`、`user_profile.patch` 管理该文件。禁止写入 `%APPDATA%`、`~/.config`、注册表或其他通用用户目录。`user/` 是技能私有用户状态，不进入安装包，升级技能时保留。

首次运行按下列顺序执行，之后不再重复搜索：

1. 日常任务优先单次调用 `user_profile.preflight`（可同时传入 `query` / `item_key` 及 `create_work_dirs: true`）：`locate` 会在传入仓库/skills 根目录时自动向下钻取 `literature-zotero2obsidian/`；一次返回档案校验、Zotero API 探测、模板列表与 `template_skeleton`、`naming_plan`（含 `suggested_asset_bucket_relative` 与 `suggested_note_relative`）、条目/PDF 定位及 `work_dirs`。仅在 `status: "uninitialized"` 或用户要求变更配置时重新发现并 `user_profile.save` / `user_profile.patch`。
2. 每个记录路径先核对实际存在性、类型与包含关系。`paths.work_root` 必须指向总工作根目录（如 `<parent>/literature-pipeline-work/<vault>`），禁止将单篇条目工作子目录写入 `work_root`；`paths.obsidian_cli` 填实际可执行命令名或路径（如 `"obsidian"`），不带说明括号。
3. 记录值与本次命令行/用户输入冲突时，以本次明确输入为准并更新记录，同时保留变更说明。
4. 不以“上次运行成功”代替本次路径可读性核对，也不因记录存在而跳过写前边界检查。

持久化字段包括 `paths`（Zotero 本地 API、Zotero 数据/存储目录、Obsidian vault、文献根、模板、术语表、附件资源根、工作根、Obsidian CLI）、`zotero_library`（`{"type":"user","id":<id>}`）、`templates`（按文献类型映射默认模板文件名与 `glossary`）与 `preferences`。`preferences` 至少保留：

| 键 | 默认 | 含义 |
|---|---|---|
| `formula_table_enhancement` | `bundled` | 默认开箱使用内置 PP-OCRv6 + 视觉核对（兼容历史 `native` / `ocr` / `unasked` 值，不再首次弹窗询问） |
| `arxiv_tex_auto` | `true` | 预印本默认尝试 TeX 源辅助 |
| `transparent_figure` | `preserve_original` | 透明插图保留原图/alpha，不合成白底 |

### OCR 与视觉核对

整包安装时已包含 PP-OCRv6、版面和表格模型。Magpie 首次运行时由后台补齐，同样使用 `literature-zotero2obsidian/runtime/ocr/`。

- 文本质量正常的原生 PDF 直接使用纯 Rust `pdf-inspector` + PdfCraft 几何聚类与 arXiv TeX 辅助；遇到扫描页、缺字/Type 3 字体或乱码块时自动调用内置 PP-OCRv6（置信度阈值 `>= 0.92`）。
- 对 2D 复杂公式、第 1 页作者/单位块及低置信/无法修复的乱码区域，自动生成 300 DPI `ocr-verify-sheet.png` 并在 `clean-layout.md` 中插入 `<!-- VISUAL_REQUIRED: ... -->` 标记，由宿主直接看图核对；未消解标记或任何乱码字符在写库校验阶段将被硬阻断。

### arXiv 预印本 TeX 源

条目类型为 `preprint`，或元数据/附件中出现 arXiv ID、`arxiv.org` 链接时，视为预印本。`arxiv_tex_auto` 为 `true`（默认）时自动尝试 TeX 辅助，不先询问：

1. 优先使用条目附件或本次工作目录中已有的 `.tex`、`.zip`、`.tar`、`.tar.gz`、`.gz` 源，调用 `prepare_source_inventory.prepare_tex_source`。
2. 否则直接调用 `prepare_source_inventory.fetch_arxiv_tex`（传入 `item.json` / arXiv ID / `pdf_path`）：自动探测 arXiv ID、下载 e-print 到库外目录、自动展开 `.bbl` 与 `.gz`/`.tar` 并生成 `tex-structure.json` 与 `clean-source.md`（默认 `compact: true` 只返回紧凑摘要）；若返回 `status: "not_arxiv"` 或 `"unavailable"`，回退到 `prepare_source_inventory.prepare_pdf_layout`。
3. TeX 只作文字、公式、表格和书目辅助；页面顺序、可见版面、图件边界仍以 PDF 页图核对。TeX 与 PDF 冲突时保留两者证据并人工裁决，不静默覆盖。
4. 下载失败、源码缺页或版本不一致时记录原因并回退到 PDF 路径；不把辅助失败写成全文失败。

## 运行预检

- 先明确本次步骤、允许写入的目标和已有运行记录，再只读检查本次实际使用的来源接口与文件工具。只有本次消费的依赖才是必需；单项本地 Markdown 修复不因未使用 Zotero 而启动全流程。
- 核对源文件可读、目标根及工作根可用，并检查解析后的路径包含关系。工作稿、备份和缓存必须位于 vault 之外；不能只凭目录名为 `Backups` 判定合规。
- 默认工作根若落入 vault，优先复用已核实的库外配置；否则规划 `<vault_root 的父目录>/literature-pipeline-work/<vault 标识>/`，核对身份、可写性及包含关系后登记。不能安全确定时阻断需要落地工作文件的步骤，保留已完成的只读发现。
- 在内容处理前核对所需模板、术语表及文件解析工具版本。发现表格后核对实际模板使用的 Rich Table 数据版本，并检查对应的 YAML/JSON 结构。
- 记录本次来源文件和目标目录的访问结果。

## 离线候选模式

用户只要求从所给附件生成本地候选时，使用明确给出的输出目录作为 `candidate_root`。这不表示已发现真实 vault。`vault_root` 保持 `null`；真实 Zotero 身份、集合和 URI 保持未知。PDF 中能核实的文献元信息写入工作值，并标明来源。

离线候选按模板完成全文和 Rich Table 源码，直接检查文件内容与引用资源。无需发现或启动 Rich Table 插件。缺少库访问时，不用“未分类”代替未知集合，也不请求无关的写库许可。

候选中的库根相对资源路径以 `candidate_root` 为解析根。例如笔记嵌入 `文献/attachments/<年份>/<首作者>/<资源桶>/fig1.png`，对应文件就放在输出根内的同一路径。该输出树可用于之后的落库规划；不得声称已写入真实 vault。工作文件继续保留在单独的工作根。

没有实际原条目 URI 时，保留模板要求的「Zotero原条目」末尾标题，记录链接待验证，不编造 URI。

## 默认相对约定

以下是默认值；实际目录名不同时以核实结果为准并记录差异，不因默认值不符直接失败。

| 项 | 默认值 |
|---|---|
| 文献根 | `<vault_root>/文献` |
| 模板 | `<vault_root>/template/`，按实际 itemType 选 `journal-article.md`、`thesis.md`、`conference-paper.md` 或 `preprint.md` |
| 术语表 | `<vault_root>/template/glossary.md` |
| 附件资源 | `<assets_resource_dir>`（默认 `attachments`，图与视频同目录） |
| 工作根 | `<project_root>/Backups/literature-pipeline/`，解析后必须位于 vault 之外；不满足时按运行预检选择库外位置 |
| Rich Table 数据版本 | 按实际模板采用 v2 或 v3，逐工作表检查数据 |

资源清单记录条目身份、附件身份与编号；最终文件名按下列规范生成。模板与术语表路径均可由用户另行指定。

### 附件资源三级目录

主文和补充材料引用的本地图片、视频、文档等资源统一存入所属文献的三级目录：

`<assets_resource_dir>/<年份>/<第一作者>/<资源桶>/<文件>`

- `<assets_resource_dir>`：默认 `<literature_root>/attachments`（目录名用英文 `attachments`，不用中文「附件」），核实后写入 run.json。
- `<年份>`：复用第 5 步核实的发表年份；可靠年份为四位数字，未知时用 `无年份` 并保留原因。YAML 年份仍为空值，不以运行年份代替。
- `<第一作者>`：复用[文件名规则](../../literature-filename/SKILL.md)的首位作者标记，外文用核实的姓、中文用全名、机构作者用原名，不附加 `and` 或 `et al`；缺失时用 `未知作者`。使用同一非法字符清洗映射，不重新猜拆作者姓名。
- `<资源桶>`：默认采用原文 title 归一化（小写、去首尾空白、合并连续空白）后的 SHA-1 十六进制前 16 位，保留已核实的既有映射。title 哈希只是目录标记，不能用来证明条目或版本相同。
- 不同库、条目或版本落到同一资源桶时，先核对已登记归属。冲突的新资源桶在 title 哈希后附加稳定身份后缀；后缀由库类型、已核实库标识、item_key、所选版本及源附件身份/哈希的规范化记录生成，使用 SHA-256 十六进制前 12 位，若仍冲突则延长并固定映射。不得覆盖或批量迁移其他条目的既有资源。
- 每个目标资源都记录身份、来源和字节哈希。既有文件仅在归属有据且哈希相同时复用；不同哈希不得覆盖，改用带内容哈希后缀的新文件名并更新本次候选引用。未确认归属或未知旧资源保留，失败恢复不删除它们。
- 提取（第 2、11 步）产出资源文件，第 4 步 plan 依本规则确定最终目录并写入 placement，commit 时随 MD 提交。全部资源落三级目录，不再平铺。
- 主文和补充材料的本地资源引用均从 Obsidian 库根起算，如 `文献/attachments/2018/Fan/<title 哈希>/fig1.png`。使用正斜杠，禁用 `../`、`./`、盘符、`file://` 和仅文件名的路径。保留可读字符，Markdown 链接的含空格目标用 `<...>` 包裹。
- 附件文件名规范化：图片名用 `figN`（正文）/`figSN`（补充材料），视频用 `videoN`/`videoSN`，音频用 `audioN`/`audioSN`，扩展名保留。N 取自原文图注/视频编号（`图1`→`fig1`、`图2a`→`fig2a`、`图2-12`→`fig2-12`、`Figure S1`→`figS1`；`Video S3`→`videoS3`）。原文无编号时，正文图从 1 起、补充材料从 S1 起，按出现顺序编号。同号多文件（子图、一图多片、跨篇共享副本）用 `-1/-2` 或保留子图字母 `a/b` 区分，保证同目录唯一。
- 上述无编号资源的序号仅作文件或导航标识，不补造可见的原文编号。多份 SI 出现同号时使用[来源命名空间](pipeline-contract.md#补充材料与书目命名空间)的稳定 source_token 消歧，如 `figS1-ab12cd34ef56.png`；既有已核实映射保持稳定，不按本次遍历顺序重分配。

## 阻断判定

有疑问按所在级别处理，不静默降级。

| 情况 | 级别 |
|---|---|
| 主全文缺失、关键页/段缺失或不可辨、身份或版本冲突 | blocking：该条目停止，其他条目继续 |
| 补充材料正文、图、表缺失或不可读 | blocking |
| 补充材料视频缺失或不可解码 | warning |
| 模板缺失 | warning：用通用结构并记录缺项 |
| 术语表缺失 | warning：可出待核译稿，不标术语核验完成 |
| 集合读取不完整、接口未提供某字段 | warning：不得据此判为“未分类”或“原文没有” |

## 工具与边界

- 使用 `z2o` 的 `zotero_local.search_items`、`inspect_item` 或 `catalog` 读取 Zotero 本地 API，先探测接口，不打印凭据。
- Obsidian 笔记与附件写入统一使用 `vault_publish.publish_standalone`（一步完成预检、冲突消歧、事务备份、写入、SHA-256/`WRITE-A04`/`WRITE-A05` 回读并生成 `report.md`）或 `vault_publish.plan_standalone` + `vault_publish.commit_markdown`。返回 `verified_file_write` 即完成。用户明确要求客户端操作时，可调用已配置的 Obsidian 客户端 CLI，先核对帮助、目标库和命令输出。
- 含 LaTeX 反斜杠或长全文时，以文件方式保存正文再回读实际文件。移动或重命名时核对可枚举的入链；使用 CLI 时读取其实际返回文件内容。CLI 不可用时可使用文件接口，无法核实的入链范围单独保留待核记录。
- 模板作为结构与格式参考。先检查模板实际行为，不盲跑其正则清理、改名或交互脚本。
- 未覆盖的文献类型保留实际类型，采用不虚构字段的通用结构，不套成期刊论文。

## 分类策略

默认镜像 Zotero 集合从根到叶的完整名称路径：`<literature_root>/<集合层级>/<文件名>`。标签与保存搜索不作为集合目录。

一个条目属于多个集合时仅保留一份正文。依次采用：用户指定主分类 → 已有笔记对应且仍有效的集合路径 → 已配置的集合优先级 → 按完整集合路径及 key 排序的首项。记录全部集合及选用理由；用户要求多目录实体副本时另行执行，不默认制造副本或索引 MD。

仅在集合列表确认完整且确实为空时使用 `<literature_root>/未分类`。集合读取失败、父级缺失或有环不是“未分类”。同名集合、清洗后重名、多库路径冲突使用稳定 key 后缀消歧；首次采用的映射后续复用。

## 内容规则归属

属性键集、题头、排版、图表、链接与文件名的规则以对应子 skill 为准，本文件不重复定义，避免两处改动分叉。

## Zotero note 兼容

同步仅在用户明确要求时启用，使用 `zotero_local.sync_child_note`。需要 Zotero 10 或更新版本并启用本地 API。准备 UTF-8 的 Zotero 兼容 HTML，转换 Obsidian 专用语法，保留本次选择的正文语言。以已核实的库、父条目 key 和稳定的 `sync_id` 定位同步对象，同一份笔记重跑时复用同一个 `sync_id`。

先用默认的 `authorized: false` 查看计划。更新前核对现有子笔记及差异，传入已核对的 `expected_version` 和 `expected_server_id`；有用户修改冲突时保留差异。传 `authorized: true` 开始写入，Zotero Desktop 会弹出授权。返回 `verified_zotero_note_write` 或 `unchanged` 后记录同步结果。若写入结果不确定，先重新查看现有笔记。同步失败单独报告。
