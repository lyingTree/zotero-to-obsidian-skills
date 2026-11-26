# Rust 运行入口

机械操作统一使用当前安装主技能目录的 `runtime/z2o`（Windows 为 `runtime/z2o.exe`；若主技能目录下尚未放置则直接检查工作区根 `runtime/z2o[.exe]`）。**禁止遍历 `rust/src`、`target/` 或整个仓库寻找二进制**。

## 统一协议

需要精确 schema 时仅按操作名查询，**禁止常规流程整份倾倒 `z2o describe`（注意 `z2o describe` 不支持 `--output` 参数）**：
- `z2o describe`：检查 `engine=rust`、`version=v2.0.3` 与全部注册 `operations`。
- `z2o describe <operation>`：查询单个操作的位置参数 `args`、关键字参数 `kwargs`、默认值、`request_template` 与核心复合接口的 `parameter_examples`（也可用 `z2o describe --all` 或 `z2o --help`）。
- 只有 `describe` 中注册的操作才能调用。将 UTF-8 JSON 请求写到实际任务文件，或通过标准输入传给 `z2o invoke -`，例如：

```json
{
  "schema_version": 1,
  "operation": "deterministic_stage_checks.author_token",
  "args": [["Smith", "Jones"]],
  "kwargs": {}
}
```

通过 `z2o invoke request.json [--output response.json]`（或 `-o <file>`，也支持 `z2o invoke -` 从标准输入读取）执行（使用已确认的实际可执行文件、请求文件路径）。传入 `--output` 时完整美化 JSON 写入 `<file>`，标准输出只打印紧凑 `saved_output` 回执，避免大 JSON 挤占对话上下文；非终端管道上标准错误自动抑制中间进度刷新。上例只演示虚构姓名接口。非零退出、未注册操作、字段错误、未完成证据或工作流门禁都保持阻断或待核；不从退出码为零推断来源语义通过。路径用实际 Unicode 路径字符串或协议规定的路径对象。所有文件、来源定位、哈希、范围和决定必须来自本次任务，不复制示例值。

## 正文组装与机械验收

| Rust operation | args（依序） | kwargs |
|---|---|---|
| `user_profile.preflight` | 无 | `skill_root`（支持仓库根目录自动钻取）、`query`、`item_key`、`zotero_storage_dir`、`create_work_dirs`（默认 `false`；单次返回档案+Zotero+模板骨架+`naming_plan`+工作目录） |
| `prepare_source_inventory.prepare_paper` | `work_dir` | `pdf_path`、`tex_target`、`glossary_path`、`render_pages`（默认 `"auto"`）、`dpi`、`expected_pdf_sha256`、`arxiv_timeout_seconds`（一站式 TeX+PDF 版面+`figure_candidates`+`matched-glossary.md`） |
| `prepare_source_inventory.fetch_arxiv_tex` | `target`（arXiv ID/URL/DOI/item JSON）、`destination` | `source_id`（默认 `"main"`）、`pdf_path`、`timeout_seconds`、`compact`（默认 `true`） |
| `prepare_source_inventory.prepare_pdf_layout` | `source_path`、`destination` | `source_id`、`render_pages`（`selective`\|`all`\|`none`，默认 `selective`）、`dpi`、`expected_source_sha256`（含 `figure_candidates`） |
| `prepare_source_inventory.prepare_sources` | `source_pdfs` 映射对象（如 `{"main": "<文件路径>"}`）、`work_directory` | `source_roles`、`glossary_path`、`dpi`、`ocr`（按实际需要） |
| `pdf_image_objects.render_page_regions_batch` | `source_pdf` | `regions`（数组，含 `page_number`/`bbox`/`destination`/`transparent`/`auto_trim`/`label`）、`overwrite`、`contact_sheet_destination`、`contact_sheet_columns`（单会话批量裁图并输出序号拼图） |
| `pdf_image_objects.render_page_region` / `inspect_crop_boundaries` | `source_pdf` | `page_number`、`bbox`、`coordinate_system`、`destination` / `context_destination`、可选 `overwrite`、`transparent`（默认 `true`） |
| `pdf_image_objects.render_pdfcraft_region` | `source_pdf` | `page_number`、`bbox`、`coordinate_system`、`destination`；`transparent` 默认 `true`，保留透明背景，不合成白底 |
| `prepare_source_inventory.inspect_tex_source` | 实际 `.tex` / `.zip` / `.tar` / `.tar.gz` / `.gz` 路径 | `source_id`（必填）、`expected_source_sha256`（可选） |
| `prepare_source_inventory.prepare_tex_source` | 实际源码路径、证据目录 | `source_id`（必填）、`expected_source_sha256`（可选） |
| `note_pipeline.prepare_content_job` | 实际 plan、sources、output_root | 无 |
| `note_pipeline.process_content` | plan、实际 content、output_root | `contract`、`resource_root`、`write_normalized_to`、`strip_display_trailing_punctuation`、`force_lf`、`convert_markdown_tables` |
| `note_pipeline.process_markdown` | 实际 source_path、output_root | `contract`、`resource_root`、`write_normalized_to`、`strip_display_trailing_punctuation`、`force_lf`、`convert_markdown_tables`（精简 `normalization.json` 并将详细缺项摘要输出到 `summary`） |
| `render_note.render_note` / `normalize_note.normalize_note` / `accept_note.accept_note` | 按当前实际接口与来源合同 | 不伪造来源 |
| `validate_delivery.validate_delivery` | 实际 manifest_path | 无 |
| `lint_presentation.lint_file` | 实际候选路径 | 无 |
| `complete_workflow.*`（仅旧工作目录恢复：`resume_workflow` / `set_naming_plan` / `bind_candidate` / `write_manifest` / `set_vault_write_scope`） | 实际 run_path 等 | 仅用于已有 `complete_pdf_candidate_v1` 目录恢复 |

