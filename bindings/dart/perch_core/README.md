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
