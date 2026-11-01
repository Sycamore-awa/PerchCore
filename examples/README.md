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

[可复现样本集](data/corpus/README.md) 提供 EPUB 2/3、多编码 TXT、Unicode 与分片边界、超长行和 PDF 大纲样本，附 SHA-256 与预期文本，可用于正确性回归和后续性能测量：

```sh
python3 tools/make_corpus.py --check
cargo run -p perch-core --example inspect -- examples/data/corpus/inputs/epub2.epub
cargo run -p perch-core --example inspect -- examples/data/corpus/inputs/cross-shard.txt 跨片检索
cargo run -p perch-core --example inspect -- examples/data/corpus/inputs/pdf-outline.pdf
```

[基准工具](../docs/benchmarks/README.md) 可测上述样本的各处理阶段与真实 C ABI/Dart 调用，输出原始 JSON、百分位、吞吐和进程 RSS：

```sh
python3 tools/bench.py --samples utf8,epub2,pdf-outline
```

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

[replay.c](c/replay.c) 是可连续调用的 C 客户端：stdin 每行输入 `method<TAB>args_json`，stdout 每行输出一条原始 JSON，句柄在整个进程中有效。CMake 同时构建 perch-replay；`--self-test` 检查输入字节长度、UTF-8 和 NULL 规则。

完整 [接口样例与验证工具](../api/README.md) 将这个客户端与 Dart 的 tool/api_replay.dart 同时运行，覆盖所有 31 个方法；与一次调用后退出的最小示例用途不同。

## Dart：后台调用与 TXT 切片

```sh
cargo build -p perch-ffi --release --locked
cd bindings/dart/perch_core
dart pub get
dart run example/main.dart ../../../target/release/libperch_ffi.so ../../../examples/data/sample.txt
```

此示例使用常驻 worker 和类型化 TXT 服务，打开后只返回少量文本，最后关闭句柄与 worker。它不需要 Flutter。
