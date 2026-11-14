# PerchCore

[![Core CI](https://github.com/Sycamore-awa/PerchCore/actions/workflows/ci.yml/badge.svg)](https://github.com/Sycamore-awa/PerchCore/actions/workflows/ci.yml)

**使用 Rust 开发的跨平台电子书核心 SDK：解析、文本定位、中文全文检索和可恢复的导入任务。**

A cross-platform ebook processing SDK written in Rust, with a C ABI and Dart bindings.

PerchCore 从 PerchReader 提取，保留既有核心实现与 Git 历史。当前版本为 **0.1.0**，接口仍在完善。它可嵌入不同技术栈的阅读器；排版、渲染、书库数据库和平台生命周期由使用方实现。

## 能力

| 模块 | 已有能力 |
|---|---|
| EPUB | EPUB 2/3 ZIP、OPF、NAV/NCX、元数据、封面识别、正文抽取、CFI 与上下文重定位 |
| TXT | UTF-8/GBK/Big5 等编码探测与解码、字符/字节换算、行索引、按字符切片 |
| 搜索 | 无词典 CJK 分词、短语搜索、分片倒排、delta+varint 索引持久化 |
| 命中回源 | TXT 原解码字符/UTF-8 字节端点；EPUB href、文本节点路径与 scalar/UTF-16 端点 |
| 导入 | 文件采样指纹、登记/去重计划、优先级、配额、退避、恢复与重试 |
| FFI 生命周期 | 独立 context、句柄归属、按 context 固定配额、对象级并发与成组销毁 |
| PDF | 页数、大纲、元数据与页定位；不包含渲染或正文抽取 |
| 定位模型 | 与 Readium Locator 字段兼容，附加带版本的 TXT/PDF 扩展 |
| 可选 Rust 模块 | HLC/LWW 同步合并；卷页几何计算 |

Rust 调用方可使用统一入口 `perch-core`，也可直接依赖单个 crate。跨语言调用方使用 `perch-ffi` 的 C ABI；附带纯 Dart 绑定与一个 iOS 静态链接适配器。

## 快速开始

需要 Rust stable。默认构建和测试不需要 Flutter、Readium、Xcode 或服务器。

```sh
git clone https://github.com/Sycamore-awa/PerchCore.git
cd PerchCore
cargo test --workspace --locked
cargo run -p perch-core --example inspect -- examples/data/sample.txt 文字
```

最小 Rust 接入：

```toml
[dependencies]
perch-core = { path = "../PerchCore/crates/perch-core" }
```

```rust
use perch_core::{index::Index, txt::TxtDocument};

let document = TxtDocument::from_bytes("这里是中文文字。".as_bytes());
let index = Index::build(document.text(), 4096);
for hit in index.search("文字") {
    println!("{}: {}", hit.char_start, hit.snippet);
}
```

当前尚未发布到 crates.io 或 pub.dev，使用本地路径或固定 Git 提交接入。更多示例见 [examples](examples/README.md)。

## 项目结构

```text
crates/
  perch-core/        Rust 统一入口；附最小解析/搜索示例
  perch-model/       Book、Locator、Annotation
  perch-epub/        EPUB 容器、元数据、目录、正文、CFI
  perch-txt/         编码、字符索引、文本切片
  perch-index/       搜索与索引落盘
  perch-import/      指纹、去重和登记计划
  perch-pipeline/    调度策略
  perch-jobs/        导入执行和文件产物
  perch-pdf/         PDF 结构
  perch-sync/        合并算法
  perch-pagecurl/    卷页几何
  perch-ffi/         C ABI 和 JSON 分发
bindings/dart/       纯 Dart 同步绑定、worker、类型化服务
integrations/flutter/  iOS 静态链接适配器；无界面
include/             公共 C 头文件
api/                 完整方法目录、JSON Schema、可重放请求与响应样例
examples/            最小 Rust、C、Dart 命令行示例
docs/                架构、接口、存储、验证、性能与迁移记录
```

## 后续目标与任务

[阅读器核心路线](docs/roadmap.md) 是后续工作的入口。BASE-01/02、API-01/02、CTX-01/02、IO-01、LOC-01 与 STORE-01 已完成；当前已有可复现样本、Linux 基线、35 方法契约、[版本与兼容策略](docs/compatibility.md)、独立上下文、对象级并发、[文件路径输入](docs/io.md)、[搜索命中回源文本](docs/source-location.md)和[产物恢复规则](docs/storage.md#store-01-恢复规则)。[恢复冷启动基准](docs/benchmarks/2026-11-14-linux-recovery.md) 已量化 1000 本小书库与 300MiB 放大产物书库，尚无需立即引入 manifest/延迟审计。[阅读器宿主流程](docs/reader-integration.md) 已跑通导入、重启恢复、搜索、回源与 TXT/EPUB 坐标输出；下一步需要真实 App 工作区完成阅读视图跳转。

## 验证与性能

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build -p perch-ffi --release --locked
```

已有 Linux 上解析、索引、查询、存储和跨语言调用的可复现基线，以及[源映射尺寸、打开与定位测量](docs/benchmarks/2026-11-13-linux-source-map.md)，跨设备性能仍待测量。精修/索引/PDF 支持文件路径输入，TXT 统计可不传输全文；TXT 仍全量解码，旧十六进制接口保留。context 已隔离所有权和配额，耗时操作不再占用根注册锁，详见 [性能与限制](docs/performance.md)。

文档：[架构](docs/architecture.md) · [C ABI](docs/ffi.md) · [完整方法参考](docs/api-reference.md) · [Schema 与样例](api/README.md) · [版本与兼容方案](docs/compatibility.md) · [独立上下文设计](docs/context.md) · [存储契约](docs/storage.md) · [阅读器宿主联调](docs/reader-integration.md) · [构建与验证](docs/build.md) · [目标与任务](docs/roadmap.md) · [状态与计划](docs/status.md) · [迁移说明](docs/migration.md)。

## 许可证

[MIT](LICENSE)。仓库历史保留原项目代码；第三方依赖使用各自的许可证。
