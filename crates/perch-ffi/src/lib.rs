//! FFI 边界：**单一入口** + **JSON 契约**。
//!
//! # 为什么是单一入口
//!
//! ABI v1 保留两个原始符号，并从 JSON API v1.2 起增加上下文调用入口：
//!
//! ```text
//! perch_call(method: utf8, args: json) -> json   // 返回堆字符串指针，调用方负责释放
//! perch_context_call(context, method, args) -> json
//! perch_string_free(ptr)
//! ```
//!
//! 好处：新增能力不需要改 Dart 侧的绑定代码，只要在 [`dispatch`] 里加一条分支；
//! 也不会因为函数签名变化而让两边的 ABI 悄悄失配。
//!
//! # 为什么用 JSON 而不是 `flutter_rust_bridge`
//!
//! `flutter_rust_bridge` 的代码生成更优雅，但它**只能在没有 Mac 的情况下写、无法验证**。
//! 而 JSON 契约可以用 `dart:ffi` 在本机直接加载 `.so` 调用 —— 整条边界都能在 Linux 上跑通。
//!
//! 契约（method 名 + JSON 结构）是稳定层，将来换成 FRB 只是换传输层，上层不用改。
//!
//! # 约定
//!
//! - 入参永远是 UTF-8 JSON（无参数时传 `{}`）
//! - 出参永远是 UTF-8 JSON；成功是业务对象，失败是 `{"error":{"code":..,"message":..}}`
//! - **不 panic**：任何错误都转成 `error` 对象。这是"崩溃率 < 0.1%"目标的边界防线
//! - 字节数组用十六进制字符串传递，避免 base64 依赖（也便于日志里看）
//!
//! # ⚠️ unsafe 的唯一集中地
//!
//! 本 crate 是工作区里**唯一**不继承 `forbid(unsafe_code)` 的 crate —— 跨语言边界必须
//! 处理裸指针。所有 `unsafe` 都集中在文件末尾的两个导出函数里，业务逻辑一律 safe。

#![allow(unsafe_code)]

use std::ffi::CString;

use perch_import::Fingerprint;
use perch_model::{Format, Locator, ModelError};
use perch_pagecurl::{Corner, CurlInput, Rect, Vec2, curl};
use serde_json::{Value, json};

/// 业务版本号，供 Dart 侧做兼容性断言。
const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 现有 C 符号及所有权规则的 ABI 版本。
const ABI_VERSION: u16 = 1;
/// JSON 契约 major：破坏现有方法才升级。
const API_MAJOR: u16 = 1;
/// JSON 契约 minor：搜索命中到 TXT/EPUB 源文本的精确区间映射。
const API_MINOR: u16 = 4;
/// 旧 `perch_call` 使用的隐式上下文；不暴露给调用方，也不能销毁。
const LEGACY_CONTEXT: u64 = 0;

/// 同时最多允许打开多少本书的索引。
///
/// 手机上不能无限开：每个索引都持有整本正文（[`perch_jobs::BookText`]）+
/// 倒排表。取 8 是为了"书架 + 阅读器 + 搜索面板"这类场景够用，
/// 同时把最坏内存占用兜住。
const MAX_OPEN_BOOKS: usize = 8;

/// 同时最多允许多少个 TXT 阅读句柄。
///
/// 与 [`MAX_OPEN_BOOKS`] **分开计数**（见 [`Registry`]）：阅读器同一时刻只需要
/// 1 个 TXT，留到 4 是给"桌面/平板分栏 + 搜索面板"的余量。分开计数的意义是
/// **后台建索引的任务再多，也挤不掉阅读句柄**（`docs/architecture.md`「句柄与生命周期」的要求）。
const MAX_OPEN_TXT: usize = 4;

/// 一次 `txt.slice` 最多返回多少字符。
///
/// 阅读器一次只画一屏（几 KB），给到 2 万字符已经远超需要。设上限是为了
/// **让"误把整本书取回来"这件事明确失败**，而不是悄悄传一个 10MB 的字符串。
const MAX_TXT_SLICE_CHARS: u64 = 20_000;

/// 一次搜索默认最多返回多少条。
///
/// 搜索面板一次也展示不了更多；上限小一点也能让大书上的单次调用更快。
const DEFAULT_SEARCH_LIMIT: usize = 50;

/// 一本书的已打开状态：正文 + 章节目录 + 倒排索引。
struct OpenBook {
    text: perch_jobs::MappedBookText,
    index: perch_index::Index,
}

type SharedBook = std::sync::Arc<OpenBook>;
type SharedTxt = std::sync::Arc<perch_txt::TxtDocument>;
type SharedPipeline = std::sync::Arc<std::sync::Mutex<perch_jobs::Pipeline<perch_jobs::PathStore>>>;

/// 句柄表里的一类东西。
///
/// **三类分开**：`book.*` 走索引/搜索（阶段③的产物），`txt.*` 走阅读路径，
/// `pipeline.*` 是导入管线的驱动句柄。各有独立配额，也各有独立的错误码——
/// "书太多"、"阅读句柄被后台任务占满"、"管线只能有一个"是三种完全不同的故障，
/// 混成一个码会让排查和降级都变难（`docs/architecture.md`「句柄与生命周期」、`docs/storage.md`「pipeline FFI」）。
enum OpenDoc {
    /// 已建索引的书：`book.open` 产出。
    Book(SharedBook),
    /// 已解码的 TXT：`txt.open` 产出，**不建倒排索引**。
    Txt(SharedTxt),
    /// 导入管线：`pipeline.open` 产出（调度 + 按路径读写的存储）。
    Pipeline(SharedPipeline),
}

impl OpenDoc {
    /// 类型名，只用于错误信息。
    fn kind(&self) -> &'static str {
        match self {
            Self::Book(_) => "book",
            Self::Txt(_) => "txt",
            Self::Pipeline(_) => "pipeline",
        }
    }
}

/// 有状态对象的句柄表。
///
/// Dart 侧拿到的是 `u64` 句柄；真正的 Rust 对象留在这里。
/// **句柄自增且永不复用**——这样"关掉之后又拿旧句柄调用"会得到
/// `BOOK_BAD_HANDLE`，而不是误操作到另一个对象上。
#[derive(Default)]
struct ContextRegistry {
    open: std::collections::HashMap<u64, OpenDoc>,
    /// 当前打开的索引句柄数。
    books: usize,
    /// 当前打开的 TXT 句柄数。
    txts: usize,
    /// 当前打开的管线句柄数。
    pipelines: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Released {
    books: usize,
    txts: usize,
    pipelines: usize,
}

struct ClosedContext {
    released: Released,
    objects: Vec<OpenDoc>,
}

struct Registry {
    next_context: Option<u64>,
    next_handle: Option<u64>,
    legacy: ContextRegistry,
    contexts: std::collections::HashMap<u64, ContextRegistry>,
    owners: std::collections::HashMap<u64, u64>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            next_context: Some(1),
            next_handle: Some(0),
            legacy: ContextRegistry::default(),
            contexts: std::collections::HashMap::new(),
            owners: std::collections::HashMap::new(),
        }
    }
}

static REGISTRY: std::sync::OnceLock<std::sync::Mutex<Registry>> = std::sync::OnceLock::new();

/// 借用句柄表。
///
/// 互斥锁中毒时用 `into_inner()` 继续——`Registry` 里没有会被破坏的不变量，
/// 为了"一次 panic"就让后续所有调用永久失败不值得。
fn registry() -> std::sync::MutexGuard<'static, Registry> {
    let mutex = REGISTRY.get_or_init(|| std::sync::Mutex::new(Registry::default()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Registry {
    fn create_context(&mut self) -> Result<u64, FfiError> {
        let context = self
            .next_context
            .ok_or_else(|| FfiError::new("CONTEXT_ID_EXHAUSTED", "context 编号空间已经耗尽"))?;
        self.next_context = context.checked_add(1);
        self.contexts.insert(context, ContextRegistry::default());
        Ok(context)
    }

    fn ensure_context(&self, context: u64) -> Result<(), FfiError> {
        if context == LEGACY_CONTEXT || self.contexts.contains_key(&context) {
            Ok(())
        } else {
            Err(FfiError::new(
                "CONTEXT_BAD_HANDLE",
                format!("上下文 {context} 不存在（可能已 close 或从未 open）"),
            ))
        }
    }

    fn context(&self, context: u64) -> Result<&ContextRegistry, FfiError> {
        if context == LEGACY_CONTEXT {
            Ok(&self.legacy)
        } else {
            self.contexts.get(&context).ok_or_else(|| {
                FfiError::new(
                    "CONTEXT_BAD_HANDLE",
                    format!("上下文 {context} 不存在（可能已 close 或从未 open）"),
                )
            })
        }
    }

    fn context_mut(&mut self, context: u64) -> Result<&mut ContextRegistry, FfiError> {
        if context == LEGACY_CONTEXT {
            Ok(&mut self.legacy)
        } else {
            self.contexts.get_mut(&context).ok_or_else(|| {
                FfiError::new(
                    "CONTEXT_BAD_HANDLE",
                    format!("上下文 {context} 不存在（可能已 close 或从未 open）"),
                )
            })
        }
    }

    fn close_context(&mut self, context: u64) -> Result<Option<ClosedContext>, FfiError> {
        if context == LEGACY_CONTEXT {
            return Err(FfiError::new(
                "CONTEXT_LEGACY",
                "legacy 上下文 0 不能显式关闭",
            ));
        }
        let Some(closed) = self.contexts.remove(&context) else {
            return Ok(None);
        };
        for handle in closed.open.keys() {
            self.owners.remove(handle);
        }
        Ok(Some(ClosedContext {
            released: Released {
                books: closed.books,
                txts: closed.txts,
                pipelines: closed.pipelines,
            },
            objects: closed.open.into_values().collect(),
        }))
    }

    fn ensure_owner(&self, context: u64, handle: u64) -> Result<(), FfiError> {
        if let Some(owner) = self.owners.get(&handle)
            && *owner != context
        {
            return Err(FfiError::new(
                "CONTEXT_HANDLE_MISMATCH",
                format!("句柄 {handle} 属于上下文 {owner}，不能由上下文 {context} 使用"),
            ));
        }
        Ok(())
    }

    /// 放进一个对象，返回进程内全局唯一且永不复用的句柄。
    fn push(&mut self, context: u64, doc: OpenDoc) -> Result<u64, FfiError> {
        self.ensure_context(context)?;
        let handle = self
            .next_handle
            .ok_or_else(|| FfiError::new("HANDLE_ID_EXHAUSTED", "对象句柄编号空间已经耗尽"))?;
        self.next_handle = handle.checked_add(1);
        self.context_mut(context)?.open.insert(handle, doc);
        self.owners.insert(handle, context);
        Ok(handle)
    }

    /// 登记一个索引句柄（受 [`MAX_OPEN_BOOKS`] 限制）。
    fn insert_book(&mut self, context: u64, book: impl Into<SharedBook>) -> Result<u64, FfiError> {
        if self.context(context)?.books >= MAX_OPEN_BOOKS {
            return Err(FfiError::new(
                "BOOK_TOO_MANY_OPEN",
                format!("同时最多打开 {MAX_OPEN_BOOKS} 本书的索引，请先 book.close"),
            ));
        }
        let handle = self.push(context, OpenDoc::Book(book.into()))?;
        self.context_mut(context)?.books += 1;
        Ok(handle)
    }

    /// 登记一个 TXT 阅读句柄（受 [`MAX_OPEN_TXT`] 限制）。
    fn insert_txt(
        &mut self,
        context: u64,
        document: impl Into<SharedTxt>,
    ) -> Result<u64, FfiError> {
        if self.context(context)?.txts >= MAX_OPEN_TXT {
            return Err(FfiError::new(
                "TXT_TOO_MANY_OPEN",
                format!("同时最多打开 {MAX_OPEN_TXT} 个 TXT，请先 txt.close"),
            ));
        }
        let handle = self.push(context, OpenDoc::Txt(document.into()))?;
        self.context_mut(context)?.txts += 1;
        Ok(handle)
    }

    /// 登记一个管线句柄。**只允许 1 个**：管线自己就是"全局后台任务"，
    /// 开两个只会让同一本书被两条队列同时处理。
    fn insert_pipeline(
        &mut self,
        context: u64,
        pipeline: impl Into<SharedPipeline>,
    ) -> Result<u64, FfiError> {
        if self.context(context)?.pipelines >= 1 {
            return Err(FfiError::new(
                "PIPELINE_TOO_MANY_OPEN",
                "同时只允许开一条导入管线，请先 pipeline.close",
            ));
        }
        let handle = self.push(context, OpenDoc::Pipeline(pipeline.into()))?;
        self.context_mut(context)?.pipelines += 1;
        Ok(handle)
    }

    /// 摘掉一个句柄，并维护计数。
    fn remove(&mut self, context: u64, handle: u64) -> Result<Option<OpenDoc>, FfiError> {
        self.ensure_context(context)?;
        self.ensure_owner(context, handle)?;
        let doc = self.context_mut(context)?.open.remove(&handle);
        let Some(doc) = doc else {
            return Ok(None);
        };
        match doc {
            OpenDoc::Book(_) => {
                let registry = self.context_mut(context)?;
                registry.books = registry.books.saturating_sub(1);
            }
            OpenDoc::Txt(_) => {
                let registry = self.context_mut(context)?;
                registry.txts = registry.txts.saturating_sub(1);
            }
            OpenDoc::Pipeline(_) => {
                let registry = self.context_mut(context)?;
                registry.pipelines = registry.pipelines.saturating_sub(1);
            }
        }
        self.owners.remove(&handle);
        Ok(Some(doc))
    }

    /// 借一个索引句柄；句柄不存在或**类型不对**时给不同的错误码。
    fn book(&self, context: u64, handle: u64) -> Result<SharedBook, FfiError> {
        self.ensure_owner(context, handle)?;
        match self.context(context)?.open.get(&handle) {
            Some(OpenDoc::Book(book)) => Ok(std::sync::Arc::clone(book)),
            Some(other) => Err(FfiError::new(
                "HANDLE_WRONG_KIND",
                format!(
                    "句柄 {handle} 是 {} 类型，book.* 用不了；该用 {}.close 释放",
                    other.kind(),
                    other.kind()
                ),
            )),
            None => Err(FfiError::new(
                "BOOK_BAD_HANDLE",
                format!("句柄 {handle} 不存在（可能已 close 或从未 open）"),
            )),
        }
    }

    /// 借一个 TXT 句柄。
    fn txt(&self, context: u64, handle: u64) -> Result<SharedTxt, FfiError> {
        self.ensure_owner(context, handle)?;
        match self.context(context)?.open.get(&handle) {
            Some(OpenDoc::Txt(document)) => Ok(std::sync::Arc::clone(document)),
            Some(other) => Err(FfiError::new(
                "HANDLE_WRONG_KIND",
                format!(
                    "句柄 {handle} 是 {} 类型，txt.* 用不了；该用 {}.close 释放",
                    other.kind(),
                    other.kind()
                ),
            )),
            None => Err(FfiError::new(
                "TXT_BAD_HANDLE",
                format!("句柄 {handle} 不存在（可能已 close 或从未 open）"),
            )),
        }
    }

    /// 取得一个管线租约。同一管线的操作由对象自己的锁串行，
    /// 不再占用根注册表锁。
    fn pipeline(&self, context: u64, handle: u64) -> Result<SharedPipeline, FfiError> {
        self.ensure_owner(context, handle)?;
        match self.context(context)?.open.get(&handle) {
            Some(OpenDoc::Pipeline(pipeline)) => Ok(std::sync::Arc::clone(pipeline)),
            Some(other) => Err(FfiError::new(
                "HANDLE_WRONG_KIND",
                format!(
                    "句柄 {handle} 是 {} 类型，pipeline.* 用不了；该用 {}.close 释放",
                    other.kind(),
                    other.kind()
                ),
            )),
            None => Err(FfiError::new(
                "PIPELINE_BAD_HANDLE",
                format!("句柄 {handle} 不存在（可能已 close 或从未 open）"),
            )),
        }
    }

    /// 判断句柄是不是某一类（给 `*.close` 用，避免关错类型）。
    fn kind_of(&self, context: u64, handle: u64) -> Result<Option<&'static str>, FfiError> {
        self.ensure_owner(context, handle)?;
        Ok(self.context(context)?.open.get(&handle).map(OpenDoc::kind))
    }
}

fn lock_pipeline(
    pipeline: &SharedPipeline,
) -> std::sync::MutexGuard<'_, perch_jobs::Pipeline<perch_jobs::PathStore>> {
    match pipeline.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 统一错误对象。
#[derive(Debug)]
struct FfiError {
    code: &'static str,
    message: String,
}

impl FfiError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn bad_args(message: impl Into<String>) -> Self {
        Self::new("FFI_BAD_ARGS", message)
    }

    fn from_model(error: &ModelError) -> Self {
        Self::new(error.code(), error.to_string())
    }

    fn from_sync(error: &perch_sync::SyncError) -> Self {
        Self::new(error.code(), error.to_string())
    }

    fn to_value(&self) -> Value {
        json!({ "error": { "code": self.code, "message": self.message } })
    }
}

