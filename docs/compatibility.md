# 版本与兼容策略

> API-02 已于 2026-11-01 按本文落地；2026-11-13 的 CTX-01、IO-01 与 LOC-01 按同一兼容边界增加独立上下文、路径输入与源位置映射。

## 目标

调用方需要能回答五个问题：

1. 当前动态库是否支持我要调用的 C ABI 和 JSON 方法？
2. 新增字段、新方法和修改旧方法分别属于什么兼容级别？
3. 数据库中的 Locator 能否安全解释？
4. `index.bin` / `text.json` 能否直接读取，还是必须从原书重建？
5. 遇到未来版本时，如何返回稳定、可分支处理的错误？

版本维度相互独立。crate 版本不等于 C ABI、JSON API、Locator 或产物版本，也不能代替能力检测。

## 当前基线

| 维度 | 当前值 | 实际行为 |
|---|---:|---|
| Rust workspace / Dart 包 | `0.1.0` | 各 Rust crate 同版本；Dart 包同步版本 |
| C ABI | v1 | 保留 `perch_call` / `perch_string_free`，增量导出 `perch_context_call` |
| JSON 方法集 | v1.5 | 36 个方法；`pipeline.step` 新增 `cancelled` 结果，并新增非阻塞 `pipeline.progress` |
| JSON Schema | v1.5 当前快照 | Draft 2020-12，`$id` 使用 `urn:perchcore:api:v1.5:*`；v1.1/v1.2/v1.3/v1.4 冻结保留 |
| Locator `perch` 扩展 | `v = 1` | 写入 v1；`Locator::validate` 用 `MODEL_UNSUPPORTED_VERSION` 拒绝其他版本 |
| `index.bin` | v1 | 魔数后的单字节版本；未知版本返回 `INDEX_UNSUPPORTED_VERSION` |
| `text.json` | 读 v0/v1，写 v1 | 缺省版本按 legacy v0；未知版本返回 `BOOK_TEXT_UNSUPPORTED_VERSION` |
| 可选 `sourceMap` | v1 | 与正文版本独立；缺失、不支持或损坏只影响精确定位 |

本批已将 Locator v1、`index.bin` v1 和无 `format` 的 `text.json` v0 作为固定夹具收录到 `examples/data/compatibility`，防止后续重构意外丢失旧读取能力。

## 版本规则

### 包与 crate

- workspace 内 Rust crate 继续同步发版，Dart 绑定与核心保持相同的主/次版本。
- `0.1.x` 只允许修复和向后兼容的增量；公开 Rust API 或绑定的破坏性变更至少升到 `0.2.0`。
- Cargo 版本只管 Rust 源码依赖，不用它推断落盘产物是否可读。
- 发布时同步记录 Rust、Dart、C ABI、JSON API 和产物写入版本。

### C ABI

- 原有两个符号定义为 **ABI v1**，其函数签名、调用约定、内存所有权和 NULL 规则不在 v1 内修改。
- 头文件定义 `PERCH_CORE_ABI_VERSION 1`，供编译期检查；不新增只返回同一数字的 C 符号。
- 可在 v1 中新增导出符号，但不得删除或改变旧符号。CTX-01 因此新增 `perch_context_call`，复用原返回内存规则；旧 `perch_call` 继续使用 legacy context。
- 动态加载失败或缺少 v1 必需符号属于宿主接入错误，不能通过 JSON 错误返回。

### JSON API 与 Schema

- 保留 `version` 的现有 `{ "core": "..." }` 形状，避免打破已按严格 Schema 解析它的调用方。
- `compatibility` 返回 ABI/API 版本、支持的 Locator 版本及各产物的可读/写入版本。新调用方先调用它；旧核心返回 `FFI_UNKNOWN_METHOD` 时，调用方只能进入明确的 legacy 兼容分支，不得猜测新能力。
- JSON API 定义为 `{major, minor}`。新方法或保留旧请求的可选输入分支可增加 minor；不得新增旧调用必须提供的字段、改变成功结果形状或重解释既有字段语义。context 由新 C 调用入口携带，不给旧业务 args 增加 context。
- v1.3 为 jobs.refine/book.open 增加互斥的 path 输入分支，旧 bytes_hex 分支的参数与结果保留。不能与旧请求兼容的形状使用新方法或新 major，旧方法至少保留一个次版本并在迁移文档中标明替代项。
- v1.4 新增 book.locate，搜索命中与旧 Locator 语义保留。v1.3 的三个 Schema 与 189 组请求/响应冻结，新增源端点使用 v1.4 快照。
- v1.5 新增 pipeline.step 的 cancelled 分支与协作取消语义，并新增 `pipeline.progress` 返回在途 book/阶段/粗粒度 checkpoint。同一 minor 内收紧 `context.close` 生命周期语义：返回前请求 pipeline 取消并排空已接受调用，请求/响应形状不变。v1.4 的三个 Schema 与 261 组请求/响应冻结；旧客户端如对 step outcome 做穷举，应在启用取消前升级类型化绑定或保留未知分支。
- Schema `$id` 使用固定快照 URN。API v1.1 的 `urn:perchcore:api:v1:*`、v1.2 的 `urn:perchcore:api:v1.2:*` 和 v1.3 的 `urn:perchcore:api:v1.3:*` 永久保留；增加 minor 时创建新快照，不给旧 URN 换内容。新 major 同样使用新的 major URN。
- Schema 是离线规范而不是运行时自动验证。调用方必须同时检查 API major 和自己依赖的方法/field 能力。

