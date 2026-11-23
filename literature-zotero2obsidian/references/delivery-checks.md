# 来源清单与交付检查

## 目录

- [检查边界](#检查边界)
- [准备来源证据](#准备来源证据)
- [填写原子来源单元](#填写原子来源单元)
- [记录各来源的题头](#记录各来源的题头)
- [映射最终候选](#映射最终候选)
- [逐图视觉核对记录](#逐图视觉核对记录)
- [合并单元格](#合并单元格)
- [完成交付检查](#完成交付检查)

## 检查边界

[prepare_source_inventory.py](../scripts/prepare_source_inventory.py) 从实际 PDF 生成逐页证据。[validate_delivery.py](../scripts/validate_delivery.py) 将这些证据与最终候选关联起来。两者都是本地脚本，不连接 Zotero 或 Obsidian。

`structurally_consistent: true` 表示已记录的证据链没有发现机器可检出的矛盾。它不表示源清单分类完整，不表示模型确实看过页面，也不表示译文、图件或科学内容正确。报告始终保留 `source_fidelity: not_assessed` 与 `independent_review_required: true`。

一个清单单元对应一个来源对象，例如一名作者、一条单位、一段正文、一个公式、一张表或一次引文调用。整个文献不能压成一个“正文已完成”单元。源段落与译文的完整对应仍须独立验收。

## 准备来源证据

完整流程在工作目录的调用脚本中导入函数，把本次参数保存在调用脚本中。保留技能目录内源码不变。独立编辑器使用时也可先将来源准备脚本复制到工作目录，再设置其顶部参数。

```python
from pathlib import Path
from prepare_source_inventory import prepare_sources

run_root = Path("本次运行根")
work = run_root.joinpath("work")
input_root = run_root.joinpath("input")
output_root = run_root.joinpath("output")
skills_root = run_root.joinpath("skills")
inventory_path = prepare_sources(
    {
        "main": input_root.joinpath("main.pdf"),
        "si_a": input_root.joinpath("supplement.pdf"),
    },
    work.joinpath("evidence"),
    source_roles={"main": "main", "si_a": "supplement"},
    glossary_path=skills_root.joinpath("template", "术语表.md"),
)
```

该调用生成 `source-inventory.json`。每个来源包含实际 PDF 的路径、SHA-256 和页数。每页包含完整 PNG、文字层文件和非空文字块。PNG 的像素哈希与文件哈希分别保留。脚本显示逐页进度，不覆盖已有同名清单。

传入 `glossary_path` 时同时创建 `terminology.json`，术语表路径与哈希由实际文件生成，其余采用记录仍待填写。按[翻译技能](../../literature-translate-zh/SKILL.md#术语记录的具体做法)逐块完成术语台账；`write_delivery_manifest()` 自动绑定台账的最终文件哈希。`TERMINOLOGY_PENDING` 表示记录尚未完成，`TERM_UNRESOLVED` 表示仍有译法未决；台账通过结构检查不等于译义已通过独立验收。

生成的页面只是待查看证据。文字层与可见页面可能冲突。题头、日期、公式、表格和图件的清单证据采用 `basis: visual`；该标记只是记录所用证据类型，不会替代实际查看页面。

## 填写原子来源单元

读取 `source-inventory.json` 的 `sources[].pages[]`。逐页查看 PNG 后填写 `units`。不修改机器生成的 `sources[].pages[].blocks`；检查器会从原 PDF 重新提取这些块进行比较。

单元的基本结构如下。示例内容与坐标只解释格式，实际值取自本次来源。

```json
{
  "id": "main-title",
  "source": "main",
  "kind": "metadata",
  "field": "title",
  "evidence": [
    {
      "page": 1,
      "basis": "visual",
      "bbox": [40, 35, 550, 90],
      "quote": "The literal title visible on the source page",
      "blocks": ["main-p1-b0"]
    }
  ]
}
```

| 字段 | 表示的单一目标 |
|---|---|
| `id` | 本次清单内的唯一对象身份。 |
| `source` | 来源 PDF 的身份。 |
| `kind` | 对象的种类。 |
| `evidence[].page` | 原对象所在的 PDF 实际页码。 |
| `evidence[].basis` | 此项使用 `visual` 或 `text` 证据。 |
| `evidence[].bbox` | 对象在该页的左上原点坐标，单位为 PDF 点。 |
| `evidence[].coordinate_system` | 图框 bbox 所采用的明确坐标系。 |
| `evidence[].quote` | 对象的来源原文片段。 |
| `evidence[].blocks` | 此对象覆盖的机器提取文字块 ID。 |

每个 evidence 至少给出 `bbox` 或 `quote`。跨页对象使用多个 evidence。一个原文字块可以对应多个单元，例如同一块中的作者和单位。没有文字层的图或公式可用 bbox 定位。

| `kind` | 额外数据 |
|---|---|
| `metadata` | `field` 为 `title`、`author`、`affiliation`、`publication_date`、`doi`、`email`、`header_note` 或 `external_url`。DOI、邮箱与网址另给来源原值 `value`。 |
| `paragraph` | 一段原文对应一个单元。 |
| `section` | 一个来源章节标题对应一个单元。 |
| `figure` | 用布尔值 `contains_table` 记录图中是否有真正的行列表格。 |
| `table` | `shape` 给数据行数和列数；`merges` 给源合并范围。嵌图表另给父图 `parent_id`。 |
| `formula` | 一个来源公式对应一个单元。 |
| `reference` | `number` 保存来源书目的编号或标签。 |
| `citation` | `references` 保存该次调用对应的书目单元 ID。 |
| `cross_reference` | `target_unit` 保存该次图表、公式或章节调用的实际对象 ID。 |

引用在源文中出现两次时建立两个 citation 单元。它们可以指向同一 reference 单元。跨来源使用书目时，另用 `cross_source_reason` 保存来源依据；主文与 SI 的同号书目不是同一条。

单位后的每条通讯说明、版权说明等独立登记为一个 `metadata` / `header_note` 单元，目标为该来源题头引用块中的对应行。作者行中的邮箱链接另属于 `email`；这条链接不能代替来源中的完整通讯说明。`HEADER_NOTE_LAYOUT` 检查已登记说明是否映射到引用块；说明有没有漏盘点仍须对照源页核验。

每处来源裸网址独立登记为 `metadata` / `external_url`，`value` 为经源页核定的完整 URL，`display_text` 为来源的原显示文字。每次调用有独立的目标片段。例如来源显示 `https://example.org/data/`，则两个字段均保留该 URL；源页原本显示命名链接时，`display_text` 保留该名称。`URL_DISPLAY_TEXT` 检查显示文字，`URL_LINK` 检查实际目标。来源中的排版断行须按源页复原，不凭网站名称替换显示文字。DOI 解析网址使用 `doi` 专用字段，按 DOI 规范化规则处理。

图中存在真正表格时，图件仍完整保留。表格另建 `table` 单元。地图标签、刻度、色标或普通图例不因此变成表格。

新图单元的 bbox 同时登记 `coordinate_system: {"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"}`。这些坐标采用未旋转的 PDF 页面空间，不采用渲染 PNG 的像素空间。bbox 顺序为 `[x0, y0, x1, y1]`。原图对象定位可另用该页的实际 `image_xref`；字段位置在图单元的来源 `evidence[]` 中。

每个文字块须被某单元的 evidence 引用，或进入 `excluded_blocks`。排除记录只使用实际块 ID与具体原因：

```json
{"block": "main-p2-b0", "reason": "该块仅为与上一页重复的运行页眉，不含新增正文。"}
```

整页排除记录使用 `excluded_pages`，例如 `{"source":"main","page":12,"reason":"来源中的空白页。"}`。排除原因的正确性由独立评审核验；非空理由不是语义通过凭证。

## 记录各来源的题头

主文与每份 SI 分别填写自己的 `header_fields`。四类决策是 `title`、`authors`、`affiliations`、`publication_date`。存在该字段时引用对应单元：

```json
{
  "title": {"status": "present", "units": ["main-title"]},
  "authors": {"status": "present", "units": ["main-author-1", "main-author-2"]},
  "affiliations": {"status": "present", "units": ["main-affiliation-1"]},
  "publication_date": {"status": "present", "units": ["main-date"]}
}
```

独立 SI 没有某字段时使用 `absent_in_source`。记录具体来源页证据，不从主文复制一个值来填空：

```json
{
  "status": "absent_in_source",
  "reason": "该附件题头未列作者单位。",
  "evidence": [{"page": 1, "basis": "visual", "bbox": [0, 0, 595, 220]}]
}
```

有字段就交付字段；没有字段就记录不存在的依据。生成器留下的 `pending` 在最终检查中会报错。

## 映射最终候选

正文写完后建立 `deliveries` 列表。每个 unit 都有一个最终目标。优先采用稳定块锚或包含足够上下文的唯一原样片段：

```python
deliveries = [
    {"unit": "main-title", "target": {"quote": "The complete final title"}},
    {"unit": "main-ref-1", "target": {"anchor": "ref-1"}},
    {"unit": "main-equation-1", "target": {"anchor": "eq-1"}},
    {"unit": "main-table-1", "target": {"anchor": "tbl-1"}},
    {
        "unit": "main-figure-1",
        "target": {"quote": "![[文献/attachments/2020/Example/0123456789abcdef/fig1.png]]  \n**图1：** 完整中文题注。\n^fig-1"},
        "assets": [{"path": "文献/attachments/2020/Example/0123456789abcdef/fig1.png"}],
    },
]
```

示例路径只解释格式，实际路径使用本次核实的年份、首作者与资源桶。`anchor` 只写 ID，不加 `^`。图件目标须覆盖实际嵌入语法；只选题注不足以检查图片路径。所有嵌入的本地资产都须有文件哈希。图件资产还须通过实际图像解码。

离线候选默认以 Markdown 所在目录为资源解析根。真实笔记按已核实的 vault 根路径解析库根相对附件路径；调用 `write_delivery_manifest(..., resource_root=vault_root)` 明确记录该根。嵌套在 `vault/文献/分类/` 的候选不能用笔记所在目录去拼接 `文献/attachments/`。检查器按记录的根解析 `assets[].path` 和图片嵌入路径，不猜测目录。

引文目标选中该次实际链接及必要上下文。所选范围中的书目链接须与 `references` 精确相符。不要为一次调用选择包含其他调用的整个长段。

`cross_reference` 的目标映射使检查器能够区分具体方法小节与整个 SI 章。给 section 单元选择完整的 Markdown 标题行；给 cross_reference 单元选择实际链接所在片段。

如需行号定位，使用 `{"lines":[起行,止行],"sha256":"该行段的真实哈希"}`。行段按 UTF-8、LF、包含原有行尾换行计算。整个候选的文件哈希同时绑定，后续编辑会使检查失败。

## 逐图视觉核对记录

新生成的完整交付流程在图单元已盘点、最终图文件已准备好后调用 `prepare_figure_reviews(...)`。旧清单没有图像台账时仍可执行原有检查；这种兼容结果不表示已经完成逐图视觉核对。

以下代码从实际清单和交付映射取值，不通过图号数量猜测对象，也不生成视觉结论：

```python
import json
from pathlib import Path
from prepare_source_inventory import prepare_figure_reviews

inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
figure_ids = {
    unit["id"]
    for unit in inventory["units"]
    if unit["kind"] == "figure"
}
final_assets = {
    delivery["unit"]: [Path(asset["path"]) for asset in delivery["assets"]]
    for delivery in deliveries
    if delivery["unit"] in figure_ids
}
figure_reviews_path = prepare_figure_reviews(
    inventory_path,
    final_assets,
    work.joinpath("figure-review"),
    resource_root=output_root,
)
```

相对图片路径按 `resource_root` 解析。实际笔记使用已核实的 vault 根；本示例使用离线 `output_root`。传入的映射须恰好覆盖实际图单元。函数保留跨页定位顺序，验证所引用的实际文件，并给每幅图建立 FIG-A01 至 FIG-A28 共 28 个 `pending` 条目。函数在清单中登记 `figure_reviews_path`，不会覆盖已有台账。

每幅图的数据分开保存如下：

| 字段 | 单一目标 |
|---|---|
| `unit` | 对应实际清单中的图身份。 |
| `source` | 对应该图自己的来源身份。 |
| `source_file` | 绑定来源 PDF 文件。 |
| `source_locations[]` | 定位本图在来源中的位置。 |
| `source_locations[].page_render` | 绑定该源页的 PNG。 |
| `final_assets[]` | 绑定最终待嵌入图像文件。 |
| `checks[].id` | 指定一个 FIG-A 原子目标。 |
| `checks[].goal` | 保留该 ID 对应的目标文字。 |
| `checks[].status` | 记录该目标的核对状态。 |
| `checks[].reason` | 记录该结果的具体理由。 |
| `checks[].evidence` | 指向本条结果的查看证据。 |
| `checks[].applicability_condition` | 说明不适用结果的具体条件。 |

台账内的文件引用只包含 `path` 和 `sha256`。`final_assets[]` 与 `deliveries[].assets[]` 不夹带来源 bbox。页码、bbox、坐标系及原图对象位置属于 `source_locations[]`。检查器将它们与原清单逐项对应。

生成器还保存不可覆盖的 `figure-inventory-snapshot.json`。台账顶层的 `inventory` 绑定该真实快照文件的哈希；`inventory_path` 单独指向当前实际库存，检查器核验这个文件身份。当前库存与快照进行严格内容比较，只忽略顶层 `terminology_path`、`figure_reviews_path`、`checks_path` 三条工具指针。这些指针由各自台账的交付绑定检查负责，因此随后新增 `checks_path` 不会使已核对的图记录失效。来源、units、嵌套字段和所有其他内容变化仍触发 `FIG_REVIEW_INVENTORY_CHANGED`。快照文件自身变化仍触发文件哈希错误。

模型实际查看所列源页 PNG 与最终图片后填写结果。每条证据采用 `{"kind":"source_page","index":0,"observation":"本次实际查看的内容"}` 或 `{"kind":"final_asset","index":0,"observation":"本次实际查看的内容"}`，`index` 是相应数组的零起始位置。一个项目可以引用多页或多幅图片。视觉目标的 `pass` 记录须同时引用来源页和最终图片。

不存在的对象使用 `not_applicable`。以下只演示一幅无坐标轴照片的记录格式，不是本次来源的结论：

```json
{
  "id": "FIG-A04",
  "goal": "轴线完整。",
  "status": "not_applicable",
  "reason": "源图是一幅未叠加坐标轴的照片。",
  "applicability_condition": "这幅来源图没有坐标轴。",
  "evidence": [
    {"kind": "source_page", "index": 0, "observation": "源页中的完整照片未显示坐标轴。"},
    {"kind": "final_asset", "index": 0, "observation": "最终图片对应同一照片。"}
  ]
}
```

全部条目填写后才执行下一步交付绑定。`write_delivery_manifest()` 会从清单读取 `figure_reviews_path`，把台账最终哈希写入 `figure_reviews`。后来编辑台账必须重新绑定；替换最终图片必须重新核对该图。

检查器逐图检查原子 ID 集合、理由、证据引用及不适用条件。检查器核对最终图像与交付映射的路径和哈希。检查器拒绝缺失坐标系和像素坐标声明。`FIG_REVIEW_PENDING` 表示仍有未决项目，`FIG_REVIEW_FAILED` 表示仍有失败项目。`FIG_REVIEW_CHECK_MISSING`、`FIG_REVIEW_CHECK_UNKNOWN` 与 `ID_DUPLICATE` 分别指出缺失、未知及重复身份。无效 schema 或字段类型返回错误发现，不作为通过结果。

台账存在、自填 `pass` 或结构检查通过，都不构成独立视觉核验。报告仍保留 `source_fidelity: not_assessed` 与 `independent_review_required: true`。

## 合并单元格

每个 table 单元记录 `shape: {"rows": 数据行数, "columns": 列数}`。源表无合并时记录 `merges: []`。存在合并时用原表网格坐标记录；数据行、列从 1 开始，表头行为 0：

```json
{
  "shape": {"rows": 4, "columns": 3},
  "merges": [{"row_start": 1, "row_end": 3, "column_start": 1, "column_end": 1}]
}
```

最终 Rich Table 的真实格式是 `merges: [{anchor: "r1.c1", end: "r3.c1"}]`。这两个端点使用目标表的行 ID 和列 ID。合并区域仅左上角保存显示值，覆盖格为空或省略。`header.c1` 是表头端点形式。该结构依据 [Rich Table 的固定版本模型定义](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/src/model.ts) 与 [官方技能说明](https://github.com/SdKay/obsidian-rich-table/blob/d1717904a5d0b4f42d11671ee6129b721394d0a1/skills/obsidian-rich-table/SKILL.md)。

v2 的 `merges` 位于表模型顶层。v3 的 `merges` 位于对应工作表中。v3 每个工作表单独对应一个 table 单元，delivery 另加 `sheet: {"id":"实际工作表ID"}` 或唯一 `name`。检查器逐表比较源合并范围与目标范围；重复填入分组文字不等于保留合并。

## 完成交付检查

候选文件和清单实际存在后再绑定哈希：

```python
from validate_delivery import write_delivery_manifest, validate_delivery
from prepare_source_inventory import progress_bar

manifest_path = write_delivery_manifest(
    inventory_path,
    output_root.joinpath("candidate.md"),
    deliveries,
    work.joinpath("delivery-manifest.json"),
    resource_root=output_root,
)
report = validate_delivery(manifest_path, progress=progress_bar)
print(report.to_dict())
```

检查器重新计算来源 PDF 和候选的哈希。检查器重新确认 PDF 页数。检查器将每页 PNG 的 RGB 像素与该 PDF 页重新渲染的结果比较。检查器从 PDF 重新提取文字块。检查器验证源单元与交付目标的双向映射。现有 `validate_note.py` 同时检查 Rich Table 所有工作表的锁定、主题和本篇断链。交付检查还检查每张工作表是否有来源单元；未映射的后续工作表也会检查合并端点。

交付目标中的公式、表格和书目锚须属于实际可见对象。代码示例里的同名对象不能代替交付。Rich Table 的引文按 YAML 解码后的可见字段检查，注释不计作调用。既有书目锚的插件兼容提示沿用只读 warning，不据此强制迁移已有链接。

`SOURCE_BLOCK_UNMAPPED` 定位遗漏盘点的文字块。`HEADER_PENDING` 定位未核实的独立题头字段。`UNIT_NOT_DELIVERED` 定位尚未交付的来源对象。`TARGET_QUOTE` 定位不唯一或已变化的目标片段。`EMBEDDED_TABLE_MISSING` 定位图内表格的独立结构缺口。`TABLE_MERGES_MISMATCH` 定位合并范围差异。`OUTPUT_CITATION_UNMAPPED` 定位没有来源调用记录的书目链接。

按发现逐项修复后重新绑定最终候选哈希。保留完整机器发现清单。最后另行对照原页核验译文、数字、公式、书目身份、引文语境和图像完整性；Obsidian 原生缓存与插件界面仍单独验收。