/// 主分发：方法名 + JSON 入参 → JSON 出参。
pub fn dispatch(method: &str, args: &str) -> Value {
    dispatch_in_context(LEGACY_CONTEXT, method, args)
}

fn dispatch_in_context(context: u64, method: &str, args: &str) -> Value {
    match call(context, method, args) {
        Ok(value) => value,
        Err(error) => error.to_value(),
    }
}

fn call(context: u64, method: &str, args: &str) -> Result<Value, FfiError> {
    if context != LEGACY_CONTEXT {
        registry().ensure_context(context)?;
        if matches!(method, "context.open" | "context.close") {
            return Err(FfiError::new(
                "CONTEXT_METHOD_SCOPE",
                format!("{method} 只能通过 perch_call 调用"),
            ));
        }
    }
    let args: Value = if args.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args).map_err(|err| FfiError::bad_args(err.to_string()))?
    };

    match method {
        "version" => Ok(json!({ "core": CORE_VERSION })),
        "compatibility" => Ok(json!({
            "core": CORE_VERSION,
            "abi": ABI_VERSION,
            "api": { "major": API_MAJOR, "minor": API_MINOR },
            "locator": {
                "read": [perch_model::LOCATOR_VERSION],
                "write": perch_model::LOCATOR_VERSION,
            },
            "artifacts": {
                "index": {
                    "read": [perch_index::INDEX_FORMAT_VERSION],
                    "write": perch_index::INDEX_FORMAT_VERSION,
                },
                "text": {
                    "read": [
                        perch_jobs::BOOK_TEXT_LEGACY_VERSION,
                        perch_jobs::BOOK_TEXT_VERSION,
                    ],
                    "write": perch_jobs::BOOK_TEXT_VERSION,
                },
            },
        })),
        "context.open" => {
            let context = registry().create_context()?;
            Ok(json!({ "context": context }))
        }
        "context.close" => {
            let target = required_u64(&args, "context")?;
            let closed = registry().close_context(target)?;
            match closed {
                Some(closed) => {
                    let released = closed.released;
                    // 大对象的最后一个 Arc 可能在这里释放；此时根锁已经归还。
                    drop(closed.objects);
                    Ok(json!({
                        "closed": true,
                        "released": {
                            "books": released.books,
                            "txts": released.txts,
                            "pipelines": released.pipelines,
                        },
                    }))
                }
                None => Ok(json!({
                    "closed": false,
                    "released": { "books": 0, "txts": 0, "pipelines": 0 },
                })),
            }
        }

        // ---- 定位（perch-model）----
        "locator.validate" => {
            let locator: Locator =
                serde_json::from_value(args).map_err(|err| FfiError::bad_args(err.to_string()))?;
            locator
                .validate()
                .map_err(|err| FfiError::from_model(&err))?;
            Ok(json!({ "valid": true, "format": locator.format().map(|f| f.as_str()) }))
        }

        // ---- 导入（perch-import）----
        "fingerprint" => {
            let size = args
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| FfiError::bad_args("缺少 size"))?;
            let head = hex_to_bytes(required_str(&args, "head_hex")?)?;
            let tail = hex_to_bytes(required_str(&args, "tail_hex")?)?;
            let fingerprint = Fingerprint::compute(size, &head, &tail);
            Ok(json!({ "fingerprint": fingerprint.to_hex() }))
        }

        // ---- 阶段②精修（perch-jobs）----
        "jobs.refine" => {
            let bytes = file_or_hex(&args, "JOBS_IO")?;
            let max_chars = args
                .get("max_chars")
                .and_then(Value::as_u64)
                .unwrap_or(perch_jobs::ExtractOptions::default().max_chars);
            let format = format_arg(&args)?.unwrap_or_else(|| perch_jobs::sniff_format(&bytes));
            let refinement =
                perch_jobs::refine(&bytes, format, perch_jobs::ExtractOptions { max_chars })
                    .map_err(|error| FfiError::new(error.code(), error.to_string()))?;
            serde_json::to_value(&refinement).map_err(|error| FfiError::bad_args(error.to_string()))
        }

        // 打开一本书：抽正文 + 建索引，返回句柄。后续搜索复用这个索引。
        "book.open" => {
            let bytes = file_or_hex(&args, "BOOK_OPEN_IO")?;
            let chunk_chars = args
                .get("chunk_chars")
                .and_then(Value::as_u64)
                .unwrap_or(perch_index::DEFAULT_CHUNK_CHARS);
            let format = format_arg(&args)?.unwrap_or_else(|| perch_jobs::sniff_format(&bytes));
            let text = perch_jobs::extract_text_with_source_map(
                &bytes,
                format,
                perch_jobs::ExtractOptions::default(),
            )
            .map_err(|error| FfiError::new(error.code(), error.to_string()))?;
            let index = perch_jobs::index_book(text.book_text(), chunk_chars);
            let stats = index.stats();
            let char_count = text.char_count();
            let chapter_count = text.book_text().chapters.len();
            let truncated = text.book_text().truncated;
            let book = std::sync::Arc::new(OpenBook { text, index });
            let handle = registry().insert_book(context, std::sync::Arc::clone(&book))?;
            Ok(json!({
                "handle": handle,
                "charCount": char_count,
                "chapterCount": chapter_count,
                "chunks": stats.chunks,
                "tokens": stats.tokens,
                "truncated": truncated,
            }))
        }

        // Load stage-3 artifacts without extracting source text or rebuilding the index.
        "book.load" => {
            let index_path = required_str(&args, "index_path")?;
            let text_path = required_str(&args, "text_path")?;
            let index = perch_index::Index::load_from(index_path)
                .map_err(|error| FfiError::new(error.code(), error.to_string()))?;
            let text_json = std::fs::read(text_path).map_err(|error| {
                FfiError::new("BOOK_LOAD_IO", format!("读取搜索定位文本失败: {error}"))
            })?;
            let text = perch_jobs::mapped_book_text_from_slice(&text_json)
                .map_err(|error| FfiError::new(error.code(), error.to_string()))?;
            if index.text() != text.book_text().text {
                return Err(FfiError::new(
                    "BOOK_INDEX_MISMATCH",
                    "index.bin 与 text.json 正文不一致",
                ));
            }
            let stats = index.stats();
            let char_count = text.char_count();
            let chapter_count = text.book_text().chapters.len();
            let truncated = text.book_text().truncated;
            let book = std::sync::Arc::new(OpenBook { text, index });
            let handle = registry().insert_book(context, std::sync::Arc::clone(&book))?;
            Ok(json!({
                "handle": handle,
                "charCount": char_count,
                "chapterCount": chapter_count,
                "chunks": stats.chunks,
                "tokens": stats.tokens,
                "truncated": truncated,
            }))
        }

        // 在已打开的索引里检索；命中会还原成粗定位 Locator。
        "book.search" => {
            let handle = required_u64(&args, "handle")?;
            let query = required_str(&args, "query")?;
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(DEFAULT_SEARCH_LIMIT, |n| n as usize);
            let book = registry().book(context, handle)?;
            let hits: Vec<Value> = book
                .index
                .search(query)
                .into_iter()
                .take(limit)
                .map(|hit| {
                    let locator = perch_jobs::locate_hit(book.text.book_text(), &hit);
                    json!({
                        "charStart": hit.char_start,
                        "charEnd": hit.char_end,
                        "snippet": hit.snippet,
                        "locator": locator,
                    })
                })
                .collect();
            Ok(json!({ "hits": hits }))
        }

        // Resolve only the selected hit; keep the existing search response small.
        "book.locate" => {
            let handle = required_u64(&args, "handle")?;
            let book = registry().book(context, handle)?;
            let bad_range = || {
                let error = perch_jobs::SourceMapError::BadRange;
                FfiError::new(error.code(), error.to_string())
            };
            let char_start = args
                .get("char_start")
                .and_then(Value::as_u64)
                .ok_or_else(bad_range)?;
            let char_end = args
                .get("char_end")
                .and_then(Value::as_u64)
                .ok_or_else(bad_range)?;
            let location = perch_jobs::locate_source_range(&book.text, char_start, char_end)
                .map_err(|error| FfiError::new(error.code(), error.to_string()))?;
            serde_json::to_value(location).map_err(|error| FfiError::bad_args(error.to_string()))
        }

        // 释放一本书的索引。
        "book.close" => {
            let handle = required_u64(&args, "handle")?;
            let mut guard = registry();
            // 类型不对就不关：拿 txt 句柄调 book.close 是调用方的 bug，
            // 静默关掉会让真正的持有者后续拿到 BAD_HANDLE，很难定位。
            if let Some(kind) = guard.kind_of(context, handle)?
                && kind != "book"
            {
                return Err(FfiError::new(
                    "HANDLE_WRONG_KIND",
                    format!("句柄 {handle} 是 {kind} 类型，请用 {kind}.close"),
                ));
            }
            let removed = guard.remove(context, handle)?;
            drop(guard);
            let removed = removed.is_some();
            Ok(json!({ "closed": removed }))
        }

        // ---- TXT 阅读路径（perch-txt）----
        //
        // 为什么句柄化：`txt.analyze` 要 `bytes_hex`（10MB 文件 = 20MB 十六进制字符串）
        // 而且把**全文**当 JSON 字符串吐回来。阅读路径只需要"打开一次 + 每次取一屏"，
        // 所以走 `txt.open` + `txt.slice`（`docs/architecture.md`「句柄与生命周期」）。
        "txt.open" => {
            let document = txt_from_path(&args)?;
            let meta = document.meta();
            let chars = meta.chars;
            let lines = meta.lines;
            // `encoding` 是 `&'static str`，直接拷贝引用即可（不是 String）
            let encoding = meta.encoding;
            let certain = meta.certain_encoding;
            let had_errors = meta.had_decode_errors;
            let document = std::sync::Arc::new(document);
            let handle = registry().insert_txt(context, std::sync::Arc::clone(&document))?;
            Ok(json!({
                "handle": handle,
                "chars": chars,
                "lines": lines,
                "encoding": encoding,
                "certain": certain,
                "hadErrors": had_errors,
            }))
        }

        // 取一屏文本。`start` 是字符偏移（跨端最稳的锚点就是它），`chars` 必填且有上限。
        "txt.slice" => {
            let handle = required_u64(&args, "handle")?;
            let start = args.get("start").and_then(Value::as_u64).unwrap_or(0);
            let want = required_u64(&args, "chars")?;
            if want == 0 || want > MAX_TXT_SLICE_CHARS {
                return Err(FfiError::new(
                    "TXT_SLICE_BAD_SIZE",
                    format!("chars 必须在 1..={MAX_TXT_SLICE_CHARS} 之间（收到 {want}）"),
                ));
            }
            let document = registry().txt(context, handle)?;
            let total = document.char_count();
            if start > total {
                return Err(FfiError::new(
                    "TXT_OUT_OF_RANGE",
                    format!("start {start} 超出总字符数 {total}"),
                ));
            }
            // 末尾不足一屏是正常情况（最后一页），按实际长度返回
            let end = start.saturating_add(want).min(total);
            let text = document.text_range(start..end).ok_or_else(|| {
                FfiError::new(
                    "TXT_OUT_OF_RANGE",
                    format!("取不到 [{start}, {end}) 这一段（总字符数 {total}）"),
                )
            })?;
            Ok(json!({
                "text": text,
                "start": start,
                "end": end,
                "chars": end.saturating_sub(start),
                "total": total,
            }))
        }

        // 字符偏移 → Locator（进度落库用这个）。
        "txt.locate" => {
            let handle = required_u64(&args, "handle")?;
            let char_offset = required_u64(&args, "char")?;
            let document = registry().txt(context, handle)?;
            let locator = document.locator(char_offset).ok_or_else(|| {
                FfiError::new(
                    "TXT_OUT_OF_RANGE",
                    format!(
                        "字符偏移 {char_offset} 超出总字符数 {}",
                        document.char_count()
                    ),
                )
            })?;
            Ok(json!({ "locator": locator }))
        }

        // Locator → 字符偏移。**解析不出来不是错误**：返回 `{"char": null}`，
        // 让人调用方决定是不是退回 progression（三级回退链在文档里）。
        "txt.offset" => {
            let handle = required_u64(&args, "handle")?;
            let locator: Locator = serde_json::from_value(
                args.get("locator")
                    .cloned()
                    .ok_or_else(|| FfiError::bad_args("缺少 locator"))?,
            )
            .map_err(|error| FfiError::bad_args(format!("locator: {error}")))?;
            let document = registry().txt(context, handle)?;
            match document.char_offset_of_locator(&locator) {
                Some(char_offset) => Ok(json!({ "char": char_offset, "resolved": true })),
                None => Ok(json!({ "char": null, "resolved": false })),
            }
        }

        // 字符偏移 → 行号与该行的字符区间（阅读器排版/目录用）。
        "txt.line" => {
            let handle = required_u64(&args, "handle")?;
            let char_offset = required_u64(&args, "char")?;
            let document = registry().txt(context, handle)?;
            let line = document.line_index_at(char_offset).ok_or_else(|| {
                FfiError::new(
                    "TXT_OUT_OF_RANGE",
                    format!(
                        "字符偏移 {char_offset} 超出总字符数 {}",
                        document.char_count()
                    ),
                )
            })?;
            let range = document.line_char_range(line);
            Ok(json!({
                "line": line,
                "start": range.as_ref().map(|r| r.start),
                "end": range.as_ref().map(|r| r.end),
                "totalLines": document.line_count(),
            }))
        }

        // ---- 导入管线（perch-jobs::Pipeline + PathStore，见 docs/storage.md）----
        //
        // 设计要点：**平台只给路径与产物目录**，读文件、写产物、调度都在 Rust 里。
        // 不做跨语言回调——那样只能在设备上验，而这条链路在 Linux 上用临时目录
        // 就能完整跑通（`perch-jobs` 的 store 单测就是这么做的）。
        "pipeline.open" => {
            let out_dir = required_str(&args, "out_dir")?;
            let store = perch_jobs::PathStore::new(out_dir)
                .map_err(|error| FfiError::new("PIPELINE_IO", error))?;
            let recovered: Vec<Value> = store
                .recovered()
                .iter()
                .map(|item| {
                    json!({
                        "book": item.book,
                        "state": item.state,
                        "attempts": item.attempts,
                        "failedStage": item.failed_stage,
                        "reason": item.reason,
                        "retryAtMs": item.retry_at_ms,
                    })
                })
                .collect();
            let policy = policy_arg(&args)?;
            let max_chars = args
                .get("max_chars")
                .and_then(Value::as_u64)
                .unwrap_or(perch_jobs::ExtractOptions::default().max_chars);
            let chunk_chars = args
                .get("chunk_chars")
                .and_then(Value::as_u64)
                .unwrap_or(perch_index::DEFAULT_CHUNK_CHARS);
            let pipeline = perch_jobs::Pipeline::new(
                store,
                policy,
                perch_jobs::ExtractOptions { max_chars },
                chunk_chars,
            )
            .map_err(|error| FfiError::new("PIPELINE_POLICY", error.to_string()))?;
            let pipeline = std::sync::Arc::new(std::sync::Mutex::new(pipeline));
            let handle = registry().insert_pipeline(context, std::sync::Arc::clone(&pipeline))?;
            Ok(json!({ "handle": handle, "recovered": recovered }))
        }

        // 登记一本书（阶段①完成后调用）。format **必须显式给**：
        // 嗅探会把一个损坏的 EPUB 当纯文本收下，静默产出一本乱码书。
        // 已被恢复出来的状态优先——崩溃前停在哪就接着做哪。
        "pipeline.enqueue" => {
            let handle = required_u64(&args, "handle")?;
            let book = book_arg(&args)?;
            let path = required_str(&args, "path")?;
            let format = format_arg(&args)?.ok_or_else(|| {
                FfiError::bad_args("必须显式给 format（嗅探会把损坏的 EPUB 当纯文本）")
            })?;
            let priority = priority_arg(&args)?;

            let pipeline = registry().pipeline(context, handle)?;
            let mut pipeline = lock_pipeline(&pipeline);
            pipeline.store_mut().register(book, path, format);
            let recorded = pipeline.store().state(book);
            let state = match &recorded {
                Some(item) => item.state,
                None => state_arg(&args)?,
            };
            let attempts = recorded.as_ref().map_or(0, |item| item.attempts);
            let stage = recorded.as_ref().and_then(|item| item.failed_stage);
            let retry_at = recorded.as_ref().and_then(|item| item.retry_at_ms);
            let outcome = pipeline.restore(book, state, priority, attempts, stage, retry_at);
            Ok(json!({
                "outcome": enqueue_outcome_str(outcome),
                "stage": enqueue_outcome_stage(outcome),
                "state": state,
                "attempts": attempts,
            }))
        }

        // 做**一个**任务。一次一个，平台可以随时停手（进后台、让位给阅读）。
        "pipeline.step" => {
            let handle = required_u64(&args, "handle")?;
            let now = args
                .get("now_ms")
                .and_then(Value::as_u64)
                .unwrap_or_else(now_ms_now);
            let gate = gate_arg(&args)?;

            let pipeline = registry().pipeline(context, handle)?;
            let mut pipeline = lock_pipeline(&pipeline);
            let outcome = pipeline.step(now, gate);
            if let perch_jobs::StepOutcome::IoFailed { error, .. } = &outcome {
                return Err(FfiError::new("PIPELINE_IO", error.clone()));
            }
            let stats = pipeline.scheduler().stats();
            Ok(step_outcome_json(&outcome, stats))
        }

        // 队列概览（书架上的"正在精修 N 本"读这个）。
        "pipeline.status" => {
            let handle = required_u64(&args, "handle")?;
            let pipeline = registry().pipeline(context, handle)?;
            let pipeline = lock_pipeline(&pipeline);
            let stats = pipeline.scheduler().stats();
            Ok(json!({
                "idle": pipeline.is_idle(),
                "stats": stats_json(stats),
            }))
        }

        // 单本书的落盘状态（书架卡片按需查）。
        "pipeline.state" => {
            let handle = required_u64(&args, "handle")?;
            let book = book_arg(&args)?;
            let pipeline = registry().pipeline(context, handle)?;
            let pipeline = lock_pipeline(&pipeline);
            match pipeline.store().state(book) {
                Some(item) => Ok(json!({
                    "known": true,
                    "state": item.state,
                    "attempts": item.attempts,
                        "failedStage": item.failed_stage,
                        "reason": item.reason,
                        "retryAtMs": item.retry_at_ms,
                })),
                None => Ok(json!({ "known": false })),
            }
        }

        // 用户点开某本书：插队。
        "pipeline.promote" => {
            let handle = required_u64(&args, "handle")?;
            let book = book_arg(&args)?;
            let priority = priority_arg(&args)?;
            let pipeline = registry().pipeline(context, handle)?;
            let mut pipeline = lock_pipeline(&pipeline);
            pipeline.promote(book, priority);
            Ok(json!({ "promoted": true, "priority": priority_str(priority) }))
        }

        // 手动重试一本耗尽的（卡在 failed 的书不能自己回来，得有人推一把）。
        "pipeline.retry" => {
            let handle = required_u64(&args, "handle")?;
            let book = book_arg(&args)?;
            let priority = priority_arg(&args)?;
            let pipeline = registry().pipeline(context, handle)?;
            let mut pipeline = lock_pipeline(&pipeline);
            let revived = pipeline
                .revive(book, priority)
                .map_err(|error| FfiError::new("PIPELINE_IO", error))?;
            Ok(json!({ "retried": revived }))
        }

        // 删书：取消它所有相关任务。
        "pipeline.cancel" => {
            let handle = required_u64(&args, "handle")?;
            let book = book_arg(&args)?;
            let pipeline = registry().pipeline(context, handle)?;
            let mut pipeline = lock_pipeline(&pipeline);
            pipeline.cancel(book);
            Ok(json!({ "cancelled": true }))
        }

        "pipeline.close" => {
            let handle = required_u64(&args, "handle")?;
            let mut guard = registry();
            if let Some(kind) = guard.kind_of(context, handle)?
                && kind != "pipeline"
            {
                return Err(FfiError::new(
                    "HANDLE_WRONG_KIND",
                    format!("句柄 {handle} 是 {kind} 类型，请用 {kind}.close"),
                ));
            }
            let removed = guard.remove(context, handle)?;
            drop(guard);
            let removed = removed.is_some();
            Ok(json!({ "closed": removed }))
        }

        "txt.close" => {
            let handle = required_u64(&args, "handle")?;
            let mut guard = registry();
            if let Some(kind) = guard.kind_of(context, handle)?
                && kind != "txt"
            {
                return Err(FfiError::new(
                    "HANDLE_WRONG_KIND",
                    format!("句柄 {handle} 是 {kind} 类型，请用 {kind}.close"),
                ));
            }
            let removed = guard.remove(context, handle)?;
            drop(guard);
            let removed = removed.is_some();
            Ok(json!({ "closed": removed }))
        }

        // ---- PDF（perch-pdf）----
        "pdf.analyze" => {
            let bytes = file_or_hex(&args, "PDF_IO")?;
            let info = perch_pdf::analyze(&bytes)
                .map_err(|error| FfiError::new(error.code(), error.to_string()))?;
            serde_json::to_value(&info).map_err(|error| FfiError::bad_args(error.to_string()))
        }

        // ---- TXT（perch-txt）----
        // 导入时只传路径、只返回统计；阅读仍使用 txt.open/slice。
        "txt.inspect" => {
            let document = txt_from_path(&args)?;
            let meta = document.meta();
            Ok(json!({
                "encoding": meta.encoding,
                "certain_encoding": meta.certain_encoding,
                "chars": meta.chars,
                "lines": meta.lines,
                "had_decode_errors": meta.had_decode_errors,
            }))
        }
        "txt.analyze" => {
            let bytes = hex_to_bytes(required_str(&args, "bytes_hex")?)?;
            let document = perch_txt::TxtDocument::from_bytes(&bytes);
            let meta = document.meta();
            Ok(json!({
                "encoding": meta.encoding,
                "certain_encoding": meta.certain_encoding,
                "chars": meta.chars,
                "lines": meta.lines,
                "had_decode_errors": meta.had_decode_errors,
                "text": document.text(),
            }))
        }

        // ---- 检索（perch-index）----
        "index.search" => {
            let text = required_str(&args, "text")?;
            let query = required_str(&args, "query")?;
            let chunk = args
                .get("chunk_chars")
                .and_then(Value::as_u64)
                .unwrap_or(perch_index::DEFAULT_CHUNK_CHARS);
            let index = perch_index::Index::build(text.to_owned(), chunk);
            let hits: Vec<Value> = index
                .search(query)
                .into_iter()
                .map(|hit| {
                    json!({
                        "char_start": hit.char_start,
                        "char_end": hit.char_end,
                        "chunk": hit.chunk,
                        "snippet": hit.snippet,
                    })
                })
                .collect();
            Ok(json!({ "hits": hits }))
        }

        // ---- 卷页几何（perch-pagecurl）----
        "curl" => {
            let page = Rect::new(
                number(&args, "page_w")? as f32,
                number(&args, "page_h")? as f32,
            );
            let corner = match required_str(&args, "corner")? {
                "top_left" => Corner::TopLeft,
                "top_right" => Corner::TopRight,
                "bottom_left" => Corner::BottomLeft,
                "bottom_right" => Corner::BottomRight,
                other => return Err(FfiError::bad_args(format!("未知 corner: {other}"))),
            };
            let input = CurlInput {
                page,
                corner,
                drag: Vec2::new(
                    number(&args, "drag_x")? as f32,
                    number(&args, "drag_y")? as f32,
                ),
                radius: number(&args, "radius")? as f32,
            };
            match curl(&input) {
                None => Ok(json!({ "geometry": Value::Null })),
                Some(geometry) => Ok(json!({
                    "geometry": {
                        "fold_start": point(geometry.fold_start),
                        "fold_end": point(geometry.fold_end),
                        "midpoint": point(geometry.midpoint),
                        "direction": point(geometry.direction),
                        "radius": geometry.radius,
                        "curl_angle": geometry.curl_angle,
                        "band_extent": geometry.band_extent,
                        "progress": geometry.progress,
                        "front_outline": geometry.front_outline.iter().map(|p| point(*p)).collect::<Vec<_>>(),
                        "shadow": {
                            "start": point(geometry.shadow.start),
                            "end": point(geometry.shadow.end),
                            "start_opacity": geometry.shadow.start_opacity,
                            "end_opacity": geometry.shadow.end_opacity,
                        }
                    }
                })),
            }
        }

        // ---- 同步（perch-sync）----
        "hlc.parse" => {
            let text = required_str(&args, "hlc")?;
            match perch_sync::Hlc::parse(text) {
                None => Err(FfiError::new("SYNC_BAD_HLC", "HLC 格式非法")),
                Some(hlc) => Ok(json!({
                    "wall_ms": hlc.wall_ms(),
                    "counter": hlc.counter(),
                    "device": hlc.device(),
                })),
            }
        }

        // 一次往返完成双向同步的合并：本地状态 + 远端变更 → 新状态 + 冲突统计
        "sync.merge" => {
            let local = changes_arg(&args, "local")?;
            let remote = changes_arg(&args, "remote")?;
            let mut log = perch_sync::ChangeLog::from_changes(&local)
                .map_err(|err| FfiError::from_sync(&err))?;
            let report = log
                .apply_all(&remote)
                .map_err(|err| FfiError::from_sync(&err))?;
            Ok(json!({
                "state": log.to_changes(),
                "stats": report.stats,
                "conflictRate": report.conflict_rate(),
                "effects": report.effects.iter().map(|e| e.as_str()).collect::<Vec<_>>(),
            }))
        }

        // 增量同步的服务端实现：给游标 + 页大小，返回一页变更
        "sync.delta" => {
            let state = changes_arg(&args, "state")?;
            let log = perch_sync::ChangeLog::from_changes(&state)
                .map_err(|err| FfiError::from_sync(&err))?;
            let since = match args.get("since").and_then(Value::as_str) {
                None => None,
                Some(text) => Some(
                    perch_sync::Hlc::parse(text)
                        .ok_or_else(|| FfiError::new("SYNC_BAD_HLC", "since 的 HLC 格式非法"))?,
                ),
            };
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(perch_sync::DEFAULT_PAGE_LIMIT, |n| n as usize);
            let page = log.changes_since(since.as_ref(), limit);
            Ok(json!({
                "changes": page.changes,
                "nextCursor": page.next_cursor.map(|hlc| hlc.to_string()),
                "hasMore": page.has_more,
            }))
        }

        other => Err(FfiError::new(
            "FFI_UNKNOWN_METHOD",
            format!("未知方法: {other}"),
        )),
    }
}

