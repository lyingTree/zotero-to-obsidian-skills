# 项目配置与运行时发现

首次运行读取。本文件不含任何绝对路径：项目根、vault 与文献根在运行时确认后写入 run.json，同一项目后续复用。迁移到其他机器或仓库无需修改本文件。

## 配置来源

按顺序取第一个可核实值，不按目录名猜测：

1. 用户本次明确给出的路径、库名或笔记位置。
2. 当前工作目录及其向上最近的 `.obsidian` 目录（vault 根）。
3. `obsidian-cli` 列出的实际 vault 列表。
4. 仍不确定时向用户确认一次，不静默假设。

确认后记录 `project_root`、`vault_root`、`literature_root`、`templates_root`、`glossary_path`、`assets_resource_dir`、`work_root`，存绝对路径并同时保留相对 vault 根的版本。路径别名、符号链接与 OneDrive 重定向先解析再比较，不同路径不得因名称相似视为同一位置。

## 默认相对约定

以下是默认值；实际目录名不同时以核实结果为准并记录差异，不因默认值不符直接失败。

| 项 | 默认值 |
|---|---|
| 文献根 | `<vault_root>/文献` |
| 模板 | `<literature_root>/template/`，按实际 itemType 选期刊论文、学位论文、会议论文或预印本 |
| 术语表 | `<literature_root>/template/术语表.md` |
| 附件资源 | `<assets_resource_dir>`（默认 `attachments`，图与视频同目录） |
| 工作根 | `<project_root>/Backups/literature-pipeline/`，位于 vault 与文献目录之外 |
| rich-table 插件 | `<vault_root>/.obsidian/plugins/rich-table/manifest.json` |

资源清单记录条目身份、附件身份与编号；最终文件名按下列规范生成。模板与术语表路径均可由用户另行指定。

### 附件资源三级目录

主文和补充材料引用的本地图片、视频、文档等资源统一存入所属文献的三级目录：

`<assets_resource_dir>/<年份>/<第一作者>/<title 哈希>/<文件>`

- `<assets_resource_dir>`：默认 `<literature_root>/attachments`（目录名用英文 `attachments`，不用中文「附件」），核实后写入 run.json。
- `<年份>`：文献发表年份，四位数字，取 item.json 的 normalized 年份；不以运行年份代替。
- `<第一作者>`：第一作者姓（family）原文，同一规则清理非法字符；无法取得时用 `未知作者` 并记 warning。
- `<title 哈希>`：原文 title 归一化（小写、去首尾空白、合并连续空白）后取哈希，固定取 SHA-1 十六进制前 16 位。
- 提取（第 2、11 步）产出资源文件，第 4 步 plan 依本规则确定最终目录并写入 placement，commit 时随 MD 提交。全部资源落三级目录，不再平铺。
- 主文和补充材料的本地资源引用均从 Obsidian 库根起算，如 `文献/attachments/2018/Fan/<title 哈希>/fig1.png`。使用正斜杠，禁用 `../`、`./`、盘符、`file://` 和仅文件名的路径。保留可读字符，Markdown 链接的含空格目标用 `<...>` 包裹。
- 附件文件名规范化：图片名用 `figN`（正文）/`figSN`（补充材料），视频用 `videoN`/`videoSN`，音频用 `audioN`/`audioSN`，扩展名保留。N 取自原文图注/视频编号（`图1`→`fig1`、`图2a`→`fig2a`、`图2-12`→`fig2-12`、`Figure S1`→`figS1`；`Video S3`→`videoS3`）。原文无编号时，正文图从 1 起、补充材料从 S1 起，按出现顺序编号。同号多文件（子图、一图多片、跨篇共享副本）用 `-1/-2` 或保留子图字母 `a/b` 区分，保证同目录唯一。

## 阻断判定

有疑问按所在级别处理，不静默降级。

| 情况 | 级别 |
|---|---|
| 主全文缺失、关键页/段缺失或不可辨、身份或版本冲突 | blocking：该条目停止，其他条目继续 |
| rich-table 插件缺失或版本不兼容 | blocking：该表阻断，不降级为管道表 |
| 补充材料正文、图、表缺失或不可读 | blocking |
| 补充材料视频缺失或不可解码 | warning |
| 模板缺失 | warning：用通用结构并记录缺项 |
| 术语表缺失 | warning：可出待核译稿，不标术语核验完成 |
| 集合读取不完整、接口未提供某字段 | warning：不得据此判为“未分类”或“原文没有” |

## 工具与边界

- Zotero 操作使用同级 [zotero-skills](../../zotero-skills/SKILL.md)，先探测再调用，不假定 MCP 工具已安装，不打印凭据。
- Obsidian 操作使用同级 [obsidian-cli](../../obsidian-cli/SKILL.md)，先读取实际 help；明确 vault 与相对 path，解析返回内容，不能只看退出码。
- 含 LaTeX 反斜杠或长全文时，按 obsidian-cli 限制以文件方式保存正文再经 CLI 回读；移动或重命名用 CLI 以更新入链。CLI 不可用时保留候选稿与待核项，不冒充已在 Obsidian 验证。
- 模板作为结构与格式参考。先检查模板实际行为，不盲跑其正则清理、改名或交互脚本。
- 未覆盖的文献类型保留实际类型，采用不虚构字段的通用结构，不套成期刊论文。

## 分类策略

默认镜像 Zotero 集合从根到叶的完整名称路径：`<literature_root>/<集合层级>/<文件名>`。标签与保存搜索不作为集合目录。

一个条目属于多个集合时仅保留一份正文。依次采用：用户指定主分类 → 已有笔记对应且仍有效的集合路径 → 已配置的集合优先级 → 按完整集合路径及 key 排序的首项。记录全部集合及选用理由；用户要求多目录实体副本时另行执行，不默认制造副本或索引 MD。

仅在集合列表确认完整且确实为空时使用 `<literature_root>/未分类`。集合读取失败、父级缺失或有环不是“未分类”。同名集合、清洗后重名、多库路径冲突使用稳定 key 后缀消歧；首次采用的映射后续复用。

## 内容规则归属

属性键集、题头、排版、图表、链接与文件名的规则以对应子 skill 为准，本文件不重复定义，避免两处改动分叉。

## Zotero note 兼容

同步仅在用户明确要求时启用。按同一条目及版本寻找既有译文 note，不因重跑再建一份；多个候选或用户修改冲突先保留差异，不自行删除或合并。通过 zotero-skills 的写入能力提交并回读，Obsidian 专用语法不能原样视为 Zotero 可呈现内容。单独记录 note 同步状态；失败不撤销已验证的 Obsidian 写入。
