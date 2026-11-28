这两个字符表来自 PaddlePaddle 的 PP-OCRv6 tiny 和 medium 识别模型，遵循 Apache License 2.0。来源为各模型 `inference.yml` 中按原始顺序排列的 `PostProcess.character_dict`。上游文件地址及 SHA-256 记录在 [ocr-bundle-assets.json](../ocr-bundle-assets.json)。

运行器用它们恢复已知旧包中截短的字符表。发布构建使用完整 YAML 解析器提取字符表，并与这里的解压字节核对。
