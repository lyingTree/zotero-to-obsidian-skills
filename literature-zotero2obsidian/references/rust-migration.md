# Rust 原生引擎迁移评估

目标：最终替换内嵌 Python 的运行器，使可确定的机械处理由 Rust 原生执行；Agent 仅做来源理解和翻译。不把 Rust 改写等同于提高翻译正确性。二进制分发与语言迁移分开验收。

| 顺序 | 模块 | 候选实现 | 验收重点 |
|---|---|---|---|
| 1 | 版本化 JSON 请求、错误、路径、哈希、命名、集合父链 | serde/serde_json、sha2、std::path、unicode-normalization | Windows UTF-16 长度、大小写与 Unicode 冲突、JSON 重复键、证据精确对应 |
| 2 | YAML、Markdown、Rich Table、链接、锚点、格式修复 | pulldown-cmark 或 comrak + Obsidian 专用解析；审阅维护状态后选 YAML 库 | 保留未修改字节、YAML 重复键、公式/注释/围栏、旧锚点兼容、所有 sheet 与幂等 |
| 3 | 来源交付合同、翻译验收记录、原子写入、完整任务编排 | serde、rayon（独立任务）、文件锁、同文件系统独占发布 | 失效证据、并发、断点恢复、来源范围 |
| 4 | DOCX 原生对象及图片、Zotero API | zip、quick-xml、image、reqwest | XML 顺序、媒体关系、坐标和文件散列；真实 API 状态和凭证隔离 |
| 5 | PDF 渲染、文本、图片提取 | 比较 pdfium-render/PDFium、lopdf 或经许可的 MuPDF 绑定 | 字符位置、旋转/裁框、裁剪、SMask、对象引用、OCR 区域、Python 基准一致性 |

PDF 是风险最高的模块。PDFium 仍需相应平台的动态或静态 PDFium 库；不是所有 PDF 能力都可用纯 Rust crate 替代。PyMuPDF 已调用 C/C++ MuPDF，不能假设 Rust 一定更快。先固定同一 PDF 的提取、页面图、对象身份与边界判定，再替换库。各候选依赖单独核对许可与安全维护状态。

保留当前 Python 引擎为迁移期间的参考实现。冻结 JSON 契约，Rust 按操作逐项接管，必要时显式回退已验证的 Python 二进制（过渡期，不是最终无 Python 方案）。两种实现独立运行并比较真实文件产物及原子诊断；不能因两种实现都输出 pass 而授予来源语义通过。现有全部回归、新增二进制 smoke、历史模型样例、正常/异常/Unicode/并发/恢复用例均纳入迁移门槛。Markdown 固定规则输出字节一致；因 PDF 引擎导致的坐标差异须有明确证据和经审核的容差，不降低验收标准。

实测冷启动、每篇处理时间、峰值内存、批量吞吐和失败分类，不预报未经测量的加速倍数。优先并行独立文献/页面，保持对象顺序及哈希确定性。长期目标为 Rust 统一运行器，必要的 PDF/OCR 原生库随平台分发，用户不安装 Rust 编译器。全部操作迁移与跨平台对照回归通过后，再删除内嵌 Python 运行时。
