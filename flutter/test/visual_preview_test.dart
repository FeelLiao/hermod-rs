// Reproducible UI previews with fake business state; no native library loaded.
import 'dart:io';
import 'dart:ui' as ui;

import 'package:material_ui/material_ui.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hermod_app/app_controller.dart';
import 'package:hermod_app/main.dart';

import 'fake_gateway.dart';

void main() {
  for (final sample in [
    ('desktop', const Size(1280, 900), false, 1.0),
    ('mobile', const Size(390, 844), false, 1.0),
    ('dark', const Size(1280, 900), true, 1.0),
    ('large-text', const Size(390, 1000), false, 2.0),
  ]) {
    testWidgets('visual preview ${sample.$1}', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      await tester.binding.setSurfaceSize(sample.$2);
      tester.platformDispatcher.platformBrightnessTestValue = sample.$3
          ? Brightness.dark
          : Brightness.light;
      tester.platformDispatcher.textScaleFactorTestValue = sample.$4;
      addTearDown(() {
        tester.binding.setSurfaceSize(null);
        tester.platformDispatcher.clearPlatformBrightnessTestValue();
        tester.platformDispatcher.clearTextScaleFactorTestValue();
      });
      if (const bool.fromEnvironment('SAVE_UI_PREVIEWS')) {
        await tester.runAsync(() async {
          final font = File(
            'assets/fonts/noto-sans-sc/NotoSansSC-Variable.ttf',
          );
          if (await font.exists()) {
            final loader = FontLoader('HermodSansSC')
              ..addFont(
                font.readAsBytes().then((bytes) => bytes.buffer.asByteData()),
              );
            await loader.load();
          }
        });
      }
      final controller = HermodController(gateway: FakeGateway());
      addTearDown(controller.dispose);
      final boundary = GlobalKey();
      await tester.pumpWidget(
        RepaintBoundary(
          key: boundary,
          child: HermodApp(controller: controller),
        ),
      );
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      if (sample.$1 == 'desktop') {
        expect(find.byType(AppBar), findsNothing);
        final selector = find.byKey(const ValueKey('transfer-direction'));
        expect(
          tester.getTopLeft(selector).dy,
          lessThan(140),
          reason: 'Do not stack oversized header and top padding.',
        );
      }
      if (const bool.fromEnvironment('SAVE_UI_PREVIEWS')) {
        await tester.runAsync(() async {
          final image =
              await (boundary.currentContext!.findRenderObject()
                      as RenderRepaintBoundary)
                  .toImage(pixelRatio: 1);
          final data = await image.toByteData(format: ui.ImageByteFormat.png);
          final directory = Directory('build/ui-previews');
          await directory.create(recursive: true);
          await File('${directory.path}/${sample.$1}.png')
              .writeAsBytes(data!.buffer.asUint8List());
          image.dispose();
        });
      }
      debugDefaultTargetPlatformOverride = null;
    });
  }
}
