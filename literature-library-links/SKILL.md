---
name: literature-library-links
description: 将文献参考条目与 Obsidian 文献库中已有笔记核对，建立有身份与版本证据的跨文件出链。保留完整书目显示文字，不凭相似题名强配、不创建缺失文献。
---

# 建立参考文献到库内文件的出链

## 必要输入

- 候选稿、原始书目、指定文献库根及既有身份映射。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

1. 在指定库内查找候选，优先复用有效身份映射。
2. 核对每个候选自身的题头、属性及关联 Zotero 条目。
3. 保存参考 ID、目标路径、条目身份、版本证据和指纹。

## 约束

- 排除模板、附件、备份和工作稿；需要缓存时只存工作目录。
- DOI 等稳定标识仅在比对时规范化，保留原书目。
- 相似度只用于召回候选。
- 相同 DOI 遇到题名或版本冲突时保留歧义。
- 无 DOI 时须由完整题名、作者、年份及版本共同支持唯一对应。
- 候选正文引用的 DOI、译名、截断文件名、同姓同年或重复副本不足以确认身份。
- 库内出链只写入对应参考文献条目，使用库根相对完整路径和完整书目显示文字。禁止写入或更新 YAML 链接索引；发现废弃键时交[属性技能](../literature-note-properties/SKILL.md)清理。
- 保留原编号和篇内锚点，不追加题名中译。
- 显示文字正确转义；必要时用可验证的 Markdown 链接。
- 已有 DOI/URL 作为相邻独立链接保留，避免嵌套和重复整项书目。
- 未匹配项保留原书目，并区分 absent、ambiguous、unavailable、source_error。
- Windows 长路径需先尝试实际可读的访问方式；检索不完整时记录范围。
- 不创建空壳笔记、不下载文献、不向目标笔记写入反向索引。
- 提交前复查目标存在且身份未变；改名后的入链交第 4 步。

## 产物

- drafts/10-library.md、link-map.json 的 library_matches 及本步 unresolved。

## 验收标准

- [ ] 已建出链逐条可达且有唯一身份和版本证据。
- [ ] 每条未链接书目有原因。
- [ ] 原始书目信息、DOI/URL 与本篇锚点均保留，YAML 属性区与输入一致。
