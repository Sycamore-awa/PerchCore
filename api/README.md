# 可校验的接口契约

这里描述当前 31 个 JSON 方法，入口与所有权见 [FFI 契约](../docs/ffi.md)，逐项字段、默认值、单位和错误码见 [方法参考](../docs/api-reference.md)。JSON API 当前为 v1.1，运行时可通过 `compatibility` 查询 ABI/API、Locator 和产物版本。

| 文件 | 用途 |
|---|---|
| [catalog.json](catalog.json) | API major/minor 及每方法的请求、成功结果、业务错误和说明 |
| [common.schema.json](common.schema.json) | Locator、Annotation、Change、管线、PDF 和几何共享结构 |
| [request.schema.json](request.schema.json) | 校验 `{ "method": "txt.slice", "args": {...} }` |
| [response.schema.json](response.schema.json) | 校验 `{ "method": "txt.slice", "result": {...} }`，含成功与错误 |
| [examples.json](examples.json) | 同进程调用序列、动态值引用、独立行为断言和受控文件操作 |
| [responses.json](responses.json) | 真实 C/Dart 一致后记录的完整响应样例 |

method/args 与 method/result 外层对象仅供离线工具使用。C ABI 仍分别接收方法 UTF-8 和 args JSON；库返回 result 本身。这些文件不是运行时协议协商，不添加方法、版本字段或运行时严格校验。

采用 JSON Schema Draft 2020-12，`$id` 为带 API major 的固定 URN：`urn:perchcore:api:v1:*`。消费者需将三个 Schema 注册到本地解析器。检查工具使用 referencing.Registry，并拒绝未注册的引用；验证期间不联网下载 Schema。新 major 必须使用新 URN，不原地改写 v1。

Schema 定义规范请求：对象根、正确类型、明确字段与范围，拒绝未知字段。现有运行时通常忽略未知字段，对不少可选标量类型错误回退默认值，也接受部分非对象根。这些宽松行为不作为规范调用方式。结构检查与语义检查分开：Locator 的锚点/进度规则、HLC 解析、策略跨字段约束和 Change 一致性仍由 Rust 判断。通过请求 Schema 不保证调用成功。

整数 Schema 覆盖 Rust 的 u64/u32/i64 范围。原生 Dart `int` 为有符号 64 位，其他 JSON 消费者可能以 double 处理数字；跨语言实际传参须受调用语言精确表示范围限制。JS/JSON 消费者应特别避免把大于 2^53−1 的整数经浮点数传递。本批不增加数字字符串兼容入口。

## 重建与离线检查

生成器仅用 Python 标准库。修改 [api_catalog.py](../tools/api_catalog.py) 或 [api_examples.py](../tools/api_examples.py)，然后重建；不要手动修改生成文件。

```sh
python3 tools/api_catalog.py
python3 tools/api_catalog.py --check
python3 -m venv target/api-tools
target/api-tools/bin/python -m pip install -r tools/requirements-api.txt
target/api-tools/bin/python tools/check_api.py
target/api-tools/bin/python -m unittest discover -s tools -p 'api_contract_tests.py'
```

Windows 的虚拟环境命令使用 `target/api-tools/Scripts/python.exe`。jsonschema 是开发工具依赖，不进入 Rust/Dart SDK 生产依赖。已有样本/基准的标准库检查继续独立运行。

## 真实 C 与 Dart 重放

需要 Rust、C 编译器/CMake、Python 开发依赖和 Dart SDK。默认构建 Rust release 库、C 持久客户端和 Dart AOT 持久客户端，给两个新进程分别建立临时源文件与产物目录：

```sh
target/api-tools/bin/python tools/check_api.py --replay
# 已构建 Rust 库时可复用；C/Dart 客户端仍会重新编译：
target/api-tools/bin/python tools/check_api.py --replay --skip-build
# 平台仅验证 C 时：
target/api-tools/bin/python tools/check_api.py --replay --clients c --skip-build
```

每个请求引用自己的动态句柄和路径，`$var` 取样本/路径，`$ref` 取之前的结果（如 `txt_open.handle`）。固定 now_ms，使退避分支可复现；受控文件操作仅作用于工具自己的临时目录。`canonical:false` 明确标出规范 Schema 应拒绝、但需验证现有运行时行为的输入。

每次实调校验响应 Schema、独立行为断言和记录的响应；两个客户端的成功数据逐字段一致，错误按 code 一致且 message 必须为非空字符串。响应样例中的错误 message 以 `<diagnostic>` 表示，因为路径、OS 和解析库诊断可能变化；业务失败的 reason 保留。recovered 无顺序承诺，比较和样例按 book 排序。其他数组保留真实顺序。

有意改变契约时，先审阅变更，再显式运行 `--replay --record` 更新响应样例；只有 C 与 Dart 都通过才允许记录。普通检查和 CI 不修改样例。检查直接提取 Rust 分发方法集合，新增/删除方法会触发目录与样例覆盖检查失败。

本批覆盖中文/emoji/组合附加符、GBK 显式解码、UTF-8 字节锚点、EOF/换行、错误句柄、重复关闭、固定配额、各 step 分支、I/O 错误、索引正文不一致、未来 Locator/text 版本、管线重开、同步分页/墓碑/一致性，以及 C 输入长度/无效 UTF-8/NULL。当前序列为每个 C/Dart 客户端 147 次调用。平台实测证据见 [状态记录](../docs/status.md)；不以离线 Schema 通过代替原生平台执行验证。
