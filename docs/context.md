# 独立上下文设计（CTX-01）

> 状态：已于 2026-11-13 实施并验证。

## 目标与边界

CTX-01 为同一进程内的不同 SDK 使用方提供独立的句柄所有权和固定配额：

- 两个上下文分别拥有 8 个 book、4 个 TXT、1 条 pipeline 的现有配额，互不挤占。
- 句柄明确属于创建它的上下文；把句柄交给其他上下文时返回稳定错误。
- 销毁上下文会一次释放它拥有的全部对象，不影响 legacy 调用或其他上下文。
- 现有 `perch_call`、`perch_string_free`、31 个 JSON 方法及其请求/结果形状继续可用。

本任务不缩短全局锁的持有时间，也不引入可配置配额、取消或异步任务。这些分别属于 CTX-02、CTX-04 和 CTX-03。

## 已实施方案

保留 C ABI v1 的两个既有符号，新增一个兼容符号 `perch_context_call`。JSON API v1.2 增加 `context.open` / `context.close`；其他业务方法的 JSON 参数保持原样，由调用入口携带上下文。

```c
char *perch_context_call(uint64_t context,
                         const uint8_t *method_ptr, size_t method_len,
                         const uint8_t *args_ptr, size_t args_len);
```

典型调用：

```text
perch_call("context.open", "{}")
  -> {"context":1}

perch_context_call(1, "txt.open", {"path":"/books/a.txt"})
  -> {"handle":42,...}

perch_context_call(1, "txt.slice", {"handle":42,"chars":800})
  -> {"text":"...",...}

perch_call("context.close", {"context":1})
  -> {"closed":true,"released":{"books":0,"txts":1,"pipelines":0}}
```

原 `perch_call` 始终进入不可销毁的 legacy 上下文，行为、结果形状和固定配额保持不变。因此旧 C/Dart 客户端无需修改，也不会与显式上下文共享配额。

### 为什么由调用入口携带上下文

- 已发布的 v1 Schema 按 [兼容策略](compatibility.md) 只读；给 19 个旧业务请求增加 `context` 字段会原地改写这些契约。
- 不引入结果为任意 JSON 的 `context.invoke` 包装方法，每个业务方法仍使用 API-01 已建立的独立 Schema 和样例。
- 新符号复用 `perch_call` 的 UTF-8、JSON、错误和返回内存规则；结果仍由 `perch_string_free` 释放。
- C ABI v1 允许新增符号但不改变旧符号。旧动态库仍可被旧绑定加载；新绑定只在使用显式上下文时解析新符号。

## 方法与 ABI 契约

### `context.open`

通过 legacy `perch_call` 调用，请求为空对象。成功返回：

```json
{"context":1}
```

上下文编号是当前进程内的 `u64` 临时标识，不是指针或持久化 ID。0 保留给内部 legacy 上下文，不由 `context.open` 返回。编号关闭后不复用。

CTX-01 不接收配额配置。显式上下文先使用与 legacy 相同的固定配额；CTX-04 再基于测量结果增加配置，并保持空请求采用默认值。

### `perch_context_call`

除开头的 `context` 外，指针、长度、同步借用、UTF-8、NULL、返回字符串和错误 JSON 规则与 `perch_call` 完全相同。

它接受现有业务方法名和原有 args，不给业务 JSON 增加字段。上下文不存在、已关闭或传入保留值 0 时返回 `CONTEXT_BAD_HANDLE`。`context.open` / `context.close` 只能通过 legacy `perch_call` 调用；从上下文入口调用返回 `CONTEXT_METHOD_SCOPE`。

无状态方法可以通过任一入口调用。通过 `perch_context_call` 调用时仍先验证上下文存在，使服务销毁后所有后续请求一致失败。

### `context.close`

通过 legacy `perch_call` 调用，请求：

```json
{"context":1}
```

成功返回是否实际关闭，以及本次释放的各类对象数量：

```json
{"closed":true,"released":{"books":2,"txts":1,"pipelines":0}}
```

重复关闭或从未存在的编号返回 `closed:false` 和全 0 计数，保持现有 `*.close` 的幂等风格。context 0 返回 `CONTEXT_LEGACY`。

`context.close` 与同进程其他调用仍由当前全局锁串行；关闭完成后，该上下文的所有对象句柄立即失效。

## 句柄与注册表

根注册表维护：

```text
RootRegistry
├── next_context
├── next_handle
├── legacy ContextRegistry
├── contexts: context -> ContextRegistry
└── owners: object handle -> context

ContextRegistry
├── open: object handle -> OpenDoc
├── books
├── txts
└── pipelines
```