/// 同步读取路径或解码旧字节入口；文件与缓冲区不保留到调用结束之后。
/// 字段即使为 null 也算存在，避免错误输入静默选择另一个来源。
fn file_or_hex(args: &Value, io_code: &'static str) -> Result<Vec<u8>, FfiError> {
    match (args.get("path"), args.get("bytes_hex")) {
        (Some(_), None) => std::fs::read(required_str(args, "path")?)
            .map_err(|error| FfiError::new(io_code, error.to_string())),
        (None, Some(_)) => hex_to_bytes(required_str(args, "bytes_hex")?),
        _ => Err(FfiError::bad_args("path 与 bytes_hex 必须且只能提供一个")),
    }
}

/// 共用 TXT 解码规则；对象构造与文件 I/O 均在根注册锁外执行。
fn txt_from_path(args: &Value) -> Result<perch_txt::TxtDocument, FfiError> {
    let path = required_str(args, "path")?;
    match args.get("encoding").and_then(Value::as_str) {
        Some(label) => {
            let bytes = std::fs::read(path).map_err(txt_io_error)?;
            perch_txt::TxtDocument::with_encoding(&bytes, label).ok_or_else(|| {
                FfiError::new("TXT_UNKNOWN_ENCODING", format!("不认识的编码: {label}"))
            })
        }
        None => perch_txt::TxtDocument::open(path).map_err(txt_io_error),
    }
}

/// 解析可选的 `format` 参数（`"epub"` / `"pdf"` / `"txt"`）。
///
/// 不给时返回 `None`，由调用方按内容嗅探 —— 但上层知道扩展名时**应当显式传**：
/// 嗅探会把一个损坏的 EPUB 当成 TXT 收下，静默产出一本乱码书。
fn format_arg(args: &Value) -> Result<Option<Format>, FfiError> {
    let Some(text) = args.get("format").and_then(Value::as_str) else {
        return Ok(None);
    };
    match text {
        "epub" => Ok(Some(Format::Epub)),
        "pdf" => Ok(Some(Format::Pdf)),
        "txt" => Ok(Some(Format::Txt)),
        other => Err(FfiError::bad_args(format!("未知格式: {other}"))),
    }
}

/// 从入参取 `book`（UUID 字符串）。
///
/// 借 `serde` 反序列化而不是手写 UUID 解析：`BookId` 是 `#[serde(transparent)]`，
/// 这样"契约形状"只有一处定义。
fn book_arg(args: &Value) -> Result<perch_model::BookId, FfiError> {
    let value = args
        .get("book")
        .cloned()
        .ok_or_else(|| FfiError::bad_args("缺少 book"))?;
    serde_json::from_value(value).map_err(|error| FfiError::bad_args(format!("book: {error}")))
}

/// 从入参取调度策略（缺省用默认值；`docs/storage.md`「pipeline FFI」）。
fn policy_arg(args: &Value) -> Result<perch_pipeline::Policy, FfiError> {
    let mut policy = perch_pipeline::Policy::default();
    let Some(object) = args.get("policy") else {
        return Ok(policy);
    };
    if let Some(value) = object.get("maxRefine").and_then(Value::as_u64) {
        policy.max_refine = value as usize;
    }
    if let Some(value) = object.get("maxIndex").and_then(Value::as_u64) {
        policy.max_index = value as usize;
    }
    if let Some(value) = object.get("maxAttempts").and_then(Value::as_u64) {
        policy.max_attempts =
            u16::try_from(value).map_err(|_| FfiError::bad_args("maxAttempts 太大"))?;
    }
    if let Some(value) = object.get("backoffBaseMs").and_then(Value::as_u64) {
        policy.backoff_base_ms = value;
    }
    if let Some(value) = object.get("backoffMaxMs").and_then(Value::as_u64) {
        policy.backoff_max_ms = value;
    }
    Ok(policy)
}

/// 从入参取闸门。**缺省是全开**——平台必须按电量/网络/热状态自己决定；
/// 这里给缺省值只是为了测试与"前台空闲"这种明确知道可以干的场景。
fn gate_arg(args: &Value) -> Result<perch_pipeline::Gate, FfiError> {
    let Some(object) = args.get("gate") else {
        return Ok(perch_pipeline::Gate::foreground());
    };
    Ok(perch_pipeline::Gate {
        allow_any: object
            .get("allowAny")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        allow_heavy: object
            .get("allowHeavy")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

/// 从入参取初始状态（`pipeline.enqueue` 用；缺省 `registered`）。
fn state_arg(args: &Value) -> Result<perch_model::ImportState, FfiError> {
    let Some(value) = args.get("state") else {
        return Ok(perch_model::ImportState::Registered);
    };
    serde_json::from_value(value.clone())
        .map_err(|error| FfiError::bad_args(format!("state: {error}")))
}

/// 从入参取优先级（缺省 `normal`）。
fn priority_arg(args: &Value) -> Result<perch_pipeline::Priority, FfiError> {
    Ok(match args.get("priority").and_then(Value::as_str) {
        None | Some("normal") => perch_pipeline::Priority::Normal,
        Some("urgent") => perch_pipeline::Priority::Urgent,
        Some("background") => perch_pipeline::Priority::Background,
        Some(other) => {
            return Err(FfiError::bad_args(format!(
                "未知优先级 {other}（urgent / normal / background）"
            )));
        }
    })
}

/// 优先级的稳定字符串。
fn priority_str(priority: perch_pipeline::Priority) -> &'static str {
    match priority {
        perch_pipeline::Priority::Urgent => "urgent",
        perch_pipeline::Priority::Normal => "normal",
        perch_pipeline::Priority::Background => "background",
    }
}

/// 入队结果的稳定字符串。
fn enqueue_outcome_str(outcome: perch_pipeline::EnqueueOutcome) -> &'static str {
    match outcome {
        perch_pipeline::EnqueueOutcome::Queued(_) => "queued",
        perch_pipeline::EnqueueOutcome::AlreadyPresent => "alreadyPresent",
        perch_pipeline::EnqueueOutcome::NotNeeded => "notNeeded",
    }
}

/// 入队结果落在哪个阶段（`notNeeded` 时为 `null`）。
fn enqueue_outcome_stage(outcome: perch_pipeline::EnqueueOutcome) -> Option<&'static str> {
    match outcome {
        perch_pipeline::EnqueueOutcome::Queued(stage) => Some(stage.as_str()),
        _ => None,
    }
}

