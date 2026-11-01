#!/usr/bin/env python3
"""Single source for the existing FFI catalog, schemas, and method reference.

Generation uses only the standard library. Validation lives in check_api.py.
These schemas describe canonical calls; they do not change runtime parsing.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from api_examples import fixtures

ROOT = Path(__file__).resolve().parents[1]
DRAFT = "https://json-schema.org/draft/2020-12/schema"
API_MAJOR = 1
API_MINOR = 1
COMMON_ID = f"urn:perchcore:api:v{API_MAJOR}:common"
REQUEST_ID = f"urn:perchcore:api:v{API_MAJOR}:request"
RESPONSE_ID = f"urn:perchcore:api:v{API_MAJOR}:response"


def obj(properties=None, required=None, **kwargs):
    properties = properties or {}
    return {"type": "object", "properties": properties,
            "required": list(properties) if required is None else required,
            "additionalProperties": False, **kwargs}


def ref(name):
    return {"$ref": f"{COMMON_ID}#/$defs/{name}"}


def array(items):
    return {"type": "array", "items": items}


def nullable(schema):
    return {"anyOf": [schema, {"type": "null"}]}


def field(schema, description, default=None):
    result = {**schema, "description": description}
    if default is not None:
        result["default"] = default
    return result


def enum(*values):
    return {"type": "string", "enum": list(values)}


STRING = {"type": "string"}
BOOL = {"type": "boolean"}
NUMBER = {"type": "number"}
U64 = {"type": "integer", "minimum": 0, "maximum": 18446744073709551615}
U32 = {"type": "integer", "minimum": 0, "maximum": 4294967295}
U16 = {"type": "integer", "minimum": 0, "maximum": 65535}
I64 = {"type": "integer", "minimum": -9223372036854775808, "maximum": 9223372036854775807}
FORMAT = enum("epub", "txt", "pdf")
PRIORITY = enum("urgent", "normal", "background")
STATE = enum("registered", "refining", "indexing", "ready", "failed")
STAGE = enum("refine", "index")
HEX = {"type": "string", "pattern": "^(?:[0-9A-Fa-f]{2})*$"}
UUID = {"type": "string", "format": "uuid"}
HLC = field(STRING, "HLC 字符串，推荐非负十进制 wall-counter-device；解析和排序由 Rust 执行。")
HANDLE = field(U64, "当前进程的共享注册表句柄，0 有效；不能当作指针、布尔值或持久化 ID。")
CHUNK = field(U64, "索引分片的 Unicode scalar 数；0 在运行时回退为 4096。", 4096)
MAX_CHARS = field(U64, "抽取上限，按非空白 Unicode scalar 计数。", 5000000)
BYTES = field(HEX, "原始文件字节的偶数位 ASCII hex，无 0x 前缀；大小写均可。")
PATH = field(STRING, "文件路径，由运行库所在进程访问；推荐绝对路径。")

LOCATIONS = obj({
    "fragments": field(array(STRING), "EPUB 片段锚点，默认空数组。", []),
    "progression": field(nullable(NUMBER), "文档内进度；locator.validate 要求 [0,1]。"),
    "totalProgression": field(nullable(NUMBER), "全书进度；locator.validate 要求 [0,1]。"),
    "position": field(nullable(U32), "Readium position，未在此接口换算。"),
    "cssSelector": nullable(STRING), "partialCfi": nullable(STRING),
}, [])
PERCH = obj({
    "v": field({"const": 1}, "扩展结构 major；当前规范只支持 v1。"),
    "format": nullable(FORMAT), "pdfPage": field(nullable(U32), "PDF 页号，0 基。"),
    "pdfRect": field(nullable({"type": "array", "items": NUMBER, "minItems": 4, "maxItems": 4}),
                     "[x,y,w,h] 归一化矩形；validate 检查分量 [0,1]、w/h>0，不检查 x+w/y+h。"),
    "txtOffset": field(nullable(U64), "解码原文的 Unicode scalar 偏移。"),
    "txtByteOffset": field(nullable(U64), "解码后 UTF-8 文本的字节偏移，不是源编码文件的字节偏移。"),
    "chapterIndex": field(nullable(U32), "章节索引，0 基。"), "chapterTitle": nullable(STRING),
    "createdAt": field(nullable(I64), "UTC Unix epoch 毫秒。"),
    "modifiedAt": field(nullable(I64), "UTC Unix epoch 毫秒。"), "hlc": nullable(HLC),
}, ["v"])
LOCATOR = obj({
    "href": field(STRING, "资源标识；validate 要求非空白，不要求路径实际存在。"),
    "type": nullable(STRING), "title": nullable(STRING),
    "locations": field(ref("Locations"), "缺省为空对象。", {}),
    "text": nullable(ref("TextContext")),
    "perch": field(ref("Perch"), "缺省为默认扩展；提供对象时 v 必填。"),
}, ["href"])
ANNOTATION = obj({
    "id": UUID, "bookId": UUID, "kind": enum("highlight", "underline", "note", "bookmark"),
    "start": ref("Locator"), "end": nullable(ref("Locator")),
    "color": nullable(STRING), "note": nullable(STRING), "excerpt": nullable(STRING),
    "createdAt": field(I64, "UTC Unix epoch 毫秒。"), "modifiedAt": nullable(I64),
    "deleted": field(BOOL, "缺省 false；输出 false 时省略。", False), "hlc": nullable(HLC),
}, ["id", "bookId", "kind", "start", "createdAt"])
CHANGE_FIELDS = {"book": UUID, "annotation": nullable(UUID), "hlc": HLC}
CHANGE = {"oneOf": [
    obj({**CHANGE_FIELDS, "op": {"const": "set"}, "value": ref("WireValue")},
        ["book", "hlc", "op", "value"]),
    obj({**CHANGE_FIELDS, "op": {"const": "tombstone"}}, ["book", "hlc", "op"]),
]}
COMMON = {
    "Locator": LOCATOR, "Locations": LOCATIONS, "Perch": PERCH,
    "TextContext": obj({k: nullable(STRING) for k in ("before", "highlight", "after")}, []),
    "Annotation": ANNOTATION, "Change": CHANGE,
    "WireValue": {"oneOf": [
        obj({"kind": {"const": "progress"}, "locator": ref("Locator")}),
        obj({"kind": {"const": "annotation"}, "annotation": ref("Annotation")}),
    ]},
    "Error": obj({"error": obj({"code": STRING, "message": STRING})}),
    "BookOpened": obj({"handle": HANDLE, "charCount": U64, "chapterCount": U64,
                       "chunks": U64, "tokens": U64, "truncated": BOOL}),
    "BookHit": obj({"charStart": U64, "charEnd": U64, "snippet": STRING,
                    "locator": nullable(ref("Locator"))}),
    "IndexHit": obj({"char_start": U64, "char_end": U64, "chunk": U64, "snippet": STRING}),
    "Refinement": obj({"title": STRING, "author": STRING, "language": STRING, "encoding": STRING,
                       "coverHref": STRING, "toc": array(STRING), "spineCount": U32,
                       "charCount": U64}, ["spineCount", "charCount"]),
    "Outline": obj({"title": STRING, "pageIndex": U32, "children": array(ref("Outline"))}, ["title"]),
    "PdfInfo": obj({"pageCount": U32, "version": STRING, "title": STRING, "author": STRING,
                    "subject": STRING, "producer": STRING, "outline": array(ref("Outline"))},
                   ["pageCount", "version"]),
    "Policy": obj({"maxRefine": field(U64, "0 禁用精修；与 maxIndex 不能同时为 0；当前 step 每次只做一个任务。", 2),
                   "maxIndex": field(U64, "0 禁用索引；与 maxRefine 不能同时为 0。", 1),
                   "maxAttempts": field(U16, "须 >0，超过 u16 范围返回 FFI_BAD_ARGS。", 3),
                   "backoffBaseMs": field(U64, "退避毫秒，须 <= backoffMaxMs。", 2000),
                   "backoffMaxMs": field(U64, "最大退避毫秒。", 60000)}, []),
    "Gate": obj({"allowAny": field(BOOL, "false 时不开始任务。", True),
                 "allowHeavy": field(BOOL, "false 时不开始重任务。", True)}, []),
    "PipelineStats": obj({k: U64 for k in ("pendingRefine", "pendingIndex", "inFlight", "exhausted")}),
    "Recovered": obj({"book": UUID, "state": STATE, "attempts": U16,
                      "failedStage": nullable(STAGE), "reason": nullable(STRING), "retryAtMs": nullable(U64)}),
    "PipelineState": {"oneOf": [obj({"known": {"const": False}}),
        obj({"known": {"const": True}, "state": STATE, "attempts": U16,
             "failedStage": nullable(STAGE), "reason": nullable(STRING), "retryAtMs": nullable(U64)})]},
    "PipelineStep": {"oneOf": [
        obj({"outcome": {"const": "idle"}, "stats": ref("PipelineStats")}),
        obj({"outcome": {"const": "refined"}, "stats": ref("PipelineStats"), "book": UUID,
             "stage": {"const": "refine"}}),
        obj({"outcome": {"const": "indexed"}, "stats": ref("PipelineStats"), "book": UUID,
             "stage": {"const": "index"}, "chunks": U64, "tokens": U64}),
        obj({"outcome": {"const": "failed"}, "stats": ref("PipelineStats"), "book": UUID,
             "stage": STAGE, "reason": STRING, "attempts": U16, "retryAtMs": U64}),
        obj({"outcome": {"const": "exhausted"}, "stats": ref("PipelineStats"), "book": UUID,
             "stage": STAGE, "reason": STRING, "attempts": U16}),
    ]},
    "Point": obj({"x": NUMBER, "y": NUMBER}),
    "Shadow": obj({"start": ref("Point"), "end": ref("Point"),
                   "start_opacity": NUMBER, "end_opacity": NUMBER}),
    "Geometry": obj({"fold_start": ref("Point"), "fold_end": ref("Point"), "midpoint": ref("Point"),
                     "direction": ref("Point"), "radius": NUMBER, "curl_angle": NUMBER,
                     "band_extent": NUMBER, "progress": NUMBER, "front_outline": array(ref("Point")),
                     "shadow": ref("Shadow")}),
    "MergeStats": obj({k: U32 for k in ("initialized", "updated", "staleIgnored", "conflicts", "unchanged")}),
}

METHODS = []
JOBS_ERRORS = ["JOBS_EPUB", "JOBS_NO_CONTENT", "JOBS_EMPTY_CONTENT", "JOBS_UNSUPPORTED_FORMAT"]
BOOK_HANDLE_ERRORS = ["BOOK_BAD_HANDLE", "HANDLE_WRONG_KIND"]
TXT_HANDLE_ERRORS = ["TXT_BAD_HANDLE", "HANDLE_WRONG_KIND"]
PIPE_HANDLE_ERRORS = ["PIPELINE_BAD_HANDLE", "HANDLE_WRONG_KIND"]
SYNC_ERRORS = ["SYNC_TARGET_MISMATCH", "SYNC_INCONSISTENT"]


def method(name, summary, request, result, errors=(), notes=()):
    METHODS.append({"name": name, "summary": summary, "request": request, "result": result,
                    "errors": ["FFI_BAD_ARGS", *errors], "notes": list(notes)})


method("version", "读取核心 crate 版本", obj(), obj({"core": STRING}), notes=[
    "core 当前为 0.1.0；为保持旧严格解析器兼容，该结果不扩展。完整能力使用 compatibility。"])
method("compatibility", "读取 ABI、API、Locator 与产物兼容能力", obj(), obj({
    "core": STRING,
    "abi": {"const": 1},
    "api": obj({"major": {"const": API_MAJOR}, "minor": {"const": API_MINOR}}),
    "locator": obj({"read": {"const": [1]}, "write": {"const": 1}}),
    "artifacts": obj({
        "index": obj({"read": {"const": [1]}, "write": {"const": 1}}),
        "text": obj({"read": {"const": [0, 1]}, "write": {"const": 1}}),
    }),
}), notes=[
    "ABI/API/Locator/index/text 版本相互独立；read 是当前读取器接受的版本，write 是当前写入版本。",
    "旧核心没有此方法时返回 FFI_UNKNOWN_METHOD；调用方只能进入已知 legacy 分支。"])
method("locator.validate", "校验 Locator 的结构与定位语义", ref("Locator"),
       obj({"valid": {"const": True}, "format": nullable(FORMAT)}),
       ["MODEL_EMPTY_FIELD", "MODEL_UNSUPPORTED_VERSION", "MODEL_INVALID_PROGRESSION", "MODEL_INVALID_RECT", "MODEL_MISSING_ANCHOR"], [
           "args 本身是 Locator。format 仅来自 perch.format，不从 href/type 推断。",
           "语义规则见共享结构说明；Schema 检查结构，语义失败仍是合法请求及错误响应。"])
method("fingerprint", "计算文件大小与首尾样本指纹",
       obj({"size": field(U64, "原文件字节数。"), "head_hex": BYTES, "tail_hex": BYTES}),
       obj({"fingerprint": {"type": "string", "pattern": "^[0-9a-f]{64}$"}}), notes=[
           "推荐采样首尾各 64 KiB；这是采样指纹，不能替代完整文件 SHA-256。"])
method("jobs.refine", "精修 EPUB/TXT 元数据并统计正文",
       obj({"bytes_hex": BYTES, "format": FORMAT, "max_chars": MAX_CHARS}, ["bytes_hex"]),
       ref("Refinement"), JOBS_ERRORS, [
           "省略 format 时按内容嗅探，损坏 EPUB 可能被识别为 TXT，已知格式时应显式给定。",
           "charCount 为非空白 scalar；max_chars 控制抽取，结果不提供 truncated 字段。PDF 不支持精修。"])
method("book.open", "抽取正文并构建可复用搜索索引",
       obj({"bytes_hex": BYTES, "format": FORMAT, "chunk_chars": CHUNK}, ["bytes_hex"]),
       ref("BookOpened"), [*JOBS_ERRORS, "BOOK_TOO_MANY_OPEN"], [
           "省略 format 按内容嗅探；抽取固定上限 5,000,000 非空白 scalar，无 max_chars 参数。",
           "truncated 表示抽取被截断；charCount 是已抽取索引文本的 scalar 数。不要放在阅读首屏路径。"])
method("book.load", "加载管线产物并验证正文一致性",
       obj({"index_path": PATH, "text_path": PATH}), ref("BookOpened"),
       ["INDEX_IO", "INDEX_BAD_MAGIC", "INDEX_UNSUPPORTED_VERSION", "INDEX_CORRUPTED", "INDEX_BAD_UTF8",
        "INDEX_INTEGRITY", "BOOK_LOAD_IO", "BOOK_TEXT_JSON", "BOOK_TEXT_UNSUPPORTED_VERSION",
        "BOOK_INDEX_MISMATCH", "BOOK_TOO_MANY_OPEN"], [
            "index_path 对应 index.bin，text_path 对应 text.json；正文不相同则拒绝，不重新抽取或建索引。",
            "text.json 缺省 version 按 legacy v0 读取；当前写 v1，未知版本与 JSON 损坏使用不同错误码。"])
method("book.search", "搜索已打开的索引",
       obj({"handle": HANDLE, "query": STRING, "limit": field(U64, "结果上限，0 返回空数组。", 50)},
           ["handle", "query"]), obj({"hits": array(ref("BookHit"))}), BOOK_HANDLE_ERRORS, [
           "charStart/charEnd 是索引文本中的半开 scalar 区间。TXT 空白归一化后不等同原文偏移；EPUB Locator 为粗定位。",
           "按字面短语匹配，ASCII 忽略大小写；纯 emoji/标点无 token 时返回空。limit 在完整查询后应用。"])
method("book.close", "释放搜索索引句柄", obj({"handle": HANDLE}), obj({"closed": BOOL}),
       ["HANDLE_WRONG_KIND"], ["重复关闭或未知数字返回 closed:false；错类型不关闭其他对象。"])
method("txt.open", "按路径打开并解码 TXT",
       obj({"path": PATH, "encoding": field(STRING, "可选 encoding_rs 编码标签，显式标签跳过探测。")}, ["path"]),
       obj({"handle": HANDLE, "chars": U64, "lines": U64, "encoding": STRING,
            "certain": BOOL, "hadErrors": BOOL}), ["TXT_IO", "TXT_UNKNOWN_ENCODING", "TXT_TOO_MANY_OPEN"], [
            "默认按 BOM、合法 UTF-8、chardetng 探测；全文读取并解码，保留换行和空白。chars 是 scalar 数。"])
method("txt.slice", "读取有上限的原文窗口",
       obj({"handle": HANDLE, "start": field(U64, "起点，Unicode scalar 偏移，可等于 EOF。", 0),
            "chars": field({**U64, "minimum": 1, "maximum": 20000}, "请求 scalar 数，1..20,000。")},
           ["handle", "chars"]), obj({"text": STRING, "start": U64, "end": U64, "chars": U64, "total": U64}),
       [*TXT_HANDLE_ERRORS, "TXT_SLICE_BAD_SIZE", "TXT_OUT_OF_RANGE"], [
           "返回 [start,end)，尾部不足时截到 EOF，chars 为实际数量；EOF 切片返回空串。"])
method("txt.locate", "将原文字符偏移转换为 Locator", obj({"handle": HANDLE, "char": U64}),
       obj({"locator": ref("Locator")}), [*TXT_HANDLE_ERRORS, "TXT_OUT_OF_RANGE"], [
           "char 是 scalar 偏移，允许 EOF；生成 txtOffset、解码 UTF-8 txtByteOffset 和 progression。"])
method("txt.offset", "将 Locator 解析为原文字符偏移",
       obj({"handle": HANDLE, "locator": ref("Locator")}), obj({"char": nullable(U64), "resolved": BOOL}),
       TXT_HANDLE_ERRORS, [
           "不调用 Locator.validate。按 txtOffset、txtByteOffset、progression 顺序解析。",
           "存在的较高优先级锚点无效时直接 unresolved，不继续回退；progression 路径没有额外范围检查，结果不保证在文档内。"])
method("txt.line", "读取偏移所在行与区间", obj({"handle": HANDLE, "char": U64}),
       obj({"line": U64, "start": nullable(U64), "end": nullable(U64), "totalLines": U64}),
       [*TXT_HANDLE_ERRORS, "TXT_OUT_OF_RANGE"], [
           "char 是 scalar 偏移，允许 EOF；line 从 0 起。行区间含行尾 LF；CRLF 中 CR 属于范围，单独 CR 不新增索引行。"])
method("txt.close", "释放 TXT 句柄", obj({"handle": HANDLE}), obj({"closed": BOOL}),
       ["HANDLE_WRONG_KIND"], ["重复关闭或未知数字返回 closed:false；错类型不关闭其他对象。"])
method("txt.analyze", "解码字节并返回全文", obj({"bytes_hex": BYTES}),
       obj({"encoding": STRING, "certain_encoding": BOOL, "chars": U64, "lines": U64,
            "had_decode_errors": BOOL, "text": STRING}), notes=[
            "保留 underscore 字段名；不提供显式编码参数，chars 为原解码文本 scalar 数。",
            "bytes_hex 为原文件两倍长度，结果返回全文。阅读窗口使用 txt.open/slice。"])
method("pipeline.open", "打开可恢复的路径导入管线",
       obj({"out_dir": field(STRING, "产物根目录，一条管线单独拥有；按 book UUID 建子目录。"),
            "policy": ref("Policy"), "max_chars": MAX_CHARS, "chunk_chars": CHUNK}, ["out_dir"]),
       obj({"handle": HANDLE, "recovered": array(ref("Recovered"))}),
       ["PIPELINE_IO", "PIPELINE_POLICY", "PIPELINE_TOO_MANY_OPEN"], [
           "创建和扫描目录在策略/配额检查前执行；失败可能已产生目录副作用。恢复规则见 storage.md。"])
method("pipeline.enqueue", "登记源路径并按落盘状态恢复入队",
       obj({"handle": HANDLE, "book": UUID, "path": PATH, "format": FORMAT,
            "priority": field(PRIORITY, "调度优先级。", "normal"),
            "state": field(STATE, "无已记录状态时使用。", "registered")}, ["handle", "book", "path", "format"]),
       obj({"outcome": enum("queued", "alreadyPresent", "notNeeded"), "stage": nullable(STAGE),
            "state": STATE, "attempts": U16}), PIPE_HANDLE_ERRORS, [
            "已有落盘状态优先于 state 参数；alreadyPresent/notNeeded 的 stage 为 null。",
            "源路径注册在部分参数校验前更新；PDF 可入队，但精修/索引任务会失败。"])
method("pipeline.step", "同步执行一个符合闸门的任务",
       obj({"handle": HANDLE, "now_ms": field(U64, "UTC Unix epoch 毫秒；省略取系统当前时间。"),
            "gate": ref("Gate")}, ["handle"]), ref("PipelineStep"), [*PIPE_HANDLE_ERRORS, "PIPELINE_IO"], [
            "正常 outcome 有 idle/refined/indexed/failed/exhausted，字段依分支而异；failed 可退避重试，exhausted 须手动 retry。",
            "存储 I/O 失败映射顶层 PIPELINE_IO，不返回 ioFailed 成功结果。执行期间持共享注册锁。"])
method("pipeline.status", "读取队列概览", obj({"handle": HANDLE}),
       obj({"idle": BOOL, "stats": ref("PipelineStats")}), PIPE_HANDLE_ERRORS)
method("pipeline.state", "读取单本书的落盘状态", obj({"handle": HANDLE, "book": UUID}),
       ref("PipelineState"), PIPE_HANDLE_ERRORS, ["unknown 仅返回 known:false；结果不含 updatedAtMs。"])
method("pipeline.promote", "调整书籍优先级",
       obj({"handle": HANDLE, "book": UUID, "priority": field(PRIORITY, "调度优先级。", "normal")},
           ["handle", "book"]), obj({"promoted": {"const": True}, "priority": PRIORITY}), PIPE_HANDLE_ERRORS,
       ["promoted:true 不保证书籍存在或已经排队。"])
method("pipeline.retry", "手动复活已耗尽的任务",
       obj({"handle": HANDLE, "book": UUID, "priority": field(PRIORITY, "调度优先级。", "normal")},
           ["handle", "book"]), obj({"retried": BOOL}), [*PIPE_HANDLE_ERRORS, "PIPELINE_IO"],
       ["仅 exhausted 任务可复活；不存在或尚未耗尽时 retried:false。"])
method("pipeline.cancel", "撤出书籍排队任务", obj({"handle": HANDLE, "book": UUID}),
       obj({"cancelled": {"const": True}}), PIPE_HANDLE_ERRORS,
       ["不删除源文件/产物；不抢占或中断已在执行的同步 step。"])
method("pipeline.close", "释放导入管线句柄", obj({"handle": HANDLE}), obj({"closed": BOOL}),
       ["HANDLE_WRONG_KIND"], ["重复关闭或未知数字返回 closed:false；保留磁盘产物供重开恢复。"])
method("pdf.analyze", "分析 PDF 页数、大纲与元数据",
       obj({"path": PATH, "bytes_hex": BYTES}, [], oneOf=[{"required": ["path"]}, {"required": ["bytes_hex"]}]),
       ref("PdfInfo"), ["PDF_IO", "PDF_EMPTY", "PDF_NOT_PDF", "PDF_ENCRYPTED", "PDF_NO_PAGES", "PDF_CORRUPTED"],
       ["path/bytes_hex 必须且只能存在一个；null 字段也算存在。只做结构解析，不抽正文。",
        "outline 为空时省略；每项 title 必填，pageIndex 为 0 基可省略，children 为空时省略。"])
method("index.search", "一次性构建临时索引并搜索",
       obj({"text": STRING, "query": STRING, "chunk_chars": CHUNK}, ["text", "query"]),
       obj({"hits": array(ref("IndexHit"))}), notes=[
           "保留 underscore 字段名；char_start/char_end 为入参 text 的半开 scalar 区间。chunk 从 0 起。",
           "字面短语，ASCII 忽略大小写；纯 emoji/标点无 token 时返回空；没有结果 limit。"])
method("curl", "计算页面卷曲几何",
       obj({"page_w": NUMBER, "page_h": NUMBER, "corner": enum("top_left", "top_right", "bottom_left", "bottom_right"),
            "drag_x": NUMBER, "drag_y": NUMBER, "radius": NUMBER}), obj({"geometry": nullable(ref("Geometry"))}),
       notes=["坐标与页面尺寸同单位，原点左上、y 向下；curl_angle 为弧度。",
              "无效尺寸/半径或无有效折线通常返回 geometry:null；不是业务错误。"])
method("hlc.parse", "解析混合逻辑时钟", obj({"hlc": HLC}),
       obj({"wall_ms": I64, "counter": U32, "device": STRING}), ["SYNC_BAD_HLC"], [
           "格式为 wall-counter-device，device 可含连字符但不可空；wall 为 epoch 毫秒，counter 从 0 起。",
           "现有 splitn 解析不接受负 wall 字符串，即使内部类型支持负值。Schema 不替代 Rust HLC 解析。"])
method("sync.merge", "合并当前寄存器状态与远端变更",
       obj({"local": field(array(ref("Change")), "缺省为空数组。", []),
            "remote": field(array(ref("Change")), "缺省为空数组。", [])}, []),
       obj({"state": array(ref("Change")), "stats": ref("MergeStats"), "conflictRate": NUMBER,
            "effects": array(enum("initialized", "updated", "staleIgnored", "conflictResolved", "unchanged"))}),
       SYNC_ERRORS, ["local/remote 显式 null 是 FFI_BAD_ARGS；仅保留各目标当前 LWW 胜者，不保存撤销历史。",
                     "无网络 I/O，也没有协议 version 参数；Change 内部一致性检查见共享结构。"])
method("sync.delta", "按 HLC 游标分页读取寄存器当前状态",
       obj({"state": field(array(ref("Change")), "缺省为空数组。", []), "since": nullable(HLC),
            "limit": field(U64, "页大小，运行时 0 clamp 到 1。", 500)}, []),
       obj({"changes": array(ref("Change")), "nextCursor": nullable(HLC), "hasMore": BOOL}),
       [*SYNC_ERRORS, "SYNC_BAD_HLC"], [
           "仅选择 HLC 严格大于 since 的当前寄存器；使用返回 nextCursor 继续。无变化时 nextCursor 为 null。",
           "省略/null 的 since 从头读取；运行时非字符串 since 也当作缺省，规范 Schema 拒绝该类型。"])


def schema_type(schema):
    if "$ref" in schema:
        name = schema["$ref"].rsplit("/", 1)[-1]
        return f"[{name}](#{name.lower()})"
    if "const" in schema:
        return "`" + json.dumps(schema["const"], ensure_ascii=False) + "`"
    if "enum" in schema:
        return " / ".join(f"`{value}`" for value in schema["enum"])
    if "anyOf" in schema:
        return " 或 ".join(schema_type(part) for part in schema["anyOf"])
    if "oneOf" in schema:
        return "按分支对象（见下表）"
    if schema.get("type") == "array":
        return f"array&lt;{schema_type(schema['items'])}&gt;"
    if schema.get("type") == "integer":
        return f"integer [{schema.get('minimum', '−∞')}, {schema.get('maximum', '∞')}]"
    return schema.get("type", "object")


def tables(schema):
    if "$ref" in schema:
        return [f"结构：{schema_type(schema)}。", ""]
    if "oneOf" in schema:
        lines = []
        for i, part in enumerate(schema["oneOf"], 1):
            lines += [f"分支 {i}：", "", *tables(part)]
        return lines
    if not schema.get("properties"):
        return ["空对象 `{}`。", ""]
    lines = ["| 字段 | 类型 | 必填 | 默认值 / 说明 |", "|---|---|---|---|"]
    for key, value in schema["properties"].items():
        description = value.get("description", "")
        if "default" in value:
            description = "`" + json.dumps(value["default"], ensure_ascii=False) + "`；" + description
        if "pattern" in value:
            description += f" 正则：`{value['pattern']}`。"
        lines.append(f"| `{key}` | {schema_type(value)} | {'是' if key in schema['required'] else '否'} | {description} |")
    return [*lines, ""]


def reference():
    lines = ["# JSON 方法参考", "", "<!-- Generated by tools/api_catalog.py; edit that source. -->", "",
             f"本参考覆盖现有 {len(METHODS)} 个 FFI 方法。调用、内存与宽松解析规则见 [FFI 契约](ffi.md)，",
             "可重放请求与完整响应见 [示例序列](../api/examples.json) 和 [响应样例](../api/responses.json)。",
             "Schema 检查结构与规范类型；明确注明的语义约束由对应 Rust 方法判断。", "",
             "所有方法都可能因 JSON 语法/字段问题返回 `FFI_BAD_ARGS`；C 输入 UTF-8 无效时返回",
             "`FFI_BAD_METHOD_PTR` / `FFI_BAD_ARGS_PTR`，未知方法返回 `FFI_UNKNOWN_METHOD`。",
             "下方错误列表列出各方法的字段/业务错误；message 为说明，不作为稳定匹配字段。", "",
             "## 方法索引", "", "| 方法 | 用途 |", "|---|---|"]
    for entry in METHODS:
        lines.append(f"| [{entry['name']}](#{entry['name'].replace('.', '')}) | {entry['summary']} |")
    lines += ["", "## 共享结构", "",
              "Locator 的 locations/text/perch 可省略。可空字段的 null 与省略在反序列化时等价，",
              "但数组/对象的默认值不接受 null，perch 对象存在时 v 必填。输出的可空模型字段一般省略；",
              "直接构造的 FFI 结果（如 txt.offset、PipelineState）按表明确返回 null。", "",
              "locator.validate 额外检查 href 非空白、progression/totalProgression 在 [0,1]、",
              "pdfRect 分量在 [0,1] 且 w/h>0；不检查 x+w/y+h。perch.format=txt 要求 txtOffset，",
              "pdf 要求 pdfPage，epub 要求 partialCfi/cssSelector/fragments/progression 至少一个；",
              "无 format 时不要求格式锚点。perch 存在时 v 必须为当前支持的 1。", "",
              "Change 的 annotation 省略/null 表示阅读进度目标。set 的 value 为 progress 或 annotation，",
              "tombstone 无 value。Rust 检查目标与值类型、标注 id/bookId/自带 hlc 的一致性；",
              "不会全面调用 Locator.validate 或 Annotation.validate。HLC 的语法由 Rust 解析，",
              "UUID 使用规范带连字符表示（运行时还可能接受其他 UUID 写法）。", ""]
    for name, structure in COMMON.items():
        lines += [f"### {name}", "", *tables(structure)]
    lines += ["## 逐项调用", ""]
    for entry in METHODS:
        lines += [f"### {entry['name']}", "", entry["summary"] + "。", "", "参数：", "",
                  *tables(entry["request"]), "成功结果：", "", *tables(entry["result"]),
                  "错误码：" + "、".join(f"`{e}`" for e in entry["errors"]) + "。", ""]
        for note in entry["notes"]:
            lines += [note, ""]
    return "\n".join(lines)


def artifacts():
    outputs = {"api/examples.json": fixtures(),
               "api/common.schema.json": {"$schema": DRAFT, "$id": COMMON_ID, "$defs": COMMON},
               "api/catalog.json": {"description": "Versioned canonical FFI contracts and runtime negotiation metadata.",
                                    "api": {"major": API_MAJOR, "minor": API_MINOR},
                                    "methods": METHODS}}
    for key, ident, filename in (("request", REQUEST_ID, "request"), ("result", RESPONSE_ID, "response")):
        alternatives = []
        for entry in METHODS:
            payload = entry[key]
            if key == "result":
                payload = {"oneOf": [payload, ref("Error")]}
            alternatives.append(obj({"method": {"const": entry["name"]},
                                     "args" if key == "request" else "result": payload}))
        outputs[f"api/{filename}.schema.json"] = {"$schema": DRAFT, "$id": ident, "oneOf": alternatives}
    result = {path: json.dumps(content, ensure_ascii=False, indent=2) + "\n" for path, content in outputs.items()}
    result["docs/api-reference.md"] = reference()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Fail if generated contracts differ")
    args = parser.parse_args()
    different = []
    for name, content in artifacts().items():
        path = ROOT / name
        if args.check:
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                different.append(name)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8", newline="\n")
    if different:
        parser.exit(1, "Contract drift; run python tools/api_catalog.py: " + ", ".join(different) + "\n")
    print(f"{'Checked' if args.check else 'Generated'} {len(METHODS)} methods / {len(artifacts())} artifacts")


if __name__ == "__main__":
    main()
