"""Prepare isolated, reproducible model trials; this script does not call a model.

Edit the configuration at the bottom, then run this file in a Python editor.
Keep source articles, expected answers, and complete outputs outside the repo.
The host must launch fresh agents and record their real launch receipts.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from hashlib import sha256
from io import BytesIO
from pathlib import Path
import json
import random
import shutil
import subprocess
import tarfile


MODELS = ("gpt-6-luna", "gpt-6.1-terra", "gpt-6.2-sol")
EFFORTS = ("low", "medium", "high")

TASK = """请使用本执行目录 skills/literature-zotero2obsidian/SKILL.md 及其引用的子技能，
将 input/main.pdf（主文）和 input/supplement.docx（补充材料）整理为完整的
Obsidian 中文文献笔记候选，附所需图件和技能要求的处理记录。
本次只生成离线候选，不向真实 Zotero 或 Obsidian 写入。
没有提供真实文献库访问；库身份、集合、Zotero URI 无法核实时如实记录，
继续完成能够从附件完成的全文内容、图表、公式、参考文献与链接。
模板与术语表位于 skills/template/；离线候选的资源根是 output/。
译文允许合理措辞差异与有来源依据的译注，保留原文信息和含义。
只使用本执行目录内的原始材料与技能，不读取其他运行目录、验收样例、
历史产物、评审材料或当前对话，不使用联网检索，不创建子代理。
可以使用宿主的文件处理工具与依赖。技能和输入只读，工作稿写入 work/。
Python 使用可直接运行的脚本、pathlib 及 joinpath；重度循环报告进度；
使用卫语句并避免不必要复制。不以命令行参数作为给用户的脚本接口。
请实际完成整篇处理，不只写方案。文件层验收不涉及应用界面。
候选保存为 output/candidate.md，执行状态保存为 output/execution-report.md。
"""


@dataclass(frozen=True)
class MatrixConfig:
    repository: Path
    revision: str
    main_pdf: Path
    supplement_docx: Path
    expected_markdown: Path
    destination: Path
    seed: int = 20261124


def digest(data: bytes) -> str:
    return sha256(data).hexdigest()


def bind_file(path: Path) -> dict[str, object]:
    path = path.resolve(strict=True)
    return {"path": str(path), "size": path.stat().st_size,
            "sha256": digest(path.read_bytes())}


def frozen_files(repository: Path, revision: str) -> tuple[str, dict[str, bytes]]:
    commit = subprocess.run(
        ["git", "rev-parse", "--verify", f"{revision}^{{commit}}"],
        cwd=repository, check=True, capture_output=True, text=True,
    ).stdout.strip()
    archive = subprocess.run(
        ["git", "archive", "--format=tar", commit], cwd=repository,
        check=True, capture_output=True,
    ).stdout
    files: dict[str, bytes] = {}
    with tarfile.open(fileobj=BytesIO(archive), mode="r:") as stream:
        for member in stream.getmembers():
            path = Path(member.name)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError(f"Unsafe archive entry: {member.name}")
            root = path.parts[0]
            if not (root.startswith("literature-") or root == "template"):
                continue
            if not member.isfile():
                if member.isdir():
                    continue
                raise ValueError(f"Non-regular skill file: {member.name}")
            source = stream.extractfile(member)
            if source is None:
                raise ValueError(f"Unreadable archive entry: {member.name}")
            files[path.as_posix()] = source.read()
    if "literature-zotero2obsidian/SKILL.md" not in files:
        raise ValueError("Frozen commit does not contain the entry skill")
    return commit, files


def write_json(path: Path, data: object) -> None:
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n",
                    encoding="utf-8", newline="\n")


def prepare_matrix(config: MatrixConfig) -> Path:
    repository = config.repository.resolve(strict=True)
    destination = config.destination.resolve()
    if destination.is_relative_to(repository):
        raise ValueError("Keep private inputs and model outputs outside the repository")
    if destination.exists():
        raise FileExistsError(f"Keep previous trials: {destination}")
    inputs = {"main.pdf": bind_file(config.main_pdf),
              "supplement.docx": bind_file(config.supplement_docx)}
    reference = bind_file(config.expected_markdown)
    commit, files = frozen_files(repository, config.revision)
    file_hashes = {name: digest(data) for name, data in sorted(files.items())}
    package_hash = digest(json.dumps(file_hashes, sort_keys=True).encode())
    configurations = [(model, effort) for model in MODELS for effort in EFFORTS]
    random.Random(config.seed).shuffle(configurations)
    destination.mkdir(parents=True)
    reviewer_root = destination.joinpath("reviewer")
    reviewer_root.mkdir()
    manifest: dict[str, object] = {
        "schema_version": 1, "created_at": datetime.now(timezone.utc).isoformat(),
        "skill_commit": commit, "skill_package_sha256": package_hash,
        "skill_files": file_hashes, "input_files": inputs,
        "reviewer_only_reference": reference, "task_sha256": digest(TASK.encode()),
        "seed": config.seed, "runs": [],
        "isolation": "Separate contexts and directories with explicit read restrictions; shared host, not an OS sandbox.",
        "launch_status": "prepared_only; no model invocation is performed by this script",
    }
    for index, (model, effort) in enumerate(configurations, 1):
        run_id = f"r{index:02d}"
        run_root = destination.joinpath("trials", run_id)
        for directory in ("input", "skills", "output", "work"):
            run_root.joinpath(directory).mkdir(parents=True)
        for name, data in files.items():
            target = run_root.joinpath("skills", name)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        for name, record in inputs.items():
            shutil.copyfile(record["path"], run_root.joinpath("input", name))
        run_root.joinpath("task.txt").write_text(TASK, encoding="utf-8", newline="\n")
        manifest["runs"].append({
            "run_id": run_id, "root": str(run_root), "model": model,
            "reasoning_effort": effort, "status": "not_started",
        })
        print(f"[{index}/{len(configurations)}] Prepared {run_id}", flush=True)
    manifest_path = reviewer_root.joinpath("matrix-manifest.json")
    write_json(manifest_path, manifest)
    return manifest_path


def inspect_run(run_root: Path, manifest_path: Path) -> dict[str, object]:
    """Record integrity and all artifacts, without assigning content quality."""
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    changes: list[str] = []
    for relative, expected in manifest["skill_files"].items():
        path = run_root.joinpath("skills", relative)
        if not path.is_file() or digest(path.read_bytes()) != expected:
            changes.append(f"skills/{relative}")
    for relative, record in manifest["input_files"].items():
        path = run_root.joinpath("input", relative)
        if not path.is_file() or digest(path.read_bytes()) != record["sha256"]:
            changes.append(f"input/{relative}")
    for folder, expected in (("skills", manifest["skill_files"]),
                             ("input", manifest["input_files"])):
        base = run_root.joinpath(folder)
        for path in base.rglob("*"):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            relative = path.relative_to(base).as_posix()
            if relative not in expected:
                changes.append(f"{folder}/{relative} (added)")
    task_path = run_root.joinpath("task.txt")
    if not task_path.is_file() or digest(task_path.read_bytes()) != manifest["task_sha256"]:
        changes.append("task.txt")
    artifacts = []
    for folder in ("output", "work"):
        for path in sorted(run_root.joinpath(folder).rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            artifacts.append({"relative": path.relative_to(run_root).as_posix(),
                              **bind_file(path)})
    return {"run_id": run_root.name, "protected_file_changes": changes,
            "candidate_exists": run_root.joinpath("output", "candidate.md").is_file(),
            "artifacts": artifacts,
            "quality_status": "not_assessed_by_integrity_check"}


if __name__ == "__main__":
    # Set actual local paths before running. Raw sources and the reference are private.
    CONFIG = MatrixConfig(
        repository=Path(__file__).resolve().parents[1],
        revision="HEAD",
        main_pdf=Path("/replace-with-private-input/main.pdf"),
        supplement_docx=Path("/replace-with-private-input/supplement.docx"),
        expected_markdown=Path("/replace-with-private-input/expected.md"),
        destination=Path("/replace-with-private-output/model-matrix"),
    )
    print(prepare_matrix(CONFIG))
