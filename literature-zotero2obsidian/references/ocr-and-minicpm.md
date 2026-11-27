# PDF 质量预检、内置 PP-OCRv6 与零乱码视觉核对指南

本文记录 `z2o` Rust 运行时内置的 **4 层零乱码防火墙**（`pdf-inspector` + `lopdf` 字体 CMap 诊断 + `mojibake` 乱码审计 + `validate_note` 硬门禁）、开箱即用的内置 PP-OCRv6 OCR / 版面 / 表格识别能力，以及已注册的 10 个 `prepare_source_inventory.*` PDF 质量与 OCR 操作。所有操作统一通过 `z2o invoke request.json` 执行，先用 `z2o describe` 确认注册状态。

## 开箱即用（无首次安装询问）

- 发行版通过 `install.ps1` / `install.sh` 与 `z2o package --ocr-bundle` 一体化安装：12 个 `literature-*` 技能、预编译 `z2o[.exe]` 与内置 PP-OCRv6 + ONNX Runtime（位于 `literature-zotero2obsidian/runtime/ocr/`，含 `package.json` 与 `package.sha256`）一次落盘并通过 SHA-256 完整性校验，**不存在首次运行“是否下载可选 OCR”弹窗**。
- `z2o describe` 中的 `ocr_backend` 字段报告 `"qualification": "bundled_package"` 与实时自动发现结果 `bundled_status`（`available`、`source: "exe_adjacent" | "env" | "absent"`、`manifest_path`、`manifest_sha256`）。
- 历史遗留的 `user/profile.json` 中若仍存有 `formula_table_enhancement: "unasked"`，运行时自动向后兼容为开箱即用模式，不触发任何阻断或询问。
- 所有 OCR 与表格结构识别结果均仅作**候选识别草稿**，固定携带 `"source_fidelity": "not_assessed"` 与 `"acceptance_ready": false`，绝不跳过对原页的独立视觉核对。

## 4 层零乱码防火墙（禁止输出乱码）

1. **Layer 1 — 纯 Rust `pdf-inspector` 结构预检**：`prepare_source_inventory.inspect_pdf_quality` 与 `prepare_pdf_layout` 在解析 PDF 前自动分类文档类型（`TextBased` / `Scanned` / `ImageBased` / `Mixed`）、定位需 OCR 页面、检测多栏与表格页以及 CMap 映射缺口。
2. **Layer 2 — `lopdf` 字体与 CMap 审计**：逐页扫描 `/Resources /Font`，捕获缺少 `/ToUnicode` 的 Type 3 字体与 Identity-H/V Type 0 CID 字体，自动标为整页视觉核对与 OCR 路由。
3. **Layer 3 — 内置 300 DPI PP-OCRv6 自动修复 + `<!-- VISUAL_REQUIRED: ... -->` + 拼图**：
   - `prepare_pdf_layout` 对检出乱码（`U+FFFD`、控制符、私用区 PUA `U+E000..=U+F8FF` 或误解码字节序列）的文本块，若已内置 OCR 包则自动在 300 DPI 下运行 PP-OCRv6 修复；**仅当修复文本零乱码且全部行置信度 `text_score >= 0.92`、`box_score >= 0.60`、`location_valid == true` 时采纳**。
   - 未能高置信修复的乱码块**严禁原样写入 `clean-layout.md`**，而是替换为显式 `<!-- VISUAL_REQUIRED: <id> | reason=<原因> | page=<页码> | bbox=[x0,y0,x1,y1] -->` 占位标记；同时对 2D 显示公式、第 1 页作者/单位块及缺字/扫描页生成 300 DPI 裁剪与编号拼图 `ocr-verify-sheet.png`（最多 32 块/张）。
4. **Layer 4 — 写库硬门禁（`validate_note` / `accept_note` / `vault_publish`）**：
   - 发布前强制扫描整篇笔记，任何残留的 `U+FFFD`、`NUL`/非法控制符、PUA 字符、GBK/UTF-8 典型乱码片段或尚未消解的 `<!-- VISUAL_REQUIRED: ... -->` 标记均直接阻断发布（`error` 级）。
   - TeX 源码（`.tex` / `.zip` / `.tar.gz`）与纯 Rust 多格式文档（HTML/XHTML、EPUB、ODT、TXT/MD）均经过 `encoding_rs` + `chardetng` 自动转码为干净 UTF-8，杜绝 GBK/Latin-1/Shift-JIS 乱码。

## 10 个已注册 PDF 质量与 OCR 操作清单

