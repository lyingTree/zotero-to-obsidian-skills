# 交接约定 v1

所有子 skill 共用此约定，可独立调用。以下是运行时应创建的数据契约，不表示已有程序会自动生成这些文件。字段未取得时用 null、空列表及原因，不编造内容或路径。

JSON 交接文件用序列化器生成并立即回读；LaTeX 反斜杠不得手拼进 JSON 字符串。每项检查区分“计划、已执行、通过、未做”，不在实际检查之前写通过。

## 身份、范围与产物

- 条目身份：实际 library_type、library_id 与 item_key 的组合。个人库与群组库不能只凭 item_key 混用；本地 API 的路由别名不能冒充已核实的真实库身份。
- 接口未给出真实库 ID 时，记录待核并以已确认的本地库标识隔离工作目录；不把路由中的 0 当跨设备身份，也不据此合并条目。
- 文献版本与 Zotero 记录修订号分开记录。附件以 attachment_key + SHA-256 标识；同一条目更换附件必须重新核验。
- 每条目独立工作目录，包含当前 run.json；重跑复用已验证产物，输入变化时保留旧记录再建修订。不跨条目共享可变稿件。
- 全流程的原始稿、译稿、各阶段候选稿和报告留在工作目录；只有选定最终 MD 和必要资源进入 vault。正文中不放运行日志或额外索引 MD。

| 文件 | 必需内容 | 写入者 |
|---|---|---|
| run.json | schema_version=1；run_id；scope；identity；config；versions；stages；naming；placement；content_input；issues；publish | 总入口维护总状态；各步骤只更新自己负责字段 |
| item.json | raw；normalized；collections；children；attachments；language；coverage；existing_notes；evidence | 1 |
| source.raw.jsonl | 格式解析/OCR 的原始输出及页、段或元素位置，不做通顺化覆盖 | 2 |
| source.md | 原语言、连续段落、保留逻辑结构的完整内容；稳定块标识 | 2 |
| source-map.json | 块、来源、图表公式、引文和参考文献之间的映射 | 2；后续只追加核验与对齐信息 |
| assets/ | 原图裁剪或提取结果、必要表格结构及核对材料 | 2 |
| translated.md | 逐块对应的中文全文；中文文献跳过时不生成重复文件 | 3 |
| terminology.json | 术语表路径与哈希、采用译法、语境、未决术语及块号 | 3 |
| drafts/06-properties.md | 以 content_input 或本次指定既有 MD 为基础的完整候选稿 | 6 |
| drafts/07-header.md | 上一步稿件的题头修订版 | 7 |
| drafts/08-body.md | 上一步稿件的排版版 | 8 |
| drafts/09-internal.md | 上一步稿件的篇内链接版 | 9 |
| link-map.json | anchors、internal_links、library_matches、unresolved；每项含来源及核验状态 | 9、10 各维护所属部分 |
| si.md | 补充材料原文稿：独立来源、S 编号命名空间、块标识 | 11 |
| si-map.json | 补充材料块、图、表、视频、题注及来源位置 | 11 |
| assets/si/ | 补充材料图与视频资源 | 11 |
| drafts/11-supplement.md | 主稿合并补充材料章节后的候选稿，第 6 步接收合并稿并沿候选链传至第 8 步 | 11 |
| drafts/10-library.md | 库内出链完成的候选稿，作为完整流程最终输入 | 10 |
| final.md | 本次最后候选去除工作块标记、固定最终资源路径后的待提交稿；正文其他内容不变 | 总入口 |
| write-receipt.json | 目标、备份、写前/候选/写后哈希、资源清单、CLI 回读及失败恢复记录 | 4 |
| report.md | 范围、内容覆盖、来源核验、结构核验、写入、界面与遗留问题的分别结论 | 总入口 |

单项修复允许从用户指定 MD 直接进入对应步骤，并声明前序哪些证据复用、哪些未做。输出使用对应阶段名；提交该次实际最终候选，不要求不存在的 10-library.md。

第 11 步（补充材料）在第 3 步之后、第 8 步之前执行：先把 SI 文本、图 S`N`、视频 S`N` 并入候选，再由第 8 步排版、第 9 步统一链接；SI 与主全文各用独立来源和编号命名空间，不拼接出混合版本。

