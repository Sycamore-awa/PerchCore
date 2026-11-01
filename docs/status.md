# 状态与计划

## 2026-11-01：API-02 版本与兼容策略

已完成 [版本与兼容策略](compatibility.md) 的实施。C 头文件现在声明 ABI v1；JSON API 为 v1.1，新增 `compatibility` 方法，而旧 `version` 结果保持不变；3 个 Draft 2020-12 Schema 改用 `urn:perchcore:api:v1:*` 标识。调用方可分别检测 ABI/API、Locator、index 和 text 的可读/写入版本。

新增 `examples/data/compatibility` 固定夹具和 Rust 集成回归，锁定 Locator v1、index v1 以及缺少 `version`/`format` 的 text v0 读取行为。Locator 未知版本返回 `MODEL_UNSUPPORTED_VERSION`；`text.json` 当前读 v0/v1、只写 v1，未知版本返回 `BOOK_TEXT_UNSUPPORTED_VERSION`；index v1 继续使用 `INDEX_UNSUPPORTED_VERSION`。版本不支持与损坏不再混为同一分支。

本地验证：全 workspace/all-features 共 457 passed / 1 ignored；兼容夹具 5/5；`cargo fmt --check` 与全 target/feature Clippy `-D warnings` 通过；31 方法/3 Schema/147 组响应的离线 API 契约及 7 项工具回归通过；真实 C 与 Dart AOT 客户端各重放 147 次，成功结果一致、错误码一致且诊断非空；Dart analyze 与 130 项 FFI smoke 通过；24 个样本文件重建校验和 9 项标准库工具测试通过。

持久化升级不做就地迁移：index/text 作为同一组从原书重建；原书缺失时保留旧产物并报告不支持。自动清理与重建编排留给 STORE-01。API-02 已完成，下一项为 CTX-01。

## 2026-11-01：API-01 完整接口与数据契约

已完成 API-01，交付 [30 方法参考](api-reference.md)、[机器可读目录与验证说明](../api/README.md)、Draft 2020-12 的 3 个 Schema、144 步请求/完整响应样例，以及持久 C 与 Dart AOT 客户端。方法参考、目录、Schema 和请求序列由 Python 标准库生成；jsonschema 4.25.1 仅作为隔离环境中的开发校验依赖。

逐项明确参数、必填/可空字段、默认值、结果分支、错误码、Unicode scalar/解码 UTF-8 字节单位、时间/页号、Rust 内存所有权、进程共享句柄与固定配额、重复关闭、worker 生命周期和目录副作用。实调确认单侧并发可为 0、txt.offset 不自动校验 Locator、无效高优先级锚点不继续回退、PDF 任务失败与顶层 PIPELINE_IO 分离等现有细节。

规范 Schema 与运行时宽松解析明确分开，包含离线包装与本地 URN 注册方式，不给现有 FFI 增加运行时严格校验。所有公开方法都有真实成功样例，错误、配额/关闭、GBK、中文/emoji/组合符、EOF/LF/CRLF、管线分支/恢复、索引正文不一致和同步分页/墓碑/一致性均有对应验证。

环境：Linux x86_64、Rust 1.99.0、Python 3.12.14（API 工具）/3.14.8（标准库工具）、Dart 3.13.5、GCC/CMake。

| 检查 | 本地结果 |
|---|---|
| 全部方法的真实 C 与 Dart AOT 调用 | 各 144 次，全部通过；成功数据一致、错误码一致且诊断非空 |
| Schema / 文档生成 / 方法覆盖 | 3 个 Draft 2020-12 Schema、30 方法、144 对请求与响应通过；生成文件无漂移 |
| API 工具回归 | 7 passed，含 26 组非法请求和 9 组非法响应子用例、离线引用不联网与结构/语义边界 |
| C 示例与输入缓冲区契约 | CTest 4/4；另覆盖 7 项字节长度、无效 UTF-8、嵌入 NUL 与 NULL/free 规则 |
| Rust 全 workspace / 全 feature / 文档测试 | 450 passed / 1 ignored；现有实现保持通过 |
| cargo fmt / Clippy / Dart analyze | 通过，Clippy 使用 -D warnings；新增 Dart 文件格式检查通过 |
| 样本重建 / 标准库 Python 检查 | 24 个样本文件一致，9 passed |
| 历史基准 / 文档 / just | 三套历史基准的测量源码 SHA-256 仍匹配；本地文档链接、just 入口通过 |

真实重放使用两个新进程和各自临时产物目录，固定时钟。错误 message 在响应样例中以占位诊断表示，recovered 按 book 排序比较；其余成功结果逐字段和记录值一致。C/Dart 普通检查不会更新样例；显式 record 也必须两端通过。

三平台离线契约、Linux/macOS C 实调、Linux C/Dart 完整一致性已加入 CI，尚未取得本批远端结果。当前改动未提交；不改变现有 C 导出、运行时默认配额、Locator 版本或存储格式。下一项为 API-02 版本与兼容方案，M1 尚未全部完成。

## 2026-11-01：BASE-02 本地基线与测量工具

已完成 BASE-02，交付 [测量说明](benchmarks/README.md)、[Linux 本地报告与解读](benchmarks/2026-11-01-linux.md)、Rust/Python C ABI/Dart AOT 测量入口、JSON/Markdown 原始报告以及 just/CI 命令。实现与数据仍在当前工作区；报告记录源码提交、dirty 状态和入口/产物 SHA-256。

覆盖登记、格式打开、TXT 解码、精修、抽取、索引、命中/缺失查询、保存/重开、末尾窗口切片、真实 PathStore 管线、C ABI 传输、Dart 同步/worker 调用与两线程切片。正确性集和 64 倍性能集各 194 个案例、每项 30 次计时与 3 次预热；全部通过。另有 8 项 256 倍长行超限案例、每项 3 次计时，均明确保留截断状态与已抽取范围。

