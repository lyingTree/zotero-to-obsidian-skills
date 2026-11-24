# 从来源逐块写出完整候选

## 目录

- [先产出当前内容](#先产出当前内容)
- [最短脚本入口](#最短脚本入口)
- [按当前任务继续](#按当前任务继续)
- [分组与多媒体](#分组与多媒体)
- [合稿与后续核对](#合稿与后续核对)
- [文件版本与边界](#文件版本与边界)

## 先产出当前内容

完整任务在来源初始化后使用 `draft_queue.py`。它回答“下一块还没有写出的内容是什么”。`next_review_task()` 则用于已有译稿的语义复核，不负责从零生成正文。

逐页读取来源，按实际阅读顺序分组，每写完一个语义单元便保存其 Markdown 片段，再继续下一单元。一个单元可以跨文字块、跨页；不要把排版碎片当成独立段落翻译。保留来源中的引文、数值、条件和限定语，不用摘要代替未处理段落。来源支持的本地内容尚未完成时，保存进度后继续，不以一份“部分稿执行报告”替代剩余产出。

初始化得到的原始 PDF 文字块、图片调用和 DOCX 原生块全部进入队列。未分配的源块不会因为只登记了摘要、结论而消失。重复页眉等可以逐项排除，但必须记录该源块自己的依据。队列不自动排除文字或图片，不自动填验收结论。

## 最短脚本入口

将下面内容保存为本次工作目录里的 Python 脚本。只需填写现有 `run.json` 的实际绝对路径。再次运行时恢复已有队列，不重复初始化。

```python
from pathlib import Path
import json
import sys

RUN_PATH = Path("本次已有 run.json 的实际绝对路径")
run = json.loads(RUN_PATH.read_text(encoding="utf-8"))
work = Path(run["config"]["work_root"])
scripts = Path(run["config"]["skills_root"]).joinpath(
    "literature-zotero2obsidian", "scripts")
sys.path.insert(0, str(scripts))

from draft_queue import DraftQueue, prepare_draft_queue

queue_path = work.joinpath("draft-queue.json")
if not queue_path.exists():
    prepare_draft_queue(Path(run["records"]["source_inventory"]), queue_path)

queue = DraftQueue.open(queue_path)
task = queue.next_task()
print(json.dumps(task, ensure_ascii=False, indent=2))
```

该脚本显示当前真实来源及下一步，不生成译文。每完成下面的一次实际动作，重新读取 `next_task()`。不要循环给每个块填相同文本、同一个通过状态或通用排除理由。

## 按当前任务继续

| `stage` | 本次要完成的动作 |
|---|---|
| `inventory_page` | 查看此 PDF 页的实际页图，登记文字层之外的对象，确认此页阅读顺序。 |
| `group_or_exclude` | 把当前源块与属于同一语义单元的其他块分组；确为重复版面内容时逐块排除。 |
| `write_unit` | 读取本单元的全部来源，写出完整译文或对象源码并立即保存。 |
| `source_issue` | 按原生解析器报告的具体问题核实来源；未解决的阻断保留。 |
| `assemble` | 全部登记内容已有明确去向，可以组装一个新的完整候选版本。 |
| `no_draft_units` | 当前没有任何已登记的可写单元，不能把空稿称为完成稿。 |

### PDF 页的阅读顺序

`task["page"]` 提供页图、原始文字和图片调用索引；`task["entries"]` 保留此页的全部源条目。文字提取顺序可能串栏，所以工具不会把坐标排序或 PDF 内部顺序当成已经核定的阅读顺序。实际查看后，按页面阅读顺序传入每个条目 ID，恰好一次：

```python
queue.review_pdf_page_order(
    actual_source_id, actual_page_number,
    actual_ordered_entry_ids,
    reason=actual_reading_order_observation,
)
```

图片调用也在这个顺序里。同一幅图可以有多个调用，它们不是多个逻辑图号。PDF 纯图页和没有文字块的页仍会出现在 `inventory_page`，不能因文字层为空而跳过。

查看页面中未被文字块和图片调用覆盖的内容。发现向量图、独立公式等时先增加真实对象，再重新确认包含它的阅读顺序：

```python
queue.add_page_object(
    actual_entry_id, actual_source_id, actual_page_number,
    kind=actual_object_kind,
    reason=actual_visible_object_observation,
    bbox=actual_pdf_point_bbox,
    text=actual_visible_transcription,
)
```

新增对象会使该页的旧顺序与对象盘点记录失效，要求重新确认。然后单独保存文字层之外对象的盘点观察：

提供的裁框必须是有限数值，且完整位于实际 PDF 页内。工具优先使用冻结来源库存中的真实页矩形；库存没有页矩形时读取已绑定的 PDF 页面，不接受 NaN、无穷值或页外坐标。

```python
queue.review_additional_objects(
    actual_source_id, actual_page_number,
    reason=actual_nontext_object_inventory_observation,
)
```

这两类记录分别保存顺序判断与额外对象盘点，不把一次声明当成已完成图片四边、公式含义等后续验收。

### 当前内容的分组与落盘

一个语义段落跨两个 PDF 文字块或两页时，把它们明确分到同一单元，源块 ID 按本段原文顺序排列：

```python
queue.group(
    actual_unit_id,
    actual_source_entry_ids,
    kind="paragraph",
    reason=actual_paragraph_boundary_observation,
)
queue.write_unit(actual_unit_id, actual_complete_markdown)
print(json.dumps(queue.next_task(), ensure_ascii=False, indent=2))
```

同一源块不能被两个单元重复使用，不能同时分组和排除，不把主文与补充文件的源块混成一个单元。题名、正文、题注、书目、表格和公式根据它们各自的内容保存。生成表格时使用 `format_helpers` 中的实际 Rich Table 序列化入口；图像使用实际提取资源；公式按可见源文转录。

图件可保存多份真实媒体绑定：

```python
queue.write_unit(
    actual_figure_unit_id,
    actual_figure_markdown_with_caption,
    assets=actual_final_image_paths,
)
```

`assets` 可以有多件，不要求一幅图等于一个文件。片段里的路径仍须在最终资源根中可解析；队列只核对所传文件的实际哈希，不替代嵌入资源检查。

确认某个块只是重复页眉等版面内容后，保存具体依据：

```python
queue.exclude(actual_repeated_entry_id, reason=actual_source_based_exclusion_reason)
```

### 原生解析问题

DOCX 的 `native_issues` 保留为独立问题。问题未解决时可以继续处理其他来源单元，但最终合稿仍列出该问题。解决、排除和阻断分别记录；证据使用实际存在的文件：

```python
queue.record_issue_decision(
    actual_issue_id,
    status=actual_status,  # resolved、excluded 或 blocked
    reason=actual_issue_observation,
    evidence_paths=actual_evidence_file_paths,
)
```

不得仅为清空队列把问题写成 `resolved`。`blocked` 仍属于未完成。记录存在也不证明该判断正确，仍需后续来源核验。

## 分组与多媒体

PDF 源条目保留物理页码、原始文字块、坐标及实际图片调用。新增可见对象须指向真实源页。自动提取没有发现的对象仍由逐页盘点补入。

DOCX 源条目严格保留原始 `native_blocks` 的 XML 顺序和 `native_locator`，不伪造页码。图片所在的空段落也进入覆盖全集；其 `media_calls` 可以指向多个媒体。题注所属关系依据段落、媒体调用及可见文档核定，不能根据 `image1.png` 等文件名推断图号。一个图单元可以组合图片段落、多个媒体调用及题注段落。

源块分组只决定稿件组织。它不创建已经通过的语义单元，也不代替 `source-map.json`、`si-map.json` 和完整的翻译复核记录。后续登记语义单元时复用这些真实来源定位。

## 合稿与后续核对

`queue.status()` 分别返回未分配源条目、未写单元、未盘点 PDF 页和未解决的原生问题。任何一类仍非空时，`assemble()` 拒绝把当前部分稿组装为完整草稿。

全部当前产出任务处理后，使用一个尚不存在的版本路径：

```python
draft_path = work.joinpath("drafts", "assembled-v001.md")
report = queue.assemble(draft_path)
print(json.dumps({
    "candidate": report["candidate"],
    "draft_ready": report["draft_ready"],
    "source_fidelity": report["source_fidelity"],
}, ensure_ascii=False, indent=2))
```

合稿按已确认的来源顺序拼接，每个分组单元放在它的首个来源条目处。跨页段落的所有来源块仍在映射中。内部 Markdown、数学字符串及内部换行保持原样；片段边界统一为一个空行。相邻产生特殊语法结构的片段，应在分组阶段合成一个完整单元。

同层的 `assembled-v001.md.draft-map.json` 保存每个片段的候选行范围、源块 ID、原片段哈希与资源绑定。这里的 `draft_ready` 只表示登记内容已经落盘。下一步继续题头、正文格式、链接、逐段语义核对和全部适用验收；`source_fidelity` 保持 `not_assessed`。

## 文件版本与边界

准备队列时复制当前来源清单的原始字节，保存为不可覆盖的 `draft-queue-source-snapshot.json`。后续在工作清单中增加语义单元、排除记录或链接字段，不会使这个冻结快照失效。原始 PDF/DOCX 文件哈希仍受检查。

每次 `write_unit()` 新建片段版本，保留旧字节；修订后合稿使用新版本。来源文件、快照、最终使用的片段和声明资源在合稿时重新核对哈希。输出已存在时使用新的版本路径，不覆盖旧候选。

一份调用脚本内复用同一个 `DraftQueue` 对象，避免每个单元重复读取大文件。跨脚本恢复时重新打开队列；并发脚本发现队列已变化会拒绝覆盖。不要直接改队列 JSON 来删除来源条目或清空未决项。

队列不能判断译文是否忠实、排除理由是否成立、图件是否完整或源内容是否漏提。它阻止可见的内容跳过并提供持续落盘入口，不授予任何验收 ID 通过。
