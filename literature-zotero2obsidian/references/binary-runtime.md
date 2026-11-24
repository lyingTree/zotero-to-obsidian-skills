# 独立二进制运行器（试验性）

目标：用户下载对应平台的压缩包，不需要安装 Python、pip、Python 编译器或 Git。PyInstaller 封装 CPython、核心依赖和现有脚本；不是 Rust 原生重写。每个平台单独构建。开发者仍可使用原 Python API 与源码测试。

## 安装与调用

验证发布包的 SHA-256，完整解压后运行主技能 bin/z2o.exe（Windows）或 bin/z2o（Linux/macOS）。将目标技能根作为明确参数传入 install，例如在用户授权的本地终端执行：

    z2o install --skills-root "D:\YourAgent\skills"

安装沿用原暂存、文件指纹、独占锁和备份回滚；两个外部技能的锁定提交在构建时纳入包，安装不调用 Git 或 pip。Zotero、Obsidian 应用及相应 API/CLI 不包含在二进制中。OCR 或视频等可选外部工具仍需按实际任务检查。

在全部 12 个技能中，优先使用已安装的主技能 bin/z2o。原 Python API 任务脚本的接口不变，以 run-script <实际本地脚本路径> 执行；Zotero 依赖使用 pyzotero 的固定版脚本也可通过此入口执行，不调用系统 Python。run-script 仅供用户或 Agent 审核的可信本地脚本，与原 Python 脚本权限一致，不是沙箱；未打包的任意第三方包和使用 sys.executable 调用 pip/任意解释器参数不受支持。原有动态模块加载和哈希检查需要真实文件，打包保留这些文件。不得临时替换验收器。

对于无需任务脚本的操作，使用 describe 获取明确注册的函数和签名；invoke 接收 JSON 请求文件。例如：

    {
      "schema_version": 1,
      "operation": "deterministic_stage_checks.author_token",
      "args": [["Smith", "Jones"]],
      "kwargs": {}
    }

路径参数使用对象 {"$path":"实际路径"}，二进制输入使用 {"$bytes":"base64内容"}，普通 JSON 对象递归解析。返回 JSON；进度写到 stderr，调用成功不代表文献内容通过验收。非注册函数不能通过 JSON 调用。其他既有 Python API（包括翻译审计 writer、图片对象处理）通过可信任务脚本调用。

开发者在 Python 3.12 环境安装 requirements-binary.txt，运行 scripts/build_binary.py 构建当前 OS/架构的包及 SHA-256 文件；scripts/smoke_binary.py 在清空系统 Python/Git PATH 的环境运行真实二进制，检查 PDF、DOCX、Markdown、依赖导入、全量安装和包指纹反例。GitHub Actions 分别构建 Windows、Linux、macOS；未验收的平台不得标为已支持。

## 发布边界

当前是二进制试验分支，构建脚本存在不等于发布完成。公开分发前须核对 PyMuPDF/MuPDF 的 AGPLv3 或商业许可及完整对应源码、通知和第三方许可。仓库所有者未确认前，CI 仅构建和运行测试，不上传公开二进制，不发布新 tag/release。压缩包散列不是代码签名；正式发布还需考虑 Windows 代码签名、macOS 签名/公证、Linux glibc 兼容性、依赖精确锁定及软件物料清单。历史回执及来源语义边界不变。
