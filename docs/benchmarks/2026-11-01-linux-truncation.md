# PerchCore BASE-02 measurement

Generated: 2026-11-01T03:17:12.359033+00:00

Corpus: `performance`, scale=256; iterations=3; warmups=1.

Cache: OS cache uncontrolled; setup/validation and warmups read the same input. No cold-cache claim.

Peak RSS covers the entire isolated process, including setup, validation and runtime. Values from different language runtimes include different overhead.

| Client | Sample | Stage | p50 ms | p90 ms | p99 ms | Peak RSS MiB |
|---|---|---|---:|---:|---:|---:|
| rust | long-line | extract | 14.2556 | 14.4214 | 14.4214 | 61.07 |
| rust | long-line | index | 104.6796 | 104.6893 | 104.6893 | 59.21 |
| rust | long-line | query.hit | 0.0006 | 0.0012 | 0.0012 | 59.18 |
| rust | long-line | pipeline | 145.0636 | 146.1142 | 146.1142 | 87.13 |
| ctypes | long-line | book.open | 170.4427 | 171.0585 | 171.0585 | 206.37 |
| ctypes | long-line | book.search | 0.0072 | 0.0113 | 0.0113 | 196.82 |
| dart | long-line | book.open | 421.3635 | 422.0345 | 422.0345 | 246.88 |
| dart | long-line | book.search | 0.0016 | 0.0027 | 0.0027 | 246.18 |
