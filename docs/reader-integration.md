# 阅读器宿主联调

本文描述 PerchCore 与阅读器宿主之间的最小完整流程。当前仓库不包含旧 PerchReader App，因此可在本仓库内验证的边界到“宿主已取得可交给渲染器的源坐标”为止。

## 可执行流程

```sh
cargo build -p perch-ffi --release --locked
cd bindings/dart/perch_core
dart pub get
dart run example/reader_flow.dart \
  ../../../target/release/libperch_ffi.so \
  ../../../examples/data/sample.txt 文字
```

`just reader-flow` 执行同一流程。`just ffi-smoke` 也包含该示例，避免宿主链路在后续改动中静默失效。

示例只使用公开 Dart API，依次完成：

1. 创建 `PerchWorker` 和独立 `PerchServices` context。
2. 按路径打开管线，登记 BookId，执行 refine 和 index。
3. 关闭所有句柄与 worker，模拟 App 进程重启。
4. 重新打开产物目录，确认该书恢复为 Ready。
5. 通过 `book.load` 加载 `index.bin` 与 `text.json`，执行搜索。
6. 将命中区间交给 `AsyncBookHandle.locate` 取得源端点。
7. TXT 用源 scalar 区间调用 `txt.slice` 验证原文；EPUB 输出 href、nodePath 和 UTF-16 偏移。
8. 显式关闭 book、TXT、pipeline、context 和 worker。

## 渲染器接线

TXT 宿主使用 `TxtSourceEndpoint.charOffset` 作为阅读文本的 Unicode scalar 边界。不要直接当作 Dart `String.length` 的 UTF-16 索引；可以让核心通过 `txt.slice` 返回匹配文本，再由阅读视图滚动或高亮。

EPUB 宿主先打开 `EpubSourceEndpoint.href` 指向的 spine 资源，然后按 `nodePath` 定位实体解码后的文本节点，最后用 `utf16Offset` 对接 Dart/JavaScript DOM 字符串边界。跨章节命中的两个端点可有不同 href；宿主需决定跳到起点、展示跨章节高亮，或只显示命中所在章节。

## 真实 App 验收清单

- 前台打开、搜索、跳转、关闭不阻塞 UI isolate。
- App 被系统终止后，重启能从 Ready/Indexing/Failed 检查点恢复。
- TXT 的 emoji、组合附加符、CRLF 与多空行命中能滚动到正确原文。
- EPUB 的实体、CDATA、重复文本和跨节点命中能解析 nodePath/UTF-16 偏移。
- 源文件删除、产物损坏和旧版本产物在 UI 中有可理解的恢复/重方向提示。
- App 关闭书籍或销毁服务后，所有句柄和 context 被回收。

当前机器上没有可写的 PerchReader App 工程，所以排版、DOM 选区、页面滚动与真机生命周期尚未执行。提供 App 工作区后，上述示例的 `PerchServices` 调用顺序可直接迁入应用层。
