# zotero-to-obsidian-skills

将 Zotero 文献全文处理为 Obsidian 中文文献笔记的 Agent 技能包，由 12 个 `literature-*` 技能加 2 个依赖技能组成。适用于支持 Agent Skills（`SKILL.md`）的宿主。

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
| `obsidian-cli` | 通过 `obsidian` CLI 操作库（依赖） |
| `zotero-skills` | Zotero 条目与库管理（依赖） |

## 安装

1. 把本仓库各技能目录放到宿主 Skills 目录下，保持目录同级（技能间使用 `../literature-*` 相对引用）。
2. 安装 `zotero-skills` 的 Python 依赖：

   ```bash
   pip install -r zotero-skills/requirements.txt
   ```

3. 需要 Zotero 桌面端运行（本地读）、Obsidian 桌面端运行（`obsidian` CLI）。
4. Zotero 写入所需 API Key / library ID 从环境变量或本机配置读取，绝不写入仓库。

## 使用

对 Agent 说"把某篇 Zotero 文献整理进 Obsidian"，主入口 `literature-zotero2obsidian` 会按流程调用其余技能。也可单独触发某个子技能（如只改文件名、只建链接）。

## 说明

- 本仓库不含任何个人路径、账号或凭据；项目根、vault 与文献根在运行时确认后写入 `run.json`。
- `zotero-skills` 源自 [WenyuChiou/zotero-skills](https://github.com/WenyuChiou/zotero-skills)（commit `e390313c82e76856a525e87a0ae2d0281499e3ab`），遵循 MIT 许可，见 `zotero-skills/LICENSE`。
- 已剔除第三方 `vendor/`（改由 `requirements.txt` 重建）与编译缓存/二进制。
