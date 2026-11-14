//! [`BookStore`] 的一个**按路径实现**：平台只给"书在哪"和"产物写哪"，其余都在 Rust 里。
//!
//! # 为什么不是"平台实现，Rust 回调"
//!
//! 回调（C 函数指针 / `NativeCallable`）能让平台完全掌控存储，但跨语言回调 + 阻塞调用 +
//! 错误传播**只能在设备上验证**；而本项目的约束是"能本地验的优先"。
//! 按路径实现的代价是：存储布局暂时由 Rust 决定；将来换 SQLite 时换成 `SqliteStore` 即可，
//! **驱动循环与调度一个字都不用改**（`docs/storage.md`「模块与存储边界」）。
//!
//! # 目录布局
//!
//! ```text
//! <out_dir>/<bookId>/
//!   state.json        进度与失败次数（崩溃恢复读它）
//!   refinement.json   阶段②产物（元数据 / 目录 / 字数 / coverHref）
//!   index.bin         阶段③产物：倒排索引（delta + varint）
//!   text.json         阶段③产物：正文 + 章节映射（搜索命中还原 Locator 要用）
//! ```
//!
//! # 崩溃恢复的两个要点
//!
//! 1. **状态先落盘再做活**：`set_state` 在 [`crate::Pipeline::step`] 里总在真正的工作
//!    之前调用，所以崩溃后看到的最坏情况是"这一步没做完"，而②③都是幂等的，重做安全。
//! 2. **失败次数也要落盘**：`perch_pipeline::Scheduler::resume` 的注释写得很清楚——
//!    attempts 不持久化，重启就会把失败次数清零、**无限重试**。
//!    所以 `state.json` 里带 `attempts`，由调用方在收到失败结果时调用
//!    [`PathStore::record_attempts`]。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use perch_model::{BookId, Format, ImportState};
use serde::{Deserialize, Serialize};

use crate::pipeline::BookStore;
use crate::{BookText, MappedBookText, Refinement};

const RECOVERY_STATE_MISSING: &str = "STORE_STATE_MISSING";
const RECOVERY_STATE_UNREADABLE: &str = "STORE_STATE_UNREADABLE";
const RECOVERY_STATE_CORRUPTED: &str = "STORE_STATE_CORRUPTED";
const RECOVERY_REFINEMENT_INVALID: &str = "STORE_REFINEMENT_INVALID";
const RECOVERY_INDEX_INVALID: &str = "STORE_INDEX_INVALID";

/// `state.json` 的形状。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateFile {
    /// 当前导入状态。
    pub state: ImportState,
    /// 同类失败的次数（退避重试要用；缺省 0 兼容旧文件）。
    #[serde(default)]
    pub attempts: u16,
    /// 最后更新时间（毫秒，仅供排查）。
    #[serde(default)]
    pub updated_at_ms: u64,
    /// Stage that failed; absent in older checkpoints.
    #[serde(default)]
    pub failed_stage: Option<perch_pipeline::Stage>,
    /// Last business failure or a `STORE_*` recovery diagnostic.
    #[serde(default)]
    pub reason: Option<String>,
    /// Absolute retry deadline; None means exhausted.
    #[serde(default)]
    pub retry_at_ms: Option<u64>,
}

/// [`PathStore::new`] 从磁盘恢复出来的一条记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// 书。
    pub book: BookId,
    /// 崩溃前停在的状态。
    pub state: ImportState,
    /// 崩溃前的失败次数。
    pub attempts: u16,
    /// Failed stage.
    pub failed_stage: Option<perch_pipeline::Stage>,
    /// Last business failure or recovery diagnostic.
    pub reason: Option<String>,
    /// Absolute retry deadline.
    pub retry_at_ms: Option<u64>,
}

/// 书的来源：文件路径 + 格式。
#[derive(Debug, Clone)]
struct Source {
    path: PathBuf,
    format: Format,
}

/// 按路径读、按目录写的 [`BookStore`]。
///
/// **文件路径不进 `state.json`**：iOS 沙盒目录在重启/升级后会变，存下来下次也未必对。
/// 平台自己知道每本书在哪（它有书库数据库），`enqueue` 时再给一次即可。
#[derive(Debug)]
pub struct PathStore {
    out_dir: PathBuf,
    books: HashMap<BookId, Source>,
    states: HashMap<BookId, StateFile>,
    recovered: Vec<Recovered>,
}

