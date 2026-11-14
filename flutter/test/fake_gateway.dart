import 'dart:async';

import 'package:hermod_app/core_bridge.dart';

class FakeGateway implements CoreGateway {
  final settings = ApiSettings(
    version: 2,
    connection: const ApiConnection(
      transport: 'quic',
      mtu: 'auto',
      congestion: 'auto',
      profile: 'auto',
    ),
    receiveTransport: 'auto',
    ipFamily: 'auto',
    server: 'wss://example.test',
    words: 3,
    verbose: 'none',
    quiet: false,
    theme: 'system',
    language: 'en-US',
    tuning: ApiTuning(
      streams: null,
      chunkBytes: 65536,
      windowBytes: BigInt.from(1048576),
      udpBufferBytes: 1048576,
      sliceBytes: BigInt.from(1048576),
      prefetch: 2,
      keepaliveSecs: 5,
      idleTimeoutSecs: 30,
    ),
  );
  StartRequest? started;
  ApiSettings? saved;
  TransferSnapshot? snapshot;
  Object? failure;
  Completer<void>? startGate;
  Completer<Selection?>? chooseGate;
  Completer<bool>? exportGate;
  Completer<void>? saveGate;
  Completer<TransferSnapshot?>? statusGate;
  void Function()? onWatch;
  late final statusEvents = StreamController<TransferSnapshot?>(
    sync: true,
    onListen: () => watchCalls++,
    onCancel: () => unsubscribeCalls++,
  );
  int watchCalls = 0, unsubscribeCalls = 0, statusCalls = 0;
  int inboxCalls = 0, deviceCalls = 0, cancelCalls = 0, closeCalls = 0;
  List<GuiDeviceDto> deviceItems = [];
  List<Received> receivedItems = [];
  @override
  void configureLifecycle(Future<bool> Function() confirmClose) {}
  @override
  Future<ApiBootstrap> initialize() async => ApiBootstrap(
    version: 'test',
    ipv4: [],
    ipv6: [],
    platform: 'test',
    settingsSaved: true,
    settings: settings,
    deviceId: 'self',
  );
  @override
  Future<Selection?> chooseSource() async => chooseGate != null
      ? await chooseGate!.future
      : Selection(id: 'source', name: 'fixture.bin', size: BigInt.from(12));
  @override
  Future<void> start(StartRequest request) async {
    started = request;
    if (failure case final e?) throw e;
    await startGate?.future;
    snapshot = transferSnapshot();
  }

  @override
  Future<TransferSnapshot?> status() async {
    statusCalls++;
    return statusGate == null ? snapshot : await statusGate!.future;
  }

  @override
  Stream<TransferSnapshot?> watchStatus() {
    onWatch?.call();
    return statusEvents.stream;
  }

  @override
  Future<void> cancel() async {
    cancelCalls++;
    snapshot = transferSnapshot(
      phase: TransferPhase.cancelled,
      done: true,
      cancelled: true,
    );
  }

  @override
  Future<List<Received>> inbox() async {
    inboxCalls++;
    return receivedItems;
  }

  @override
  Future<bool> exportReceived(Received item) async =>
      exportGate == null ? true : await exportGate!.future;
  @override
  Future<void> remove(String id) async {
    receivedItems = receivedItems.where((item) => item.id != id).toList();
  }

  @override
  Future<List<GuiDeviceDto>> devices() async {
    deviceCalls++;
    return deviceItems;
  }

  @override
  Future<void> renameDevice(String id, String name) async {}
  @override
  Future<void> removeDevice(String id) async {}
  @override
  Future<List<String>> logs() async => [];
  @override
  Future<String> probeServer(String server) async => 'fingerprint';
  @override
  Future<void> trustServer(String server, String fingerprint) async {}
  @override
  Future<void> saveSettings(ApiSettings settings) async {
    await saveGate?.future;
    if (failure case final e?) throw e;
    saved = settings;
  }

  @override
  Future<void> close() async {
    closeCalls++;
  }
}

TransferSnapshot transferSnapshot({
  TransferPhase phase = TransferPhase.sending,
  bool done = false,
  bool cancelled = false,
}) => TransferSnapshot(
  phase: phase,
  phaseMessage: '',
  bytes: BigInt.from(12),
  total: BigInt.from(12),
  elapsed: 1,
  done: done,
  cancelled: cancelled,
);
