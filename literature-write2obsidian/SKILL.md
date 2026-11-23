---
name: literature-write2obsidian
description: 按 Zotero 集合层级规划 Obsidian 文献路径，并提交已完成的候选稿、资源和命名变更，备份后回读验证。支持仅规划或只提交，不负责提取、翻译和内容重写。
---

# 规划目录与提交笔记

## 必要输入

- plan 使用 item.json、命名计划和目标库；commit 另需 placement、final.md 与所需资源。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

1. plan 核实 vault、CLI 身份、文献根与附件根。
2. plan 按集合层级和命名计划列出本次涉及的文件、资源、入链及目标预期哈希或“不存在”。
3. commit 核对授权、候选验收记录、源与目标的写前条件。解析 `final.md` 起始 YAML；若存在 `库内参考文献` 键，先交[属性技能](../literature-note-properties/SKILL.md)修正并重验候选。新增笔记或本次涉及书目时，写前按[第 8 步](../literature-body-format/SKILL.md#验收标准)核对 `final.md` 的书目标题与节序号。新增笔记或本次涉及书目、书目锚点或引文时，写前须通过[第 9 步书目锚点验收](../literature-internal-links/SKILL.md#验收标准)。
4. commit 将已有 MD、受影响文件及操作计划备份到库外。
5. 先写必要资源，再写引用它们的笔记。
6. 逐项通过文件和 [obsidian-cli](../obsidian-cli/SKILL.md) 回读，记录实际状态。

## 约束

- plan 解析别名与链接，拒绝绝对集合名、`..` 与越界路径。
- 集合层级、多集合选择、目录消歧与附件布局按项目配置执行。
- plan 只确定路径，不在库内创建文件。
- 多个副本保留差异，不任意覆盖。
- 写前目标被占用、身份冲突或人工修改时保留候选并报告差异。
- 相同提交已有回执且实际内容一致时只回读。
- 新文件使用目标存在即失败的创建方式。
- 已有文件在同一文件系统可行时用临时文件原子替换；逐项记录整组操作。
- 重命名或移动使用 CLI 并检查受影响入链。
- Windows 调用先解析实际 CLI 控制台入口。
- 长全文和 LaTeX 走文件写入，避免 shell 插值和 `content=`。
- 文件以 UTF-8、LF 保存；回执哈希按 LF 规范化。
- 重存后哈希变化先逐行确认，只有换行差异才更新回执。
- CLI 超时先检查实际写入状态，再决定是否重试。
- 部分失败仅恢复本次写入且之后未变的文件。
- 保留用户新修改与未知或共享资源；记录备份、剩余文件及恢复说明。
- 只有明确要求时才单独同步 Zotero note 并回读。

## 产物

- plan 更新 run.json.placement；commit 生成 write-receipt.json。

## 验收标准

- [ ] plan 的最终路径均位于确认的根目录内。
- [ ] commit 有备份位置和写前比较证据。
- [ ] 写后文件与待提交内容哈希一致，回读的 YAML 顶层没有 `库内参考文献` 键。
- [ ] 嵌入指向实际资源；SI 视频存在且可读取。范围内的 PDF 图像写前逐幅通过[裁剪视觉验收](../literature-fulltext-extract/references/figure-crop.md)，写后回读最终资源路径和哈希并与验收记录核对。
- [ ] 实际文件的锚点、入链及原条目链接经原生回读核对。适用时回读书目标题的实际层级与节序号，并执行第 9 步书目锚点回读，结果写入回执。
- [ ] 回执分别记录写入、回读及适用的 Zotero 同步结果。
- [ ] 发生部分失败时回执标 partial/failed，并可定位恢复记录。
