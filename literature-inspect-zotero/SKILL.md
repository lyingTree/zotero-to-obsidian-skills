---
name: literature-inspect-zotero
description: 只读检查指定 Zotero 条目的完整可用信息、集合层级、文献语言、全文及补充附件。用于全文处理前取证，不负责翻译、分类修改或下载文献。
---

# 读取条目与全文状态

使用已安装的[独立二进制运行器](../literature-zotero2obsidian/references/binary-runtime.md)时，原 Python API 的任务脚本由主技能 bin/z2o 的 run-script 执行；已注册的确定性操作可用 invoke JSON 请求文件。无需系统 Python 或 pip；开发环境才直接运行源码。不得用模型手工替代程序验收。

每条规则的 ID 用于定位对应证据。

## 必要输入

- 库与条目 key，或能唯一定位条目的条件。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

集合父链及本地附件的文件事实使用 [确定性阶段辅助](../literature-zotero2obsidian/references/atomic-stage-checks.md) 的 collection_ancestry / snapshot，程序遍历已取得的真实 API 记录，输出缺项、成环和分页状态；不得手工构造缺失父链或将字节可读等同于格式有效。

1. 通过 [zotero-skills](../zotero-skills/SKILL.md) 定位条目。
2. 遍历字段、子项、附件、批注与集合分页。
3. 依据文档内容确认全文版本及附件角色。
4. 核对关联笔记、人工内容和重复候选。

## 约束

- [ZOT-C01] 多个候选匹配时逐项核对身份。
- [ZOT-C02] 多个候选匹配时逐项核对版本。
- [ZOT-C03] raw 保留接口返回的完整字段。
- [ZOT-C04] raw 保留作者角色。
- [ZOT-C05] 记录条目的直接归属集合。
- [ZOT-C06] 记录每个归属集合的逐级父链。
- [ZOT-C07] 为每个集合节点记录所属库。
- [ZOT-C08] 为每个集合节点记录 key。
- [ZOT-C09] 为每个集合节点记录原名。
- [ZOT-C10] 标签不参与分类。
- [ZOT-C11] 搜索视图不参与分类。
- [ZOT-C12] 集合父链缺失时标记集合信息不完整。
- [ZOT-C13] 集合父链成环时标记集合信息不完整。
- [ZOT-C14] 集合分页未读完时标记集合信息不完整。
- [ZOT-C15] 附件清单覆盖全部格式。
- [ZOT-C16] 核对每个附件的 linkMode。
- [ZOT-C17] 核对每个附件声明的 MIME。
- [ZOT-C18] 核对每个附件的文件签名。
- [ZOT-C19] 核对每个附件的实际路径。
- [ZOT-C20] 核对每个本地附件是否非空。
- [ZOT-C21] 核对每个本地附件是否可读取。
- [ZOT-C22] 核对每个附件的格式有效性。
- [ZOT-C23] 将附件角色标记为主全文、SI、快照、仅链接或其他附件之一。
- [ZOT-C24] 核对全文候选的题名是否属于指定版本。
- [ZOT-C25] 核对全文候选的作者是否属于指定版本。
- [ZOT-C26] 核对全文候选的内容是否属于指定版本。
- [ZOT-C27] 全文候选难以区分时列出候选差异。
- [ZOT-C28] 全文候选差异无法裁决时阻断全文选择。
- [ZOT-C29] 不同版本的图分别保留。
- [ZOT-C30] 不同版本的表分别保留。
- [ZOT-C31] 不同版本的书目分别保留。
- [ZOT-C32] 读取声明语言作为语言判断线索。
- [ZOT-C33] 采集正文开头的语言样本。
- [ZOT-C34] 采集正文中部的语言样本。
- [ZOT-C35] 采集正文结尾的语言样本。
- [ZOT-C36] 依据声明语言与正文样本，将正文语言归为 zh、non-zh、mixed 或 unknown。
- [ZOT-C37] 不得仅凭题名决定正文语言。
- [ZOT-C38] 不得仅凭摘要决定正文语言。
- [ZOT-C39] 不得仅凭书目决定正文语言。
- [ZOT-C40] 不得仅凭汉字外观决定正文语言。
- [ZOT-C41] 扫描件没有可用正文样本时，将语言标为 provisional/unknown。
- [ZOT-C42] 收到第 2 步的语言新证据后更新语言判断。
- [ZOT-C43] 语言判断变化时保留判断历史。
- [ZOT-C44] 语言复核不改动其他条目字段。
- [ZOT-C45] 只进行读取检查。
- [ZOT-C46] 不下载附件。
- [ZOT-C47] 不改动条目。
- [ZOT-C48] 不改动集合。
- [ZOT-C49] 不改动笔记。

## 产物

- item.json；run.json 的身份与本步状态。

## 验收标准

- [ ] [ZOT-A01] 已获数据可追溯到原记录。 <!-- check-type: offline_outcome -->
- [ ] [ZOT-A02] 每项缺失信息标明原记录为空、无法读取或未读完。 <!-- check-type: offline_outcome -->
- [ ] [ZOT-A03] 每个附件都有角色记录。 <!-- check-type: offline_outcome -->
- [ ] [ZOT-A04] 每个附件都有可用性记录。 <!-- check-type: offline_outcome -->
- [ ] [ZOT-A05] 已选主全文有版本证据。 <!-- check-type: offline_outcome -->
- [ ] [ZOT-A06] 主全文选择被阻断时有明确原因。 <!-- check-type: offline_outcome -->
- [ ] [ZOT-A07] 每个语言结论有正文定位或待核原因。 <!-- check-type: offline_outcome -->
