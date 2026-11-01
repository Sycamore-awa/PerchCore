# 兼容夹具

这些小文件锁定 API-02 开始时已存在的读取契约，不是性能样本。

| 文件 | 版本 | 用途 |
|---|---:|---|
| `locator-v1.json` | Locator perch v1 | 验证已持久化的 v1 定位仍可解析与校验 |
| `text-v0.json` | text.json legacy v0 | 验证没有 `format` 和版本字段的旧 EPUB 正文仍按 EPUB 读取 |
| `index-v1.hex` | index.bin v1 | 以 hex 保存精确字节，验证 v1 索引仍能加载和查询 |

`index-v1.hex` 解码后的正文为 ASCII `a`，分片大小为 4096。夹具不由当前写入器在测试时重建，否则写入器和读取器同时改变时会掩盖兼容回归。

更新格式时新增夹具，不覆盖旧版本。详细规则见 [版本与兼容策略](../../../docs/compatibility.md)。

