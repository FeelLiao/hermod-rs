import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/foundation.dart' show debugPrint;
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:path/path.dart' as p;

import 'src/rust/api.dart' as api;
import 'src/rust/api.dart' show ApiBootstrap, ApiSettings;
import 'src/rust/api/platform.dart' as platform_api;
import 'src/rust/api/events.dart' as api_events;
import 'src/rust/app.dart' show Selection;
import 'src/rust/app/storage.dart' show Received;
import 'src/rust/frb_generated.dart';
import 'src/rust/gui_api/error.dart' show GuiError;
import 'src/rust/gui_api/models.dart';

export 'src/rust/api.dart'
    show ApiBootstrap, ApiSettings, ApiConnection, ApiTuning;
export 'src/rust/app.dart' show Selection;
export 'src/rust/app/storage.dart' show Received;
export 'src/rust/app/worker.dart' show TransferPhase;
export 'src/rust/gui_api/models.dart';

class CoreException implements Exception {
  const CoreException(this.code, this.message, {this.retryable = false});
  factory CoreException.from(Object error) {
    if (error is CoreException) return error;
    if (error is GuiError) {
      return CoreException(
        error.code.name,
        error.message,
        retryable: error.retryable,
      );
    }
    if (error is PlatformException) {
      return CoreException(error.code, error.message ?? error.code);
    }
    return CoreException('internal', error.toString());
  }
  final String code;
  final String message;
  final bool retryable;
  @override
  String toString() => '$code: $message';
}

/// The injected boundary is typed all the way through to FRB. Tests never load
/// the native library; only the production factory does.
abstract interface class CoreGateway {
  void configureLifecycle(Future<bool> Function() confirmClose);
  Future<ApiBootstrap> initialize();
  Future<TransferSnapshot?> status();
  Stream<TransferSnapshot?> watchStatus() => const Stream.empty();
  Future<Selection?> chooseSource();
  Future<void> start(StartRequest request);
  Future<void> cancel();
  Future<List<Received>> inbox();
  Future<bool> exportReceived(Received item);
  Future<void> remove(String id);
  Future<List<GuiDeviceDto>> devices();
  Future<void> renameDevice(String id, String name);
  Future<void> removeDevice(String id);
  Future<List<String>> logs();
  Future<String> probeServer(String server);
  Future<void> trustServer(String server, String fingerprint);
  Future<void> saveSettings(ApiSettings settings);
  Future<void> close();
}

class CoreBridge implements CoreGateway {
  static const _platform = MethodChannel('hermod/platform');
  static int _operationSequence = 0;
  String _operationId() =>
      '${DateTime.now().microsecondsSinceEpoch}-${++_operationSequence}';
  String? _pendingOperation;
  String? _serviceOperation;
  final Map<String, String?> _stagedSources = {};

  Future<void> _releaseSource(String id) async {
    if (!_stagedSources.containsKey(id)) return;
    final path = _stagedSources[id];
    if (path == null) {
      await platform_api.releaseWindowsSource(selectionId: id);
    } else {
      await _releaseAndroidSource(path);
    }
    _stagedSources.remove(id);
  }

  Future<void> _releaseAndroidSource(String path) async {
    final released = await _platform.invokeMethod<bool>('releaseStagedSource', {
      'path': path,
    });
    if (released != true) {
      throw const CoreException(
        'storage',
        'Cannot release private source staging.',
      );
    }
  }

  Future<void> _releaseSources() async {
    for (final id in _stagedSources.keys.toList()) {
      await _releaseSource(id);
    }
  }

  @override
  void configureLifecycle(Future<bool> Function() confirmClose) {
    _platform.setMethodCallHandler((call) async {
      if (call.method == 'closeRequested') {
        if (await confirmClose()) {
          await close();
          await _platform.invokeMethod<void>('confirmClose');
        } else {
          await _platform.invokeMethod<void>('cancelClose');
        }
      } else if (call.method == 'transferServiceStopped') {
        final arguments = call.arguments;
        final id = arguments is Map ? arguments['operationId'] : null;
        if (id == null || id != _serviceOperation) return;
        _serviceOperation = null;
        final snapshot = await api.status();
        if (snapshot != null && !snapshot.done) await api.cancel();
      }
    });
  }

