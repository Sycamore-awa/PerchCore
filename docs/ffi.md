# C ABI 与 JSON 契约

公共头文件为 [include/perch_core.h](../include/perch_core.h)，实际方法实现见 [perch-ffi](../crates/perch-ffi/src/lib.rs)。

全部 36 个方法的参数、返回字段、默认值、单位和错误码见 [完整方法参考](api-reference.md)。机器可读契约与重放说明见 [api](../api/README.md)，包括 Draft 2020-12 Schema 和真实 C/Dart 响应样例。

```c
char *perch_call(const uint8_t *method_ptr, size_t method_len,
                 const uint8_t *args_ptr, size_t args_len);
char *perch_context_call(uint64_t context,
                         const uint8_t *method_ptr, size_t method_len,
                         const uint8_t *args_ptr, size_t args_len);
void perch_string_free(char *ptr);
```

`perch_call` 使用不可销毁的 legacy 上下文。新调用方先用 `perch_call("context.open", "{}")` 取得 context，再把它作为 `perch_context_call` 的第一个参数；业务方法名和 args JSON 不变。`context.close` 仍通过 `perch_call` 调用，并一次释放该 context 的全部对象。

## 内存与错误

输入采用 UTF-8 字节和显式字节长度，不要求 NUL 结尾。指针非空时必须至少有对应长度的有效内存，且在同步调用期间保持有效；库不会保留输入。

任一输入指针为 NULL 或长度为 0 时，当前实现视其为空字符串；空 args（包括纯空白）视为 `{}`。空方法返回 `FFI_UNKNOWN_METHOD`。有效内存中的非法 UTF-8 分别返回 `FFI_BAD_METHOD_PTR` / `FFI_BAD_ARGS_PTR`。输入中的 NUL 也按长度参与解析，不能用 strlen 截断数据来代替实际字节长度。

两个调用入口的返回值都是 Rust 分配的 NUL 结尾 UTF-8 JSON，必须用 `perch_string_free` 释放一次。不得用 C free，不能重复释放。NULL 返回值表示调用失败；`perch_string_free(NULL)` 安全。

业务失败统一返回：

```json
{"error":{"code":"FFI_BAD_ARGS","message":"缺少字符串字段 path"}}
```

调用方先解析 error，再读取业务字段。错误码用于逻辑判断，message 用于说明。`version` 保持 `{"core":"0.1.0"}` 旧形状；`compatibility` 返回 C ABI v1、JSON API v1.5、Locator 及 index/text 的可读与写入版本。详细规则见 [版本与兼容策略](compatibility.md)。

调用错误与任务失败分开：pipeline.step 的 `failed` / `exhausted` 是正常结果，带 reason；存储失败返回顶层 `PIPELINE_IO`。Dart `invoke` 将顶层 error 转为 `PerchCoreException(code,message)`，并在 finally 中释放 Rust 返回字符串。

## 方法集合

| 方法组 | 方法 |
|---|---|
| 基础 | `version`、`compatibility`、`context.open/close`、`locator.validate`、`fingerprint` |
| 文本精修 | `jobs.refine` |
| 搜索 | `book.open/load/search/locate/close`、`index.search` |
| TXT | `txt.open/slice/locate/offset/line/close`、`txt.analyze/inspect` |
| PDF | `pdf.analyze` |
| 导入调度 | `pipeline.open/enqueue/step/status/progress/state/promote/retry/cancel/close` |
| 可选算法对应接口 | `curl`、`hlc.parse`、`sync.merge/delta` |

该列表、逐项参考与当前 v1.5 Schema 覆盖同一组方法，CI 检查生成文件和 Rust 分发集合无漂移；保留 v1.1/v1.2/v1.3/v1.4 Schema 与对应夹具冻结快照。Rust 统一入口的 optional features 不改变当前 `perch-ffi` 方法集合。

## 规范请求与实际解析

Schema 的 `{method,args}` / `{method,result}` 是离线校验包装；不要把包装整体传给 perch_call。Schema 要求规范对象和类型、拒绝未知字段。运行时不自动执行这些 Schema：多数未知字段忽略，许多可选标量的类型错误回退默认值；必填字段、模型反序列化和各方法语义规则仍可能返回错误。

共享 Locator Schema 检查 v1 结构；`locator.validate` 另行检查版本、锚点和进度，未知 `perch.v` 返回 `MODEL_UNSUPPORTED_VERSION`。`txt.offset`、同步 Change 一致性检查不全面调用 Locator/Annotation 的语义验证。需要语义有效定位时由调用方先调用 locator.validate。行为差异与空值规则见 [方法参考](api-reference.md)。