impl PathStore {
    /// 打开（或新建）一个产物目录，并扫描已有 `state.json` 做崩溃恢复。
    ///
    /// # Errors
    ///
    /// 目录建不出来或根目录无法扫描时返回错误。单书检查点或产物损坏
    /// 不会阻断其它书；该书会降级到可安全重做的阶段，并在 [`Recovered::reason`]
    /// 中带上稳定的 `STORE_*` 原因前缀。
    pub fn new(out_dir: impl Into<PathBuf>) -> Result<Self, String> {
        let out_dir = out_dir.into();
        std::fs::create_dir_all(&out_dir)
            .map_err(|error| format!("建不出产物目录 {}: {error}", out_dir.display()))?;

        let mut store = Self {
            out_dir,
            books: HashMap::new(),
            states: HashMap::new(),
            recovered: Vec::new(),
        };
        store.scan()?;
        Ok(store)
    }

    /// 扫描子目录，重建"崩溃前做到哪了"。
    fn scan(&mut self) -> Result<(), String> {
        let entries = std::fs::read_dir(&self.out_dir)
            .map_err(|error| format!("读不了产物目录 {}: {error}", self.out_dir.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue, // 单个目录项读不了就跳过，不让它挡住启动
            };
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            // 目录名就是书 id（UUID）
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(book) = name.parse::<uuid::Uuid>() else {
                continue; // 不是我们建的目录，忽略
            };
            let book = BookId::from_uuid(book);

            let state = self.recover_book(&entry.path());

            self.states.insert(book, state.clone());
            self.recovered.push(Recovered {
                book,
                state: state.state,
                attempts: state.attempts,
                failed_stage: state.failed_stage,
                reason: state.reason,
                retry_at_ms: state.retry_at_ms,
            });
        }
        // 顺序稳定，便于测试与日志
        self.recovered.sort_by_key(|item| item.book);
        Ok(())
    }

    fn recover_book(&self, dir: &Path) -> StateFile {
        let path = dir.join("state.json");
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<StateFile>(&bytes) {
                Ok(state) => state,
                Err(error) => recovery_state(
                    ImportState::Registered,
                    format!("{RECOVERY_STATE_CORRUPTED}: {error}"),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => recovery_state(
                ImportState::Registered,
                format!("{RECOVERY_STATE_MISSING}: state.json 不存在"),
            ),
            Err(error) => recovery_state(
                ImportState::Registered,
                format!("{RECOVERY_STATE_UNREADABLE}: {error}"),
            ),
        };
        audit_checkpoint(dir, state)
    }

    /// 恢复出来的记录（`new` 时扫描到的）。
    #[must_use]
    pub fn recovered(&self) -> &[Recovered] {
        &self.recovered
    }

    /// 登记一本书的来源（`enqueue` 时调用）。
    pub fn register(&mut self, book: BookId, path: impl Into<PathBuf>, format: Format) {
        self.books.insert(
            book,
            Source {
                path: path.into(),
                format,
            },
        );
    }

    /// 这本书当前的落盘状态（没有则 `None`）。
    #[must_use]
    pub fn state(&self, book: BookId) -> Option<StateFile> {
        self.states.get(&book).cloned()
    }

    /// 这本书的产物目录。
    #[must_use]
    pub fn book_dir(&self, book: BookId) -> PathBuf {
        self.out_dir.join(book.to_string())
    }

    /// 把失败次数落盘（重启后不会清零、不会无限重试）。
    ///
    /// # Errors
    ///
    /// 写盘失败时返回错误——调用方可以选择忽略（调度器仍会退避重试），
    /// 但**不该假装成功**。
    pub fn record_attempts(&mut self, book: BookId, attempts: u16) -> Result<(), String> {
        let mut state = self.states.get(&book).cloned().unwrap_or(StateFile {
            state: ImportState::Failed,
            attempts: 0,
            updated_at_ms: 0,
            failed_stage: None,
            reason: None,
            retry_at_ms: None,
        });
        state.attempts = attempts;
        self.write_state(book, state)
    }

    /// 写 `state.json`（并更新内存里的副本）。
    fn write_state(&mut self, book: BookId, mut state: StateFile) -> Result<(), String> {
        state.updated_at_ms = now_ms();
        let path = self.book_dir(book).join("state.json");
        let bytes = serde_json::to_vec_pretty(&state)
            .map_err(|error| format!("序列化 state.json 失败: {error}"))?;
        write_atomic(&path, &bytes)?;
        self.states.insert(book, state);
        Ok(())
    }
}

impl BookStore for PathStore {
    fn read_bytes(&mut self, book: BookId) -> Result<Vec<u8>, String> {
        let source = self
            .books
            .get(&book)
            .ok_or_else(|| format!("书 {book} 还没登记来源（先 enqueue）"))?;
        std::fs::read(&source.path)
            .map_err(|error| format!("读不了 {}: {error}", source.path.display()))
    }

