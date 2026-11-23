# 完整候选的开始与收尾

从 PDF 新建整篇笔记时，使用 `complete_workflow.py` 的完整入口。局部修复仍使用对应子技能。单独的 `validate_note` 或 `validate_delivery` 没有检查整套工作记录是否齐全，不能替代这里的收尾检查。

## 开始：先生成清单

在本次工作目录保存 Python 调用脚本，先填写实际路径。以下文件名是示例，不表示实际来源已经读取。

```python
from pathlib import Path
import sys

TASK_ROOT = Path("/本次任务的绝对路径")
SKILLS_ROOT = Path("/实际技能根目录")
WORK_ROOT = TASK_ROOT.joinpath("work")
OUTPUT_ROOT = TASK_ROOT.joinpath("output")
SCRIPTS = SKILLS_ROOT.joinpath("literature-zotero2obsidian", "scripts")
sys.path.insert(0, str(SCRIPTS))

from complete_workflow import initialize_workflow
from prepare_source_inventory import progress_bar

RUN_PATH = initialize_workflow(
    skills_root=SKILLS_ROOT,
    work_root=WORK_ROOT,
    sources={
        "main": TASK_ROOT.joinpath("input", "article.pdf"),
        "supplement": TASK_ROOT.joinpath("input", "supplement.pdf"),
    },
    source_roles={"main": "main", "supplement": "supplement"},
    candidate_path=OUTPUT_ROOT.joinpath("candidate.md"),
    resource_root=OUTPUT_ROOT,
    glossary_path=SKILLS_ROOT.joinpath("template", "术语表.md"),
    translation_required=True,
    progress=progress_bar,
)
```

没有 SI 时从两个映射中同时去掉对应来源。中文原文确实无需翻译时，将 `translation_required` 设为 `False`，并在语言分流记录中保存依据。初始化会保存逐页材料、来源哈希、全量验收骨架及 `run.json`；它不会把任何内容判断填成通过。恢复任务时读取已有 `run.json`，不要重新初始化或覆盖旧记录。

## 中间阶段：按对象保存实际判断

沿[分阶段路线](execution-plan.md)完成来源盘点、翻译、排版和链接。`run.json.records` 给出本次必需记录的计划路径；尚未生成的文件应在对应阶段补齐。

| 记录 | 在哪个阶段完成 | 核对目标 |
|---|---|---|
| `run.json.naming` | 来源题头核定后 | 拟用文件名有明确来源与变换依据 |
| `source-map.json` | 主文提取后 | 主文每个块能回到来源 |
| `si-map.json` | SI 提取后 | SI 每个对象能回到独立来源 |
| `link-map.json` | 链接组装后 | 每处调用能回到实际目标 |
| `translation-review.json` | 逐段自核后 | 每个译文源块有独立核对记录 |
| `checks.json` | 各阶段持续填写 | 每个验收 ID 有自己的判断 |

记录的字段沿用[交接约定](pipeline-contract.md)。同一个事实可以存一次，再由多个明确 ID 引用其精确位置；不能把整个技能视作一个已通过目标。译文复核使用[翻译核对说明](../../literature-translate-zh/references/translation-review.md)。

完整入口读取以下集合字段；集合可以用列表保存记录，也可以用对象 ID 到记录的映射保存。没有对应对象时保留空集合，并在该验收 ID 记录实际不适用依据：`source-map.json` 的 `blocks`、`objects`、`references`、`citations`；`si-map.json` 的 `sources`、`blocks`、`objects`；`link-map.json` 的 `anchors`、`internal_links`、`library_matches`、`unresolved`。集合存在只说明结构齐全，不证明内容已经核对。

来源单元和交付位置登记后，可调用 `write_maps(RUN_PATH, deliveries)`，由实际 inventory 和明确的交付定位生成这三份映射。返回报告会列出不能可靠派生的字段；补齐实际来源依据后再生成，不能填猜测值绕过诊断。该函数保留 `extracted`／`pending` 状态，既不声称已经看过来源，也不替代逐单项目标的实际核对。首次生成不覆盖旧文件；需要重新生成时在 `run.json.records` 指定新版本路径，保留原文件。

三份映射的 `bindings` 分别绑定当前 `source_inventory`、`candidate` 及按来源 ID 登记的 `sources`。生成以后检查每个对象的源位置、目标位置和对应关系，再把实际比较结果写入相应对象记录及 `checks.json`。当前稿与旧稿的行号相同不代表证据仍然有效。

最终文件即使被用户指定为 `candidate.md`，也在 `run.json.naming` 单独保存[离线命名计划](../../literature-filename/SKILL.md)。指定交付路径不会自动产生文献的拟用名称，也不能据此宣称真实库冲突已经检查。

### 图件输出的调用顺序

先从完整源页核定图件范围。只有已确定整幅图就是该原始图片对象时，才走直接导出分支。复合图直接使用页面范围渲染。以下变量均取实际图件清单，不从示例猜页码或对象号。

