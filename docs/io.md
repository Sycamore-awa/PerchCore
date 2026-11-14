# 文件路径输入（IO-01）

JSON API v1.3 为 `jobs.refine`、`book.open` 增加路径输入，并新增只返回统计的 `txt.inspect`。C ABI 仍为 v1，既有字节请求、结果形状、句柄和产物格式保持兼容。旧 v1.1/v1.2 Schema 永久保留，新快照使用 `urn:perchcore:api:v1.3:*`。

## 入口选择

| 用途 | 推荐请求 | 返回 |
|---|---|---|
| EPUB/TXT 导入精修 | `jobs.refine {"path":"/books/a.epub","format":"epub"}` | 现有元数据、目录和字数 |
| EPUB/TXT 搜索索引 | `book.open {"path":"/books/a.epub","format":"epub"}` | 句柄与索引统计，沿用 500 万 scalar 抽取上限和 truncated |
| TXT 编码与统计 | `txt.inspect {"path":"/books/a.txt"}` | encoding、certain_encoding、chars、lines、had_decode_errors |
| TXT 阅读 | `txt.open` 后 `txt.slice` | 元数据和每次最多 20,000 scalar 的文本窗口 |
| PDF 结构 | 现有 `pdf.analyze {"path":"/books/a.pdf"}` | 页数、元数据和大纲 |

`jobs.refine`、`book.open` 和 `pdf.analyze` 的 `path` 与 `bytes_hex` 必须且只能提供一个。null 字段也算存在；两者同时提供、两者均省略或来源类型错误返回 `FFI_BAD_ARGS`。旧字节分支的格式嗅探、抽取选项和解析错误保留。已知格式时显式提供 `format`，避免把损坏的 EPUB 嗅探为 TXT。

`txt.inspect` 与 `txt.open` 共用解码规则：BOM、合法 UTF-8、编码探测；提供 `encoding` 可以跳过探测。统计对应原解码文本的 Unicode scalar 和行数，不经过索引空白归一化。inspect 不创建句柄、不占 TXT 配额，不返回 text。旧 `txt.analyze` 仍可用于已有字节调用。

## 所有权、错误与边界

路径是运行库所在进程可访问的 UTF-8 文件路径，推荐绝对路径；相对路径以宿主进程工作目录解析。JSON 字符串仅在调用中借用。核心同步读取文件，在根注册锁之外解析/解码/建索引，不保留文件描述符。成功打开后，正文与索引由句柄持有；源文件修改或删除不影响已打开对象，也不会自动刷新它。

| 分支 | 文件读取失败 | 其他错误 |
|---|---|---|
| jobs.refine 路径 | `JOBS_IO` | 沿用 `JOBS_EPUB` / `JOBS_EMPTY_CONTENT` / `JOBS_UNSUPPORTED_FORMAT` 等 |
| book.open 路径 | `BOOK_OPEN_IO` | 沿用解析、配额、context 与句柄错误 |
| txt.inspect | `TXT_IO` | `TXT_UNKNOWN_ENCODING` |
| pdf.analyze 路径 | `PDF_IO` | 沿用 PDF 结构错误 |

路径入口去掉整本十六进制字符串与 JSON 字节副本，仍全量读取文件；TXT 仍全量解码，索引仍驻留正文。这里不承诺流式处理、文件大小预算或峰值内存上限。精修目录/PDF 大纲的大小仍取决于源文件结构；`jobs.refine` 沿用 max_chars 与不返回 truncated 的现有结果契约。统一输入/结构预算仍按实际需求启动 LIMIT-01。

`book.open` 的 context 所有权、配额及 close/use 租约规则与字节入口相同；打开期间 context 已关闭时，最终登记失败并释放已构造对象。路径不需要在后续搜索期间继续存在。

## Dart 接入

同步门面提供 `inspectTextPath`、`refineBookPath`、`openBookPath`，字节方法继续可用。`PerchServices` 提供同名异步方法，在 worker 中读取文件；open 返回托管的 `AsyncBookHandle`，服务关闭时成组释放。

```dart
final stats = await services.inspectTextPath('/books/a.txt');
final book = await services.openBookPath('/books/a.epub', format: 'epub');
try {
  final hits = await book.search('海风');
} finally {
  await book.close();
}
```

使用新方法/请求前，调用方检查 `compatibility.api.major == 1` 且 minor 至少为 3。旧核心不自动回退到读取并生成 bytes_hex；迁移选择由宿主决定。

## 验证

Rust 回归比较 EPUB 2/3、UTF-8/BOM/GBK/Big5 和跨分片样本的路径与字节精修、索引统计、搜索结果；覆盖读取失败、来源互斥、Unicode 截断、context 所有权和删除源文件后的句柄使用。15 MB TXT 的路径索引明确返回 500 万字符和 truncated:true，inspect 仍报告完整统计；两类结果均小于 256 字节。

持久 C 与 Dart AOT 客户端在同一组真实调用中比较两种输入，保留 v1.2 的 161 组冻结请求/响应。Dart smoke 另行验证同步/worker 门面和服务关闭回收。实际验证数量与环境见 [状态记录](status.md)。