  @override
  Future<ApiBootstrap> initialize() async {
    await RustLib.init(
      externalLibrary: Platform.isWindows
          ? ExternalLibrary.open(
              p.join(
                File(Platform.resolvedExecutable).parent.path,
                'hermod_flutter_bridge.dll',
              ),
            )
          : null,
    );
    final String support;
    if (Platform.isWindows) {
      support = await platform_api.windowsSupportDirectory();
    } else if (Platform.isAndroid) {
      final directory = await _platform.invokeMethod<String>(
        'getSupportDirectory',
      );
      if (directory == null || directory.isEmpty) {
        throw const CoreException(
          'storage',
          'Application private directory is unavailable.',
        );
      }
      support = directory;
    } else {
      throw const CoreException(
        'unsupportedPlatform',
        'Only Windows and Android are supported.',
      );
    }
    return api.initialize(appDir: support);
  }

  Future<BigInt> _windowHandle() async {
    final handle = await _platform.invokeMethod<int>('getWindowHandle');
    if (handle == null || handle == 0) {
      throw const CoreException('platform', 'Window handle is unavailable.');
    }
    return BigInt.from(handle);
  }

  @override
  Future<Selection?> chooseSource() async {
    final previous = _stagedSources.keys.toList();
    final Selection? selection;
    if (Platform.isWindows) {
      selection = await platform_api.windowsPickSource(
        windowHandle: await _windowHandle(),
      );
      if (selection != null) _stagedSources[selection.id] = null;
    } else {
      final operation = _operationId();
      _pendingOperation = operation;
      Map<String, dynamic>? selected;
      try {
        selected = await _platform.invokeMapMethod<String, dynamic>(
          'pickSource',
          {'operationId': operation},
        );
      } finally {
        if (_pendingOperation == operation) _pendingOperation = null;
      }
      if (selected == null) return null;
      final path = selected['path'];
      if (path is! String || path.isEmpty) {
        throw const CoreException(
          'platform',
          'Source staging did not return a private path.',
        );
      }
      try {
        final name = selected['name'];
        if (name is! String || name.isEmpty) {
          throw const CoreException(
            'platform',
            'Source staging did not return a filename.',
          );
        }
        selection = await api.selectNamedSource(path: path, name: name);
        _stagedSources[selection.id] = path;
      } catch (_) {
        await _releaseAndroidSource(path);
        rethrow;
      }
    }
    if (selection == null) return null;
    // Only after successful replacement; cancelling a picker preserves the old source.
    for (final id in previous) {
      await _releaseSource(id);
    }
    return selection;
  }

  @override
  Future<void> start(StartRequest request) async {
    try {
      if (Platform.isAndroid) {
        final operation = _operationId();
        _serviceOperation = operation;
        _pendingOperation = operation;
        final bool? started;
        try {
          started = await _platform.invokeMethod<bool>('startTransferService', {
            'operationId': operation,
          });
        } finally {
          if (_pendingOperation == operation) _pendingOperation = null;
        }
        if (started != true || _serviceOperation != operation) {
          throw const CoreException(
            'backgroundService',
            'Cannot start transfer service.',
          );
        }
      }
      await api.start(request: request);
      _serviceTerminal = false;
    } catch (_) {
      await _stopService('failed');
      rethrow;
    }
  }

  final Map<String, String> _pendingServiceStops = {};
  Future<void> _stopService([String reason = 'completed']) async {
    final operation = _serviceOperation;
    // Clear current before awaiting, but retain failed cleanup independently so
    // a stale reverse callback cannot cancel a new task and shutdown can retry.
    _serviceOperation = null;
    if (operation != null) _pendingServiceStops[operation] = reason;
    if (!Platform.isAndroid) return;
    for (final entry in _pendingServiceStops.entries.toList()) {
      await _platform.invokeMethod<bool>('stopTransferService', {
        'operationId': entry.key,
        'reason': entry.value,
      });
      // false means stale/already gone, not an operation still owned by us.
      _pendingServiceStops.remove(entry.key);
    }
  }

  bool _serviceTerminal = false;
  @override
  Future<TransferSnapshot?> status() async {
    final snapshot = await api.status();
    try {
      await _observeStatus(snapshot);
    } catch (error) {
      // Cleanup is ancillary: never turn a terminal task back into an active UI.
      debugPrint('Terminal cleanup failed: ${CoreException.from(error).code}');
    }
    return snapshot;
  }

