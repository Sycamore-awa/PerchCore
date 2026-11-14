# STORE-01 恢复冷启动基准

原始记录：[2026-11-14-linux-recovery.json](2026-11-14-linux-recovery.json)。

每项使用全新 release Rust 进程，只计时 `PathStore::new`；20 次测量、2 次预热。
书库由真实 TXT/EPUB 管线产物复制得到，生成、复制、进程启动和结果校验不在恢复计时内。
系统页缓存未驱逐，因此“冷启动”只表示全新进程的首次恢复，不表示冷块设备 I/O。
RSS 是 worker 整个进程的 VmRSS/VmHWM 最大记录值，不是 SDK 独占分配。

| 书库 | 本数 | 状态 | 产物 MiB | p50 ms | p95 ms | p99 ms | ms/本(p50) | 最大 RSS MiB | 恢复结果 |
|---|---:|---|---:|---:|---:|---:|---:|---:|---|
| mixed-small | 1 | 健康 | 0.004 | 0.096 | 0.106 | 0.109 | 0.0963 | 3.426 | ready=1, indexing=0 |
| mixed-small | 10 | 健康 | 0.047 | 0.375 | 0.416 | 0.418 | 0.0375 | 3.434 | ready=10, indexing=0 |
| mixed-small | 100 | 健康 | 0.467 | 3.028 | 3.262 | 4.760 | 0.0303 | 3.453 | ready=100, indexing=0 |
| mixed-small | 1000 | 健康 | 4.674 | 29.391 | 30.073 | 30.765 | 0.0294 | 3.809 | ready=1000, indexing=0 |
| mixed-small | 1000 | 1 本索引损坏 | 4.672 | 29.252 | 29.777 | 30.103 | 0.0293 | 3.805 | ready=999, indexing=1 |
| mixed-scale-64 | 1 | 健康 | 5.753 | 4.593 | 4.861 | 4.874 | 4.5926 | 11.809 | ready=1, indexing=0 |
| mixed-scale-64 | 10 | 健康 | 30.004 | 21.407 | 22.783 | 23.177 | 2.1407 | 14.820 | ready=10, indexing=0 |
| mixed-scale-64 | 100 | 健康 | 300.045 | 197.949 | 212.550 | 216.869 | 1.9795 | 14.828 | ready=100, indexing=0 |

## 口径

- `mixed-small`：正确性集的 UTF-8 TXT 与 EPUB3 Ready 产物交替组成。
- `mixed-scale-64`：性能集的 long-line TXT 与 EPUB3 Ready 产物交替组成。
- 损坏案例仅覆盖一本书的 `index.bin`；该书稳定降级到 indexing，其余书保持 Ready。
- 报告不设自动性能门槛；宿主 Linux 数据用于判断是否值得引入 manifest/延迟审计，不代表手机预算。
