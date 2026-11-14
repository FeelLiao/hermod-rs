import 'dart:async';

import 'package:flutter/foundation.dart';

import 'core_bridge.dart';

class HermodController extends ChangeNotifier {
  HermodController({CoreGateway? gateway}) : _core = gateway ?? CoreBridge();
  final CoreGateway _core;
  StreamSubscription<TransferSnapshot?>? _statusSubscription;
  Future<void>? _subscriptionCancellation;
  bool _shuttingDown = false;
  int _statusRevision = 0;
  bool _disposed = false;
  bool _initialized = false;
  bool _statusLoading = false;
  bool _terminalRefreshed = false;
  ApiBootstrap? bootstrap;
  ApiSettings? _settings;
  ApiSettings? get settings => _settings;
  ApiSettings? taskSettings;
  TransferSnapshot? status;
  Selection? selection;
  List<Received> inbox = const [];
  List<GuiDeviceDto> devices = const [];
  List<String> logs = const [];
  CoreException? error;
  bool busy = true;
  bool operating = false;
  bool fileOperating = false;
  bool _fileCancellationRequested = false;
  bool _cancellingFile = false;
  bool requestPairing = true;
  bool get active => status != null && !status!.done;
  bool get canStart =>
      _initialized &&
      !_disposed &&
      !_shuttingDown &&
      !busy &&
      !operating &&
      !active;

  void configureLifecycle(Future<bool> Function() confirmClose) =>
      _core.configureLifecycle(confirmClose);
  void _notify() {
    if (!_disposed) notifyListeners();
  }

  void setPairing(bool value) {
    if (canStart) {
      requestPairing = value;
      _notify();
    }
  }

  Future<void> initialize() async {
    if (_initialized || operating || _disposed || _shuttingDown) return;
    operating = true;
    try {
      final value = await _core.initialize();
      if (_disposed || _shuttingDown) return;
      bootstrap = value;
      _settings = value.settings;
      _initialized = true;
      // Listen before reading the initial snapshot: an event during that read
      // wins, including an event delivered synchronously by a test gateway.
      final revision = _statusRevision;
      _statusSubscription = _core.watchStatus().listen(
        (value) {
          if (_disposed || _shuttingDown) return;
          _statusRevision++;
          unawaited(_applyStatus(value));
        },
        onError: (Object exception) {
          if (_disposed || _shuttingDown) return;
          error = CoreException.from(exception);
          _notify();
        },
      );
      await Future.wait([
        _refreshStatus(revision),
        refreshInbox(),
        refreshDevices(),
        refreshLogs(),
      ]);
    } catch (exception) {
      if (!_disposed) error = CoreException.from(exception);
    } finally {
      busy = false;
      operating = false;
      _notify();
    }
  }

  Future<void> refreshStatus() => _refreshStatus(_statusRevision);

  Future<void> _refreshStatus(int revision) async {
    if (!_initialized || _disposed || _shuttingDown || _statusLoading) return;
    _statusLoading = true;
    try {
      final value = await _core.status();
      if (_disposed || _shuttingDown || revision != _statusRevision) return;
      await _applyStatus(value);
    } catch (exception) {
      if (!_disposed && !_shuttingDown && revision == _statusRevision) {
        error = CoreException.from(exception);
      }
    } finally {
      _statusLoading = false;
      _notify();
    }
  }

  Future<void> _applyStatus(TransferSnapshot? value) async {
    if (_disposed || _shuttingDown) return;
    status = value;
    final terminal = value?.done == true;
    if (value != null && !terminal) _terminalRefreshed = false;
    final refresh = terminal && !_terminalRefreshed;
    if (refresh) {
      // Mark before awaiting so duplicate events cannot schedule another refresh.
      _terminalRefreshed = true;
      selection = null;
    }
    _notify();
    if (refresh) await Future.wait([refreshInbox(), refreshDevices()]);
  }

  Future<void> refreshAll() async {
    await Future.wait([
      refreshStatus(),
      refreshInbox(),
      refreshDevices(),
      refreshLogs(),
    ]);
  }

  Future<void> refreshInbox() => _refresh(() async {
    final value = await _core.inbox();
    if (!_disposed) inbox = value;
  });
  Future<void> refreshDevices() => _refresh(() async {
    final value = await _core.devices();
    if (!_disposed) devices = value;
  });
  Future<void> refreshLogs() => _refresh(() async {
    final value = await _core.logs();
    if (!_disposed) logs = value;
  });
  Future<void> _refresh(Future<void> Function() action) async {
    if (!_initialized || _disposed || _shuttingDown) return;
    try {
      await action();
    } catch (exception) {
      if (!_disposed) error = CoreException.from(exception);
    }
    _notify();
  }

