import 'dart:io';
import 'dart:math';

import 'package:perch_core/perch_core.dart';

/// A host-side reader integration flow with no Flutter UI:
/// import -> restart/recover -> load -> search -> source jump -> reading slice.
Future<void> main(List<String> args) async {
  if (args.length < 2 || args.length > 4) {
    stderr.writeln(
      'usage: dart run example/reader_flow.dart '
      '<library-path> <book.txt|book.epub> [query] [artifact-dir]',
    );
    exitCode = 2;
    return;
  }

  final libraryPath = File(args[0]).absolute.path;
  final source = File(args[1]).absolute;
  final query = args.length >= 3 ? args[2] : '文字';
  if (!File(libraryPath).existsSync()) {
    throw ArgumentError('动态库不存在: $libraryPath');
  }
  if (!source.existsSync()) throw ArgumentError('书籍不存在: ${source.path}');
  final format = switch (source.path.toLowerCase()) {
    final path when path.endsWith('.epub') => 'epub',
    final path when path.endsWith('.txt') => 'txt',
    _ => throw ArgumentError('reader_flow 只接受 EPUB/TXT'),
  };
  final temporaryArtifacts = args.length < 4;
  final artifacts =
      temporaryArtifacts
            ? Directory.systemTemp.createTempSync('perch-reader-flow-')
            : Directory(args[3]).absolute
        ..createSync(recursive: true);
  final book = _uuidV4();

  try {
    stdout.writeln('[1/4] 导入 $format: ${source.path}');
    final first = PerchServices(
      await PerchWorker.spawn(libraryPath: libraryPath),
    );
    try {
      final pipeline = await first.openPipeline(artifacts.path);
      final queued = await pipeline.enqueue(
        book: book,
        path: source.path,
        format: format,
      );
      if (queued.result != EnqueueResult.queued) {
        throw StateError('新书未入队: ${queued.result.name}');
      }
      await _runToReady(pipeline, book);
      await pipeline.close();
    } finally {
      await first.close();
    }

    stdout.writeln('[2/4] 模拟 App 重启并恢复 Ready 检查点');
    final second = PerchServices(
      await PerchWorker.spawn(libraryPath: libraryPath),
    );
    try {
      final pipeline = await second.openPipeline(artifacts.path);
      final recovered = pipeline.recovered
          .where((record) => record.book == book)
          .single;
      if (recovered.state != ImportState.ready) {
        throw StateError(
          '恢复状态不是 ready: ${recovered.state.name} ${recovered.reason ?? ''}',
        );
      }

      stdout.writeln('[3/4] 加载落盘索引并搜索: $query');
      final bookDir = Directory('${artifacts.path}/$book');
      final opened = await second.loadBook(
        indexPath: '${bookDir.path}/index.bin',
        textPath: '${bookDir.path}/text.json',
      );
      try {
        final hits = await opened.search(query, limit: 1);
        if (hits.isEmpty) throw StateError('查询未命中: $query');
        final hit = hits.single;
        final location = await opened.locate(hit);
        stdout.writeln('  snippet: ${hit.snippet}');

        stdout.writeln('[4/4] 回到阅读源位置');
        await _printSource(second, source.path, location);
      } finally {
        await opened.close();
        await pipeline.close();
      }
    } finally {
      await second.close();
    }

    stdout.writeln('阅读器宿主流程验证通过。');
  } finally {
    if (temporaryArtifacts) artifacts.deleteSync(recursive: true);
  }
}

Future<void> _runToReady(PipelineHandle pipeline, String book) async {
  final base = DateTime.now().millisecondsSinceEpoch;
  for (var step = 0; step < 8; step++) {
    final outcome = await pipeline.step(nowMs: base + step);
    switch (outcome) {
      case StepRefined():
        stdout.writeln('  refinement ready');
      case StepIndexed(:final chunks, :final tokens):
        stdout.writeln('  index ready: $chunks chunks / $tokens tokens');
      case StepCancelled(:final stage):
        throw StateError('导入已取消: ${stage.wireName}');
      case StepFailed(:final reason):
        throw StateError('导入失败: $reason');
      case StepExhausted(:final reason):
        throw StateError('导入重试耗尽: $reason');
      case StepIoFailed(:final error):
        throw error;
      case StepIdle():
        break;
    }
    if ((await pipeline.state(book))?.state == ImportState.ready) return;
  }
  throw StateError('导入未在预期步数内进入 ready');
}

Future<void> _printSource(
  PerchServices services,
  String sourcePath,
  BookSourceLocation location,
) async {
  switch ((location.start, location.end)) {
    case (TxtSourceEndpoint start, TxtSourceEndpoint end):
      final reader = await services.openTxt(sourcePath);
      try {
        final slice = await reader.slice(
          start.charOffset,
          chars: end.charOffset - start.charOffset,
        );
        stdout.writeln(
          '  TXT chars ${start.charOffset}..${end.charOffset}, '
          'UTF-8 bytes ${start.byteOffset}..${end.byteOffset}: ${slice.text}',
        );
      } finally {
        await reader.close();
      }
    case (EpubSourceEndpoint start, EpubSourceEndpoint end):
      stdout.writeln(
        '  EPUB ${start.href}${start.nodePath}:${start.utf16Offset} -> '
        '${end.href}${end.nodePath}:${end.utf16Offset}',
      );
    default:
      throw StateError('源位置端点类型不一致');
  }
}

String _uuidV4() {
  final random = Random.secure();
  final bytes = List<int>.generate(16, (_) => random.nextInt(256));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  String hex(int start, int end) => bytes
      .sublist(start, end)
      .map((value) => value.toRadixString(16).padLeft(2, '0'))
      .join();
  return '${hex(0, 4)}-${hex(4, 6)}-${hex(6, 8)}-'
      '${hex(8, 10)}-${hex(10, 16)}';
}