| Rust operation | positional args（依序） | kwargs（必填与常用可选项） |
|---|---|---|
| `prepare_source_inventory.inspect_pdf_quality` | `source_pdf` | `expected_source_sha256`（可选；返回 `pdf-inspector` + `lopdf` 字体/CMap/乱码/表格/多栏诊断报告） |
| `prepare_source_inventory.ocr_hardware_plan` | 无 | `profile`（可选，默认 `"auto"`；支持 `"auto"` / `"tiny"` / `"small"` / `"medium"`） |
| `prepare_source_inventory.install_ocr_package` | `package_manifest` | `expected_package_sha256`（必填）、`store`（必填，目标安装目录；用于离线手工更新或自定义模型包导入） |
| `prepare_source_inventory.inspect_ocr_package` | 无 | `package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`layout`（`"auto"` / `"on"` / `"off"`） |
| `prepare_source_inventory.recognize_ocr_png` | `image_path` | `package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`layout`（`"auto"`）、`expected_image_sha256`（可选；返回含 `mojibake_audit` 与 `< 0.92` 复核提示） |
| `prepare_source_inventory.recognize_ocr_png_accelerated` | `image_path` | `expected_image_sha256`（必填）、`package_manifest`（必填，CPU 清单）、`expected_package_sha256`（必填）、`gpu_package_manifest`（可选，需与 `expected_gpu_package_sha256` 成对）、`profile`（`"auto"`）、`layout`（`"auto"`）、`acceleration`（`"auto"` / `"cpu"` / `"gpu_required"`）、`timeout_seconds`（默认 `180`） |
| `prepare_source_inventory.prepare_pdf_ocr_evidence` | `source_pdf` | `destination`（必填）、`package_manifest`（可选；留空自动发现内置 `<exe_dir>/ocr/`）、`expected_package_sha256`（可选）、`expected_source_sha256`（可选）、`profile`（`"auto"`）、`layout`（`"auto"`）、`mode`（`"auto"`）、`dpi`（默认 `200`）、`regions`（默认 `{}`）、`coordinate_system`（可选）、`accelerator`（可选对象） |
| `prepare_source_inventory.recognize_ocr_table_png` | `image_path` | `package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`cell_mode`（默认 `"off"`）、`expected_image_sha256`（可选） |
| `prepare_source_inventory.recognize_pdf_table_region` | `source_pdf` | `destination`（必填）、`page_number`（必填整数）、`bbox`（必填四元数）、`coordinate_system`（必填）、`dpi`（默认 `200`）、`package_manifest`（必填）、`expected_package_sha256`（必填）、`profile`（`"auto"`）、`cell_mode`（`"off"`）、`expected_source_sha256`（可选） |
| `prepare_source_inventory.audit_ocr_evidence` | `primary_record` | `expected_primary_sha256`（必填）、`comparison_record`（可选，需与 `expected_comparison_sha256` 成对） |

> 注：实验性重型模块（MiniCPM、Jev 及 `PP-FormulaNet_plus-L`）已彻底移除；复杂 2D 数学公式统一通过 arXiv TeX 源或 300 DPI `ocr-verify-sheet.png` 裁剪由宿主多模态视觉直接核对，准确度更高且无需数 GB 额外权重。

## 硬件自适应调度阈值（CPU-first）

`ocr_hardware_plan` 按本机总内存与可用内存自动选择档位，不依赖独立显卡：

| 内存条件（总内存 / 可用内存） | 自动档位 | 最大检测边长 | 线程上限 | 版面分析（`PP-DocLayoutV3`） |
|---|---|---|---|---|
| `>= 8 GiB` 且 `>= 2 GiB` | `medium`（默认随发行版内置） | `2048 px` | `8` | 允许（`auto` 启用） |
| `>= 4 GiB` 且 `>= 1 GiB` | `small` | `1600 px` | `4` | 关闭 |
| `< 4 GiB` 或 `< 1 GiB` | `tiny` | `1280 px` | `2` | 关闭 |

## 锁定资产清单与 GPU/CPU 回退

仓库 `rust/` 目录下维护两份 SHA-256 锁定清单（Apache-2.0 / MIT）：

- `rust/ocr-cpu-assets.json`：5 大目标平台的 ONNX Runtime `1.32.0` 动态库清单。
- `rust/ocr-bundle-assets.json`：官方 `PaddlePaddle` ONNX 模型清单（`PP-OCRv6_medium`、`PP-DocLayoutV3`、`PP-LCNet_x1_0_doc_ori`、`SLANet_plus` 及备选 `small`/`tiny` 档位），每个权重文件均绑定精确字节大小与 SHA-256，供 CI 发行流水线打包为 `z2o package --ocr-bundle`。
- `recognize_ocr_png_accelerated` 支持 `acceleration: "auto" | "cpu" | "gpu_required"`；在 `"auto"` 下若提供已校验 GPU 包则优先在隔离 worker 子进程尝试（Windows DirectML / macOS CoreML / Linux CUDA），失败或超时自动回退 CPU，绝不污染主进程。

## 本地验证命令

无需联网下载外部大权重即可运行仓库内全套确定性夹具与防火墙单测：

```bash
cargo test --manifest-path rust/Cargo.toml --locked --test ocr
cargo test --manifest-path rust/Cargo.toml --locked --lib mojibake::tests
cargo test --manifest-path rust/Cargo.toml --locked --lib pdf_inspector_bridge::tests
cargo test --manifest-path rust/Cargo.toml --locked --lib pdf_layout::tests
```
