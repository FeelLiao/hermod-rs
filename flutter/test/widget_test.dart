import 'dart:async';

import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/app_controller.dart';
import 'package:hermod_app/core_bridge.dart';
import 'package:hermod_app/main.dart';
import 'package:hermod_app/l10n/app_localizations.dart';
import 'package:hermod_app/transfer_status_widget.dart';

import 'fake_gateway.dart';

void main() {
  testWidgets(
    'narrow navigation, file send and default pairing use fake only',
    (tester) async {
      final fake = FakeGateway();
      final controller = HermodController(gateway: fake);
      addTearDown(controller.dispose);
      await tester.binding.setSurfaceSize(const Size(390, 844));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(HermodApp(controller: controller));
      await tester.pumpAndSettle();
      expect(find.byType(NavigationBar), findsOneWidget);
      expect(find.byKey(const ValueKey('transfer-direction')), findsOneWidget);
      expect(find.text('文本'), findsNothing);
      await tester.tap(find.text('Choose file'));
      await tester.pumpAndSettle();
      expect(find.text('fixture.bin'), findsOneWidget);
      await tester.tap(find.text('Generate transfer code'));
      await tester.pumpAndSettle();
      expect(fake.started?.sourceId, 'source');
      expect(fake.started?.pair, isTrue);
      expect(fake.started?.text, isNull);
      expect(find.text('Generate transfer code'), findsNothing);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(fake.cancelCalls, 1);
      expect(find.text('Cancelled'), findsOneWidget);
    },
  );

  testWidgets(
    'pending file picker exposes cancellation despite the operation lock',
    (tester) async {
      final fake = FakeGateway()..chooseGate = Completer<Selection?>();
      final controller = HermodController(gateway: fake);
      addTearDown(controller.dispose);
      await tester.pumpWidget(HermodApp(controller: controller));
      await tester.pumpAndSettle();
      expect(find.text('Cancel file operation'), findsNothing);
      await tester.tap(find.text('Choose file'));
      await tester.pump();
      expect(find.text('Cancel file operation'), findsOneWidget);
      await tester.tap(find.text('Cancel file operation'));
      await tester.pump();
      expect(fake.cancelCalls, 1);
      fake.chooseGate!.complete(null);
      await tester.pumpAndSettle();
      expect(find.text('Cancel file operation'), findsNothing);
      expect(controller.operating, isFalse);
    },
  );

  testWidgets('wide navigation exposes devices, inbox and settings', (
    tester,
  ) async {
    final fake = FakeGateway();
    final controller = HermodController(gateway: fake);
    addTearDown(controller.dispose);
    await tester.binding.setSurfaceSize(const Size(1280, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(HermodApp(controller: controller));
    await tester.pumpAndSettle();
    expect(find.byType(NavigationRail), findsOneWidget);
    await tester.tap(find.text('Devices'));
    await tester.pumpAndSettle();
    expect(find.text('self'), findsOneWidget);
    await tester.tap(find.text('Inbox'));
    await tester.pumpAndSettle();
    expect(find.text('No verified files'), findsOneWidget);
    await tester.tap(find.text('Settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save settings'));
    await tester.pumpAndSettle();
    expect(fake.saved, isNotNull);
    expect(find.text('Settings saved'), findsOneWidget);
  });

  testWidgets('saved appearance and language apply to the root app', (
    tester,
  ) async {
    final fake = FakeGateway();
    final controller = HermodController(gateway: fake);
    addTearDown(controller.dispose);
    await tester.binding.setSurfaceSize(const Size(1280, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(HermodApp(controller: controller));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('settings-theme')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Dark').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('settings-language')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('简体中文').last);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save settings'));
    await tester.pumpAndSettle();
    final app = tester.widget<MaterialApp>(find.byType(MaterialApp));
    expect(app.themeMode, ThemeMode.dark);
    expect(app.locale, const Locale('zh'));
    expect(find.text('保存设置'), findsOneWidget);
  });

  testWidgets(
    'IP address family is localized and saved without changing destination',
    (tester) async {
      final fake = FakeGateway();
      final controller = HermodController(gateway: fake);
      addTearDown(controller.dispose);
      await tester.binding.setSurfaceSize(const Size(1280, 1200));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(HermodApp(controller: controller));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Settings'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Advanced network tuning'));
      await tester.pumpAndSettle();
      final family = find.byKey(const ValueKey('settings-ip-family'));
      await tester.ensureVisible(family);
      await tester.tap(family);
      await tester.pumpAndSettle();
      await tester.tap(find.text('IPv6 only').last);
      await tester.pumpAndSettle();
      final save = find.text('Save settings');
      await tester.ensureVisible(save);
      await tester.tap(save);
      await tester.pumpAndSettle();
      expect(fake.saved?.ipFamily, 'ipv6');
      expect(fake.saved?.destination, fake.settings.destination);
      expect(fake.saved?.connection, fake.settings.connection);
    },
  );

  testWidgets(
    'compact status keeps phase, code, cancellation, errors and digest',
    (tester) async {
      var cancelled = false;
      await tester.pumpWidget(
        MaterialApp(
          locale: const Locale('en'),
          localizationsDelegates: [
            AppLocalizations.delegate,
            ...GlobalMaterialLocalizations.delegates,
          ],
          supportedLocales: AppLocalizations.supportedLocales,
          home: Scaffold(
            body: TransferStatusCard(
              quiet: true,
              status: TransferSnapshot(
                phase: TransferPhase.sending,
                phaseMessage: '',
                code: 'safe-transfer-code',
                bytes: BigInt.from(12),
                total: BigInt.from(24),
                elapsed: 1,
                done: false,
                cancelled: false,
                error: 'Peer connection interrupted',
                digest: 'verified-digest',
                transport: 'QUIC',
                rttMs: 42,
              ),
              onCancel: () => cancelled = true,
            ),
          ),
        ),
      );
      expect(find.text('Sending'), findsOneWidget);
      expect(find.text('safe-transfer-code'), findsOneWidget);
      expect(find.text('Peer connection interrupted'), findsOneWidget);
      expect(find.text('SHA-256: verified-digest'), findsOneWidget);
      expect(find.textContaining('bytes'), findsNothing);
      expect(find.textContaining('42'), findsNothing);
      expect(find.byType(LinearProgressIndicator), findsNothing);
      await tester.tap(find.text('Cancel'));
      expect(cancelled, isTrue);
    },
  );

  testWidgets('transfer page uses the frozen task quiet setting', (
    tester,
  ) async {
    final fake = FakeGateway();
    final controller = HermodController(gateway: fake);
    addTearDown(controller.dispose);
    await controller.initialize();
    final initial = fake.settings;
    final compact = ApiSettings(
      version: initial.version,
      connection: initial.connection,
      receiveTransport: initial.receiveTransport,
      ipFamily: initial.ipFamily,
      server: initial.server,
      words: initial.words,
      verbose: initial.verbose,
      quiet: true,
      theme: initial.theme,
      language: initial.language,
      destination: initial.destination,
      tuning: initial.tuning,
    );
    await controller.saveSettings(compact);
    await controller.receive('code');
    // A settings edit affects the next task, not this one's presentation.
    await controller.saveSettings(initial);
    await tester.pumpWidget(HermodApp(controller: controller));
    await tester.pumpAndSettle();
    expect(
      tester.widget<TransferStatusCard>(find.byType(TransferStatusCard)).quiet,
      isTrue,
    );
    expect(find.text('Sending'), findsOneWidget);
    expect(find.text('Cancel'), findsOneWidget);
    expect(find.textContaining('bytes/s'), findsNothing);
  });

  testWidgets('cancelled full-byte task never renders 100 percent success', (
    tester,
  ) async {
    final fake = FakeGateway()
      ..snapshot = transferSnapshot(
        phase: TransferPhase.cancelled,
        done: true,
        cancelled: true,
      );
    final controller = HermodController(gateway: fake);
    addTearDown(controller.dispose);
    await tester.pumpWidget(HermodApp(controller: controller));
    await tester.pumpAndSettle();
    final card = find.byType(TransferStatusCard);
    final indicator = tester.widget<LinearProgressIndicator>(
      find.descendant(of: card, matching: find.byType(LinearProgressIndicator)),
    );
    expect(indicator.value, lessThan(1));
    expect(find.text('Cancelled'), findsOneWidget);
  });
}
