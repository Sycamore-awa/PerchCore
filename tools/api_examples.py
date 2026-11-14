"""Reviewable calls and independent behavioral assertions for API-01 replay."""

from __future__ import annotations

TEXT = "中文🦀café\r\n第二行\r末尾\n"
BOOK = "00000000-0000-4000-8000-000000000001"
BAD_BOOK = "00000000-0000-4000-8000-000000000002"
NOTE = "00000000-0000-4000-8000-000000000003"
IO_BOOK = "00000000-0000-4000-8000-000000000004"
LOC_BOOK = "00000000-0000-4000-8000-000000000005"
LOC_TEXT = "\r\n甲\r\n\r\n\r\n乙😀é中文"


def variable(name):
    return {"$var": name}


def capture(step, field):
    return {"$ref": f"{step}.{field}"}


def fixtures():
    calls = []

    def call(ident, method, args=None, expect=None, error=None, canonical=True, context=None):
        step = {"id": ident, "method": method, "args": args or {}, "canonical": canonical}
        if context is not None:
            step["context"] = context
        if expect is not None:
            step["expect"] = expect
        if error:
            step["error"] = error
        calls.append(step)

    txt = capture("txt_open", "handle")
    book = capture("book_open", "handle")
    pipe = capture("pipeline_open", "handle")
    locator = {"href": "中文.txt", "locations": {"progression": 0.5},
               "perch": {"v": 1, "format": "txt", "txtOffset": 2, "txtByteOffset": 6}}
    progress = {"book": BOOK, "hlc": "1700000000000-0-phone-a", "op": "set",
                "value": {"kind": "progress", "locator": locator}}
    annotation = {"id": NOTE, "bookId": BOOK, "kind": "note", "start": locator,
                  "note": "中文标注 🦀", "createdAt": 1700000000000, "modifiedAt": None,
                  "hlc": "1700000000001-0-tablet"}
    change = {"book": BOOK, "annotation": NOTE, "hlc": "1700000000001-0-tablet",
              "op": "set", "value": {"kind": "annotation", "annotation": annotation}}
    call("version", "version", expect={"core": "0.1.0"})
    call("compatibility", "compatibility", expect={"abi": 1, "api": {"major": 1, "minor": 5},
         "locator": {"read": [1], "write": 1},
         "artifacts": {"index": {"read": [1], "write": 1},
                       "text": {"read": [0, 1], "write": 1}}})
    call("unknown_method", "unknown.method", error="FFI_UNKNOWN_METHOD", canonical=False)
    call("version_loose", "version", {"unexpected": True}, {"core": "0.1.0"}, canonical=False)
    call("locator_txt", "locator.validate", locator, {"valid": True, "format": "txt"})
    call("locator_plain", "locator.validate", {"href": "chapter.xhtml"}, {"format": None})
    call("locator_epub", "locator.validate", {"href": "c.xhtml", "locations": {"partialCfi": "epubcfi(/6/2)"},
         "perch": {"v": 1, "format": "epub"}}, {"format": "epub"})
    call("locator_future", "locator.validate", {"href": "page", "perch": {"v": 2, "format": "pdf",
         "pdfPage": 0}}, error="MODEL_UNSUPPORTED_VERSION", canonical=False)
    call("locator_pdf", "locator.validate", {"href": "page", "perch": {"v": 1, "format": "pdf",
         "pdfPage": 0, "pdfRect": [0.9, 0.9, 0.2, 0.2]}}, {"format": "pdf"})
    call("locator_empty", "locator.validate", {"href": " "}, error="MODEL_EMPTY_FIELD")
    call("locator_progression", "locator.validate", {"href": "a", "locations": {"progression": 1.2}},
         error="MODEL_INVALID_PROGRESSION")
    call("locator_anchor", "locator.validate", {"href": "a", "perch": {"v": 1, "format": "txt"}},
         error="MODEL_MISSING_ANCHOR")
    call("locator_rect", "locator.validate", {"href": "a", "perch": {"v": 1, "pdfRect": [0, 0, 0, 1]}},
         error="MODEL_INVALID_RECT")
    call("fingerprint", "fingerprint", {"size": 12, "head_hex": "e4B8ad", "tail_hex": "f09Fa680"})
    call("hex_odd", "fingerprint", {"size": 0, "head_hex": "0", "tail_hex": ""},
         error="FFI_BAD_ARGS", canonical=False)
    call("hex_non_ascii", "txt.analyze", {"bytes_hex": "中文"}, error="FFI_BAD_ARGS", canonical=False)
    call("txt_analyze", "txt.analyze", {"bytes_hex": TEXT.encode().hex()}, {"text": TEXT, "chars": 17})
    call("refine", "jobs.refine", {"bytes_hex": TEXT.encode().hex(), "format": "txt"})
    call("refine_epub", "jobs.refine", {"bytes_hex": variable("epub_hex"), "format": "epub"})
    call("refine_pdf", "jobs.refine", {"bytes_hex": "", "format": "pdf"}, error="JOBS_UNSUPPORTED_FORMAT")
    call("book_open", "book.open", {"bytes_hex": TEXT.encode().hex(), "format": "txt", "chunk_chars": 0},
         {"handle": 0, "truncated": False})
    call("book_search", "book.search", {"handle": book, "query": "中文"})
    call("book_search_zero", "book.search", {"handle": book, "query": "中文", "limit": 0}, {"hits": []})
    call("book_epub_open", "book.open", {"bytes_hex": variable("epub_hex"), "format": "epub"})
    call("book_epub_search", "book.search", {"handle": capture("book_epub_open", "handle"), "query": "海风"})
    call("book_epub_close", "book.close", {"handle": capture("book_epub_open", "handle")}, {"closed": True})
    call("txt_open", "txt.open", {"path": variable("source")}, {"chars": 17, "lines": 3, "hadErrors": False})
    call("wrong_close", "book.close", {"handle": txt}, error="HANDLE_WRONG_KIND")
    call("wrong_use", "book.search", {"handle": txt, "query": "中文"}, error="HANDLE_WRONG_KIND")
    call("txt_slice", "txt.slice", {"handle": txt, "start": 1, "chars": 3},
         {"text": "文🦀c", "start": 1, "end": 4, "chars": 3, "total": 17})
    call("txt_slice_eof", "txt.slice", {"handle": txt, "start": 17, "chars": 20000}, {"text": "", "chars": 0})
    call("txt_slice_tail", "txt.slice", {"handle": txt, "start": 15, "chars": 20000}, {"text": "尾\n", "chars": 2})
    call("txt_slice_zero", "txt.slice", {"handle": txt, "chars": 0}, error="TXT_SLICE_BAD_SIZE", canonical=False)
    call("txt_slice_large", "txt.slice", {"handle": txt, "chars": 20001}, error="TXT_SLICE_BAD_SIZE", canonical=False)
    call("txt_slice_range", "txt.slice", {"handle": txt, "start": 18, "chars": 1}, error="TXT_OUT_OF_RANGE")
    call("txt_locate", "txt.locate", {"handle": txt, "char": 2}, {"locator": {"perch": {"txtOffset": 2, "txtByteOffset": 6}}})
    call("txt_locate_eof", "txt.locate", {"handle": txt, "char": 17}, {"locator": {"perch": {"txtOffset": 17}}})
    call("txt_offset", "txt.offset", {"handle": txt, "locator": capture("txt_locate", "locator")}, {"char": 2, "resolved": True})
    call("txt_offset_byte", "txt.offset", {"handle": txt, "locator": {"href": "a", "perch": {"v": 1, "txtByteOffset": 6}}}, {"char": 2})
    call("txt_offset_byte_invalid", "txt.offset", {"handle": txt, "locator": {"href": "a", "locations": {"progression": 0.5},
         "perch": {"v": 1, "txtByteOffset": 7}}}, {"char": None, "resolved": False})
    call("txt_offset_no_fallback", "txt.offset", {"handle": txt, "locator": {"href": "a", "locations": {"progression": 0.5},
         "perch": {"v": 1, "txtOffset": 99}}}, {"char": None, "resolved": False})
    call("txt_offset_progression", "txt.offset", {"handle": txt, "locator": {"href": "a", "locations": {"progression": 0.5}}}, {"char": 9})
    call("txt_offset_unchecked", "txt.offset", {"handle": txt, "locator": {"href": "", "locations": {"progression": 1.2}}}, {"char": 20})
    call("txt_line", "txt.line", {"handle": txt, "char": 9}, {"line": 0, "start": 0, "end": 10, "totalLines": 3})
    call("txt_line_cr", "txt.line", {"handle": txt, "char": 14}, {"line": 1, "start": 10, "end": 17})
    call("txt_line_eof", "txt.line", {"handle": txt, "char": 17}, {"line": 2, "start": 17, "end": 17})
    call("txt_encoding", "txt.open", {"path": variable("gbk"), "encoding": "GBK"}, {"encoding": "GBK", "hadErrors": False})
    call("txt_encoding_slice", "txt.slice", {"handle": capture("txt_encoding", "handle"), "chars": 3}, {"text": "中文编"})
    call("txt_encoding_close", "txt.close", {"handle": capture("txt_encoding", "handle")}, {"closed": True})
    call("txt_bad_encoding", "txt.open", {"path": variable("source"), "encoding": "unknown-label"}, error="TXT_UNKNOWN_ENCODING")
    call("txt_missing", "txt.open", {"path": variable("missing")}, error="TXT_IO")
    call("pipeline_bad_policy", "pipeline.open", {"out_dir": variable("bad_policy_dir"), "policy": {"maxRefine": 0, "maxIndex": 0}}, error="PIPELINE_POLICY")
    call("pipeline_refine_only", "pipeline.open", {"out_dir": variable("bad_policy_dir"), "policy": {"maxIndex": 0}}, {"recovered": []})
    call("pipeline_refine_only_close", "pipeline.close", {"handle": capture("pipeline_refine_only", "handle")}, {"closed": True})
    call("pipeline_bad_attempts", "pipeline.open", {"out_dir": variable("bad_policy_dir"), "policy": {"maxAttempts": 65536}}, error="FFI_BAD_ARGS", canonical=False)
    call("pipeline_bad_io", "pipeline.open", {"out_dir": variable("blocked")}, error="PIPELINE_IO")
    call("pipeline_open", "pipeline.open", {"out_dir": variable("out"), "policy": {"maxAttempts": 2,
         "backoffBaseMs": 10, "backoffMaxMs": 10}}, {"recovered": []})
    call("pipeline_quota", "pipeline.open", {"out_dir": variable("out2")}, error="PIPELINE_TOO_MANY_OPEN")
    call("pipeline_state_unknown", "pipeline.state", {"handle": pipe, "book": BOOK}, {"known": False})
    call("pipeline_enqueue", "pipeline.enqueue", {"handle": pipe, "book": BOOK, "path": variable("source"), "format": "txt"}, {"outcome": "queued", "stage": "refine"})
    call("pipeline_enqueue_duplicate", "pipeline.enqueue", {"handle": pipe, "book": BOOK, "path": variable("source"), "format": "txt"}, {"outcome": "alreadyPresent", "stage": None})
    call("pipeline_status", "pipeline.status", {"handle": pipe}, {"idle": False, "stats": {"pendingRefine": 1}})
    call("pipeline_progress_idle", "pipeline.progress", {"handle": pipe}, {"active": None})
    call("pipeline_gate", "pipeline.step", {"handle": pipe, "now_ms": 1700000000000, "gate": {"allowAny": False}}, {"outcome": "idle"})
    call("pipeline_promote", "pipeline.promote", {"handle": pipe, "book": BOOK, "priority": "urgent"}, {"promoted": True, "priority": "urgent"})
    call("pipeline_refined", "pipeline.step", {"handle": pipe, "now_ms": 1700000000000}, {"outcome": "refined", "stage": "refine"})
    call("pipeline_heavy_gate", "pipeline.step", {"handle": pipe, "now_ms": 1700000000000, "gate": {"allowHeavy": False}}, {"outcome": "idle"})
    call("pipeline_indexed", "pipeline.step", {"handle": pipe, "now_ms": 1700000000000}, {"outcome": "indexed", "stage": "index"})
    call("pipeline_state", "pipeline.state", {"handle": pipe, "book": BOOK}, {"known": True, "state": "ready", "attempts": 0, "failedStage": None})
    call("pipeline_ready_enqueue", "pipeline.enqueue", {"handle": pipe, "book": BOOK, "path": variable("source"), "format": "txt", "state": "illegal"},
         {"outcome": "notNeeded", "state": "ready"}, canonical=False)
    call("book_load", "book.load", {"index_path": variable("index"), "text_path": variable("text_json")})
    call("book_load_search", "book.search", {"handle": capture("book_load", "handle"), "query": "中文"})
    call("book_load_close", "book.close", {"handle": capture("book_load", "handle")}, {"closed": True})
    calls.append({"action": "future_text"})
    call("book_load_future_text", "book.load", {"index_path": variable("index"),
         "text_path": variable("future_json")}, error="BOOK_TEXT_UNSUPPORTED_VERSION")
    calls.append({"action": "mismatch_text"})
    call("book_load_mismatch", "book.load", {"index_path": variable("index"), "text_path": variable("mismatch_json")}, error="BOOK_INDEX_MISMATCH")
    call("pipeline_failed_enqueue", "pipeline.enqueue", {"handle": pipe, "book": BAD_BOOK, "path": variable("pdf"), "format": "pdf"}, {"outcome": "queued"})
    call("pipeline_failed", "pipeline.step", {"handle": pipe, "now_ms": 1700000000000}, {"outcome": "failed", "attempts": 1, "retryAtMs": 1700000000010})
    call("pipeline_early_retry", "pipeline.step", {"handle": pipe, "now_ms": 1700000000009}, {"outcome": "idle"})
    call("pipeline_exhausted", "pipeline.step", {"handle": pipe, "now_ms": 1700000000010}, {"outcome": "exhausted", "attempts": 2})
    call("pipeline_failed_state", "pipeline.state", {"handle": pipe, "book": BAD_BOOK}, {"state": "failed", "attempts": 2, "failedStage": "refine", "retryAtMs": None})
    call("pipeline_retry", "pipeline.retry", {"handle": pipe, "book": BAD_BOOK}, {"retried": True})
    call("pipeline_cancel", "pipeline.cancel", {"handle": pipe, "book": BAD_BOOK}, {"cancelled": True})
    call("pipeline_retry_unknown", "pipeline.retry", {"handle": pipe, "book": NOTE}, {"retried": False})
    calls.append({"action": "block_book_dir", "book": IO_BOOK})
    call("pipeline_io_enqueue", "pipeline.enqueue", {"handle": pipe, "book": IO_BOOK, "path": variable("source"), "format": "txt"})
    call("pipeline_step_io", "pipeline.step", {"handle": pipe, "now_ms": 1700000000020}, error="PIPELINE_IO")
    call("pipeline_io_cancel", "pipeline.cancel", {"handle": pipe, "book": IO_BOOK})
    call("pipeline_idle", "pipeline.step", {"handle": pipe, "now_ms": 1700000000020}, {"outcome": "idle"})
    call("pipeline_close", "pipeline.close", {"handle": pipe}, {"closed": True})
    call("pipeline_close_twice", "pipeline.close", {"handle": pipe}, {"closed": False})
    call("pipeline_reopen", "pipeline.open", {"out_dir": variable("out")})
    call("pipeline_reopened_state", "pipeline.state", {"handle": capture("pipeline_reopen", "handle"), "book": BOOK}, {"state": "ready"})
    call("pipeline_reopened_close", "pipeline.close", {"handle": capture("pipeline_reopen", "handle")}, {"closed": True})
    call("pdf_analyze", "pdf.analyze", {"path": variable("pdf")}, {"pageCount": 3})
    call("pdf_bytes", "pdf.analyze", {"bytes_hex": variable("pdf_hex")}, {"pageCount": 3})
    call("pdf_both", "pdf.analyze", {"path": variable("pdf"), "bytes_hex": ""}, error="FFI_BAD_ARGS", canonical=False)
    call("pdf_empty", "pdf.analyze", {"bytes_hex": ""}, error="PDF_EMPTY")
    call("pdf_invalid", "pdf.analyze", {"bytes_hex": "616263"}, error="PDF_NOT_PDF")
    call("index_search", "index.search", {"text": "中文🦀 English Phrase 中文", "query": "english phrase", "chunk_chars": 4})
    call("index_no_tokens", "index.search", {"text": "中文🦀", "query": "🦀"}, {"hits": []})
    curl_args = {"page_w": 600, "page_h": 800, "corner": "bottom_right", "drag_x": 300, "drag_y": 650, "radius": 40}
    call("curl", "curl", curl_args)
    call("curl_none", "curl", {**curl_args, "radius": 0}, {"geometry": None})
    call("hlc_parse", "hlc.parse", {"hlc": "1700000000000-3-phone-a"}, {"wall_ms": 1700000000000, "counter": 3, "device": "phone-a"})
    call("hlc_bad", "hlc.parse", {"hlc": "-1-0-a"}, error="SYNC_BAD_HLC")
    call("sync_merge", "sync.merge", {"local": [progress], "remote": [change]}, {"effects": ["initialized"]})
    call("sync_delta", "sync.delta", {"state": capture("sync_merge", "state"), "limit": 0}, {"hasMore": True, "nextCursor": "1700000000000-0-phone-a"})
    call("sync_delta_next", "sync.delta", {"state": capture("sync_merge", "state"), "since": capture("sync_delta", "nextCursor"), "limit": 1}, {"hasMore": False})
    call("sync_tombstone", "sync.merge", {"local": capture("sync_merge", "state"), "remote": [
         {"book": BOOK, "annotation": NOTE, "hlc": "1700000000002-0-tablet", "op": "tombstone"}]})
    call("sync_target_bad", "sync.merge", {"remote": [{**progress, "annotation": NOTE}]}, error="SYNC_TARGET_MISMATCH")
    call("sync_inconsistent", "sync.merge", {"remote": [{**change, "book": BAD_BOOK}]}, error="SYNC_INCONSISTENT")
    call("sync_null", "sync.merge", {"local": None}, error="FFI_BAD_ARGS", canonical=False)
    call("sync_since_bad", "sync.delta", {"since": "bad"}, error="SYNC_BAD_HLC")
    call("book_close", "book.close", {"handle": book}, {"closed": True})
    call("book_close_twice", "book.close", {"handle": book}, {"closed": False})
    call("book_closed_use", "book.search", {"handle": book, "query": "中文"}, error="BOOK_BAD_HANDLE")
    call("txt_close", "txt.close", {"handle": txt}, {"closed": True})
    call("txt_close_twice", "txt.close", {"handle": txt}, {"closed": False})
    call("txt_closed_use", "txt.slice", {"handle": txt, "chars": 1}, error="TXT_BAD_HANDLE")
    call("pipeline_closed_use", "pipeline.status", {"handle": pipe}, error="PIPELINE_BAD_HANDLE")
    # Hold exactly the quota, verify rejection, release, and verify capacity returns.
    for kind, maximum in (("book", 8), ("txt", 4)):
        args = {"bytes_hex": "e4b8ad", "format": "txt"} if kind == "book" else {"path": variable("source")}
        for n in range(maximum):
            call(f"{kind}_quota_{n}", f"{kind}.open", args)
        call(f"{kind}_quota_reject", f"{kind}.open", args, error=f"{kind.upper()}_TOO_MANY_OPEN")
        for n in range(maximum):
            call(f"{kind}_quota_close_{n}", f"{kind}.close", {"handle": capture(f"{kind}_quota_{n}", "handle")}, {"closed": True})
        call(f"{kind}_quota_release", f"{kind}.open", args)
        call(f"{kind}_quota_release_close", f"{kind}.close", {"handle": capture(f"{kind}_quota_release", "handle")}, {"closed": True})
    first_context = capture("context_first_open", "context")
    second_context = capture("context_second_open", "context")
    call("context_first_open", "context.open", expect={"context": 1})
    call("context_second_open", "context.open", expect={"context": 2})
    call("context_first_txt", "txt.open", {"path": variable("source")}, context=first_context)
    call("context_second_txt", "txt.open", {"path": variable("source")}, context=second_context)
    call("context_first_pipeline", "pipeline.open", {"out_dir": variable("context_out_first")},
         {"recovered": []}, context=first_context)
    call("context_second_pipeline", "pipeline.open", {"out_dir": variable("context_out_second")},
         {"recovered": []}, context=second_context)
    call("context_first_pipeline_quota", "pipeline.open", {"out_dir": variable("context_out_extra")},
         error="PIPELINE_TOO_MANY_OPEN", context=first_context)
    call("context_cross_use", "txt.slice", {
        "handle": capture("context_first_txt", "handle"), "chars": 1,
    }, error="CONTEXT_HANDLE_MISMATCH", context=second_context)
    call("context_first_close", "context.close", {"context": first_context}, {
        "closed": True, "released": {"books": 0, "txts": 1, "pipelines": 1},
    })
    call("context_second_slice", "txt.slice", {
        "handle": capture("context_second_txt", "handle"), "chars": 1,
    }, {"text": "中"}, context=second_context)
    call("context_second_pipeline_status", "pipeline.status", {
        "handle": capture("context_second_pipeline", "handle"),
    }, {"idle": True}, context=second_context)
    call("context_closed_call", "version", error="CONTEXT_BAD_HANDLE", context=first_context)
    call("context_first_close_twice", "context.close", {"context": first_context}, {
        "closed": False, "released": {"books": 0, "txts": 0, "pipelines": 0},
    })
    call("context_second_close", "context.close", {"context": second_context}, {
        "closed": True, "released": {"books": 0, "txts": 1, "pipelines": 1},
    })
    call("inspect_path", "txt.inspect", {"path": variable("source")}, {"chars": 17, "lines": 3})
    call("inspect_gbk", "txt.inspect", {"path": variable("gbk"), "encoding": "gbk"},
         {"encoding": "GBK", "certain_encoding": True})
    call("inspect_big5", "txt.inspect", {"path": variable("big5"), "encoding": "big5"},
         {"encoding": "Big5", "certain_encoding": True})
    call("inspect_large", "txt.inspect", {"path": variable("long_line")})
    call("inspect_missing", "txt.inspect", {"path": variable("missing")}, error="TXT_IO")
    call("inspect_encoding", "txt.inspect", {"path": variable("source"), "encoding": "invalid-label"}, error="TXT_UNKNOWN_ENCODING")
    call("refine_path", "jobs.refine", {"path": variable("source"), "format": "txt"})
    call("refine_epub_path", "jobs.refine", {"path": variable("epub"), "format": "epub"})
    call("refine_path_limited", "jobs.refine", {"path": variable("source"), "max_chars": 3}, {"charCount": 3})
    call("refine_path_missing", "jobs.refine", {"path": variable("missing")}, error="JOBS_IO")
    call("book_path_missing", "book.open", {"path": variable("missing")}, error="BOOK_OPEN_IO")
    for method, prefix in [("jobs.refine", "refine"), ("book.open", "book")]:
        for label, args in [("neither", {}), ("both", {"path": variable("source"), "bytes_hex": ""}),
                            ("null_path", {"path": None}), ("null_both", {"path": None, "bytes_hex": "00"})]:
            call(f"{prefix}_path_{label}", method, args, error="FFI_BAD_ARGS", canonical=False)
        call(f"{prefix}_path_bad_epub", method, {"path": variable("source"), "format": "epub"}, error="JOBS_EPUB")
    path_context = capture("path_context", "context")
    call("path_context", "context.open")
    call("book_path", "book.open", {"path": variable("source"), "format": "txt", "chunk_chars": 0}, context=path_context)
    call("book_path_search", "book.search", {"handle": capture("book_path", "handle"), "query": "中文"}, context=path_context)
    call("book_epub_path", "book.open", {"path": variable("epub"), "format": "epub"}, context=path_context)
    call("book_epub_path_search", "book.search", {"handle": capture("book_epub_path", "handle"), "query": "海风"}, context=path_context)
    call("book_path_cross_context", "book.search", {"handle": capture("book_path", "handle"), "query": "中文"}, error="CONTEXT_HANDLE_MISMATCH")
    call("path_context_close", "context.close", {"context": path_context},
         {"closed": True, "released": {"books": 2, "txts": 0, "pipelines": 0}})

    # Append LOC-01 calls so all 189 frozen v1.3 responses retain their handles.
    loc_context = capture("loc_context", "context")

    def loc_call(ident, method, args=None, expect=None, error=None, canonical=True):
        call(ident, method, args, expect, error, canonical, context=loc_context)

    def txt_point(chars, bytes):
        return {"kind": "txt", "charOffset": chars, "byteOffset": bytes}

    loc_handle = capture("loc_txt_open", "handle")
    loc_range = {"handle": loc_handle, "char_start": capture("loc_txt_search", "hits.0.charStart"),
                 "char_end": capture("loc_txt_search", "hits.0.charEnd")}
    call("loc_context", "context.open")
    loc_call("loc_txt_open", "book.open", {"bytes_hex": LOC_TEXT.encode().hex(), "format": "txt", "chunk_chars": 2}, {"charCount": 9})
    loc_call("loc_txt_search", "book.search", {"handle": loc_handle, "query": "乙"})
    loc_call("loc_txt", "book.locate", loc_range,
             {"start": txt_point(9, 11), "end": txt_point(10, 14), "containsSynthetic": False})
    loc_call("loc_txt_emoji", "book.locate", {"handle": loc_handle, "char_start": 4, "char_end": 5},
             {"start": txt_point(10, 14), "end": txt_point(11, 18), "containsSynthetic": False})
    loc_call("loc_txt_combining", "book.locate", {"handle": loc_handle, "char_start": 5, "char_end": 7},
             {"start": txt_point(11, 18), "end": txt_point(13, 21), "containsSynthetic": False})
    loc_call("loc_txt_collapsed", "book.locate", {"handle": loc_handle, "char_start": 2, "char_end": 3},
             {"start": txt_point(5, 7), "end": txt_point(9, 11), "containsSynthetic": False})
    for label, start, end in [("empty", 0, 0), ("reversed", 5, 4), ("past_end", 0, 10),
                               ("negative", -1, 1), ("string", "0", 1), ("boolean", False, 1)]:
        loc_call(f"loc_range_{label}", "book.locate", {"handle": loc_handle, "char_start": start, "char_end": end},
                 error="BOOK_LOCATE_BAD_RANGE", canonical=label not in ("negative", "string", "boolean"))
    loc_call("loc_range_missing", "book.locate", {"handle": loc_handle, "char_start": 0}, error="BOOK_LOCATE_BAD_RANGE", canonical=False)
    call("loc_cross_context", "book.locate", loc_range, error="CONTEXT_HANDLE_MISMATCH")
    loc_call("loc_txt_reader", "txt.open", {"path": variable("loc_source")})
    loc_call("loc_txt_read", "txt.slice", {"handle": capture("loc_txt_reader", "handle"),
             "start": capture("loc_txt", "start.charOffset"), "chars": 1}, {"text": "乙"})
    loc_call("loc_wrong_kind", "book.locate", {"handle": capture("loc_txt_reader", "handle"), "char_start": 0, "char_end": 1}, error="HANDLE_WRONG_KIND")
    loc_call("loc_txt_path_open", "book.open", {"path": variable("loc_source"), "format": "txt", "chunk_chars": 2})
    loc_call("loc_txt_path", "book.locate", {**loc_range, "handle": capture("loc_txt_path_open", "handle")})

    for encoding in ("gbk", "big5"):
        loc_call(f"loc_{encoding}_open", "book.open", {"path": variable(encoding), "format": "txt"})
        loc_call(f"loc_{encoding}_search", "book.search", {"handle": capture(f"loc_{encoding}_open", "handle"), "query": "中文"})
        loc_call(f"loc_{encoding}", "book.locate", {"handle": capture(f"loc_{encoding}_open", "handle"),
            "char_start": capture(f"loc_{encoding}_search", "hits.0.charStart"), "char_end": capture(f"loc_{encoding}_search", "hits.0.charEnd")})
        loc_call(f"loc_{encoding}_close", "book.close", {"handle": capture(f"loc_{encoding}_open", "handle")})

    loc_epub = capture("loc_epub_open", "handle")
    loc_call("loc_epub_open", "book.open", {"path": variable("epub"), "format": "epub"})
    loc_call("loc_epub_search", "book.search", {"handle": loc_epub, "query": "海风"})
    loc_call("loc_epub", "book.locate", {"handle": loc_epub, "char_start": capture("loc_epub_search", "hits.0.charStart"), "char_end": capture("loc_epub_search", "hits.0.charEnd")},
             {"start": {"kind": "epub", "href": "/OEBPS/text/ch01.xhtml", "scalarOffset": 0, "utf16Offset": 0},
              "end": {"kind": "epub", "scalarOffset": 2, "utf16Offset": 2}, "containsSynthetic": False})
    loc_call("loc_epub_cafe_search", "book.search", {"handle": loc_epub, "query": "café"})
    loc_call("loc_epub_utf16", "book.locate", {"handle": loc_epub, "char_start": capture("loc_epub_cafe_search", "hits.0.charStart"), "char_end": capture("loc_epub_cafe_search", "hits.0.charEnd")},
             {"start": {"scalarOffset": 5, "utf16Offset": 6}, "end": {"scalarOffset": 10, "utf16Offset": 11}, "containsSynthetic": False})
    loc_call("loc_epub_synthetic", "book.locate", {"handle": loc_epub, "char_start": 3, "char_end": 4}, error="BOOK_SOURCE_UNMAPPABLE")
    loc_call("loc_epub_inward_start", "book.locate", {"handle": loc_epub, "char_start": 3, "char_end": 6},
             {"start": {"scalarOffset": 0, "utf16Offset": 0}, "end": {"scalarOffset": 2, "utf16Offset": 2}, "containsSynthetic": True})
    loc_call("loc_epub_inward_end", "book.locate", {"handle": loc_epub, "char_start": 0, "char_end": 4},
             {"start": {"scalarOffset": 0}, "end": {"scalarOffset": 3, "utf16Offset": 3}, "containsSynthetic": True})
    loc_call("loc_epub_cross_chapters", "book.locate", {"handle": loc_epub, "char_start": 0, "char_end": capture("loc_epub_open", "charCount")},
             {"start": {"href": "/OEBPS/text/ch01.xhtml"}, "end": {"href": "/OEBPS/text/ch02.xhtml"}, "containsSynthetic": True})

    loc_pipe = capture("loc_pipeline_open", "handle")
    loc_call("loc_pipeline_open", "pipeline.open", {"out_dir": variable("loc_out"), "chunk_chars": 2})
    loc_call("loc_pipeline_enqueue", "pipeline.enqueue", {"handle": loc_pipe, "book": LOC_BOOK, "path": variable("loc_source"), "format": "txt"})
    loc_call("loc_pipeline_refine", "pipeline.step", {"handle": loc_pipe, "now_ms": 1000}, {"outcome": "refined"})
    loc_call("loc_pipeline_index", "pipeline.step", {"handle": loc_pipe, "now_ms": 2000}, {"outcome": "indexed"})
    loc_call("loc_load", "book.load", {"index_path": variable("loc_index"), "text_path": variable("loc_text_json")})
    loc_call("loc_load_search", "book.search", {"handle": capture("loc_load", "handle"), "query": "乙"})
    loc_call("loc_loaded", "book.locate", {**loc_range, "handle": capture("loc_load", "handle")})
    calls.append({"action": "source_map_variants"})
    for variant, error in [("missing_map", "BOOK_SOURCE_MAP_MISSING"), ("future_map", "BOOK_SOURCE_MAP_UNSUPPORTED_VERSION"),
                           ("bad_map", "BOOK_SOURCE_MAP_INVALID"), ("null_map", "BOOK_SOURCE_MAP_INVALID")]:
        loc_call(f"loc_{variant}_load", "book.load", {"index_path": variable("loc_index"), "text_path": variable(variant)})
        loc_call(f"loc_{variant}_search", "book.search", {"handle": capture(f"loc_{variant}_load", "handle"), "query": "乙"})
        loc_call(f"loc_{variant}", "book.locate", {**loc_range, "handle": capture(f"loc_{variant}_load", "handle")}, error=error)
        loc_call(f"loc_{variant}_close", "book.close", {"handle": capture(f"loc_{variant}_load", "handle")})
    loc_call("loc_legacy_load", "book.load", {"index_path": variable("legacy_index"), "text_path": variable("legacy_text")})
    loc_call("loc_legacy_search", "book.search", {"handle": capture("loc_legacy_load", "handle"), "query": "a"})
    loc_call("loc_legacy", "book.locate", {"handle": capture("loc_legacy_load", "handle"), "char_start": 0, "char_end": 1}, error="BOOK_SOURCE_MAP_MISSING")
    loc_call("loc_legacy_close", "book.close", {"handle": capture("loc_legacy_load", "handle")})
    call("loc_context_close", "context.close", {"context": loc_context},
         {"closed": True, "released": {"books": 4, "txts": 1, "pipelines": 1}})
    loc_call("loc_context_closed", "book.locate", loc_range, error="CONTEXT_BAD_HANDLE")
    call("loc_handle_closed", "book.locate", loc_range, error="BOOK_BAD_HANDLE")

    # Append STORE-01 calls so all 256 v1.4 responses retain their handles.
    calls.append({"action": "corrupt_index", "book": BOOK})
    call("pipeline_recovery_open", "pipeline.open", {"out_dir": variable("out")})
    call("pipeline_recovery_state", "pipeline.state", {"handle": capture("pipeline_recovery_open", "handle"),
         "book": BOOK}, {"known": True, "state": "indexing", "attempts": 0})
    call("pipeline_recovery_enqueue", "pipeline.enqueue", {"handle": capture("pipeline_recovery_open", "handle"),
         "book": BOOK, "path": variable("source"), "format": "txt"},
         {"outcome": "queued", "stage": "index", "state": "indexing", "attempts": 0})
    call("pipeline_recovery_indexed", "pipeline.step", {"handle": capture("pipeline_recovery_open", "handle"),
         "now_ms": 1700000000030}, {"outcome": "indexed", "stage": "index"})
    call("pipeline_recovery_close", "pipeline.close", {"handle": capture("pipeline_recovery_open", "handle")}, {"closed": True})
    return {"description": "Replay in one fresh process per client; $var resolves paths/bytes, $ref resolves earlier results. canonical=false intentionally probes runtime permissiveness or malformed input.",
            "source_text": TEXT, "location_text": LOC_TEXT, "calls": calls}