环境：CachyOS Linux x86_64、Ryzen 5 9500F、约 15.25 GiB 内存、Btrfs/NVMe；Rust 1.99.0、Python 3.14.8、Dart 3.13.5 AOT。全新 Cargo target 的构建步骤共 14.452 秒，操作计时独立记录。Linux RSS 使用被测进程的 VmHWM，避免父进程启动高水位干扰；峰值仍包含运行时、准备与校验。缓存未驱逐，写入未做 fsync，设备/冷缓存/其他平台的性能数据未测。

| 检查 | 本地结果 |
|---|---|
| Rust 全 workspace / 全 feature / 文档测试 | 450 passed / 1 ignored；手动基准维持忽略 |
| cargo fmt / 全 target、feature Clippy | 通过，Clippy 使用 -D warnings |
| Python 统计与 CLI 检查 | Python 3.12 / 3.14 均 9 passed（含已有样本生成器 4 项） |
| Dart 静态分析与 AOT 构建 | 通过；同步/worker 的真实测量及结果校验通过 |
| 主基线与压力检查 | 388 个主案例 + 8 个超限案例通过；保留原始耗时、百分位、吞吐和 RSS |
| 数据溯源 / 文档 / just | 报告中的源码 SHA-256 与当前文件一致，本地链接与命令入口检查通过 |

约 144 万字符的 long-line 中，Rust 建索引 p50 为 30.633ms，Dart 全文分析为 84.367ms，800 字窗口切片为 0.0070ms。重复命中查询的摘要生成和 book.search 在限量前计算全部结果，也是已记录的后续优化方向。报告不作为跨设备性能承诺或新的默认资源预算。

三平台 Rust/C ABI 基准工具冒烟和 Linux Dart AOT/worker 冒烟已加入 CI，尚未取得本批远端执行结果。本记录完成时下一项为 API-01；最新进展见上方记录，M1 整个阶段尚未完成。

## 2026-11-01：BASE-01 可复现样本集

已完成 [BASE-01](roadmap.md) 的实现，交付 [样本说明与 manifest](../examples/data/corpus/README.md)、`tools/make_corpus.py`、真实 Rust 集成回归和生成器 CLI 检查。此记录对应当前工作区改动，尚未创建提交。

9 类自生成样本覆盖 EPUB 2/NCX、EPUB 3/NAV、UTF-8 与 BOM、GBK、Big5、emoji/组合附加符、跨章节/4096 字分片短语、22,530 字单行，以及带两级 Unicode 大纲的三页 PDF。原文件、解码文本、索引文本分别记录大小、scalar 字符数和完整 SHA-256。性能 profile 按显式倍数生成，PDF 固定为结构小样本；默认长行超过现有抽取上限，作为截断压力输入。

生成器固定 ZIP 元数据并使用 Stored；同时修复已有 EPUB 示例生成器的时间与平台属性不固定问题。Git attributes 保留夹具的原始字节；Linux/macOS/Windows CI 增加 Python 3.12 重建校验和 CLI 测试。Rust 的默认测试直接读取已包含的正确性样本，不需要先安装 Python。

本地环境：Linux x86_64、Rust 1.99.0、Python 3.12.14 / 3.14.8。

| 检查 | 本地结果 |
|---|---|
| Rust 全 workspace + 全 feature + 文档测试 | 450 passed / 1 ignored；新增 5 项真实样本集成回归 |
| cargo fmt 与全 target/feature Clippy | 通过，Clippy 使用 -D warnings |
| 正确性样本重新生成与逐字节校验 | 两个 Python 版本均得到相同的 24 个生成文件 |
| 默认 scale=256 性能样本生成与校验 | 通过；样本写入 target/performance-corpus |
| 生成器 CLI 检查 | 两个 Python 版本均 4 passed；覆盖损坏、缺失、多余文件、非法 scale 与 EPUB 固定 ZIP 元数据 |

BASE-01 本身不包含性能测量，后续 BASE-02 的证据见上方记录。新增 CI 的 macOS/Windows 执行结果尚未取得；不复用下方旧提交的成功记录作为本次验证。此项未变更 C ABI、Dart 或持久化格式。

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

BASE-01 可复现样本、BASE-02 Linux 本地基线与 API-01 完整接口契约已完成，下一项为 **API-02**：版本与兼容方案。之后按 M2 并发隔离、M3 大文件与内存、M4 定位和可靠性、M5 接入与发布推进；重要方案实施前确认。

各项状态以路线图和上方新增记录为准；下方 CI 记录描述独立核心提取时的验证。

## 远端 CI 验证

[Core CI run 41193897127](https://github.com/Sycamore-awa/PerchCore/actions/runs/41193897127) 对代码提交 `f1a99b571abe1223b8bea3d0aa71e963519784e2` 的六个 job 全部通过：

- Linux、macOS、Windows Rust 格式、Clippy、测试、最小客户端和 FFI 库构建。
- Linux/macOS 的真实 C ABI 示例。
- Rust 1.88.0 全 target/feature 编译检查。
- iOS arm64 真机与两种模拟器 Rust target、静态库及公共符号。
- Dart 静态分析、127 项真实 FFI 冒烟与最小 TXT worker 示例。

首次 iOS 符号检查因 runner 的系统 LLVM 21 与当前 Rust LLVM 23 不匹配失败；已改用 Rust 工具链配套的 llvm-tools-preview，并补上错误诊断，重跑后通过。

这证明独立核心和上述绑定/构建路径可运行，不包含 iOS App 链接、阅读渲染、真机性能或发布包验收。
