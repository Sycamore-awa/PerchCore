#!/usr/bin/env python3
"""BASE-01: self-authored, byte-reproducible EPUB/TXT/PDF fixtures (stdlib only)."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

from make_sample_epub import CONTAINER, epub_bytes

ROOT = Path(__file__).resolve().parents[1]
GENERATOR_VERSION = 1


def digest(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def text_info(text: str, path: str) -> dict:
    content = text.encode("utf-8")
    return {
        "path": path,
        "chars": len(text),
        "utf8_bytes": len(content),
        "sha256": digest(content),
    }


def normalized_txt(text: str) -> str:
    # Expected search text: normalize line endings, trim outer blank lines,
    # retain at most one blank line and preserve inline whitespace.
    lines = re.split(r"\r\n|\r|\n", text)
    kept: list[str] = []
    blank = False
    for line in lines:
        if not line.strip():
            blank = bool(kept)
        else:
            if blank:
                kept.append("")
            kept.append(line)
            blank = False
    return "\n".join(kept)


def query_info(text: str, query: str, chunk_chars: int = 4096) -> dict:
    starts = []
    start = text.find(query)
    while start >= 0:
        starts.append(start)
        start = text.find(query, start + 1)
    return {
        "query": query,
        "chunk_chars": chunk_chars,
        "ranges": [[start, start + len(query)] for start in starts],
    }


def make_epub(version: str, scale: int) -> tuple[bytes, str, list[dict]]:
    title = f"PerchCore EPUB {version} 样本"
    chapters = [
        ("第一章", ["海风 🦀 cafe\u0301 与中文。" * (32 * scale), "跨章节"]),
        ("检索接续", ["English Phrase 与定位 🧭。" * (32 * scale), "结束。"]),
    ]
    manifest = []
    spine = []
    entries: list[tuple[str, str | bytes]] = [("META-INF/container.xml", CONTAINER)]
    text = ""
    spans = []
    for number, (heading, paragraphs) in enumerate(chapters, 1):
        name = f"text/ch{number:02}.xhtml"
        manifest.append(
            f'<item id="ch{number}" href="{name}" media-type="application/xhtml+xml"/>'
        )
        spine.append(f'<itemref idref="ch{number}"/>')
        # No indentation text nodes; title and paragraphs have explicit expectations.
        body = "".join(f"<p>{paragraph}</p>" for paragraph in paragraphs)
        xhtml = (
            '<html xmlns="http://www.w3.org/1999/xhtml">'
            f"<head><title>{heading}</title></head><body>{body}</body></html>"
        )
        entries.append((f"OEBPS/{name}", xhtml))
        start = len(text)
        text += "\n".join([heading, *paragraphs]) + "\n"
        spans.append({"href": f"/OEBPS/{name}", "start_char": start, "end_char": len(text)})

    if version == "2.0":
        manifest.append('<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>')
        navigation = (
            '<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">'
            '<head><meta name="dtb:uid" content="urn:perch:base-01:epub2"/></head>'
            f"<docTitle><text>{title}</text></docTitle><navMap>"
            + "".join(
                f'<navPoint id="n{i}" playOrder="{i}"><navLabel><text>{heading}</text>'
                f'</navLabel><content src="text/ch{i:02}.xhtml"/></navPoint>'
                for i, (heading, _) in enumerate(chapters, 1)
            )
            + "</navMap></ncx>"
        )
        entries.append(("OEBPS/toc.ncx", navigation))
        spine_attr = ' toc="ncx"'
        modified = ""
    else:
        manifest.append('<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>')
        navigation = (
            '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">'
            '<head><title>目录</title></head><body><nav epub:type="toc"><ol>'
            + "".join(
                f'<li><a href="text/ch{i:02}.xhtml">{heading}</a></li>'
                for i, (heading, _) in enumerate(chapters, 1)
            )
            + "</ol></nav></body></html>"
        )
        entries.append(("OEBPS/nav.xhtml", navigation))
        spine_attr = ""
        modified = '<meta property="dcterms:modified">2026-11-01T00:00:00Z</meta>'

    package = (
        f'<package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="bookid">'
        '<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">'
        f'<dc:identifier id="bookid">urn:perch:base-01:epub{version[0]}</dc:identifier>'
        f"<dc:title>{title}</dc:title><dc:creator>PerchCore</dc:creator>"
        f"<dc:language>zh</dc:language>{modified}</metadata>"
        f'<manifest>{"".join(manifest)}</manifest><spine{spine_attr}>{"".join(spine)}</spine></package>'
    )
    entries.insert(1, ("OEBPS/content.opf", package))
    return epub_bytes(entries), text, spans


def pdf_string(text: str) -> bytes:
    return b"<" + (b"\xfe\xff" + text.encode("utf-16-be")).hex().encode("ascii") + b">"


def make_pdf() -> bytes:
    # Three blank pages, Info metadata and a two-level outline. Classic xref
    # uses byte offsets; no clocks, random IDs, fonts or external PDF tool.
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        *[b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << >> /Contents 11 0 R >>"] * 3,
        b"<< /Type /Outlines /First 7 0 R /Last 9 0 R /Count 3 >>",
        b"<< /Title " + pdf_string("第一章") + b" /Parent 6 0 R /Dest [3 0 R /Fit] /Next 9 0 R /First 8 0 R /Last 8 0 R /Count 1 >>",
        b"<< /Title " + pdf_string("第一节 🧭") + b" /Parent 7 0 R /Dest [4 0 R /Fit] >>",
        b"<< /Title " + pdf_string("第二章") + b" /Parent 6 0 R /Dest [5 0 R /Fit] /Prev 7 0 R >>",
        b"<< /Title " + pdf_string("PerchCore PDF 结构样本") + b" /Author (PerchCore) /Producer (BASE-01) >>",
        b"<< /Length 0 >>\nstream\n\nendstream",
    ]
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, content in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n".encode("ascii") + content + b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode("ascii"))
    for offset in offsets[1:]:
        output.extend(f"{offset:010} 00000 n \n".encode("ascii"))
    output.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R /Info 10 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode("ascii")
    )
    return bytes(output)


def build_corpus(profile: str, scale: int) -> dict[str, bytes]:
    files: dict[str, bytes] = {}
    samples = []

    def add(sample_id: str, extension: str, content: bytes, **metadata) -> dict:
        path = f"inputs/{sample_id}.{extension}"
        files[path] = content
        sample = {"id": sample_id, "path": path, "bytes": len(content), "sha256": digest(content), **metadata}
        samples.append(sample)
        return sample

    def add_text(sample: dict, key: str, text: str) -> None:
        path = f'texts/{sample["id"]}.{key}.txt'
        files[path] = text.encode("utf-8")
        sample[key] = text_info(text, path)

    mixed = "中文与海风 🦀🧭，cafe\u0301，English Phrase。\r\n第二行\r第三行\n\n \t\n末行。\n"
    gbk = "中文编码测试，海风吹过书页，读者记录文字与位置。English Phrase。\r\n"
    big5 = "繁體中文編碼測試，海風吹過書頁，讀者記錄文字與位置。English Phrase。\r\n"
    txt_cases = [
        ("utf8", "UTF-8", mixed * (16 * scale), False, ["中文", "English Phrase", "海风 🦀🧭"]),
        ("utf8-bom", "UTF-8", mixed * (16 * scale), True, ["中文", "海风 🦀🧭"]),
        ("gbk", "GBK", gbk * (16 * scale), False, ["中文编码测试", "English Phrase"]),
        ("big5", "Big5", big5 * (16 * scale), False, ["繁體中文編碼測試", "English Phrase"]),
        ("cross-shard", "UTF-8", ("序" * 4094 + "跨片检索🧭\n") * scale, False, ["跨片检索", "检索🧭"]),
        ("long-line", "UTF-8", "长行中文🧭cafe\u0301 " * (2048 * scale) + "终点", False, ["终点"]),
    ]
    for sample_id, encoding, text, bom, queries in txt_cases:
        content = (b"\xef\xbb\xbf" if bom else b"") + text.encode(encoding.lower())
        indexed = normalized_txt(text)
        sample = add(sample_id, "txt", content, format="txt", encoding=encoding, bom=bom)
        add_text(sample, "decoded", text)
        add_text(sample, "indexed", indexed)
        sample["queries"] = [query_info(indexed, query) for query in queries]
        if sample_id in ["utf8", "utf8-bom"]:
            # Current index ignores standalone emoji/punctuation during tokenization.
            sample["queries"].append({"query": "🦀🧭", "chunk_chars": 4096, "ranges": [], "reason": "no_indexable_tokens"})

    for version in ["2.0", "3.0"]:
        content, text, spans = make_epub(version, scale)
        sample = add(f"epub{version[0]}", "epub", content, format="epub", encoding="UTF-8", epub_version=version)
        add_text(sample, "indexed", text)
        sample["chapters"] = spans
        sample["title"] = f"PerchCore EPUB {version} 样本"
        sample["toc"] = ["第一章", "检索接续"]
        sample["queries"] = [query_info(text, "跨章节\n检索接续"), query_info(text, "English Phrase")]

    add("pdf-outline", "pdf", make_pdf(), format="pdf", encoding="PDF strings: UTF-16BE / ASCII", text_chars=None,
        title="PerchCore PDF 结构样本", page_count=3, outline=[
            {"title": "第一章", "page_index": 0, "children": [{"title": "第一节 🧭", "page_index": 1}]},
            {"title": "第二章", "page_index": 2},
        ])
    manifest = {
        "version": 1,
        "generator": "tools/make_corpus.py",
        "generator_version": GENERATOR_VERSION,
        "profile": profile,
        "scale": scale,
        "license": "MIT",
        "character_unit": "Unicode scalar",
        "samples": samples,
    }
    files["manifest.json"] = (json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def check_corpus(destination: Path, expected: dict[str, bytes]) -> list[str]:
    problems = []
    for path, content in expected.items():
        actual = destination / path
        if not actual.is_file():
            problems.append(f"missing: {path}")
        elif actual.read_bytes() != content:
            problems.append(f"content mismatch: {path}")
    for directory in ["inputs", "texts"]:
        for actual in sorted((destination / directory).rglob("*")):
            if actual.is_file() and actual.relative_to(destination).as_posix() not in expected:
                problems.append(f"unexpected: {actual.relative_to(destination).as_posix()}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=["correctness", "performance"], default="correctness")
    parser.add_argument("--scale", type=int, help="repetition multiplier; correctness=1, performance default=256")
    parser.add_argument("--output", type=Path, help="default: examples/data/corpus or target/performance-corpus")
    parser.add_argument("--check", action="store_true", help="rebuild in memory and compare without writing")
    args = parser.parse_args()
    scale = args.scale if args.scale is not None else (1 if args.profile == "correctness" else 256)
    if scale < 1 or (args.profile == "correctness" and scale != 1):
        parser.error("scale must be positive; correctness requires scale=1 (use --profile performance for larger files)")
    destination = args.output if args.output is not None else ROOT / (
        "examples/data/corpus" if args.profile == "correctness" else "target/performance-corpus"
    )
    try:
        files = build_corpus(args.profile, scale)
        if args.check:
            problems = check_corpus(destination, files)
            if problems:
                print("\n".join(problems), file=sys.stderr)
                return 1
        else:
            for path, content in files.items():
                target = destination / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(content)
    except OSError as error:
        print(f"corpus I/O error: {error}", file=sys.stderr)
        return 1
    action = "Verified" if args.check else "Generated"
    print(f"{action} {len(files)} files ({args.profile}, scale={scale}) at {destination}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
