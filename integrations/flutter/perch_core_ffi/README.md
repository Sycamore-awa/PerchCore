# iOS 静态链接适配器

该 Flutter 插件只将 perch-ffi 编译为 iOS 静态库，并用 force_load 保留 C ABI 导出符号。它不包含 UI、MethodChannel 业务接口或渲染器。

使用方在自己的 Flutter App pubspec 中以路径依赖接入本目录，配合纯 Dart perch_core 绑定。需要 macOS、Xcode、Rust 和 CocoaPods；当前插件部署目标为 iOS 18。

build_rust.sh 默认从 integrations/flutter/perch_core_ffi/ios 向上寻找四级仓库根目录。可用 PERCH_CORE_DIR 指定另一份 Rust workspace；CARGO_TARGET_DIR 可覆盖构建产物目录。

当前保留已使用过的静态链接流程，但新目录布局的 App 编译和真机行为尚需使用方验收。它不提供独立 pub.dev 发布或 Android 构建适配器。

许可证为 [MIT](../../../LICENSE)。
