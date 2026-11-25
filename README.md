# zotero-to-obsidian-skills

将 Zotero 文献全文处理为 Obsidian 中文文献笔记的 Agent 技能包（版本 `v2.0.3`），含 12 个 `literature-*` 技能与 Rust 运行器 `z2o`，另配两个外部依赖技能。适用于支持 Agent Skills（`SKILL.md`）的宿主。

## 组成

主入口：`literature-zotero2obsidian`（编排完整流程，可按步骤/断点执行，所有机械操作通过 `literature-zotero2obsidian/runtime/z2o` 执行）。

| 技能 | 版本 | 职责 |
|---|---|---|
| `literature-zotero2obsidian` | `v2.0.3` | 主编排：Zotero → Obsidian 全流程 |
| `literature-inspect-zotero` | `v2.0.3` | 读取条目、集合、语言、附件状态 |
| `literature-fulltext-extract` | `v2.0.3` | 多格式全文提取、按需 OCR、断行修复 |
| `literature-translate-zh` | `v2.0.3` | 按术语表翻译为中文 |
| `literature-write2obsidian` | `v2.0.3` | 目录规划、提交、回读与恢复 |
| `literature-filename` | `v2.0.3` | 文件名规范与冲突处理 |
| `literature-note-properties` | `v2.0.3` | Obsidian YAML 属性 |
| `literature-header-format` | `v2.0.3` | 标题、作者、单位、通讯身份 |
| `literature-body-format` | `v2.0.3` | 正文排版、图表公式、书目编号 |
| `literature-internal-links` | `v2.0.3` | 篇内图/表/公式/引文链接 |
| `literature-library-links` | `v2.0.3` | 参考文献指向库内其他文献的出链 |
| `literature-supplement-materials` | `v2.0.3` | 补充材料（SI）提取与翻译 |

## 支持平台（预编译分发）

GitHub Releases 提供五大平台的预编译技能包与 `z2o` 二进制，直接下载即可安装与运行：
- `x86_64-pc-windows-msvc`（Windows x64）
- `aarch64-pc-windows-msvc`（Windows ARM64）
- `x86_64-unknown-linux-gnu`（Linux x86-64）
- `aarch64-unknown-linux-gnu`（Linux ARM64）
- `aarch64-apple-darwin`（macOS Apple Silicon）

## 安装

运行 [install.ps1](install.ps1)（Windows）或 [install.sh](install.sh)（Linux / macOS），脚本会自动识别当前平台、从 GitHub Release 下载对应预编译包、校验 `SHA256SUMS.txt` 与 `package.json`，并调用包内 `z2o install` 将 12 个技能的 `.md` 文档与 `literature-zotero2obsidian/runtime/z2o[.exe]` 原子安装到目标 Skills 目录（同时按 [dependencies.lock.json](dependencies.lock.json) 拉取 `obsidian-cli` 与 `zotero-skills`）：

```powershell
# Windows
./install.ps1 -SkillsRoot "D:\YourAgent\skills"
```

```bash
# Linux / macOS
./install.sh "/path/to/your/agent/skills"
```

安装完成后可通过安装目录内的 `z2o` 核验：

```powershell
& "D:\YourAgent\skills\literature-zotero2obsidian\runtime\z2o.exe" describe
```

## 模板与术语表

`template/` 提供文献笔记的默认资源，复制到你的文献根 `template/` 目录使用：

- [journal-article.md](template/journal-article.md)、[thesis.md](template/thesis.md)、[conference-paper.md](template/conference-paper.md)、[preprint.md](template/preprint.md) — Templater 模板，按 Zotero 条目类型创建文献笔记
- [glossary.md](template/glossary.md) — 文献翻译用的中英术语表与译名核对说明

## 使用

对 Agent 说"把某篇 Zotero 文献整理进 Obsidian"，主入口 `literature-zotero2obsidian` 会按 [Rust 运行入口](literature-zotero2obsidian/references/rust-entrypoints.md) 的 `z2o describe` / `z2o invoke` JSON 协议调用各阶段机械操作。
