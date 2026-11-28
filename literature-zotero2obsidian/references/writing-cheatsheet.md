# 文献笔记快速写作速查（Cheatsheet）

只读本页 + `preflight` 返回值即可完成全文撰写与入库；**禁预读 11 个子 skill、Templater 脚本、全量术语表或库内已有笔记**。

## 1. 标准 7 步极速流程（~15–22 次调用）

1. **定位二进制**：直查 `<主技能目录>/runtime/z2o.exe`（没有则查仓库根 `runtime/z2o.exe`）；**禁遍历源码/`target`，禁整份倾倒 `z2o describe`**。
2. **单次预检**：`user_profile.preflight`（`query`/`item_key` + `create_work_dirs: true`）取 `naming_plan`（含 `and` / `et al`）、`suggested_asset_bucket_relative`（仅 `<第一作者>`）、`template_skeleton` 与 `work_dirs`。
3. **源码与术语**：`prepare_source_inventory.prepare_paper`（`compact: true`、`work_dir`、`pdf_path`、`tex_target`、`glossary_path`）。按 **400–600 行大块**通读 `clean-source.md`（或 `clean-layout.md`）与 `matched-glossary.md`。
4. **批量裁图**：逐批将 `crop_batches` 的文件绑定作为 `regions`，各批使用独立的拼图文件名。最多每批 32 张，查看拼图并优先检查 `priority_visual_review`，只重裁异常图。SI 图用 `figS1` / `fig-s1`。
5. **分段写完整 `candidate.md`**：先在草稿核对图注、公式、书目及来源疑点，再按第 2 节格式写完 `<work>/work/candidate.md`（**禁只写 PART 1 或留 `<!--PART-->` 占位符就结束**）。
6. **机械规范化**：`note_pipeline.process_markdown`（`preflight_resources: true`、`resource_root`、`compact: true`、`write_normalized_to`、`force_lf: true`、`convert_markdown_tables: true`、`strip_display_trailing_punctuation: true`）。根据 `summary.failed_checks` 集中修复，变更输入后才重跑。来源忠实度待核时对照原文。查询旧结果时读取回执，标点实验使用文本片段。
7. **授权写库**：用户要求“处理/整理某文献到 Obsidian”（非仅要方案/审查/离线稿），**即已授权写库**，须调用 `vault_publish.publish_standalone`（`acceptance_receipt` 取流水线 `receipt`、`authorized: true`、`asset_source_dir`、`report_output`）写入并回读。草稿复核通过后发布一次。收到 `verified_file_write` 后直接汇报路径和资源数量。孤立资源清理仅按独立请求执行。

## 2. 模板与排版精确语法（照抄即可）

- **Frontmatter（YAML `[PROP-C01]`）**：仅允许以下键（**禁写** `作者`/`标题`/`DOI`/`链接`/`库内参考文献`）：
  ```yaml
  ---
  类型: "预印本"   # 期刊论文 | 会议论文 | 学位论文 | 预印本（按 template_skeleton.type_label）
  年份: 2026       # 发表年整数；无来源留空
  关键词: ["..."]
  期刊名: "..."    # 仅 journalArticle 保留
  ---
  ```
- **题头（H1 + 译名 + 作者 + 单位）**：
  ```markdown
  # Original Paper Title
  中文译名

  [First Author](mailto:a@b.edu)${}^{1,\ast}$; Second Author${}^{1,2}$

  ${}^{1}$ First Affiliation, City, Country  
  ${}^{2}$ Second Affiliation, City, Country
  ```
  （作者/单位照录原文不译，单位每行末加两空格换行）
- **摘要与章节**：有摘要写 `## 摘要` 下接首段；无摘要略。正文从 `## 1 ...` 连续编号至 `## N 参考文献`（保留原文文献标题，每行末标 `^ref-n`）。
- **混排与块结构**：中文与英文/数字/行内 `$...$` 间**不加空格**（数值与单位如 `100 km` 保留）。
  - **图块**（三行紧贴无空行）：`![[<suggested_asset_bucket_relative>/fig1.png]]` ↵ `**图1：** 题注` ↵ `^fig-1`；引用 `[图1](<#^fig-1>)`。
  - **表块**：`**表1：** 题注` + 标准 GFM 表 + `^tbl-1`（自动转 Rich Table）；合并单元格调 `format_helpers.build_rich_table_block`（传 `merges`）；引用 `[表1](<#^tbl-1>)`。
  - **公式**：`$$\n... \tag{1}\n$$` ↵ `^eq-1`；引用 `[公式1](<#^eq-1>)`。
  - **引文**：数字 `<sup>\[[1](<#^ref-1>),[2](<#^ref-2>)\]</sup>` 或 `<sup>\[[1](<#^ref-1>)-[5](<#^ref-5>)\]</sup>`；作者年份 `[Hines (1960)](<#^ref-22>)` / `（[Hines, 1960](<#^ref-22>)）`。
  - **文末**：`---` ↵↵ `## [Zotero原条目](zotero://select/library/items/<KEY>)`。