/// 调度器统计的 JSON（形状与 `Stats` 字段一一对应）。
fn stats_json(stats: perch_pipeline::Stats) -> Value {
    json!({
        "pendingRefine": stats.pending_refine,
        "pendingIndex": stats.pending_index,
        "inFlight": stats.in_flight,
        "exhausted": stats.exhausted,
    })
}

/// 一次 `step` 的结果（形状复用 [`perch_jobs::StepOutcome`]，不另立一套）。
fn step_outcome_json(outcome: &perch_jobs::StepOutcome, stats: perch_pipeline::Stats) -> Value {
    let mut value = match outcome {
        perch_jobs::StepOutcome::Idle => json!({ "outcome": "idle" }),
        perch_jobs::StepOutcome::Refined { book } => json!({
            "outcome": "refined",
            "book": book,
            "stage": "refine",
        }),
        perch_jobs::StepOutcome::Indexed {
            book,
            chunks,
            tokens,
        } => json!({
            "outcome": "indexed",
            "book": book,
            "stage": "index",
            "chunks": chunks,
            "tokens": tokens,
        }),
        // ⚠️ 字段名用 `reason` 而不是 `error`：`{"error": {code, message}}` 是 FFI 的
        // **错误通道**（Dart 的 `invoke` 见到 `error.code` 就抛异常）。业务字段重名
        // 会让"这一步失败了"和"这次调用失败了"混在一起，排查时非常难受。
        perch_jobs::StepOutcome::Failed {
            book,
            stage,
            error,
            attempts,
            retry_at_ms,
        } => json!({
            "outcome": "failed",
            "book": book,
            "stage": stage.as_str(),
            "reason": error,
            "attempts": attempts,
            "retryAtMs": retry_at_ms,
        }),
        perch_jobs::StepOutcome::Exhausted {
            book,
            stage,
            attempts,
            error,
        } => json!({
            "outcome": "exhausted",
            "book": book,
            "stage": stage.as_str(),
            "attempts": attempts,
            "reason": error,
        }),
        perch_jobs::StepOutcome::IoFailed { .. } => json!({"outcome": "ioFailed"}),
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("stats".to_owned(), stats_json(stats));
    }
    value
}

/// 现在（毫秒）。平台不传 `now_ms` 时的缺省值。
///
/// 退避重试要看"时间前进"，所以测试里必须能把时钟捏在手里（传 `now_ms`）；
/// 生产里让 Rust 取系统时间即可。
fn now_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

/// `std::io::Error` → FFI 错误。
///
/// 给稳定错误码而不是把 io 错误当成参数错误：读不到文件和参数写错是两回事，
/// 调用方（Dart）对它们的降级方式也不同。
fn txt_io_error(error: std::io::Error) -> FfiError {
    FfiError::new("TXT_IO", format!("读取失败: {error}"))
}

/// 取一个必填的无符号整数字段。
fn required_u64(args: &Value, field: &str) -> Result<u64, FfiError> {
    args.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| FfiError::bad_args(format!("缺少或非法的 {field}")))
}

/// 从入参里取一个变更数组（字段缺省视为空数组）。
fn changes_arg(args: &Value, field: &str) -> Result<Vec<perch_sync::Change>, FfiError> {
    match args.get(field) {
        None => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| FfiError::bad_args(format!("{field}: {error}"))),
    }
}

fn point(value: Vec2) -> Value {
    json!({ "x": value.x, "y": value.y })
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, FfiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| FfiError::bad_args(format!("缺少字符串字段 {key}")))
}

fn number(args: &Value, key: &str) -> Result<f64, FfiError> {
    args.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| FfiError::bad_args(format!("缺少数字字段 {key}")))
}

/// 十六进制字符串 → 字节。
fn hex_to_bytes(text: &str) -> Result<Vec<u8>, FfiError> {
    // 不用 is_multiple_of：它 1.87 才稳定，而工作区 MSRV 是 1.85
    if !text.len().is_multiple_of(2) {
        return Err(FfiError::bad_args("十六进制长度必须是偶数"));
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.as_chunks::<2>().0 {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Result<u8, FfiError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(FfiError::bad_args("十六进制里有非法字符")),
    }
}

// ---------------------------------------------------------------------------
// 以下为唯一的 unsafe 区域：跨越 C ABI 的裸指针
// ---------------------------------------------------------------------------

/// 调用一个方法。
///
/// # Safety
///
/// `method_ptr` / `args_ptr` 必须是调用方保证在本次调用期间有效的 UTF-8 内存，
/// 长度分别是 `method_len` / `args_len` 字节。返回的指针必须交给
/// [`perch_string_free`] 释放，且只能释放一次。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn perch_call(
    method_ptr: *const u8,
    method_len: usize,
    args_ptr: *const u8,
    args_len: usize,
) -> *mut std::ffi::c_char {
    // SAFETY: 与本函数的公开契约相同。
    unsafe { ffi_call(LEGACY_CONTEXT, method_ptr, method_len, args_ptr, args_len) }
}

/// 在显式上下文中调用一个方法。
///
/// # Safety
///
/// 指针、长度和返回值所有权与 [`perch_call`] 相同。`context` 必须来自
/// `context.open`，且尚未由 `context.close` 销毁。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn perch_context_call(
    context: u64,
    method_ptr: *const u8,
    method_len: usize,
    args_ptr: *const u8,
    args_len: usize,
) -> *mut std::ffi::c_char {
    // SAFETY: 与本函数的公开契约相同。
    unsafe { ffi_call(context, method_ptr, method_len, args_ptr, args_len) }
}

unsafe fn ffi_call(
    context: u64,
    method_ptr: *const u8,
    method_len: usize,
    args_ptr: *const u8,
    args_len: usize,
) -> *mut std::ffi::c_char {
    // 任何输入问题都不能 panic：panic 跨过 FFI 边界是未定义行为
    let method = match slice_to_str(method_ptr, method_len) {
        Some(text) => text,
        None => {
            return to_c_string(
                &json!({ "error": { "code": "FFI_BAD_METHOD_PTR", "message": "方法名不是合法 UTF-8" } }),
            );
        }
    };
    let args = match slice_to_str(args_ptr, args_len) {
        Some(text) => text,
        None => {
            return to_c_string(
                &json!({ "error": { "code": "FFI_BAD_ARGS_PTR", "message": "参数不是合法 UTF-8" } }),
            );
        }
    };
    to_c_string(&dispatch_in_context(context, method, args))
}

/// 释放 [`perch_call`] 或 [`perch_context_call`] 返回的字符串。
///
/// # Safety
///
/// `ptr` 必须来自 [`perch_call`] 或 [`perch_context_call`]，且只能释放一次；
/// 传空指针是安全的（直接返回）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn perch_string_free(ptr: *mut std::ffi::c_char) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: 指针由 CString::into_raw 产生，调用方承诺只释放一次
    drop(unsafe { CString::from_raw(ptr) });
}

/// 把裸指针 + 长度转成 `&str`（非法 UTF-8 或空指针返回 `None`）。
fn slice_to_str(ptr: *const u8, len: usize) -> Option<&'static str> {
    if ptr.is_null() || len == 0 {
        return Some("");
    }
    // SAFETY: 调用方保证 [ptr, ptr+len) 在本次调用期间有效
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(slice).ok()
}

