# C ABI 与 JSON 契约

公共头文件为 [include/perch_core.h](../include/perch_core.h)，实际方法实现见 [perch-ffi](../crates/perch-ffi/src/lib.rs)。

全部 31 个方法的参数、返回字段、默认值、单位和错误码见 [完整方法参考](api-reference.md)。机器可读契约与重放说明见 [api](../api/README.md)，包括 Draft 2020-12 Schema 和真实 C/Dart 响应样例。

```c
char *perch_call(const uint8_t *method_ptr, size_t method_len,
                 const uint8_t *args_ptr, size_t args_len);
void perch_string_free(char *ptr);
```

## 内存与错误

输入采用 UTF-8 字节和显式字节长度，不要求 NUL 结尾。指针非空时必须至少有对应长度的有效内存，且在同步调用期间保持有效；库不会保留输入。

任一输入指针为 NULL 或长度为 0 时，当前实现视其为空字符串；空 args（包括纯空白）视为 `{}`。空方法返回 `FFI_UNKNOWN_METHOD`。有效内存中的非法 UTF-8 分别返回 `FFI_BAD_METHOD_PTR` / `FFI_BAD_ARGS_PTR`。输入中的 NUL 也按长度参与解析，不能用 strlen 截断数据来代替实际字节长度。

返回值是 Rust 分配的 NUL 结尾 UTF-8 JSON，必须用 `perch_string_free` 释放一次。不得用 C free，不能重复释放。NULL 返回值表示调用失败；`perch_string_free(NULL)` 安全。

业务失败统一返回：

```json
{"error":{"code":"FFI_BAD_ARGS","message":"缺少字符串字段 path"}}
```

调用方先解析 error，再读取业务字段。错误码用于逻辑判断，message 用于说明。`version` 保持 `{"core":"0.1.0"}` 旧形状；`compatibility` 返回 C ABI v1、JSON API v1.1、Locator 及 index/text 的可读与写入版本。详细规则见 [版本与兼容策略](compatibility.md)。

调用错误与任务失败分开：pipeline.step 的 `failed` / `exhausted` 是正常结果，带 reason；存储失败返回顶层 `PIPELINE_IO`。Dart `invoke` 将顶层 error 转为 `PerchCoreException(code,message)`，并在 finally 中释放 Rust 返回字符串。

## 方法集合

| 方法组 | 方法 |
|---|---|
| 基础 | `version`、`compatibility`、`locator.validate`、`fingerprint` |
| 文本精修 | `jobs.refine` |
| 搜索 | `book.open/load/search/close`、`index.search` |
| TXT | `txt.open/slice/locate/offset/line/close`、`txt.analyze` |
| PDF | `pdf.analyze` |
| 导入调度 | `pipeline.open/enqueue/step/status/state/promote/retry/cancel/close` |
| 可选算法对应接口 | `curl`、`hlc.parse`、`sync.merge/delta` |

该列表、逐项参考与 Schema 覆盖同一组方法，CI 检查生成文件和 Rust 分发集合无漂移。Rust 统一入口的 optional features 不改变当前 `perch-ffi` 方法集合。

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

`book.open` 接收 bytes_hex，抽正文并建索引。它适合小样本，不应放进阅读首屏路径。导入管线可在后台生成 index.bin 与 text.json，再通过 book.load 按路径载入并反复 book.search。

book.load 会验证正文与索引一致。搜索命中定位需按 [架构中的字符和文本规则](architecture.md) 校验；PDF 不支持正文搜索。

`pdf.analyze` 必须且只能提供 path 或 bytes_hex 中一个。大文件应优先使用 path。jobs.refine、book.open、txt.analyze 的 bytes_hex 会产生两倍原始字节长度的字符串；txt.analyze 还返回全文。

## 句柄与并发

句柄属于当前进程，不能持久化后跨进程使用。错误类型包括 `*_BAD_HANDLE`、`HANDLE_WRONG_KIND` 和配额错误。拿错类型不会关闭其他对象；重复关闭返回 closed:false。

book、txt、pipeline 在同一数字空间分配句柄，包含 0。关闭后通常不复用（计数器使用 u64 饱和递增），不能假定无限分配；当前固定配额为 8 个 book、4 个 txt、1 条 pipeline。关闭仅释放该对象，不删除源文件或已落盘产物；失效句柄不能再读取。

当前有状态调用由全局 Mutex 串行保护，pipeline.step 在执行期间可能阻塞其他句柄。业务配额固定；独立上下文、细粒度锁、取消和事件接口均在后续计划中。

调用方协调同一对象的 use/close。Dart worker 按序处理请求；PerchServices.close 等待请求、释放所拥有句柄并关闭 worker。异常 isolate 退出不能自动回收 Rust 注册表资源。pipeline.cancel 撤出排队任务，不中断已执行的同步 step；产物目录由单拥有者管理，失败调用也可能创建目录或更新源路径登记。
