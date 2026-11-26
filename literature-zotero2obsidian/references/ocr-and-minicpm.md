# OCR、表格/公式识别与 MiniCPM / Jev 运行指南

本文记录 `z2o` Rust 运行时已注册的 15 个 OCR、表格/公式识别、OCR 质量审计与 MiniCPM / Jev 推理操作、Schema 1 清单绑定、离线安装与 GPU/CPU 回退策略及证据边界。所有操作统一通过 `z2o invoke request.json` 执行，先用 `z2o describe` 确认注册状态。

## 证据边界与能力状态

## 首次运行的选择

复杂公式与表格有两条路径，由技能内 `user/profile.json` 的 `preferences.formula_table_enhancement` 记录：

- **选项 A：直接执行**。使用 PDF 原生文字层、PdfCraft 页图/区域渲染、模型视觉核对，以及可用的 arXiv TeX 源。不要求下载模型，不因缺少 OCR 包阻断。
- **选项 B：可选 OCR 增强**。首次运行且记录为 `unasked` 时询问一次是否下载安装 `ocr-cpu-assets.json` 绑定的公式/表格资源。用户同意后调用 `install_ocr_package`；拒绝或失败保持选项 A，后续不重复询问。

已记录为 `native` 或 `ocr` 后直接沿用。安装位置仍须位于当前技能内的持久化 `store`，不得写入通用用户目录。

- `ocr_backend`（`ppocr-rs`，pinned revision `b87550b775206655acf84d7fc421ca5bdf745fd3`）在 `z2o describe` 中的定位为 `"qualification": "optional_package"`、`"policy": "cpu_first"`、`"gpu_required": false`。
- 所有 OCR、表格、公式识别与 MiniCPM 转录结果均仅作**候选识别草稿**；输出固定包含 `"source_fidelity": "not_assessed"`（MiniCPM/Jev 另附 `"acceptance_ready": false`）。自信度或 Jev 概率分数均不替代对源文件的独立核对与视觉比较，也不自动覆盖正文或授予验收。

## 15 个已注册操作清单

