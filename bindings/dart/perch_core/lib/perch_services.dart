import 'dart:async';

import 'perch_core.dart';

enum ImportState { registered, refining, indexing, ready, failed }

enum PipelineStage {
  refine,
  indexing;

  static PipelineStage parse(String value) =>
      value == 'index' ? indexing : values.byName(value);
  String get wireName => this == indexing ? 'index' : name;
}

enum PipelinePriority { urgent, normal, background }

enum EnqueueResult { queued, alreadyPresent, notNeeded }

class PipelinePolicy {
  const PipelinePolicy({
    this.maxRefine = 2,
    this.maxIndex = 1,
    this.maxAttempts = 3,
    this.backoffBaseMs = 2000,
    this.backoffMaxMs = 60000,
  });
  final int maxRefine, maxIndex, maxAttempts, backoffBaseMs, backoffMaxMs;
  Map<String, dynamic> toJson() => {
    'maxRefine': maxRefine,
    'maxIndex': maxIndex,
    'maxAttempts': maxAttempts,
    'backoffBaseMs': backoffBaseMs,
    'backoffMaxMs': backoffMaxMs,
  };
}

class PipelineGate {
  const PipelineGate({this.allowAny = true, this.allowHeavy = true});
  const PipelineGate.suspended() : allowAny = false, allowHeavy = false;
  final bool allowAny, allowHeavy;
  Map<String, dynamic> toJson() => {
    'allowAny': allowAny,
    'allowHeavy': allowHeavy,
  };
}

class PipelineStats {
  const PipelineStats({
    this.pendingRefine = 0,
    this.pendingIndex = 0,
    this.inFlight = 0,
    this.exhausted = 0,
  });
  factory PipelineStats.fromJson(Map<String, dynamic> json) => PipelineStats(
    pendingRefine: json['pendingRefine'] as int,
    pendingIndex: json['pendingIndex'] as int,
    inFlight: json['inFlight'] as int,
    exhausted: json['exhausted'] as int,
  );
  final int pendingRefine, pendingIndex, inFlight, exhausted;
  bool get hasWork => pendingRefine + pendingIndex + inFlight > 0;
}

class PipelineRecord {
  const PipelineRecord({
    this.book,
    required this.state,
    this.attempts = 0,
    this.failedStage,
    this.reason,
    this.retryAtMs,
  });
  factory PipelineRecord.fromJson(Map<String, dynamic> json) => PipelineRecord(
    book: json['book'] as String?,
    state: ImportState.values.byName(json['state'] as String),
    attempts: json['attempts'] as int? ?? 0,
    failedStage: json['failedStage'] == null
        ? null
        : PipelineStage.parse(json['failedStage'] as String),
    reason: json['reason'] as String?,
    retryAtMs: json['retryAtMs'] as int?,
  );
  final String? book, reason;
  final ImportState state;
  final int attempts;
  final PipelineStage? failedStage;
  final int? retryAtMs;
  Map<String, dynamic> toJson() => {
    'state': state.name,
    'attempts': attempts,
    'failedStage': failedStage?.wireName,
    'reason': reason,
    'retryAtMs': retryAtMs,
  };
}

sealed class StepOutcome {
  const StepOutcome(this.stats, [this.book]);
  final PipelineStats stats;
  final String? book;
  factory StepOutcome.fromJson(Map<String, dynamic> json) {
    final stats = PipelineStats.fromJson(
      (json['stats'] as Map).cast<String, dynamic>(),
    );
    final book = json['book'] as String?;
    return switch (json['outcome']) {
      'idle' => StepIdle(stats),
      'refined' => StepRefined(stats, book!),
      'indexed' => StepIndexed(
        stats,
        book!,
        json['chunks'] as int,
        json['tokens'] as int,
      ),
      'failed' => StepFailed(
        stats,
        book!,
        PipelineStage.parse(json['stage'] as String),
        json['attempts'] as int,
        json['reason'] as String,
        json['retryAtMs'] as int,
      ),
      'exhausted' => StepExhausted(
        stats,
        book!,
        PipelineStage.parse(json['stage'] as String),
        json['attempts'] as int,
        json['reason'] as String,
      ),
      _ => throw FormatException('Unknown step outcome: ${json['outcome']}'),
    };
  }
}

final class StepIdle extends StepOutcome {
  const StepIdle(super.stats);
}

final class StepRefined extends StepOutcome {
  const StepRefined(super.stats, String super.book);
}

final class StepIndexed extends StepOutcome {
  const StepIndexed(super.stats, String super.book, this.chunks, this.tokens);
  final int chunks, tokens;
}

final class StepFailed extends StepOutcome {
  const StepFailed(
    super.stats,
    String super.book,
    this.stage,
    this.attempts,
    this.reason,
    this.retryAtMs,
  );
  final PipelineStage stage;
  final int attempts, retryAtMs;
  final String reason;
}

