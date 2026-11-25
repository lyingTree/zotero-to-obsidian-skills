# 完整图块的组装

每个逻辑图分别组装。多面板可以对应多个图片文件；所有图片、题注与图锚之间没有空行。没有印刷图号的图仍使用实际来源支持的标签，并分配本篇唯一的图锚。图块是否完整不以有没有正文调用为条件。

先按[裁剪与验收规则](../../literature-fulltext-extract/references/figure-crop.md)核对实际图件。下面只演示虚构内容的源码组装；资源路径、题注及图锚都由调用方按当前来源提供。

```json
{
  "schema_version": 1,
  "operation": "format_helpers.build_figure_block",
  "args": [
    [
      {"target": "assets/example-left.png", "alt": "图S8"},
      {"target": "assets/example-right.png", "alt": "图S8"}
    ]
  ],
  "kwargs": {
    "caption": "**图S8：** 已逐项核对的来源题注。",
    "anchor": "fig-s8"
  }
}
```

`build_figure_block` 只返回 Markdown 字符串，末尾含一个换行。所有图片行保留两个空格硬换行，题注紧接最后一个图片，独占行图锚紧接题注。组装整篇时让相邻图块/正文之间恰留一个空行。

`target` 传入实际资源路径的原始文字，不预先进行 URL 或 HTML 转义；助手负责保留路径中的百分号、井号、问号、反斜线与实体样式文字。题注是已明确编排的 Markdown，来源编号中转义过的星号等标记同样保留。

若来源只有“概念示意”这一标签，可明确提供 `caption="**概念示意**"` 和本篇已分配的锚。不为无说明文字的图增写解释。不改源中已有的面板标注，也不从文件名猜图号。

该函数不读取或修改图片，不确认题注译义，不证明来源图框完整，也不代替资源存在、来源哈希和逐图核验。最后读取完整候选并运行 `validate_note()` 与 `lint_presentation()`，按实际来源逐条处理提示。