独立调用子 skill 时，由当前执行者承担最小交接职责：先按实际范围完成[项目预检](project-profile.md#运行预检)，复用仍有效的证据并初始化必要记录；只读或仅规划任务交付对应产物。需要写库时生成 final.md，调用第 4 步提交并回读。若输入只有既有 MD，按其身份和证据工作，不为补齐可选字段启动整套流程。

## 核心字段

### run.json

- `scope`：明确的条目/文件列表、requested_stages、是否允许写库及是否要求 Zotero note 同步。局部修复另记修改对象/范围、输入基线及受影响链接，范围外既有问题仅登记，不默默扩大修改。
- `config`：已解析的项目根、vault 标识与绝对根、文献相对根、附件资源根（图片与视频三级目录的共同父目录）、工作根、模板、术语表、集合路径策略；preflight 记录本次范围所需的能力、路径检查、结果、证据与时间。
- `versions`：所用 skill/模板/术语表指纹；仓库版本以根 VERSION 为准，安装环境保留可取得的版本来源；不以修改时间替代内容指纹。
- `stages`：以 `01`…`11` 为键；第 4 步分别记录 plan、commit。每项保存 status、input_fingerprint、inputs、outputs、output_hashes、checks、issues、updated_at。
- `naming`：original_title、author_token、year、filename、清洗映射、冲突判定。
- `placement`：完整集合清单、选定集合、原因、target_relative_path、资源映射（每个图/视频的类别、年份、第一作者、title 哈希值、资源桶身份/版本消歧、资源归属及最终相对路径）、existing_path 及 expected_existing_hash；新建文件记录预期“不存在”。资源字节哈希与笔记规范化文本哈希分开记录。
- `content_input`：第 3 步选择的 source.md 或 translated.md；局部任务为已核实的现有稿件。
- `issues[]`：id、stage、severity（blocking/warning）、source_locator、说明、处理状态与证据。
- `publish`：prepared/committed/verified/partial/failed；另记 zotero_note_status。写入失败按实际文件与哈希记录，不能靠重写全文补偿。

时间使用带时区的 ISO 时间，取用户本地时区并保留偏移。不把运行时间当文献发表年份。

### item.json

- `raw` 保留接口实际返回的所有条目字段；`normalized` 提供题名、日期/年份、类型、作者角色、DOI 等工作值，不覆盖 raw。
- `collections[]` 包含 library、collection_key、完整 ancestry（逐级 key/name）；保留原名与本地映射名。
- `children` 保存可取得的 note 正文、附件信息及批注；不能取得的内容在 `coverage` 标为 partial/unavailable 并写原因。
- `attachments[]` 包含 key、link_mode、declared_mime、detected_format、resolved_path/url、availability、role、version、hash、probe_evidence；role 为 main/supplement/non_fulltext/unknown。
- `availability` 区分 local_readable、remote_only、missing、unreadable、unsupported、unknown；只有读到本地有效内容才是 local_readable。
- `language` 包含 declared、classification（zh/non-zh/mixed/unknown）、detected_languages、evidence、confidence（high/medium/low）、provisional，以及提取后的复核结果。
- `coverage` 按字段、集合、子项、附件、批注分别记录 complete/partial/unavailable 与分页完成情况；“接口未提供”不同于“原条目没有”。

### source-map.json 与 link-map.json

- 每个内容块有稳定 `block_id`、kind、source_attachment、source_locator、顺序、原文哈希、保留/排除决定及理由。定位可为 PDF 页/框、DOCX 段落、EPUB spine/元素或其他格式的可重现位置。
- source.md 用 `<!-- lit:block b000001 -->` 标识块；译稿及候选沿用 ID，拆并段记录一对多映射。总入口生成 final.md 时仅移除这些已登记的工作标记，保留可导航锚点和其他原有注释。
- objects 记录 figure/table/equation 的 source_id、原编号、子图、题注、资源及定位；references 记录 bibliography_id、原编号/标签、列表序号、原书目全文、稳定 ID，并另记插件解析出的命名空间、数字后缀及兼容状态；citations 记录出现位置、来源与书目作用域、原显示文字及目标候选。来源/书目身份不能由插件前缀或当前列表位置替代；相同数字在不同来源或书目中不是同一对象。
- 链接记录保存 origin_block、原显示文字、target_id/path、证据与状态。图表原号、导航序号、书目身份分开，不依赖“当前第几段”推断目标。引文显示按第9步规范，章节链接记录新旧目标。图表及公式保留原编号整串（含子图字母与面板），中文标签按第3、8步规范。跨图号范围不整体链接。
- 主文与补充材料按下方命名空间隔离；不占用彼此的图表、公式、视频、内容块和书目 ID。
- 图表目标另记 target_kind（image/table）、资源或数据块位置，以及原生解析的实际目标范围。原生解析范围须包含对应图片嵌入或覆盖完整表格。
- PDF 图像在 source-map、si-map 或局部修复记录中逐图保存[裁剪证据](../../literature-fulltext-extract/references/figure-crop.md)。写入回执以最终资源路径和哈希关联对应的视觉验收记录。
- 无法匹配分别记录 absent（库内无候选）、ambiguous（有歧义）、unavailable（未能检索）或 source_error（原文疑点）；不能一概写成“未找到”。

### 补充材料与书目命名空间

- `source_id` 标识实际来源，由已核实的库、条目及附件身份建立；同一附件内容变化另记哈希与版本，不把遍历次序当身份。每份独立书目有 `bibliography_id`：主文默认为 `main`，补充书目记录对应 `source_id` 与实际书目列表身份。不能凭“最近的一组数字”跨作用域匹配引文。
- 为尚无映射的对象新建 ID 时，单份 SI 默认使用块 `s000001`、图 `^fig-sN`、表 `^tbl-sN`、公式 `^eq-sN`、视频 `^mov-sN`。新建书目锚点使用主文 `^ref-N`、单份 SI `^sref-N`；`N` 取该书目列表序号（1–999999999），原文显示编号/标签另外保存。已有 ID 的保留规则见下，不按新建格式统一改名。
- 首次处理多份 SI 时，按稳定来源身份分配 `source_token`，采用该身份规范化记录的 SHA-256 十六进制前 12 位；冲突时延长并保存，不按文件名或处理次序重新分配。例如 token 为 `ab12cd34ef56` 时，块可为 `sab12cd34ef56-b000001`，图/表/公式/视频分别为 `^fig-sab12cd34ef56-N`、`^tbl-sab12cd34ef56-N`、`^eq-sab12cd34ef56-N`、`^mov-sab12cd34ef56-N`；新建书目使用 `^sref-srcab12cd34ef56-N`。书目前缀中的 `src` 固定保留，使 token 即使以数字开头也符合插件规则。同一附件含多份独立书目时，以来源身份和 bibliography_id 的组合为书目命名空间分配不冲突的稳定 token，独立登记，不改变该来源的对象 token。
- 以上 token 仅用于内部标识、锚点和必要的资源消歧。可见题注、公式号及引文标签仍采用原文编号，不能为消除重号改写原文。
- Citation Preview 1.3.0 按锚点本身解析书目命名空间，依据为该版本的 [referenceAnchor 实现](https://github.com/lyingTree/citation-preview/blob/165503f647590b7d9379e8b1ba72a2b9b61de049/src/bibliography.ts)。它接受 `ref-N`、`sref-N`、紧凑形式 `refN`/`srefN` 以及 `ref-appendix-N`、`sref-a-N` 等前缀；扩展前缀的每段须以 ASCII 字母开头，其余为字母或数字，数字后缀为 1–999999999。不以标题位置或 `ref`/`sref` 字样推断实际 source_id。
- 上述命名为全新书目的默认值。给既有书目补锚时，优先复用已核实且插件支持的命名空间，先核对数字后缀占用与条目身份；不能安全分配时登记待核，不改旧 ID 腾出编号。
- 复跑、增减 SI 或重排列表时，逐字保留已核实的既有 ID 和调用目标，包括 `^ref-s<token>-N`、`^ref-appendix-N`、`^refN` 等插件已支持的形式；数字后缀与当前列表序号不一致时只登记需核对项，不自动重排。旧 `^ref-sN` 或其他有效 Obsidian 块 ID 若不受插件识别，保留原 ID 与链接并记录插件兼容 warning，不将格式差异当成断链。新增来源使用独立 token。
- 仅在任务范围明确要求迁移既有 ID 时，按来源与书目身份建立逐条映射，迁移全部已知调用及可枚举入链；无法安全迁移则阻断该迁移，不因格式统一删除旧 ID。真正缺锚、重复定义或目标错配仍按问题处理。

## 状态、重跑与保护

- 阶段状态：pending、running、passed、blocked、failed、skipped。skipped 必须有适用性理由，如中文无需全文翻译；不等于完成未做的核验。
- passed 要有相应检查证据，不能仅凭输出文件存在。某些非必需信息不可用时记录缺项，完成范围按 coverage 陈述。
- 检查证据分为文件/静态源码、原文内容、CLI 回读、原生缓存、离屏渲染及阅读视图；每类分别保存范围、结果和位置。脚本零问题只表示其实际检查范围通过，不自动把阶段、整篇或界面状态置为 passed。
- 输入指纹包含实际消费的来源、配置和版本。阶段输出不可被下一阶段原地覆盖；重跑同输入复用输出，不重复追加标题、序号、锚点或笔记。
- 产物指纹一致但缺少验收证据时，仅补缺项检查。
- 指纹变化时，从实际变化的附件、块、术语或路径定位受影响部分；下游实际消费的内容或目标改变时才重做相应部分。
- 单纯调整说明文字时核对规则语义，记录新版本；复用仍满足现行标准的产物及证据。
- 不同条目可有界并行，先按工具限流、资源和用户要求登记并发上限；同一条目的阶段按依赖顺序执行。各条目使用独立工作目录和 run.json，总控汇总只读状态，不让多个执行者同时覆盖同一状态文件。大批量任务持续报告完成、运行、阻断和剩余数，异常条目留证后让其他独立条目继续。
- 不同时提交同一条目或目标。提交前重新检查预期哈希/不存在条件；变化时保留候选，停止覆盖并报告冲突。
- 以条目身份和最终路径取得独占写入记录，其他执行者发现占用即停止提交；异常遗留记录先核实实际执行状态，不按时间自动抢占。哈希比较是修改检测，不能声称它单独消除了外部应用同时写入的风险。
- 有关键缺页、未辨符号、遗漏段落或身份冲突的完整流程不能发布为已完成。可保留待核稿；用户明确要求保存不完整稿时另记 partial 与未决范围。
- 文件写入成功但后续中断时，先检查回执、实际文件和哈希，再继续回读或状态记录；不盲目重建副本，也不删除未知归属的资源。
