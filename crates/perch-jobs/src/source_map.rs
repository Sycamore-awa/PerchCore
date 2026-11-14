//! Compact mappings from indexed Unicode scalar ranges to the extraction source.
//!
//! Copy runs retain offsets, not source text or a record for every character. TXT
//! newline/collapsed-blank runs cover their complete decoded source interval;
//! EPUB separators are synthetic. A small, in-memory boundary index bounds the
//! work needed to convert a scalar offset to UTF-8 or UTF-16.

use perch_model::Format;
use serde::{Deserialize, Serialize};

use crate::{BOOK_TEXT_VERSION, BookText, BookTextIoError, StoredBookText, read_stored_book_text};

/// Version of the optional `text.json` source map, independent of text/index versions.
pub const SOURCE_MAP_VERSION: u32 = 1;
const BOUNDARY_STRIDE: u64 = 256;

/// A point in the decoded TXT or an EPUB text node, at a Unicode scalar boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum SourceEndpoint {
    /// Offsets in the decoded, BOM-stripped UTF-8 reading text, not encoded file bytes.
    Txt { char_offset: u64, byte_offset: u64 },
    /// A local CFI-style text-node path with separate scalar and UTF-16 offsets.
    Epub {
        href: String,
        node_path: String,
        scalar_offset: u64,
        utf16_offset: u64,
    },
}

/// Exactly two source endpoints, even when the indexed range crosses chapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocation {
    pub start: SourceEndpoint,
    pub end: SourceEndpoint,
    /// Whether any indexed character in the requested range is artificial.
    pub contains_synthetic: bool,
}

/// Stable failures for source mapping. These never prevent loading/searching a book.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceMapError {
    #[error("索引缺少 sourceMap，请从原书重建索引以定位源文本")]
    Missing,
    #[error("sourceMap 版本 {0} 不受支持，请从原书重建索引")]
    UnsupportedVersion(u64),
    #[error("sourceMap 损坏: {0}，请从原书重建索引")]
    Invalid(String),
    #[error("定位区间必须满足 0 ≤ char_start < char_end ≤ 已索引字符数")]
    BadRange,
    #[error("定位区间只包含合成内容，无法定位源文本")]
    Unmappable,
}

impl SourceMapError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Missing => "BOOK_SOURCE_MAP_MISSING",
            Self::UnsupportedVersion(_) => "BOOK_SOURCE_MAP_UNSUPPORTED_VERSION",
            Self::Invalid(_) => "BOOK_SOURCE_MAP_INVALID",
            Self::BadRange => "BOOK_LOCATE_BAD_RANGE",
            Self::Unmappable => "BOOK_SOURCE_UNMAPPABLE",
        }
    }
}

/// Source metadata needed to validate mapping bounds without reopening the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "format",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum SourceDocument {
    Txt {
        decoded_chars: u64,
        decoded_bytes: u64,
    },
    Epub,
}

/// Source of a continuous indexed run. Byte offsets on TXT are decoded UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceSpan {
    /// Scalar and byte deltas are identical to the indexed text's deltas.
    Txt {
        char_start: u64,
        byte_start: u64,
    },
    /// One normalized newline covers a nonempty source interval.
    TxtCollapsed {
        char_start: u64,
        char_end: u64,
        byte_start: u64,
        byte_end: u64,
    },
    /// A complete, entity-decoded text node, beginning at node offset zero.
    Epub {
        href: String,
        node_path: String,
    },
    Synthetic,
}

/// Half-open scalar and UTF-8 byte interval in the indexed text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRun {
    pub start_char: u64,
    pub end_char: u64,
    pub start_byte: u64,
    pub end_byte: u64,
    pub source: SourceSpan,
}

/// Persisted v1 source map. Runs cover the indexed text in order without gaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap {
    pub version: u32,
    pub indexed_chars: u64,
    pub document: SourceDocument,
    pub runs: Vec<SourceRun>,
}

// The persisted map uses lengths and a single TXT origin. Consecutive starts
// are implied, so they cannot drift and do not repeat four offsets per run.
// t=[copy scalars,copy bytes], n=[source scalars,source bytes],
// e=[node scalars,node bytes,href,path], s=one synthetic newline.
#[derive(Deserialize)]
enum WireRun {
    #[serde(rename = "t")]
    Txt(u64, u64),
    #[serde(rename = "n")]
    TxtCollapsed(u64, u64),
    #[serde(rename = "e")]
    Epub(u64, u64, String, String),
    #[serde(rename = "s")]
    Synthetic,
}

#[derive(Serialize)]
enum WireRunRef<'a> {
    #[serde(rename = "t")]
    Txt(u64, u64),
    #[serde(rename = "n")]
    TxtCollapsed(u64, u64),
    #[serde(rename = "e")]
    Epub(u64, u64, &'a str, &'a str),
    #[serde(rename = "s")]
    Synthetic,
}

struct WireRunsRef<'a>(&'a [SourceRun]);

impl Serialize for WireRunsRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for run in self.0 {
            let invalid = || <S::Error as serde::ser::Error>::custom("映射区间长度无效");
            let chars = run
                .end_char
                .checked_sub(run.start_char)
                .filter(|count| *count > 0)
                .ok_or_else(invalid)?;
            let bytes = run
                .end_byte
                .checked_sub(run.start_byte)
                .filter(|count| *count > 0)
                .ok_or_else(invalid)?;
            let wire = match &run.source {
                SourceSpan::Txt { .. } => WireRunRef::Txt(chars, bytes),
                SourceSpan::TxtCollapsed {
                    char_start,
                    char_end,
                    byte_start,
                    byte_end,
                } => WireRunRef::TxtCollapsed(
                    char_end
                        .checked_sub(*char_start)
                        .filter(|count| *count > 0)
                        .ok_or_else(invalid)?,
                    byte_end
                        .checked_sub(*byte_start)
                        .filter(|count| *count > 0)
                        .ok_or_else(invalid)?,
                ),
                SourceSpan::Epub { href, node_path } => {
                    WireRunRef::Epub(chars, bytes, href, node_path)
                }
                SourceSpan::Synthetic => WireRunRef::Synthetic,
            };
            sequence.serialize_element(&wire)?;
        }
        sequence.end()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSourceMapRef<'a> {
    version: u32,
    indexed_chars: u64,
    #[serde(flatten)]
    document: &'a SourceDocument,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<[u64; 2]>,
    runs: WireRunsRef<'a>,
}