接口中的时间戳均按 UTC Unix epoch 毫秒表达，HLC counter 为计数器，curl_angle 为弧度。字符区间采用 Unicode scalar 的半开区间 `[start,end)`，不是 UTF-8 字节、Dart UTF-16 code unit 或用户可见字形数；组合字符和部分 emoji 会占多个 scalar。`txtByteOffset` 是解码文本的 UTF-8 字节偏移，不是 GBK 等源文件字节偏移。整数范围及调用语言限制见 [Schema 说明](../api/README.md)。

## TXT 读取示例

在同一进程连续调用：

```text
txt.open   {"path":"/absolute/path/book.txt"}
           → {"handle":0,"chars":1200,"lines":30,"encoding":"UTF-8",...}
txt.slice  {"handle":0,"start":0,"chars":120}
           → {"text":"...","start":0,"end":120,"chars":120,"total":1200}
txt.locate {"handle":0,"char":120}
           → {"locator":{...}}
txt.close  {"handle":0}
           → {"closed":true}
```

handle 值由运行时分配，包含 0，不能当作指针或布尔值。`start`/`char` 按 Unicode scalar 计数。每次 txt.slice 的 chars 上限为 20,000；窗口切片减少边界传输，但 txt.open 仍全量读取和解码。

## 搜索与持久化

`book.open` 接收 path 或 bytes_hex，抽正文并建索引。路径输入避免整本十六进制传输，建索引仍应放在后台。导入管线可生成 index.bin 与 text.json，再通过 book.load 按路径载入并反复 book.search。

book.load 会验证正文与索引一致。`book.locate {handle,char_start,char_end}` 将搜索命中的非空 scalar 区间映射到建索引时的源文本，返回 start/end 两个端点及 containsSynthetic。TXT 包含原解码 charOffset/UTF-8 byteOffset，EPUB 包含 href/nodePath/scalarOffset/utf16Offset；跨章节保留各端点的 href。人工分隔符的端点向内吸附，只含合成内容时返回 `BOOK_SOURCE_UNMAPPABLE`。

旧产物缺少映射时仍可加载和搜索；精确定位分别用 `BOOK_SOURCE_MAP_MISSING`、`BOOK_SOURCE_MAP_UNSUPPORTED_VERSION`、`BOOK_SOURCE_MAP_INVALID` 报告映射问题，非法区间返回 `BOOK_LOCATE_BAD_RANGE`。调用不重读或自动重建源文件，已有 book.search 与旧 Locator 结果保持不变。坐标、截断和阅读切片规则见 [搜索命中回源文本](source-location.md)；PDF 不支持正文搜索。

`jobs.refine`、`book.open`、`pdf.analyze` 必须且只能提供 path 或 bytes_hex 中一个。大文件优先使用 path；TXT 导入统计使用 `txt.inspect`，阅读使用 `txt.open/slice`。旧 `txt.analyze` 仍接收 bytes_hex 并返回全文。文件所有权、I/O 错误与边界见 [路径输入说明](io.md)。

## Context、句柄与并发

context 与对象句柄都属于当前进程，不能持久化后跨进程使用。context 0 保留给 legacy 入口；显式 context 从 1 开始、关闭后不复用。对象句柄在整个进程内全局递增且不复用，同时记录所属 context。

book、txt、pipeline 在同一数字空间分配句柄，包含 0。每个 context 独立拥有 8 个 book、4 个 txt、1 条 pipeline 的当前固定配额；一个 context 满额不占用另一个的名额。关闭单个对象只释放该对象；`context.close` 返回释放计数并成组释放其对象，都不删除源文件或已落盘产物。

错误类型包括 `*_BAD_HANDLE`、`HANDLE_WRONG_KIND`、`CONTEXT_BAD_HANDLE`、`CONTEXT_HANDLE_MISMATCH` 和配额错误。跨 context 误用不会碰巧操作相同数字的另一个对象；拿错类型不会关闭其他对象；重复对象/context close 均幂等。

根 Registry/Mutex 只管理 context、所有权、配额和对象租约；解析、I/O、搜索、定位与切片在根锁之外执行。book/TXT 只读共享，pipeline 按对象 Mutex 串行。单对象 close 摘除句柄后拒绝新请求，已经取得 Arc 租约的请求允许完成；`context.close` 还会请求 pipeline 协作取消并等待全部已接受调用退出。

调用方协调同一对象的 use/close。Dart worker 按序处理普通请求；每个 `PerchServices` 通过受跟踪的短生命周期 isolate 创建一个显式 context，close 等待在途 context 创建、调用原生排空关闭，再关闭 worker。`PipelineHandle.cancel` 通过独立控制 isolate 登记锁外取消信号，在途 step 于安全检查点返回 cancelled；细节见 [协作取消](cancellation.md)。异常 isolate 退出会通过清理 isolate 回收 context 及句柄，包括常驻 worker 被强制终止时尚在创建的 context。产物目录由单拥有者管理，失败调用也可能创建目录或更新源路径登记。
