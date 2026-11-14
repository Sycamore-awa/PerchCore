// 把核心调用挪到后台 isolate。
//
// # 为什么必须要有这一层
//
// [`PerchCore.invoke`] 是**同步** FFI：调一次就占着调用线程直到 Rust 返回。
// 而有些调用很贵——`txt.open` 要读文件 + 解码（chardetng 探测）+ 建字符索引与行索引，
// 10MB 的 TXT 是几百毫秒量级；`book.open` 还要建倒排索引。
// 直接放在 `build` / `initState` 里，UI isolate 会被堵死：掉帧、手势无响应、
// 连 loading 动画都转不起来。
//
// # 契约（docs/architecture.md「FFI 契约」）
//
// 一个**常驻** worker isolate，Dart 侧只保留
// `Future<Map<String, dynamic>> call(String method, Map args)`。
// 所有超过 ~10ms 的工作排队过去，UI 侧统一 `await` + 骨架态。
//
// # 几个刻意的设计
//
// - **普通请求是串行的**：worker 里处理消息的函数是同步的，一条没处理完不会看下一条。
//   这正是我们要的——句柄表在 Rust 侧是一把互斥锁，串行执行让"打开→切片→关闭"
//   的顺序天然正确，也不会出现并发抢配额。
// - **管线控制走独立短调用**：`pipeline.cancel` 和 `pipeline.progress` 可由临时
//   控制 isolate 送达。Rust 侧取消不等待执行锁，进度查询只读独立的活动快照。
// - **context 创建也可回收**：由受跟踪的短生命周期 isolate 创建，不依赖
//   常驻 worker 回传编号；worker 启动期异常退出后 `dispose` 仍能成组回收。
// - **错误跨 isolate 重建**：Rust 的业务错误在 worker 里被拆成
//   `{code, message}` 再送回来，主 isolate 重新抛 [`PerchCoreException`]——
//   这样调用方捕获的类型和直接调 [`PerchCore`] 时完全一样。
// - **worker 死了不会挂住调用方**：监听 isolate 的 error/exit，
//   一旦异常退出，所有在飞的请求立刻以 `StateError` 失败，而不是永远等待。
//
// # 用法
//
// ```dart
// final worker = await PerchWorker.spawn();          // iOS 不用传路径
// final info = await worker.call('txt.open', {'path': path});
// final handle = info['handle'] as int;
// final slice = await worker.call('txt.slice', {'handle': handle, 'start': 0, 'chars': 800});
// await worker.call('txt.close', {'handle': handle});
// await worker.dispose();
// ```

import 'dart:async';
import 'dart:isolate';
import 'dart:io';

import 'perch_core.dart';

/// 后台 isolate 里的核心门面。
///
/// 生命周期：`spawn` 一次，长期复用，退出前 `dispose`。
class PerchWorker {
  PerchWorker._(
    this._isolate,
    this._commands,
    this._responses,
    this._errors,
    this._exits,
    this._libraryPath,
  );

  final Isolate _isolate;
  final SendPort _commands;
  final ReceivePort _responses;

  /// 这两个端口要留住并显式关闭：**开着它们，Dart VM 不会退出**
  /// （事件循环永远有一个活着的订阅）。第一版就是这么让调用方"调完了不结束"的。
  final ReceivePort _errors;
  final ReceivePort _exits;
  final String? _libraryPath;

  final _failures = StreamController<String>.broadcast();
  Stream<String> get failures => _failures.stream;
  bool get failed => _failure != null;

  final Map<int, Completer<Map<String, dynamic>>> _pending = {};
  final Set<Future<Map<String, dynamic>>> _controls = {};
  final Set<Future<PerchContext>> _contextOpens = {};
  final Set<int> _contexts = {};
  final Set<Future<void>> _contextCleanups = {};
  int _nextId = 0;
  bool _disposed = false;
  bool _disposing = false;
  String? _failure;
  Future<void>? _shutdown;
  final Completer<void> _exit = Completer<void>();

