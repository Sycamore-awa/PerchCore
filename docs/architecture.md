# 架构与边界

PerchCore 是进程内 SDK。Rust API 和 C ABI 共用算法模块，使用方选择自己的存储、渲染和任务运行环境。

```text
Rust 调用方 → perch-core 或单个 crate
C / Dart / 其他绑定 → perch-ffi → Rust 模块
使用方 → 自己的数据库、渲染器、平台权限与生命周期
```

## 模块

| crate | 职责 |
|---|---|
| perch-core | 重导出通用模块；`sync` / `pagecurl` 可选 |
| perch-model | Book、Locator、Annotation、ImportState、校验与序列化 |
| perch-epub | ZIP / OPF / NAV / NCX、封面、XHTML 索引、CFI 与文本重定位 |
| perch-txt | 编码探测、解码、字符抽样索引、行索引与切片 |
| perch-pdf | lopdf 页数、大纲和元数据解析 |
| perch-index | CJK/拉丁分词、分片倒排、短语查询与索引落盘 |
| perch-import | 文件指纹、去重和登记计划 |
| perch-pipeline | 优先级、配额、闸门、退避、恢复和重试策略 |
| perch-jobs | EPUB/TXT 精修、文本抽取、索引和命中定位；BookStore 与 PathStore |
| perch-sync | HLC、LWW、墓碑、增量合并；内存协议测试服务端 |
| perch-pagecurl | 卷页几何、完成和回弹判定 |
| perch-ffi | C ABI、JSON 分发、进程内句柄 |

Rust 算法层不依赖 Flutter、Readium 或系统 GUI。文件 I/O 由 TXT 打开、索引持久化和 PathStore 明确执行；库不自带书库数据库、网络传输、系统后台任务或服务器部署。

算法 crate 继承 `unsafe_code = "forbid"`。`perch-ffi` 单独处理 C 裸指针；可预期错误通过 Result 或 JSON 错误对象返回。禁止 unwrap/expect/panic 的 workspace lint 不构成无崩溃保证，第三方依赖、内存耗尽与错误的 C 指针仍需调用方和集成测试处理。

## Rust 接入

可以依赖统一入口，也可以只依赖 `perch-txt` 等 crate。

```toml
[dependencies]
perch-core = { git = "https://github.com/Sycamore-awa/PerchCore.git", features = ["sync"] }
```

生产使用应固定审核过的 `rev`。同一 workspace 的 crate 版本统一；0.1.x 接口可能调整。`perch-core` 默认排除同步与卷页依赖，C ABI 为保持兼容仍包含现有完整方法集合。

## 定位与字符单位

Locator 顶层字段与 Readium 对齐：`href`、`type`、`title`、`locations`、`text`。附加字段在 `perch` 内，并通过 `v` 标明版本；现有 JSON 键名保持不变。

| 格式 | 定位信息 |
|---|---|
| EPUB | href、CFI / CSS selector、章节进度和前后文 |
| TXT | `perch.txtOffset`，必要时字节偏移和 progression 回退 |
| PDF | `perch.pdfPage`，可选归一化矩形 |

TXT 字符单位为 **Unicode scalar**；Dart/Java 常用 UTF-16 code unit，原始文件又可能是 GBK。三者不可直接互换。TXT 字节偏移指解码后的 UTF-8 文本偏移，不应当作原编码文件偏移。PDF 页索引为 **0 基**。

Readium `partialCfi` 只包含第一个 `!` 右侧的片段。结构兼容并不保证渲染器能直接导航所有位置，使用方需要验证实际平台行为。

搜索用文本可能经过归一化；EPUB 命中当前只提供粗 Locator，TXT 索引偏移也不保证等同原阅读文本。调用方需校验命中原文并重锚。Rust 已有重定位算法，完整阅读器的选区、重排和高亮回放仍需集成。

## 执行与生命周期

Rust API 由调用方决定线程和对象生命周期。C ABI 的有状态方法目前共享一个全局 Registry/Mutex。配额为 8 个搜索句柄、4 个 TXT 句柄、1 个管线句柄，不能当作多租户服务。

Dart 同步调用由 `PerchCore` 封装，`PerchWorker` 在常驻 isolate 中按顺序执行；`PerchServices` 管理类型化句柄并在关闭时等待请求、释放句柄和 worker。worker 异常退出后遗留句柄不能通过新 worker 自动恢复。

同一句柄的并发 close/use 必须由使用方协调。阻塞任务虽然可以移出 UI 线程，阅读请求仍可能等待全局锁；相关改进见 [性能与限制](performance.md)。
