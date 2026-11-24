"""Deterministic, restartable formatting and mechanical acceptance.

Call process_markdown() or process_content() from an editor script. Originals
are read only; each input/engine version gets a separate output directory. A
matching restart verifies every existing artifact instead of overwriting it.
No function grants a translation or visual-source review result.
"""
from __future__ import annotations

from bisect import bisect_right
from copy import deepcopy
from hashlib import sha256
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
from threading import RLock
from typing import Callable


SCHEMA_VERSION = 1
PROGRESS = Callable[[int, int, str], None]
_MODULE_LOCK = RLock()
STAGES = {
    "input": "输入与版本绑定",
    "render": "内容单元与段落组装",
    "normalize": "量值、空行与块格式化",
    "mapping": "来源与交付位置更新",
    "accept": "结构、链接与资源验收",
    "save": "保存与回读",
}
CHECK_STAGE_LABELS = {
    "input": "输入读取", "render": "内容组装", "normalize": "格式规范化", "mapping": "来源与段落映射",
    "spacing": "空行格式化", "whitespace_formatting": "空行与字间空格",
    "math_formatting": "量值与数学格式", "punctuation_formatting": "可见字符规范化",
    "resources": "图片与附件", "citations": "书目结构", "internal_links": "引文及篇内链接组装",
    "headings": "标题结构", "metadata": "题头字段结构", "assembly": "文字与段落拼接",
    "source_coverage": "来源对象覆盖", "markdown_structure": "Markdown 结构",
    "figures": "图件结构", "tables": "表格结构", "translation_review": "翻译与来源语义核对",
}


class PipelineConflictError(ValueError):
    """An existing artifact does not match this deterministic run."""


def progress_bar(done: int, total: int, stage: str) -> None:
    width = 20
    filled = width * done // max(total, 1)
    print(f"[{'=' * filled}{'.' * (width - filled)}] {done}/{total} {stage}", flush=True)


def _module(name: str):
    path = Path(__file__).resolve().parent.joinpath(name + ".py")
    key = "literature_pipeline_" + name + "_" + sha256(str(path).encode()).hexdigest()[:12]
    with _MODULE_LOCK:
        if key in sys.modules:
            return sys.modules[key]
        spec = importlib.util.spec_from_file_location(key, path)
        if spec is None or spec.loader is None:
            raise ImportError(str(path))
        module = importlib.util.module_from_spec(spec)
        sys.modules[key] = module
        try:
            spec.loader.exec_module(module)
        except BaseException:
            sys.modules.pop(key, None)
            raise
        return module