`compatibility` 的当前成功结果：

```json
{
  "core": "0.1.0",
  "abi": 1,
  "api": {"major": 1, "minor": 4},
  "locator": {"read": [1], "write": 1},
  "artifacts": {
    "index": {"read": [1], "write": 1},
    "text": {"read": [0, 1], "write": 1}
  }
}
```

### Context 与句柄

- `perch_call` 永远使用不可销毁的 legacy context；旧客户端的固定配额、句柄和 close 行为不变。
- `context.open/close` 通过 `perch_call` 管理数值 context，业务方法通过 `perch_context_call` 调用，JSON args 不增加 context 字段。
- 对象句柄全进程唯一并记录所有者。跨 context 使用稳定返回 `CONTEXT_HANDLE_MISMATCH`；不存在或已关闭的 context 返回 `CONTEXT_BAD_HANDLE`。
- 每个 context 独立执行当前 8 book / 4 TXT / 1 pipeline 配额；`context.close` 成组释放内存对象，不删除磁盘产物。
- 新 Dart 绑定延迟解析 `perch_context_call`：legacy API 仍能加载旧库；使用显式 context 前必须确认 API minor 至少为 2 且符号存在。

### Locator

- `perch` 整个缺省表示纯 Readium Locator，继续允许。`perch` 存在时 `v` 必填。
- v1 内只能新增可选字段，不改变已有键的单位、坐标系或回退顺序。不理解的可选字段可忽略。
- 引入新必填锚点、改变字符单位或重解释已有字段时升到 v2。
- `Locator::validate` 严格拒绝不在支持集合内的 `perch.v`，稳定错误码为 `MODEL_UNSUPPORTED_VERSION`。
- 应用不得把未知版本当成精确位置。如 Locator 同时带标准 Readium 锚点，可由调用方显式降级为纯 Readium 定位；核心不静默删除扩展。

### `index.bin`

- 现有格式固定为 v1，写入器只写当前版本，读取器明确列出支持的版本集合。
- 改变任何字节布局、token 规则、正文表示或完整性规则时升版本。不得在相同版本下改写语义。
- 未知版本继续返回 `INDEX_UNSUPPORTED_VERSION`；损坏与版本不支持保持为两类错误。
- 首个升级版本不做就地迁移：若原书可读，删除该书的阶段三产物并重建；若原书缺失，保留旧产物并向使用方报告不支持，不伪装为空搜索结果。
- 重建必须同时生成匹配的 `index.bin` 与 `text.json`，不允许只升级其中一个。

### `text.json`

- 现有无版本形状定义为 **v0**。新读取器继续接受 v0，其中缺省 `format` 按历史行为解释为 EPUB。
- 新写入在保持现有顶层形状的前提下带 `version: 1`。版本由专用落盘编解码层处理，不给公开 Rust `BookText` 增加字段，因此旧结构体字面量仍可编译。旧 serde 读取器会忽略该字段，新读取器则在使用正文前检查版本。
- v1 内可新增有默认值的可选字段；修改正文字符单位、章节区间含义或必填字段时升到 v2。
- 未知显式版本返回 `BOOK_TEXT_UNSUPPORTED_VERSION`；JSON 语法/字段损坏继续返回 `BOOK_TEXT_JSON`。
- `index.bin` 和 `text.json` 的正文一致性检查继续保留。任一文件版本不支持或二者不匹配时，整组阶段三产物重建。

### 可选 `sourceMap`

