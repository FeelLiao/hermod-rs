import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/core_bridge.dart';
import 'package:hermod_app/src/rust/api/events.dart';
import 'package:hermod_app/src/rust/frb_generated.dart';

class MockRustApi implements RustLibApi {
  final events = StreamController<GuiEvent>.broadcast();
  final released = <String>[];
  final acknowledged = <BigInt>[];
  final unsubscribed = <String>[];
  Selection? next;
  bool failRelease = false;
  TransferSnapshot? snapshot;
  @override
  Future<TransferSnapshot?> crateApiStatus() async => snapshot;

  @override
  Future<Selection?> crateApiPlatformWindowsPickSource({
    required BigInt windowHandle,
  }) async => next;
  @override
  Future<void> crateApiPlatformReleaseWindowsSource({
    required String selectionId,
  }) async {
    if (failRelease) throw const CoreException('storage', 'cleanup failed');
    released.add(selectionId);
  }

  @override
  Stream<GuiEvent> crateApiEventsSubscribe({required String subscriptionId}) =>
      events.stream;
  @override
  Future<void> crateApiEventsAcknowledge({
    required String subscriptionId,
    required BigInt sequence,
  }) async {
    acknowledged.add(sequence);
  }

  @override
  Future<void> crateApiEventsUnsubscribe({
    required String subscriptionId,
  }) async {
    unsubscribed.add(subscriptionId);
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late MockRustApi api;
  setUp(() {
    api = MockRustApi();
    RustLib.initMock(api: api);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('hermod/platform'),
          (call) async => call.method == 'getWindowHandle' ? 1 : null,
        );
  });
  tearDown(() async {
    await api.events.close();
    RustLib.dispose();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('hermod/platform'), null);
  });

  test('picker cancellation preserves owned source; replacement releases only old id', () async {
    final bridge = CoreBridge();
    api.next = const Selection(id: 'first', name: 'first.txt');
    expect((await bridge.chooseSource())?.id, 'first');
    api.next = null;
    expect(await bridge.chooseSource(), isNull);
    expect(api.released, isEmpty);
    api.next = const Selection(id: 'second', name: 'second.txt');
    expect((await bridge.chooseSource())?.id, 'second');
    expect(api.released, ['first']);
    final terminal = TransferSnapshot(
      phase: TransferPhase.values.last,
      phaseMessage: 'done',
      bytes: BigInt.zero,
      elapsed: 0,
      done: true,
      cancelled: false,
    );
    final subscription = bridge.watchStatus().listen((_) {});
    await Future<void>.delayed(Duration.zero);
    api.events.add(GuiEvent(sequence: BigInt.one, status: terminal));
    await Future<void>.delayed(Duration.zero);
    expect(api.released, ['first', 'second']);
    await subscription.cancel();
  }, skip: !Platform.isWindows);

  test('cleanup failure still publishes and acknowledges terminal; status retries retained source', () async {
    final bridge = CoreBridge();
    api.next = const Selection(id: 'owned', name: 'file.txt');
    await bridge.chooseSource();
    api.failRelease = true;
    final terminal = TransferSnapshot(
      phase: TransferPhase.values.last,
      phaseMessage: 'done',
      bytes: BigInt.zero,
      elapsed: 0,
      done: true,
      cancelled: false,
    );
    api.snapshot = terminal;
    final values = <TransferSnapshot?>[];
    final errors = <Object>[];
    final subscription = bridge.watchStatus().listen(
      values.add,
      onError: errors.add,
    );
    await Future<void>.delayed(Duration.zero);
    api.events.add(GuiEvent(sequence: BigInt.one, status: terminal));
    await Future<void>.delayed(Duration.zero);
    expect(values, [terminal]);
    expect(errors, hasLength(1));
    expect(api.acknowledged, [BigInt.one]);
    expect(await bridge.status(), terminal);
    expect(api.released, isEmpty);
    api.failRelease = false;
    expect(await bridge.status(), terminal);
    expect(api.released, ['owned']);
    await subscription.cancel();
  }, skip: !Platform.isWindows);

  test('watch acknowledges processed events, rejects old sequences and only unsubscribes on cancel', () async {
    final bridge = CoreBridge();
    final values = <TransferSnapshot?>[];
    final subscription = bridge.watchStatus().listen(values.add);
    await Future<void>.delayed(Duration.zero);
    api.events.add(GuiEvent(sequence: BigInt.two));
    api.events.add(GuiEvent(sequence: BigInt.one));
    await Future<void>.delayed(Duration.zero);
    expect(values, [null]);
    expect(api.acknowledged, [BigInt.two, BigInt.one]);
    await subscription.cancel();
    expect(api.unsubscribed, hasLength(1));
    expect(api.released, isEmpty);
  });
}
