import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/ui/motion.dart';
import 'package:material_ui/material_ui.dart';

void main() {
  Widget frame(int index, {bool reduced = false, bool enabled = true}) =>
      MaterialApp(
        home: MediaQuery(
          data: MediaQueryData(disableAnimations: reduced),
          child: MotionSwap(
            identity: index,
            enabled: enabled,
            child: Material(
              child: IndexedStack(
                index: index,
                children: const [
                  TextField(key: ValueKey('retained-input')),
                  Text('second'),
                ],
              ),
            ),
          ),
        ),
      );

  double opacity(WidgetTester tester) => tester
      .widget<FadeTransition>(
        find.descendant(
          of: find.byType(MotionSwap),
          matching: find.byType(FadeTransition),
        ),
      )
      .opacity
      .value;

  testWidgets('effects spring is bounded, interruptible and settles exactly', (
    tester,
  ) async {
    await tester.pumpWidget(frame(0));
    await tester.enterText(find.byType(TextField), 'retained');
    await tester.pumpWidget(frame(1));
    await tester.pump(const Duration(milliseconds: 24));
    final beforeInterrupt = opacity(tester);
    expect(beforeInterrupt, inInclusiveRange(0.0, 1.0));
    await tester.pumpWidget(frame(0));
    expect(opacity(tester), beforeInterrupt);
    expect(find.text('retained'), findsOneWidget);
    for (var i = 0; i < 40; i++) {
      await tester.pump(const Duration(milliseconds: 16));
      expect(opacity(tester), inInclusiveRange(0.0, 1.0));
    }
    expect(opacity(tester), 1);
    // Retaining focus intentionally retains the TextField caret ticker;
    // unfocus before asserting that the navigation itself schedules no frames.
    tester.binding.focusManager.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    expect(tester.binding.hasScheduledFrame, isFalse);
  });

  testWidgets('reduced motion and quiet disable an in-flight effects spring', (
    tester,
  ) async {
    await tester.pumpWidget(frame(0));
    await tester.pumpWidget(frame(1));
    await tester.pump(const Duration(milliseconds: 16));
    await tester.pumpWidget(frame(1, reduced: true));
    expect(opacity(tester), 1);
    await tester.pumpWidget(frame(0));
    await tester.pump(const Duration(milliseconds: 16));
    await tester.pumpWidget(frame(0, enabled: false));
    expect(opacity(tester), 1);
    await tester.pumpAndSettle();
    expect(tester.binding.hasScheduledFrame, isFalse);
  });
}
