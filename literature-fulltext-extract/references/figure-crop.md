# PDF 图像裁剪与验收

逐幅执行。复用图采用相同标准。每个 ID 单独记录结果及证据。

## 定位与提取约束

- [FIG-C01] 按图号确认来源附件。
- [FIG-C02] 记录图所在的 PDF 页码。
- [FIG-C03] 确认原题注对应的图号。
- [FIG-C04] 结合源页渲染确定实际图框。
- [FIG-C05] 文本坐标仅作为图框定位线索。
- [FIG-C06] 能取得完整原图时优先提取原图。
- [FIG-C07] 多对象组成的图按实际图框高分辨率渲染或裁剪。
- [FIG-C08] 在实际图框外保留少量安全边距。
- [FIG-C09] 原题注单独保存。
- [FIG-C10] 整页截图仅用于取证。
- [FIG-C11] 图占满可见页面时记录按实际图框验收的依据。
- [FIG-C12] 为跨页图登记各页裁剪范围。
- [FIG-C40] 跨页图的记录顺序与面板原顺序一致。
- [FIG-C41] 提取原图对象时检查是否存在独立透明度遮罩。
- [FIG-C42] 存在透明度遮罩时按 PDF 显示方式合成输出图像。
- [FIG-C43] 需要重编码时采用无损输出格式。
- [FIG-C63] 从本次来源清单的实际图像调用记录中选择 `image_xref`。
- [FIG-C64] 原图导出使用已登记来源文件的哈希校验。
- [FIG-C65] 不把单个图像对象自动认定为整幅 figure。
- [FIG-C66] 图像调用未解析时保留明确的未决记录。
- [FIG-C67] 原图显示效果无法独立保留时使用 PDF 页面渲染。

## 视觉验收

- [FIG-C13] 将原页与最终待嵌入图片并排比较。
- [FIG-C14] 先查看完整图件。
- [FIG-C15] 放大检查图件边缘。
- [FIG-C16] 放大检查细小文字。
- [FIG-C17] 不以资源存在代替视觉验收。
- [FIG-C18] 不以资源可加载代替视觉验收。
- [FIG-C19] 不以资源可解码代替视觉验收。
- [FIG-C20] 不以图像尺寸代替视觉验收。
- [FIG-C21] 不以锚点有效代替视觉验收。
- [FIG-C44] 在像素比较记录中注明解码方式。
- [FIG-C45] 白底像素比较使用已合成透明度的图像。
- [FIG-C46] 未确认可见内容改变时，不仅凭编码器造成的像素差判定图件损坏。
- [FIG-C68] 对照完整源页复核最终图件的上边缘。
- [FIG-C69] 对照完整源页复核最终图件的右边缘。
- [FIG-C70] 对照完整源页复核最终图件的下边缘。
- [FIG-C71] 对照完整源页复核最终图件的左边缘。
- [FIG-C72] 不用选定裁剪范围内的像素相同证明图件没有缺裁。

- [ ] [FIG-A01] 子图面板齐全。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A02] 面板号完整。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A03] 图内文字完整。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A04] 轴线完整。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A05] 刻度完整。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A06] 单位完整。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A07] 图例完整。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A08] 色标完整。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A09] 图内要素可读。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A10] 图外题注没有混入图件。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A11] 相邻图件没有混入。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A12] 正文没有混入图件。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A13] 页眉没有混入图件。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A14] 页脚没有混入图件。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A15] 页码没有混入图件。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A16] 图件没有大块页边空白。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A17] 图号与图件对应。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A18] 题注与图件对应。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A26] 输出图像的透明度效果与源页一致。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A27] 输出图像的颜色与源页一致。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A28] 图内数据表在原图中完整保留。 <!-- check-type: offline_outcome -->

## 记录与提交