对象句柄在整个进程内全局递增且不复用，而不是在每个上下文内从 0 重新编号。这样把 A 的句柄通过 B 的调用入口传入时，不会碰巧操作 B 中相同编号的另一个对象。

创建有状态对象时先确认调用上下文，再按该上下文的独立计数检查配额。关闭对象或整个上下文时同步删除 `owners` 记录。对象构造通常在最终登记前执行文件 I/O；登记时必须再次确认上下文仍存在。若期间上下文已关闭，丢弃已构造对象并返回 `CONTEXT_BAD_HANDLE`。

## 错误规则

新增稳定错误码：

| 错误码 | 条件 |
|---|---|
| `CONTEXT_BAD_HANDLE` | `perch_context_call` 引用 0、不存在或已关闭的上下文 |
| `CONTEXT_HANDLE_MISMATCH` | 对象句柄存在，但属于另一个上下文 |
| `CONTEXT_METHOD_SCOPE` | 从上下文入口调用 context 生命周期方法 |
| `CONTEXT_LEGACY` | 尝试显式关闭保留的 legacy 上下文 0 |

错误判定顺序：

1. C 输入指针与 UTF-8；
2. 上下文是否有效、方法是否属于正确入口；
3. JSON 请求字段和整数范围；
4. 对象句柄是否属于该上下文；
5. 对象类型；
6. 具体业务参数与操作。

句柄在所有上下文都不存在时，继续使用方法原有的 `BOOK_BAD_HANDLE`、`TXT_BAD_HANDLE`、`PIPELINE_BAD_HANDLE` 或 close 的 `closed:false`。句柄存在但类型错误时继续使用 `HANDLE_WRONG_KIND`。

legacy `perch_call` 收到显式上下文拥有的对象句柄，同样返回 `CONTEXT_HANDLE_MISMATCH`，不能绕过所有权边界。

## 配额与销毁语义

每个 ContextRegistry 独立执行当前固定限制：

| 资源 | 每个上下文限制 |
|---|---:|
| 搜索 book | 8 |
| TXT | 4 |
| pipeline | 1 |

因此一个上下文达到 `BOOK_TOO_MANY_OPEN` 不影响另一个上下文继续打开 book。这里隔离的是所有权和配额，不承诺并行执行；CTX-01 后各上下文仍共享根 Mutex，`pipeline.step` 仍可能阻塞其他上下文，留给 CTX-02 解决。

销毁只释放内存对象，不删除源文件、pipeline 产物或状态文件。pipeline 的单产物目录所有者规则不变；不同上下文或进程同时打开同一 `out_dir` 仍属于调用方错误。STORE-01 不增加文件锁，多进程支持等出现真实需求后单独设计。

## Dart 绑定

低层 `PerchCore.invoke` 保持走 legacy `perch_call`，新增 `invokeInContext`、`openContext` 和 `closeContext`。新符号延迟解析：只使用 legacy API 时，新绑定仍可加载旧动态库；尝试显式上下文时若缺少符号，返回清楚的兼容错误。

类型化绑定增加进程内 `PerchContext` 值对象。`PerchServices.start` 默认创建并拥有一个显式上下文，后续 book/TXT/pipeline 请求通过 `invokeInContext` 发送；`PerchServices.close` 等待在途请求后关闭上下文，再退出 worker。多个 `PerchServices` 实例的配额和销毁因此互不影响。

若上下文创建失败，服务启动失败且不保留 worker。若服务关闭中单个 `*.close` 失败，最终仍调用 `context.close` 兜底；close 保持幂等。

后续 CTX-03 小切片将 Dart worker 的 context 创建移到受跟踪的短生命周期 isolate。常驻 worker 异常退出时，创建结果仍会回到主 isolate；`dispose` 和 `PerchServices.close` 先等待在途创建，再通过清理 isolate 调用 `context.close`，成组回收该 context 及句柄。

## 兼容、版本与 Schema

- C ABI 仍为 v1；旧两个符号不变，新增 `perch_context_call`。头文件明确该符号从 JSON API v1.2 对应核心开始提供。
- JSON API 从 v1.1 升到 v1.2，方法数从 31 增至 33；现有 31 个方法的请求与结果结构不变。
- `version` 结果不变；`compatibility` 只把 `api.minor` 改为 2，不新增结果字段。
- 当前 `urn:perchcore:api:v1:*` 三个 Schema 和对应夹具作为 v1.1 快照保留，不原地改写。
- 新增 `urn:perchcore:api:v1.2:*` Schema 快照，包含原样复制的 31 个方法和两个 context 生命周期方法。工具默认校验最新快照，同时继续回归 v1.1 固定样例。
- 旧 C/Dart 请求响应样例继续重放；新增显式上下文的 C 与 Dart AOT 重放。