fn to_c_string(value: &Value) -> *mut std::ffi::c_char {
    let text = value.to_string();
    // 兜底：万一 JSON 里出现内嵌 NUL（实际不会），退回一个固定错误
    match CString::new(text) {
        Ok(string) => string.into_raw(),
        Err(_) => CString::new(r#"{"error":{"code":"FFI_NUL","message":"输出含 NUL"}}"#)
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 句柄表是**进程级全局**的：这些测试共用它，必须串行跑——
    /// 并行会把 TXT/书的配额互相占满，表现为"随机某个测试报 TOO_MANY_OPEN"
    /// （第一版就踩到了）。用局部 `Registry` 的测试不需要这把锁。
    static REGISTRY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        match REGISTRY_TEST_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn call_ok(method: &str, args: &str) -> Value {
        let value = dispatch(method, args);
        assert!(value.get("error").is_none(), "{method} 出错: {value}");
        value
    }

    fn call_ok_in(context: u64, method: &str, args: &str) -> Value {
        let value = dispatch_in_context(context, method, args);
        assert!(value.get("error").is_none(), "{method} 出错: {value}");
        value
    }

    struct ContextGuard(Option<u64>);

    impl ContextGuard {
        fn open() -> Self {
            let value = call_ok("context.open", "{}");
            Self(Some(value["context"].as_u64().unwrap()))
        }

        fn id(&self) -> u64 {
            self.0.unwrap()
        }

        fn close(&mut self) -> Value {
            let context = self.0.take().unwrap();
            call_ok("context.close", &json!({ "context": context }).to_string())
        }
    }

    impl Drop for ContextGuard {
        fn drop(&mut self) {
            if let Some(context) = self.0.take() {
                let _ = dispatch("context.close", &json!({ "context": context }).to_string());
            }
        }
    }

    #[test]
    fn version_reports_core_version() {
        let value = call_ok("version", "{}");
        assert_eq!(value["core"], CORE_VERSION);
    }

    #[test]
    fn compatibility_reports_independent_contract_versions() {
        let value = call_ok("compatibility", "{}");
        assert_eq!(value["core"], CORE_VERSION);
        assert_eq!(value["abi"], ABI_VERSION);
        assert_eq!(value["api"]["major"], API_MAJOR);
        assert_eq!(value["api"]["minor"], API_MINOR);
        assert_eq!(value["locator"]["read"], json!([1]));
        assert_eq!(value["artifacts"]["index"]["read"], json!([1]));
        assert_eq!(value["artifacts"]["text"]["read"], json!([0, 1]));
    }

    #[test]
    fn contexts_isolate_handles_and_destroy_owned_objects() {
        let _serial = serial();
        let mut first = ContextGuard::open();
        let second = ContextGuard::open();
        let first_id = first.id();
        let path = txt_fixture_file("contexts", "甲乙丙丁");
        let first_txt = call_ok_in(
            first.id(),
            "txt.open",
            &json!({ "path": path.to_string_lossy() }).to_string(),
        );
        let second_txt = call_ok_in(
            second.id(),
            "txt.open",
            &json!({ "path": path.to_string_lossy() }).to_string(),
        );
        let first_handle = first_txt["handle"].as_u64().unwrap();
        let second_handle = second_txt["handle"].as_u64().unwrap();
        assert_ne!(first_handle, second_handle);

        let mismatch = dispatch_in_context(
            second.id(),
            "txt.slice",
            &json!({ "handle": first_handle, "chars": 1 }).to_string(),
        );
        assert_eq!(mismatch["error"]["code"], "CONTEXT_HANDLE_MISMATCH");
        let legacy_mismatch = dispatch(
            "txt.slice",
            &json!({ "handle": first_handle, "chars": 1 }).to_string(),
        );
        assert_eq!(legacy_mismatch["error"]["code"], "CONTEXT_HANDLE_MISMATCH");

        let closed = first.close();
        assert_eq!(closed["closed"], true);
        assert_eq!(closed["released"]["txts"], 1);
        let still_alive = call_ok_in(
            second.id(),
            "txt.slice",
            &json!({ "handle": second_handle, "chars": 1 }).to_string(),
        );
        assert_eq!(still_alive["text"], "甲");
        let after_close = dispatch_in_context(first_id, "version", "{}");
        assert_eq!(after_close["error"]["code"], "CONTEXT_BAD_HANDLE");
    }

    #[test]
    fn context_lifecycle_is_scoped_and_idempotent() {
        let _serial = serial();
        let mut context = ContextGuard::open();
        let id = context.id();
        let scoped = dispatch_in_context(id, "context.open", "{}");
        assert_eq!(scoped["error"]["code"], "CONTEXT_METHOD_SCOPE");
        let legacy = dispatch("context.close", r#"{"context":0}"#);
        assert_eq!(legacy["error"]["code"], "CONTEXT_LEGACY");
        assert_eq!(context.close()["closed"], true);
        let again = call_ok("context.close", &json!({ "context": id }).to_string());
        assert_eq!(again["closed"], false);
        assert_eq!(again["released"]["books"], 0);
    }

    #[test]
    fn unknown_method_returns_error_object() {
        let value = dispatch("nope", "{}");
        assert_eq!(value["error"]["code"], "FFI_UNKNOWN_METHOD");
    }

    #[test]
    fn locator_validate_accepts_good_and_rejects_bad() {
        let good = json!({
            "href": "/c1.xhtml",
            "locations": { "progression": 0.5 }
        });
        let value = call_ok("locator.validate", &good.to_string());
        assert_eq!(value["valid"], true);

        let bad = json!({ "href": "", "locations": {} });
        let value = dispatch("locator.validate", &bad.to_string());
        assert_eq!(value["error"]["code"], "MODEL_EMPTY_FIELD");

        let future = json!({
            "href": "txt:",
            "perch": { "v": 2, "format": "txt", "txtOffset": 0 }
        });
        let value = dispatch("locator.validate", &future.to_string());
        assert_eq!(value["error"]["code"], "MODEL_UNSUPPORTED_VERSION");
    }

    #[test]
    fn fingerprint_is_stable_across_calls() {
        let args = json!({ "size": 4, "head_hex": "6162", "tail_hex": "6364" });
        let first = call_ok("fingerprint", &args.to_string());
        let second = call_ok("fingerprint", &args.to_string());
        assert_eq!(first["fingerprint"], second["fingerprint"]);
        assert_eq!(first["fingerprint"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn txt_analyze_detects_utf8_and_counts() {
        let args = json!({ "bytes_hex": "e4b8ade69687" }); // "中文"
        let value = call_ok("txt.analyze", &args.to_string());
        assert_eq!(value["text"], "中文");
        assert_eq!(value["chars"], 2);
        assert_eq!(value["encoding"], "UTF-8");
        assert_eq!(value["certain_encoding"], true);
    }

    #[test]
    fn txt_analyze_detects_gbk() {
        let args = json!({ "bytes_hex": "d6d0cec4" }); // "中文" 的 GBK
        let value = call_ok("txt.analyze", &args.to_string());
        assert_eq!(value["text"], "中文");
        assert_eq!(value["certain_encoding"], false, "猜出来的编码不应标为确定");
    }

    #[test]
    fn index_search_returns_hits_with_offsets() {
        let args = json!({ "text": "前面的内容。中文测试。", "query": "中文测试" });
        let value = call_ok("index.search", &args.to_string());
        let hits = value["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["char_start"], 6);
        assert_eq!(hits[0]["char_end"], 10);
    }

    #[test]
    fn curl_returns_null_geometry_when_not_dragged() {
        let args = json!({
            "page_w": 400, "page_h": 700, "corner": "bottom_right",
            "drag_x": 400, "drag_y": 700, "radius": 20
        });
        let value = call_ok("curl", &args.to_string());
        assert!(value["geometry"].is_null());
    }

    #[test]
    fn curl_returns_geometry_when_dragged() {
        let args = json!({
            "page_w": 400, "page_h": 700, "corner": "bottom_right",
            "drag_x": 200, "drag_y": 400, "radius": 24
        });
        let value = call_ok("curl", &args.to_string());
        let geometry = &value["geometry"];
        assert!(geometry["curl_angle"].as_f64().unwrap() > 0.0);
        assert!(geometry["front_outline"].as_array().unwrap().len() >= 3);
    }

    #[test]
    fn hlc_parse_round_trips() {
        let value = call_ok("hlc.parse", r#"{"hlc":"100-2-phoneA"}"#);
        assert_eq!(value["wall_ms"], 100);
        assert_eq!(value["counter"], 2);
        assert_eq!(value["device"], "phoneA");

        let bad = dispatch("hlc.parse", r#"{"hlc":"nope"}"#);
        assert_eq!(bad["error"]["code"], "SYNC_BAD_HLC");
    }

    #[test]
    fn bad_json_args_produce_error_not_panic() {
        let value = dispatch("fingerprint", "{not json");
        assert_eq!(value["error"]["code"], "FFI_BAD_ARGS");
    }

    #[test]
    fn missing_fields_produce_error_not_panic() {
        let value = dispatch("fingerprint", "{}");
        assert_eq!(value["error"]["code"], "FFI_BAD_ARGS");
    }

    #[test]
    fn c_abi_round_trip_through_raw_pointers() {
        // 直接走一遍导出函数的路径（含 free），确保指针处理正确
        let method = "version";
        let args = "{}";
        let result =
            unsafe { perch_call(method.as_ptr(), method.len(), args.as_ptr(), args.len()) };
        assert!(!result.is_null());
        let text = unsafe { std::ffi::CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .to_owned();
        unsafe { perch_string_free(result) };

        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["core"], CORE_VERSION);
    }

    #[test]
    fn c_context_abi_round_trip_through_raw_pointers() {
        let _serial = serial();
        let mut context = ContextGuard::open();
        let method = "version";
        let args = "{}";
        let result = unsafe {
            perch_context_call(
                context.id(),
                method.as_ptr(),
                method.len(),
                args.as_ptr(),
                args.len(),
            )
        };
        assert!(!result.is_null());
        let text = unsafe { std::ffi::CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .to_owned();
        unsafe { perch_string_free(result) };
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["core"], CORE_VERSION);
        assert_eq!(context.close()["closed"], true);
    }

    #[test]
    fn null_pointer_free_is_safe() {
        unsafe { perch_string_free(std::ptr::null_mut()) };
    }

    // ---------- 同步（perch-sync 的传输层）----------

    const BOOK: &str = "11111111-1111-4111-8111-111111111111";

    /// 一条进度变更的 JSON（`hlc` 是文本形式）。
    fn progress_change(hlc: &str, progression: f64) -> Value {
        json!({
            "book": BOOK,
            "hlc": hlc,
            "op": "set",
            "value": {
                "kind": "progress",
                "locator": { "href": "c1.xhtml", "locations": { "progression": progression } }
            }
        })
    }

    fn merge(local: Vec<Value>, remote: Vec<Value>) -> Value {
        let args = json!({ "local": local, "remote": remote }).to_string();
        dispatch("sync.merge", &args)
    }

    #[test]
    fn sync_merge_initializes_from_empty() {
        let out = merge(vec![], vec![progress_change("100-0-iphone", 0.5)]);
        assert_eq!(out["state"].as_array().unwrap().len(), 1);
        assert_eq!(out["effects"][0], "initialized");
        assert_eq!(out["stats"]["initialized"], 1);
        assert_eq!(out["conflictRate"], 0.0);
    }

    #[test]
    fn sync_merge_ignores_older_remote() {
        let out = merge(
            vec![progress_change("200-0-iphone", 0.9)],
            vec![progress_change("100-0-ipad", 0.1)],
        );
        assert_eq!(out["effects"][0], "staleIgnored");
        assert_eq!(out["stats"]["staleIgnored"], 1);
        // 状态里仍是本地那条
        assert_eq!(out["state"][0]["hlc"], "200-0-iphone");
    }

    #[test]
    fn sync_merge_is_idempotent() {
        let change = progress_change("100-0-iphone", 0.5);
        let once = merge(vec![], vec![change.clone()]);
        let twice = merge(once["state"].as_array().unwrap().clone(), vec![change]);
        assert_eq!(twice["effects"][0], "unchanged");
        assert_eq!(twice["conflictRate"], 0.0);
    }

    #[test]
    fn sync_merge_two_devices_converge() {
        let phone = progress_change("100-0-iphone", 0.3);
        let pad = progress_change("110-0-ipad", 0.7);
        let a = merge(vec![phone.clone()], vec![pad.clone()]);
        let b = merge(vec![pad], vec![phone]);
        assert_eq!(a["state"], b["state"], "两端必须收敛到同一状态");
    }

    #[test]
    fn sync_merge_rejects_mismatched_target() {
        // 进度值却带了 annotation 字段 → 目标与值类型不匹配
        let mut bad = progress_change("100-0-iphone", 0.5);
        bad["annotation"] = json!("22222222-2222-4222-8222-222222222222");
        let out = merge(vec![], vec![bad]);
        assert_eq!(out["error"]["code"], "SYNC_TARGET_MISMATCH");
    }

    #[test]
    fn sync_delta_walks_with_cursor() {
        let args = json!({ "state": [progress_change("100-0-iphone", 0.3)] }).to_string();
        let first = dispatch("sync.delta", &args);
        assert_eq!(first["changes"].as_array().unwrap().len(), 1);
        assert_eq!(first["nextCursor"], "100-0-iphone");
        assert_eq!(first["hasMore"], false);

        // 用上一页的游标再问 → 没有增量
        let args = json!({
            "state": json!([progress_change("100-0-iphone", 0.3)]),
            "since": "100-0-iphone",
        })
        .to_string();
        let second = dispatch("sync.delta", &args);
        assert_eq!(second["changes"].as_array().unwrap().len(), 0);
    }

    // ---------- 阶段②精修 ----------

    /// 造一个最小 EPUB（1 章 + 元数据 + EPUB3 目录 + 封面声明）。
    fn epub_fixture() -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let stored =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            let deflated = SimpleFileOptions::default();

            zip.start_file("mimetype", stored).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();

            zip.start_file("META-INF/container.xml", deflated).unwrap();
            zip.write_all(r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#.as_bytes()).unwrap();

            zip.start_file("OEBPS/content.opf", deflated).unwrap();
            zip.write_all(
                r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">urn:uuid:ffi-test</dc:identifier>
    <dc:title>精修测试书</dc:title>
    <dc:creator>某作者</dc:creator>
    <dc:language>zh-CN</dc:language>
  </metadata>
  <manifest>
    <item id="c0" href="chap0.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  </manifest>
  <spine><itemref idref="c0"/></spine>
</package>"#
                    .as_bytes(),
            )
            .unwrap();

            zip.start_file("OEBPS/chap0.xhtml", deflated).unwrap();
            zip.write_all(
                r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>第一章</h1><p>春天的花开</p></body></html>"#
                    .as_bytes(),
            )
            .unwrap();

            zip.start_file("OEBPS/nav.xhtml", deflated).unwrap();
            zip.write_all(
                r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<body><nav epub:type="toc"><ol><li><a href="chap0.xhtml">第一章</a></li></ol></nav></body></html>"#
                    .as_bytes(),
            )
            .unwrap();

            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn jobs_refine_reads_metadata_and_toc() {
        let args = json!({ "bytes_hex": hex(&epub_fixture()) }).to_string();
        let out = dispatch("jobs.refine", &args);
        assert_eq!(out["title"], "精修测试书");
        assert_eq!(out["author"], "某作者");
        assert_eq!(out["language"], "zh-CN");
        assert_eq!(out["spineCount"], 1);
        assert_eq!(out["toc"][0], "第一章");
        assert!(out["charCount"].as_u64().unwrap() > 0);
    }

    #[test]
    fn path_input_matches_bytes_for_corpus() {
        let _serial = serial();
        let context = ContextGuard::open();
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/data/corpus/inputs");
        for (name, format, query) in [
            ("utf8.txt", "txt", "海风"),
            ("utf8-bom.txt", "txt", "海风"),
            ("gbk.txt", "txt", "中文"),
            ("big5.txt", "txt", "中文"),
            ("cross-shard.txt", "txt", "中文"),
            ("epub2.epub", "epub", "海风"),
            ("epub3.epub", "epub", "海风"),
        ] {
            let path = corpus.join(name);
            let bytes = std::fs::read(&path).unwrap();
            let from_path = json!({"path": path, "format": format, "chunk_chars": 8});
            let from_bytes = json!({"bytes_hex": hex(&bytes), "format": format, "chunk_chars": 8});
            assert_eq!(
                call_ok("jobs.refine", &from_path.to_string()),
                call_ok("jobs.refine", &from_bytes.to_string()),
                "{name} refinement"
            );
            let mut path_book = call_ok_in(context.id(), "book.open", &from_path.to_string());
            let mut bytes_book = call_ok_in(context.id(), "book.open", &from_bytes.to_string());
            let path_handle = path_book["handle"].as_u64().unwrap();
            let bytes_handle = bytes_book["handle"].as_u64().unwrap();
            path_book.as_object_mut().unwrap().remove("handle");
            bytes_book.as_object_mut().unwrap().remove("handle");
            assert_eq!(path_book, bytes_book, "{name} index stats");
            let search = |handle| {
                call_ok_in(
                    context.id(),
                    "book.search",
                    &json!({"handle": handle, "query": query}).to_string(),
                )
            };
            assert_eq!(search(path_handle), search(bytes_handle), "{name} search");
            for handle in [path_handle, bytes_handle] {
                call_ok_in(
                    context.id(),
                    "book.close",
                    &json!({"handle": handle}).to_string(),
                );
            }
            if format == "txt" {
                let inspected = call_ok("txt.inspect", &json!({"path": path}).to_string());
                let mut analyzed = call_ok(
                    "txt.analyze",
                    &json!({"bytes_hex": hex(&bytes)}).to_string(),
                );
                analyzed.as_object_mut().unwrap().remove("text");
                assert_eq!(inspected, analyzed, "{name} TXT statistics");
            }
        }
    }

    #[test]
    fn path_input_rejects_ambiguous_sources_and_reports_io() {
        for (method, code) in [("jobs.refine", "JOBS_IO"), ("book.open", "BOOK_OPEN_IO")] {
            for args in [
                json!({}),
                json!({"path": null}),
                json!({"path": 1}),
                json!({"path": "unused", "bytes_hex": ""}),
                json!({"path": null, "bytes_hex": "00"}),
                json!({"path": "unused", "bytes_hex": null}),
            ] {
                assert_eq!(
                    dispatch(method, &args.to_string())["error"]["code"],
                    "FFI_BAD_ARGS"
                );
            }
            let missing = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("Cargo.toml/this-is-not-a-file");
            assert_eq!(
                dispatch(method, &json!({"path": missing}).to_string())["error"]["code"],
                code
            );
        }
    }

    #[test]
    fn path_input_preserves_parser_errors_and_extraction_limit() {
        let _serial = serial();
        let context = ContextGuard::open();
        let path = txt_fixture_file("path-errors-中文", "中文🦀测试");
        for method in ["jobs.refine", "book.open"] {
            let bad = dispatch_in_context(
                context.id(),
                method,
                &json!({"path": path, "format": "epub"}).to_string(),
            );
            assert_eq!(bad["error"]["code"], "JOBS_EPUB");
            let unsupported = dispatch_in_context(
                context.id(),
                method,
                &json!({"path": path, "format": "pdf"}).to_string(),
            );
            assert_eq!(unsupported["error"]["code"], "JOBS_UNSUPPORTED_FORMAT");
        }
        let args = json!({"path": path, "format": "txt", "max_chars": 3});
        let limited = call_ok("jobs.refine", &args.to_string());
        assert_eq!(limited, call_ok("jobs.refine",
            &json!({"bytes_hex": hex("中文🦀测试".as_bytes()), "format": "txt", "max_chars": 3}).to_string()));
        assert_eq!(limited["charCount"], 3);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn txt_inspect_has_bounded_output_and_no_handles() {
        let _serial = serial();
        let mut context = ContextGuard::open();
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/data/corpus/inputs");
        let large = call_ok_in(
            context.id(),
            "txt.inspect",
            &json!({"path": corpus.join("long-line.txt")}).to_string(),
        );
        assert!(large["chars"].as_u64().unwrap() > 20_000);
        assert!(large.get("text").is_none());
        assert!(large.get("handle").is_none());
        assert!(large.to_string().len() < 256);
        let gbk = call_ok(
            "txt.inspect",
            &json!({"path": corpus.join("gbk.txt"), "encoding": "gbk"}).to_string(),
        );
        assert_eq!(gbk["encoding"], "GBK");
        assert_eq!(gbk["certain_encoding"], true);
        let invalid = dispatch(
            "txt.inspect",
            &json!({"path": corpus.join("gbk.txt"), "encoding": "not-an-encoding"}).to_string(),
        );
        assert_eq!(invalid["error"]["code"], "TXT_UNKNOWN_ENCODING");
        assert_eq!(
            context.close()["released"],
            json!({"books": 0, "txts": 0, "pipelines": 0})
        );
    }

    #[test]
    fn path_book_retains_text_after_source_disappears() {
        let _serial = serial();
        let context = ContextGuard::open();
        let path = txt_fixture_file("path-lifetime", "中文🦀测试");
        let opened = call_ok_in(
            context.id(),
            "book.open",
            &json!({"path": path}).to_string(),
        );
        std::fs::remove_file(&path).unwrap();
        let hits = call_ok_in(
            context.id(),
            "book.search",
            &json!({"handle": opened["handle"], "query": "中文"}).to_string(),
        );
        assert_eq!(hits["hits"].as_array().unwrap().len(), 1);
        assert_eq!(
            dispatch(
                "book.search",
                &json!({"handle": opened["handle"], "query": "中文"}).to_string()
            )["error"]["code"],
            "CONTEXT_HANDLE_MISMATCH"
        );
    }

    #[test]
    fn large_path_input_reports_truncation_without_returning_text() {
        let _serial = serial();
        let context = ContextGuard::open();
        let max_chars = perch_jobs::ExtractOptions::default().max_chars;
        // 15 MB 原始 UTF-8；请求只有路径，从未生成 bytes_hex。
        let text = "中".repeat(max_chars as usize + 1);
        let path = txt_fixture_file("large-path-input", &text);
        drop(text);
        let args = json!({"path": path, "format": "txt"}).to_string();
        assert!(args.len() < 256);
        let book = call_ok_in(context.id(), "book.open", &args);
        assert_eq!(book["truncated"], true);
        assert_eq!(book["charCount"], max_chars);
        assert!(book.get("text").is_none());
        assert!(book.to_string().len() < 256);
        let inspected = call_ok("txt.inspect", &json!({"path": path}).to_string());
        assert_eq!(inspected["chars"], max_chars + 1);
        assert!(inspected.to_string().len() < 256);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn jobs_refine_honours_explicit_format() {
        // 显式声明是 epub，内容却是随便的字节 → 必须报 EPUB 解析失败，
        // 绝不能悄悄当成纯文本收下。
        let args = json!({
            "bytes_hex": hex(b"not a zip at all"),
            "format": "epub",
        })
        .to_string();
        let out = dispatch("jobs.refine", &args);
        assert_eq!(out["error"]["code"], "JOBS_EPUB");
    }

    #[test]
    fn jobs_refine_auto_detects_txt() {
        // 不给 format 时按内容嗅探：非 ZIP/PDF 一律当纯文本
        let args = json!({ "bytes_hex": hex("普通的中文文本".as_bytes()) }).to_string();
        let out = dispatch("jobs.refine", &args);
        assert!(out["error"].is_null(), "应当成功: {out}");
        assert!(out["title"].is_null(), "TXT 没有内嵌标题");
        assert!(out["charCount"].as_u64().unwrap() > 0);
        assert!(out["encoding"].is_string(), "TXT 应当带编码探测结果");
    }

    #[test]
    fn jobs_refine_rejects_unknown_format() {
        let args = json!({ "bytes_hex": "00", "format": "mobi" }).to_string();
        let out = dispatch("jobs.refine", &args);
        assert_eq!(out["error"]["code"], "FFI_BAD_ARGS");
    }

    #[test]
    fn jobs_refine_rejects_pdf() {
        // PDF 只做结构解析，不进阶段②③
        let args = json!({ "bytes_hex": hex(b"%PDF-1.7"), "format": "pdf" }).to_string();
        let out = dispatch("jobs.refine", &args);
        assert_eq!(out["error"]["code"], "JOBS_UNSUPPORTED_FORMAT");
    }

    // ---------- 句柄表：打开 / 搜索 / 关闭 ----------

    fn open_book() -> u64 {
        let args = json!({ "bytes_hex": hex(&epub_fixture()) }).to_string();
        let out = dispatch("book.open", &args);
        out["handle"]
            .as_u64()
            .unwrap_or_else(|| panic!("book.open 失败: {out}"))
    }

    #[test]
    fn book_open_reports_index_stats() {
        let _serial = serial();
        let handle = open_book();
        let args = json!({ "bytes_hex": hex(&epub_fixture()) }).to_string();
        let out = dispatch("book.open", &args);
        assert!(out["charCount"].as_u64().unwrap() > 0);
        assert_eq!(out["chapterCount"], 1);
        assert!(out["chunks"].as_u64().unwrap() >= 1);
        assert_eq!(out["truncated"], false);
        // 两次 open 拿到不同句柄
        let second = out["handle"].as_u64().unwrap();
        assert_ne!(handle, second);
        dispatch("book.close", &json!({ "handle": handle }).to_string());
        dispatch("book.close", &json!({ "handle": second }).to_string());
    }

    #[test]
    fn book_load_reuses_persisted_artifacts_and_rejects_mismatch() {
        let _serial = serial();
        let directory =
            std::env::temp_dir().join(format!("perch-ffi-book-load-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        let text = perch_jobs::extract_text(
            &epub_fixture(),
            Format::Epub,
            perch_jobs::ExtractOptions::default(),
        )
        .unwrap();
        let index = perch_jobs::index_book(&text, perch_index::DEFAULT_CHUNK_CHARS);
        let index_path = directory.join("index.bin");
        let text_path = directory.join("text.json");
        index.save_to(&index_path).unwrap();
        std::fs::write(&text_path, perch_jobs::book_text_to_vec(&text).unwrap()).unwrap();

        let loaded = call_ok(
            "book.load",
            &json!({
                "index_path": index_path.to_string_lossy(),
                "text_path": text_path.to_string_lossy(),
            })
            .to_string(),
        );
        let handle = loaded["handle"].as_u64().unwrap();
        let found = call_ok(
            "book.search",
            &json!({ "handle": handle, "query": "春天的花开" }).to_string(),
        );
        assert_eq!(found["hits"].as_array().unwrap().len(), 1);
        call_ok("book.close", &json!({ "handle": handle }).to_string());

        std::fs::write(&text_path, serde_json::to_vec(&text).unwrap()).unwrap();
        let loaded = call_ok(
            "book.load",
            &json!({
                "index_path": index_path.to_string_lossy(),
                "text_path": text_path.to_string_lossy(),
            })
            .to_string(),
        );
        call_ok(
            "book.close",
            &json!({ "handle": loaded["handle"] }).to_string(),
        );

        let mut future = serde_json::to_value(&text).unwrap();
        future["version"] = json!(perch_jobs::BOOK_TEXT_VERSION + 1);
        std::fs::write(&text_path, serde_json::to_vec(&future).unwrap()).unwrap();
        let rejected = dispatch(
            "book.load",
            &json!({
                "index_path": index_path.to_string_lossy(),
                "text_path": text_path.to_string_lossy(),
            })
            .to_string(),
        );
        assert_eq!(rejected["error"]["code"], "BOOK_TEXT_UNSUPPORTED_VERSION");

        let mut mismatched = text;
        mismatched.text.push('x');
        std::fs::write(&text_path, serde_json::to_vec(&mismatched).unwrap()).unwrap();
        let rejected = dispatch(
            "book.load",
            &json!({
                "index_path": index_path.to_string_lossy(),
                "text_path": text_path.to_string_lossy(),
            })
            .to_string(),
        );
        assert_eq!(rejected["error"]["code"], "BOOK_INDEX_MISMATCH");
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn book_search_returns_locator_pointing_at_the_chapter() {
        let _serial = serial();
        let handle = open_book();
        let args = json!({ "handle": handle, "query": "春天的花开" }).to_string();
        let out = dispatch("book.search", &args);
        let hits = out["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "应当命中 1 条");
        let locator = &hits[0]["locator"];
        assert_eq!(locator["href"], "/OEBPS/chap0.xhtml");
        assert_eq!(locator["perch"]["format"], "epub");
        assert!(locator["locations"]["progression"].as_f64().is_some());
        // 粗定位：不给 CFI，精确位置由阅读器按文本重锚定
        assert!(locator["locations"].get("partialCfi").is_none());
        dispatch("book.close", &json!({ "handle": handle }).to_string());
    }

    #[test]
    fn book_locate_maps_search_offsets_to_the_decoded_txt_source() {
        let _serial = serial();
        let context = ContextGuard::open();
        let opened = call_ok_in(context.id(), "book.open", &json!({
            "bytes_hex": hex("\r\n甲\r\n\r\n\r\n乙😀é".as_bytes()), "format": "txt", "chunk_chars": 2,
        }).to_string());
        let handle = opened["handle"].as_u64().unwrap();
        let hits = call_ok_in(
            context.id(),
            "book.search",
            &json!({ "handle": handle, "query": "乙" }).to_string(),
        );
        let location = call_ok_in(context.id(), "book.locate", &json!({
            "handle": handle, "char_start": hits["hits"][0]["charStart"], "char_end": hits["hits"][0]["charEnd"],
        }).to_string());
        assert_eq!(
            location,
            json!({
                "start": { "kind": "txt", "charOffset": 9, "byteOffset": 11 },
                "end": { "kind": "txt", "charOffset": 10, "byteOffset": 14 }, "containsSynthetic": false,
            })
        );
        // The old search Locator deliberately retains its normalized offset.
        assert_eq!(hits["hits"][0]["locator"]["perch"]["txtOffset"], 3);
        for args in [
            json!({ "handle": handle, "char_start": 0, "char_end": 0 }),
            json!({ "handle": handle, "char_start": 4, "char_end": 2 }),
            json!({ "handle": handle, "char_start": 0, "char_end": opened["charCount"].as_u64().unwrap() + 1 }),
            json!({ "handle": handle, "char_start": -1, "char_end": 1 }),
            json!({ "handle": handle, "char_start": "0", "char_end": 1 }),
            json!({ "handle": handle, "char_start": 0 }),
        ] {
            assert_eq!(
                dispatch_in_context(context.id(), "book.locate", &args.to_string())["error"]["code"],
                "BOOK_LOCATE_BAD_RANGE"
            );
        }
    }

    #[test]
    fn book_locate_uses_context_ownership_and_close_lease_semantics() {
        let _serial = serial();
        let mut first = ContextGuard::open();
        let second = ContextGuard::open();
        let opened = call_ok_in(
            first.id(),
            "book.open",
            &json!({ "bytes_hex": hex("甲😀乙".as_bytes()) }).to_string(),
        );
        let handle = opened["handle"].as_u64().unwrap();
        let args = json!({ "handle": handle, "char_start": 1, "char_end": 2 }).to_string();
        assert_eq!(
            dispatch_in_context(second.id(), "book.locate", &args)["error"]["code"],
            "CONTEXT_HANDLE_MISMATCH"
        );
        assert_eq!(
            dispatch("book.locate", &args)["error"]["code"],
            "CONTEXT_HANDLE_MISMATCH"
        );
        let txt = registry()
            .insert_txt(first.id(), perch_txt::TxtDocument::from_bytes(b"reader"))
            .unwrap();
        assert_eq!(
            dispatch_in_context(
                first.id(),
                "book.locate",
                &json!({ "handle": txt, "char_start": 0, "char_end": 1 }).to_string()
            )["error"]["code"],
            "HANDLE_WRONG_KIND"
        );
        let lease = registry().book(first.id(), handle).unwrap();
        let expected = lease.text.locate(1, 2).unwrap();
        call_ok_in(
            first.id(),
            "book.close",
            &json!({ "handle": handle }).to_string(),
        );
        assert_eq!(
            dispatch_in_context(first.id(), "book.locate", &args)["error"]["code"],
            "BOOK_BAD_HANDLE"
        );
        assert_eq!(lease.text.locate(1, 2).unwrap(), expected);
        let context_id = first.id();
        first.close();
        assert_eq!(
            dispatch_in_context(context_id, "book.locate", &args)["error"]["code"],
            "CONTEXT_BAD_HANDLE"
        );
        assert_eq!(lease.text.locate(1, 2).unwrap(), expected);
    }

    #[test]
    fn book_load_defers_mapping_errors_and_preserves_existing_search_responses() {
        let _serial = serial();
        let context = ContextGuard::open();
        let directory =
            std::env::temp_dir().join(format!("perch-locate-load-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let text = perch_jobs::extract_txt_with_source_map(
            "\r\n甲😀乙".as_bytes(),
            perch_jobs::ExtractOptions::default(),
        )
        .unwrap();
        let index_path = directory.join("index.bin");
        let text_path = directory.join("text.json");
        perch_jobs::index_book(text.book_text(), 2)
            .save_to(&index_path)
            .unwrap();
        let stored: Value =
            serde_json::from_slice(&perch_jobs::mapped_book_text_to_vec(&text).unwrap()).unwrap();
        let mut expected_search = None;
        for variant in ["valid", "missing", "future", "invalid", "null"] {
            let mut value = stored.clone();
            let error = match variant {
                "missing" => {
                    value.as_object_mut().unwrap().remove("sourceMap");
                    Some("BOOK_SOURCE_MAP_MISSING")
                }
                "future" => {
                    value["sourceMap"]["version"] = json!(2);
                    Some("BOOK_SOURCE_MAP_UNSUPPORTED_VERSION")
                }
                "invalid" => {
                    value["sourceMap"]["runs"] = json!([]);
                    Some("BOOK_SOURCE_MAP_INVALID")
                }
                "null" => {
                    value["sourceMap"] = Value::Null;
                    Some("BOOK_SOURCE_MAP_INVALID")
                }
                _ => None,
            };
            std::fs::write(&text_path, serde_json::to_vec(&value).unwrap()).unwrap();
            let loaded = call_ok_in(
                context.id(),
                "book.load",
                &json!({ "index_path": index_path, "text_path": text_path }).to_string(),
            );
            let handle = loaded["handle"].as_u64().unwrap();
            let search = call_ok_in(
                context.id(),
                "book.search",
                &json!({ "handle": handle, "query": "乙" }).to_string(),
            );
            assert_eq!(
                &search,
                expected_search.get_or_insert_with(|| search.clone())
            );
            let args = json!({ "handle": handle, "char_start": 2, "char_end": 3 }).to_string();
            if let Some(error) = error {
                assert_eq!(
                    dispatch_in_context(context.id(), "book.locate", &args)["error"]["code"],
                    error
                );
            } else {
                let location = call_ok_in(context.id(), "book.locate", &args);
                assert_eq!(location["start"]["charOffset"], 4);
                assert_eq!(location["start"]["byteOffset"], 9);
            }
            call_ok_in(
                context.id(),
                "book.close",
                &json!({ "handle": handle }).to_string(),
            );
        }
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn book_search_miss_returns_empty() {
        let _serial = serial();
        let handle = open_book();
        let args = json!({ "handle": handle, "query": "完全不存在的词" }).to_string();
        let out = dispatch("book.search", &args);
        assert_eq!(out["hits"].as_array().unwrap().len(), 0);
        dispatch("book.close", &json!({ "handle": handle }).to_string());
    }

    #[test]
    fn closed_handle_is_rejected_not_misused() {
        let _serial = serial();
        let handle = open_book();
        let closed = dispatch("book.close", &json!({ "handle": handle }).to_string());
        assert_eq!(closed["closed"], true);
        // 再关一次是幂等的，不报错
        let again = dispatch("book.close", &json!({ "handle": handle }).to_string());
        assert_eq!(again["closed"], false);
        // 但用旧句柄搜索必须报错，而不是操作到别的书上
        let args = json!({ "handle": handle, "query": "春天" }).to_string();
        let out = dispatch("book.search", &args);
        assert_eq!(out["error"]["code"], "BOOK_BAD_HANDLE");
    }

    #[test]
    fn unknown_handle_is_rejected() {
        let args = json!({ "handle": 999_999, "query": "x" }).to_string();
        let out = dispatch("book.search", &args);
        assert_eq!(out["error"]["code"], "BOOK_BAD_HANDLE");
    }

    #[test]
    fn missing_handle_is_bad_args() {
        let out = dispatch("book.search", r#"{"query":"x"}"#);
        assert_eq!(out["error"]["code"], "FFI_BAD_ARGS");
    }

    #[test]
    fn registry_enforces_open_book_limit() {
        // 直接测 Registry 本身，不碰全局单例——否则会和并行跑的其他测试抢名额
        let mut local = Registry::default();
        let mut handles = Vec::new();
        for _ in 0..MAX_OPEN_BOOKS {
            let book = perch_jobs::extract_epub_text(
                &epub_fixture(),
                perch_jobs::ExtractOptions::default(),
            )
            .unwrap();
            let index = perch_jobs::index_book(&book, 64);
            handles.push(
                local
                    .insert_book(
                        LEGACY_CONTEXT,
                        OpenBook {
                            text: book.into(),
                            index,
                        },
                    )
                    .unwrap(),
            );
        }
        let book =
            perch_jobs::extract_epub_text(&epub_fixture(), perch_jobs::ExtractOptions::default())
                .unwrap();
        let index = perch_jobs::index_book(&book, 64);
        let error = local
            .insert_book(
                LEGACY_CONTEXT,
                OpenBook {
                    text: book.into(),
                    index,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "BOOK_TOO_MANY_OPEN");

        // 句柄互不相同且单调（永不复用）
        assert_eq!(handles.len(), MAX_OPEN_BOOKS);
        assert!(handles.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn registry_quotas_are_per_context() {
        let mut local = Registry::default();
        let first = local.create_context().unwrap();
        let second = local.create_context().unwrap();
        let make_book = || {
            let text = perch_jobs::extract_txt(
                "上下文配额".as_bytes(),
                perch_jobs::ExtractOptions::default(),
            )
            .unwrap();
            OpenBook {
                index: perch_index::Index::build(&text.text, 64),
                text: text.into(),
            }
        };

        for _ in 0..MAX_OPEN_BOOKS {
            local.insert_book(first, make_book()).unwrap();
        }
        assert_eq!(
            local.insert_book(first, make_book()).unwrap_err().code,
            "BOOK_TOO_MANY_OPEN"
        );
        let second_handle = local.insert_book(second, make_book()).unwrap();
        assert!(local.book(second, second_handle).is_ok());

        let released = local.close_context(first).unwrap().unwrap();
        assert_eq!(released.released.books, MAX_OPEN_BOOKS);
        assert!(local.book(second, second_handle).is_ok());
    }

    #[test]
    fn context_close_invalidates_handles_but_keeps_existing_leases_alive() {
        let mut local = Registry::default();
        let context = local.create_context().unwrap();
        let text = perch_jobs::extract_txt(
            "在途租约继续完成".as_bytes(),
            perch_jobs::ExtractOptions::default(),
        )
        .unwrap();
        let book = std::sync::Arc::new(OpenBook {
            index: perch_index::Index::build(&text.text, 64),
            text: text.into(),
        });
        let handle = local
            .insert_book(context, std::sync::Arc::clone(&book))
            .unwrap();
        let lease = local.book(context, handle).unwrap();

        let closed = local.close_context(context).unwrap().unwrap();
        assert_eq!(closed.released.books, 1);
        drop(closed.objects);
        let error = match local.book(context, handle) {
            Ok(_) => panic!("context.close 后不应再取得新租约"),
            Err(error) => error,
        };
        assert_eq!(error.code, "CONTEXT_BAD_HANDLE");
        assert_eq!(lease.index.search("租约").len(), 1);
    }

    #[test]
    fn registry_reports_exhausted_context_ids_without_reuse() {
        let mut local = Registry {
            next_context: Some(u64::MAX),
            ..Registry::default()
        };

        assert_eq!(local.create_context().unwrap(), u64::MAX);
        let error = local.create_context().unwrap_err();
        assert_eq!(error.code, "CONTEXT_ID_EXHAUSTED");
        assert!(local.contexts.contains_key(&u64::MAX));
    }

    #[test]
    fn registry_reports_exhausted_handle_ids_without_reuse() {
        let mut local = Registry {
            next_handle: Some(u64::MAX),
            ..Registry::default()
        };
        let make_book = || {
            let text = perch_jobs::extract_txt(
                "句柄空间".as_bytes(),
                perch_jobs::ExtractOptions::default(),
            )
            .unwrap();
            OpenBook {
                index: perch_index::Index::build(&text.text, 64),
                text: text.into(),
            }
        };

        assert_eq!(
            local.insert_book(LEGACY_CONTEXT, make_book()).unwrap(),
            u64::MAX
        );
        let error = local.insert_book(LEGACY_CONTEXT, make_book()).unwrap_err();
        assert_eq!(error.code, "HANDLE_ID_EXHAUSTED");
        assert!(local.book(LEGACY_CONTEXT, u64::MAX).is_ok());
        assert_eq!(local.legacy.books, 1);
    }

    // ---------- PDF 结构 ----------

    /// 造一个 n 页的最小 PDF，用于端到端验证 pdf.analyze。
    fn pdf_fixture(pages: usize) -> Vec<u8> {
        use lopdf::{Dictionary, Document, Object, dictionary};
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut page_ids = Vec::new();
        for _ in 0..pages {
            let contents = document.add_object(lopdf::Stream::new(Dictionary::new(), Vec::new()));
            let page = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Contents" => contents,
            });
            page_ids.push(page);
        }
        let kids: Vec<Object> = page_ids.iter().map(|id| Object::Reference(*id)).collect();
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => i64::try_from(pages).unwrap(),
            }),
        );
        let info_id = document.add_object(dictionary! {
            "Title" => Object::string_literal("测试文档"),
        });
        document.trailer.set("Info", info_id);
        let catalog_id =
            document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn pdf_analyze_reports_pages_and_title() {
        let bytes = pdf_fixture(7);
        let args = json!({ "bytes_hex": hex(&bytes) }).to_string();
        let out = dispatch("pdf.analyze", &args);
        assert_eq!(out["pageCount"], 7);
        assert_eq!(out["title"], "测试文档");
        assert_eq!(out["version"], "1.7");
    }

    #[test]
    fn pdf_analyze_rejects_non_pdf() {
        let args = json!({ "bytes_hex": hex(b"definitely not pdf") }).to_string();
        let out = dispatch("pdf.analyze", &args);
        assert_eq!(out["error"]["code"], "PDF_NOT_PDF");
    }

    #[test]
    fn pdf_analyze_rejects_empty() {
        let out = dispatch("pdf.analyze", r#"{"bytes_hex":""}"#);
        assert_eq!(out["error"]["code"], "PDF_EMPTY");
    }

    #[test]
    fn sync_delta_rejects_bad_cursor() {
        let args = json!({ "state": [], "since": "不是HLC" }).to_string();
        let out = dispatch("sync.delta", &args);
        assert_eq!(out["error"]["code"], "SYNC_BAD_HLC");
    }

    // ---------- TXT 阅读路径（句柄化）----------

    /// 造一个临时 TXT 文件。用真实文件而不是 bytes：`txt.open` 走的就是
    /// `std::fs` 这条路，得把"路径 → 解码 → 索引"整条链验掉。
    fn txt_fixture_file(name: &str, text: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("perch-ffi-txt-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("book.txt");
        std::fs::write(&path, text.as_bytes()).unwrap();
        path
    }

    fn open_txt(name: &str, text: &str) -> Value {
        let path = txt_fixture_file(name, text);
        call_ok(
            "txt.open",
            &json!({ "path": path.to_string_lossy() }).to_string(),
        )
    }

    #[test]
    fn txt_open_slice_locate_round_trip() {
        let _serial = serial();
        let text = "第一章 灯塔\n\n守夜人每天爬上灯塔。\n第二行在这里。\n第三行。";
        let out = open_txt("roundtrip", text);
        let handle = out["handle"].as_u64().unwrap();
        assert_eq!(out["chars"].as_u64().unwrap(), text.chars().count() as u64);
        assert_eq!(out["lines"].as_u64().unwrap(), text.lines().count() as u64);
        assert_eq!(out["encoding"], "UTF-8");
        assert_eq!(out["certain"], true);
        assert_eq!(out["hadErrors"], false);

        // 取一屏：**字符**偏移，不是字节偏移（中文一个字符 3 字节）
        let slice = call_ok(
            "txt.slice",
            &json!({ "handle": handle, "start": 3, "chars": 5 }).to_string(),
        );
        let expected: String = text.chars().skip(3).take(5).collect();
        assert_eq!(slice["text"], expected);
        assert_eq!(slice["chars"], 5);
        assert_eq!(
            slice["total"].as_u64().unwrap(),
            text.chars().count() as u64
        );

        // 字符偏移 → Locator → 字符偏移（跨端同步靠这条链）
        let located = call_ok(
            "txt.locate",
            &json!({ "handle": handle, "char": 5 }).to_string(),
        );
        assert_eq!(located["locator"]["perch"]["txtOffset"], 5);
        assert_eq!(located["locator"]["perch"]["format"], "txt");

        let back = call_ok(
            "txt.offset",
            &json!({ "handle": handle, "locator": located["locator"] }).to_string(),
        );
        assert_eq!(back["char"].as_u64().unwrap(), 5);
        assert_eq!(back["resolved"], true);

        // 行号与行区间
        let line = call_ok(
            "txt.line",
            &json!({ "handle": handle, "char": 5 }).to_string(),
        );
        assert_eq!(line["line"], 0);
        assert_eq!(line["start"], 0);
        assert_eq!(
            line["totalLines"].as_u64().unwrap(),
            text.lines().count() as u64
        );

        // 关掉之后旧句柄必须失效，而不是误操作到别的对象
        let closed = call_ok("txt.close", &json!({ "handle": handle }).to_string());
        assert_eq!(closed["closed"], true);
        let again = dispatch(
            "txt.slice",
            &json!({ "handle": handle, "chars": 1 }).to_string(),
        );
        assert_eq!(again["error"]["code"], "TXT_BAD_HANDLE");
    }

    #[test]
    fn txt_open_missing_file_is_txt_io_not_bad_args() {
        let _serial = serial();
        let out = dispatch("txt.open", r#"{"path":"/nonexistent/perch-book.txt"}"#);
        assert_eq!(out["error"]["code"], "TXT_IO");
    }

    #[test]
    fn txt_open_honours_explicit_encoding() {
        let _serial = serial();
        // GBK 的"中文"（D6 D0 CE C4）；不给 encoding 时靠探测，
        // 这么短的样本探测没把握，所以显式指定这条路必须能用。
        let directory = std::env::temp_dir().join(format!("perch-ffi-gbk-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("gbk.txt");
        std::fs::write(&path, [0xD6u8, 0xD0, 0xCE, 0xC4]).unwrap();

        let out = call_ok(
            "txt.open",
            &json!({ "path": path.to_string_lossy(), "encoding": "gbk" }).to_string(),
        );
        assert_eq!(out["chars"], 2);
        assert_eq!(out["certain"], true);

        let bad = dispatch(
            "txt.open",
            &json!({ "path": path.to_string_lossy(), "encoding": "不存在的编码" }).to_string(),
        );
        assert_eq!(bad["error"]["code"], "TXT_UNKNOWN_ENCODING");
        call_ok("txt.close", &json!({ "handle": out["handle"] }).to_string());
    }

    #[test]
    fn txt_slice_bounds_are_explicit() {
        let _serial = serial();
        let text = "一二三四五六七八九十";
        let out = open_txt("bounds", text);
        let handle = out["handle"].as_u64().unwrap();
        let total = text.chars().count() as u64;

        // 末尾不足一屏：按实际长度返回（最后一页就是这个形状）
        let tail = call_ok(
            "txt.slice",
            &json!({ "handle": handle, "start": total - 3, "chars": 100 }).to_string(),
        );
        assert_eq!(tail["chars"], 3);

        // 0 个字符 / 超过上限：明确报错，不悄悄截断
        let zero = dispatch(
            "txt.slice",
            &json!({ "handle": handle, "chars": 0 }).to_string(),
        );
        assert_eq!(zero["error"]["code"], "TXT_SLICE_BAD_SIZE");
        let too_big = dispatch(
            "txt.slice",
            &json!({ "handle": handle, "chars": MAX_TXT_SLICE_CHARS + 1 }).to_string(),
        );
        assert_eq!(too_big["error"]["code"], "TXT_SLICE_BAD_SIZE");
        let missing = dispatch("txt.slice", &json!({ "handle": handle }).to_string());
        assert_eq!(missing["error"]["code"], "FFI_BAD_ARGS");

        // start 越界
        let beyond = dispatch(
            "txt.slice",
            &json!({ "handle": handle, "start": total + 1, "chars": 1 }).to_string(),
        );
        assert_eq!(beyond["error"]["code"], "TXT_OUT_OF_RANGE");
        call_ok("txt.close", &json!({ "handle": handle }).to_string());
    }

    #[test]
    fn txt_offset_unresolved_locator_is_a_result_not_an_error() {
        let _serial = serial();
        // 定位不出来的 Locator 是**正常业务结果**：调用方要能退回 progression，
        // 所以返回 `char: null`，而不是抛错。
        let out = open_txt("unresolved", "一二三四五");
        let handle = out["handle"].as_u64().unwrap();
        let foreign = json!({
            "href": "/OEBPS/chap01.xhtml",
            "type": "application/xhtml+xml",
            "locations": { "progression": 0.5 },
        });
        let resolved = call_ok(
            "txt.offset",
            &json!({ "handle": handle, "locator": foreign }).to_string(),
        );
        // 没有 perch 字段时退到 progression：0.5 × 5 字符 ≈ 3
        assert_eq!(resolved["resolved"], true);
        assert_eq!(resolved["char"], 3);
        call_ok("txt.close", &json!({ "handle": handle }).to_string());
    }

    #[test]
    fn handle_kinds_are_not_interchangeable() {
        let _serial = serial();
        let out = open_txt("kinds", "一二三四五");
        let txt_handle = out["handle"].as_u64().unwrap();

        // 拿 TXT 句柄去查索引 → 类型错误（而不是"句柄不存在"那种含糊信息）
        let search = dispatch(
            "book.search",
            &json!({ "handle": txt_handle, "query": "一" }).to_string(),
        );
        assert_eq!(search["error"]["code"], "HANDLE_WRONG_KIND");

        // 拿 TXT 句柄去 book.close：必须拒绝，**且不能真把它关掉**
        let wrong_close = dispatch("book.close", &json!({ "handle": txt_handle }).to_string());
        assert_eq!(wrong_close["error"]["code"], "HANDLE_WRONG_KIND");
        let still_alive = call_ok(
            "txt.slice",
            &json!({ "handle": txt_handle, "start": 0, "chars": 1 }).to_string(),
        );
        assert_eq!(still_alive["text"], "一");

        // 反过来：book 句柄不能走 txt.*
        let book_handle = open_book();
        let wrong_slice = dispatch(
            "txt.slice",
            &json!({ "handle": book_handle, "chars": 1 }).to_string(),
        );
        assert_eq!(wrong_slice["error"]["code"], "HANDLE_WRONG_KIND");
        call_ok("book.close", &json!({ "handle": book_handle }).to_string());
        call_ok("txt.close", &json!({ "handle": txt_handle }).to_string());
    }

    #[test]
    fn txt_quota_is_separate_from_book_quota() {
        // 阅读句柄不该被后台索引任务挤掉：两类**分开计数**（docs/architecture.md「句柄与生命周期」）。
        let mut local = Registry::default();

        for _ in 0..MAX_OPEN_BOOKS {
            let text =
                perch_jobs::extract_txt(b"hello", perch_jobs::ExtractOptions::default()).unwrap();
            local
                .insert_book(
                    LEGACY_CONTEXT,
                    OpenBook {
                        text: text.into(),
                        index: perch_index::Index::build("hello", 64),
                    },
                )
                .unwrap();
        }
        let text =
            perch_jobs::extract_txt(b"hello", perch_jobs::ExtractOptions::default()).unwrap();
        let error = local
            .insert_book(
                LEGACY_CONTEXT,
                OpenBook {
                    text: text.into(),
                    index: perch_index::Index::build("hello", 64),
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "BOOK_TOO_MANY_OPEN");

        // 书配额满了，TXT 照开不误
        let handle = local
            .insert_txt(
                LEGACY_CONTEXT,
                perch_txt::TxtDocument::from_bytes("阅读优先".as_bytes()),
            )
            .unwrap();
        assert_eq!(local.txt(LEGACY_CONTEXT, handle).unwrap().char_count(), 4);

        // 但 TXT 自己也有上限
        for _ in 1..MAX_OPEN_TXT {
            local
                .insert_txt(LEGACY_CONTEXT, perch_txt::TxtDocument::from_bytes(b"x"))
                .unwrap();
        }
        let error = local
            .insert_txt(LEGACY_CONTEXT, perch_txt::TxtDocument::from_bytes(b"x"))
            .unwrap_err();
        assert_eq!(error.code, "TXT_TOO_MANY_OPEN");

        // 关一个就把名额还回来
        local.remove(LEGACY_CONTEXT, handle).unwrap();
        assert!(
            local
                .insert_txt(LEGACY_CONTEXT, perch_txt::TxtDocument::from_bytes(b"x"),)
                .is_ok()
        );
    }

    // ---------- 导入管线（pipeline.*，见 docs/storage.md）----------

    /// 造一个测试用的目录：`<dir>/out` 是产物目录，`<dir>/books` 放输入文件。
    fn pipeline_dirs(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("perch-ffi-pipeline-{}-{name}", std::process::id()));
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::create_dir_all(dir.join("books")).unwrap();
        dir
    }

    /// 管线句柄的 RAII 包装：**panic 也要把句柄还回去**。
    ///
    /// 句柄表是进程级全局的、管线配额只有 1；某个测试 panic 时若没归还，
    /// 后面的测试会全部以 `PIPELINE_TOO_MANY_OPEN` 失败，把真正的原因埋掉
    /// （第一版就是这样：一个字段名冲突引发四个测试连环红）。
    struct PipelineGuard(u64);

    impl Drop for PipelineGuard {
        fn drop(&mut self) {
            let _ = dispatch("pipeline.close", &json!({ "handle": self.0 }).to_string());
        }
    }

    fn open_pipeline(dir: &std::path::Path) -> PipelineGuard {
        let out = call_ok(
            "pipeline.open",
            &json!({ "out_dir": dir.join("out").to_string_lossy() }).to_string(),
        );
        PipelineGuard(out["handle"].as_u64().unwrap())
    }

    fn step_pipeline(handle: u64, now_ms: u64) -> Value {
        call_ok(
            "pipeline.step",
            &json!({ "handle": handle, "now_ms": now_ms }).to_string(),
        )
    }

    #[test]
    fn blocked_pipeline_does_not_hold_root_registry_and_close_is_safe() {
        let _serial = serial();
        let pipeline_dir = pipeline_dirs("object-lock");
        let txt_path = txt_fixture_file("object-lock", "甲乙丙丁");
        let pipeline_context = ContextGuard::open();
        let reader_context = ContextGuard::open();

        let opened_pipeline = call_ok_in(
            pipeline_context.id(),
            "pipeline.open",
            &json!({ "out_dir": pipeline_dir.join("out").to_string_lossy() }).to_string(),
        );
        let pipeline_handle = opened_pipeline["handle"].as_u64().unwrap();
        let opened_txt = call_ok_in(
            reader_context.id(),
            "txt.open",
            &json!({ "path": txt_path.to_string_lossy() }).to_string(),
        );
        let txt_handle = opened_txt["handle"].as_u64().unwrap();

        // 人为占住管线自己的锁，模拟长时间 step。status 取到 Arc 租约后
        // 会阻塞在对象锁，但根注册表已经释放。
        let shared = registry()
            .pipeline(pipeline_context.id(), pipeline_handle)
            .unwrap();
        let held = lock_pipeline(&shared);
        let (status_tx, status_rx) = std::sync::mpsc::channel();
        let status_context = pipeline_context.id();
        let status_thread = std::thread::spawn(move || {
            let value = dispatch_in_context(
                status_context,
                "pipeline.status",
                &json!({ "handle": pipeline_handle }).to_string(),
            );
            status_tx.send(value).unwrap();
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::sync::Arc::strong_count(&shared) < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "pipeline.status 没有取得对象租约"
            );
            std::thread::yield_now();
        }

        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let read_context = reader_context.id();
        let read_thread = std::thread::spawn(move || {
            let value = dispatch_in_context(
                read_context,
                "txt.slice",
                &json!({ "handle": txt_handle, "chars": 1 }).to_string(),
            );
            read_tx.send(value).unwrap();
        });
        let read = read_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("另一 context 的 TXT 读取被 pipeline 对象锁阻塞");
        assert_eq!(read["text"], "甲");

        // close 从注册表立即摘除句柄；已取得租约的 status 仍可在解锁后完成。
        let closed = call_ok_in(
            pipeline_context.id(),
            "pipeline.close",
            &json!({ "handle": pipeline_handle }).to_string(),
        );
        assert_eq!(closed["closed"], true);
        let rejected = dispatch_in_context(
            pipeline_context.id(),
            "pipeline.status",
            &json!({ "handle": pipeline_handle }).to_string(),
        );
        assert_eq!(rejected["error"]["code"], "PIPELINE_BAD_HANDLE");

        drop(held);
        let in_flight = status_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("已取得租约的 pipeline.status 未能完成");
        assert!(in_flight.get("error").is_none(), "{in_flight}");
        assert_eq!(in_flight["idle"], true);

        status_thread.join().unwrap();
        read_thread.join().unwrap();
        drop(shared);
        std::fs::remove_dir_all(pipeline_dir).ok();
        std::fs::remove_dir_all(txt_path.parent().unwrap()).ok();
    }

    #[test]
    fn pipeline_runs_a_book_to_ready_through_ffi() {
        let _serial = serial();
        let dir = pipeline_dirs("ready");
        let book = perch_model::BookId::new();
        let book_path = dir.join("books/novel.txt");
        std::fs::write(
            &book_path,
            "第一章 灯塔\n守夜人每天爬上灯塔。\n第二章 归航\n他回来了。",
        )
        .unwrap();

        let pipeline = open_pipeline(&dir);
        let handle = pipeline.0;
        let enqueued = call_ok(
            "pipeline.enqueue",
            &json!({
                "handle": handle,
                "book": book,
                "path": book_path.to_string_lossy(),
                "format": "txt",
            })
            .to_string(),
        );
        assert_eq!(enqueued["outcome"], "queued");
        assert_eq!(enqueued["stage"], "refine");

        // 一次一步：②精修 → ③索引
        assert_eq!(step_pipeline(handle, 1_000)["outcome"], "refined");
        let indexed = step_pipeline(handle, 2_000);
        assert_eq!(indexed["outcome"], "indexed");
        assert_eq!(indexed["stage"], "index");
        assert!(indexed["tokens"].as_u64().unwrap() > 0);

        // 队列空了，再 step 是 idle
        assert_eq!(step_pipeline(handle, 3_000)["outcome"], "idle");
        let status = call_ok("pipeline.status", &json!({ "handle": handle }).to_string());
        assert_eq!(status["idle"], true);
        assert_eq!(status["stats"]["pendingRefine"], 0);
        assert_eq!(status["stats"]["inFlight"], 0);

        // 状态落到磁盘，而且四个产物文件都在
        let state = call_ok(
            "pipeline.state",
            &json!({ "handle": handle, "book": book }).to_string(),
        );
        assert_eq!(state["state"], "ready");
        let book_dir = dir.join("out").join(book.to_string());
        for name in ["state.json", "refinement.json", "index.bin", "text.json"] {
            assert!(book_dir.join(name).exists(), "缺少 {name}");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipeline_open_reports_and_rebuilds_damaged_ready_artifacts() {
        let _serial = serial();
        let dir = pipeline_dirs("artifact-recovery");
        let book = perch_model::BookId::new();
        let book_path = dir.join("books/recover.txt");
        std::fs::write(&book_path, "损坏产物恢复后仍可搜索").unwrap();

        {
            let pipeline = open_pipeline(&dir);
            call_ok(
                "pipeline.enqueue",
                &json!({
                    "handle": pipeline.0,
                    "book": book,
                    "path": book_path.to_string_lossy(),
                    "format": "txt",
                })
                .to_string(),
            );
            assert_eq!(step_pipeline(pipeline.0, 1_000)["outcome"], "refined");
            assert_eq!(step_pipeline(pipeline.0, 2_000)["outcome"], "indexed");
        }

        let index_path = dir.join("out").join(book.to_string()).join("index.bin");
        std::fs::write(&index_path, b"broken index").unwrap();
        let reopened = call_ok(
            "pipeline.open",
            &json!({ "out_dir": dir.join("out") }).to_string(),
        );
        let recovered = &reopened["recovered"][0];
        assert_eq!(recovered["book"].as_str().unwrap(), book.to_string());
        assert_eq!(recovered["state"], "indexing");
        assert!(
            recovered["reason"]
                .as_str()
                .unwrap()
                .starts_with("STORE_INDEX_INVALID")
        );

        let pipeline = PipelineGuard(reopened["handle"].as_u64().unwrap());
        let queued = call_ok(
            "pipeline.enqueue",
            &json!({
                "handle": pipeline.0,
                "book": book,
                "path": book_path.to_string_lossy(),
                "format": "txt",
            })
            .to_string(),
        );
        assert_eq!(queued["state"], "indexing");
        assert_eq!(queued["stage"], "index");
        assert_eq!(step_pipeline(pipeline.0, 3_000)["outcome"], "indexed");
        assert!(perch_index::Index::load_from(index_path).is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipeline_recovers_state_and_attempts_after_restart() {
        let _serial = serial();
        let dir = pipeline_dirs("recover");
        let book = perch_model::BookId::new();
        let book_path = dir.join("books/broken.epub");
        std::fs::write(&book_path, b"definitely not an epub").unwrap();

        // 第一次运行：②失败一次（attempts=1），然后"App 被杀"
        let pipeline = open_pipeline(&dir);
        let handle = pipeline.0;
        call_ok(
            "pipeline.enqueue",
            &json!({
                "handle": handle,
                "book": book,
                "path": book_path.to_string_lossy(),
                "format": "epub",
            })
            .to_string(),
        );
        let failed = step_pipeline(handle, 1_000);
        assert_eq!(failed["outcome"], "failed");
        assert_eq!(failed["attempts"], 1);
        call_ok("pipeline.close", &json!({ "handle": handle }).to_string());

        // 第二次运行：open 应当把上次的状态与失败次数报回来
        let reopened = call_ok(
            "pipeline.open",
            &json!({ "out_dir": dir.join("out").to_string_lossy() }).to_string(),
        );
        let recovered = reopened["recovered"].as_array().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0]["book"].as_str().unwrap(), book.to_string());
        assert_eq!(recovered[0]["state"], "failed");
        assert_eq!(
            recovered[0]["attempts"], 1,
            "失败次数必须持久化，否则重启后无限重试"
        );

        // 再 enqueue：以磁盘上的状态为准（不同阶段重来），且带上历史失败次数
        let reopened_pipeline = PipelineGuard(reopened["handle"].as_u64().unwrap());
        let handle = reopened_pipeline.0;
        let enqueued = call_ok(
            "pipeline.enqueue",
            &json!({
                "handle": handle,
                "book": book,
                "path": book_path.to_string_lossy(),
                "format": "epub",
                "state": "registered",
            })
            .to_string(),
        );
        assert_eq!(
            enqueued["state"], "failed",
            "磁盘状态优先于调用方给的初始状态"
        );
        assert_eq!(enqueued["outcome"], "queued");
        assert_eq!(step_pipeline(handle, 2_999)["outcome"], "idle");
        assert_eq!(step_pipeline(handle, 3_000)["attempts"], 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failures_exhaust_across_restarts_and_manual_retry_is_durable() {
        let _serial = serial();
        let dir = pipeline_dirs("exhaust-restart");
        let path = dir.join("bad.epub");
        std::fs::write(&path, b"broken epub").unwrap();
        let book = perch_model::BookId::new();
        for attempt in 1..=3 {
            let pipeline = open_pipeline(&dir);
            call_ok(
                "pipeline.enqueue",
                &json!({"handle": pipeline.0, "book": book,
                "path": path, "format": "epub"})
                .to_string(),
            );
            let result = step_pipeline(pipeline.0, attempt * 100_000);
            assert_eq!(result["attempts"], attempt);
            assert!(result["reason"].as_str().unwrap().contains("JOBS_EPUB"));
            assert_eq!(
                result["outcome"],
                if attempt == 3 { "exhausted" } else { "failed" }
            );
        }
        {
            let pipeline = open_pipeline(&dir);
            call_ok(
                "pipeline.enqueue",
                &json!({"handle": pipeline.0, "book": book,
                "path": path, "format": "epub"})
                .to_string(),
            );
            let status = call_ok(
                "pipeline.status",
                &json!({"handle": pipeline.0}).to_string(),
            );
            assert_eq!(status["stats"]["exhausted"], 1);
            assert_eq!(step_pipeline(pipeline.0, 900_000)["outcome"], "idle");
            let checkpoint = dir.join("out").join(book.to_string()).join("state.tmp");
            std::fs::create_dir(&checkpoint).unwrap();
            let retry = dispatch(
                "pipeline.retry",
                &json!({"handle": pipeline.0, "book": book}).to_string(),
            );
            assert_eq!(retry["error"]["code"], "PIPELINE_IO");
            std::fs::remove_dir(checkpoint).unwrap();
            let retry = call_ok(
                "pipeline.retry",
                &json!({"handle": pipeline.0, "book": book}).to_string(),
            );
            assert_eq!(retry["retried"], true);
            // Exit immediately: no step is run after resetting the failure budget.
        }
        {
            let pipeline = open_pipeline(&dir);
            let result = call_ok(
                "pipeline.enqueue",
                &json!({"handle": pipeline.0, "book": book,
                "path": path, "format": "epub"})
                .to_string(),
            );
            assert_eq!(result["state"], "registered");
            assert_eq!(result["attempts"], 0);
            assert_eq!(step_pipeline(pipeline.0, 1_000_000)["attempts"], 1);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn index_failure_restores_stage_and_legacy_failure_starts_refine() {
        let _serial = serial();
        let dir = pipeline_dirs("index-failure");
        let book = perch_model::BookId::new();
        let path = dir.join("book.txt");
        std::fs::write(&path, "正文内容").unwrap();
        {
            let pipeline = open_pipeline(&dir);
            call_ok(
                "pipeline.enqueue",
                &json!({"handle": pipeline.0, "book": book,
                "path": path, "format": "txt"})
                .to_string(),
            );
            assert_eq!(step_pipeline(pipeline.0, 1_000)["outcome"], "refined");
            std::fs::remove_file(&path).unwrap();
            assert_eq!(step_pipeline(pipeline.0, 2_000)["stage"], "index");
        }
        {
            let pipeline = open_pipeline(&dir);
            let result = call_ok(
                "pipeline.enqueue",
                &json!({"handle": pipeline.0, "book": book,
                "path": path, "format": "txt"})
                .to_string(),
            );
            assert_eq!(result["stage"], "index");
            assert_eq!(step_pipeline(pipeline.0, 3_999)["outcome"], "idle");
            std::fs::write(&path, "正文修复").unwrap();
            assert_eq!(step_pipeline(pipeline.0, 4_000)["outcome"], "indexed");
        }
        let state_path = dir.join("out").join(book.to_string()).join("state.json");
        std::fs::write(state_path, br#"{"state":"failed","attempts":1}"#).unwrap();
        {
            let pipeline = open_pipeline(&dir);
            let result = call_ok(
                "pipeline.enqueue",
                &json!({"handle": pipeline.0, "book": book,
                "path": path, "format": "txt"})
                .to_string(),
            );
            assert_eq!(result["stage"], "refine");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn checkpoint_write_error_is_pipeline_io_and_can_resume() {
        let _serial = serial();
        let dir = pipeline_dirs("checkpoint-io");
        let book = perch_model::BookId::new();
        let path = dir.join("book.txt");
        std::fs::write(&path, "正文内容").unwrap();
        let pipeline = open_pipeline(&dir);
        call_ok(
            "pipeline.enqueue",
            &json!({"handle": pipeline.0, "book": book,
            "path": path, "format": "txt"})
            .to_string(),
        );
        let temp = dir.join("out").join(book.to_string()).join("state.tmp");
        std::fs::create_dir_all(&temp).unwrap();
        let error = dispatch(
            "pipeline.step",
            &json!({"handle": pipeline.0, "now_ms": 1000}).to_string(),
        );
        assert_eq!(error["error"]["code"], "PIPELINE_IO");
        std::fs::remove_dir(temp).unwrap();
        assert_eq!(step_pipeline(pipeline.0, 5_000)["outcome"], "refined");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pdf_analyze_accepts_path_and_rejects_ambiguous_input() {
        let dir = pipeline_dirs("pdf-path");
        let path = dir.join("sample.pdf");
        std::fs::write(&path, pdf_fixture(1)).unwrap();
        let result = call_ok("pdf.analyze", &json!({"path": path}).to_string());
        assert_eq!(result["pageCount"], 1);
        let error = dispatch(
            "pdf.analyze",
            &json!({"path": path, "bytes_hex": ""}).to_string(),
        );
        assert_eq!(error["error"]["code"], "FFI_BAD_ARGS");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pipeline_rejects_bad_arguments_and_wrong_handle_kinds() {
        let _serial = serial();
        let dir = pipeline_dirs("args");
        let pipeline = open_pipeline(&dir);
        let handle = pipeline.0;
        let book = perch_model::BookId::new();
        let book_path = dir.join("books/a.txt");
        std::fs::write(&book_path, "正文").unwrap();

        // format 必填：嗅探会把损坏的 EPUB 当纯文本收下
        let no_format = dispatch(
            "pipeline.enqueue",
            &json!({
                "handle": handle,
                "book": book,
                "path": book_path.to_string_lossy(),
            })
            .to_string(),
        );
        assert_eq!(no_format["error"]["code"], "FFI_BAD_ARGS");

        // book 必须是合法 UUID
        let bad_book = dispatch(
            "pipeline.enqueue",
            &json!({
                "handle": handle,
                "book": "not-a-uuid",
                "path": book_path.to_string_lossy(),
                "format": "txt",
            })
            .to_string(),
        );
        assert!(
            bad_book["error"]["code"] == "FFI_BAD_ARGS",
            "实际: {bad_book}"
        );

        // 未知方法名之外，还要挡住"拿错句柄类型"
        let txt = call_ok(
            "txt.open",
            &json!({ "path": book_path.to_string_lossy() }).to_string(),
        );
        let txt_handle = txt["handle"].as_u64().unwrap();
        let wrong = dispatch(
            "pipeline.step",
            &json!({ "handle": txt_handle, "now_ms": 1_000 }).to_string(),
        );
        assert_eq!(wrong["error"]["code"], "HANDLE_WRONG_KIND");
        // 反过来：pipeline 句柄不能走 book.close，而且不能被关掉
        let wrong_close = dispatch("book.close", &json!({ "handle": handle }).to_string());
        assert_eq!(wrong_close["error"]["code"], "HANDLE_WRONG_KIND");
        assert_eq!(step_pipeline(handle, 2_000)["outcome"], "idle");

        // 管线只允许一条
        let second = dispatch(
            "pipeline.open",
            &json!({ "out_dir": dir.join("out").to_string_lossy() }).to_string(),
        );
        assert_eq!(second["error"]["code"], "PIPELINE_TOO_MANY_OPEN");

        call_ok("txt.close", &json!({ "handle": txt_handle }).to_string());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipeline_open_reports_resumed_books_and_status_counts() {
        let _serial = serial();
        let dir = pipeline_dirs("counts");
        let pipeline = open_pipeline(&dir);
        let handle = pipeline.0;

        // 三本书：两本能做完，一本指向不存在的文件（会失败）
        let good_a = dir.join("books/a.txt");
        let good_b = dir.join("books/b.txt");
        std::fs::write(&good_a, "第一章 甲\n正文甲。").unwrap();
        std::fs::write(&good_b, "第一章 乙\n正文乙。").unwrap();

        let mut books = Vec::new();
        for (path, format) in [
            (good_a.clone(), "txt"),
            (good_b.clone(), "txt"),
            (dir.join("books/missing.txt"), "txt"),
        ] {
            let book = perch_model::BookId::new();
            books.push(book);
            call_ok(
                "pipeline.enqueue",
                &json!({
                    "handle": handle,
                    "book": book,
                    "path": path.to_string_lossy(),
                    "format": format,
                })
                .to_string(),
            );
        }

        // 排队数应当能被看见（书架上的"正在精修 N 本"读它）
        let status = call_ok("pipeline.status", &json!({ "handle": handle }).to_string());
        let pending = status["stats"]["pendingRefine"].as_u64().unwrap();
        assert_eq!(pending, 3, "三本都排着队");

        // 一路做到没有活可干
        let mut seen = Vec::new();
        for tick in 0..20 {
            let outcome = step_pipeline(handle, 1_000 + tick * 10_000);
            let name = outcome["outcome"].as_str().unwrap_or_default().to_owned();
            if name == "idle" || name == "exhausted" {
                seen.push(name);
                break;
            }
            seen.push(name);
        }
        assert!(seen.contains(&"refined".to_owned()), "实际: {seen:?}");
        assert!(seen.contains(&"failed".to_owned()), "实际: {seen:?}");

        // 失败的那本留着 attempts，另两本 ready
        let missing = call_ok(
            "pipeline.state",
            &json!({ "handle": handle, "book": books[2] }).to_string(),
        );
        assert_eq!(missing["state"], "failed");
        assert!(missing["attempts"].as_u64().unwrap() >= 1);

        std::fs::remove_dir_all(&dir).ok();
    }
}