final class StepExhausted extends StepOutcome {
  const StepExhausted(
    super.stats,
    String super.book,
    this.stage,
    this.attempts,
    this.reason,
  );
  final PipelineStage stage;
  final int attempts;
  final String reason;
}

// I/O remains the stable FFI error channel; this model is available to service consumers.
final class StepIoFailed extends StepOutcome {
  const StepIoFailed(super.stats, this.error);
  final PerchCoreException error;
}

class PipelineStatus {
  const PipelineStatus(this.idle, this.stats);
  final bool idle;
  final PipelineStats stats;
}

class PipelineEnqueued {
  PipelineEnqueued(Map<String, dynamic> json)
    : result = EnqueueResult.values.byName(json['outcome'] as String),
      state = ImportState.values.byName(json['state'] as String),
      attempts = json['attempts'] as int,
      stage = json['stage'] == null
          ? null
          : PipelineStage.parse(json['stage'] as String);
  final EnqueueResult result;
  final ImportState state;
  final int attempts;
  final PipelineStage? stage;
}

class FileFingerprint {
  const FileFingerprint(this.value, this.size);
  final String value;
  final int size;
}

class BookSearchHit {
  const BookSearchHit({
    required this.charStart,
    required this.charEnd,
    required this.snippet,
    this.locator,
  });
  factory BookSearchHit.fromJson(Map<String, dynamic> json) => BookSearchHit(
    charStart: json['charStart'] as int,
    charEnd: json['charEnd'] as int,
    snippet: json['snippet'] as String,
    locator: json['locator'] is Map
        ? (json['locator'] as Map).cast<String, dynamic>()
        : null,
  );
  final int charStart, charEnd;
  final String snippet;
  final Map<String, dynamic>? locator;
}

/// One owner per App session. All handles use this worker and close before it exits.
class PerchServices {
  PerchServices(this.worker);
  final PerchWorker worker;
  final Set<_AsyncHandle> _handles = {};
  bool _closing = false;
  Future<void>? _shutdown;
  void _ensureOpen() {
    if (_closing) throw StateError('核心服务正在关闭');
  }

  Future<FileFingerprint> fingerprintPath(String path) async {
    _ensureOpen();
    final r = await worker.call('worker.fingerprintPath', {'path': path});
    return FileFingerprint(r['fingerprint'] as String, r['size'] as int);
  }

  Future<String> fingerprint({
    required int size,
    required List<int> head,
    required List<int> tail,
  }) async {
    _ensureOpen();
    String hex(List<int> b) =>
        b.map((n) => n.toRadixString(16).padLeft(2, '0')).join();
    return (await worker.call('fingerprint', {
          'size': size,
          'head_hex': hex(head),
          'tail_hex': hex(tail),
        }))['fingerprint']
        as String;
  }

  Future<Map<String, dynamic>> analyzePdf(String path) {
    _ensureOpen();
    return worker.call('pdf.analyze', {'path': path});
  }

  Future<PipelineHandle> openPipeline(
    String outDir, {
    PipelinePolicy policy = const PipelinePolicy(),
    int? maxChars,
    int? chunkChars,
  }) async {
    _ensureOpen();
    final r = await worker.call('pipeline.open', {
      'out_dir': outDir,
      'policy': policy.toJson(),
      if (maxChars != null) 'max_chars': maxChars,
      if (chunkChars != null) 'chunk_chars': chunkChars,
    });
    final h = PipelineHandle._(
      this,
      r['handle'] as int,
      (r['recovered'] as List)
          .map(
            (e) => PipelineRecord.fromJson((e as Map).cast<String, dynamic>()),
          )
          .toList(),
    );
    _handles.add(h);
    return h;
  }

  Future<AsyncTxtHandle> openTxt(String path, {String? encoding}) async {
    _ensureOpen();
    final r = await worker.call('txt.open', {
      'path': path,
      if (encoding != null) 'encoding': encoding,
    });
    final h = AsyncTxtHandle._(this, r);
    _handles.add(h);
    return h;
  }

  Future<AsyncBookHandle> loadBook({
    required String indexPath,
    required String textPath,
  }) async {
    _ensureOpen();
    final r = await worker.call('book.load', {
      'index_path': indexPath,
      'text_path': textPath,
    });
    final h = AsyncBookHandle._(this, r);
    _handles.add(h);
    return h;
  }

  Future<void> close() => _shutdown ??= _close();
  Future<void> _close() async {
    _closing = true;
    // A barrier lets accepted open calls register their handles before closing.
    try {
      await worker.call('version');
    } finally {
      try {
        for (final handle in _handles.toList()) {
          await handle.close();
        }
      } finally {
        await worker.dispose();
      }
    }
  }
}

