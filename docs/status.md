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

## 后续优先级

1. 完善 API schema、错误码和版本兼容策略。
2. 把 FFI 全局状态改成可配置独立上下文，并缩短锁持有时间。
3. 提供大文件缓冲区/流式入口，测并控制 TXT 与索引峰值内存。
4. 用第二种技术栈的最小调用方检验接口通用性，补 Swift/Kotlin 接入。
5. 建立可复现的性能样本、损坏输入测试和跨平台发布产物。

PDF 渲染、分页排版、应用书库、选区 UI、云服务和 DRM 不属于此次独立核心的已有交付能力。