- [FIG-C22] 记录来源附件路径。
- [FIG-C23] 记录来源附件哈希。
- [FIG-C24] 记录图框坐标或原图对象定位。
- [FIG-C25] 使用图框坐标时记录坐标系。
- [FIG-C26] 记录输出图片路径。
- [FIG-C27] 记录输出图片哈希。
- [FIG-C28] 记录每项视觉验收结果。
- [FIG-C29] 记录视觉证据位置。
- [FIG-C30] 最终嵌入资源必须与已验图片一致。
- [FIG-C31] 修复缺裁的图。
- [FIG-C36] 对修复后的缺裁图重新验收。
- [FIG-C32] 修复多裁的图。
- [FIG-C37] 对修复后的多裁图重新验收。
- [FIG-C33] 替换错误的图件。
- [FIG-C38] 对替换后的图件重新验收。
- [FIG-C34] 核实不确定的图框。
- [FIG-C39] 按已核实图框重新验收图件。
- [FIG-C35] 未通过的图阻断对应提交。

- [ ] [FIG-A19] 来源页码已登记。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A20] 来源附件哈希已登记。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A21] 图框或原图对象定位已登记。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A22] 适用的坐标系已登记。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A23] 最终图片哈希已登记。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A24] 视觉证据位置已登记。 <!-- check-type: offline_outcome -->
- [ ] [FIG-A25] 最终嵌入图片与已验资源哈希一致。 <!-- check-type: offline_outcome -->

## 原图对象索引与导出

先运行 [prepare_source_inventory.py](../../literature-zotero2obsidian/scripts/prepare_source_inventory.py) 的 `prepare_sources(...)`。每页的 `image_index.occurrences` 给出实际 PDF 内容流中调用的 `image_xref`、资源名、调用位置、完整对象 `bbox` 及六项 `transform`。嵌套 Form 的调用链单独保存。对象身份来自实际 `Do` 调用；相同解码像素不会被当作相同 xref，资源表中的未使用图片也不会被列为实际出现。

每份来源的 `image_objects` 按 xref 去重。每个对象登记 `original_encoded_extension`、`extracted_extension`、原始提取文件 `raw_asset` 的路径与 SHA-256，以及 SMask 的 xref、编码、文件哈希和透明度范围。复用同一 xref 的页面共用一个提取文件。JPEG 等独立编码保留原编码；PDF 的 Flate 等采样流本来没有独立图片扩展名，此时 `original_encoded_extension` 为 `null`，`extracted_extension` 说明无损封装后的实际格式。`raw_asset` 是原对象的诊断文件，带蒙版时直接复制它会丢失独立蒙版。

查看源页后，在自己的工作脚本中设置已选中的页码和 xref，再调用 [pdf_image_objects.py](../../literature-zotero2obsidian/scripts/pdf_image_objects.py)。下面的常量需要填写本次来源中的真实值；目标路径不写扩展名时，由实际输出编码决定扩展名。

```python
import json
from pathlib import Path

from pdf_image_objects import export_image_object

INVENTORY_PATH = Path("work/source-inventory.json")
SOURCE_ID = "main"
PAGE_NUMBER: int | None = None
IMAGE_XREF: int | None = None
FINAL_ASSET_STEM = Path("output/assets/selected-figure")
EXPORT_RECORD_PATH = Path("work/selected-image-export.json")

if PAGE_NUMBER is None or IMAGE_XREF is None:
    raise ValueError("先根据实际源页和 image_index 填写页码及 image_xref")

inventory = json.loads(INVENTORY_PATH.read_text(encoding="utf-8"))
source = next(row for row in inventory["sources"] if row["id"] == SOURCE_ID)
record = export_image_object(
    Path(source["path"]),
    page_number=PAGE_NUMBER,
    image_xref=IMAGE_XREF,
    destination=FINAL_ASSET_STEM,
    expected_source_sha256=source["sha256"],
)
with EXPORT_RECORD_PATH.open("x", encoding="utf-8") as stream:
    json.dump(record, stream, ensure_ascii=False, indent=2)
```

函数重新核对 xref 的真实图像类型及其在所选页的实际调用。常规 DeviceRGB/DeviceGray 原图无显示冲突时，按原提取字节导出；非不透明 SMask 合成为具有正确 alpha 的无损 PNG。完全不透明的 SMask 不要求改变原图编码。SMask 的 `Decode` 由 PDF 解码器实际应用，导出记录保留该定义。目标扩展名错误、来源哈希变化、非图像 xref、越界页码或目标已有不同内容都会报错。