impl Serialize for SourceMap {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let origin = self
            .runs
            .first()
            .and_then(|run| match run.source {
                SourceSpan::Txt {
                    char_start,
                    byte_start,
                }
                | SourceSpan::TxtCollapsed {
                    char_start,
                    byte_start,
                    ..
                } => Some([char_start, byte_start]),
                _ => None,
            })
            .or_else(|| matches!(self.document, SourceDocument::Txt { .. }).then_some([0, 0]));
        WireSourceMapRef {
            version: self.version,
            indexed_chars: self.indexed_chars,
            document: &self.document,
            origin,
            runs: WireRunsRef(&self.runs),
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSourceMap {
    version: u32,
    indexed_chars: u64,
    #[serde(flatten)]
    document: SourceDocument,
    origin: Option<[u64; 2]>,
    runs: Vec<WireRun>,
}

impl<'de> Deserialize<'de> for SourceMap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = WireSourceMap::deserialize(deserializer)?;
        let mut source_position = match (&wire.document, wire.origin) {
            (SourceDocument::Txt { .. }, Some(origin)) => origin,
            (SourceDocument::Epub, None) => [0, 0],
            _ => return Err(serde::de::Error::custom("源格式与 origin 不一致")),
        };
        let mut start_char = 0u64;
        let mut start_byte = 0u64;
        let mut runs = Vec::with_capacity(wire.runs.len());
        let add = |left: u64, right: u64| {
            left.checked_add(right)
                .ok_or_else(|| <D::Error as serde::de::Error>::custom("映射偏移溢出"))
        };
        for run in wire.runs {
            let (chars, bytes, source) = match run {
                WireRun::Txt(chars, bytes) => {
                    let source = SourceSpan::Txt {
                        char_start: source_position[0],
                        byte_start: source_position[1],
                    };
                    source_position = [
                        add(source_position[0], chars)?,
                        add(source_position[1], bytes)?,
                    ];
                    (chars, bytes, source)
                }
                WireRun::TxtCollapsed(chars, bytes) => {
                    let end = [
                        add(source_position[0], chars)?,
                        add(source_position[1], bytes)?,
                    ];
                    let source = SourceSpan::TxtCollapsed {
                        char_start: source_position[0],
                        char_end: end[0],
                        byte_start: source_position[1],
                        byte_end: end[1],
                    };
                    source_position = end;
                    (1, 1, source)
                }
                WireRun::Epub(chars, bytes, href, node_path) => {
                    (chars, bytes, SourceSpan::Epub { href, node_path })
                }
                WireRun::Synthetic => (1, 1, SourceSpan::Synthetic),
            };
            let end_char = add(start_char, chars)?;
            let end_byte = add(start_byte, bytes)?;
            runs.push(SourceRun {
                start_char,
                end_char,
                start_byte,
                end_byte,
                source,
            });
            start_char = end_char;
            start_byte = end_byte;
        }
        Ok(Self {
            version: wire.version,
            indexed_chars: wire.indexed_chars,
            document: wire.document,
            runs,
        })
    }
}

