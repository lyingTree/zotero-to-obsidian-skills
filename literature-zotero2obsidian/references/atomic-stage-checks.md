# 原子文件检查与确定性阶段辅助（增量）

本页补充已有的 [Rust 组装入口](rust-entrypoints.md)、[源码检查](automated-validation.md) 和 [来源交付索引](delivery-checks.md)，不替代它们。统一使用 `deterministic_stage_checks.*` 操作。函数接受实际文件与已确认的字段；不要求模型再次拼接确定性格式或凭语法通过自行填写验收结论。

## 确定性辅助

| 阶段 | Rust 入口 | 证据边界 |
|---|---|---|
| 读取条目 | collection_ancestry、snapshot | 原始 API 集合父链、成环/缺项/未完分页及实际文件字节；附件角色、语言、版本、文件格式有效性仍需独立证据 |
| 全文抽取 | 既有 prepare_source_inventory、docx_source_inventory、validate_delivery；snapshot | 逐页/结构对象覆盖与真实字节绑定，不替代页图和提取内容核对 |
| 翻译 | audit_translation_binding | TRN-A17/A25 独立目标路径和哈希核验；不评价译文 |
| 规划及写入 | audit_write_readback、inspect_relative_files | WRITE-A04/A05 与明确列出的本地文件实际可读性、哈希、根边界；不自动覆盖、移动或写入实际库 |
| 文件名 | author_token、既有 build_naming_plan/validate_naming_plan | 已核实作者标记数量决定连接词，不猜姓或角色 |
| 属性 | build_properties、repair_properties、audit_properties | 新建模板字段类型；修复只删除废弃顶层键，保留正文及其他属性原始写法；复杂 YAML 无法安全隔离时阻断 |
| 题头 | audit_unchanged_properties、既有 render_note | HDR-A14 比较属性区原始字节；题头来源字段由源证据决定 |
| 正文 | audit_rich_tables、既有 normalize_note、lint_presentation、validate_note、accept_note | 已实现的格式规范、解析和固定点；语义和原文对应不能按零提示授予通过 |
| 篇内链接 | 既有 render_note、validate_note、validate_delivery、handoff_maps | 目标存在、来源作用域和已有映射；不凭号码猜书目身份 |
| 补充材料 | inspect_relative_files、既有 validate_delivery/prepare_source_inventory | 本地资源路径、字节、来源清单；显式路径列表不证明覆盖全部视频或图，也不代替解码 |
| 库内链接 | audit_unchanged_properties、inspect_relative_files | LIB-A08 属性字节相等；明确目标的可读性，不证明身份、版本或出链全覆盖 |
| 总流程 | coverage_catalog、record_file_checks、既有 inspect_workflow | 独立记录实际原子验收结果；保留源语义未决，不将辅助函数的状态外推 |

在编辑器内编写任务脚本、导入模块，路径由实际运行记录提供。新建属性的 confirmed 值须从已核定的当前来源和模板取得，作者标记须为已经核实的有序列表。示例：

```json
{
  "schema_version": 1,
  "operation": "note_pipeline.process_content",
  "args": [],
  "kwargs": {}
}
```
    record_file_checks(checks_path, work_root.joinpath("properties-file-checks.json"),
                       stage="properties", before=content_input, after=properties_draft,
                       mode="new", item_type=confirmed_item_type)

修复模式调用 repair_properties(content_input.read_bytes())，另存候选，然后用 mode="repair" 复核。函数不重序列化未知属性，不猜测属性值；源语义或无法独立判断的项仍需核验。记录函数仅接收阶段与实际输入路径，由内部审计函数实时计算；不接收模型撰写的决策报告。报告及输入文件均进入哈希绑定证据。先确保当前任务独占检查账本的写入权限，已解决的其他项不会批量替换，绑定文件变化或冲突会阻断。单条记录只对应一个实际 A ID；C ID 的辅助结果不会自动提升为未分类约束的通过。

## 全部规则的覆盖盘点

在实际技能根调用 coverage_catalog(skills_root)，使用已有 check_records 的声明解析器枚举全部 C/A ID（包括引用页）；结果逐条列出真实规则位置、原目标及本模块显式映射。输出可用 json.dumps 保存至独立工作目录。未映射项标为 not_classified_by_this_module，不把它们误记为“必须模型判断”或“已经程序化”。已有各检查器仍要逐规则核对其实现和证据范围。本增量没有用关键词自动分派验收或把未适用等同于通过。

## Rich Table 独立验收

`audit_rich_tables(candidate)` 复用 validate_note 的真实围栏扫描器，按实际 v2 表或 v3 工作表分别计算 BODY-A83/A84/A85/A86/A87/A111。锁定值必须是真正的布尔 true，主题须为 grid，列/行须为非空列表，存在的 footer 须为字符串。报告保留每表各 sheet 行列数。没有表、无法解析的结构或扫描不完整时不自动判为通过。用 `record_file_checks(checks_path, report_path, stage="tables", after=candidate)` 写入明确目标的结果。行列数仍须与源表独立核对。
