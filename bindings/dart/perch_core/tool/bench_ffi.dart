// BASE-02 AOT worker: use the public synchronous and isolate bindings.
import 'dart:convert';
import 'dart:io';

import 'package:perch_core/perch_core.dart';

Future<void> main(List<String> args) async {
  final config =
      jsonDecode(File(args.single).readAsStringSync()) as Map<String, dynamic>;
  final core = PerchCore.open(libraryPath: config['library'] as String);
  final stage = config['stage'] as String;
  final isolated = config['client'] == 'dart-worker';
  final worker = isolated
      ? await PerchWorker.spawn(libraryPath: config['library'] as String)
      : null;
  final bytes = File(config['path'] as String).readAsBytesSync();
  final decodedPath = config['decoded_path'] as String?;
  final decoded = decodedPath == null
      ? null
      : File(decodedPath).readAsStringSync();
  final indexedPath = config['indexed_path'] as String?;
  final indexedChars = indexedPath == null
      ? 0
      : File(indexedPath).readAsStringSync().runes.length;
  Future<Map<String, dynamic>> invoke(
    String method,
    Map<String, dynamic> arguments,
  ) async {
    return worker != null
        ? await worker.call(method, arguments)
        : core.invoke(method, arguments);
  }

  String hex() {
    final buffer = StringBuffer();
    for (final byte in bytes) {
      buffer.write(byte.toRadixString(16).padLeft(2, '0'));
    }
    return buffer.toString();
  }

  int? handle;
  var searchableChars = indexedChars;
  if (stage == 'txt.slice') {
    handle =
        (await invoke('txt.open', {'path': config['path']}))['handle'] as int;
  } else if (stage == 'book.search') {
    final opened = await invoke('book.open', {
      'bytes_hex': hex(),
      'format': config['format'],
    });
    handle = opened['handle'] as int;
    searchableChars = opened['charCount'] as int;
  }
  final sliceStart = config['slice_start'] as int;
  final sliceChars = config['slice_chars'] as int;
  final sliceExpected = decoded == null
      ? null
      : String.fromCharCodes(decoded.runes.skip(sliceStart).take(sliceChars));
  final durations = <int>[];
  Map<String, dynamic> stats = {};
  try {
    final warmups = config['warmups'] as int;
    final iterations = config['iterations'] as int;
    for (var iteration = 0; iteration < warmups + iterations; iteration++) {
      final stopwatch = Stopwatch()..start();
      final arguments = switch (stage) {
        'version' => <String, dynamic>{},
        'txt.open' => {'path': config['path']},
        'txt.slice' => {
          'handle': handle,
          'start': sliceStart,
          'chars': sliceChars,
        },
        'book.search' => {
          'handle': handle,
          'query': config['query'],
          'limit': 20,
        },
        'book.open' || 'txt.analyze' =>
          worker == null
              ? <String, dynamic>{}
              : {
                  'bytes_hex': hex(),
                  'format': config['format'],
                  'chunk_chars': config['chunk_chars'],
                },
        _ => throw StateError('Unknown benchmark stage: $stage'),
      };
      final value = worker == null
          ? switch (stage) {
              'book.open' => core.openBook(
                bytes,
                chunkChars: config['chunk_chars'] as int,
                format: config['format'] as String,
              ),
              'txt.analyze' => core.analyzeText(bytes),
              _ => core.invoke(stage, arguments),
            }
          : await worker.call(stage, arguments);
      stopwatch.stop();
      final elapsed =
          (stopwatch.elapsedTicks * 1000000000 / stopwatch.frequency).round();
      // Scalar checks and releases are outside the timed interval.
      stats = {};
      if (stage == 'txt.open') {
        if (value['chars'] != decoded!.runes.length ||
            value['hadErrors'] != false) {
          throw StateError('TXT open differs from corpus');
        }
        stats['chars'] = value['chars'];
        await invoke('txt.close', {'handle': value['handle']});
      } else if (stage == 'txt.slice') {
        if (value['text'] != sliceExpected)
          throw StateError('Slice differs from corpus');
        stats = {
          'chars': value['chars'],
          'utf8_bytes': utf8.encode(value['text'] as String).length,
        };
      } else if (stage == 'txt.analyze') {
        if (value['text'] != decoded)
          throw StateError('Analysis differs from corpus');
        stats['chars'] = value['chars'];
      } else if (stage == 'book.open') {
        if (value['truncated'] != true && value['charCount'] != indexedChars)
          throw StateError('Book differs from corpus');
        stats = {'chars': value['charCount'], 'truncated': value['truncated']};
        await invoke('book.close', {'handle': value['handle']});
      } else if (stage == 'book.search') {
        final actual = (value['hits'] as List)
            .map((hit) => [hit['charStart'], hit['charEnd']])
            .toList();
        final expected = (config['expected_ranges'] as List)
            .where((span) => (span[1] as int) <= searchableChars)
            .take(20)
            .toList();
        if (jsonEncode(actual) != jsonEncode(expected))
          throw StateError('Search differs from corpus');
        stats = {
          'hits': actual.length,
          'searchable_chars': searchableChars,
          'truncated': searchableChars < indexedChars,
        };
      } else if (value['core'] != core.version()) {
        throw StateError('Version differs');
      }
      if (iteration >= warmups) durations.add(elapsed);
    }
  } finally {
    if (handle != null)
      await invoke(stage == 'txt.slice' ? 'txt.close' : 'book.close', {
        'handle': handle,
      });
    await worker?.dispose();
  }
  int? peak;
  if (Platform.isLinux) {
    final line = File(
      '/proc/self/status',
    ).readAsLinesSync().where((line) => line.startsWith('VmHWM:')).first;
    peak = int.parse(line.split(RegExp(r'\s+'))[1]) * 1024;
  }
  stdout.writeln(
    jsonEncode({
      'durations_ns': durations,
      'stats': stats,
      'peak_rss': {
        'bytes': peak,
        'method': peak == null ? null : '/proc/self/status VmHWM',
        'scope':
            'post-exec Dart process: runtime, setup, warmups, operations, validation and cleanup',
        if (peak == null)
          'reason': 'native peak RSS implemented for Linux /proc only',
      },
    }),
  );
}
