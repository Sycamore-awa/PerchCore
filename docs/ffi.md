# C ABI 与 JSON 契约

公共头文件为 [include/perch_core.h](../include/perch_core.h)，实际方法实现见 [perch-ffi](../crates/perch-ffi/src/lib.rs)。

```c
char *perch_call(const uint8_t *method_ptr, size_t method_len,
                 const uint8_t *args_ptr, size_t args_len);
void perch_string_free(char *ptr);
```

## 内存与错误

输入采用 UTF-8 字节和显式字节长度，不要求 NUL 结尾。指针非空时必须至少有对应长度的有效内存，且在同步调用期间保持有效；库不会保留输入。

返回值是 Rust 分配的 NUL 结尾 UTF-8 JSON，必须用 `perch_string_free` 释放一次。不得用 C free，不能重复释放。NULL 返回值表示调用失败；`perch_string_free(NULL)` 安全。

业务失败统一返回：

```json
{"error":{"code":"FFI_BAD_ARGS","message":"缺少字符串字段 path"}}
```

调用方先解析 error，再读取业务字段。错误码用于逻辑判断，message 用于说明。API 尚未提供 schema 协商；`version` 返回 `{"core":"0.1.0"}`，只表示 crate 版本。

## 方法集合

| 方法组 | 方法 |
|---|---|
| 基础 | `version`、`locator.validate`、`fingerprint` |
| 文本精修 | `jobs.refine` |
| 搜索 | `book.open/load/search/close`、`index.search` |
| TXT | `txt.open/slice/locate/offset/line/close`、`txt.analyze` |
| PDF | `pdf.analyze` |
| 导入调度 | `pipeline.open/enqueue/step/status/state/promote/retry/cancel/close` |
| 可选算法对应接口 | `curl`、`hlc.parse`、`sync.merge/delta` |

该列表描述已有接口，参数以分发实现为准，暂未生成完整 JSON Schema。Rust 统一入口的 optional features 不改变当前 `perch-ffi` 方法集合。

## TXT 读取示例

在同一进程连续调用：

```text
txt.open   {"path":"/absolute/path/book.txt"}
           → {"handle":0,"chars":1200,"lines":30,"encoding":"UTF-8",...}
txt.slice  {"handle":0,"start":0,"chars":120}
           → {"text":"...","start":0,"end":120,"total":1200}
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

当前有状态调用由全局 Mutex 串行保护，pipeline.step 在执行期间可能阻塞其他句柄。业务配额固定；独立上下文、细粒度锁、取消和事件接口均在后续计划中。