仅在候选版本固定后绑定。写交付索引前，运行入口检查实际附件、目标路径，以及来源清单引用的验收台账；未通过的预检不改动台账。相对路径以各自记录所在目录解析。首次绑定与写索引分别原子落盘，不是跨文件原子操作；如中断，保留台账目录中 `.z2o-workflow-manifest-*` 的恢复记录，按 `plan.json` 对照实际哈希后再处理，不覆盖期间的外部修改。

不得把局部机械结果替代完整交付验收。先核定实际来源和独立合同，再调用；旧 schema 不兼容时保留原始记录并阻断。

## 翻译复核

先以 `prepare_translation_review.prepare_translation_review` 建立真实骨架。`args` 依次为实际 inventory_path、candidate_path、output_path；`kwargs.unit_ids` 必须为实际选定来源单元，`glossary_path`、`candidate_locations` 按实际需要填写。

后续 `translation_review_tasks.next_review_task`、`translation_review_tasks.require_unit_records`、`translation_review_writer.ReviewWriter.*`、`translation_review_writer.UnitReview.*`、`translation_review_writer.TermReview.*` 均使用实际 prepared_path 和完整 decisions 数组作为前两个参数（首次未有决定可使用空数组）。每次选择/记录必须由实际来源支持，按对应操作传入命名参数；保留返回的真实决定记录及选区，不手造完整台账。`require_unit_records` 另需 `kwargs.unit_id`。选择源/候选位置使用 `translation_review_writer.select_source` / `select_candidate`，另需 unit_id 和 selector；追加决定使用 `append_decision` 的 command。

如果一个原样片段在允许范围内重复，先用行段缩小范围，再显式填写从 1 开始的 `selector.occurrence`。工具将候选原样片段与 `char_range: [start, end]` 一起记录；此范围用全文从 0 开始的 Unicode 码点（不含结尾），不是 UTF-8 字节，不能手写或改动它来迁移旧结论。保存或回读时，验证器会按实际文件重新核对字符、行段和所属单元。后续判断传回选择结果中的 `selector`，不要把 `locator` 或渲染后的译文当作原文选择条件。

使用 `translation_review_writer.save_decision_draft`（kwargs.output_path）保存增量，`restore_decision_draft`（args: 实际 draft_path）恢复；`translation_review_writer.save_review`（kwargs.output_path，可选 terminology_path）保存审阅。最后 `validate_translation_review.validate_translation_review` 接收实际审阅路径及可选 inventory_path、candidate_path。逐方法参数与证据内容以实际实现及证据约定为准；未覆盖或不兼容时保持待核。

## 其余机械操作

| 步骤 | 主要 Rust operation 命名空间 |
|---|---|
| 技能内配置 | `user_profile.preflight`、`user_profile.locate`、`user_profile.load`、`user_profile.save`、`user_profile.patch`（固定写入主技能 `user/profile.json`） |
| Zotero 只读检查 | `zotero_local.search_items`、`zotero_local.inspect_item`、`zotero_local.catalog`（全库或大结果建议传入 `output_path` 落盘） |
| 文件名 | `format_helpers.build_naming_plan`、`format_helpers.validate_naming_plan` |
| 属性、来源绑定和回读 | `deterministic_stage_checks.*` |
| 来源、草稿队列与图件 | `prepare_source_inventory.*`、`docx_source_inventory.*`、`pdf_image_objects.*`、`draft_queue.*` |
| OCR、表格/公式识别与 MiniCPM/Jev | 见[OCR、表格/公式识别与 MiniCPM/Jev 指南](ocr-and-minicpm.md)（15 个 `prepare_source_inventory.*` 操作） |
| 表格、图件与篇内锚 | `format_helpers.build_rich_table_block`、`format_helpers.new_figure_table_anchor`、`format_helpers.plan_supplement_figure_caption` |
| 库内链接身份 | `vault_library.index`（支持 `use_cache: true` 目录指纹缓存与 `output_path`）、`vault_library.match_reference`、`vault_library.match_references_batch`（批量 O(N+M) 匹配） |
| 写库与恢复 | `vault_publish.publish_standalone`（一步预检+`asset_source_dir` 扁平资产查找+冲突消歧+事务写入+生成 `report.md`）、`vault_publish.plan_standalone`、`vault_publish.plan`（legacy）、`vault_publish.commit_markdown`、`vault_publish.rollback` |

表中的 `*` 仅表示命名空间，不可作为实际 operation。只调用 `describe` 中的精确名称。库内操作需明确本地库和真实文件，写入需明确授权、完整实际写前条件、备份及回读。不得用独立 Markdown 模式绕开完整文献门禁。
