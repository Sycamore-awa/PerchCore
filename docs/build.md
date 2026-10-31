# 构建与验证

## Rust

在仓库根目录运行，工具链由根目录 rust-toolchain.toml 选择 stable：

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build -p perch-ffi --release --locked
```

使用 just 时可运行 `just ci`；C/Dart 示例和 iOS 交叉检查另有任务。默认 Rust 测试不需要 Flutter 或 Apple SDK。

清单最低 Rust 版本改为 **1.88**：当前锁定的 zip、lopdf、weezl 已要求 1.88，原 1.85 声明不能满足完整依赖。最低版本由独立 CI job 校验，已实测环境与结果见 [状态记录](status.md)。

## C 与 Dart

参见 [示例说明](../examples/README.md)。C 需要 CMake 和 C 编译器；Dart 需要 Dart SDK 及 ffi 包。

```sh
just c-example
just ffi-smoke
just dart-example
```

宿主动态库分别为 Linux `libperch_ffi.so`、macOS `libperch_ffi.dylib`、Windows `perch_ffi.dll`。Dart 桌面调用应显式传 libraryPath；iOS 静态链接后使用进程符号。

## iOS 核心交叉编译

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
just check-ios
just check-ios-sim
just ios-symbols
```

Linux 可以检查 Rust target 并打包 iOS staticlib。用 cargo rustc 覆盖 crate-type 为 staticlib，避免同时链接 cdylib。符号检查需要 llvm-nm。

这些检查不包含 App 链接、Readium、真机签名或渲染。旧项目的模拟器验证记录不能当作新目录已验证。

## Flutter 适配器

[integrations/flutter/perch_core_ffi](../integrations/flutter/perch_core_ffi/README.md) 只负责 iOS Rust 静态库编译和 force_load，不提供 UI 或渲染器。移动端使用方需要自己的平台配置。

## CI

- `core-host`：Linux/macOS/Windows 上格式、Clippy、全量测试、示例和 FFI 构建；Linux/macOS 运行 C 示例。
- `msrv`：Rust 1.88 的全量 target/feature 编译检查。
- `ios-cross`：Linux 上 iOS 真机/模拟器交叉检查和静态库符号检查。
- `dart-ffi`：Linux 上 Dart 绑定分析、127 项真实 FFI 冒烟和最小 TXT 示例。

CI 仅检验核心与绑定，未设置 Flutter App 构建、真机或发布到包仓库的自动任务。
