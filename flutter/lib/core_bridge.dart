import 'dart:io';

import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';

import 'src/rust/api.dart' as api;
import 'src/rust/app/storage.dart' show Received;
import 'src/rust/frb_generated.dart';
import 'src/rust/gui_api/error.dart' show GuiError;
import 'src/rust/gui_api/models.dart' show StartRequest;

/// Core 抛给 UI 的稳定错误，字段与 Rust `GuiError` 对齐。
class CoreException implements Exception {
  final String code;
  final String message;
  final bool retryable;

  const CoreException(this.code, this.message, {this.retryable = false});

  CoreException.fromCore(GuiError error)
    : code = error.code.name,
      message = error.message,
      retryable = error.retryable;

  @override
  String toString() => message;

  @override
  bool operator ==(Object other) =>
      other is CoreException &&
      other.code == code &&
      other.message == message &&
      other.retryable == retryable;

  @override
  int get hashCode => Object.hash(code, message, retryable);
}

/// 桥接层。保持与旧 JSON 行协议相同的 Map 字段约定（serde 字段名），
/// 让传输/收件箱/设置页面无感知地切换到 flutter_rust_bridge FFI。
class CoreBridge {
  CoreBridge._(this._bootstrap);

  final Map<String, dynamic> _bootstrap;

  static Future<CoreBridge> load() async {
    await RustLib.init();
    final support = await _resolveSupportDirectory();
    final bootstrap = await api.initialize(appDir: support.path);
    return CoreBridge._(_bootstrapMap(bootstrap));
  }

  /// 解析 Core 数据根目录：优先环境变量，其次系统应用支持目录；
  /// 当 AppData 被安全策略拒绝写入时回退到可执行文件旁，避免开发调试失败。
  static Future<Directory> _resolveSupportDirectory() async {
    final override = Platform.environment['HERMOD_APP_SUPPORT'];
    if (override != null && override.trim().isNotEmpty) {
      return Directory(override.trim());
    }
    try {
      final support = await getApplicationSupportDirectory();
      await support.create(recursive: true);
      return support;
    } catch (_) {
      final executable = File(Platform.resolvedExecutable).parent;
      final fallback = Directory(p.join(executable.path, 'app-support'));
      return fallback;
    }
  }

  Map<String, dynamic> get bootstrap => _bootstrap;

  /// 与旧协议兼容的显式初始化入口；FFI 在 load() 内已完成初始化。
  Future<Map<String, dynamic>> initialize() async => _bootstrap;

  Future<Map<String, dynamic>?> status() async {
    final snapshot = await api.status();
    if (snapshot == null) return null;
    return {
      'phase': snapshot.phase.name,
      'phase_message': snapshot.phaseMessage,
      'code': snapshot.code,
      'bytes': snapshot.bytes.toInt(),
      'total': snapshot.total?.toInt(),
      'elapsed': snapshot.elapsed,
      'done': snapshot.done,
      'cancelled': snapshot.cancelled,
      'error': snapshot.error,
      'digest': snapshot.digest,
      'rtt_ms': snapshot.rttMs,
      'transport': snapshot.transport,
      'received_id': snapshot.receivedId,
    };
  }

  Future<List<Map<String, dynamic>>> inbox() async {
    final items = await api.inbox();
    return items.map(_receivedMap).toList();
  }

  Future<List<String>> logs() => api.logs();

  Future<Map<String, dynamic>> selectSource(String path) async {
    final selection = await api.selectSource(path: path);
    return {
      'id': selection.id,
      'name': selection.name,
      'size': selection.size?.toInt(),
    };
  }

  Future<void> start({String? sourceId, String? code, String? text}) => _wrap(
    api.start(
      request: StartRequest(sourceId: sourceId, code: code, text: text),
    ),
  );

  Future<void> cancel() => api.cancel();

  Future<void> remove(String id) => _wrap(api.remove(id: id));

  Future<void> export(String id, String path, {bool overwrite = false}) =>
      _wrap(api.export_(id: id, path: path, overwrite: overwrite));

  Future<String> probeServer(String server) =>
      _wrap(api.probeServer(server: server));

  Future<void> trustServer(String server, String fingerprint) =>
      _wrap(api.trustServer(server: server, fingerprint: fingerprint));

  Future<void> saveSettings(Map<String, dynamic> settings) => _wrap(() async {
    final connection =
        (settings['connection'] as Map<String, dynamic>? ?? const {});
    await api.saveSettings(
      settings: api.ApiSettings(
        version: settings['version'] as int? ?? 2,
        connection: api.ApiConnection(
          transport: connection['transport'] as String? ?? 'quic',
          mtu: connection['mtu'] as String? ?? 'auto',
          congestion: connection['congestion'] as String? ?? 'auto',
          limitMbps: connection['limit_mbps'] as String? ?? '0',
        ),
        receiveTransport: settings['receive_transport'] as String? ?? 'auto',
        server: settings['server'] as String? ?? '',
        words: settings['words'] as int? ?? 3,
        verbose: settings['verbose'] as String? ?? 'none',
        quiet: settings['quiet'] as bool? ?? false,
        theme: settings['theme'] as String? ?? 'system',
        language: settings['language'] as String? ?? 'system',
        destination: settings['destination'] as String?,
      ),
    );
  }());

  Future<void> close() async {
    RustLib.dispose();
  }

  Future<T> _wrap<T>(Future<T> future) async {
    try {
      return await future;
    } on Object catch (exception) {
      throw _toCoreException(exception);
    }
  }

  static Object _toCoreException(Object exception) {
    if (exception is CoreException) return exception;
    return CoreException('internal', exception.toString());
  }

  static Map<String, dynamic> _bootstrapMap(api.ApiBootstrap bootstrap) => {
    'version': bootstrap.version,
    'ipv4': bootstrap.ipv4,
    'ipv6': bootstrap.ipv6,
    'platform': bootstrap.platform,
    'settings_saved': bootstrap.settingsSaved,
    'settings': _settingsMap(bootstrap.settings),
  };

  static Map<String, dynamic> _settingsMap(api.ApiSettings settings) => {
    'version': settings.version,
    'connection': {
      'transport': settings.connection.transport,
      'mtu': settings.connection.mtu,
      'congestion': settings.connection.congestion,
      'limit_mbps': settings.connection.limitMbps,
    },
    'receive_transport': settings.receiveTransport,
    'server': settings.server,
    'words': settings.words,
    'verbose': settings.verbose,
    'quiet': settings.quiet,
    'theme': settings.theme,
    'language': settings.language,
    'destination': settings.destination,
  };

  static Map<String, dynamic> _receivedMap(Received item) => {
    'id': item.id,
    'name': item.name,
    'size': item.size.toInt(),
    'sha256': item.sha256,
  };
}
