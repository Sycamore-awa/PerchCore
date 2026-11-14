# perch_core：纯 Dart 绑定

通过 dart:ffi 调用 PerchCore 的 C ABI，不依赖 Flutter。

- `PerchCore`：同步 legacy/context JSON 调用、错误转换和内存释放。
- `PerchWorker`：常驻 isolate，按顺序处理请求。
- `PerchServices`：每实例独立 context、类型化管线/TXT/搜索句柄与成组关闭。

```sh
dart pub get
dart analyze
dart run tool/ffi_smoke.dart ../../../target/release/libperch_ffi.so
dart run example/main.dart ../../../target/release/libperch_ffi.so ../../../examples/data/sample.txt
dart run example/reader_flow.dart ../../../target/release/libperch_ffi.so ../../../examples/data/sample.txt 文字
```

先在仓库根目录编译 perch-ffi。macOS 使用 dylib，Windows 使用 dll。移动端需要自己的静态/动态库打包，当前仅附带 iOS 静态链接适配器。

worker 在后台 isolate 按序执行同步计算。Rust 根注册锁只管理句柄租约，解析、搜索、源定位和切片在锁外完成；pipeline 按对象串行。多个 `PerchServices` 实例的句柄配额和销毁互相隔离；正常关闭请使用 `PerchServices.close`。异常退出仍不能自动调用 `context.close`。

接口说明见 [C ABI](../../../docs/ffi.md)，许可证为 [MIT](../../../LICENSE)。

API v1.3 的路径方法：同步和 `PerchServices` 都提供 `inspectTextPath`、`refineBookPath`、`openBookPath`。文件由核心读取，不在 Dart 构造整本 hex；inspect 只返回统计，异步 open 返回托管的 `AsyncBookHandle`。TXT 阅读继续用 openTxt/slice。详见 [路径输入](../../../docs/io.md)。

API v1.4 新增同步 `locateBook(handle:..., charStart:..., charEnd:...)` 与 `AsyncBookHandle.locate(hit)`，返回 `BookSourceLocation`。TXT 的 `TxtSourceEndpoint` 包含原解码 charOffset/UTF-8 byteOffset；EPUB 的 `EpubSourceEndpoint` 包含 href/nodePath/scalarOffset/utf16Offset，两个端点可属于不同章节。人工分隔符向内吸附，containsSynthetic 保留原请求的标记。源坐标针对建立索引时的版本，旧产物缺少映射时仍可搜索，定位错误由 `PerchCoreException.code` 区分。详见 [搜索命中回源文本](../../../docs/source-location.md)。

`example/reader_flow.dart` 是无 UI 的阅读器宿主联调入口：用 `PerchServices` 完成路径导入、阶段执行、模拟 App 重启、Ready 恢复、落盘加载、搜索、命中回源、TXT 阅读切片或 EPUB 节点坐标输出，并显式关闭所有句柄。它验证宿主调用链，不代替 Flutter 排版、DOM 选区或真机页面跳转。

全部 35 个方法可通过 `PerchCore.invoke` 调用；显式上下文使用 `openContext`、`invokeInContext` 和 `closeContext`。参数、结果和错误见 [方法参考](../../../docs/api-reference.md)；`PerchCore.compatibility()` 提供 ABI/API/Locator/产物版本查询。顶层 Rust error 转为 `PerchCoreException`，任务 failed/exhausted 仍作为业务结果返回。输入和返回内存由门面管理。

句柄包含 0，不能用 0 判断打开失败。TXT 的 start/char/chars 是 Unicode scalar 数；Dart String.length 是 UTF-16 code unit 数，含 emoji 时两者不同。txtByteOffset 对应解码后 UTF-8。Dart int 为有符号 64 位，JSON 数值传参须在本语言可精确表达范围内。

[接口验证工具](../../../api/README.md)会编译 tool/api_replay.dart 为 AOT 程序，与持久 C 客户端重放 261 次调用并比较完整成功结果、错误码、Schema 和响应样例。Dart 不在运行时自动执行 Schema；规范请求检查在开发工具中完成。
