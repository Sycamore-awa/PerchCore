import 'dart:io';

import 'package:perch_core/perch_core.dart';

Future<void> main(List<String> args) async {
  if (args.isEmpty || args.length > 2) {
    stderr.writeln(
      'usage: dart run example/main.dart <library-path> [book.txt]',
    );
    exitCode = 2;
    return;
  }
  final worker = await PerchWorker.spawn(libraryPath: args[0]);
  final services = PerchServices(worker);
  try {
    stdout.writeln('PerchCore ${(await worker.call('version'))['core']}');
    if (args.length == 2) {
      final document = await services.openTxt(File(args[1]).absolute.path);
      try {
        stdout.writeln((await document.slice(0, chars: 120)).text);
      } finally {
        await document.close();
      }
    }
  } finally {
    await services.close();
  }
}