  Future<bool> chooseFile() {
    if (active) return Future.value(false);
    return _fileOperation(() async {
      final value = await _core.chooseSource();
      if (!_disposed && !_fileCancellationRequested && value != null) {
        selection = value;
      }
    });
  }

  Future<bool> _fileOperation(Future<void> Function() action) =>
      _guard(() async {
        fileOperating = true;
        _fileCancellationRequested = false;
        _notify();
        try {
          await action();
        } finally {
          fileOperating = false;
          _notify();
        }
      });

  Future<bool> cancelFileOperation() async {
    if (_disposed || _shuttingDown || !fileOperating || _cancellingFile) {
      return false;
    }
    _cancellingFile = true;
    _fileCancellationRequested = true;
    try {
      // Deliberately bypass _guard: the picker/export already owns that guard.
      await _core.cancel();
      return !_disposed;
    } catch (exception) {
      if (!_disposed) error = CoreException.from(exception);
      return false;
    } finally {
      _cancellingFile = false;
      _notify();
    }
  }

  Future<bool> sendFile() => _start(sourceId: selection?.id);
  Future<bool> receive(String code) => _start(code: code);
  Future<bool> receiveFromDevice(String deviceId) =>
      _start(fromDevice: deviceId);
  Future<bool> _start({
    String? sourceId,
    String? code,
    String? fromDevice,
  }) async {
    if (!canStart) return false;
    return _guard(() async {
      if (sourceId == null && code == null && fromDevice == null) {
        throw const CoreException('invalidRequest', 'Choose a source first.');
      }
      final revision = _statusRevision;
      _terminalRefreshed = false;
      taskSettings = _settings;
      await _core.start(
        StartRequest(
          sourceId: sourceId,
          code: code,
          text: null,
          fromDevice: fromDevice,
          // Existing trusted-device calls authenticate their saved relationship;
          // they do not request a new pairing handshake.
          pair: fromDevice == null && requestPairing,
        ),
      );
      if (_disposed) return;
      await _refreshStatus(revision);
    });
  }

  Future<bool> cancel() => _guard(() async {
    final revision = _statusRevision;
    await _core.cancel();
    await _refreshStatus(revision);
  });
  Future<bool> exportItem(Received item) => _fileOperation(() async {
    await _core.exportReceived(item);
  });
  Future<bool> removeItem(Received item) => _guard(() async {
    await _core.remove(item.id);
    await refreshInbox();
  });
  Future<bool> renameDevice(String id, String name) => _guard(() async {
    await _core.renameDevice(id, name);
    await refreshDevices();
  });
  Future<bool> removeDevice(String id) => _guard(() async {
    await _core.removeDevice(id);
    await refreshDevices();
  });
  Future<String?> probeServer(String server) async {
    String? result;
    await _guard(() async {
      result = await _core.probeServer(server);
    });
    return result;
  }

  Future<bool> trustServer(String server, String fingerprint) =>
      _guard(() => _core.trustServer(server, fingerprint));
  Future<bool> saveSettings(ApiSettings value) => _guard(() async {
    await _core.saveSettings(value);
    if (!_disposed) _settings = value;
  });

  Future<bool> _guard(Future<void> Function() action) async {
    if (_disposed || _shuttingDown || !_initialized || operating) return false;
    operating = true;
    error = null;
    _notify();
    try {
      await action();
      return !_disposed;
    } catch (exception) {
      if (!_disposed) error = CoreException.from(exception);
      return false;
    } finally {
      operating = false;
      _notify();
    }
  }

  Future<void> _cancelSubscription() => _subscriptionCancellation ??= () async {
    final subscription = _statusSubscription;
    _statusSubscription = null;
    await subscription?.cancel();
  }();

  Future<void> shutdown() async {
    _shuttingDown = true;
    await _cancelSubscription();
    await _core.close();
  }

  @override
  void dispose() {
    _disposed = true;
    // Detaching the UI is not an instruction to cancel a native transfer.
    unawaited(_detachSafely());
    super.dispose();
  }

  Future<void> _detachSafely() async {
    try {
      await _cancelSubscription();
    } catch (exception) {
      debugPrint(
        'Status unsubscribe failed: ${CoreException.from(exception).code}',
      );
    }
  }
}