    fn format(&mut self, book: BookId) -> Result<Format, String> {
        self.books
            .get(&book)
            .map(|source| source.format)
            .ok_or_else(|| format!("书 {book} 还没登记来源（先 enqueue）"))
    }

    fn set_state(&mut self, book: BookId, state: ImportState) -> Result<(), String> {
        let mut checkpoint = self.states.get(&book).cloned().unwrap_or(StateFile {
            state,
            attempts: 0,
            updated_at_ms: 0,
            failed_stage: None,
            reason: None,
            retry_at_ms: None,
        });
        checkpoint.state = state;
        self.write_state(book, checkpoint)
    }

    fn record_failure(
        &mut self,
        book: BookId,
        stage: perch_pipeline::Stage,
        attempts: u16,
        reason: &str,
        retry_at_ms: Option<u64>,
    ) -> Result<(), String> {
        self.write_state(
            book,
            StateFile {
                state: ImportState::Failed,
                attempts,
                updated_at_ms: 0,
                failed_stage: Some(stage),
                reason: Some(reason.to_owned()),
                retry_at_ms,
            },
        )
    }

    fn reset(&mut self, book: BookId, state: ImportState) -> Result<(), String> {
        self.write_state(
            book,
            StateFile {
                state,
                attempts: 0,
                updated_at_ms: 0,
                failed_stage: None,
                reason: None,
                retry_at_ms: None,
            },
        )
    }

    fn save_refinement(&mut self, book: BookId, refinement: &Refinement) -> Result<(), String> {
        let path = self.book_dir(book).join("refinement.json");
        let bytes = serde_json::to_vec_pretty(refinement)
            .map_err(|error| format!("序列化 refinement.json 失败: {error}"))?;
        write_atomic(&path, &bytes)
    }

    fn save_index(
        &mut self,
        book: BookId,
        index: perch_index::Index,
        text: BookText,
    ) -> Result<(), String> {
        let dir = self.book_dir(book);
        // 索引自带落盘格式（分片 + delta + varint），直接用它
        index
            .save_to(dir.join("index.bin"))
            .map_err(|error| format!("写 index.bin 失败: {error}"))?;
        // 读取器可兼容 v0，但当前写入器永远产生 v1。
        let bytes = crate::book_text_to_vec(&text)
            .map_err(|error| format!("序列化 text.json 失败: {error}"))?;
        write_atomic(&dir.join("text.json"), &bytes)
    }

    fn save_index_with_source_map(
        &mut self,
        book: BookId,
        index: perch_index::Index,
        text: MappedBookText,
    ) -> Result<(), String> {
        let dir = self.book_dir(book);
        index
            .save_to(dir.join("index.bin"))
            .map_err(|error| format!("写 index.bin 失败: {error}"))?;
        let bytes = crate::mapped_book_text_to_vec(&text)
            .map_err(|error| format!("序列化 text.json 失败: {error}"))?;
        write_atomic(&dir.join("text.json"), &bytes)
    }
}

/// 原子写：先写 `.tmp` 再 `rename`。
///
/// 直接 `write` 覆盖的话，写到一半崩溃会留下一个**半截的 JSON**，
/// 下次启动解析失败——而"读不出来"比"没写"更难处理（后者只需重做）。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("建不出 {}: {error}", parent.display()))?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, bytes).map_err(|error| format!("写不了 {}: {error}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|error| {
        // 尽力清掉临时文件，不留垃圾
        std::fs::remove_file(&temp).ok();
        format!("rename 到 {} 失败: {error}", path.display())
    })
}

