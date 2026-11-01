"""Reviewable calls and independent behavioral assertions for API-01 replay."""

from __future__ import annotations

TEXT = "中文🦀café\r\n第二行\r末尾\n"
BOOK = "00000000-0000-4000-8000-000000000001"
BAD_BOOK = "00000000-0000-4000-8000-000000000002"
NOTE = "00000000-0000-4000-8000-000000000003"
IO_BOOK = "00000000-0000-4000-8000-000000000004"


def variable(name):
    return {"$var": name}


def capture(step, field):
    return {"$ref": f"{step}.{field}"}


def fixtures():
    calls = []

    def call(ident, method, args=None, expect=None, error=None, canonical=True):
        step = {"id": ident, "method": method, "args": args or {}, "canonical": canonical}
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
    call("compatibility", "compatibility", expect={"abi": 1, "api": {"major": 1, "minor": 1},
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
    return {"description": "Replay in one fresh process per client; $var resolves paths/bytes, $ref resolves earlier results. canonical=false intentionally probes runtime permissiveness or malformed input.",
            "source_text": TEXT, "calls": calls}
