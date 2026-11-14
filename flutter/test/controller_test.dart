import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/app_controller.dart';
import 'package:hermod_app/core_bridge.dart';

import 'fake_gateway.dart';

void main() {
  late FakeGateway fake;
  late HermodController controller;
  setUp(() {
    fake = FakeGateway();
    controller = HermodController(gateway: fake);
  });
  tearDown(() => controller.dispose());

  test(
    'code and device receiving are typed, paired, never legacy text',
    () async {
      await controller.initialize();
      await controller.receive('test-code');
      expect(fake.started?.code, 'test-code');
      expect(fake.started?.pair, isTrue);
      expect(fake.started?.text, isNull);
      await controller.cancel();
      await controller.receiveFromDevice('peer-id');
      expect(fake.started?.fromDevice, 'peer-id');
      expect(fake.started?.pair, isFalse);
      expect(fake.started?.code, isNull);
    },
  );

  test(
    'file cancellation bypasses operation guard and rejects late selection',
    () async {
      await controller.initialize();
      await controller.chooseFile();
      final previous = controller.selection;
      fake.chooseGate = Completer<Selection?>();
      final choosing = controller.chooseFile();
      expect(controller.operating, isTrue);
      expect(controller.fileOperating, isTrue);
      expect(await controller.cancelFileOperation(), isTrue);
      expect(fake.cancelCalls, 1);
      fake.chooseGate!.complete(
        Selection(id: 'late', name: 'late.bin', size: BigInt.one),
      );
      await choosing;
      expect(controller.fileOperating, isFalse);
      expect(controller.operating, isFalse);
      expect(controller.selection, same(previous));
      expect(await controller.cancelFileOperation(), isFalse);
    },
  );

  test(
    'export is cancellable and always releases the file operation flag',
    () async {
      await controller.initialize();
      fake.exportGate = Completer<bool>();
      final exporting = controller.exportItem(
        Received(
          id: 'item',
          name: 'file.bin',
          size: BigInt.one,
          sha256: 'digest',
        ),
      );
      expect(controller.fileOperating, isTrue);
      expect(await controller.cancelFileOperation(), isTrue);
      expect(fake.cancelCalls, 1);
      fake.exportGate!.complete(false);
      await exporting;
      expect(controller.fileOperating, isFalse);
      expect(controller.operating, isFalse);
    },
  );

  test('settings persistence is not a cancellable file operation', () async {
    await controller.initialize();
    fake.saveGate = Completer<void>();
    final saving = controller.saveSettings(fake.settings);
    expect(controller.operating, isTrue);
    expect(controller.fileOperating, isFalse);
    expect(await controller.cancelFileOperation(), isFalse);
    expect(fake.cancelCalls, 0);
    fake.saveGate!.complete();
    await saving;
  });

  test('duplicate starts are blocked and task settings are frozen', () async {
    await controller.initialize();
    fake.startGate = Completer<void>();
    final first = controller.receive('first');
    expect(controller.operating, isTrue);
    expect(await controller.receive('second'), isFalse);
    expect(fake.started?.code, 'first');
    fake.startGate!.complete();
    await first;
    expect(controller.taskSettings, same(controller.settings));
    expect(controller.canStart, isFalse);
  });

  test('terminal snapshot refreshes inbox once', () async {
    await controller.initialize();
    final before = fake.inboxCalls;
    fake.snapshot = transferSnapshot(
      phase: TransferPhase.completed,
      done: true,
    );
    await controller.refreshStatus();
    await controller.refreshStatus();
    expect(fake.inboxCalls, before + 1);
  });

  test(
    'stable error code and retryability survive the controller boundary',
    () async {
      await controller.initialize();
      fake.failure = const CoreException(
        'peerUnavailable',
        'Details',
        retryable: true,
      );
      expect(await controller.receive('code'), isFalse);
      expect(controller.error?.code, 'peerUnavailable');
      expect(controller.error?.retryable, isTrue);
      expect(controller.error?.message, 'Details');
    },
  );

  test('save reports success only after persistence succeeds', () async {
    await controller.initialize();
    fake.failure = const CoreException('invalidSettings', 'Bad MTU');
    expect(await controller.saveSettings(fake.settings), isFalse);
    expect(fake.saved, isNull);
    expect(controller.settings, same(fake.settings));
  });

  test('dispose during start is safe', () async {
    final gateway = FakeGateway();
    final state = HermodController(gateway: gateway);
    await state.initialize();
    gateway.startGate = Completer<void>();
    final pending = state.receive('code');
    state.dispose();
    gateway.startGate!.complete();
    expect(await pending, isFalse);
    expect(gateway.unsubscribeCalls, 1);
    expect(gateway.cancelCalls, 0);
    expect(gateway.closeCalls, 0);
  });

  test(
    'stream updates status without polling and terminal clears source once',
    () async {
      await controller.initialize();
      await controller.chooseFile();
      await controller.sendFile();
      final inboxBefore = fake.inboxCalls;
      final devicesBefore = fake.deviceCalls;
      final readsBefore = fake.statusCalls;
      final terminal = transferSnapshot(
        phase: TransferPhase.completed,
        done: true,
      );
      fake.statusEvents.add(terminal);
      fake.statusEvents.add(terminal);
      expect(controller.status, same(terminal));
      expect(controller.selection, isNull);
      expect(controller.canStart, isTrue);
      await Future<void>.delayed(Duration.zero);
      expect(fake.inboxCalls, inboxBefore + 1);
      expect(fake.deviceCalls, devicesBefore + 1);
      expect(fake.statusCalls, readsBefore);
      expect(fake.watchCalls, 1);
    },
  );

  test('initial snapshot is retained without a stream event', () async {
    final initial = transferSnapshot();
    fake.snapshot = initial;
    await controller.initialize();
    expect(controller.status, same(initial));
  });

  test(
    'subscribes before initial status and newer event wins pending read',
    () async {
      fake.statusGate = Completer<TransferSnapshot?>();
      final initializing = controller.initialize();
      await Future<void>.delayed(Duration.zero);
      expect(fake.watchCalls, 1);
      expect(fake.statusCalls, 1);
      final current = transferSnapshot(phase: TransferPhase.receiving);
      fake.statusEvents.add(current);
      fake.statusGate!.complete(transferSnapshot());
      await initializing;
      expect(controller.status, same(current));
    },
  );

  test('manual stale status cannot overwrite a newer terminal event', () async {
    await controller.initialize();
    await controller.chooseFile();
    fake.statusGate = Completer<TransferSnapshot?>();
    final refreshing = controller.refreshStatus();
    final terminal = transferSnapshot(
      phase: TransferPhase.cancelled,
      done: true,
      cancelled: true,
    );
    fake.statusEvents.add(terminal);
    fake.statusGate!.complete(transferSnapshot());
    await refreshing;
    expect(controller.status, same(terminal));
    expect(controller.selection, isNull);
  });

  test(
    'dispose unsubscribes without shutting down or cancelling a task',
    () async {
      final gateway = FakeGateway();
      final state = HermodController(gateway: gateway);
      await state.initialize();
      await state.receive('code');
      state.dispose();
      await Future<void>.delayed(Duration.zero);
      expect(gateway.unsubscribeCalls, 1);
      expect(gateway.cancelCalls, 0);
      expect(gateway.closeCalls, 0);
      gateway.statusEvents.add(transferSnapshot(done: true));
    },
  );

  test('explicit shutdown unsubscribes then closes the gateway', () async {
    await controller.initialize();
    await controller.shutdown();
    expect(fake.unsubscribeCalls, 1);
    expect(fake.closeCalls, 1);
    expect(fake.cancelCalls, 0);
  });

  test(
    'terminal event during start is not refreshed twice by command read',
    () async {
      await controller.initialize();
      fake.startGate = Completer<void>();
      final starting = controller.receive('code');
      final before = fake.inboxCalls;
      fake.statusEvents.add(transferSnapshot(done: true));
      fake.startGate!.complete();
      await starting;
      expect(controller.status?.done, isTrue);
      expect(fake.inboxCalls, before + 1);
    },
  );
}
