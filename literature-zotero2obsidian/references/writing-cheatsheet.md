# 文献笔记快速写作速查（Cheatsheet）

只读本页完成常规笔记撰写；模板脚本仅参考 `preflight` 返回的 `template_skeleton`，术语只读本篇 `matched-glossary.md`，无需通读 11 个子 skill。

## 1. 标准 7 步极速流程

1. **单次预检** `user_profile.preflight`（传 `query` / `item_key` + `create_work_dirs: true`）：获取路径、Zotero 元数据、`naming_plan`、`suggested_asset_bucket_relative`、`template_skeleton` 与库外工作目录。
2. **源码准备** `prepare_source_inventory.prepare_paper`（传 `work_dir`、`pdf_path`、`tex_target`、`glossary_path`）：自动优先获取 arXiv TeX（生成 `clean-source.md`）、执行选择性 PDF 版面聚类与 `figure_candidates` 提取，并过滤输出本篇 `matched-glossary.md`。
3. **批量裁图** `pdf_image_objects.render_page_regions_batch`（依据 `figure_candidates` 一次裁剪多图，`transparent: true` 默认、`auto_trim: true`，输出 `contact_sheet_destination` 拼图），仅看拼图微调个别裁框。
4. **撰写候选稿** 写入 `<work>/work/candidate.md`：普通 Markdown GFM 表格直接书写（交付流水线自动转 `rich-table`）。
5. **流水线规范化与验收** `note_pipeline.process_markdown`（开 `write_normalized_to`、`force_lf: true`、`convert_markdown_tables: true`、`strip_display_trailing_punctuation: true`）：只查看返回 JSON 的 `summary`（含 `failed_checks`、`unresolved_items`、`review_prompts_sample`）。
6. **可选库内出链** `vault_library.index`（`use_cache: true` 目录指纹缓存）+ `vault_library.match_references_batch`。
7. **授权一步写库** `vault_publish.publish_standalone`（传 `authorized: true`、`asset_source_dir`、`report_output`）：自动完成预检、冲突消歧、备份、写库与 SHA-256/`WRITE-A04`/`WRITE-A05` 回读，并生成回执报告。**返回 `verified_file_write` 后即终态，立即总结结果结束。**

## 2. 正文与格式高频硬约束

- **Frontmatter（YAML）**：仅保留 `template_skeleton.frontmatter_keys` 要求字段；`年份` 保持空值或核实发表年，禁止出现废弃键 `库内参考文献`。
- **题头与中英混排**：一级标题后紧接中文译名行，作者/单位按原文核实。中文与英文、数字或 `$...$` 行内公式之间不加多余空格；标题行内英文字母/数字紧贴中文（流水线会同步更新 `#标题` 与 `[[#标题]]` 锚链）。
- **章节与参考文献**：按 `template_skeleton.headings` 组织；参考文献标题参与连续一级分节编号（如 `## 10 参考文献`），置于补充材料之前。
- **图表与块锚**：
  - 图：`![[<assets_relative>/<年份>/<第一作者>/<资源桶>/figN.png]]` 后紧跟 `**图N：** 图注说明` 与单独一行 `^fig-N`（连续多子图同一行尾加两个空格断行）。默认保留透明通道，禁将透明背景涂白。
  - 表：优先写标准 GFM 表格，前置一行 `**表N：** 表注说明`，表后紧跟 `^tbl-N`，交由 `convert_markdown_tables: true` 自动生成 Rich Table v2 代码块。
- **公式与引用**：展示公式独立成段，编号用 `\tag{n}`，末尾换行紧跟 `^eq-n`（启用 `strip_display_trailing_punctuation: true` 清理句末标点）。正文引文上标统一 `<sup>[[n]](#ref-n)</sup>` 或 `<sup>[n](#ref-n)</sup>`。
- **文末原条目**：全文最末尾单独成节 `## [Zotero原条目](zotero://select/library/items/<KEY>)`。

## 3. 明确禁止项

- **禁盲扫仓库**：禁止递归扫描 `z2o` 源码或预读 `target/`、`runtime/`。
- **禁全量倾倒**：禁止读取 328 KB 全量术语表与 41 KB+ Templater 脚本；禁止向对话直接打印完整 `normalization.json` / `acceptance.json`。
- **禁无意义全页渲染**：TeX 源码已就绪时禁用 `render_pages: "all"`；常规裁图禁用 `transparent: false`。
- **禁写库后空转**：`publish_standalone` / `commit_markdown` 校验通过后禁止调用 `obsidian` CLI、`Start-Sleep` 或重复验收。