impl SourceMap {
    pub(crate) fn new(document: SourceDocument) -> Self {
        Self {
            version: SOURCE_MAP_VERSION,
            indexed_chars: 0,
            document,
            runs: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, text: &str, chars: u64, source: SourceSpan) {
        if chars == 0 {
            return;
        }
        let start_byte = self.runs.last().map_or(0, |run| run.end_byte);
        let bytes = text.len() as u64;
        // Adjacent TXT copy runs (including ordinary LF boundaries) coalesce.
        if let SourceSpan::Txt {
            char_start,
            byte_start,
        } = &source
            && let Some(previous) = self.runs.last_mut()
            && let SourceSpan::Txt {
                char_start: previous_char,
                byte_start: previous_byte,
            } = previous.source
            && previous_char + (previous.end_char - previous.start_char) == *char_start
            && previous_byte + (previous.end_byte - previous.start_byte) == *byte_start
        {
            previous.end_char += chars;
            previous.end_byte += bytes;
            self.indexed_chars += chars;
            return;
        }
        self.runs.push(SourceRun {
            start_char: self.indexed_chars,
            end_char: self.indexed_chars + chars,
            start_byte,
            end_byte: start_byte + bytes,
            source,
        });
        self.indexed_chars += chars;
    }

    fn validate(&self, book: &BookText, positions: &TextPositions) -> Result<(), SourceMapError> {
        let invalid = |reason: &str| SourceMapError::Invalid(reason.to_owned());
        if self.version != SOURCE_MAP_VERSION {
            return Err(SourceMapError::UnsupportedVersion(u64::from(self.version)));
        }
        if self.indexed_chars != positions.chars {
            return Err(invalid("indexedChars 与正文不一致"));
        }
        match (&self.document, book.format) {
            (
                SourceDocument::Txt {
                    decoded_chars,
                    decoded_bytes,
                },
                Format::Txt,
            ) if decoded_bytes >= decoded_chars
                && *decoded_bytes <= decoded_chars.saturating_mul(4) => {}
            (SourceDocument::Epub, Format::Epub) => {}
            _ => return Err(invalid("源格式或解码长度无效")),
        }
        let mut next_char = 0;
        let mut next_byte = 0;
        let mut previous_txt_end = None;
        for run in &self.runs {
            if run.start_char != next_char
                || run.start_byte != next_byte
                || run.start_char >= run.end_char
                || run.end_char > positions.chars
                || run.start_byte >= run.end_byte
            {
                return Err(invalid("区间必须有序、连续且位于正文内"));
            }
            let start = positions
                .at(&book.text, run.start_char)
                .ok_or_else(|| invalid("起点无效"))?;
            let end = positions
                .at(&book.text, run.end_char)
                .ok_or_else(|| invalid("终点无效"))?;
            if start.byte as u64 != run.start_byte || end.byte as u64 != run.end_byte {
                return Err(invalid("scalar 与 UTF-8 边界不一致"));
            }
            let run_text = book
                .text
                .get(start.byte..end.byte)
                .ok_or_else(|| invalid("UTF-8 区间无效"))?;
            let txt_bounds = match (&self.document, &run.source) {
                (
                    SourceDocument::Txt { .. },
                    SourceSpan::Txt {
                        char_start,
                        byte_start,
                    },
                ) => {
                    let char_end = char_start
                        .checked_add(run.end_char - run.start_char)
                        .ok_or_else(|| invalid("源字符偏移溢出"))?;
                    let byte_end = byte_start
                        .checked_add(run.end_byte - run.start_byte)
                        .ok_or_else(|| invalid("源字节偏移溢出"))?;
                    Some((*char_start, char_end, *byte_start, byte_end))
                }
                (
                    SourceDocument::Txt { .. },
                    SourceSpan::TxtCollapsed {
                        char_start,
                        char_end,
                        byte_start,
                        byte_end,
                    },
                ) => {
                    if run.end_char - run.start_char != 1
                        || run_text != "\n"
                        || char_start >= char_end
                        || byte_start >= byte_end
                        || byte_end - byte_start < char_end - char_start
                        || byte_end - byte_start > (char_end - char_start).saturating_mul(4)
                    {
                        return Err(invalid("折叠换行的源区间无效"));
                    }
                    Some((*char_start, *char_end, *byte_start, *byte_end))
                }
                (SourceDocument::Epub, SourceSpan::Epub { href, node_path }) => {
                    let path = perch_epub::CfiPath::parse(node_path)
                        .map_err(|_| invalid("文本节点路径无法解析"))?;
                    if !path.indirection.is_empty()
                        || path.text_offset.is_some()
                        || path.local.is_empty()
                        || path
                            .local
                            .last()
                            .is_none_or(|step| step.index.is_multiple_of(2))
                        || path.local[..path.local.len() - 1]
                            .iter()
                            .any(|step| !step.index.is_multiple_of(2))
                        || !href.starts_with('/')
                        || book.span_at(run.start_char).is_none_or(|chapter| {
                            chapter.href != *href || run.end_char > chapter.end_char
                        })
                    {
                        return Err(invalid("文本节点路径或章节 href 无效"));
                    }
                    None
                }
                (SourceDocument::Epub, SourceSpan::Synthetic) if run_text == "\n" => None,
                _ => return Err(invalid("区间源类型不符合书籍格式")),
            };
            if let Some((char_start, char_end, byte_start, byte_end)) = txt_bounds {
                let SourceDocument::Txt {
                    decoded_chars,
                    decoded_bytes,
                } = self.document
                else {
                    return Err(invalid("TXT 源格式无效"));
                };
                if char_end > decoded_chars
                    || byte_end > decoded_bytes
                    || byte_start < char_start
                    || byte_start > char_start.saturating_mul(4)
                    || previous_txt_end.is_some_and(|previous| previous != (char_start, byte_start))
                {
                    return Err(invalid("TXT 源区间不连续或越界"));
                }
                previous_txt_end = Some((char_end, byte_end));
            }
            next_char = run.end_char;
            next_byte = run.end_byte;
        }
        if next_char != positions.chars || next_byte != book.text.len() as u64 {
            return Err(invalid("映射未覆盖完整索引正文"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Boundary {
    byte: usize,
    utf16: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextPositions {
    samples: Vec<Boundary>,
    chars: u64,
    end: Boundary,
}

impl TextPositions {
    fn build(text: &str) -> Self {
        let mut samples = Vec::new();
        let mut chars = 0u64;
        let mut utf16 = 0u64;
        for (byte, scalar) in text.char_indices() {
            if chars.is_multiple_of(BOUNDARY_STRIDE) {
                samples.push(Boundary { byte, utf16 });
            }
            chars += 1;
            utf16 += scalar.len_utf16() as u64;
        }
        Self {
            samples,
            chars,
            end: Boundary {
                byte: text.len(),
                utf16,
            },
        }
    }

    fn at(&self, text: &str, offset: u64) -> Option<Boundary> {
        if offset > self.chars {
            return None;
        }
        if offset == self.chars {
            return Some(self.end);
        }
        let sample = usize::try_from(offset / BOUNDARY_STRIDE).ok()?;
        let mut boundary = *self.samples.get(sample)?;
        let remaining = usize::try_from(offset % BOUNDARY_STRIDE).ok()?;
        for scalar in text.get(boundary.byte..)?.chars().take(remaining) {
            boundary.byte += scalar.len_utf8();
            boundary.utf16 += scalar.len_utf16() as u64;
        }
        Some(boundary)
    }
}

/// Body wrapper that preserves `BookText`'s public fields and old function signatures.
/// Mapping availability is checked independently of loading/searching its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedBookText {
    text: BookText,
    source_map: Result<SourceMap, SourceMapError>,
    positions: Option<TextPositions>,
    char_count: u64,
}

impl From<BookText> for MappedBookText {
    fn from(text: BookText) -> Self {
        Self::with_source_map(text, Err(SourceMapError::Missing))
    }
}

impl MappedBookText {
    pub(crate) fn with_source_map(
        text: BookText,
        source_map: Result<SourceMap, SourceMapError>,
    ) -> Self {
        match source_map {
            Ok(map) => {
                let positions = TextPositions::build(&text.text);
                let char_count = positions.chars;
                match map.validate(&text, &positions) {
                    Ok(()) => Self {
                        text,
                        source_map: Ok(map),
                        positions: Some(positions),
                        char_count,
                    },
                    Err(error) => Self {
                        text,
                        source_map: Err(error),
                        positions: None,
                        char_count,
                    },
                }
            }
            Err(error) => {
                let char_count = text.char_count();
                Self {
                    text,
                    source_map: Err(error),
                    positions: None,
                    char_count,
                }
            }
        }
    }

    #[must_use]
    pub const fn book_text(&self) -> &BookText {
        &self.text
    }

    #[must_use]
    pub fn into_book_text(self) -> BookText {
        self.text
    }

    #[must_use]
    pub const fn char_count(&self) -> u64 {
        self.char_count
    }

    /// Obtain the validated mapping, or the precise reason it is unavailable.
    ///
    /// # Errors
    /// Missing, future-version, and malformed maps have distinct stable errors.
    pub fn source_map(&self) -> Result<&SourceMap, SourceMapError> {
        self.source_map.as_ref().map_err(Clone::clone)
    }

    /// Map a nonempty indexed scalar range. Start moves right and end moves left
    /// within the requested range to skip synthetic separators.
    ///
    /// # Errors
    /// Invalid ranges, unavailable maps, and entirely synthetic ranges are distinct.
    pub fn locate(&self, char_start: u64, char_end: u64) -> Result<SourceLocation, SourceMapError> {
        if char_start >= char_end || char_end > self.char_count {
            return Err(SourceMapError::BadRange);
        }
        let map = self.source_map()?;
        let positions = self
            .positions
            .as_ref()
            .ok_or_else(|| SourceMapError::Invalid("缺少边界索引".to_owned()))?;
        let first = map.runs.partition_point(|run| run.end_char <= char_start);
        let after = map.runs.partition_point(|run| run.start_char < char_end);
        let runs = map
            .runs
            .get(first..after)
            .ok_or_else(|| SourceMapError::Invalid("区间查找失败".to_owned()))?;
        let synthetic = |run: &&SourceRun| matches!(run.source, SourceSpan::Synthetic);
        let start_run = runs
            .iter()
            .find(|run| !synthetic(run))
            .ok_or(SourceMapError::Unmappable)?;
        let end_run = runs
            .iter()
            .rfind(|run| !synthetic(run))
            .ok_or(SourceMapError::Unmappable)?;
        Ok(SourceLocation {
            start: self.endpoint(positions, start_run, char_start.max(start_run.start_char))?,
            end: self.endpoint(positions, end_run, char_end.min(end_run.end_char))?,
            contains_synthetic: runs
                .iter()
                .any(|run| matches!(run.source, SourceSpan::Synthetic)),
        })
    }

    fn endpoint(
        &self,
        positions: &TextPositions,
        run: &SourceRun,
        offset: u64,
    ) -> Result<SourceEndpoint, SourceMapError> {
        let invalid = || SourceMapError::Invalid("端点偏移无效".to_owned());
        match &run.source {
            SourceSpan::Txt {
                char_start,
                byte_start,
            } => {
                let boundary = positions.at(&self.text.text, offset).ok_or_else(invalid)?;
                Ok(SourceEndpoint::Txt {
                    char_offset: char_start + (offset - run.start_char),
                    byte_offset: byte_start + (boundary.byte as u64 - run.start_byte),
                })
            }
            SourceSpan::TxtCollapsed {
                char_start,
                char_end,
                byte_start,
                byte_end,
            } => {
                let end = offset == run.end_char;
                Ok(SourceEndpoint::Txt {
                    char_offset: if end { *char_end } else { *char_start },
                    byte_offset: if end { *byte_end } else { *byte_start },
                })
            }
            SourceSpan::Epub { href, node_path } => {
                let start = positions
                    .at(&self.text.text, run.start_char)
                    .ok_or_else(invalid)?;
                let boundary = positions.at(&self.text.text, offset).ok_or_else(invalid)?;
                Ok(SourceEndpoint::Epub {
                    href: href.clone(),
                    node_path: node_path.clone(),
                    scalar_offset: offset - run.start_char,
                    utf16_offset: boundary.utf16 - start.utf16,
                })
            }
            SourceSpan::Synthetic => Err(SourceMapError::Unmappable),
        }
    }
}

/// Map an indexed hit range without changing the legacy `locate_hit` Locator.
///
/// # Errors
/// See [`MappedBookText::locate`].
pub fn locate_source_range(
    book: &MappedBookText,
    char_start: u64,
    char_end: u64,
) -> Result<SourceLocation, SourceMapError> {
    book.locate(char_start, char_end)
}

#[derive(Default)]
pub(crate) struct RawSourceMap(pub Option<serde_json::Value>);

impl<'de> Deserialize<'de> for RawSourceMap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Explicit null is malformed; only an absent field means a legacy map.
        serde_json::Value::deserialize(deserializer).map(|value| Self(Some(value)))
    }
}

/// Read legacy v0/current v1 bodies, keeping mapping errors local to `locate`.
///
/// # Errors
/// Only body JSON or body-version errors fail loading; see [`BookTextIoError`].
pub fn mapped_book_text_from_slice(bytes: &[u8]) -> Result<MappedBookText, BookTextIoError> {
    let StoredBookText {
        text, source_map, ..
    } = read_stored_book_text(bytes)?;
    let map = match source_map.0 {
        None => Err(SourceMapError::Missing),
        Some(value) => match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(version) if version != u64::from(SOURCE_MAP_VERSION) => {
                Err(SourceMapError::UnsupportedVersion(version))
            }
            Some(_) => serde_json::from_value(value)
                .map_err(|error| SourceMapError::Invalid(error.to_string())),
            None => Err(SourceMapError::Invalid("缺少有效映射版本".to_owned())),
        },
    };
    Ok(MappedBookText::with_source_map(text, map))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredMappedBookTextRef<'a> {
    version: u32,
    #[serde(flatten)]
    text: &'a BookText,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_map: Option<&'a SourceMap>,
}

/// Write a v1 body with its validated optional v1 map.
///
/// # Errors
/// Unavailable invalid/future maps cannot be saved; missing legacy maps stay absent.
/// Serialization failures return [`serde_json::Error`].
pub fn mapped_book_text_to_vec(text: &MappedBookText) -> Result<Vec<u8>, serde_json::Error> {
    let source_map = match &text.source_map {
        Ok(map) => Some(map),
        Err(SourceMapError::Missing) => None,
        Err(error) => {
            return Err(<serde_json::Error as serde::ser::Error>::custom(
                error.to_string(),
            ));
        }
    };
    serde_json::to_vec(&StoredMappedBookTextRef {
        version: BOOK_TEXT_VERSION,
        text: &text.text,
        source_map,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::{
        ExtractOptions, extract_epub_text, extract_epub_text_with_source_map,
        extract_txt_with_source_map, index_book,
    };
    use perch_epub::{CfiPath, EpubArchive, XhtmlIndex};
    use perch_txt::TxtDocument;
    use serde_json::{Value, json};

    fn txt(raw: &str) -> MappedBookText {
        extract_txt_with_source_map(raw.as_bytes(), ExtractOptions::default()).unwrap()
    }

    fn txt_slice<'a>(document: &'a TxtDocument, location: &SourceLocation) -> &'a str {
        let SourceEndpoint::Txt {
            char_offset: start,
            byte_offset: start_byte,
        } = location.start
        else {
            panic!("TXT start expected")
        };
        let SourceEndpoint::Txt {
            char_offset: end,
            byte_offset: end_byte,
        } = location.end
        else {
            panic!("TXT end expected")
        };
        assert_eq!(document.char_to_byte(start), Some(start_byte as usize));
        assert_eq!(document.char_to_byte(end), Some(end_byte as usize));
        let scalar_slice = document.text_range(start..end).unwrap();
        assert_eq!(
            scalar_slice,
            &document.text()[start_byte as usize..end_byte as usize]
        );
        scalar_slice
    }

    #[test]
    fn txt_crlf_blank_folding_returns_original_decoded_offset_nine() {
        let raw = "\r\n甲\r\n\r\n\r\n乙😀";
        let book = txt(raw);
        let document = TxtDocument::from_bytes(raw.as_bytes());
        assert_eq!(book.book_text().text, "甲\n\n乙😀");
        let hit = index_book(book.book_text(), 2).search("乙").remove(0);
        assert_eq!(hit.char_start, 3);
        let location = locate_source_range(&book, hit.char_start, hit.char_end).unwrap();
        assert_eq!(
            location.start,
            SourceEndpoint::Txt {
                char_offset: 9,
                byte_offset: 11
            }
        );
        assert_eq!(
            location.end,
            SourceEndpoint::Txt {
                char_offset: 10,
                byte_offset: 14
            }
        );
        assert_eq!(txt_slice(&document, &location), "乙");
        assert!(!location.contains_synthetic);
        assert_eq!(txt_slice(&document, &book.locate(1, 2).unwrap()), "\r\n");
        assert_eq!(
            txt_slice(&document, &book.locate(2, 3).unwrap()),
            "\r\n\r\n"
        );
        assert_eq!(txt_slice(&document, &book.locate(4, 5).unwrap()), "😀");
    }

    #[test]
    fn txt_newline_variants_inline_spaces_and_combining_scalars_round_trip() {
        for (raw, expected) in [
            ("\r甲\r乙\n丙\r\n丁\n", "甲\n乙\n丙\n丁"),
            (
                " \r\n\t\r甲😀e\u{301} 中 English\r\n \t\r\n\r\n乙\r\n\n ",
                "甲😀e\u{301} 中 English\n\n乙",
            ),
            ("\n\n甲\n\n乙\n\n", "甲\n\n乙"),
            ("甲\n\n\n\n\n乙", "甲\n\n乙"),
            (" 甲 \n \u{a0}\t\n乙 ", " 甲 \n\n乙 "),
        ] {
            let book = txt(raw);
            assert_eq!(book.book_text().text, expected);
            let document = TxtDocument::from_bytes(raw.as_bytes());
            for (offset, scalar) in expected.chars().enumerate() {
                let location = book.locate(offset as u64, offset as u64 + 1).unwrap();
                let source = txt_slice(&document, &location);
                if scalar == '\n' {
                    assert!(
                        source.chars().all(char::is_whitespace),
                        "{raw:?}: {source:?}"
                    );
                } else {
                    assert_eq!(source, scalar.to_string(), "{raw:?} at {offset}");
                }
            }
        }
    }

    // An independent specification of the old normalization, used with varied input.
    fn normalized_reference(raw: &str) -> String {
        let converted = raw.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<_> = converted.split('\n').collect();
        let mut result = String::new();
        let mut previous = None;
        for (index, line) in lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
        {
            if let Some(prior) = previous {
                result.push('\n');
                if index > prior + 1 {
                    result.push('\n');
                }
            }
            result.push_str(line);
            previous = Some(index);
        }
        result
    }

    #[test]
    fn varied_txt_normalization_keeps_every_scalar_mappable_and_coherent() {
        let lines = [
            "甲😀é",
            "",
            " \t",
            " English 中 ",
            "乙",
            "\u{a0}",
            "重复重复",
        ];
        let endings = ["\n", "\r", "\r\n"];
        let mut state = 17u64;
        for _ in 0..200 {
            let mut raw = String::new();
            for _ in 0..12 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                raw.push_str(lines[(state >> 32) as usize % lines.len()]);
                raw.push_str(endings[(state >> 24) as usize % endings.len()]);
            }
            let expected = normalized_reference(&raw);
            if expected.is_empty() {
                continue;
            }
            let book = txt(&raw);
            assert_eq!(book.book_text().text, expected);
            let document = TxtDocument::from_bytes(raw.as_bytes());
            assert_eq!(
                normalized_reference(txt_slice(
                    &document,
                    &book.locate(0, book.char_count()).unwrap()
                )),
                expected
            );
            for (offset, scalar) in expected.chars().enumerate() {
                let source = txt_slice(
                    &document,
                    &book.locate(offset as u64, offset as u64 + 1).unwrap(),
                );
                if scalar == '\n' {
                    assert!(source.chars().all(char::is_whitespace));
                } else {
                    assert_eq!(source, scalar.to_string());
                }
            }
        }
    }

    #[test]
    fn txt_gbk_big5_and_bom_offsets_use_the_reading_decode() {
        for encoding in [encoding_rs::GBK, encoding_rs::BIG5] {
            let raw = "\r\n中文測試\r\n\r\n\r\n中文內容 English";
            let (bytes, _, errors) = encoding.encode(raw);
            assert!(!errors);
            let document = TxtDocument::from_bytes(&bytes);
            let book = extract_txt_with_source_map(&bytes, ExtractOptions::default()).unwrap();
            assert_eq!(book.book_text().text, normalized_reference(document.text()));
            for hit in index_book(book.book_text(), 3).search("中文") {
                assert_eq!(
                    txt_slice(
                        &document,
                        &book.locate(hit.char_start, hit.char_end).unwrap()
                    ),
                    "中文"
                );
            }
        }
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend_from_slice("\n甲😀".as_bytes());
        let book = extract_txt_with_source_map(&bytes, ExtractOptions::default()).unwrap();
        assert_eq!(
            book.locate(0, 1).unwrap().start,
            SourceEndpoint::Txt {
                char_offset: 1,
                byte_offset: 1
            }
        );
    }

    #[test]
    fn long_mixed_txt_uses_one_copy_run_and_sample_boundaries_are_exact() {
        let raw = format!("{}终点", "甲😀aéé".repeat(2000));
        let book = txt(&raw);
        assert_eq!(
            book.source_map().unwrap().runs.len(),
            1,
            "copy mapping is not per-character"
        );
        let document = TxtDocument::from_bytes(raw.as_bytes());
        for offset in [
            0,
            255,
            256,
            257,
            511,
            512,
            4095,
            4096,
            10001,
            book.char_count() - 1,
        ] {
            let expected = raw.chars().nth(offset as usize).unwrap().to_string();
            assert_eq!(
                txt_slice(&document, &book.locate(offset, offset + 1).unwrap()),
                expected
            );
        }
    }

    #[test]
    fn txt_truncation_keeps_partial_copy_and_complete_collapsed_boundaries() {
        let raw = "\r\n甲😀\r\n\r\n\r\n乙é";
        let full = txt(raw);
        let document = TxtDocument::from_bytes(raw.as_bytes());
        for limit in 1..=full.char_count() + 1 {
            let book =
                extract_txt_with_source_map(raw.as_bytes(), ExtractOptions { max_chars: limit })
                    .unwrap();
            assert_eq!(
                book.book_text().text,
                full.book_text()
                    .text
                    .chars()
                    .take(limit as usize)
                    .collect::<String>()
            );
            assert_eq!(book.book_text().truncated, limit < full.char_count());
            let map = book.source_map().unwrap();
            assert_eq!(map.indexed_chars, book.char_count());
            assert_eq!(
                map.runs.last().unwrap().end_byte,
                book.book_text().text.len() as u64
            );
            let last = book.char_count() - 1;
            txt_slice(&document, &book.locate(last, last + 1).unwrap());
            assert_eq!(
                book.locate(0, book.char_count() + 1),
                Err(SourceMapError::BadRange)
            );
        }
        assert!(
            extract_txt_with_source_map(raw.as_bytes(), ExtractOptions { max_chars: 0 }).is_err()
        );
        assert_eq!(
            txt_slice(&document, &full.locate(3, 4).unwrap()),
            "\r\n\r\n"
        );
    }

    fn epub() -> (Vec<u8>, MappedBookText) {
        let bytes = crate::tests::build_epub(
            &["重复&amp;😀<![CDATA[<é>]]><b>重复</b>尾", "重复&amp;😀"],
            false,
            false,
        );
        let book = extract_epub_text_with_source_map(&bytes, ExtractOptions::default()).unwrap();
        (bytes, book)
    }

    fn assert_epub_point(
        archive: &mut EpubArchive<std::io::Cursor<Vec<u8>>>,
        point: &SourceEndpoint,
    ) -> String {
        let SourceEndpoint::Epub {
            href,
            node_path,
            scalar_offset,
            utf16_offset,
        } = point
        else {
            panic!("EPUB point expected")
        };
        let bytes = archive.read(href.trim_start_matches('/')).unwrap();
        let index = XhtmlIndex::build(&bytes).unwrap();
        let path = CfiPath::parse(node_path).unwrap();
        let node = index.resolve(&path).unwrap();
        assert!(path.text_offset.is_none());
        assert!(*scalar_offset <= node.text.chars().count() as u64);
        let prefix: String = node.text.chars().take(*scalar_offset as usize).collect();
        assert_eq!(*utf16_offset, prefix.encode_utf16().count() as u64);
        node.text.clone()
    }

    #[test]
    fn epub_entities_cdata_repeated_text_and_utf16_resolve_to_source_nodes() {
        let (bytes, book) = epub();
        let mut archive = EpubArchive::from_bytes(bytes.clone()).unwrap();
        assert_eq!(
            book.book_text(),
            &extract_epub_text(&bytes, ExtractOptions::default()).unwrap()
        );
        for run in &book.source_map().unwrap().runs {
            if matches!(run.source, SourceSpan::Synthetic) {
                continue;
            }
            let whole = book.locate(run.start_char, run.end_char).unwrap();
            assert!(!whole.contains_synthetic);
            let node = assert_epub_point(&mut archive, &whole.start);
            assert_eq!(node, assert_epub_point(&mut archive, &whole.end));
            assert_eq!(
                node,
                &book.book_text().text[run.start_byte as usize..run.end_byte as usize]
            );
            for offset in run.start_char..run.end_char {
                let location = book.locate(offset, offset + 1).unwrap();
                assert_epub_point(&mut archive, &location.start);
                assert_epub_point(&mut archive, &location.end);
            }
        }
        let run = book
            .source_map()
            .unwrap()
            .runs
            .iter()
            .find(|run| {
                book.book_text().text[run.start_byte as usize..run.end_byte as usize]
                    .starts_with("重复&😀<")
            })
            .unwrap();
        let emoji = book.locate(run.start_char + 3, run.start_char + 4).unwrap();
        let SourceEndpoint::Epub {
            scalar_offset,
            utf16_offset,
            ..
        } = emoji.end
        else {
            panic!("EPUB point")
        };
        assert_eq!(
            (scalar_offset, utf16_offset),
            (4, 5),
            "UTF-16 differs after the emoji"
        );
        let hits = index_book(book.book_text(), 2).search("重复");
        assert_eq!(hits.len(), 3);
        let starts: Vec<_> = hits
            .iter()
            .map(|hit| book.locate(hit.char_start, hit.char_end).unwrap().start)
            .collect();
        assert_ne!(starts[0], starts[1]);
        assert_ne!(starts[0], starts[2]);
    }

    #[test]
    fn epub_synthetic_boundaries_snap_inward_and_cross_chapter_hrefs_survive() {
        let (bytes, book) = epub();
        let mut archive = EpubArchive::from_bytes(bytes).unwrap();
        let runs = &book.source_map().unwrap().runs;
        for (index, run) in runs
            .iter()
            .enumerate()
            .filter(|(_, run)| matches!(run.source, SourceSpan::Synthetic))
        {
            assert_eq!(
                book.locate(run.start_char, run.end_char),
                Err(SourceMapError::Unmappable)
            );
            let previous = &runs[index - 1];
            let left = book.locate(previous.start_char, run.end_char).unwrap();
            assert!(left.contains_synthetic);
            assert_eq!(
                left.end,
                book.locate(previous.start_char, previous.end_char)
                    .unwrap()
                    .end
            );
            if let Some(next) = runs.get(index + 1) {
                let right = book.locate(run.start_char, next.end_char).unwrap();
                assert!(right.contains_synthetic);
                assert_eq!(
                    right.start,
                    book.locate(next.start_char, next.end_char).unwrap().start
                );
            }
        }
        let whole = book.locate(0, book.char_count()).unwrap();
        assert!(whole.contains_synthetic);
        let SourceEndpoint::Epub {
            href: start_href, ..
        } = &whole.start
        else {
            panic!("EPUB")
        };
        let SourceEndpoint::Epub { href: end_href, .. } = &whole.end else {
            panic!("EPUB")
        };
        assert_eq!(start_href, "/OEBPS/chap0.xhtml");
        assert_eq!(end_href, "/OEBPS/chap1.xhtml");
        assert_epub_point(&mut archive, &whole.start);
        assert_epub_point(&mut archive, &whole.end);
    }

    #[test]
    fn epub_keeps_existing_whole_node_truncation_and_separator_overrun() {
        let (bytes, _) = epub();
        for max_chars in 0..45 {
            let legacy = extract_epub_text(&bytes, ExtractOptions { max_chars });
            let mapped = extract_epub_text_with_source_map(&bytes, ExtractOptions { max_chars });
            match (legacy, mapped) {
                (Ok(legacy), Ok(mapped)) => {
                    assert_eq!(&legacy, mapped.book_text());
                    assert!(mapped.char_count() <= max_chars + 1);
                    mapped.source_map().unwrap();
                    mapped.locate(0, mapped.char_count()).unwrap();
                }
                (Err(legacy), Err(mapped)) => assert_eq!(legacy.code(), mapped.code()),
                _ => panic!("truncation behavior differs"),
            }
        }
    }

    #[test]
    fn persisted_maps_round_trip_for_txt_and_epub_and_old_readers_ignore_them() {
        for book in [txt("\r\n甲😀\r\n\r\n\r\n乙é"), epub().1] {
            let bytes = mapped_book_text_to_vec(&book).unwrap();
            let stored: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(stored["version"], 1);
            assert_eq!(stored["sourceMap"]["version"], 1);
            assert_eq!(
                crate::book_text_from_slice(&bytes).unwrap(),
                *book.book_text()
            );
            assert_eq!(
                serde_json::from_slice::<BookText>(&bytes).unwrap(),
                *book.book_text()
            );
            let loaded = mapped_book_text_from_slice(&bytes).unwrap();
            assert_eq!(loaded, book);
            for start in 0..book.char_count() {
                for end in start + 1..=book.char_count() {
                    assert_eq!(loaded.locate(start, end), book.locate(start, end));
                }
            }
        }
    }

    #[test]
    fn missing_future_or_corrupt_maps_leave_body_searchable_and_errors_distinct() {
        let book = txt("甲😀乙");
        let original: Value =
            serde_json::from_slice(&mapped_book_text_to_vec(&book).unwrap()).unwrap();
        let mut cases = Vec::new();
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove("sourceMap");
        cases.push((missing, "BOOK_SOURCE_MAP_MISSING"));
        for version in [0, 2, u64::MAX] {
            let mut value = original.clone();
            value["sourceMap"]["version"] = json!(version);
            cases.push((value, "BOOK_SOURCE_MAP_UNSUPPORTED_VERSION"));
        }
        for map in [
            Value::Null,
            json!([]),
            json!({"version": "1"}),
            json!({"version": -1}),
            json!({"version": 1}),
        ] {
            let mut value = original.clone();
            value["sourceMap"] = map;
            cases.push((value, "BOOK_SOURCE_MAP_INVALID"));
        }
        for (field, bad) in [
            ("indexedChars", json!(2)),
            ("runs", json!([])),
            ("decodedChars", json!(1)),
        ] {
            let mut value = original.clone();
            value["sourceMap"][field] = bad;
            cases.push((value, "BOOK_SOURCE_MAP_INVALID"));
        }
        for bad in [
            json!({"t": [0, 0]}),
            json!({"t": [4, 10]}),
            json!({"t": [3, 7]}),
            json!({"n": [0, 0]}),
            json!({"z": []}),
            json!({"t": [3]}),
            json!(["t", 3, 10]),
        ] {
            let mut value = original.clone();
            value["sourceMap"]["runs"][0] = bad;
            cases.push((value, "BOOK_SOURCE_MAP_INVALID"));
        }
        for origin in [
            json!([u64::MAX, 0]),
            json!([0, u64::MAX]),
            json!([1, 1]),
            json!([0, 0, 0]),
            Value::Null,
        ] {
            let mut value = original.clone();
            value["sourceMap"]["origin"] = origin;
            cases.push((value, "BOOK_SOURCE_MAP_INVALID"));
        }
        for (value, expected) in cases {
            let bytes = serde_json::to_vec(&value).unwrap();
            let loaded = mapped_book_text_from_slice(&bytes).unwrap();
            assert_eq!(loaded.book_text(), book.book_text());
            assert_eq!(index_book(loaded.book_text(), 2).search("乙").len(), 1);
            assert_eq!(loaded.locate(0, 1).unwrap_err().code(), expected, "{value}");
            assert_eq!(loaded.locate(1, 1), Err(SourceMapError::BadRange));
            assert_eq!(loaded.locate(3, 4), Err(SourceMapError::BadRange));
        }
    }

    #[test]
    fn corrupt_epub_node_paths_hrefs_and_synthetic_runs_are_rejected_lazily() {
        let (_, book) = epub();
        let original: Value =
            serde_json::from_slice(&mapped_book_text_to_vec(&book).unwrap()).unwrap();
        for (field, value) in [
            (2, "other.xhtml"),
            (2, "/OEBPS/missing.xhtml"),
            (3, "bad"),
            (3, "epubcfi(/2/2:1)"),
            (3, "epubcfi(/2/1/2)"),
        ] {
            let mut stored = original.clone();
            stored["sourceMap"]["runs"][0]["e"][field] = json!(value);
            let loaded =
                mapped_book_text_from_slice(&serde_json::to_vec(&stored).unwrap()).unwrap();
            assert_eq!(
                loaded.locate(0, 1).unwrap_err().code(),
                "BOOK_SOURCE_MAP_INVALID"
            );
            assert!(mapped_book_text_to_vec(&loaded).is_err());
        }
    }

    #[test]
    fn source_map_v1_wire_is_compact_and_uses_contiguous_lengths() {
        let book = txt("\r\n甲\r\n\r\n\r\n乙😀");
        let value: Value =
            serde_json::from_slice(&mapped_book_text_to_vec(&book).unwrap()).unwrap();
        assert_eq!(value["sourceMap"]["origin"], json!([2, 2]));
        assert_eq!(
            value["sourceMap"]["runs"],
            json!([
                {"t": [1, 3]}, {"n": [2, 2]}, {"n": [4, 4]}, {"t": [2, 7]},
            ])
        );
        let long = txt(&"甲😀aé".repeat(10000));
        assert!(
            serde_json::to_vec(long.source_map().unwrap())
                .unwrap()
                .len()
                < 256
        );
        let mut invalid = long.source_map().unwrap().clone();
        invalid.runs[0].end_char = 0;
        assert!(
            serde_json::to_vec(&invalid).is_err(),
            "invalid public maps must not panic on serialization"
        );
    }

    #[test]
    fn empty_persisted_bodies_keep_mapping_round_trips_and_reject_all_ranges() {
        for format in [Format::Txt, Format::Epub] {
            let document = match format {
                Format::Txt => SourceDocument::Txt {
                    decoded_chars: 0,
                    decoded_bytes: 0,
                },
                _ => SourceDocument::Epub,
            };
            let book = MappedBookText::with_source_map(
                BookText {
                    format,
                    ..BookText::default()
                },
                Ok(SourceMap::new(document)),
            );
            let bytes = mapped_book_text_to_vec(&book).unwrap();
            let loaded = mapped_book_text_from_slice(&bytes).unwrap();
            assert_eq!(loaded, book);
            assert_eq!(loaded.locate(0, 1), Err(SourceMapError::BadRange));
            assert_eq!(loaded.locate(0, 0), Err(SourceMapError::BadRange));
        }
    }
}
