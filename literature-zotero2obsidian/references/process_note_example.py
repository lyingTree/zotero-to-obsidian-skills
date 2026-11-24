"""在编辑器设置路径与MODE后运行；不接收命令行参数。

prepare: 从已核对的plan生成独立任务目录，保存合同和空译文模板。
structured: 使用冻结计划/合同和填好的content生成笔记。
markdown: 对既有Markdown做同一套确定性规范化和机械检查。
"""
from __future__ import annotations

import importlib.util
from pathlib import Path


WORK_ROOT = Path("/your/work/directory")
RESOURCE_ROOT = WORK_ROOT.joinpath("resources")
MODE = "structured"
PLAN_PATH = WORK_ROOT.joinpath("plan.json")
CONTENT_PATH = WORK_ROOT.joinpath("content.json")
CONTRACT_PATH = WORK_ROOT.joinpath("contract.json")
CANDIDATE_PATH = WORK_ROOT.joinpath("candidate.md")
SOURCE_FILES = {"main": WORK_ROOT.joinpath("main.pdf"),
                "si": WORK_ROOT.joinpath("supplement.docx")}


def load_pipeline():
    path = Path(__file__).resolve().parents[1].joinpath("scripts", "note_pipeline.py")
    spec = importlib.util.spec_from_file_location("literature_example_pipeline", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    pipeline = load_pipeline()
    if MODE == "prepare":
        plan = pipeline.read_json(PLAN_PATH)
        job = pipeline.prepare_content_job(plan, SOURCE_FILES, WORK_ROOT.joinpath("prepared"))
        print(f"已准备：{job['directory']}\n将此目录中的模板另存为content.json，填译文；把顶部三个JSON路径指向该任务。")
        return
    contract = pipeline.read_json(CONTRACT_PATH)
    if MODE == "structured":
        result = pipeline.process_content(pipeline.read_json(PLAN_PATH), pipeline.read_json(CONTENT_PATH),
                                          WORK_ROOT.joinpath("runs"), contract=contract,
                                          resource_root=RESOURCE_ROOT)
    elif MODE == "markdown":
        result = pipeline.process_markdown(CANDIDATE_PATH, WORK_ROOT.joinpath("runs"),
                                           contract=contract, resource_root=RESOURCE_ROOT)
    else:
        raise ValueError("MODE 只能是 prepare、structured 或 markdown")
    print(f"程序状态：{result['status']}；报告：{Path(result['directory']).joinpath('report.md')}")


if __name__ == "__main__":
    main()
