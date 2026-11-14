# 搜索命中回源文本

LOC-01 为 TXT 与 EPUB 增加索引文本到源文本的区间映射。JSON API v1.4 新增 `book.locate`，搜索仍返回原有命中与 Locator；阅读器选中命中后再获取两个源端点。

## 调用与单位

在创建该书句柄的 context 内调用：

```text
book.locate {"handle":0,"char_start":3,"char_end":4}
```

起止值直接取 `book.search` 的 `charStart/charEnd`，满足 `0 ≤ start < end ≤ charCount`。区间按 Unicode scalar 半开边界计数，包含组合附加符与独立的 emoji scalar；不按 UTF-16 码元或用户可见字形计数。

TXT 原解码文本 `"\r\n甲\r\n\r\n\r\n乙😀"` 的索引正文为 `"甲\n\n乙😀"`。“乙”的索引偏移为 3，返回：

```json
{
  "start": {"kind":"txt","charOffset":9,"byteOffset":11},
  "end": {"kind":"txt","charOffset":10,"byteOffset":14},
  "containsSynthetic": false
}
```

`charOffset` 对应建索引时原解码 TXT 的 scalar 边界，可以交给匹配解码版本的 `txt.slice`。`byteOffset` 对应该解码文本的 UTF-8 字节边界；BOM 已移除，GBK/Big5 等原编码文件字节偏移与它不同。

EPUB 端点形状：

```json
{
  "kind":"epub",
  "href":"/OEBPS/text/ch01.xhtml",
  "nodePath":"/2/4/2/1",
  "scalarOffset":5,
  "utf16Offset":6
}
```

`nodePath` 是 `XhtmlIndex` 的局部 CFI 风格文本节点路径，不带 `!` 间接路径或 `:offset`。编号保留空白文本节点，元素与文本各自计数；相邻 Text、实体和 CDATA 按现有抽取语义合并。Rust 可以用 `CfiPath::parse` 和 `XhtmlIndex::resolve` 找到节点。两个偏移都指向实体解码后的节点文本；非 BMP 字符在 UTF-16 中占两个码元。跨章节时，起点与终点分别保留自己的 href。

## 边界与截断

TXT 保持原有换行转换、首尾空行移除、连续空行折叠及行内空白保留规则。每个转换/折叠后的换行覆盖其完整原解码区间；被移除的首尾空行不进入映射。

EPUB 每个文本节点后原有的人工 `\n` 标为合成内容。定位时起点向右、终点向左选择请求区间内的可映射边界；不扩展到请求之外。`containsSynthetic` 表示整个原请求是否包含人工分隔符，即使端点已向内吸附也保留 true。原节点里的空白仍是源文本。只包含合成内容的请求无法定位。

保持现有截断结果：TXT 可按 scalar 在行内截断，映射同步裁到该前缀；EPUB 仍按完整文本节点接纳，并保留节点后的分隔符，因此 charCount 可能比 max_chars 多 1。定位上界取实际已索引字符数。

默认定位针对建立索引时的源文本版本与解码。调用不重新读取源文件；源文件之后改变或删除，不会改变已打开或已保存的映射。本项提供源坐标，页面跳转、选区高亮和内容变更后的模糊重锚由后续阅读器集成处理。

## 错误与旧索引

| 情况 | 错误码 |
|---|---|
| text.json 没有 sourceMap | `BOOK_SOURCE_MAP_MISSING` |
| 映射版本未知 | `BOOK_SOURCE_MAP_UNSUPPORTED_VERSION` |
| 映射字段、区间、UTF-8 边界、源类型或节点路径无效 | `BOOK_SOURCE_MAP_INVALID` |
| 空区间、反向、越界或区间字段不是 u64 | `BOOK_LOCATE_BAD_RANGE` |
| 区间只含人工分隔符 | `BOOK_SOURCE_UNMAPPABLE` |

前三类问题不会阻止 `book.load` 与 `book.search`。直接 `book.load` 不重建产物，调用方需要从原书重建映射才能精确定位。PathStore 管线在打开审计时会将这三类 Ready 产物降级为 indexing，等重新提供原书路径后成组重建。整个正文 JSON 损坏、正文版本不支持或 index/text 不一致仍按原有 `book.load` 错误处理。

沿用 `BOOK_BAD_HANDLE`、`HANDLE_WRONG_KIND`、`CONTEXT_HANDLE_MISMATCH` 和 context 错误。定位取得只读 Arc 租约后在根注册锁外完成；close 摘除句柄，已经取得租约的请求仍可结束。

## Rust 与 Dart

Rust 新入口为 `extract_text_with_source_map`、`extract_txt_with_source_map`、`extract_epub_text_with_source_map`，返回 `MappedBookText`。`book_text()` 借用原 `BookText`，`locate(start,end)` / `locate_source_range` 返回 `SourceLocation`。`SourceEndpoint::Txt` 与 `SourceEndpoint::Epub` 区分坐标单位；`source_map()` 单独检查映射可用性。

`BookText` 的公开字段、原抽取函数、`index_book` 和 `locate_hit` 签名不变。旧 `locate_hit` 保留粗 EPUB Locator 与归一化 TXT 偏移，精确回源使用新的映射入口。持久化通过 `mapped_book_text_to_vec/from_slice`；旧的 body-only 编解码继续工作。

同步 Dart：

```dart
final location = core.locateBook(
  handle: handle,
  charStart: hit.charStart,
  charEnd: hit.charEnd,
);
final source = location.start as TxtSourceEndpoint;
```

服务层的 `AsyncBookHandle.locate(hit)` 自动携带该句柄的 context，返回 `BookSourceLocation`。EPUB 使用 `EpubSourceEndpoint` 的 href、nodePath、scalarOffset、utf16Offset；关闭后的异步句柄立即抛 StateError，映射业务错误保留 `PerchCoreException.code`。

存储形状见 [存储契约](storage.md)，字段和 Schema 见 [方法参考](api-reference.md) 与 [API 夹具](../api/README.md)。尺寸、打开耗时和进程内存见 [LOC-01 测量记录](benchmarks/2026-11-13-linux-source-map.md)。
