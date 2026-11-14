# LOC-01 源映射测量

原始记录：[2026-11-13-linux-source-map.json](2026-11-13-linux-source-map.json)。

同一 release 库通过 C ABI 测量。重开比较使用相同 index/body，仅去掉可选 sourceMap；
路径抽取打开仅记录新实现，不将其解释为改动前后比较。
共 56 项，每项独立进程、30 次计时、2 次预热，系统页缓存未控制。
RSS 包含 Python、Rust、准备和校验。保留每轮 VmRSS 与最终 VmHWM 原值，表中取最大记录值。
Linux 计数存在近似/异步更新，可能出现最终 VmHWM 小于先前 VmRSS，不能保证捕获所有瞬时峰值。
进程间差值不能解释为 SDK 独占分配量。

| 样本 | index 字节 | 原 text 字节 | 带映射 text 字节 | 增量字节 | 区间数 |
|---|---:|---:|---:|---:|---:|
| correctness-1/utf8 | 1651 | 1551 | 2502 | 951 | 65 |
| correctness-1/gbk | 1937 | 1567 | 2094 | 527 | 31 |
| correctness-1/big5 | 2079 | 1663 | 2190 | 527 | 31 |
| correctness-1/cross-shard | 12450 | 12412 | 12557 | 145 | 1 |
| correctness-1/long-line | 47310 | 47225 | 47373 | 148 | 1 |
| correctness-1/epub2 | 2485 | 2264 | 2657 | 393 | 12 |
| correctness-1/epub3 | 2485 | 2264 | 2657 | 393 | 12 |
| performance-64/utf8 | 87551 | 92273 | 144637 | 52364 | 4097 |
| performance-64/gbk | 93017 | 93297 | 120037 | 26740 | 2047 |
| performance-64/big5 | 99287 | 99441 | 126181 | 26740 | 2047 |
| performance-64/cross-shard | 787919 | 787314 | 787467 | 153 | 1 |
| performance-64/long-line | 3017633 | 3014779 | 3014937 | 158 | 1 |
| performance-64/epub2 | 129633 | 129277 | 129678 | 401 | 12 |
| performance-64/epub3 | 129633 | 129277 | 129678 | 401 | 12 |