  final Set<String> _subscriptions = {};
  @override
  Stream<TransferSnapshot?> watchStatus() {
    final id = _operationId();
    BigInt? sequence;
    StreamSubscription<api_events.GuiEvent>? upstream;
    late StreamController<TransferSnapshot?> output;
    Future<void> process(api_events.GuiEvent event) async {
      try {
        if (sequence != null && event.sequence <= sequence!) return;
        sequence = event.sequence;
        try {
          await _observeStatus(event.status);
        } catch (error, stack) {
          if (!output.isClosed) output.addError(error, stack);
        }
        // A cleanup error is surfaced separately, without withholding done.
        if (!output.isClosed) output.add(event.status);
      } catch (error, stack) {
        if (!output.isClosed) output.addError(error, stack);
      } finally {
        await api_events.acknowledge(
          subscriptionId: id,
          sequence: event.sequence,
        );
      }
    }

    output = StreamController<TransferSnapshot?>(
      onListen: () {
        _subscriptions.add(id);
        upstream = api_events
            .subscribe(subscriptionId: id)
            .listen(
              (event) async {
                upstream?.pause();
                try {
                  await process(event);
                } finally {
                  upstream?.resume();
                }
              },
              onError: output.addError,
              onDone: output.close,
            );
      },
      onCancel: () async {
        _subscriptions.remove(id);
        // Explicit native unsubscribe must precede cancellation: an idle FRB
        // stream has no next event to wake an async* finally block.
        try {
          await api_events.unsubscribe(subscriptionId: id);
        } finally {
          await upstream?.cancel();
        }
      },
    );
    return output.stream;
  }

  Future<void>? _terminalCleanup;
  Future<void> _observeStatus(TransferSnapshot? snapshot) async {
    if (snapshot?.done == true && !_serviceTerminal) {
      final cleanup = _terminalCleanup ??= _finishTransfer(snapshot!);
      try {
        await cleanup;
      } finally {
        if (identical(_terminalCleanup, cleanup)) _terminalCleanup = null;
      }
    } else if (snapshot != null && !snapshot.done) {
      _serviceTerminal = false;
    }
  }

  Future<void> _finishTransfer(TransferSnapshot snapshot) async {
    try {
      await _stopService(
        snapshot.cancelled
            ? 'cancelled'
            : snapshot.error != null
            ? 'failed'
            : 'completed',
      );
    } finally {
      await _releaseSources();
    }
    _serviceTerminal = true;
  }

  @override
  Future<bool> exportReceived(Received item) async {
    if (Platform.isWindows) {
      return platform_api.windowsExportReceived(
        windowHandle: await _windowHandle(),
        id: item.id,
      );
    }
    final source = await platform_api.prepareExport(id: item.id);
    final operation = _operationId();
    _pendingOperation = operation;
    try {
      return await _platform.invokeMethod<bool>('exportFile', {
            'operationId': operation,
            'sourcePath': source,
            'name': item.name,
          }) ??
          false;
    } finally {
      if (_pendingOperation == operation) _pendingOperation = null;
      await platform_api.releaseExport(path: source);
    }
  }

  @override
  Future<void> cancel() async {
    if (Platform.isWindows) {
      final cancelledFile = await platform_api.cancelFileOperation();
      if (cancelledFile) return;
    }
    final pending = _pendingOperation;
    if (Platform.isAndroid && pending != null) {
      await _platform.invokeMethod<bool>('cancelPlatformOperation', {
        'operationId': pending,
      });
    }
    await api.cancel();
  }

  @override
  Future<List<Received>> inbox() => api.inbox();
  @override
  Future<void> remove(String id) => api.remove(id: id);
  @override
  Future<List<GuiDeviceDto>> devices() => api.devices();
  @override
  Future<void> renameDevice(String id, String name) =>
      api.renameDevice(deviceId: id, name: name);
  @override
  Future<void> removeDevice(String id) => api.removeDevice(deviceId: id);
  @override
  Future<List<String>> logs() => api.logs();
  @override
  Future<String> probeServer(String server) => api.probeServer(server: server);
  @override
  Future<void> trustServer(String server, String fingerprint) =>
      api.trustServer(server: server, fingerprint: fingerprint);
  @override
  Future<void> saveSettings(ApiSettings settings) =>
      api.saveSettings(settings: settings);
  Future<void>? _closing;
  @override
  Future<void> close() => _closing ??= _close();

  Future<void> _close() async {
    _platform.setMethodCallHandler(null);
    try {
      for (final id in _subscriptions.toList()) {
        await api_events.unsubscribe(subscriptionId: id);
      }
      await cancel();
    } finally {
      try {
        await api.shutdown();
      } finally {
        try {
          await _stopService('cancelled');
        } finally {
          try {
            await _releaseSources();
          } finally {
            RustLib.dispose();
          }
        }
      }
    }
  }
}
