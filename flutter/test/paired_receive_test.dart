import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/app_controller.dart';
import 'package:hermod_app/core_bridge.dart';
import 'package:hermod_app/main.dart';

import 'fake_gateway.dart';

void main() {
  testWidgets(
    'Receive offers only authenticated paired devices and connects by id without a code',
    (tester) async {
      final fake = FakeGateway()
        ..deviceItems = [
          GuiDeviceDto(
            deviceId: 'trusted-peer',
            name: 'Laptop',
            createdAt: BigInt.zero,
            updatedAt: BigInt.zero,
            active: true,
          ),
          GuiDeviceDto(
            deviceId: 'pending-peer',
            name: 'Pending',
            createdAt: BigInt.zero,
            updatedAt: BigInt.zero,
            active: false,
          ),
        ];
      final controller = HermodController(gateway: fake);
      addTearDown(controller.dispose);
      await tester.binding.setSurfaceSize(const Size(1280, 1000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(HermodApp(controller: controller));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Receive'));
      await tester.pumpAndSettle();
      expect(find.text('Laptop'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('receive-paired-pending-peer')),
        findsNothing,
      );
      final button = find.byKey(const ValueKey('receive-paired-trusted-peer'));
      await tester.ensureVisible(button);
      await tester.tap(button);
      await tester.pumpAndSettle();
      expect(fake.started?.fromDevice, 'trusted-peer');
      expect(fake.started?.code, isNull);
      expect(fake.started?.pair, isFalse);
      expect(find.byKey(const ValueKey('paired-receive-panel')), findsNothing);
      expect(find.text('Cancel'), findsOneWidget);
    },
  );
}