| 样本 | 方法 | p50 ms | p90 ms | p99 ms | 首次操作后 RSS MiB | 最大记录 RSS MiB |
|---|---|---:|---:|---:|---:|---:|
| correctness-1/utf8 | book.open.path | 0.0347 | 0.0368 | 0.0395 | 26.176 | 26.176 |
| correctness-1/utf8 | book.load.legacy | 0.0117 | 0.0134 | 0.0163 | 26.176 | 26.180 |
| correctness-1/utf8 | book.load.mapped | 0.0359 | 0.0390 | 0.0443 | 26.066 | 26.066 |
| correctness-1/utf8 | book.locate | 0.0061 | 0.0070 | 0.0094 | 26.410 | 26.418 |
| correctness-1/gbk | book.open.path | 0.1097 | 0.1155 | 0.1207 | 26.379 | 26.379 |
| correctness-1/gbk | book.load.legacy | 0.0126 | 0.0137 | 0.0210 | 26.133 | 26.133 |
| correctness-1/gbk | book.load.mapped | 0.0248 | 0.0277 | 0.0312 | 26.094 | 26.094 |
| correctness-1/gbk | book.locate | 0.0060 | 0.0067 | 0.0091 | 26.258 | 26.266 |
| correctness-1/big5 | book.open.path | 0.0963 | 0.1012 | 0.1071 | 26.340 | 26.344 |
| correctness-1/big5 | book.load.legacy | 0.0119 | 0.0136 | 0.0176 | 26.062 | 26.070 |
| correctness-1/big5 | book.load.mapped | 0.0262 | 0.0285 | 0.0321 | 26.176 | 26.176 |
| correctness-1/big5 | book.locate | 0.0063 | 0.0070 | 0.0090 | 26.227 | 26.234 |
| correctness-1/cross-shard | book.open.path | 0.1996 | 0.2046 | 0.2100 | 26.102 | 26.105 |
| correctness-1/cross-shard | book.load.legacy | 0.0176 | 0.0189 | 0.0246 | 26.090 | 26.090 |
| correctness-1/cross-shard | book.load.mapped | 0.0228 | 0.0248 | 0.0328 | 26.129 | 26.129 |
| correctness-1/cross-shard | book.locate | 0.0062 | 0.0071 | 0.0093 | 26.332 | 26.332 |
| correctness-1/long-line | book.open.path | 0.5213 | 0.5763 | 0.5914 | 26.207 | 26.207 |
| correctness-1/long-line | book.load.legacy | 0.0470 | 0.0483 | 0.0525 | 26.078 | 26.082 |
| correctness-1/long-line | book.load.mapped | 0.0640 | 0.0657 | 0.0711 | 26.180 | 26.180 |
| correctness-1/long-line | book.locate | 0.0057 | 0.0065 | 0.0085 | 26.309 | 26.312 |
| correctness-1/epub2 | book.open.path | 0.0427 | 0.0463 | 0.0624 | 26.363 | 26.363 |
| correctness-1/epub2 | book.load.legacy | 0.0126 | 0.0139 | 0.0179 | 26.059 | 26.066 |
| correctness-1/epub2 | book.load.mapped | 0.0197 | 0.0221 | 0.0252 | 26.273 | 26.273 |
| correctness-1/epub2 | book.locate | 0.0075 | 0.0085 | 0.0113 | 26.461 | 26.465 |
| correctness-1/epub3 | book.open.path | 0.0431 | 0.0474 | 0.0538 | 26.234 | 26.234 |
| correctness-1/epub3 | book.load.legacy | 0.0126 | 0.0145 | 0.0245 | 26.141 | 26.145 |
| correctness-1/epub3 | book.load.mapped | 0.0197 | 0.0217 | 0.0262 | 26.312 | 26.312 |
| correctness-1/epub3 | book.locate | 0.0074 | 0.0085 | 0.0107 | 26.289 | 26.293 |
| performance-64/utf8 | book.open.path | 1.7817 | 1.8289 | 1.8446 | 26.863 | 26.863 |
| performance-64/utf8 | book.load.legacy | 0.1035 | 0.1070 | 0.1217 | 26.219 | 26.305 |
| performance-64/utf8 | book.load.mapped | 1.7046 | 1.7237 | 1.7402 | 28.129 | 29.727 |
| performance-64/utf8 | book.locate | 0.0063 | 0.0072 | 0.0097 | 30.098 | 34.066 |
| performance-64/gbk | book.open.path | 6.3387 | 6.3672 | 6.4383 | 26.648 | 26.699 |
| performance-64/gbk | book.load.legacy | 0.1154 | 0.1172 | 0.1227 | 26.312 | 26.398 |
| performance-64/gbk | book.load.mapped | 0.8734 | 0.9953 | 1.0180 | 28.004 | 28.113 |
| performance-64/gbk | book.locate | 0.0061 | 0.0070 | 0.0099 | 32.176 | 34.180 |
| performance-64/big5 | book.open.path | 5.4990 | 5.5280 | 5.5774 | 26.516 | 26.668 |
| performance-64/big5 | book.load.legacy | 0.1001 | 0.1074 | 0.1137 | 26.230 | 26.320 |
| performance-64/big5 | book.load.mapped | 0.8682 | 1.0244 | 1.0405 | 28.164 | 28.234 |
| performance-64/big5 | book.locate | 0.0065 | 0.0075 | 0.0103 | 32.270 | 34.156 |
| performance-64/cross-shard | book.open.path | 12.3928 | 12.4745 | 12.5605 | 27.387 | 28.133 |
| performance-64/cross-shard | book.load.legacy | 0.8031 | 0.8128 | 0.8252 | 28.246 | 30.488 |
| performance-64/cross-shard | book.load.mapped | 1.0387 | 1.0640 | 1.1252 | 28.207 | 30.430 |
| performance-64/cross-shard | book.locate | 0.0063 | 0.0072 | 0.0095 | 28.465 | 28.734 |
| performance-64/long-line | book.open.path | 33.5827 | 33.8943 | 34.1274 | 31.879 | 34.742 |
| performance-64/long-line | book.load.legacy | 2.4777 | 2.5466 | 2.5926 | 34.496 | 34.508 |
| performance-64/long-line | book.load.mapped | 3.5733 | 3.6632 | 3.7126 | 34.766 | 34.781 |
| performance-64/long-line | book.locate | 0.0059 | 0.0069 | 0.0089 | 34.832 | 34.836 |
| performance-64/epub2 | book.open.path | 1.3729 | 1.3899 | 1.4001 | 26.859 | 26.992 |
| performance-64/epub2 | book.load.legacy | 0.1185 | 0.1233 | 0.1325 | 26.223 | 26.477 |
| performance-64/epub2 | book.load.mapped | 0.1788 | 0.1832 | 0.1999 | 26.512 | 26.629 |
| performance-64/epub2 | book.locate | 0.0067 | 0.0082 | 0.0104 | 26.570 | 26.574 |
| performance-64/epub3 | book.open.path | 1.3771 | 1.3895 | 1.4130 | 26.898 | 27.027 |
| performance-64/epub3 | book.load.legacy | 0.1185 | 0.1250 | 0.1298 | 26.227 | 26.480 |
| performance-64/epub3 | book.load.mapped | 0.1833 | 0.1923 | 0.2082 | 26.391 | 26.504 |
| performance-64/epub3 | book.locate | 0.0065 | 0.0078 | 0.0119 | 26.480 | 26.484 |
