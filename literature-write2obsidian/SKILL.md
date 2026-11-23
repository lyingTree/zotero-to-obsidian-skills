---
name: literature-write2obsidian
description: 按 Zotero 集合层级规划 Obsidian 文献路径，并提交已完成的候选稿、资源和命名变更，备份后回读验证。支持仅规划或只提交，不负责提取、翻译和内容重写。
---

# 规划目录与提交笔记

每条规则的 ID 用于定位对应证据。

## 必要输入

- plan 使用 item.json、命名计划和目标库；commit 另需 placement、final.md 与所需资源。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

1. plan 先完成[运行预检](../literature-zotero2obsidian/references/project-profile.md#运行预检)，核实 vault、CLI 身份、文献根、附件根及库外工作/备份根。
2. plan 按集合层级和命名计划列出本次涉及的文件、资源、入链及目标预期哈希或“不存在”。
3. commit 核对授权、候选验收记录、源与目标的写前条件。解析 `final.md` 起始 YAML；若存在 `库内参考文献` 键，先交[属性技能](../literature-note-properties/SKILL.md)修正并重验候选。新增笔记或本次涉及书目时，写前按[第 8 步](../literature-body-format/SKILL.md#验收标准)核对 `final.md` 的书目标题与节序号。新增笔记或本次涉及书目、书目锚点或引文时，写前须通过[第 9 步书目锚点验收](../literature-internal-links/SKILL.md#验收标准)。
4. commit 将已有 MD、受影响文件及操作计划备份到库外。
5. 持有条目及目标路径独占写入权时，先写必要资源，再写引用它们的笔记。资源归属与字节哈希均一致才复用，不同哈希改用已规划的新路径，不覆盖旧资源。
6. 逐项通过文件和 [obsidian-cli](../obsidian-cli/SKILL.md) 回读，记录实际状态。

## 约束

- [WRITE-C01] plan 解析路径别名。
- [WRITE-C02] plan 解析路径链接的实际目标。
- [WRITE-C03] plan 拒绝绝对集合名。
- [WRITE-C04] plan 拒绝含 `..` 的集合路径。
- [WRITE-C05] plan 拒绝越出确认根目录的路径。
- [WRITE-C06] 集合层级按[项目配置](../literature-zotero2obsidian/references/project-profile.md)处理。
- [WRITE-C07] 多集合选择按项目配置处理。
- [WRITE-C08] 目录消歧按项目配置处理。
- [WRITE-C09] 附件布局按项目配置处理。
- [WRITE-C10] plan 只确定路径。
- [WRITE-C11] plan 不在库内创建文件。
- [WRITE-C12] 提取翻译前完成预检。
- [WRITE-C13] 提取翻译前完成命名。
- [WRITE-C14] 提取翻译前完成目录初步规划。
- [WRITE-C15] 资源产生后在 plan 中补齐实际路径。
- [WRITE-C16] 资源产生后在 plan 中补齐资源归属。
- [WRITE-C17] 资源产生后在 plan 中补齐资源哈希。
- [WRITE-C18] 不把规划占位值记为已验证资源。
- [WRITE-C19] 保留多个副本之间的差异。
- [WRITE-C20] 不任意覆盖已有副本。
- [WRITE-C21] 写前发现目标占用、身份冲突或人工修改时保留候选稿。
- [WRITE-C22] 写前发现目标占用、身份冲突或人工修改时报告差异。
- [WRITE-C23] 相同提交已有回执且实际内容一致时只回读。
- [WRITE-C24] 新文件采用目标已存在即失败的创建方式。
- [WRITE-C25] 已有文件在同一文件系统可行时采用临时文件原子替换。
- [WRITE-C26] 逐项记录整组文件操作。
- [WRITE-C27] 重命名或移动使用 CLI。
- [WRITE-C28] 重命名或移动后检查受影响入链。
- [WRITE-C29] Windows 调用前解析实际 CLI 控制台入口。
- [WRITE-C30] 长全文通过文件写入。
- [WRITE-C31] LaTeX 内容通过文件写入。
- [WRITE-C32] 不对长全文或 LaTeX 使用 shell 插值。
- [WRITE-C33] 不对长全文或 LaTeX 使用 `content=` 传参。
- [WRITE-C34] 文件以 UTF-8 编码保存。
- [WRITE-C35] 文件以 LF 换行保存。
- [WRITE-C36] 回执哈希按 LF 规范化后计算。
- [WRITE-C37] 重存后哈希变化时逐行确认差异。
- [WRITE-C38] 仅有换行差异时才按重存结果更新回执哈希。
- [WRITE-C39] CLI 超时后先核对实际写入状态，再决定是否重试。
- [WRITE-C40] 部分失败时仅恢复本次写入且之后未变化的文件。
- [WRITE-C41] 恢复时保留用户新修改。
- [WRITE-C42] 恢复时保留未知资源。
- [WRITE-C43] 恢复时保留共享资源。
- [WRITE-C44] 恢复记录包含备份位置。
- [WRITE-C45] 恢复记录包含剩余文件。
- [WRITE-C46] 恢复记录包含恢复说明。
- [WRITE-C47] 仅在用户明确要求时单独同步 Zotero note。
- [WRITE-C48] Zotero note 同步后回读。
- [WRITE-C49] 单项提交按 scope 核验。
- [WRITE-C50] 单项提交对照输入基线核验。
- [WRITE-C51] 登记范围外的既有问题。
- [WRITE-C52] 在候选差异中明确标出必需的依赖修复。
- [WRITE-C53] 在报告中明确说明必需的依赖修复。
- [WRITE-C54] 不静默重写范围外内容。
- [WRITE-C55] 新建笔记以全部候选为验收范围。
- [WRITE-C56] 单独记录文件哈希核对结果。
- [WRITE-C57] 单独记录静态源码检查结果。
- [WRITE-C58] 单独记录 CLI 返回结果。
- [WRITE-C59] 单独记录原生缓存检查结果。
- [WRITE-C60] 单独记录离屏渲染检查结果。
- [WRITE-C61] 单独记录阅读视图检查结果。
- [WRITE-C62] 不以文件写入成功代替 Obsidian 回读成功。
- [WRITE-C63] 不以 CLI 退出码正常代替缓存验收通过。
- [WRITE-C64] 不以 CLI 退出码正常代替界面验收通过。

## 产物

- plan 更新 run.json.placement；commit 生成 write-receipt.json。

## 验收标准

- [ ] [WRITE-A01] plan 的最终路径均位于确认的根目录内。
- [ ] [WRITE-A02] commit 记录备份位置。
- [ ] [WRITE-A03] commit 保存写前比较证据。
- [ ] [WRITE-A04] 写后文件与待提交内容的哈希一致。
- [ ] [WRITE-A05] 回读的 YAML 顶层不存在 `库内参考文献` 键。
- [ ] [WRITE-A06] 每个嵌入目标均为实际存在的资源。
- [ ] [WRITE-A07] 每个 SI 视频均存在。
- [ ] [WRITE-A08] 每个 SI 视频均可读取。
- [ ] [WRITE-A09] 范围内的 PDF 图像写前逐幅通过[裁剪视觉验收](../literature-fulltext-extract/references/figure-crop.md)。
- [ ] [WRITE-A10] 写后资源路径与裁剪验收记录一致。
- [ ] [WRITE-A11] 写后资源哈希与裁剪验收记录一致。
- [ ] [WRITE-A12] 实际文件的锚点已通过原生回读核对。
- [ ] [WRITE-A13] 实际文件的入链已通过原生回读核对。
- [ ] [WRITE-A14] 实际文件的原条目链接已通过原生回读核对。
- [ ] [WRITE-A15] 本次涉及的书目标题实际层级已回读核对。
- [ ] [WRITE-A16] 本次涉及的书目节序号已回读核对。
- [ ] [WRITE-A17] 本次涉及的书目锚点已通过[第 9 步回读验收](../literature-internal-links/SKILL.md#验收标准)。
- [ ] [WRITE-A18] 回执保存书目锚点回读结果。
- [ ] [WRITE-A19] 回执记录写入结果。
- [ ] [WRITE-A20] 回执记录回读结果。
- [ ] [WRITE-A21] 适用时回执记录 Zotero 同步结果。
- [ ] [WRITE-A22] 发生部分失败时，回执状态为 partial 或 failed。
- [ ] [WRITE-A23] 发生部分失败时，回执可定位到恢复记录。
