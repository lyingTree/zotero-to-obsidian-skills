---
name: literature-library-links
version: v2.1.2
description: 将文献参考条目与 Obsidian 文献库中已有笔记核对，建立有身份与版本证据的跨文件出链。保留完整书目显示文字，不凭相似题名强配、不创建缺失文献。
---

<!-- z2o:first-run:start -->
从本文件实际位置定位同级主技能，先按[首次运行](../literature-zotero2obsidian/references/project-profile.md#magpie-首次运行)验证并在缺失时自动下载共享 `z2o`。然后确认模板和术语表路径，配置成功后移除本组入口中的首次运行段，再继续原任务。
<!-- z2o:first-run:end -->

# 建立参考文献到库内文件的出链

需要模板或术语表且目标库尚未配置时，按[模板与术语表配置](../literature-zotero2obsidian/references/project-profile.md#模板与术语表配置)向用户确认路径。

机械操作统一通过 [Rust 运行入口](../literature-zotero2obsidian/references/rust-entrypoints.md) 的 `z2o describe` / `z2o invoke` 执行。未通过实际验收的能力保持阻断或待核。

每条规则的 ID 用于定位对应证据。

## 必要输入

- 候选稿、原始书目、指定文献库根及既有身份映射。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

LIB-A08 直接使用 [确定性阶段辅助](../literature-zotero2obsidian/references/atomic-stage-checks.md) 的 audit_unchanged_properties(stage="library") 比较原始 YAML 字节；对明确列出的出链目标运行 inspect_relative_files，文件存在不等于身份或版本唯一。

1. 通过 `vault_library.index` 索引实际 vault 和已确认文献根的原条目 URI，结合 `zotero_local.catalog` 实际原始条目清单，按 `vault_library.match_reference` 核对绑定快照、完整原文题名、作者、年份、出版类型和 DOI。唯一候选仍须核对实际版本；不得将库目录缺失或旧快照视为当前无候选。Rust 查询不可用时阻断，不回退旧脚本。
2. 核对每个候选自身的题头、属性及关联 Zotero 条目。
3. 保存 bibliography_id、参考 ID、目标路径、条目身份、版本证据和指纹；主文与各份 SI 的同号条目分别核对。

## 约束

- [LIB-C01] 检索排除模板。
- [LIB-C02] 检索排除附件。
- [LIB-C03] 检索排除备份。
- [LIB-C04] 检索排除工作稿。
- [LIB-C05] 缓存只保存在工作目录。
- [LIB-C06] DOI 等稳定标识只在比对时规范化。
- [LIB-C07] 保留原始书目。
- [LIB-C08] 相似度只用于召回候选。
- [LIB-C09] 相同 DOI 的候选存在题名冲突时保留歧义。
- [LIB-C10] 相同 DOI 的候选存在版本冲突时保留歧义。
- [LIB-C11] 无 DOI 时核对候选的完整题名。
- [LIB-C12] 无 DOI 时核对候选的作者。
- [LIB-C13] 无 DOI 时核对候选的年份。
- [LIB-C14] 无 DOI 时核对候选的版本。
- [LIB-C15] 无 DOI 时，只有上述核对共同支持唯一对应才确认匹配。
- [LIB-C16] 不得仅凭候选正文引用的 DOI 确认身份。
- [LIB-C17] 不得仅凭译名确认身份。
- [LIB-C18] 不得仅凭截断文件名确认身份。
- [LIB-C19] 不得仅凭同姓同年确认身份。
- [LIB-C20] 不得仅凭重复副本确认身份。
- [LIB-C21] 库内出链只写入对应参考文献条目。
- [LIB-C22] 库内出链使用库根相对完整路径。
- [LIB-C23] 库内出链使用完整书目作为显示文字。
- [LIB-C24] 不写入 YAML 链接索引。
- [LIB-C25] 不更新 YAML 链接索引。
- [LIB-C26] 发现废弃键时交[属性技能](../literature-note-properties/SKILL.md)清理。
- [LIB-C27] 保留原书目编号。
- [LIB-C28] 保留原篇内锚点。
- [LIB-C29] 不追加题名中译。
- [LIB-C30] 正确转义显示文字。
- [LIB-C31] 必要时采用可验证的 Markdown 链接。
- [LIB-C32] 保留已有 DOI/URL。
- [LIB-C45] 已有 DOI/URL 使用独立链接。
- [LIB-C46] 已有 DOI/URL 链接位于对应库内出链旁。
- [LIB-C33] 不嵌套链接。
- [LIB-C34] 不重复整项书目。
- [LIB-C35] 未匹配项保留原书目。
- [LIB-C36] 未匹配原因标为 absent、ambiguous、unavailable 或 source_error。
- [LIB-C37] Windows 长路径先尝试实际可读的访问方式。
- [LIB-C38] 检索不完整时记录实际检索范围。
- [LIB-C39] 不创建空壳笔记。
- [LIB-C40] 不下载文献。
- [LIB-C41] 不向目标笔记写入反向索引。
- [LIB-C42] 提交前复查链接目标存在。
- [LIB-C43] 提交前复查链接目标身份未变。
- [LIB-C44] 改名后的入链处理交第 4 步。

## 产物

- drafts/10-library.md、link-map.json 的 library_matches 及本步 unresolved。

## 验收标准

- [ ] [LIB-A01] 每条已建出链均可达。 <!-- check-type: offline_outcome -->
- [ ] [LIB-A02] 每条已建出链有唯一身份证据。 <!-- check-type: offline_outcome -->
- [ ] [LIB-A03] 每条已建出链有版本证据。 <!-- check-type: offline_outcome -->
- [ ] [LIB-A04] 每条未链接书目都有原因。 <!-- check-type: offline_outcome -->
- [ ] [LIB-A05] 原始书目信息完整保留。 <!-- check-type: offline_outcome -->
- [ ] [LIB-A06] 原 DOI/URL 完整保留。 <!-- check-type: offline_outcome -->
- [ ] [LIB-A07] 本篇锚点完整保留。 <!-- check-type: offline_outcome -->
- [ ] [LIB-A08] YAML 属性区与输入一致。 <!-- check-type: offline_outcome -->