  /// 拉起一个 worker（会真的开一个 isolate，并在里面打开核心库）。
  ///
  /// [libraryPath] 只有桌面/测试要传；iOS/Android 走平台默认路径（见 [PerchCore.open]）。
  /// 核心打不开时抛 [StateError] —— 这是启动期的硬失败，调用方应当让 UI 进降级态。
  static Future<PerchWorker> spawn({String? libraryPath}) async {
    final handshake = ReceivePort();
    final responses = ReceivePort();
    final errors = ReceivePort();
    final exits = ReceivePort();

    final isolate = await Isolate.spawn(
      _workerMain,
      <Object?>[handshake.sendPort, responses.sendPort, libraryPath],
      onError: errors.sendPort,
      onExit: exits.sendPort,
      debugName: 'perch-core-worker',
      errorsAreFatal: true,
    );

    // 握手：worker 要么回一个 SendPort，要么回一个错误字符串
    final Object? ready = await handshake.first.timeout(
      const Duration(seconds: 20),
      onTimeout: () => <String, Object?>{'error': 'worker 启动超时（20s）'},
    );
    handshake.close();
    if (ready is! Map || ready['port'] is! SendPort) {
      isolate.kill(priority: Isolate.immediate);
      errors.close();
      exits.close();
      responses.close();
      throw StateError(
        '核心 worker 启动失败：${ready is Map ? ready['error'] : ready}',
      );
    }

    final worker = PerchWorker._(
      isolate,
      ready['port'] as SendPort,
      responses,
      errors,
      exits,
      libraryPath,
    );

    // 响应分发
    responses.listen((message) {
      if (message is! Map) {
        return;
      }
      final completer = worker._pending.remove(message['id']);
      if (completer == null || completer.isCompleted) {
        return;
      }
      if (message['ok'] == true) {
        completer.complete(
          (message['value'] as Map?)?.cast<String, dynamic>() ??
              <String, dynamic>{},
        );
      } else {
        completer.completeError(
          PerchCoreException(
            message['code'] as String? ?? 'WORKER_ERROR',
            message['message'] as String? ?? '核心调用失败（worker 未给出原因）',
          ),
        );
      }
    });

    // worker 异常退出时，别让调用方永远等下去
    void failAll(String reason) {
      if (worker._failure == null) worker._failures.add('$reason，请重启应用。');
      worker._failure ??= reason;
      worker._isolate.kill(priority: Isolate.immediate);
      for (final completer in worker._pending.values) {
        if (!completer.isCompleted) {
          completer.completeError(StateError('$reason，请重启应用。'));
        }
      }
      worker._pending.clear();
      worker._cleanupKnownContexts();
    }

    errors.listen((error) => failAll('核心 worker 报错：$error'));
    exits.listen((_) {
      if (!worker._exit.isCompleted) worker._exit.complete();
      if (!worker._disposed &&
          (!worker._disposing || worker._pending.isNotEmpty)) {
        failAll('核心 worker 意外退出');
      }
    });

    return worker;
  }

  /// 调一个核心方法（参数与返回值契约同 `perch_call`）。
  ///
  /// 抛出的 [PerchCoreException] 与直接调 [PerchCore] 时是同一个类型和错误码。
  Future<Map<String, dynamic>> call(
    String method, [
    Map<String, dynamic>? args,
  ]) {
    if (method == 'context.open') {
      return openContext().then(
        (context) => <String, dynamic>{'context': context.id},
      );
    }
    return _call(null, method, args);
  }

  /// Create a context outside the serial worker and track it before returning.
  ///
  /// The short-lived isolate is independent from [_isolate]. If the worker dies
  /// while the context is being created, its id still reaches this owner and is
  /// reclaimed by [dispose].
  Future<PerchContext> openContext() {
    if (_failure != null) {
      return Future.error(StateError('$_failure，请重启应用。'));
    }
    if (_disposed || _disposing) {
      return Future.error(StateError('PerchWorker 已经 dispose 了，请重新 spawn。'));
    }
    late final Future<PerchContext> operation;
    operation = _runContextOpen(_libraryPath).then((context) {
      _contexts.add(context.id);
      if (_failure != null) _cleanupKnownContexts();
      return context;
    });
    _contextOpens.add(operation);
    operation.then<void>(
      (_) => _contextOpens.remove(operation),
      onError: (Object _) => _contextOpens.remove(operation),
    );
    return operation;
  }

  /// 在显式 Rust context 中调用，业务 JSON 契约与 [call] 相同。
  Future<Map<String, dynamic>> callInContext(
    PerchContext context,
    String method, [
    Map<String, dynamic>? args,
  ]) => _call(context.id, method, args);

