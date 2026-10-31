# perch-core

PerchCore 的统一 Rust 入口，重导出解析、文本、索引、模型、导入和 PDF 结构模块。

默认不启用同步合并与卷页几何，可通过 `sync`、`pagecurl` feature 选择。
它不依赖 FFI、Flutter 或渲染器。也可以直接依赖 `perch-txt` 等单个模块。

```rust
use perch_core::{index::Index, txt::TxtDocument};

let document = TxtDocument::from_bytes("中文文字".as_bytes());
let index = Index::build(document.text(), 4096);
assert_eq!(index.search("文字").len(), 1);
```

完整说明见[项目 README](../../README.md)，许可证为 [MIT](../../LICENSE)。
