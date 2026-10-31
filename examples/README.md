# 最小示例

示例均为命令行程序，不包含阅读器 UI。

## Rust：解析与搜索

在仓库根目录运行：

```sh
cargo run -p perch-core --example inspect -- examples/data/sample.txt 文字
python3 tools/make_sample_epub.py target/sample.epub
cargo run -p perch-core --example inspect -- target/sample.epub 索引
```

示例实现位于 [inspect.rs](../crates/perch-core/examples/inspect.rs)。PDF 输入只返回结构信息。

## C：调用 JSON 接口

Linux：

```sh
cargo build -p perch-ffi --release --locked
cmake -S examples/c -B target/c-example \
  -DPERCH_CORE_LIBRARY="$PWD/target/release/libperch_ffi.so"
cmake --build target/c-example
ctest --test-dir target/c-example --output-on-failure
target/c-example/perch-example version
target/c-example/perch-example locator.validate '{"href":"txt:","type":"text/plain","perch":{"v":1,"format":"txt","txtOffset":0}}'
```

macOS 将库文件后缀替换为 `.dylib`。C 示例只打印原始 JSON，业务错误仍在 JSON 内；完整客户端应解析 `error`，管理句柄并调用 `perch_string_free`。

## Dart：后台调用与 TXT 切片

```sh
cargo build -p perch-ffi --release --locked
cd bindings/dart/perch_core
dart pub get
dart run example/main.dart ../../../target/release/libperch_ffi.so ../../../examples/data/sample.txt
```

此示例使用常驻 worker 和类型化 TXT 服务，打开后只返回少量文本，最后关闭句柄与 worker。它不需要 Flutter。
