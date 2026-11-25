---
name: literature-note-properties
version: v2.0.3
description: 按文献类型及既有模板构建或修复 Obsidian 文献 YAML 属性，保留人工自定义字段。只处理属性，不重写题头、正文或参考文献。
---

# 构建笔记属性

机械操作统一通过 [Rust 运行入口](../literature-zotero2obsidian/references/rust-entrypoints.md) 的 `z2o describe` / `z2o invoke` 执行。未通过实际验收的能力保持阻断或待核。

每条规则的 ID 用于定位对应证据。

## 必要输入

- item.json、对应类型模板、content_input 或既有笔记。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

新建属性直接调用 [确定性阶段辅助](../literature-zotero2obsidian/references/atomic-stage-checks.md) 的 build_properties；既有稿删除废弃顶层键使用 repair_properties，无法安全隔离时阻断。另存阶段稿后使用 audit_properties，实际文件的 PROP-A01/A02/A04/A05/A06/A08 由 Rust 操作分别判定；来源及其他未覆盖项仍单独核验。

1. 读取实际 itemType 对应模板。
2. 仅编辑起始 YAML 属性区。

## 约束

- [PROP-C01] 新建管理键限 `类型`、`年份`、`关键词`；期刊论文额外允许 `期刊名`。
- [PROP-C02] 新建笔记禁用 `库内参考文献` 键。
- [PROP-C03] 修复笔记时删除整个 `库内参考文献` 键及其值，空值或人工值同样删除。
- [PROP-C04] 库内出链保存在参考文献条目中。
- [PROP-C05] 不新增索引字段。
- [PROP-C06] 不新增路径字段。
- [PROP-C07] 不新增哈希字段。
- [PROP-C08] 不新增运行状态字段。
- [PROP-C09] 除废弃键外，保留未知自定义键。
- [PROP-C10] 除废弃键外，保留人工值。
- [PROP-C11] 字段冲突时记录双方来源。
- [PROP-C12] 年份与命名所用文献版本一致。
- [PROP-C13] 年份使用整数或空值。
- [PROP-C14] 关键词使用列表类型。
- [PROP-C15] 每个关键词可追溯到原文或条目字段。
- [PROP-C16] 未知值按模板类型采用空值、null 或省略。
- [PROP-C17] 含 `: `、` #` 或以 `:` 结尾的字符串加引号。
- [PROP-C18] 其他值保留原写法。

## 产物

- drafts/06-properties.md。

## 验收标准

- [ ] [PROP-A01] YAML 可成功解析。 <!-- check-type: offline_outcome -->
- [ ] [PROP-A02] YAML 没有重复键。 <!-- check-type: offline_outcome -->
- [ ] [PROP-A03] 全文仅有一个起始属性区。 <!-- check-type: offline_outcome -->
- [ ] [PROP-A04] 解析后的 YAML 顶层没有 `库内参考文献` 键。 <!-- check-type: offline_outcome -->
- [ ] [PROP-A05] 属性键集合符合本次新建或修复场景。 <!-- check-type: offline_outcome -->
- [ ] [PROP-A06] 属性值类型符合对应模板。 <!-- check-type: offline_outcome -->
- [ ] [PROP-A07] 属性值来源符合本次新建或修复场景。 <!-- check-type: offline_outcome -->
- [ ] [PROP-A08] 属性区外内容与输入候选一致。 <!-- check-type: offline_outcome -->
