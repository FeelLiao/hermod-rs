import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/ui/formatters.dart';
import 'package:hermod_app/ui/motion.dart';
import 'package:hermod_app/ui/theme.dart';

void main() {
  test('binary size labels are compact and rates reject invalid values', () {
    expect(formatBytes(BigInt.zero), '0 B');
    expect(formatBytes(BigInt.from(1536)), '1.5 KiB');
    expect(formatBytes(BigInt.from(1048576)), '1.0 MiB');
    expect(formatRate(double.nan), '0 B/s');
  });
  test('Offline typography and official component Expressive variants are configured', () {
    final theme = buildHermodTheme(
      Brightness.light,
      platform: TargetPlatform.windows,
    );
    expect(theme.textTheme.bodyMedium?.fontFamily, 'HermodSansSC');
    expect(theme.elevatedButtonTheme.variant, StyleVariant.material3Expressive);
    expect(theme.iconButtonTheme.variant, StyleVariant.material3Expressive);
    expect(buildHermodTheme(Brightness.dark).brightness, Brightness.dark);
  });
  testWidgets(
    'reduced-motion navigation remains immediate and keeps child state',
    (tester) async {
      Widget frame(int index) => MaterialApp(
        home: MediaQuery(
          data: const MediaQueryData(disableAnimations: true),
          child: Material(
            child: MotionSwap(
              identity: index,
              child: IndexedStack(
                index: index,
                children: const [
                  TextField(key: ValueKey('retained-field')),
                  Text('second'),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pumpWidget(frame(0));
      await tester.enterText(
        find.byKey(const ValueKey('retained-field')),
        'keep me',
      );
      await tester.pumpWidget(frame(1));
      expect(
        tester
            .widget<FadeTransition>(
              find
                  .descendant(
                    of: find.byType(MotionSwap),
                    matching: find.byType(FadeTransition),
                  )
                  .first,
            )
            .opacity
            .value,
        1,
      );
      await tester.pumpWidget(frame(0));
      expect(find.text('keep me'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );
}