```python
from pdf_image_objects import (
    PDF_BBOX_COORDINATES, PageRenderRequired,
    export_image_object, render_page_region,
)

if IS_ONE_COMPLETE_IMAGE_OBJECT:
    try:
        figure_record = export_image_object(
            SOURCE_PDF, page_number=PAGE_NUMBER, image_xref=IMAGE_XREF,
            destination=DESTINATION_WITHOUT_SUFFIX,
            expected_source_sha256=SOURCE_SHA256,
        )
    except PageRenderRequired as error:
        render_reason = str(error)
        figure_record = render_page_region(
            SOURCE_PDF, page_number=PAGE_NUMBER, bbox=VERIFIED_FIGURE_BBOX,
            coordinate_system=PDF_BBOX_COORDINATES,
            destination=DESTINATION_WITHOUT_SUFFIX,
            margin_pt=6.0, expected_source_sha256=SOURCE_SHA256,
        )
        figure_record["render_reason"] = render_reason
else:
    figure_record = render_page_region(
        SOURCE_PDF, page_number=PAGE_NUMBER, bbox=VERIFIED_FIGURE_BBOX,
        coordinate_system=PDF_BBOX_COORDINATES,
        destination=DESTINATION_WITHOUT_SUFFIX,
        margin_pt=6.0, expected_source_sha256=SOURCE_SHA256,
    )
```

将返回记录保存到工作目录。渲染记录包含每边实际安全边距；页边限制导致的边距不足可从记录定位。随后按[图件规则](../../literature-fulltext-extract/references/figure-crop.md)分别核对四边。`pending` 记录不会自动转为通过。直接调用 `get_pixmap(clip=bbox)` 只保留原框，不会自动加入安全边距。

## 最终候选：先固定版本，再核对

候选内容完成后，调用 `bind_candidate(RUN_PATH)`。它把当前实际候选的路径与哈希写入验收骨架，不填写结果。随后对这一版本逐项核对。候选又有变化时，保留旧记录并重新核验受影响目标；该函数拒绝静默刷新已有候选绑定。

`pending_check_batch(CHECKS_PATH, limit=20)` 列出下一批尚未完成的文件目标。每条带原规则文字和文件位置，直接读取对应规则、查实际对象、保存证据并填写该 ID。完成当前批次后再取下一批。批次大小只影响展示，不改变目标集合。

`record_check_batch(CHECKS_PATH, results, evidence=EVIDENCE)` 接收明确的逐 ID 结果。`results` 中每条独立给出 `id`、`status`、`scope`、`evidence`、`reason`；不适用项另给实际 `applicability_condition`。共享证据通过 `check_records.bind_evidence()` 绑定已存在文件。函数不接受整技能通过、默认通过、按关键词推断不适用或改变目标类型。填写格式见[单项记录](acceptance.md#单项记录)。

首次最终绑定前先完成会改动 inventory 的指针登记。翻译复核与图件复核按各自说明绑定稳定版本，避免相互引用仍在变化的文件造成哈希循环。使用 `write_manifest(RUN_PATH, deliveries)` 生成交付索引；它复用同一运行的实际路径，并绑定已完成的翻译复核文件。

## 收尾：按缺项继续，不在零语法错误处结束

把以下代码放入本次调用脚本。报告写在工作目录，不写入文献正文。

```python
import json
from complete_workflow import inspect_workflow, pending_check_batch

report = inspect_workflow(RUN_PATH, progress=progress_bar)
WORK_ROOT.joinpath("completion-report.json").write_text(
    json.dumps(report.as_dict(), ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
print({"records_ready": report.records_ready, "finding_count": len(report.findings)})
for finding in report.findings[:20]:
    print(finding.code, finding.target, finding.message)
print(json.dumps(pending_check_batch(WORK_ROOT.joinpath("checks.json")),
                 ensure_ascii=False, indent=2))
```

`WORKFLOW_RECORD_MISSING` 表示需要补对应记录。`WORKFLOW_CHECK_PENDING` 表示需要完成该单项目标。`WORKFLOW_MANIFEST_BINDING` 表示报告引用了另一份文件或旧版本。翻译记录中的 `TRN_PENDING`、`TRN_UNRESOLVED` 等逐项提示同样保留其具体目标位置，可据此回到对应段落或术语记录。先处理实际缺项，再运行收尾检查。

排版提示仍出现时，回到原规则判定。带单位的近似时长同样适用数学排版，例如“数十年”可写为 `$\text{数十}\,\text{年}$`；非精确不构成忽略提示的理由。真正的非问题项保存为 `presentation_review` 指向的 JSON。该文件的 `bindings.candidate` 用 `bind_file(CANDIDATE_PATH)` 绑定当前实际文件；`items` 每项写当前 `code`、`line`、`status: "not_an_issue"` 和具体 `reason`。候选变化后重新对照提示，不复用旧行号或旧理由。

`records_ready` 只表示当前检查所需记录与源码条件已满足。`source_fidelity` 始终为 `not_assessed`，程序不认证译义、原文身份、图件完整性或记录者声明的真实性。执行者完成自己的实际对源核对后，才据各项证据报告文件结果。独立评审是复核，不承担补做执行者尚能完成的文件目标。