def _bytes(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")


def _sha(value: bytes) -> str:
    return sha256(value).hexdigest()


def _file_hash(path: Path) -> str:
    digest = sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def read_json(path: Path) -> dict:
    """Read configuration without silently discarding duplicate JSON fields."""
    path = Path(path)
    def reject_constant(token):
        raise ValueError(f"Nonfinite JSON: {token}")

    value = json.loads(path.read_bytes().decode("utf-8"), object_pairs_hook=_unique_pairs,
                       parse_constant=reject_constant)
    if not isinstance(value, dict):
        raise ValueError(f"Expected a JSON object: {path}")
    return value


def _publish(path: Path, payload: bytes) -> None:
    """Publish a fully written file exclusively, or verify an identical retry.

    A temporary file and a same-directory hard link provide atomic publication
    without the overwrite behavior of rename/replace. A crash before linking
    cannot expose a truncated artifact. Unsupported filesystems fail explicitly.
    """
    if path.is_symlink():
        raise PipelineConflictError(f"Refusing a symlink artifact: {path}")
    if path.exists():
        if not path.is_file() or path.read_bytes() != payload:
            raise PipelineConflictError(f"Existing artifact differs; preserve it and use a new output root: {path}")
        return
    descriptor, temporary_name = tempfile.mkstemp(prefix=".pending-", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        try:
            os.link(temporary, path)
        except FileExistsError:
            if path.is_symlink() or not path.is_file() or path.read_bytes() != payload:
                raise PipelineConflictError(f"Concurrent artifact conflict: {path}")
        if path.read_bytes() != payload:
            raise PipelineConflictError(f"Artifact readback differs: {path}")
    finally:
        temporary.unlink(missing_ok=True)


def _contract_paths(contract: dict | None, base: Path) -> dict | None:
    result = deepcopy(contract)
    if not isinstance(result, dict) or not isinstance(result.get("sources"), dict):
        return result
    for value in result["sources"].values():
        if not isinstance(value, dict) or not isinstance(value.get("path"), str):
            continue
        path = Path(value["path"])
        value["path"] = str(path.resolve() if path.is_absolute() else base.joinpath(path).resolve())
    return result


def _version() -> dict:
    scripts = Path(__file__).resolve().parent
    return {"schema_version": SCHEMA_VERSION, "python": sys.version.split()[0],
            "files": {path.name: _file_hash(path) for path in sorted(scripts.glob("*.py"))}}


# File digests alone must not label already-loaded code as a new engine version.
# Run a new interpreter after changing a pipeline or one of its sibling modules.
_IMPORTED_ENGINE = _version()


def build_contract(plan: dict, sources: dict[str, Path]) -> dict:
    """Bind a reviewed source plan once, before requesting translation slots.

    Counts describe this explicit plan, not a claim that an arbitrary paper was
    exhaustively parsed. The caller supplies the source-derived plan; no content
    from a generated candidate is used to decide the expected object counts.
    """
    _module("render_note").content_template(plan)
    if not isinstance(sources, dict) or not sources:
        raise ValueError("Provide source IDs and their actual files")
    required_ids = {unit["source"]["source_id"] for unit in plan["units"]}
    if required_ids != set(sources):
        raise ValueError("Source files must match the reviewed plan's source IDs exactly")
    bindings = {}
    for identity, filename in sources.items():
        path = Path(filename).resolve(strict=True)
        bindings[identity] = {"path": str(path), "sha256": _file_hash(path)}
    units = [{key: deepcopy(unit[key]) for key in ("unit_id", "kind", "source", "scope", "anchor")
              if key in unit} for unit in plan["units"]]
    canonical = json.dumps(plan, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)
    counts = {"figures": 0, "images": 0, "equations": 0, "tables": 0, "bibliography": 0}
    for unit in plan["units"]:
        kind = unit["kind"]
        if kind == "figure":
            counts["figures"] += 1
            counts["images"] += len(unit["images"])
        elif kind == "formula":
            counts["equations"] += 1
        elif kind == "table":
            counts["tables"] += 1
        elif kind in {"reference", "bibliography"}:
            counts["bibliography"] += 1
    return {"schema_version": 1, "sources": bindings, "required_units": units, "counts": counts,
            "plan_sha256": _sha(canonical.encode("utf-8"))}


def prepare_content_job(plan: dict, sources: dict[str, Path], output_root: Path) -> dict:
    """Freeze a source plan/contract and a reusable blank translation template.

    The model writes content.json separately. Retrying preparation never erases
    a populated translation file. Input changes create a different job folder.
    """
    engine = _version()
    if engine != _IMPORTED_ENGINE:
        raise PipelineConflictError("Engine files changed after import; start a new Python interpreter")
    contract = build_contract(plan, sources)
    template = _module("render_note").content_template(plan)
    request = {"plan": plan, "contract": contract, "engine": engine}
    payload = _bytes(request)
    identity = _sha(payload)
    output_root = Path(output_root).resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    directory = output_root.joinpath(identity)
    if directory.is_symlink():
        raise PipelineConflictError(f"Refusing a symlink job directory: {directory}")
    directory.mkdir(exist_ok=True)
    files = {"request.json": payload, "plan.json": _bytes(plan), "contract.json": _bytes(contract),
             "content-template.json": _bytes(template)}
    for name, data in files.items():
        _publish(directory.joinpath(name), data)
    return {"job_id": identity, "directory": str(directory),
            "files": {name: {"path": str(directory.joinpath(name)), "sha256": _sha(data)}
                      for name, data in files.items()},
            "next_action": "Fill a new content.json using content-template.json; keep plan.json and contract.json unchanged.",
            "source_fidelity": "not_assessed"}


def replay_changes(original: str, changes: list[dict]) -> str:
    """Verify and replay sequential Unicode-offset edits, without fuzzy matching."""
    current = original
    for index, change in enumerate(changes):
        start, end = change.get("start"), change.get("end")
        if type(start) is not int or type(end) is not int or not 0 <= start <= end <= len(current):
            raise ValueError(f"Invalid edit offsets at change {index}")
        if current[start:end] != change.get("before") or not isinstance(change.get("after"), str):
            raise ValueError(f"Edit preimage differs at change {index}")
        current = current[:start] + change["after"] + current[end:]
    return current


def rebase_rendering(rendering: dict, normalized: dict) -> dict:
    """Move verified unit spans through the exact formatting edits.

    An edit that consumes only part of a unit boundary cannot establish a new
    mapping. It produces a located failure instead of relabeling stale spans.
    """
    original, updated = rendering["text"], normalized["text"]
    if replay_changes(original, normalized["changes"]) != updated:
        raise ValueError("Normalization edits do not reproduce the normalized text")
    result = deepcopy(rendering)
    units = result["mapping"]
    for row in units:
        start, end = row["start_offset"], row["end_offset"]
        if not 0 <= start <= end <= len(original) or _sha(original[start:end].encode()) != row["sha256"]:
            raise ValueError(f"Original rendering span differs: {row['unit_id']}")
    for change in normalized["changes"]:
        start, end = change["start"], change["end"]
        delta = len(change["after"]) - (end - start)
        for row in units:
            a, b = row["start_offset"], row["end_offset"]
            if end <= a:
                row["start_offset"], row["end_offset"] = a + delta, b + delta
                continue
            if start >= b:
                continue
            if a <= start <= end <= b:
                row["end_offset"] = b + delta
                continue
            raise ValueError(f"Formatting edit crosses a unit boundary: {row['unit_id']}")
    starts = [0] + [i + 1 for i, char in enumerate(updated) if char == "\n"]
    for row in units:
        a, b = row["start_offset"], row["end_offset"]
        if a not in starts or (b < len(updated) and updated[b] != "\n"):
            raise ValueError(f"Normalized unit is not line-aligned: {row['unit_id']}")
        row.update(start_line=bisect_right(starts, a), end_line=bisect_right(starts, max(a, b - 1)),
                   sha256=_sha(updated[a:b].encode("utf-8")))
    result.update(text=updated, output_sha256=_sha(updated.encode("utf-8")),
                  normalization_input_sha256=normalized["input_sha256"],
                  mapping_basis="original renderer spans rebased through verified sequential formatting edits")
    return result


def _markdown_report(result: dict, normalization: dict | None, acceptance: dict | None) -> str:
    lines = ["# Python 规范化与机械验收", "", f"运行标识：`{result['run_id']}`", "",
             f"程序状态：**{result['status']}**。来源语义核对：**not_assessed**。", "",
             "该状态描述本次程序实际执行的检查；译义正确、源图裁剪正确和来源清单完整性没有自动获得通过。", "",
             "## 阶段记录", "", "| 阶段 | 状态 |", "|---|---|"]
    lines.extend(f"| {row['label']} | {row['status']} |" for row in result["stages"])
    lines.extend(["", "## 定位信息", ""])
    for issue in result["issues"]:
        lines.append(f"- `{issue['code']}`：{issue['message']}")
    if not result["issues"]:
        lines.append("没有程序执行异常。具体检查与保留提示见下方。")
    if normalization is not None:
        lines.extend(["", "## 自动规范化", "", f"记录 {len(normalization['changes'])} 次变换；已逐项回放并验证二次运行字节一致。", "",
                      "原稿保存在 original.md，实际输出为 normalized.md；变换位置、原值、新值见 normalization.json。"])
        for issue in normalization.get("unresolved", []):
            lines.append(f"- `{issue.get('code', 'NORMALIZE_REVIEW')}`，行 {issue.get('line', '?')}：{issue.get('message', '')}")
    if acceptance is not None:
        lines.extend(["", "## 机械检查", "", f"机械规则：**{acceptance['mechanical_status']}**；来源合同覆盖：**{acceptance['source_coverage']}**。", "",
                      "详细 expected / actual、检查代码与位置保存在 acceptance.json。"])
        for row in acceptance.get("checks", []):
            if row.get("status") not in {"fail", "failed", "blocked"}:
                continue
            stage = CHECK_STAGE_LABELS.get(row.get("stage"), row.get("stage", "机械检查"))
            expected = json.dumps(row.get("expected"), ensure_ascii=False)
            actual = json.dumps(row.get("actual"), ensure_ascii=False)
            lines.append(f"- **{stage}** / `{row.get('code', 'CHECK')}` / `{row.get('location', '')}`：期望 {expected}；实际 {actual}。")
        lines.extend(["", f"另有 {len(acceptance.get('review_prompts', []))} 条启发式提示；未将提示直接计作来源语义失败。"])
    lines.extend(["", "## 结果边界", "", "缺少来源合同或单元映射时，未核实范围明确保留。翻译差异和有来源依据的译注不按逐字不一致判失败。",
                  "程序不会修改源 PDF、DOCX、已有原稿或原资源，也不会写入 Zotero / Obsidian。", ""])
    return "\n".join(lines)


def _phase_report(normalization: dict | None, acceptance: dict | None, events: list[dict]) -> dict:
    """Separate observed output defects from unobserved model-process causes."""
    automatic, findings, pending = [], [], []
    if normalization is not None:
        automatic = [{key: deepcopy(change[key]) for key in
                      ("stage", "rule", "line", "column", "start", "end", "before", "after") if key in change}
                     for change in normalization["changes"]]
        pending.extend({"detected_in": item.get("stage", "normalize"),
                        "cause_status": "not_established", **item}
                       for item in normalization.get("unresolved", []) if item.get("severity") != "info")
    if acceptance is not None:
        findings = [{"detected_in": item["stage"], "cause_status": "not_established", **item}
                    for item in acceptance["checks"] if item["status"] == "fail"]
        pending.extend({"detected_in": item["stage"], "cause_status": "not_established", **item}
                       for item in acceptance["review_prompts"])
    return {"schema_version": 1, "executed_stages": deepcopy(events), "automatic_repairs": automatic,
            "observed_failures": findings, "review_prompts": pending,
            "unobserved_model_processes": ["pdf_text_extraction", "docx_extraction", "source_segmentation",
                "title_recognition", "author_affiliation_recognition", "translation", "citation_identity_matching"],
            "boundary": "This local run observes rendering and file checks; an output defect alone does not prove which upstream model operation caused it."}


def _run(*, output_root: Path, source_path: Path | None, plan: dict | None, content: dict | None,
         contract: dict | None, resource_root: Path | None, progress: PROGRESS | None) -> dict:
    engine = _version()
    if engine != _IMPORTED_ENGINE:
        raise PipelineConflictError("Engine files changed after import; start a new Python interpreter to use the new version")
    events, issues = [], []
    callback = progress or (lambda *_: None)

    def event(stage: str, status: str):
        events.append({"stage": stage, "label": STAGES[stage], "status": status,
                       "observation": "direct_python_execution"})
        callback(len(events), len(STAGES), STAGES[stage] + "：" + status)

    original_bytes = None
    if source_path is not None:
        source_path = Path(source_path).resolve(strict=True)
        original_bytes = source_path.read_bytes()
    base = source_path.parent if source_path is not None else Path.cwd()
    contract = _contract_paths(contract, base)
    resource_root = Path(resource_root).resolve() if resource_root is not None else base
    request = {"schema_version": SCHEMA_VERSION, "mode": "markdown" if source_path is not None else "structured",
               "input": {"path": str(source_path), "sha256": _sha(original_bytes)} if source_path is not None else None,
               "plan": deepcopy(plan), "content": deepcopy(content), "contract": contract,
               "resource_root": str(resource_root), "engine": engine}
    request_bytes = _bytes(request)
    identity = _sha(request_bytes)
    output_root = Path(output_root).resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    run_dir = output_root.joinpath(identity)
    if run_dir.is_symlink():
        raise PipelineConflictError(f"Refusing a symlink run directory: {run_dir}")
    run_dir.mkdir(exist_ok=True)
    published = {}

    def publish(name, data):
        _publish(run_dir.joinpath(name), data)
        published[name] = _sha(data)

    publish("request.json", request_bytes)
    event("input", "verified")
    normalized = acceptance = rendering = None
    raw = None
    if source_path is not None:
        publish("original.md", original_bytes)
        try:
            raw = original_bytes.decode("utf-8")
        except UnicodeDecodeError as error:
            issues.append({"stage": "input", "code": "INPUT_ENCODING", "message": str(error)})
        event("render", "not_requested")
    else:
        rendering = _module("render_note").render_note(plan, content)
        publish("rendering-original.json", _bytes(rendering))
        raw = rendering.get("text")
        if raw is None:
            issues.extend({"stage": "render", **row} for row in rendering["findings"])
            event("render", "blocked")
        else:
            publish("original.md", raw.encode("utf-8"))
            event("render", "rendered")
    if raw is not None:
        normalized = _module("normalize_note").normalize_note(raw)
        replayed = replay_changes(raw, normalized["changes"])
        second = _module("normalize_note").normalize_note(normalized["text"])
        if replayed != normalized["text"] or second["text"] != normalized["text"] or second["changes"]:
            raise ValueError("Normalization failed replay or idempotence verification; no final note was published")
        publish("normalization.json", _bytes(normalized))
        publish("normalized.md", normalized["text"].encode("utf-8"))
        event("normalize", "verified")
    else:
        event("normalize", "blocked")
    if rendering is not None and raw is not None:
        try:
            rendering = rebase_rendering(rendering, normalized)
        except ValueError as error:
            issues.append({"stage": "mapping", "code": "MAPPING_REBASE", "message": str(error)})
            rendering = None
            event("mapping", "blocked")
        else:
            publish("rendering.json", _bytes(rendering))
            event("mapping", "verified")
    else:
        event("mapping", "not_available")
    if normalized is not None:
        acceptance = _module("accept_note").accept_note(run_dir.joinpath("normalized.md"), contract,
                                                       resource_root=resource_root, rendering=rendering)
        publish("acceptance.json", _bytes(acceptance))
        event("accept", acceptance["mechanical_status"])
    else:
        event("accept", "blocked")
    status = "blocked" if normalized is None else "needs_attention"
    unsafe_parse = normalized is not None and any(
        item.get("code", "").startswith("UNCLOSED_") or item.get("code") == "INTERACTING_RULES"
        for item in normalized.get("unresolved", []))
    if (acceptance is not None and acceptance["mechanical_status"] == "pass" and not issues
            and not unsafe_parse and acceptance["source_coverage"] == "verified_against_contract"):
        status = "mechanical_pass"
    event("save", "committing")
    if acceptance is not None:
        final_check = _module("accept_note").accept_note(run_dir.joinpath("normalized.md"), contract,
                                                         resource_root=resource_root, rendering=rendering)
        if final_check != acceptance:
            raise PipelineConflictError("Source, resource, candidate or evidence changed after acceptance; no final receipt was published")
    if source_path is not None and _file_hash(source_path) != request["input"]["sha256"]:
        raise PipelineConflictError("Original changed during processing; retain evidence and rerun the new version")
    if _version() != engine:
        raise PipelineConflictError("Engine files changed during processing; no final receipt was published")
    for name, expected in published.items():
        path = run_dir.joinpath(name)
        if path.is_symlink() or not path.is_file() or _file_hash(path) != expected:
            raise PipelineConflictError(f"Published artifact changed during processing: {path}")
    events[-1]["status"] = "verified"
    publish("phases.json", _bytes(_phase_report(normalized, acceptance, events)))
    result = {"schema_version": SCHEMA_VERSION, "run_id": identity, "directory": str(run_dir),
              "status": status, "source_fidelity": "not_assessed", "stages": events, "issues": issues,
              "outputs": {path.name: {"path": str(path), "sha256": _file_hash(path)}
                          for path in sorted(run_dir.iterdir())
                          if path.is_file() and path.name in {"request.json", "original.md", "normalized.md",
                              "normalization.json", "rendering-original.json", "rendering.json", "acceptance.json", "phases.json"}}}
    publish("report.md", _markdown_report(result, normalized, acceptance).encode("utf-8"))
    result["outputs"]["report.md"] = {"path": str(run_dir.joinpath("report.md")),
                                     "sha256": _file_hash(run_dir.joinpath("report.md"))}
    publish("result.json", _bytes(result))
    return result


def process_markdown(source_path: Path, output_root: Path, *, contract: dict | None = None,
                     resource_root: Path | None = None, progress: PROGRESS | None = progress_bar) -> dict:
    """Normalize an existing candidate and verify the explicit mechanical scope."""
    return _run(output_root=output_root, source_path=source_path, plan=None, content=None,
                contract=contract, resource_root=resource_root, progress=progress)


def process_content(plan: dict, content: dict, output_root: Path, *, contract: dict | None = None,
                    resource_root: Path | None = None, progress: PROGRESS | None = progress_bar) -> dict:
    """Render checked plan data plus translation slots, then normalize and verify."""
    return _run(output_root=output_root, source_path=None, plan=plan, content=content,
                contract=contract, resource_root=resource_root, progress=progress)
