# Rich Table 合并单元格

本说明用于保留来源表格的合并结构。表中被合并的组标签只写入左上角单元格；后续数据行仍保留各自的数据单元格。重复组标签的展平表不能代替原合并结构。

## 已核对的字段

字段依据为官方 `SdKay/obsidian-rich-table` 仓库的固定提交 `d1717904a5d0b4f42d11671ee6129b721394d0a1`，核对日期为 2026-11-23。目标库的插件实际版本仍按项目配置核验。

| 字段或位置 | 语义 | 固定来源 |
|---|---|---|
| `merges` | 合并范围列表 | [model.ts](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/src/model.ts#L98) |
| `anchor` | 左上端点，形式为 `行ID.列ID` | [MergeRangeV2](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/src/model.ts#L99) |
| `end` | 右下端点，范围包含该单元格 | [官方格式说明](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/skills/obsidian-rich-table/SKILL.md#L137) |
| v2 位置 | 与 `columns`、`rows` 同层 | [TableModelV2](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/src/model.ts#L168) |
| v3 位置 | 每个 `sheets` 成员自己的字段 | [SheetDefV2](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/src/model.ts#L260) |
| `header.列ID` | 表头作为合并端点时使用的形式 | [表头合并说明](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/skills/obsidian-rich-table/SKILL.md#L148) |

行列位置取相应表或工作表中的实际顺序，不能从 ID 的数字后缀推断顺序。`anchor` 和 `end` 均引用该表已经存在的 ID。左上单元格保存显示内容，被覆盖单元格为空或省略。解析器只接受 `anchor` 和 `end` 都是字符串的项；通过 YAML 解析不等于合并范围有效。参见[实际解析代码](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/src/parser.ts#L230)。

## v2 虚构示例

下例的“组甲”跨两行。量值各占一个数据行。这是格式示例，不是文献数据。

```yaml
version: 2
title: "**表1：** 虚构的分组量值"
columns:
  - id: c_000001
    name: 组别
  - id: c_000002
    name: 量值
rows:
  - id: r_000001
    cells:
      c_000001: 组甲
      c_000002: '$5$'
  - id: r_000002
    cells:
      c_000001: ''
      c_000002: '$7$'
merges:
  - anchor: r_000001.c_000001
    end: r_000002.c_000001
theme: grid
locked: true
```

## v3 所在层级

下例表达同一虚构表。多工作表时，各工作表的合并关系独立保存。

```yaml
version: 3
active_sheet: s_000001
sheets:
  - id: s_000001
    name: 分组量值
    title: "**表1：** 虚构的分组量值"
    columns:
      - id: c_000001
        name: 组别
      - id: c_000002
        name: 量值
    rows:
      - id: r_000001
        cells:
          c_000001: 组甲
          c_000002: '$5$'
      - id: r_000002
        cells:
          c_000001: ''
          c_000002: '$7$'
    merges:
      - anchor: r_000001.c_000001
        end: r_000002.c_000001
    theme: grid
    locked: true
```

## 验收边界

来源合并范围保存在 source-map 或 si-map 的对象记录中。Rich Table 的 `merges` 是交付结构；只有来源映射而没有交付合并字段，仍未完成合并结构交付。现有四份模板会保留已输入 v2/v3 的 `merges`，但普通管道表自动转换不会推断合并关系。

源码核对关注字段位置及范围端点。模板回归核对 `merges` 经过实际模板后是否保留。物理合并显示由真实 Rich Table 插件验收，缺少该环境时单独记待验。不能把静态数据保留声称为插件界面已通过。