fn recovery_state(state: ImportState, reason: String) -> StateFile {
    StateFile {
        state,
        attempts: 0,
        updated_at_ms: 0,
        failed_stage: None,
        reason: Some(reason),
        retry_at_ms: None,
    }
}

fn audit_checkpoint(dir: &Path, mut state: StateFile) -> StateFile {
    let needs_refinement = matches!(state.state, ImportState::Indexing | ImportState::Ready)
        || (state.state == ImportState::Failed
            && state.failed_stage == Some(perch_pipeline::Stage::Index));

    if needs_refinement && let Err(error) = audit_refinement(dir) {
        let reason = format!("{RECOVERY_REFINEMENT_INVALID}: {error}");
        if state.state == ImportState::Failed {
            state.failed_stage = Some(perch_pipeline::Stage::Refine);
            state.reason = Some(append_recovery_reason(state.reason.take(), reason));
        } else {
            state = recovery_state(ImportState::Registered, reason);
        }
        return state;
    }

    if state.state == ImportState::Ready
        && let Err(error) = audit_index_group(dir)
    {
        return recovery_state(
            ImportState::Indexing,
            format!("{RECOVERY_INDEX_INVALID}: {error}"),
        );
    }

    state
}

fn append_recovery_reason(previous: Option<String>, recovery: String) -> String {
    match previous {
        Some(previous) => format!("{recovery}; previous: {previous}"),
        None => recovery,
    }
}

