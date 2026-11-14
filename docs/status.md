# 状态与计划

## 2026-11-14：阅读器宿主流程联调

当前机器上没有旧 PerchReader App 工作区，无法直接实施 Flutter UI、EPUB DOM 选区或真机页面跳转。为了继续推进可在本仓库验证的集成边界，新增 [阅读器宿主流程](reader-integration.md) 和 `example/reader_flow.dart`。

该示例只使用公开 `PerchServices` API，完成路径导入、refine/index、显式关闭、模拟 App 重启、Ready 恢复、落盘索引加载、搜索、命中回源和阅读切片。本地以 `sample.txt` 查询“文字”，恢复后得到源 scalar 区间 7..9、UTF-8 字节区间 17..23，`txt.slice` 验证原文为“文字”。

`just reader-flow` 可独立重放；`just ffi-smoke` 现也运行这条宿主链路。Dart analyze 通过，示例实调通过。下一个不可在本仓库内替代的环节是将 TXT scalar 坐标或 EPUB href/nodePath/UTF-16 坐标交给真实阅读视图；需要可写的 App 工作区才能继续。

## 2026-11-14：STORE-01 产物恢复与迁移

已完成 [STORE-01 恢复规则](storage.md#store-01-恢复规则)。PathStore 打开时现在按书审计 `state.json`、`refinement.json`、`index.bin`、`text.json` 和 sourceMap。单书状态缺失、损坏或不可读会以 `STORE_STATE_*` 原因降级到 registered，不阻断其他书；indexing/ready 缺失有效 refinement 时从阶段二重做。

Ready 只在索引可读、正文可读、两者正文一致且 sourceMap v1 有效时保留。索引/正文缺失、损坏、版本不支持、正文错配或旧正文缺映射时，该书在内存中降级为 indexing，通过既有 `pipeline.enqueue` 重新提供源路径后成组重建 index/text/sourceMap。打开审计不删除旧产物；源文件缺失或无权时保留它们，并进入既有失败、持久化次数、退避和耗尽流程。

写入顺序保持“产物先成功、检查点最后提交”，中断后最多重做幂等阶段，不会把半成品当成 Ready。多进程共用同一产物目录仍不支持；本项明确继续使用单管线/单拥有者规则，不增加无法可靠判定进程存活的锁文件。

JSON API 保持 v1.4 和 35 个方法，既有 recovered/state/reason 形状已能表达恢复计划，无需新增契约。当前序列追加 5 步 Ready 索引损坏、降级、重入队和重建场景，累计 261 组；v1.2/v1.3 的 161/189 组冻结夹具无漂移。

| 检查 | 本地 Linux 结果 |
|---|---|
| Rust workspace / all-features / 文档测试 | 497 passed / 1 ignored；忽略项为手动登记基准 |
| Rust 格式 / 全 target、feature Clippy | 通过，Clippy 使用 `-D warnings` |
| PathStore / FFI 恢复回归 | perch-jobs 67 passed；perch-ffi 71 passed |
| API 生成 / 离线 Schema / 冻结兼容 | 35 方法、3 个当前 + 9 个冻结 Schema、261 组当前及 161/189 组冻结夹具通过 |
| API 工具回归 / 真实 C 与 Dart AOT 重放 | 8 passed；两个客户端各 261 次，全部一致 |
| Dart 分析 / 真实 FFI smoke | 通过；172 passed / 0 failed |

当前改动保留在工作区，未提交。`pipeline.open` 的 Ready 产物审计成本与已存书籍的 index/text 总量成比例，本轮未测大书库冷启；如真实阅读器暴露瓶颈，再以代际 manifest 或延迟审计单独优化。IO-01、LOC-01 和 STORE-01 的主动队列已全部完成；下一步回到真实阅读器联调。

## 2026-11-13：LOC-01 搜索命中回源文本

已完成 [搜索命中回源文本](source-location.md)。抽取时同步生成连续区间映射，保留现有正文、归一化和截断结果。TXT 端点是原解码 scalar 与 UTF-8 字节位置；批准计划中的 CRLF/空行示例已验证，索引偏移 3 对应源字符偏移 9、源字节偏移 11。EPUB 保留各端点的 href、局部文本节点路径和独立 scalar/UTF-16 偏移，人工分隔符向内吸附，只含合成内容时明确无法定位。

Rust 新增 MappedBookText 与抽取/定位/持久化入口，原 BookText 字段和公开函数签名保留。book.open 与 PathStore 导入生成映射，text.json v1 的可选 sourceMap 使用独立版本 1，index.bin 保持 v1。BookStore 保存扩展提供默认正文保存实现，既有 Store 继续编译；自定义 Store 需要实现扩展才能落盘映射。缺失、不支持或损坏的映射分别用专用错误报告，旧产物仍可加载和搜索，已有 book.search 与 Locator 结果不变。

JSON API 升至 v1.4，共 35 个方法，新增 book.locate。v1.3 的三个 Schema 与 189 组请求/响应被冻结，v1.1/v1.2 快照继续保留；当前 256 组契约以真实 C/Dart 客户端逐字段重放。定位在根注册锁外使用只读 Arc 租约，沿用 context 所有权及关闭语义。Dart 增加类型化源端点、同步 locateBook 与 AsyncBookHandle.locate(hit)。

回归覆盖 CR/LF/CRLF、首尾/连续空行、GBK/Big5/BOM、emoji、组合字符、中英混排、源阅读切片、EPUB 实体/CDATA/重复节点/跨节点与跨章节、UTF-16 换算、截断/合成内容、非法区间、映射错误与句柄归属；落盘重开的坐标与直接打开一致，源文件变更后保留索引建立时的坐标。

| 检查 | 本地 Linux 结果 |
|---|---|
| Rust workspace / all-features / 文档测试 | 490 passed / 1 ignored；忽略项为手动登记基准 |
| Rust 格式 / 全 target、feature Clippy / release FFI 构建 | 通过，Clippy 使用 -D warnings |
| API 生成 / 离线 Schema / 冻结兼容 | 35 方法、3 个当前 + 9 个冻结 Schema、256 组当前及 161/189 组冻结夹具通过 |
| API 工具回归 / 真实 C 与 Dart AOT 重放 | 8 passed；两个客户端各 256 次，全部一致；C ABI 缓冲区等 CTest 4/4 通过 |
| Dart 格式 / analyze / 真实 FFI smoke | 通过；172 passed / 0 failed |
| 样本重建 / 标准库 Python 工具 | 24 个文件一致；9 passed |
| 源映射基准 / 溯源 / 文档 | 56 项测量完成；源/入口/库 SHA-256、RSS 记录与本地链接通过 |

[LOC-01 Linux 报告](benchmarks/2026-11-13-linux-source-map.md) 记录正确性集与 scale=64 性能集的 14 个样本、四种操作，每项 30 次计时与 2 次预热。64 倍长行的 text.json 从 3,014,779 增至 3,014,937 字节（增加 158），EPUB3 增加 401 字节；换行折叠较多的 UTF-8 样本增加 52,364 字节。长行路径打开 p50 为 33.5827ms，同一实现的无映射/带映射重开分别为 2.4777/3.5733ms，定位为 0.0059ms；带映射重开的整个进程最大记录 RSS 为 34.781MiB。

重开比较使用相同 index/body，仅省略 sourceMap；路径打开只有新实现的测量，不作为改动前后抽取比较。RSS 保留每轮 VmRSS 和最终 VmHWM，取最大记录值；Linux 计数存在近似/异步更新，本轮 13 项最终 VmHWM 小于先前采样值。进程包含 Python/Rust、准备和校验，差值不表示 SDK 独占分配量；系统页缓存未控制，移动设备仍需实测。

改动保留在工作区，未提交。源坐标针对索引建立时的源版本与解码，自动重建、内容变更后的模糊重锚及阅读器页面跳转/高亮由后续任务处理。LOC-01 已完成，下一项为 STORE-01 产物恢复。

## 2026-11-13：IO-01 文件路径输入

已完成 [文件路径输入](io.md)。`jobs.refine` 和 `book.open` 现在支持互斥的 path/bytes_hex 输入，路径分支分别用 `JOBS_IO` / `BOOK_OPEN_IO` 报告读取失败；旧字节参数、精修/索引结果、解析错误、配额和 context 所有权保留。新增 `txt.inspect` 按路径解码，只返回编码与字符/行统计，支持显式编码，不返回全文、不创建句柄。TXT 阅读仍使用 open/slice，PDF 沿用现有路径入口。

Dart 同步门面与 `PerchServices` 均增加 inspectTextPath/refineBookPath/openBookPath；异步路径索引由服务托管并在关闭时回收。文件在核心调用中同步读取，不保留文件描述符；源文件删除后已打开的索引仍可搜索。

JSON API 升至 v1.3，当前为 34 方法、189 组请求/响应。v1.1/v1.2 的 6 个 Schema 保留，v1.2 的 161 组夹具被冻结并继续离线校验；当前旧响应除 compatibility minor 外逐字段无漂移。真实 C/Dart 重放同时比较路径与字节输入的统计、精修和检索结果。

Rust 回归覆盖 EPUB 2/3、UTF-8/BOM/GBK/Big5、跨分片、中文路径、来源互斥、缺失文件、解析失败、截断及句柄归属。额外使用 15 MB / 5,000,001 scalar TXT，路径请求不构造 hex，索引返回 5,000,000 字符与 truncated:true，inspect 保留完整统计，两类结果均小于 256 字节。固定长行样本的 47,110 字节输入原需 94,236 字节 bytes_hex JSON，路径 JSON 为 84 字节，inspect 结果为 94 字节；此比较只描述传输大小，不作为耗时或峰值内存结论。

本地 Linux 验证：Rust workspace/all-features 共 471 passed / 1 ignored；格式与全 target/feature Clippy `-D warnings` 通过；34 方法、9 个当前/冻结 Schema、189 组当前契约、161 组旧夹具及 7 项 API 工具回归通过；C 与 Dart AOT 客户端各重放 189 次并一致，C 客户端输入/内存契约检查通过；Dart analyze、格式和 145 项真实 FFI smoke 通过；24 个样本重建与 9 项标准库工具测试通过。

当前改动保留在工作区，未提交。路径输入仍全量读文件/解码，全文索引仍驻留内存；不新增流式输入、文件预算或产物格式。新分支本次只验证 Linux，移动设备内存与性能尚未测量。下一项为 LOC-01 命中回源文本。

## 2026-11-13：路线收缩为阅读器刚需

项目近期目标已从“面向多个未知调用方的通用发布级 SDK”调整为“服务真实电子书阅读器的稳定核心”。已完成的样本、基线、API/兼容契约、独立 context、EPUB/TXT、检索和导入恢复能力继续保留。

当前主动队列只保留 IO-01、LOC-01 和 STORE-01，分别解决整本十六进制传输、检索命中回源文本和产物恢复；CTX-02 已完成。CTX-03、TXT/IDX/LIMIT 与损坏输入工作改为真实问题驱动；可配置配额、重锚契约扩展、Swift/Kotlin、完整平台矩阵与发布包明确暂缓。详细启动条件见 [阅读器核心路线](roadmap.md)。

## 2026-11-13：CTX-02 对象级并发

根注册表现在只管理 context、所有权、配额和短时句柄查找。book/TXT 以只读 `Arc` 租约共享，pipeline 以每对象 `Mutex` 串行；文件 I/O、解析、建索引、搜索、切片与 `pipeline.step` 不再持有根锁。释放大对象也移到根锁之外。

close/use 的竞争语义已固定：`*.close` 和 `context.close` 立即摘除句柄，之后的新请求失败；close 前已取得租约的在途请求允许完成。close 不等待或取消在途任务，该能力仍属于按需启动的 CTX-03。

新回归用人为占住 pipeline 对象锁来模拟可控长任务，证明另一 context 的 `txt.slice` 能在 pipeline 仍阻塞时完成，且 close 与在途 use 没有死锁、误关或悬挂。同机复跑 BASE-02 的 C ABI 双线程 `txt.slice.concurrent`，30 次的 p50 为 0.0347ms，对比 M1 报告 0.0377ms 无回退。本项不改 C/JSON/Dart 公开契约，下一项为 IO-01。

本地验证：Rust workspace/all-features 共 465 passed / 1 ignored；格式与全 target/feature Clippy `-D warnings` 通过；33 方法、6 个当前/冻结 Schema、161 组离线契约和 7 项工具回归通过；24 个样本重建与 9 项标准库工具测试通过；C ABI 4/4；Dart analyze 与 135 项 FFI smoke 通过。

## 2026-11-13：CTX-01 独立上下文

已按 [独立上下文设计](context.md) 完成 CTX-01。C ABI v1 保留 `perch_call` / `perch_string_free`，增量导出 `perch_context_call`；JSON API 升至 v1.2，新增 `context.open/close`。旧调用继续进入不可销毁的 legacy context，31 个既有业务方法的 JSON 参数与结果不变。

根注册表现在管理多个 ContextRegistry。对象句柄仍全进程递增且不复用，并记录所属 context；跨 context 误用返回 `CONTEXT_HANDLE_MISMATCH`。每个 context 独立拥有 8 book、4 TXT、1 pipeline 的固定配额；`context.close` 幂等并返回成组释放计数，销毁一个 context 不影响其他 context 或磁盘产物。

Dart 增加 `PerchContext`、`invokeInContext` 和 worker 上下文调用；每个 `PerchServices` 实例自动创建并销毁自己的 context。新符号延迟解析，因此低层 legacy API 仍可加载旧动态库。异常 isolate 退出仍不能自动清理，留给 CTX-03。

API v1.1 的 `urn:perchcore:api:v1:*` 三个 Schema 原样冻结；当前 v1.2 使用 `urn:perchcore:api:v1.2:*` 新快照。目录现为 33 个方法、161 步请求/响应样例，C 与 Dart AOT 持久客户端均覆盖显式 context、独立 pipeline 配额、跨 context 错误和成组销毁。

本地验证：Rust workspace/all-features 共 463 passed / 1 ignored；格式与全 target/feature Clippy `-D warnings` 通过；当前/冻结 6 个 Schema、33 方法和 161 组响应的离线契约及 7 项工具回归通过；真实 C 与 Dart AOT 客户端各重放 161 次并一致；C ABI 4/4；Dart analyze 与 135 项 FFI smoke 通过；24 个样本重建和 9 项标准库工具测试通过；iOS arm64 静态库的 3 个公共符号均已验证。

CTX-01 完成时只隔离所有权、配额和销毁，当时所有 context 仍共享根 Registry/Mutex；该限制已由上方 CTX-02 消除。

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

BASE-01/02、API-01/02、CTX-01/02、IO-01、LOC-01 和 STORE-01 已完成。当前回到真实阅读器联调，其他任务按路线图的启动条件推进；重要方案实施前确认。

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