| Rust operation | positional args（依序） | kwargs（必填与常用可选项） |
|---|---|---|
| `prepare_source_inventory.ocr_hardware_plan` | 无 | `profile`（可选，默认 `"auto"`；支持 `"auto"` / `"tiny"` / `"small"` / `"medium"`） |
| `prepare_source_inventory.install_ocr_package` | `package_manifest` | `expected_package_sha256`（必填）、`store`（必填，目标安装目录） |
| `prepare_source_inventory.inspect_ocr_package` | 无 | `package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`layout`（`"auto"` / `"on"` / `"off"`） |
| `prepare_source_inventory.recognize_ocr_png` | `image_path` | `package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`layout`（`"auto"`）、`expected_image_sha256`（可选） |
| `prepare_source_inventory.recognize_ocr_png_accelerated` | `image_path` | `expected_image_sha256`（必填）、`package_manifest`（必填，CPU 清单）、`expected_package_sha256`（必填）、`gpu_package_manifest`（可选，需与 `expected_gpu_package_sha256` 成对）、`profile`（`"auto"`）、`layout`（`"auto"`）、`acceleration`（`"auto"` / `"cpu"` / `"gpu_required"`）、`timeout_seconds`（默认 `180`） |
| `prepare_source_inventory.prepare_pdf_ocr_evidence` | `source_pdf` | `destination`（必填）、`package_manifest`（可选）、`expected_package_sha256`（可选）、`expected_source_sha256`（可选）、`profile`（`"auto"`）、`layout`（`"auto"`）、`mode`（`"auto"`）、`dpi`（默认 `200`）、`regions`（默认 `{}`）、`coordinate_system`（可选）、`accelerator`（可选对象） |
| `prepare_source_inventory.recognize_ocr_table_png` | `image_path` | `package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`cell_mode`（默认 `"off"`）、`expected_image_sha256`（可选） |
| `prepare_source_inventory.recognize_pdf_table_region` | `source_pdf` | `destination`（必填）、`page_number`（必填整数）、`bbox`（必填四元数）、`coordinate_system`（必填）、`dpi`（默认 `200`）、`package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`cell_mode`（`"off"`）、`expected_source_sha256`（可选） |
| `prepare_source_inventory.recognize_ocr_formula_png` | `image_path` | `package_manifest`（必填）、`expected_package_sha256`（必填）、`expected_image_sha256`（可选） |
| `prepare_source_inventory.recognize_pdf_formula_region` | `source_pdf` | `destination`（必填）、`page_number`（必填整数）、`bbox`（必填四元数）、`coordinate_system`（必填）、`dpi`（默认 `200`）、`package_manifest`（必填）、`expected_package_sha256`（必填）、`expected_source_sha256`（可选） |
| `prepare_source_inventory.audit_ocr_evidence` | `primary_record` | `expected_primary_sha256`（必填）、`comparison_record`（可选，需与 `expected_comparison_sha256` 成对） |
| `prepare_source_inventory.inspect_minicpm_package` | `package_manifest` | `expected_package_sha256`（必填）、`model_kind`（必填：`"minicpm-ocr"` 或 `"jev"`） |
| `prepare_source_inventory.recognize_minicpm_region` | `image_path` | `expected_image_sha256`（必填）、`package_manifest`（必填）、`expected_package_sha256`（必填）、`category`（必填：`"text"` / `"table"` / `"formula"`）、`acceleration`（`"auto"` / `"cpu"` / `"gpu_required"`）、`timeout_seconds`（默认 `300`，范围 `5..=1800`）、`max_tokens`（默认 `2048`，范围 `1..=4096`） |
| `prepare_source_inventory.recognize_minicpm_pdf_region` | `source_pdf` | `destination`（必填）、`page_number`（必填整数）、`bbox`（必填四元数）、`coordinate_system`（必填未旋转 PDF 点坐标）、`dpi`（默认 `200`，范围 `72..=600`）、`expected_source_sha256`（必填）、`package_manifest`（必填）、`expected_package_sha256`（必填）、`category`（必填）、`acceleration`（`"auto"`）、`timeout_seconds`（默认 `300`，范围 `5..=1800`）、`max_tokens`（默认 `2048`，范围 `1..=4096`） |
| `prepare_source_inventory.score_jev_region` | `image_path` | `expected_image_sha256`（必填）、`render_image_path`（必填）、`expected_render_sha256`（必填）、`candidate_text_path`（必填）、`expected_candidate_sha256`（必填）、`package_manifest`（必填）、`expected_package_sha256`（必填）、`category`（必填）、`acceleration`（`"auto"`）、`timeout_seconds`（默认 `300`，范围 `5..=1800`） |

## Schema 1 资产清单与离线包安装

仓库 `rust/` 目录下维护 3 份固定资产清单，均要求 `schema_version: 1` 及完整小写 SHA-256 绑定：

- `rust/ocr-cpu-assets.json`：PP-OCRv6 CPU 模型与 ONNX Runtime 动态库清单，支持按硬件预算自动/手动选择 `tiny` / `small` / `medium` 配置，可选版面识别（`layout`: `auto` / `on` / `off`）、表格识别与公式识别资源。使用 `prepare_source_inventory.install_ocr_package` 将清单校验后安装到持久化 `store` 目录，禁止符号链接或 name-surrogate reparse point；`install_ocr_package` 可安全复用已核验资源并拒绝未登记或篡改文件。
- `rust/minicpm-runtime-assets.json`：`model_kind: "minicpm-ocr"`，`engine: "llama.cpp"`，锁定 `LLAMA_RELEASE` 为 `b11901`（上游提交 `c21a3ca524263ef8166cb6d350864b10c3056b61`），主模型仅限 `MiniCPM-OCR-1_0-Q8_0.gguf` 或 `MiniCPM-OCR-1_0-Q4_K_M.gguf`，多模态投影仅限 `mmproj-model-f16-4x.gguf`（选用 Q8 时可配 `dspark-MiniCPM-OCR-1_0-Q8_0.gguf` 草稿模型）。
- `rust/jev-runtime-assets.json`：`model_kind: "jev"`，`engine: "llama.cpp"`，锁定 `JEV_LLAMA_RELEASE` 为 `b11928`，模型仅限 `MiniCPM-OCR-Jev-Q4_K_M.gguf` / `Q8_0` / `F16` 与 `mmproj-model-f16.gguf`。

