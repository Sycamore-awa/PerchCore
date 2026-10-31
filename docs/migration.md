# 从 PerchReader 迁移

2026-10-31 将项目重点转向通用电子书核心。现有 GitHub 仓库已重命名为 PerchCore，继续使用原提交历史；不执行 history rewrite 或 force push。

提取依据为 PerchReader 本地 HEAD `8e3327938334a345f979adbd4865790cba29a32c` 与当时未提交的核心/绑定改动。提取提交将一并保存这些当前实现，不会只提取旧 HEAD 的代码。

| 原路径 | 新路径 |
|---|---|
| core/Cargo.toml、Cargo.lock、rust-toolchain.toml | 仓库根目录 |
| core/perch-* | crates/perch-* |
| dart/perch_core | bindings/dart/perch_core |
| plugins/perch_core_ffi | integrations/flutter/perch_core_ffi |
| tools/make_spike_epub.py | tools/make_sample_epub.py |
| app/、UI 文档和截图 | 当前核心仓库不包含；本地旧目录保留 |

旧 PerchReader 本地目录保留原工作区及未提交内容，作为保存副本，不再作为新核心的维护入口。其远端后续与重命名后的地址对齐。

## 保持兼容的部分

- 既有 crate 名称和 0.1.0 版本。
- `perch_call`、`perch_string_free` 与方法名。
- 动态/静态库文件名 libperch_ffi / perch_ffi。
- Locator JSON 内的 perch 扩展和字符单位。
- Dart 包名 perch_core 与 iOS 插件名 perch_core_ffi。

由于锁定依赖已要求 Rust 1.88，清单最低工具链声明从 1.85 修正为 1.88。新统一入口 perch-core 不替换旧模块接口。

## 旧 App 接入新核心

旧 App 如需继续验证，可改为指向同级 PerchCore 的绑定与适配器路径：

```yaml
perch_core:
  path: ../../PerchCore/bindings/dart/perch_core
perch_core_ffi:
  path: ../../PerchCore/integrations/flutter/perch_core_ffi
```

iOS 适配器默认从自身位置寻找新仓库根目录，也支持 PERCH_CORE_DIR。旧 App 测试中的原 core/target 路径需要显式设置 PERCH_LIBRARY_PATH 或更新为新 target 路径。本次不将旧 App 发布为示例。