ICC、CMYK、索引色、资源定义的色彩空间、嵌入式 `SMaskInData`、Matte、不同尺寸的蒙版，以及需要页面裁切或图形状态的显示效果，使用 `PageRenderRequired` 明确转入页面区域渲染。内联图像没有可选择的间接 xref；未解析的内容流、Type 3 字形程序及图案内容会在索引 `issues` 中说明，不能把 `partial` 或 `unresolved` 当成完整盘点。原编码文件仍可作为来源证据保留。

`bbox` 的含义是**裁切前的完整对象边界**，不是已经确认的完整 figure 边界。`transform` 把左上角为原点的图片单位方框 `[0, 0, 1, 1]` 映射到未旋转页面空间；图片像素先分别除以原图宽、高再应用该变换。页面自身的旋转、CropBox 与坐标变换单独保存在 `image_index`。导出的原图保持原始像素方向，比较时依据该次调用的变换核对。对象外的矢量线条、面板号、其他图片或文字可能仍属于同一幅图，需从完整源页确认。

导出结果的 `asset` 只含 `path` 和 `sha256`，来源定位与透明度处理另存。`visual_review` 初始为 `pending`，`figure_completeness` 为 `not_assessed`。这些状态仅指源 PDF 与图片资源文件的内容核对。导出成功不填写 FIG-A 的通过结论。

## 裁剪路径的四边复核

PDF Form 的 BBox 可用任意一对对角点表示，工具先规范化坐标再处理裁切，依据是 [PDF Reference §3.8.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.5_v6.pdf)。真正零面积的 Form 记录为空裁切；无法解析的 Form 保留局部未决记录，后续其他图像调用继续索引，整体不会因此获得完整通过结论。

PDF 中存在旋转、镜像、斜切或非等比显示时，独立原编码图片可能改变图件的显示方式，导出函数也会明确要求页面渲染。`render_page_region(...)` 的输入仍使用未旋转 PDF 点坐标，输出图片保留源页实际显示方向。

当图由多个对象组成，或独立导出不能保留 PDF 的显示效果时，先在完整源页上确定全部图件内容。以完整对象 bbox 为起点；对象外仍有属于图件的标注时，扩展到这些标注的实际边界。然后使用 `render_page_region(...)` 渲染已经确认的页面范围。函数要求明确的未旋转 PDF 点坐标，默认向四周各加 6 pt 安全边距，实际边距受页面边界限制并逐边登记。这个默认值只是起点，不能代替目视核对。

```python
from pdf_image_objects import PDF_BBOX_COORDINATES, render_page_region

VERIFIED_BBOX: list[float] | None = None
if VERIFIED_BBOX is None:
    raise ValueError("先在完整源页确定包含所有图件要素的 PDF 点坐标范围")

record = render_page_region(
    Path(source["path"]),
    page_number=PAGE_NUMBER,
    bbox=VERIFIED_BBOX,
    coordinate_system=PDF_BBOX_COORDINATES,
    destination=Path("output/assets/selected-figure.png"),
    margin_pt=6,
    dpi=300,
    expected_source_sha256=source["sha256"],
)
```

保留完整源页文件，并打开最终输出的图片文件。先完整比较图件，再放大到图内小字和端点可辨认的尺度，逐边核对：

1. **上边缘**：沿整条上边缘比较其内侧内容与源页中对应边界外侧的一条区域，确认上方的面板号、标题、刻度或标注没有留在输出之外。
2. **右边缘**：沿整条右边缘核对端点、图例、色标及右侧标注在源页中的实际终点，确认它们完整进入最终图片。
3. **下边缘**：沿整条下边缘核对下方刻度、轴标题、脚标及图内表格的末行，确认底部没有被裁掉。
4. **左边缘**：沿整条左边缘核对左侧刻度、单位、轴标题及线条起点，确认左侧要素完整保留。

发现线条、字符或面板在边缘被截断时，依据源页扩展该边，重新输出并重新检查受影响的边。边距不足时继续扩展；加入了相邻正文或题注时，在保留图件全部要素的前提下修正边界。`render_page_region` 的 `edge_reviews` 为四条边各留一个 `pending` 记录；逐边填写实际观察、源页/最终文件的证据定位和结果。核对依据必须能看到裁框外的源内容，只有裁框内的像素比较无法发现被排除的部分。

