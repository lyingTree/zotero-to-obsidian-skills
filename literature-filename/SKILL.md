---
name: literature-filename
version: v2.1.1
description: 为文献生成“年份-作者-原文标题.md”命名计划，处理作者人数、非法字符、长度和身份冲突。用于独立命名或改名准备，实际移动交写入步骤执行。
---

<!-- z2o:first-run:start -->
若本文件位于项目的 `.agent/skills/本技能名/`，先按[首次运行](../literature-zotero2obsidian/references/project-profile.md#magpie-首次运行)验证并在缺失时自动下载共享 `z2o`。然后确认模板和术语表路径，配置成功后移除本组入口中的首次运行段，再继续原任务。
<!-- z2o:first-run:end -->

# 规范文件名

需要模板或术语表且目标库尚未配置时，按[模板与术语表配置](../literature-zotero2obsidian/references/project-profile.md#模板与术语表配置)向用户确认路径。

机械操作统一通过 [Rust 运行入口](../literature-zotero2obsidian/references/rust-entrypoints.md) 的 `z2o describe` / `z2o invoke` 执行。未通过实际验收的能力保持阻断或待核。

每条规则的 ID 用于定位对应证据。

## 必要输入

- 选定版本中已核定的发表年、作者标记和完整原题，以及逐字段来源定位。
- 需要本地冲突检查时，另提供实际目标目录及其名称比较规则；目录快照不能代替条目身份核对。
- 读取[交接约定](../literature-zotero2obsidian/references/pipeline-contract.md)；涉及项目路径、模板、工具或阻断级别时读取[项目配置](../literature-zotero2obsidian/references/project-profile.md)。

## 执行步骤

将已核实、按源顺序排列的作者标记交 [确定性阶段辅助](../literature-zotero2obsidian/references/atomic-stage-checks.md) 的 author_token 生成单人、双人或多人标记，再调用既有 build_naming_plan；不得依据空格自动切姓或猜作者角色。

1. 从来源核定发表年、作者标记和完整原题，保留对应文件哈希与定位。
2. 用下方完整入口生成 `年份-作者-原文标题.md` 的离线命名计划，并保存整个返回值。
3. 按实际可用证据核对长度与冲突，提交命名计划给第 4 步。

下列字段、来源文件和定位均来自本次来源核对记录；按Rust JSON 协议填入实际值：

1. 调用 `format_helpers.build_naming_plan`，`args` 为空数组，`kwargs` 包含 `year`、`author_token`、`original_title`、`source_evidence`、`delivery_path`。`source_evidence` 分别以 `year`、`author_token`、`original_title` 为键，每项包含实际 `file` 绑定及实际 `locator`。
2. 将整个返回的 `result` 保存至 `run.naming`。
3. 调用 `format_helpers.validate_naming_plan`，`args` 为含完整命名计划的单元素数组，`kwargs.delivery_path` 为同一个实际候选路径。若返回 `structurally_consistent=false`，按 `findings` 阻断并修正。

固定交付名 `candidate.md` 只确定 `delivery_path`；仍保存完整的拟用文献名、原题和变换映射。不要用只有 `filename` 的对象代替该返回值。`validate_naming_plan` 只读复算同一构建算法，逐字段检查来源文件哈希，按 `code`、JSON-pointer `field` 和 `message` 返回缺项或不一致；它不补写记录。`structurally_consistent` 只表示记录自洽，`plan_status="blocked"` 仍需处理阻断原因。未知真实路径限额保持未核，校验不认证元数据语义、真实库冲突或写入权限。

仅交付本地候选时，路径验收的 `scope` 指向本次实际 `delivery_path`。保存该完整路径的长度检查依据。保存实际文件工具对该路径的访问结果。限额计数单位按当前文件系统解释，不能把 UTF-8 字节数当作 UTF-16 单元数。拟用文献名仍保留在命名计划中。未提供的未来库目录保持未核。未提供的未来库限额保持未核。当前交付路径的证据不能外推为未来入库路径的证据。

## 约束

- [NAME-C01] 无可靠发表年时使用“无年份”。
- [NAME-C02] 记录使用“无年份”的原因。
- [NAME-C03] 不以添加时间代替发表年。
- [NAME-C04] 无可确认作者时使用“未知作者”。
- [NAME-C05] 机构作者使用原名。
- [NAME-C06] 不以编辑身份代替作者。
- [NAME-C07] 一位作者时使用其作者标记。
- [NAME-C08] 两位作者时以 ` and ` 连接作者标记。
- [NAME-C09] 三位及以上作者时使用第一作者标记加 ` et al`。
- [NAME-C10] 外文作者使用已核实的姓。
- [NAME-C11] 中文作者使用全名。
- [NAME-C12] 不依空格猜切复姓、后缀或机构名。
- [NAME-C13] 只清洗文件名中的非法字符。
- [NAME-C14] 保留完整原题。
- [NAME-C15] 保留原题到文件名的变换映射。
- [NAME-C16] 检查文件名中的控制字符。
- [NAME-C17] 检查文件名中的尾随点。
- [NAME-C18] 检查文件名中的尾随空格。
- [NAME-C19] 检查文件名是否为保留名。
- [NAME-C20] 检查 `[]#^|` 等可能造成链接歧义的字符。
- [NAME-C21] 文件名连同扩展名及消歧后缀最多占 250 个 UTF-16 单元。
- [NAME-C22] 完整路径满足文件系统限制。
- [NAME-C23] 完整路径满足实际工具限制。
- [NAME-C24] 超长时只截断标题部分。
- [NAME-C25] 截断不得拆开 Unicode 字符。
- [NAME-C26] 作者前缀已超限时记录问题。
- [NAME-C27] 按目标系统的大小写规则判重。
- [NAME-C28] 按目标系统的 Unicode 规范化规则判重。
- [NAME-C29] 身份相同时复用既有笔记。
- [NAME-C30] 身份不明时先核对身份。
- [NAME-C31] 不同条目重名时添加稳定 key。
- [NAME-C32] 跨库重名时另加库标识。
- [NAME-C33] 实际改名交第 4 步执行。
- [NAME-C34] 改名后受影响入链的核对交第 4 步执行。
- [NAME-C35] 改名后资源核对交第 4 步执行。

## 离线命名帮助

Rust 操作 `format_helpers.build_naming_plan` 帮助执行上述规则。它接受已经核定的字段，不从姓名空格、标题、PDF 文件名或添加日期猜作者及发表年。作者标记须由调用方先按 NAME-C04–C12 形成，例如单作者标记、两个已核实标记之间的 ` and `，或首作者标记后的 ` et al`。

`source_file_record` 使用 `{"path": 实际路径, "sha256": 实际文件哈希}`；每个字段可以引用不同来源文件。函数核对文件是否存在及哈希是否匹配，不认证定位处的语义或字段身份。无可靠年份时显式传 `year=None` 和 `no_year_reason`；无法确认作者时显式传 `author_token="未知作者"` 和 `unknown_author_reason`。缺失原因也要有来源定位。

固定交付名只确定 `delivery_path`，拟用的文献文件名仍保留在 `run.json.naming.filename`。`original_title` 保存完整原题，`title_mapping` 保存清洗与截断前后的标题，`transformations` 保存字符位置及变换。函数将文件名非法字符和控制字符逐个映射为 `_`，保留仍合法的大小写与标点；点和空格在 `.md` 之前时不等于完整文件名的尾随点或空格。清洗后的 `[]#^` 等链接歧义单独记录；`|` 的替换已保留在非法字符映射中，不凭格式检查改写原题。

长度预算包含年份、完整作者前缀、消歧后缀及 `.md`。默认上限为 250 个 UTF-16 单元，按 Unicode 码点边界截取标题，不拆开代理对；作者前缀不截短。固定部分已占满预算或连一个完整标题字符都放不下时，返回 `status="blocked"`、空的 `filename` 和原因。仅在目标文件系统限额已明确时传 `filename_max_utf8_bytes`；仅在完整路径限额已明确时同时传 `target_directory` 和 `full_path_max_utf16_units`。未提供的文件系统或工具限制保持未核，单个限额比较不等于完整路径已全面验收。

需要查重时，由调用脚本单独调用 `check_local_name_conflicts(filename=..., directory=..., case_sensitive=..., unicode_normalization=...)`，将结果记入 `naming.local_conflicts`。比较规则由调用方依据目标系统提供；不区分大小写时使用 Unicode `casefold`，规范化可选 `none`、`NFC` 或 `NFD`，不能把这套算法自动认作任意文件系统的真实规则。返回值只证明所列目录名称快照中的比较结果，不覆盖子目录、未连接的库、条目身份或稍后的目录状态。出现同名后仍按 NAME-C29–C32 核对身份；已核定的稳定 key 或库标识可通过 `disambiguation_suffix` 与 `suffix_evidence` 传入，函数不会自行编造消歧依据。

这些函数不写入或移动笔记，不复用既有条目，不授权覆盖，也不把语义、身份或真实库冲突标为通过。

## 产物

- run.json.naming（原始字段、逐字段来源证据、old_path、拟用 filename、独立的 delivery_path、变换映射、长度及冲突证据）。

## 验收标准

- [ ] [NAME-A01] 相同输入与映射产生相同名称。 <!-- check-type: offline_outcome -->
- [ ] [NAME-A02] 文件名通过长度检查。 <!-- check-type: offline_outcome -->
- [ ] [NAME-A03] 完整目标路径通过长度检查。 <!-- check-type: offline_outcome -->
- [ ] [NAME-A04] 冲突处理有身份证据。 <!-- check-type: offline_outcome -->
- [ ] [NAME-A05] 冲突处理不会覆盖既有文件。 <!-- check-type: offline_outcome -->
