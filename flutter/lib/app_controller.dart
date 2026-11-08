import 'dart:async';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/foundation.dart';

import 'core_bridge.dart';

class HermodController extends ChangeNotifier {
  CoreBridge? _core;
  Timer? _timer;
  Map<String, dynamic>? bootstrap;
  Map<String, dynamic>? status;
  Map<String, dynamic>? selection;
  List<Map<String, dynamic>> inbox = const [];
  List<String> logs = const [];
  String? error;
  bool busy = true;
  bool _statusLoading = false;

  Map<String, dynamic>? get settings =>
      bootstrap?['settings'] as Map<String, dynamic>?;

  Future<void> initialize() async {
    try {
      _core = await CoreBridge.load();
      bootstrap = await _core!.initialize();
      await refreshAll();
      _timer = Timer.periodic(
        const Duration(milliseconds: 500),
        (_) => refreshStatus(),
      );
    } catch (exception, stackTrace) {
      error = exception.toString();
      // 保留初始化失败根因，便于诊断 FFI 加载与 Core 初始化问题。
      debugPrint('Core initialize failed: $exception\n$stackTrace');
    } finally {
      busy = false;
      notifyListeners();
    }
  }

  Future<void> refreshStatus() async {
    if (_core == null || _statusLoading) return;
    _statusLoading = true;
    try {
      status = await _core!.status();
      error = status?['error'] as String?;
      if (status?['done'] == true && status?['received_id'] != null) {
        await refreshInbox();
      }
    } catch (exception) {
      error = exception.toString();
    } finally {
      _statusLoading = false;
      notifyListeners();
    }
  }

  Future<void> refreshAll() async {
    await Future.wait([refreshStatus(), refreshInbox(), refreshLogs()]);
  }

  Future<void> refreshInbox() async {
    if (_core == null) return;
    try {
      inbox = await _core!.inbox();
    } catch (exception) {
      error = exception.toString();
    }
    notifyListeners();
  }

  Future<void> refreshLogs() async {
    if (_core == null) return;
    try {
      logs = await _core!.logs();
    } catch (exception) {
      error = exception.toString();
    }
    notifyListeners();
  }

  Future<void> chooseFile() => _guard(() async {
    final file = await openFile();
    if (file != null) selection = await _core!.selectSource(file.path);
  });

  Future<void> sendFile() => _guard(() async {
    final id = selection?['id'] as String?;
    if (id == null) throw StateError('请先选择文件。');
    await _core!.start(sourceId: id);
    await refreshStatus();
  });

  Future<void> sendText(String text) => _guard(() async {
    if (text.isEmpty) throw StateError('请输入要发送的文本。');
    await _core!.start(text: text);
    await refreshStatus();
  });

  Future<void> receive(String code) => _guard(() async {
    if (code.isEmpty) throw StateError('请输入传输码。');
    await _core!.start(code: code);
    await refreshStatus();
  });

  Future<void> cancel() => _guard(() async {
    await _core!.cancel();
    await refreshStatus();
  });

  Future<void> exportItem(Map<String, dynamic> item) => _guard(() async {
    final location = await getSaveLocation(
      suggestedName: item['name'] as String? ?? 'received-file',
    );
    if (location != null) {
      await _core!.export(item['id'] as String, location.path);
    }
  });

  Future<void> removeItem(Map<String, dynamic> item) => _guard(() async {
    await _core!.remove(item['id'] as String);
    await refreshInbox();
  });

  Future<String?> probeServer(String server) async {
    String? fingerprint;
    await _guard(() async => fingerprint = await _core!.probeServer(server));
    return fingerprint;
  }

  Future<void> trustServer(String server, String fingerprint) =>
      _guard(() async => _core!.trustServer(server, fingerprint));

  Future<void> saveSettings(Map<String, dynamic> value) => _guard(() async {
    await _core!.saveSettings(value);
    bootstrap = {...?bootstrap, 'settings': value};
  });

  Future<void> _guard(Future<void> Function() action) async {
    error = null;
    try {
      await action();
    } catch (exception) {
      error = exception.toString();
    }
    notifyListeners();
  }

  @override
  void dispose() {
    _timer?.cancel();
    unawaited(_core?.close());
    super.dispose();
  }
}
