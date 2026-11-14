# BASE-02 测量工具与结果

[tools/bench.py](../../tools/bench.py) 编译并调用独立的 [Rust worker](../../crates/perch-core/examples/bench_core.rs)、Python ctypes C ABI 客户端和 [Dart AOT worker](../../bindings/dart/perch_core/tool/bench_ffi.dart)。工具逐项输出原始耗时、统计、吞吐、进程峰值内存、样本校验值及环境；耗时和内存均来自实际运行。

## 运行

需要 Rust stable、Python 3.12+；Dart 客户端另需 Dart SDK。本地结果只代表当前宿主，平台支持和设备性能仍按各自证据判断。

```sh
# 小规模正确性样本，默认测 Rust + 真实 C ABI；30 次测量、3 次预热。
python3 tools/bench.py --output target/base02/correctness.json

# 原生、C ABI、Dart 同步与常驻 worker 一起测。
python3 tools/bench.py --clients rust,ctypes,dart,dart-worker --output target/base02/all.json

# 生成输入在计时之外；64 倍长行约 144 万字符。
python3 tools/make_corpus.py --profile performance --scale 64 --output target/corpus-64
python3 tools/bench.py --corpus target/corpus-64 --clients rust,ctypes,dart,dart-worker --output target/base02/performance.json

# 限定样本和阶段。query.hit 与 book.search 是不同入口。
python3 tools/bench.py --samples epub2,long-line --stages open,index,query.hit,book.search --iterations 30
```

`--samples`、`--stages` 和 `--clients` 使用逗号分隔。`--iterations` 为正整数，`--warmups` 可为零；CI 的少量重复只验证工具和正确性，不设耗时阈值。`--timeout-seconds` 默认 120，限制每个案例；失败案例在报告中标为 failed，工具最终返回非零退出码。

## 真实阅读器大 TXT 冒烟

[2026-11-14 Linux 报告](2026-11-14-linux-large-txt.md) 由同级 `PerchReader/tool/bench_large_txt.dart` 通过公开 Dart worker/service API 执行，覆盖指纹、pipeline 导入、worker 重启、Ready 恢复、书籍加载、高频词限量搜索、TXT 打开与首页切片。它是单机预热冒烟，不代替 BASE-02 的隔离 worker 分布。

```sh
cargo build -p perch-ffi --release
cd ../PerchReader
dart tool/bench_large_txt.dart 10 ../PerchCore/target/release/libperch_ffi.so
# 扩展到 100 MiB；默认仍只索引前 5,000,000 字符
dart tool/bench_large_txt.dart 100 ../PerchCore/target/release/libperch_ffi.so
```

首次编译和依赖缓存单独记录：

```sh
# clean-target 必须为新目录或空目录；不会删除已有构建产物。
python3 tools/bench.py --clean-build --target-dir target/base02/clean-target --clients rust,ctypes,dart,dart-worker

# 已编译入口可重复使用，报告记录库和可执行文件 SHA-256。
python3 tools/bench.py --skip-build --target-dir target/base02/clean-target
```

普通运行允许 Cargo/Dart 缓存，报告中的 build 日志与耗时明确标为缓存可复用。`--clean-build` 使用空 Cargo target，`first_clean_build_elapsed_ns` 是记录的构建步骤耗时之和；依赖下载缓存和 Dart pub 缓存仍可能已存在，不代表完全冷的工具链安装或网络下载。`--skip-build` 不提供本次编译耗时。

## 各阶段计时范围

