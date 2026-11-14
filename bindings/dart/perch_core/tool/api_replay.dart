// Persistent client for context<TAB>method<TAB>JSON, shared with C replay.
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
    final contextTab = line.indexOf('\t');
    final methodTab = line.indexOf('\t', contextTab + 1);
    if (contextTab < 0 || methodTab < 0) {
      throw const FormatException('Missing context/method separator');
    }
    final contextText = line.substring(0, contextTab);
    final method = line.substring(contextTab + 1, methodTab);
    final request =
        jsonDecode(line.substring(methodTab + 1)) as Map<String, dynamic>;
    Map<String, dynamic> result;
    try {
      result = contextText.isEmpty
          ? core.invoke(method, request)
          : core.invokeInContext(
              PerchContext(int.parse(contextText)),
              method,
              request,
            );
    } on PerchCoreException catch (error) {
      result = {
        'error': {'code': error.code, 'message': error.message},
      };
    }
    stdout.writeln(jsonEncode(result));
    await stdout.flush();
  }
}