- text.json v1 内可选 sourceMap 使用独立 `version:1`，不修改正文、章节区间或 index.bin。旧 BookText 读取器忽略该字段，既有公开 Rust 字段与函数签名保留。
- 没有 sourceMap 的旧产物仍加载和搜索；book.locate 返回 `BOOK_SOURCE_MAP_MISSING`。显式 null 或无效结构返回 `BOOK_SOURCE_MAP_INVALID`，未知映射版本返回 `BOOK_SOURCE_MAP_UNSUPPORTED_VERSION`，均不把正文判为损坏。
- 新建索引与 PathStore 导入生成映射。既有 BookStore 的默认保存扩展退回原正文保存，实现扩展后才保存映射。
- 精确坐标对应索引建立时的源版本与解码，不对内容变更自动重锚。需要定位的旧产物从原书重新生成匹配的 index/text/sourceMap；STORE-01 已让 PathStore 将缺失或无效映射降级为阶段三重建。
- 紧凑区间形状见 [存储契约](storage.md)，源单位、边界吸附及错误见 [搜索命中回源文本](source-location.md)。

## 变更分类

| 变更 | 同版本允许 | 处理方式 |
|---|---|---|
| 新 Rust 类型/方法，不改旧签名 | 是 | crate patch/minor，增加测试与文档 |
| 新 C 符号 | 是 | 保留 ABI v1 必需符号，通过能力检测使用 |
| 新 JSON 方法 | 是 | API minor +1；旧核心返回 `FFI_UNKNOWN_METHOD` |
| 保留旧请求的可选输入分支 | 是 | API minor +1、新 Schema 快照；旧请求/结果继续回归 |
| 现有请求新必填字段 | 否 | 新方法或新 API major |
| 现有成功结果新必填字段 | 否 | 新方法/结果版本，不改写旧 Schema |
| 新稳定错误码 | 是 | 只用于新可辨认分支；message 仍不稳定 |
| Locator v1 新可选字段 | 是 | 旧读取器忽略；不可改变旧字段语义 |
| Locator 必填锚点/单位改变 | 否 | 升 `perch.v` |
| `index.bin` 字节或索引语义改变 | 否 | 升格式版本，成组重建阶段三产物 |
| `text.json` 新默认字段 | 是 | v1 内可增加，旧夹具必须继续可读 |
| `text.json` 字符/区间语义改变 | 否 | 升格式版本，与 index 一起重建 |

## 错误与调用方行为

| 场景 | 稳定信号 | 调用方动作 |
|---|---|---|
| 旧核心无能力查询 | `FFI_UNKNOWN_METHOD` | 只进入已知 legacy 分支，否则拒绝启动不支持功能 |
| JSON API major 不匹配 | 宿主 SDK 兼容检查失败 | 不发送业务请求，提示更新核心或绑定 |
| Locator 扩展版本不支持 | `MODEL_UNSUPPORTED_VERSION` | 不信任扩展锚点；可显式尝试纯 Readium 降级 |
| 索引版本不支持 | `INDEX_UNSUPPORTED_VERSION` | 原书存在时重建索引+定位文本 |
| 定位文本版本不支持 | `BOOK_TEXT_UNSUPPORTED_VERSION` | 与 index 一起重建 |
| 源映射缺失、不支持或损坏 | `BOOK_SOURCE_MAP_MISSING` / `BOOK_SOURCE_MAP_UNSUPPORTED_VERSION` / `BOOK_SOURCE_MAP_INVALID` | 搜索可继续；需要精确定位时从原书重建映射产物 |
| 定位区间无效或只含合成内容 | `BOOK_LOCATE_BAD_RANGE` / `BOOK_SOURCE_UNMAPPABLE` | 检查搜索命中的 scalar 区间或报告该区间无源位置 |
| 索引/正文不匹配 | `BOOK_INDEX_MISMATCH` | 与版本升级相同，成组重建 |
| 产物损坏 | 现有 `INDEX_*` / `BOOK_TEXT_JSON` | 不当成“无命中”；重建或报告原书缺失 |

错误 `code` 是稳定分支信号，`message` 只作诊断，可随路径、操作系统和依赖版本变化。

## 实施结果

已按以下顺序落地，每一步均保持旧夹具通过：

1. 增加带 major 的 Schema URN 与 `compatibility` 方法，更新 C/Dart 真实重放。
2. 在 C 头文件增加 ABI v1 宏，不改函数签名。
3. 让 Locator 校验拒绝未知 `perch.v`，新增稳定错误码和 v1/未来版本回归。
4. 为 `text.json` 写入 `version: 1`，保留 v0 读取，在 `book.load` 增加专用的不支持错误。
5. STORE-01 已实施阶段三成组重建编排；打开时不删除旧产物，源文件缺失时保留并进入既有失败/重试流程。

## 已确认决策

1. 使用新 `compatibility` 方法，而不扩展既有 `version` 结果。
2. `text.json` 保持顶层形状，通过新增 `version: 1` 升级；缺省版本按 v0 读取。
3. Locator 从当前宽松行为改为严格拒绝未知 `perch.v`，错误码为 `MODEL_UNSUPPORTED_VERSION`。
4. 不对 index/text 做就地迁移；首选从原书成组重建，原书缺失时保留旧产物并报错。