| 客户端 / 阶段 | 计时工作 | 计时前准备 / 计时后处理 |
|---|---|---|
| Rust register | 打开源文件，读首尾各最多 64 KiB，算 Fingerprint，为一本候选生成登记计划 | 不复制原书、不写外部书库数据库；每次均使用空去重集合 |
| Rust open / TXT | 路径读取、编码探测/解码、字符与行索引 | 预期解码文本提前读入；逐字节结果校验在计时后 |
| Rust open / EPUB | 打开 ZIP、读取 container/OPF、解析 Package | 不抽正文、不建全文索引 |
| Rust open / PDF | 文件读取与结构解析 | 校验三页、两级目录；不测渲染或正文 |
| Rust decode | 内存字节编码探测与解码 | 源字节提前读入，不建字符/行索引 |
| Rust refine / extract | 元数据精修 / 正文抽取 | 源字节与预期索引文本提前读入 |
| Rust index | 正文复制、分词、倒排构建 | BookText 提前抽取；字符/内容一致性校验在计时后 |
| Rust query.hit / query.miss | 已建索引上的字面量查询，含全部 Hit 和 snippet 构造 | 命中查询取 manifest 的首条；缺失查询固定为「未出现的检索词」；校验全部字符范围 |
| Rust save | index.bin 序列化、写入并重命名；text.json 序列化、写入并重命名 | 正文和索引提前构建；匹配当前存储方式，不做 fsync |
| Rust reopen | 读取/反序列化 index.bin 与 text.json | 两文件提前落盘；计时后核对正文一致性 |
| Rust slice | TxtDocument 对末尾至多 800 scalar 的借用切片 | 文档提前打开；不分配跨语言结果 |
| Rust pipeline | PathStore 创建、Registered 状态落盘、调度精修/索引、产物与 Ready 状态落盘 | UUID 固定，每次使用独立子目录；产物重开校验在计时后 |
| ctypes book.open / jobs.refine / txt.analyze | Python bytes.hex、JSON 编码、C ABI、返回复制与 JSON 解码、Rust 字符串释放 | 原书字节提前读入；打开的句柄在计时后关闭 |
| ctypes book.load / book.search | 产物路径加载 / 对已打开句柄搜索，含 JSON 传输 | 产物或搜索句柄提前准备；search limit=20，核心达限后停止扫描后续分片 |
| ctypes txt.open / txt.slice / pdf.analyze | 路径打开 / 末尾窗口切片 / PDF 路径解析，含 JSON 传输 | slice 句柄提前打开；校验和句柄关闭在计时后 |
| ctypes txt.slice.concurrent | 向两个工作线程提交同一句柄的切片并等待二者完成 | 两个请求为一轮；另记录每个请求自身耗时；轮耗时包含线程池调度 |
| Dart book.open / txt.analyze | 调用公共 openBook / analyzeText，含绑定内的十六进制编码、JSON 和 FFI | AOT 编译、库加载、读源文件在计时外；关闭在计时后 |
| Dart txt.open / txt.slice / book.search | 公共 invoke 方法的同步调用与结果解码 | 句柄预先准备；坐标校验按 runes/scalar，避免 UTF-16 误差 |
| Dart worker txt.slice / version | 主 isolate 发出请求，到常驻 PerchWorker 返回解码结果 | worker 启动和退出在计时外，含消息传递与排队 |
| ctypes / Dart / worker version | 小结果的真实跨语言调用 | 用于观察调用开销；不代替书籍处理测量 |

PDF 仅有登记和结构打开，不进入 EPUB/TXT 精修、正文索引或管线。管线案例从已有源路径开始，包含 Registered → Ready，不包含使用方的原文件复制和数据库事务。各阶段计时范围不同，不能把分项 p50 直接相加当成端到端延迟。

固定 `max_chars=5,000,000`、`chunk_chars=4096`、`slice_chars=800`。发生抽取截断时保留 `stats.truncated` 和实际字符数；搜索预期只包含已抽取范围。默认 256 倍 long-line 超过上限，适合压力检查；完整基线使用 64 倍输入。基准不修改 SDK 限额。

## 缓存、存储和内存

输入生成、全内容校验、编译、进程/库加载与各阶段准备均在操作计时外。预热和多次操作复用同一输入；系统页缓存不驱逐，因此缓存状态明确记录为 **未控制的页缓存，含准备/预热访问**，没有冷缓存数据。`warmups=0` 也不能证明冷缓存。

产物默认写到 target-dir/base02/scratch，每个案例用临时子目录，结束后清理。可以通过 `--scratch-root` 选择文件系统；报告记录输入与产物路径的 Linux 挂载类型、设备和选项。在 tmpfs 上的写入、缓存写入和未做 fsync 的延迟都不能作为持久化磁盘吞吐承诺。

Linux worker 读取自身 `/proc/self/status` 的 **VmHWM**，得到 exec 后该进程的真实 RSS 高水位，包含运行时、输入/预期文本、准备、预热、操作、校验和清理。它不是某一行代码的分配量或 SDK 的独占内存，也不能通过减去另一进程的 RSS 推算。独立进程避免将前一个案例的高水位带到下一案例；不用父进程的子进程高水位替代被测地址空间。

Rust/Dart 在 Linux 以外尚未实现原生 RSS 采集，返回 null 和原因；ctypes 在 macOS 使用自身 getrusage，Windows 无 resource 模块时标记 unavailable。未测平台、移动设备、功耗和冷缓存不填估算值。并发切片只说明两个提交请求的本轮成本；没有保证短调用在核心内重叠，也不证明长管线任务不会阻塞读取。

## 报告字段

输出 JSON 和同名 Markdown 表格。JSON `version=1` 是报告格式版本。

| 字段 | 含义 |
|---|---|
| environment | CPU、逻辑核与可用 affinity、物理内存、cgroup 限额、OS/架构、工具链、CPU governor、运行前后负载、Git 提交与 dirty 状态 |
| corpus | profile、scale、manifest SHA-256、各样本的完整大小/编码/字符/指纹/预期查询 |
| sources / artifacts | 测量入口、Cargo.lock 和被测可执行文件/动态库的完整 SHA-256 |
| builds / first_clean_build_elapsed_ns | 构建命令、单项耗时、日志、缓存条件；与操作耗时分开 |
| measurement | 重复数、预热数、计时器、限额、文件系统、写入持久性和超时 |
| cases[].durations_ns | 每次操作的原始纳秒数据，保持运行顺序 |
| summary | nearest-rank p50/p90/p99、min/max/mean、p50 操作吞吐和可定义的 MiB/s |
| stats | 实际字符数、命中数、截断、索引规模、文件体积或 C ABI 请求/响应字节等 |
| peak_rss | 真实字节数、采集方法与范围；无法取得时为 null 并说明原因 |
| process_elapsed_ns | 包含启动、准备、预热、校验与结束的整个案例墙钟时间，不是操作延迟 |
| request_durations_ns / request_summary | ctypes 两线程案例每个请求的耗时，含自身 JSON/FFI 工作，不含提交前的队列等待 |
| unavailable / status / error | 未测指标和失败原因；失败不得解释为通过 |