增加 minor Schema 快照是对现有“major Schema 只读”规则的具体落地：旧 URN 永不换内容，API minor 的新增方法进入新 URN。实现时同步澄清 [兼容策略](compatibility.md) 和 [API 工具说明](../api/README.md)。

## 验收测试

实现与回归覆盖：

1. legacy 客户端的现有完整重放结果除 API minor 外无漂移，v1.1 Schema/夹具仍可离线验证。
2. A、B 各自可打开满额的 8 book / 4 TXT / 1 pipeline；A 满额不影响 B。
3. A 的句柄通过 B 的入口调用返回 `CONTEXT_HANDLE_MISMATCH`，不触碰 B 的对象。
4. 关闭 A 后其所有句柄失效，B 的读取、搜索和 pipeline 状态不变。
5. 重复 `context.close` 幂等；错误类型句柄与未知句柄维持原契约。
6. 打开对象与关闭上下文竞争时，对象要么成功归属并被关闭，要么返回 `CONTEXT_BAD_HANDLE`，不能泄漏到 legacy 或其他上下文。
7. 真实 C 客户端使用新符号，Dart 同步/worker 使用同一上下文；内存均由 `perch_string_free` 正确释放。
8. Rust 全量测试、C/Dart 真实重放、两版 Schema、生成漂移、Dart analyze 和 FFI smoke 全部通过。

CTX-02 再增加可控制的长任务并发测试，证明一条 pipeline 不阻塞另一个上下文；该性能结论不提前计入 CTX-01。

## 未采用的方案

### 给旧业务请求增加可选 `context`

运行时兼容，但会修改 19 个已经发布的 v1 请求 Schema；复用同一 URN 会造成缓存和离线验证歧义，因此不采用。

### 新增不透明 C 指针

`perch_context_new/call/free` 的指针形式会增加重复释放和悬挂指针规则。数值 context 由根注册表校验，错误可以稳定返回为 JSON，也更适合 Dart isolate 传递。

### `context.invoke` 包装任意方法

单一 JSON 包装方法不需要新 C 符号，但 result 必须是所有方法结果的动态联合，削弱 API-01 已建立的逐方法 Schema、样例和错误覆盖，调用日志也多一层嵌套。

### 每个上下文使用局部对象编号

局部编号会让 A/0 和 B/0 同时合法，误用上下文时可能静默操作错误对象，无法满足“跨上下文误用可诊断”。因此对象句柄必须进程内全局唯一。

## 实施结果

1. 根注册表已拆为 legacy 与多个 ContextRegistry；对象句柄保持全进程唯一。
2. 已导出 `perch_context_call`，并实现 context 生命周期、所有权检查和独立配额。
3. v1.1 Schema 原样保留，v1.2 Schema、33 方法目录和 161 步样例已生成。
4. C/Dart 持久客户端和 `PerchServices` 已使用显式 context；旧低层调用继续走 legacy。
5. CTX-01 验证结果见 [状态记录](status.md)；当时保留的全局锁限制已由后续 CTX-02 解决。

## CTX-02 后续实施

CTX-02 在不改变本文 C ABI、JSON 方法和句柄归属契约的前提下，将根注册表收缩为短时元数据锁：

- book 和 TXT 在查找时克隆只读 `Arc`，随后在根锁之外搜索、定位和切片。
- pipeline 在查找时克隆 `Arc<Mutex<_>>`，同一 pipeline 的 step/status/enqueue 等操作串行，不同句柄和 context 不互相占用根锁。
- 打开对象时的文件 I/O、解析和建索引仍先在锁外完成，最后登记时重新检查 context 和配额。
- 句柄或 context 销毁时先从根表摘除，然后在根锁之外释放可能很大的对象。

use/close 的线性化点是“是否已取得对象租约”：已取得租约的在途调用可继续完成；close 一旦摘除句柄，之后的查找立即返回原有 BAD_HANDLE 或 CONTEXT_BAD_HANDLE。因为在途调用持有 `Arc`，对象不会悬挂；close 本身也不需要等待对象锁。

上述是单对象 `*.close` 的语义。CTX-03 后的 `context.close` 额外跟踪 context 级调用租约：先摘除 context，再请求其 pipeline 协作取消，并等待全部已接受调用退出。可控阻塞回归和并发切片延迟对比见 [状态记录](status.md)。
