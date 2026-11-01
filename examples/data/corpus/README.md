# 可复现样本集（BASE-01）

样本由 [make_corpus.py](../../../tools/make_corpus.py) 使用 Python 3.12+ 标准库生成，正文为项目自编内容，按仓库 MIT 许可证再分发。生成不读取时钟、不使用随机值，不下载电子书。EPUB 的条目顺序、时间、权限与平台标记固定，使用 ZIP Stored 避免不同 zlib 版本改变文件校验值；PDF 使用固定对象和字节偏移交叉引用表。

## 重建和校验

在仓库根目录运行：

```sh
# 重建随源码提供的小规模正确性样本，覆盖生成文件。
python3 tools/make_corpus.py

# 在内存中重建并逐字节比较；不会修复或覆盖现有文件。
python3 tools/make_corpus.py --check

# 可在其他目录重建，路径不进入样本内容或 manifest。
python3 tools/make_corpus.py --output target/corpus-copy
python3 tools/make_corpus.py --output target/corpus-copy --check

# 用真实 Rust 解析器和索引核对预期结果。
cargo test -p perch-core --test corpus --locked
```

`--check` 检查 manifest、源文件和预期文本，缺失、改动或 inputs/texts 内多出旧文件均返回非零退出码，并报告相对路径。增加或移除样本时须清理不再使用的生成文件。根目录的本说明不参与内容校验。

Git attributes 禁止对 inputs/texts 转换换行，manifest 固定为 LF，保证 Windows checkout 不改变校验值。CI 在 Linux/macOS/Windows 上重建校验并执行 Rust 回归；本地已校验的环境见 [状态记录](../../../docs/status.md)。

## 正确性样本

本目录 `manifest.json` 的 `profile=correctness`、`scale=1`。9 个输入、14 个预期文本和一个 manifest 均随源码提供，默认 Cargo 测试无需 Python 或生成步骤。

| ID | 原文件字节 | 解码字符 | 索引字符 | 覆盖行为 |
|---|---:|---:|---:|---|
| utf8 | 1,424 | 768 | 703 | 无 BOM、中英混排、emoji、组合附加符、CRLF/CR/LF、连续空行 |
| utf8-bom | 1,427 | 768 | 703 | UTF-8 BOM 去除；与无 BOM 样本解码结果相同 |
| gbk | 1,056 | 656 | 639 | GBK 自动探测与显式解码 |
| big5 | 1,120 | 688 | 671 | Big5 繁体中文自动探测与显式解码 |
| cross-shard | 12,299 | 4,100 | 4,099 | 「跨片检索」在字符 `[4094,4098)`，跨默认 4096 字分片 |
| long-line | 47,110 | 22,530 | 22,530 | 无换行长行，emoji/组合附加符及末尾「终点」定位 |
| epub2 | 4,320 | — | 1,171 | EPUB 2.0 / NCX、两章、Unicode 文本、跨章节短语 |
| epub3 | 4,208 | — | 1,171 | EPUB 3.0 / NAV；正文与 EPUB 2 样本相同 |
| pdf-outline | 1,362 | — | — | PDF 1.7、三张空白页、中文/emoji 元数据和两级目录 |

EPUB 的跨章节查询是 `跨章节\n检索接续`，命中起点属于第一章，终点落在第二章。当前正文抽取会包含 XHTML `<title>` 节点，并给每个文本节点追加换行；预期文本明确记录此行为。章节 href 为带前导 `/` 的容器绝对路径。

emoji 和 `e` + U+0301 都参与 Unicode scalar 偏移校验；一个 emoji 是一个 scalar，组合字符 `é` 是两个 scalar。检索样本同时覆盖含中文 token 的 emoji 短语，以及纯 emoji 查询返回空列表的现有行为。样本回归检查解码、逐字符切片、非法 UTF-8 字节边界、Locator、章节范围、短语范围和索引序列化往返。

## Manifest 契约

所有路径相对 manifest 所在目录。`version=1` 是本样本清单格式版本，`generator_version=1` 是生成方案版本，均不等同 SDK、ABI 或持久化索引版本。

| 字段 | 含义 |
|---|---|
| profile / scale | 样本用途与重复倍数；比较数据时须同时记录 |
| samples[].id / path / format / encoding | 稳定标识、相对路径、格式、源编码；PDF 说明文本字符串编码 |
| bytes / sha256 | 原文件字节数与完整文件 SHA-256，包含 TXT BOM |
| decoded | TXT 解码文本的 path / chars / utf8_bytes / sha256；BOM 去除，源换行保留 |
| indexed | 索引输入文本的同一组字段；TXT 换行和空行归一化，EPUB 按节点抽取拼接 |
| queries | 查询、chunk_chars 和预期 `[start,end)` 字符范围；纯 emoji 查询以 reason 说明空结果 |
| chapters | EPUB 每章 href 与 `[start_char,end_char)`，单位为索引文本的 scalar |
| title / toc / epub_version | EPUB 元数据和目录预期 |
| page_count / outline | PDF 页数和嵌套目录；page_index 为 0 基 |

`chars` 均按 **Unicode scalar** 计数，`utf8_bytes` 和文本 SHA-256 指 UTF-8 预期文本。解码文本和索引文本的坐标是两套坐标，不能直接互换。PDF 的 `text_chars=null` 表示未抽正文，不能解读为全文字数为零。指纹为全内容 SHA-256，与 `perch-import` 的首尾采样 Fingerprint 不同。

## 性能与压力样本

```sh
# 默认 scale=256，写入已忽略的 target/performance-corpus。
python3 tools/make_corpus.py --profile performance
python3 tools/make_corpus.py --profile performance --check

# 自选倍数和目录；校验须提供相同 profile / scale。
python3 tools/make_corpus.py --profile performance --scale 64 --output target/corpus-64
python3 tools/make_corpus.py --profile performance --scale 64 --output target/corpus-64 --check
```

性能样本复用同一组 ID，扩大 TXT 文本和 EPUB 章节正文；目录结构保持两章，PDF 保持三页，仅用于小文件结构解析。`scale` 必须为正整数，正确性 profile 固定为 1。生成器和预期文本会驻留内存，生成过程应放在测量区间之外。

默认性能样本的 cross-shard 有 1,049,599 个索引字符，两个 EPUB 各有 294,931 个索引字符；long-line 有 5,767,170 个字符、12,058,630 字节，超过现有 `ExtractOptions::default()` 的 500 万字符上限，可用于检验截断行为。需要完整抽取时由测量工具显式配置 max_chars，并记录配置和 truncated；本样本集不修改 SDK 默认限额。

输入对应的耗时、内存和吞吐见 [BASE-02 测量工具与本地结果](../../../docs/benchmarks/README.md)。ZIP Stored 样本不能代表 deflate 解压成本，三页空白 PDF 不能代表复杂文档，重复文本也不能代表任意书籍的 token 分布。压缩率、复杂 PDF、更多真实内容和设备测量仍需单独记录；坏样本与 fuzz 入口属于 ROBUST-01。