四边检查后继续执行现有 FIG-A 项，核对面板、文字、轴线、刻度、单位、图例、色标及图内表格。将最终输出文件的哈希绑定到逐图台账；任何后续修改按修复后的图片重新核对。

## 可执行的逐图核对台账

- [FIG-C47] 新生成流程建立 `figure-reviews.json`。
- [FIG-C48] 新核对条目的初始状态为 `pending`。
- [FIG-C49] 不适用条目登记具体适用条件。
- [FIG-C50] 最终图片文件引用只含 `path` 与 `sha256`。
- [FIG-C51] 来源定位保存在独立的 `source_locations` 字段。
- [FIG-C52] 交付清单绑定核对台账的最终哈希。
- [FIG-C53] 未决核对条目阻断对应提交。
- [FIG-C54] 不把自填结果视为独立视觉核验。
- [FIG-C55] 每幅图的每个 FIG-A 目标只登记一次。
- [FIG-C56] 台账中的图身份对应实际清单图单元。
- [FIG-C57] 每条核对结果登记理由。
- [FIG-C58] 每条核对结果登记证据引用。
- [FIG-C59] 图台账绑定独立的库存快照。
- [FIG-C60] 已有库存快照不得覆盖。
- [FIG-C61] 图台账登记当前库存文件的身份。
- [FIG-C62] 当前库存的非工具指针内容与快照一致。

完整调用见[交付检查中的逐图视觉核对记录](../../literature-zotero2obsidian/references/delivery-checks.md#逐图视觉核对记录)。`prepare_figure_reviews(...)` 从本次 `source-inventory.json` 的 `kind: figure` 单元与最终资产映射生成台账；每幅图保留 FIG-A01 至 FIG-A28 的独立条目。生成器不会替模型填写通过结果，也不会自动删除它猜测不适用的项目。

图框坐标采用 PyMuPDF 的未旋转页面空间。单位为 PDF 点，原点是页面左上角，x 轴向右，y 轴向下。图单元的每个 bbox 记录以下完整声明：

```json
{
  "space": "pdf_page",
  "unit": "pt",
  "origin": "top_left",
  "x_axis": "right",
  "y_axis": "down",
  "rotation": "unrotated"
}
```

该声明放在 `coordinate_system`，bbox 顺序为 `[x0, y0, x1, y1]`。渲染图片的像素坐标不能直接填入 bbox；从像素定位时先处理实际 DPI 和页面旋转，再保存为上述页面坐标。检查器会拒绝像素单位、缺失声明及越出源页的框，但坐标声明本身不能证明图框选得正确。直接定位 PDF 原图对象时可记录同页实际 `image_xref`；未使用 bbox 的条目可将 FIG-A22 标为 `not_applicable`，并说明具体条件。

台账将来源 PDF、源页 PNG、最终图片分别绑定到实际文件。最终图片的引用保持最小 `{path, sha256}` 结构；来源页码、bbox、坐标系和原图对象位置保存在来源定位记录。跨页图按来源顺序保留多个定位。

生成器另写不可覆盖的 `figure-inventory-snapshot.json`。台账的 `inventory` 是该快照的最小文件引用，`inventory_path` 单独记录当前实际库存路径。校验时，当前库存与快照除顶层 `terminology_path`、`figure_reviews_path`、`checks_path` 三条工具指针外，内容须严格一致。这三条指针由各自清单绑定校验，因此随后创建其他工具台账不会使图台账误报；来源、单元、嵌套字段或其他内容变化仍会报错。

模型查看源页和最终图件后逐条填写 `status`、`reason`、`evidence`。图中没有面板号、轴线或色标等对象时，用 `not_applicable` 并填写 `applicability_condition`，说明该源图为什么不触发该目标。`pending`、`blocked`、`not_tested`、`ambiguous` 仍是未决状态；`fail` 仍是失败状态。图件修复或替换后重新查看受影响的最终文件，再绑定台账哈希。

结构校验能发现记录缺项和文件变化。即使所有条目被填写为 `pass`，它也不能证明模型实际完成了视觉查看。
