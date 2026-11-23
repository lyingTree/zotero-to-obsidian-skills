---
name: literature-supplement-materials
description: 提取并翻译 Zotero 条目的补充材料（SI 文本、图、表、视频），合并为正文后的补充材料章节；在第 8 步排版与第 9 步链接之前执行。
---

# 处理补充材料

## 必要输入

- item.json 中的 supplement 附件、第 3 步选定主稿及术语记录。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

1. 复用第 1 步清单，逐附件核对 key、MIME、实际格式、路径与哈希。
2. 提取 SI 文本、图、表、公式及视频，登记数据文件。
3. 按[来源命名空间](../literature-zotero2obsidian/references/pipeline-contract.md#补充材料与书目命名空间)登记 source_id、bibliography_id 和稳定 token，将已核对内容并入当前主稿一次；续做时按来源身份和原编号更新既有章节。

## 约束

- SI 来源和编号独立于主全文。
- 多份 SI 可以各自有 S1、参考条目1或相同公式号。保留这些可见原号，以来源命名空间消歧块、资源和链接；不能按当前附件顺序统一重排原号。已有已核实映射继续使用。
- 不可读附件按实际情况记录 remote_only、missing 或其他可用性状态。
- PDF 文本层优先，扫描页按需 OCR；沿用[全文提取规则](../literature-fulltext-extract/SKILL.md)。
- 保留 Text、Figure、Table、Movie 的原 S 编号与顺序，不补造原文编号。
- 补充图执行[PDF 图像裁剪与验收规则](../literature-fulltext-extract/references/figure-crop.md)；一页仅一幅图时也须定位图框。
- 表格保留行列、合并、单位与表注。
- 公式保留 LaTeX 与原号。
- 视频逐个探测实际容器、编码、时长、分辨率、音轨。Obsidian 嵌入限 `.mkv`、`.mov`、`.mp4`、`.ogv`、`.webm`。其他格式或目标端无法解码时，自动转为 H.264 MP4，保留原件和已有音轨，记录原件与转码件的哈希。
- 资源先存工作目录。按项目配置将图片、视频、文档等归入所属文献的 `attachments/<年份>/<第一作者>/<资源桶>/`，沿用已核实的 title 哈希目录及必要的身份/版本消歧。不同资源哈希不得覆盖旧文件；引用用从库根起算的完整路径，由第 4 步提交。
- 按 SI 实际语种复用[翻译规则](../literature-translate-zh/SKILL.md)与主稿术语。
- 图表、视频题注直译，视频画面不改绘。
- 将 `## 补充材料` 整章置于主文全部内容之后（含附录、声明和主文参考文献），置于「Zotero原条目」之前。新建及既有稿修复均适用；章名统一为「补充材料」，编号由第 8 步处理。
- 补充书目保留为补充材料章内的 H3，由第 8 步按同级次序编号，如 `### 11.3 参考文献`。沿用书目顺序及锚点映射；单份 SI 默认 `^ref-sN`，多份 SI 新建时按来源用 `^ref-s<source_token>-N`，不得复用主文 `^ref-N`。
- 子标题依原顺序使用 `### 文本 S…`、`### 图 S…`、`### 表 S…`、`### 视频 S…`。
- 图块依次为图片、下方题注、图锚，相邻行之间无空行；单份 SI 默认 `^fig-sN`，多份 SI 按来源 token 消歧。
- 表块沿用第 8 步 Rich Table 规则，闭合围栏的下一行顶格独占一行写表锚，中间零空行；单份 SI 默认 `^tbl-sN`，多份 SI 按来源 token 消歧。表锚后有后续内容时留一空行。
- 视频就近排题注，独立一行用 Obsidian 原生嵌入 `![[从库根起算的视频完整路径]] ^mov-sN`；多份 SI 按来源 token 消歧。沿用已有锚点和调用。仅远程可用或转码失败时记录状态及原来源。
- 正文调用保留显示文字，交第 9 步绑定现有目标。
- 缺页、缺图表、缺视频分别按项目配置判级。

## 产物

- si.md、si-map.json、assets/si/、drafts/11-supplement.md。

## 验收标准

- [ ] 每一页、图、表和视频均有覆盖决定。
- [ ] 每个应译文本块均有对应译块或未决定位。
- [ ] 逐项核对本地资源的三级目录、库根引用、最终路径及哈希，确认 Obsidian 可解析。补充图逐幅通过[裁剪视觉验收](../literature-fulltext-extract/references/figure-crop.md)。
- [ ] 逐个核对视频原生嵌入、实际容器与编解码器、首帧解码及所属 `^mov-s…` 落点。转码件为 H.264 MP4，核对时长、分辨率与音轨；记录检查范围和未通过项。
- [ ] 内容块、图、表、公式、视频和书目锚点全篇唯一，所属来源与实际对象一致；同号不同 SI 未混配，新增来源未破坏已有调用。
- [ ] 主稿仅有一份「补充材料」章节，整章位于主文全部内容之后、「Zotero原条目」之前；补充书目（如有）留在该章内，交第 8 步核对最终章号和小节序号。