  /// Send a short pipeline control call without waiting for the serial queue.
  Future<Map<String, dynamic>> controlCallInContext(
    PerchContext context,
    String method, [
    Map<String, dynamic>? args,
  ]) {
    if (method != 'pipeline.cancel' && method != 'pipeline.progress') {
      return Future.error(
        ArgumentError.value(
          method,
          'method',
          '控制通道只接受 pipeline.cancel / pipeline.progress',
        ),
      );
    }
    if (_failure != null) {
      return Future.error(StateError('$_failure，请重启应用。'));
    }
    if (_disposed || _disposing) {
      return Future.error(StateError('PerchWorker 已经 dispose 了，请重新 spawn。'));
    }
    final config = <Object?>[
      _libraryPath,
      context.id,
      method,
      args ?? const <String, dynamic>{},
    ];
    final operation = _runControl(config).then(_unwrapControlResult);
    _controls.add(operation);
    operation.then<void>(
      (_) => _controls.remove(operation),
      onError: (Object _) => _controls.remove(operation),
    );
    return operation;
  }

  Future<Map<String, dynamic>> _call(
    int? context,
    String method,
    Map<String, dynamic>? args,
  ) {
    if (_failure != null) {
      return Future.error(StateError('$_failure，请重启应用。'));
    }
    if (_disposed || _disposing) {
      return Future.error(StateError('PerchWorker 已经 dispose 了，请重新 spawn。'));
    }
    final id = _nextId++;
    final completer = Completer<Map<String, dynamic>>();
    _pending[id] = completer;
    try {
      _commands.send(<String, Object?>{
        'id': id,
        'method': method,
        'args': args ?? const <String, dynamic>{},
        if (context != null) 'context': context,
      });
    } on Object catch (error, stack) {
      _pending.remove(id);
      completer.completeError(error, stack);
    }
    final result = completer.future;
    if (context == null && method == 'context.close') {
      final contextId = args?['context'];
      return result.then((value) {
        if (contextId is int) _contexts.remove(contextId);
        return value;
      });
    }
    return result;
  }

  /// Close a context even if the serial worker has already exited.
  Future<void> closeContext(PerchContext context) async {
    if (_failure == null && !_disposed && !_disposing) {
      try {
        await call('context.close', {'context': context.id});
        return;
      } on StateError {
        if (_failure == null) rethrow;
      }
    }
    await _cleanupContexts(<int>[context.id]);
  }

  /// Drain accepted requests, then stop the worker (idempotent).
  Future<void> dispose() => _shutdown ??= _dispose();

  /// Emergency stop. The exit listener fails pending and future requests and
  /// schedules known contexts for cleanup; await [dispose] to finish reclaiming.
  void terminate() => _isolate.kill(priority: Isolate.immediate);

  Future<void> _dispose() async {
    if (_disposed) {
      return;
    }
    _disposing = true;
    // Drain accepted calls before asking the isolate to stop.
    await Future.wait(
      _pending.values
          .map((c) => c.future.then<void>((_) {}, onError: (Object _) {}))
          .toList(),
    );
    await Future.wait(
      _controls
          .map((future) => future.then<void>((_) {}, onError: (Object _) {}))
          .toList(),
    );
    await Future.wait(
      _contextOpens
          .map((future) => future.then<void>((_) {}, onError: (Object _) {}))
          .toList(),
    );
    if (_failure == null) {
      _commands.send(const <String, Object?>{'cmd': 'stop'});
      await _exit.future;
    }
    await _cleanupKnownContexts();
    await Future.wait(
      _contextCleanups
          .map((future) => future.then<void>((_) {}, onError: (Object _) {}))
          .toList(),
    );
    _disposed = true;
    _responses.close();
    _errors.close();
    _exits.close();
    await _failures.close();
  }

  Future<void> _cleanupKnownContexts() => _cleanupContexts(_contexts.toList());

  Future<void> _cleanupContexts(List<int> contextIds) {
    final ids = contextIds.where(_contexts.contains).toSet().toList();
    if (ids.isEmpty) return Future<void>.value();
    late final Future<void> operation;
    operation = _runContextCleanup(<Object?>[_libraryPath, ids]).then((_) {
      _contexts.removeAll(ids);
    });
    _contextCleanups.add(operation);
    operation.then<void>(
      (_) => _contextCleanups.remove(operation),
      onError: (Object error) {
        _contextCleanups.remove(operation);
        if (!_failures.isClosed) {
          _failures.add('核心 context 回收失败：$error');
        }
      },
    );
    return operation;
  }
}