## GPU / CPU 回退与 `max_tokens` 参数说明

1. **硬件调度与路径隔离**：
   - `recognize_ocr_png_accelerated` 与 MiniCPM / Jev 操作均支持 `acceleration: "auto" | "cpu" | "gpu_required"`。
   - 在 `"auto"` 模式下，仅当清单提供已校验 `gpu` 运行时包时先在子进程尝试 GPU（Windows `DmlExecutionProvider` / llama CUDA 或 Vulkan；macOS `CoreMLExecutionProvider` / Metal；Linux `CUDAExecutionProvider` / CUDA 或 Vulkan），失败、超时或未报告实际 GPU 层卸载时记录诊断并自动回退 CPU 尝试。指定 `"gpu_required"` 时若未提供或无法完成 GPU 卸载则返回 `status: "blocked"` 而不静默降级。
   - 子进程启动时将运行目录**前置拼接**到 `PATH`（Linux 同时前置 `LD_LIBRARY_PATH`，macOS 前置 `DYLD_LIBRARY_PATH`），既优先加载绑定运行时库，也保留系统与显卡驱动 DLL/SO 搜索路径。
2. **MiniCPM 生成长度限制（`max_tokens`）**：
   - `recognize_minicpm_region` 与 `recognize_minicpm_pdf_region` 均接收可选整数 `max_tokens`，默认 `2048`（对应 `--ctx-size 4096`），允许范围 `1..=4096`；长表格或整段公式若出现截断，返回中将包含 `"truncated": true` 及 `"status": "truncated_review_required"`，此时可在上下文限制内调大 `max_tokens` 或拆分区域重新识别。

## MiniCPM / Jev 调用示例骨架

```json
{
  "schema_version": 1,
  "operation": "prepare_source_inventory.recognize_minicpm_region",
  "args": ["/本次任务的绝对路径/work/region.png"],
  "kwargs": {
    "expected_image_sha256": "<实际 region.png SHA-256>",
    "package_manifest": "/运行目录/minicpm-runtime-assets.json",
    "expected_package_sha256": "<实际清单 SHA-256>",
    "category": "table",
    "acceleration": "auto",
    "timeout_seconds": 300,
    "max_tokens": 2048
  }
}
```

PDF 区域直接裁剪识别使用 `prepare_source_inventory.recognize_minicpm_pdf_region`：

```json
{
  "schema_version": 1,
  "operation": "prepare_source_inventory.recognize_minicpm_pdf_region",
  "args": ["/本次任务的绝对路径/input/article.pdf"],
  "kwargs": {
    "destination": "/本次任务的绝对路径/work/minicpm-region",
    "page_number": 3,
    "bbox": [72.0, 140.0, 520.0, 420.0],
    "coordinate_system": {
      "space": "pdf_page",
      "unit": "pt",
      "origin": "top_left",
      "x_axis": "right",
      "y_axis": "down",
      "rotation": "unrotated"
    },
    "dpi": 200,
    "expected_source_sha256": "<实际 PDF SHA-256>",
    "package_manifest": "/运行目录/minicpm-runtime-assets.json",
    "expected_package_sha256": "<实际清单 SHA-256>",
    "category": "formula",
    "acceleration": "auto",
    "timeout_seconds": 300,
    "max_tokens": 2048
  }
}
```

## CI 与本地夹具烟雾验证

无需下载外部大权重即可运行仓库内确定性夹具测试，验证包校验、旋转逆变换、内存预算、诊断脱敏、`max_tokens` 边界及失败隔离逻辑：

```bash
cargo test --manifest-path rust/Cargo.toml --locked --test ocr
cargo test --manifest-path rust/Cargo.toml --locked --lib minicpm::tests
```
