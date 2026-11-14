# 导入与存储契约

核心提供登记计划、调度和产物存储。文件选择、权限、书库数据库和原文件复制由使用方管理。

## 阶段

1. 使用方获取文件大小与首尾采样，计算 Fingerprint。
2. `perch-import::plan_import` 根据候选和已导入指纹生成登记、跳过和拒绝列表。
3. 使用方保存原文件副本与 BookId，登记书库，并给管线提供可读路径。
4. `perch-jobs::Pipeline` 执行 EPUB/TXT 元数据精修和文本索引。
5. 使用方读取状态并更新自己的界面或业务记录。

指纹基于大小与采样，不是全文件哈希。适合快速去重，不是密码学意义上的内容身份或完整性校验。

## PathStore

`BookStore` 是存储边界，`PathStore` 是文件系统实现。产物以 UUID BookId 为目录名：

```text
out-dir/
  <book-uuid>/
    state.json
    refinement.json
    text.json
    index.bin
```

state.json 保存阶段、重试等信息；refinement.json 保存精修结果；text.json 包含索引用正文、章节区间与可选源映射；index.bin 使用版本化二进制索引。

原文件路径不持久化在 state.json 内。重启后使用方要重新提供 BookId 与路径映射；手机沙盒的绝对路径可能在升级或恢复后改变。

PathStore 打开时扫描状态并审计已宣称完成的产物。损坏、缺失或不可读的单书 `state.json` 不会阻断其它书；该 UUID 目录会以 `registered` 恢复，`reason` 使用 `STORE_STATE_*` 前缀。只有根产物目录无法创建或扫描时 `PathStore::new` 整体失败。状态恢复不等于恢复操作系统后台任务。

## STORE-01 恢复规则

`state.json` 是最后提交的检查点：精修成功后才写 `indexing`，`refinement.json`、`index.bin` 和 `text.json` 都成功后才写 `ready`。因此写入中断最多使状态停留在更早阶段，不会把半成品当作成功。

PathStore 重开时按以下规则收敛：

| 磁盘情况 | 恢复状态 | 原因前缀 |
|---|---|---|
| `state.json` 缺失、损坏或不可读 | `registered` | `STORE_STATE_MISSING` / `STORE_STATE_CORRUPTED` / `STORE_STATE_UNREADABLE` |
| `indexing` / `ready` 但 refinement 缺失或不可解析 | `registered` | `STORE_REFINEMENT_INVALID` |
| `ready` 但 index/text 缺失、损坏、版本不支持或正文不一致 | `indexing` | `STORE_INDEX_INVALID` |
| `ready` 但 sourceMap 缺失、损坏或版本不支持 | `indexing` | `STORE_INDEX_INVALID` |
| 失败的 index 任务同时缺失有效 refinement | 保留 `failed` 和次数，重试阶段改为 refine | `STORE_REFINEMENT_INVALID` |

降级只修正内存中的恢复计划，不在打开时删除或覆盖旧产物。调用方通过 `pipeline.enqueue` 重新提供 BookId、源路径和格式后，管线从恢复阶段幂等重做。源文件已删除或失去权限时，任务进入现有失败/退避流程，旧产物保留供诊断或旧能力降级使用。

`index.bin` 与 `text.json` 是一个逻辑产物组，任何一个不可用或两者正文不一致都重建整组；不就地转换未知版本。legacy text v0 仍可由通用读取器打开，但恢复管线要求当前 sourceMap v1，因此会明确触发阶段三重建。

写入使用临时文件后重命名。多进程共用产物目录没有文件锁或事务协调，当前必须由单个管线拥有该目录；STORE-01 不声明多进程安全，也不用锁文件猜测进程存活。使用方须处理磁盘容量、权限和旧产物回收。

## 索引兼容性

索引读取会检查版本和内容有效性。`index.bin` 当前读写 v1；`text.json` 读 legacy v0 与 v1、只写 v1。未知版本分别返回 `INDEX_UNSUPPORTED_VERSION` / `BOOK_TEXT_UNSUPPORTED_VERSION`，与损坏文件区分。升级时 index/text 作为同一组从原书重建；当前不要依赖内部二进制结构。位置 JSON 的 perch 扩展名保持兼容，不随仓库重命名改变。详见 [版本与兼容策略](compatibility.md)。

## 可选 sourceMap v1

LOC-01 在 `text.json` v1 增加可选 `sourceMap`，正文和 `index.bin` 的格式、字符单位与截断结果保留。映射自身 `version:1` 与 text/index 版本独立。新导入管线和 `book.open` 均生成映射；原 BookText 字段与保存接口不变。

`BookStore::save_index_with_source_map` 有默认实现，将正文交给既有 `save_index`。原 Store 无需新增方法即可编译，但需要实现该扩展才能保存源映射。PathStore 的扩展保存完整正文与映射；显式使用原 body-only save_index 仍产生无映射产物。

TXT 示例，对应原解码 `"\r\n甲\r\n\r\n\r\n乙😀"`：

```json
{
  "version":1,
  "indexedChars":5,
  "format":"txt",
  "decodedChars":11,
  "decodedBytes":18,
  "origin":[2,2],
  "runs":[{"t":[1,3]},{"n":[2,2]},{"n":[4,4]},{"t":[2,7]}]
}
```

`origin` 为首个保留 TXT 源边界的 `[scalar,UTF-8 byte]`；只在 TXT 存在，空正文使用 `[0,0]` 和空 runs。EPUB 的 format 为 epub，不带 decodedChars/decodedBytes/origin。`indexedChars` 等于索引正文 scalar 数。

连续区间的起点由前一区间长度累加，避免逐字符保存记录或重复写入所有端点。v1 runs 的紧凑编码为：

| 记录 | 含义 |
|---|---|
| `{"t":[chars,bytes]}` | TXT 连续坐标区间；索引与源的 scalar/UTF-8 增量相同 |
| `{"n":[sourceChars,sourceBytes]}` | TXT 一个归一化换行，对应完整源区间；索引增量为 1 scalar / 1 byte |
| `{"e":[chars,bytes,href,nodePath]}` | 一个完整 EPUB 解码文本节点，从节点偏移 0 起 |
| `"s"` | 一个 EPUB 人工换行；索引增量为 1 scalar / 1 byte，没有源坐标 |

TXT 的源起点也由 origin 与前一区间的源长度累加。正文中的 CR 转 LF 若两端坐标增量同为 1，可并入 t。相邻连续 TXT 区间合并；EPUB 路径保留到具体文本节点，人工分隔符单独标记。

读取器验证累计长度、UTF-8/scalar 边界、源范围、格式和 EPUB 路径与章节 href。只在内存中每 256 scalar 保存 UTF-8/UTF-16 边界抽样点，定位无需逐次从全文起点扫描；该缓存不写入产物。结构校验不重新读取原书或检查已经修改的源文件。

缺少 sourceMap 的旧文件继续可由 `book.load` 加载和搜索。未知映射版本、损坏映射与缺失映射在 `book.locate` 返回相应错误；显式 null 属于损坏。整个 text.json 语法损坏仍是正文加载错误。PathStore 恢复审计更严格：Ready 产物缺失有效 sourceMap 时降级为 `indexing`，等重新提供原书后成组重建 index/text/sourceMap。源端点语义见 [搜索命中回源文本](source-location.md)。

PDF 只提供结构分析，不进入 EPUB/TXT 的正文抽取与索引任务。书库不能将已解析 PDF 标成支持全文搜索。