fn audit_refinement(dir: &Path) -> Result<(), String> {
    let path = dir.join("refinement.json");
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice::<Refinement>(&bytes)
        .map(|_| ())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn audit_index_group(dir: &Path) -> Result<(), String> {
    let index_path = dir.join("index.bin");
    let index = perch_index::Index::load_from(&index_path)
        .map_err(|error| format!("{}: {error}", index_path.display()))?;
    let text_path = dir.join("text.json");
    let bytes =
        std::fs::read(&text_path).map_err(|error| format!("{}: {error}", text_path.display()))?;
    let text = crate::mapped_book_text_from_slice(&bytes)
        .map_err(|error| format!("{}: {error}", text_path.display()))?;
    text.source_map()
        .map_err(|error| format!("{}: {error}", text_path.display()))?;
    if index.text() != text.book_text().text {
        return Err("index.bin 与 text.json 正文不一致".to_owned());
    }
    Ok(())
}

/// 现在（毫秒）。只用于 `state.json` 里的排查字段，不参与任何逻辑判断。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::tests::build_epub;
    use crate::{
        ExtractOptions, extract_text, index_book,
        pipeline::{Pipeline, StepOutcome},
        refine,
    };
    use perch_pipeline::{EnqueueOutcome, Gate, Policy, Priority};

    /// 每个测试一个独立目录（进程内唯一，避免并行互相覆盖）。
    fn temp_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "perch-store-{}-{name}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_book(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn options() -> ExtractOptions {
        ExtractOptions::default()
    }

    fn build_ready_txt(dir: &Path, book: BookId, path: &Path) {
        let mut pipeline = Pipeline::new(
            PathStore::new(dir).unwrap(),
            Policy::default(),
            options(),
            64,
        )
        .unwrap();
        pipeline.store_mut().register(book, path, Format::Txt);
        pipeline.enqueue(book, ImportState::Registered, Priority::Normal);
        let summary = pipeline.run_until_idle(1_000, Gate::foreground(), 8);
        assert_eq!(summary.refined, 1);
        assert_eq!(summary.indexed, 1);
    }

    #[test]
    fn incomplete_indexing_checkpoint_downgrades_to_refine() {
        let dir = temp_dir("state");
        let book = BookId::new();

        {
            let mut store = PathStore::new(&dir).unwrap();
            store.register(book, dir.join("a.epub"), Format::Epub);
            store.set_state(book, ImportState::Indexing).unwrap();
            store.record_attempts(book, 2).unwrap();
        }

        // indexing 本应已有 refinement.json。缺失表示检查点与产物不一致，
        // 必须回到阶段二，不能直接把半成品当作可续传的索引阶段。
        let reopened = PathStore::new(&dir).unwrap();
        let recovered = reopened.recovered();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].book, book);
        assert_eq!(recovered[0].state, ImportState::Registered);
        assert_eq!(recovered[0].attempts, 0);
        assert!(
            recovered[0]
                .reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with(RECOVERY_REFINEMENT_INVALID))
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn set_state_keeps_previously_recorded_attempts() {
        let dir = temp_dir("attempts");
        let book = BookId::new();
        let mut store = PathStore::new(&dir).unwrap();
        store.register(book, dir.join("a.txt"), Format::Txt);

        store.record_attempts(book, 3).unwrap();
        store.set_state(book, ImportState::Refining).unwrap();

        let state = store.state(book).unwrap();
        assert_eq!(state.state, ImportState::Refining);
        assert_eq!(state.attempts, 3, "set_state 不该把失败次数抹掉");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_state_file_is_recovered_without_blocking_other_books() {
        let dir = temp_dir("corrupt");
        let good = BookId::new();
        let broken = BookId::new();
        let missing = BookId::new();

        {
            let mut store = PathStore::new(&dir).unwrap();
            store.register(good, dir.join("a.txt"), Format::Txt);
            store.set_state(good, ImportState::Registered).unwrap();
        }
        // 手写一个半截的 state.json（模拟"写到一半崩溃"）
        let broken_dir = dir.join(broken.to_string());
        std::fs::create_dir_all(&broken_dir).unwrap();
        std::fs::write(broken_dir.join("state.json"), b"{\"state\": \"ind").unwrap();
        // UUID 目录已建立、但 state.json 还没提交（模拟首次写检查点前中断）。
        std::fs::create_dir_all(dir.join(missing.to_string())).unwrap();
        // 还有一个根本不是 UUID 的目录，也该被忽略
        std::fs::create_dir_all(dir.join("not-a-book")).unwrap();

        let reopened = PathStore::new(&dir).unwrap();
        let recovered = reopened.recovered();
        assert_eq!(recovered.len(), 3);
        let good = recovered.iter().find(|item| item.book == good).unwrap();
        assert_eq!(good.state, ImportState::Registered);
        assert_eq!(good.reason, None);
        let broken = recovered.iter().find(|item| item.book == broken).unwrap();
        assert_eq!(broken.state, ImportState::Registered);
        assert!(
            broken
                .reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with(RECOVERY_STATE_CORRUPTED))
        );
        let missing = recovered.iter().find(|item| item.book == missing).unwrap();
        assert_eq!(missing.state, ImportState::Registered);
        assert!(
            missing
                .reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with(RECOVERY_STATE_MISSING))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unregistered_book_reports_a_readable_error() {
        let dir = temp_dir("unregistered");
        let mut store = PathStore::new(&dir).unwrap();
        let error = store.read_bytes(BookId::new()).unwrap_err();
        assert!(error.contains("还没登记来源"), "实际信息：{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn artifacts_are_written_and_parse_back() {
        let dir = temp_dir("artifacts");
        let book = BookId::new();
        let epub = build_epub(&["第一章正文", "第二章正文"], true, true);
        let path = write_book(&dir, "book.epub", &epub);

        let mut store = PathStore::new(&dir).unwrap();
        store.register(book, &path, Format::Epub);

        let refinement = refine(&epub, Format::Epub, options()).unwrap();
        store.save_refinement(book, &refinement).unwrap();

        let text = extract_text(&epub, Format::Epub, options()).unwrap();
        let index = index_book(&text, 64);
        let expected_hits = index.search("第一章正文").len();
        store.save_index(book, index, text.clone()).unwrap();
        store.set_state(book, ImportState::Ready).unwrap();

        // 四个文件都在
        let book_dir = store.book_dir(book);
        for name in ["state.json", "refinement.json", "index.bin", "text.json"] {
            assert!(book_dir.join(name).exists(), "缺少 {name}");
        }
        // 临时文件不许残留
        assert!(!book_dir.join("state.tmp").exists());
        assert!(!book_dir.join("text.tmp").exists());

        // refinement.json 能读回来，且与内存里的一致
        let bytes = std::fs::read(book_dir.join("refinement.json")).unwrap();
        let restored: Refinement = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, refinement);

        // index.bin 能按既有格式读回来，且搜索结果一致
        let loaded = perch_index::Index::load_from(book_dir.join("index.bin")).unwrap();
        assert_eq!(loaded.search("第一章正文").len(), expected_hits);

        // text.json 能读回来，章节映射还在
        let text_bytes = std::fs::read(book_dir.join("text.json")).unwrap();
        assert!(
            String::from_utf8_lossy(&text_bytes).contains("\"version\":1"),
            "当前写入器必须产生 text v1"
        );
        let restored_text = crate::book_text_from_slice(&text_bytes).unwrap();
        assert_eq!(restored_text, text);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipeline_reaches_ready_and_writes_every_artifact() {
        // 走完整条路：登记 → 阶段② → 阶段③ → ready，全部落在真实目录里
        let dir = temp_dir("e2e");
        let book = BookId::new();
        let content = "第一章 灯塔\n守夜人每天爬上灯塔。\n第二章 归航\n他终于回来了。";
        let path = write_book(&dir, "book.txt", content.as_bytes());

        let mut pipeline = Pipeline::new(
            PathStore::new(&dir).unwrap(),
            Policy::default(),
            options(),
            64,
        )
        .unwrap();
        pipeline.store_mut().register(book, &path, Format::Txt);
        pipeline.enqueue(book, ImportState::Registered, Priority::Normal);

        let summary = pipeline.run_until_idle(1_000, Gate::foreground(), 8);
        assert_eq!(summary.refined, 1);
        assert_eq!(summary.indexed, 1);
        assert_eq!(summary.failed, 0);
        assert!(pipeline.is_idle());

        let book_dir = pipeline.store().book_dir(book);
        for name in ["state.json", "refinement.json", "index.bin", "text.json"] {
            assert!(book_dir.join(name).exists(), "缺少 {name}");
        }
        assert_eq!(
            pipeline.store().state(book).map(|state| state.state),
            Some(ImportState::Ready)
        );

        // 重启：ready 的书恢复出来是 ready，再 enqueue 应当 NotNeeded（不重复做活）
        let mut reopened = pipeline_after_restart(&dir, book, &path);
        let outcome = reopened.resume(book, ImportState::Ready, Priority::Normal, 0);
        assert_eq!(outcome, EnqueueOutcome::NotNeeded);
        assert_eq!(reopened.step(2_000, Gate::foreground()), StepOutcome::Idle);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipeline_persists_source_maps_for_txt_and_epub_with_unchanged_index_format() {
        for (format, content) in [
            (Format::Txt, "\r\n甲\r\n\r\n\r\n乙😀é".as_bytes().to_vec()),
            (
                Format::Epub,
                build_epub(
                    &["甲&amp;😀<![CDATA[乙]]><b>重复</b>", "重复"],
                    false,
                    false,
                ),
            ),
        ] {
            let dir = temp_dir("source-map");
            let book = BookId::new();
            let path = write_book(&dir, "source", &content);
            let direct = crate::extract_text_with_source_map(&content, format, options()).unwrap();
            let mut pipeline = Pipeline::new(
                PathStore::new(&dir).unwrap(),
                Policy::default(),
                options(),
                64,
            )
            .unwrap();
            pipeline.store_mut().register(book, &path, format);
            pipeline.enqueue(book, ImportState::Registered, Priority::Normal);
            assert_eq!(
                pipeline
                    .run_until_idle(1_000, Gate::foreground(), 8)
                    .indexed,
                1
            );
            let book_dir = pipeline.store().book_dir(book);
            let index_bytes = std::fs::read(book_dir.join("index.bin")).unwrap();
            let direct_index = index_book(direct.book_text(), 64);
            assert_eq!(index_bytes.len(), direct_index.to_bytes().len());
            let loaded_index = perch_index::Index::from_bytes(&index_bytes).unwrap();
            assert_eq!(loaded_index.text(), direct_index.text());
            assert_eq!(loaded_index.chunk_chars(), direct_index.chunk_chars());
            assert_eq!(loaded_index.search("重复"), direct_index.search("重复"));
            let text_bytes = std::fs::read(book_dir.join("text.json")).unwrap();
            let loaded = crate::mapped_book_text_from_slice(&text_bytes).unwrap();
            assert_eq!(loaded, direct);
            drop(pipeline);
            std::fs::remove_file(&path).unwrap();
            let reopened = PathStore::new(&dir).unwrap();
            assert_eq!(reopened.state(book).unwrap().state, ImportState::Ready);
            assert_eq!(
                loaded.locate(0, loaded.char_count()).unwrap(),
                direct.locate(0, direct.char_count()).unwrap()
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn ready_artifact_damage_downgrades_only_the_affected_book() {
        let dir = temp_dir("ready-damage");
        let healthy = BookId::new();
        let broken = BookId::new();
        let healthy_path = write_book(&dir, "healthy.txt", "健康文本".as_bytes());
        let broken_path = write_book(&dir, "broken.txt", "待修复文本".as_bytes());
        build_ready_txt(&dir, healthy, &healthy_path);
        build_ready_txt(&dir, broken, &broken_path);

        std::fs::write(
            dir.join(broken.to_string()).join("index.bin"),
            b"not-an-index",
        )
        .unwrap();

        let reopened = PathStore::new(&dir).unwrap();
        assert_eq!(reopened.state(healthy).unwrap().state, ImportState::Ready);
        let broken_state = reopened.state(broken).unwrap();
        assert_eq!(broken_state.state, ImportState::Indexing);
        assert!(
            broken_state
                .reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with(RECOVERY_INDEX_INVALID))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_refinement_for_ready_book_restarts_from_refine() {
        let dir = temp_dir("missing-refinement");
        let book = BookId::new();
        let path = write_book(&dir, "book.txt", "章节文本".as_bytes());
        build_ready_txt(&dir, book, &path);
        std::fs::remove_file(dir.join(book.to_string()).join("refinement.json")).unwrap();

        let reopened = PathStore::new(&dir).unwrap();
        let state = reopened.state(book).unwrap();
        assert_eq!(state.state, ImportState::Registered);
        assert!(
            state
                .reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with(RECOVERY_REFINEMENT_INVALID))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_body_without_source_map_triggers_stage_three_rebuild() {
        let dir = temp_dir("legacy-map");
        let book = BookId::new();
        let path = write_book(&dir, "book.txt", "需要源映射的文本".as_bytes());
        build_ready_txt(&dir, book, &path);
        let book_dir = dir.join(book.to_string());
        let mapped =
            crate::mapped_book_text_from_slice(&std::fs::read(book_dir.join("text.json")).unwrap())
                .unwrap();
        let legacy = serde_json::to_vec(mapped.book_text()).unwrap();
        std::fs::write(book_dir.join("text.json"), legacy).unwrap();

        let reopened = PathStore::new(&dir).unwrap();
        let state = reopened.state(book).unwrap();
        assert_eq!(state.state, ImportState::Indexing);
        assert!(state.reason.unwrap().contains("sourceMap"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mismatched_index_and_text_trigger_group_rebuild() {
        let dir = temp_dir("mismatch");
        let first = BookId::new();
        let second = BookId::new();
        let first_path = write_book(&dir, "first.txt", "第一本书".as_bytes());
        let second_path = write_book(&dir, "second.txt", "第二本书".as_bytes());
        build_ready_txt(&dir, first, &first_path);
        build_ready_txt(&dir, second, &second_path);
        std::fs::copy(
            dir.join(second.to_string()).join("index.bin"),
            dir.join(first.to_string()).join("index.bin"),
        )
        .unwrap();

        let reopened = PathStore::new(&dir).unwrap();
        let state = reopened.state(first).unwrap();
        assert_eq!(state.state, ImportState::Indexing);
        assert!(state.reason.unwrap().contains("正文不一致"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn downgraded_ready_book_rebuilds_after_source_is_registered() {
        let dir = temp_dir("rebuild");
        let book = BookId::new();
        let path = write_book(&dir, "book.txt", "修复后仍可搜索文本".as_bytes());
        build_ready_txt(&dir, book, &path);
        let index_path = dir.join(book.to_string()).join("index.bin");
        std::fs::write(&index_path, b"broken").unwrap();

        let mut pipeline = pipeline_after_restart(&dir, book, &path);
        assert_eq!(
            pipeline.store().state(book).unwrap().state,
            ImportState::Indexing
        );
        let outcome = pipeline.step(2_000, Gate::foreground());
        assert!(matches!(outcome, StepOutcome::Indexed { book: id, .. } if id == book));
        assert_eq!(
            pipeline.store().state(book).unwrap().state,
            ImportState::Ready
        );
        let loaded = perch_index::Index::load_from(index_path).unwrap();
        assert_eq!(loaded.search("可搜索").len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_source_keeps_old_artifacts_and_reports_index_failure() {
        let dir = temp_dir("missing-source");
        let book = BookId::new();
        let path = write_book(&dir, "book.txt", "旧产物不应被提前删除".as_bytes());
        build_ready_txt(&dir, book, &path);
        let book_dir = dir.join(book.to_string());
        let text_before = std::fs::read(book_dir.join("text.json")).unwrap();
        std::fs::write(book_dir.join("index.bin"), b"broken").unwrap();
        std::fs::remove_file(&path).unwrap();

        let mut pipeline = pipeline_after_restart(&dir, book, &path);
        match pipeline.step(2_000, Gate::foreground()) {
            StepOutcome::Failed { stage, error, .. } => {
                assert_eq!(stage, perch_pipeline::Stage::Index);
                assert!(error.contains("读不了"));
            }
            other => panic!("应当明确报告源文件缺失，实际 {other:?}"),
        }
        assert_eq!(
            std::fs::read(book_dir.join("text.json")).unwrap(),
            text_before
        );
        assert_eq!(
            std::fs::read(book_dir.join("index.bin")).unwrap(),
            b"broken"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 模拟"App 重启"：用同一个产物目录新建一条管线，并按磁盘状态恢复入队。
    fn pipeline_after_restart(dir: &Path, book: BookId, path: &Path) -> Pipeline<PathStore> {
        let mut pipeline = Pipeline::new(
            PathStore::new(dir).unwrap(),
            Policy::default(),
            options(),
            64,
        )
        .unwrap();
        let recovered = pipeline.store().recovered().to_vec();
        assert_eq!(recovered.len(), 1, "重启后该恢复出这本书");
        let record = recovered[0].clone();
        pipeline.store_mut().register(book, path, Format::Txt);
        pipeline.resume(record.book, record.state, Priority::Normal, record.attempts);
        pipeline
    }

    #[test]
    fn failed_book_keeps_attempts_across_restart() {
        // 坏文件（不是 TXT 也不是 EPUB）→ 阶段②失败 → 记下次数 → 重启后次数还在
        let dir = temp_dir("failed");
        let book = BookId::new();
        let path = write_book(&dir, "broken.epub", b"definitely not an epub");

        let mut pipeline = Pipeline::new(
            PathStore::new(&dir).unwrap(),
            Policy::default(),
            options(),
            64,
        )
        .unwrap();
        pipeline.store_mut().register(book, &path, Format::Epub);
        pipeline.enqueue(book, ImportState::Registered, Priority::Normal);

        let outcome = pipeline.step(1_000, Gate::foreground());
        let attempts = match outcome {
            StepOutcome::Failed { attempts, .. } => attempts,
            other => panic!("应当是 Failed，实际 {other:?}"),
        };
        assert!(attempts >= 1);
        pipeline
            .store_mut()
            .record_attempts(book, attempts)
            .unwrap();

        let reopened = PathStore::new(&dir).unwrap();
        assert_eq!(reopened.recovered()[0].attempts, attempts);

        std::fs::remove_dir_all(&dir).ok();
    }
}