Map<String, dynamic> _unwrapControlResult(Map<String, dynamic> message) {
  if (message['ok'] == true) {
    return (message['value'] as Map?)?.cast<String, dynamic>() ??
        <String, dynamic>{};
  }
  throw PerchCoreException(
    message['code'] as String? ?? 'WORKER_ERROR',
    message['message'] as String? ?? '核心控制调用失败（worker 未给出原因）',
  );
}

Future<Map<String, dynamic>> _runControl(List<Object?> config) =>
    Isolate.run(() => _controlInvoke(config), debugName: 'perch-core-control');

Map<String, dynamic> _controlInvoke(List<Object?> config) {
  final libraryPath = config[0] as String?;
  final context = config[1]! as int;
  final method = config[2]! as String;
  final payload = (config[3]! as Map).cast<String, dynamic>();
  try {
    final value = PerchCore.open(
      libraryPath: libraryPath,
    ).invokeInContext(PerchContext(context), method, payload);
    return <String, dynamic>{'ok': true, 'value': value};
  } on PerchCoreException catch (error) {
    return <String, dynamic>{
      'ok': false,
      'code': error.code,
      'message': error.message,
    };
  } on Object catch (error) {
    return <String, dynamic>{
      'ok': false,
      'code': 'WORKER_ERROR',
      'message': '$error',
    };
  }
}

Future<void> _runContextCleanup(List<Object?> config) => Isolate.run(
  () => _closeContexts(config),
  debugName: 'perch-core-context-cleanup',
);

Future<PerchContext> _runContextOpen(String? libraryPath) => Isolate.run(
  () => PerchCore.open(libraryPath: libraryPath).openContext(),
  debugName: 'perch-core-context-open',
);

void _closeContexts(List<Object?> config) {
  final libraryPath = config[0] as String?;
  final contexts = (config[1]! as List).cast<int>();
  final core = PerchCore.open(libraryPath: libraryPath);
  for (final context in contexts) {
    core.closeContext(PerchContext(context));
  }
}

/// worker isolate 的入口（必须是顶层函数）。
void _workerMain(Object? config) {
  final args = config! as List<Object?>;
  final handshake = args[0]! as SendPort;
  final responses = args[1]! as SendPort;
  final libraryPath = args[2] as String?;

  final PerchCore core;
  try {
    core = PerchCore.open(libraryPath: libraryPath);
  } on Object catch (error) {
    // 打不开就明确说，不要让调用方等到超时
    handshake.send(<String, Object?>{'error': '$error'});
    return;
  }

  final inbox = ReceivePort();
  handshake.send(<String, Object?>{'port': inbox.sendPort});

  inbox.listen((message) {
    if (message is! Map) {
      return;
    }
    if (message['cmd'] == 'stop') {
      inbox.close();
      return;
    }

    final id = message['id'];
    final method = message['method'] as String;
    final context = message['context'] as int?;
    final payload = (message['args'] as Map?)?.cast<String, dynamic>();
    try {
      final value = method == 'worker.fingerprintPath'
          ? _fingerprintPath(core, payload!['path'] as String)
          : context == null
          ? core.invoke(method, payload)
          : core.invokeInContext(PerchContext(context), method, payload);
      responses.send(<String, Object?>{'id': id, 'ok': true, 'value': value});
    } on PerchCoreException catch (error) {
      responses.send(<String, Object?>{
        'id': id,
        'ok': false,
        'code': error.code,
        'message': error.message,
      });
    } on Object catch (error) {
      // 不该发生（核心不 panic，FFI 层把错误都包装成 PerchCoreException），
      // 但真发生了也要把消息带回主 isolate，而不是让请求悬着。
      responses.send(<String, Object?>{
        'id': id,
        'ok': false,
        'code': 'WORKER_ERROR',
        'message': '$error',
      });
    }
  });
}

Map<String, dynamic> _fingerprintPath(PerchCore core, String path) {
  final file = File(path).openSync();
  try {
    final size = file.lengthSync();
    final count = size < 65536 ? size : 65536;
    final head = file.readSync(count);
    file.setPositionSync(size - count);
    final tail = file.readSync(count);
    return {
      'size': size,
      'fingerprint': core.fingerprint(size: size, head: head, tail: tail),
    };
  } finally {
    file.closeSync();
  }
}
