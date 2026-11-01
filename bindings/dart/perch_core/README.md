# perch_core：纯 Dart 绑定

通过 dart:ffi 调用 PerchCore 的 C ABI，不依赖 Flutter。

- `PerchCore`：同步 JSON 调用、错误转换和内存释放。
- `PerchWorker`：常驻 isolate，按顺序处理请求。
- `PerchServices`：类型化管线/TXT/搜索句柄与关闭管理。

```sh
dart pub get
dart analyze
dart run tool/ffi_smoke.dart ../../../target/release/libperch_ffi.so
dart run example/main.dart ../../../target/release/libperch_ffi.so ../../../examples/data/sample.txt
```

先在仓库根目录编译 perch-ffi。macOS 使用 dylib，Windows 使用 dll。移动端需要自己的静态/动态库打包，当前仅附带 iOS 静态链接适配器。

worker 移出同步计算，但不改变 Rust 全局锁的并发约束。正常关闭请使用 PerchServices.close；异常退出不能自动清空 Rust 注册表。

接口说明见 [C ABI](../../../docs/ffi.md)，许可证为 [MIT](../../../LICENSE)。

全部 31 个方法可通过 `PerchCore.invoke` 调用，参数、结果和错误见 [方法参考](../../../docs/api-reference.md)；`PerchCore.compatibility()` 提供 ABI/API/Locator/产物版本查询，其他便捷封装按需提供。顶层 Rust error 转为 `PerchCoreException`，任务 failed/exhausted 仍作为业务结果返回。输入和返回内存由门面管理。

句柄包含 0，不能用 0 判断打开失败。TXT 的 start/char/chars 是 Unicode scalar 数；Dart String.length 是 UTF-16 code unit 数，含 emoji 时两者不同。txtByteOffset 对应解码后 UTF-8。Dart int 为有符号 64 位，JSON 数值传参须在本语言可精确表达范围内。

[接口验证工具](../../../api/README.md)会编译 tool/api_replay.dart 为 AOT 程序，与持久 C 客户端重放相同调用并比较完整成功结果、错误码、Schema 和响应样例。Dart 不在运行时自动执行 Schema；规范请求检查在开发工具中完成。