MiB/s 用清楚的逻辑工作字节计算：登记为首尾窗口总字节，索引为索引输入 UTF-8 字节，save/reopen 为产物字节，切片为窗口/JSON 输出，其余完整处理阶段为原书字节。它不表示实际块设备 I/O；查询和 version 只报告操作吞吐。并发切片吞吐每轮按两个请求计算。

30 个重复的 p99 为观测最大值；小样本不提供稳定尾延迟保证。纳秒计时器、调度和分配器会影响微小操作，报告不扣除估算开销。后续性能门槛应结合更大样本、重复运行、真实设备和已确认预算。

## 本地基线

[2026-11-01 Linux 基线](2026-11-01-linux.md) 记录本地结果和解读，附两套 194 案例主报告和超限报告。CI 仅运行小规模冒烟与正确性校验，远端机器的耗时不自动成为发布性能门槛。当前开发队列见 [路线图](../roadmap.md)，保留现有 ABI、存储格式和资源限额。

## LOC-01 源映射测量

[tools/bench_source_map.py](../../tools/bench_source_map.py) 通过同一 API v1.4 release 库测 TXT/EPUB 的映射产物增量、路径打开、无映射/带映射重开和 book.locate。`just bench-source-map` 构建库后测正确性集；完整的 [Linux LOC-01 报告](2026-11-13-linux-source-map.md) 包含两套样本的 56 项测量，可按以下命令复现：

```sh
cargo build -p perch-ffi --release --locked
python3 tools/make_corpus.py --profile performance --scale 64 --output target/loc01/performance-corpus
python3 tools/bench_source_map.py \
  --corpus examples/data/corpus \
  --corpus target/loc01/performance-corpus \
  --output docs/benchmarks/2026-11-13-linux-source-map.json
```

默认每项 30 次计时、2 次预热；`--samples` 可选 indexed 样本，`--iterations` / `--warmups` 可调。生成输入、PathStore 产物准备、库加载和定位句柄/查询准备在计时外；每轮计时包含 JSON/C ABI、文件读取及核心工作，关闭和结果校验在计时后。

重开对照的 index 与 body 完全相同，旧形状仅省略可选 sourceMap，两个分支都运行当前实现。路径打开只测新实现，不能解释为改动前后抽取耗时对比。JSON 保留原始耗时、源/入口/库 SHA-256、样本规模、区间数和产物大小。

每样本/方法使用新的 exec 进程。Linux 保留准备前后、每轮操作后及清理后的 VmRSS，并保留最终 VmHWM 原值；`maximum_recorded_rss` 取所有这些记录的最大值，Markdown 对应“最大记录 RSS”。Linux 的近似/异步计数可能让最终 VmHWM 小于先前 VmRSS，采样也不保证全部瞬时峰值。RSS 包含 Python/Rust、准备和校验，不能把进程差值解释为 SDK 独占分配量。系统页缓存未控制，本报告不代表冷缓存或设备预算。

## STORE-01 恢复冷启动测量

[tools/bench_recovery.py](../../tools/bench_recovery.py) 用真实管线分别生成 TXT 和 EPUB Ready 产物，再组成不同规模的书库。每次测量都启动全新的 release Rust worker，只计时 `PathStore::new`，进程启动、产物复制与结果校验均不在计时区间。

```sh
# 默认：小产物 1/10/100/1000 本，scale=64 产物 1/10/100 本
just bench-recovery

# 复现归档的 Linux 报告
python3 tools/bench_recovery.py \
  --output docs/benchmarks/2026-11-14-linux-recovery.json

# 小规模工具/正确性检查
python3 tools/bench_recovery.py \
  --counts 1,2 --large-counts 1 --iterations 1 --warmups 0 \
  --performance-scale 2 --output target/store01/smoke.json
```

[当前 Linux 报告](2026-11-14-linux-recovery.md) 覆盖健康书库与单本 `index.bin` 损坏。JSON 保留每轮纳秒耗时、进程墙钟、VmRSS/VmHWM、产物字节数、恢复状态计数、环境与源码/可执行文件 SHA-256。默认每项 20 次测量、2 次预热。系统页缓存没有被驱逐；这是“全新进程的首次恢复”，不是冷块设备 I/O。RSS 是 worker 整个进程的记录，不是 SDK 独占分配。
