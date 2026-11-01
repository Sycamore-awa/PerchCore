# 构建与验证

## Rust

在仓库根目录运行，工具链由根目录 rust-toolchain.toml 选择 stable：

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build -p perch-ffi --release --locked
```

使用 just 时先运行 `just api-setup` 安装开发工具依赖，再运行 `just ci`，包含样本重建、离线接口契约校验（Python 3.12+）；C/Dart 实调和 iOS 交叉检查另有任务。默认 Cargo 测试直接读取已包含的正确性样本，不需要 Python、Flutter 或 Apple SDK。

清单最低 Rust 版本改为 **1.88**：当前锁定的 zip、lopdf、weezl 已要求 1.88，原 1.85 声明不能满足完整依赖。最低版本由独立 CI job 校验，已实测环境与结果见 [状态记录](status.md)。

## 可复现样本

```sh
python3 tools/make_corpus.py --check
python3 -m unittest discover -s tools -p 'test_*.py'
python3 tools/make_corpus.py --profile performance
```

也可运行 `just corpus-check`、`just corpus-performance 64`。输入大小、字符单位、内容校验值与样本限制见 [样本说明](../examples/data/corpus/README.md)。性能输入默认写到 target；生成输入不等同测量性能。

## 基准工具

```sh
python3 tools/bench.py
python3 tools/bench.py --clients rust,ctypes,dart,dart-worker
```

工具将编译和运行分开，输出带原始耗时、百分位、吞吐和 RSS 的 JSON/Markdown。`just bench` 测 Rust/C ABI，`just bench-all` 加测 Dart，`just bench-performance 64` 先生成输入再测。参数、缓存条件、文件系统与内存范围见 [测量说明](benchmarks/README.md)。

## C 与 Dart

参见 [示例说明](../examples/README.md)。C 需要 CMake 和 C 编译器；Dart 需要 Dart SDK 及 ffi 包。

```sh
just c-example
just ffi-smoke
just dart-example
```

宿主动态库分别为 Linux `libperch_ffi.so`、macOS `libperch_ffi.dylib`、Windows `perch_ffi.dll`。Dart 桌面调用应显式传 libraryPath；iOS 静态链接后使用进程符号。

## 完整接口契约

```sh
just api-setup
just api-check
just api-replay
```

离线检查覆盖 31 个方法的生成文档、Draft 2020-12 Schema、请求/响应样例和规范拒绝用例。真实重放编译持久 C 与 Dart AOT 客户端，以相同序列验证成功结果、错误码、版本协商、句柄/配额、EOF、管线与同步；详情见 [接口工具说明](../api/README.md)。只跑离线检查不需要 C/Dart 工具链。

## iOS 核心交叉编译

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
rustup component add llvm-tools-preview
just check-ios
just check-ios-sim
just ios-symbols
```

Linux 可以检查 Rust target 并打包 iOS staticlib。用 cargo rustc 覆盖 crate-type 为 staticlib，避免同时链接 cdylib。符号检查优先使用 Rust 工具链配套的 llvm-nm，避免系统 LLVM 无法读取较新 Rust bitcode；也可通过 PERCH_NM_BIN 显式指定兼容版本。

这些检查不包含 App 链接、Readium、真机签名或渲染。旧项目的模拟器验证记录不能当作新目录已验证。

## Flutter 适配器

[integrations/flutter/perch_core_ffi](../integrations/flutter/perch_core_ffi/README.md) 只负责 iOS Rust 静态库编译和 force_load，不提供 UI 或渲染器。移动端使用方需要自己的平台配置。

## CI

- `core-host`：Linux/macOS/Windows 上样本重建、Python CLI/统计与离线 API 检查、格式、Clippy、全量测试、示例、FFI 构建和基准工具冒烟；Linux/macOS 运行 C 示例及完整调用序列。
- `msrv`：Rust 1.88 的全量 target/feature 编译检查。
- `ios-cross`：Linux 上 iOS 真机/模拟器交叉检查和静态库符号检查。
- `dart-ffi`：Linux 上 Dart 绑定分析、127 项真实 FFI 冒烟、完整 C/Dart AOT 契约一致性、最小 TXT 示例和 Dart/AOT worker 基准冒烟。

CI 仅检验核心与绑定，未设置 Flutter App 构建、真机或发布到包仓库的自动任务。
