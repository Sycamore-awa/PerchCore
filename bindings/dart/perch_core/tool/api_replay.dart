// Persistent client for the same method<TAB>JSON protocol as examples/c/replay.c.
import 'dart:convert';
import 'dart:io';

import 'package:perch_core/perch_core.dart';

Future<void> main(List<String> args) async {
  if (args.length != 1) {
    stderr.writeln('Usage: dart run tool/api_replay.dart LIBRARY');
    exitCode = 2;
    return;
  }
  final core = PerchCore.open(libraryPath: args.single);
  await for (final line
      in stdin.transform(utf8.decoder).transform(const LineSplitter())) {
    final tab = line.indexOf('\t');
    if (tab < 0) throw const FormatException('Missing method separator');
    final method = line.substring(0, tab);
    final request = jsonDecode(line.substring(tab + 1)) as Map<String, dynamic>;
    Map<String, dynamic> result;
    try {
      result = core.invoke(method, request);
    } on PerchCoreException catch (error) {
      result = {
        'error': {'code': error.code, 'message': error.message},
      };
    }
    stdout.writeln(jsonEncode(result));
    await stdout.flush();
  }
}