abstract class _AsyncHandle {
  _AsyncHandle(this.owner, this.handle, this.kind);
  final PerchServices owner;
  final int handle;
  final String kind;
  Future<bool>? _closing;
  Future<Map<String, dynamic>> _call(
    String method, [
    Map<String, dynamic> args = const {},
  ]) {
    if (_closing != null || owner._closing)
      return Future.error(StateError('$kind 句柄已关闭'));
    return owner.worker.call('$kind.$method', {'handle': handle, ...args});
  }

  Future<bool> close() => _closing ??= _close();
  Future<bool> _close() async {
    final r = await owner.worker.call('$kind.close', {'handle': handle});
    owner._handles.remove(this);
    return r['closed'] as bool;
  }
}

class PipelineHandle extends _AsyncHandle {
  PipelineHandle._(PerchServices owner, int handle, this.recovered)
    : super(owner, handle, 'pipeline');
  final List<PipelineRecord> recovered;
  Future<PipelineEnqueued> enqueue({
    required String book,
    required String path,
    required String format,
    ImportState state = ImportState.registered,
    PipelinePriority priority = PipelinePriority.normal,
  }) async => PipelineEnqueued(
    await _call('enqueue', {
      'book': book,
      'path': path,
      'format': format,
      'state': state.name,
      'priority': priority.name,
    }),
  );
  Future<StepOutcome> step({
    int? nowMs,
    PipelineGate gate = const PipelineGate(),
  }) async {
    try {
      return StepOutcome.fromJson(
        await _call('step', {
          if (nowMs != null) 'now_ms': nowMs,
          'gate': gate.toJson(),
        }),
      );
    } on PerchCoreException catch (error) {
      if (error.code != 'PIPELINE_IO') rethrow;
      return StepIoFailed((await status()).stats, error);
    }
  }

  Future<PipelineStatus> status() async {
    final r = await _call('status');
    return PipelineStatus(
      r['idle'] as bool,
      PipelineStats.fromJson((r['stats'] as Map).cast<String, dynamic>()),
    );
  }

  Future<PipelineRecord?> state(String book) async {
    final r = await _call('state', {'book': book});
    return r['known'] == true
        ? PipelineRecord.fromJson({...r, 'book': book})
        : null;
  }

  Future<void> promote(
    String book, {
    PipelinePriority priority = PipelinePriority.urgent,
  }) async {
    await _call('promote', {'book': book, 'priority': priority.name});
  }

  Future<bool> retry(
    String book, {
    PipelinePriority priority = PipelinePriority.normal,
  }) async =>
      (await _call('retry', {
            'book': book,
            'priority': priority.name,
          }))['retried']
          as bool;
  Future<void> cancel(String book) async {
    await _call('cancel', {'book': book});
  }
}

class AsyncTxtHandle extends _AsyncHandle {
  AsyncTxtHandle._(PerchServices owner, Map<String, dynamic> r)
    : chars = r['chars'] as int,
      lines = r['lines'] as int,
      encoding = r['encoding'] as String,
      certain = r['certain'] as bool? ?? false,
      hadErrors = r['hadErrors'] as bool? ?? false,
      super(owner, r['handle'] as int, 'txt');
  final int chars, lines;
  final String encoding;
  final bool certain, hadErrors;
  Future<TxtSlice> slice(int start, {int chars = 800}) async {
    final r = await _call('slice', {'start': start, 'chars': chars});
    return TxtSlice(
      text: r['text'] as String,
      start: r['start'] as int,
      end: r['end'] as int,
      total: r['total'] as int,
    );
  }

  Future<Map<String, dynamic>> locate(int offset) async =>
      ((await _call('locate', {'char': offset}))['locator'] as Map)
          .cast<String, dynamic>();
  Future<int?> offsetOf(Map<String, dynamic> locator) async =>
      (await _call('offset', {'locator': locator}))['char'] as int?;
  Future<TxtLine> lineAt(int offset) async {
    final r = await _call('line', {'char': offset});
    return TxtLine(
      line: r['line'] as int,
      start: r['start'] as int?,
      end: r['end'] as int?,
      totalLines: r['totalLines'] as int,
    );
  }
}

class AsyncBookHandle extends _AsyncHandle {
  AsyncBookHandle._(PerchServices owner, Map<String, dynamic> r)
    : charCount = r['charCount'] as int,
      chapterCount = r['chapterCount'] as int,
      chunks = r['chunks'] as int,
      tokens = r['tokens'] as int,
      truncated = r['truncated'] as bool,
      super(owner, r['handle'] as int, 'book');

  final int charCount, chapterCount, chunks, tokens;
  final bool truncated;

  Future<List<BookSearchHit>> search(String query, {int limit = 50}) async {
    final r = await _call('search', {'query': query, 'limit': limit});
    return (r['hits'] as List)
        .map(
          (hit) => BookSearchHit.fromJson((hit as Map).cast<String, dynamic>()),
        )
        .toList(growable: false);
  }
}
