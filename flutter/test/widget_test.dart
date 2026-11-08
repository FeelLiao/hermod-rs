import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/main.dart';

void main() {
  testWidgets('renders application startup shell', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    await tester.pumpWidget(const HermodApp());
    await tester.pump();

    expect(find.byType(NavigationRail), findsOneWidget);
    expect(find.text('传输'), findsOneWidget);
    expect(find.text('收件箱'), findsOneWidget);
    expect(find.text('设置'), findsOneWidget);
    expect(find.text('日志'), findsOneWidget);
  });
}
