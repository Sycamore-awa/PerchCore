# 状态与计划

## 2026-10-31：独立核心提取

已从 PerchReader 当前工作区提取核心，新增统一 Rust 入口、C 头文件、最小 Rust/C/Dart 示例、MIT 许可证和独立 CI。保留 crate 名称、C 导出符号、JSON 方法名及 Locator 的 perch 扩展。

当前文件树不包含原 App、UI 截图或背景资产。原项目的提交历史继续保留。

本次目标为独立构建和复用；未同时重写并发模型、文本存储或渲染器。当前约束见 [性能与限制](performance.md)。

## 本次验证

环境：Linux x86_64、Rust 1.99.0、Dart 3.13.5。以下在独立目录执行，未依赖旧 App 或旧 core 源码。

| 检查 | 本地结果 |
|---|---|
| Rust 全量 + 全 feature + 文档测试 | 445 passed / 1 ignored；忽略项为手动性能基准 |
| cargo fmt 与 Clippy | 通过，Clippy 使用 -D warnings |
| Dart 静态分析 | 无问题 |
| 真实 Dart ↔ Rust FFI | 127 passed / 0 failed |
| C 示例 | 3/3：版本、业务错误、中文 Locator |
| 最小 Rust 示例 | TXT 和生成的 EPUB 解析/搜索通过 |
| 最小 Dart 示例 | TXT worker 打开、切片、释放和退出通过 |
| iOS Rust 交叉检查 | arm64 真机、arm64/x86_64 模拟器均通过 |
| iOS 静态库 | 编译通过，两个公共 C ABI 全局符号均存在 |
| 文档与脚本 | 本地链接、shell 语法、just 入口检查通过 |

本地未验证 Windows/macOS 执行、Rust 1.88、App 链接或真机表现。前两项由新 CI 检查；平台结果以对应运行记录为准。

## 后续目标与任务

详细计划集中维护在 [开发路线与任务清单](roadmap.md)，以任务 ID 记录完成状态和依赖，避免在多份文档重复维护清单。

下一批为 **BASE-01 → BASE-02 → API-01**：可复现样本、性能/内存基线、完整接口契约。之后按 M2 并发隔离、M3 大文件与内存、M4 定位和可靠性、M5 接入与发布推进；重要方案实施前确认。

路线与验收标准已写入文档，尚未开始这些新增任务。现有实现和验证继续以下方记录为准。

## 远端 CI 验证

[Core CI run 41193897127](https://github.com/Sycamore-awa/PerchCore/actions/runs/41193897127) 对代码提交 `f1a99b571abe1223b8bea3d0aa71e963519784e2` 的六个 job 全部通过：

- Linux、macOS、Windows Rust 格式、Clippy、测试、最小客户端和 FFI 库构建。
- Linux/macOS 的真实 C ABI 示例。
- Rust 1.88.0 全 target/feature 编译检查。
- iOS arm64 真机与两种模拟器 Rust target、静态库及公共符号。
- Dart 静态分析、127 项真实 FFI 冒烟与最小 TXT worker 示例。

首次 iOS 符号检查因 runner 的系统 LLVM 21 与当前 Rust LLVM 23 不匹配失败；已改用 Rust 工具链配套的 llvm-tools-preview，并补上错误诊断，重跑后通过。

这证明独立核心和上述绑定/构建路径可运行，不包含 iOS App 链接、阅读渲染、真机性能或发布包验收。
