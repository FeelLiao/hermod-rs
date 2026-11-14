import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';

/// Official component-scoped Material 3 Expressive; there is no global switch.
/// One offline variable font unifies Chinese and Latin metrics across platforms.
ThemeData buildHermodTheme(Brightness brightness, {TargetPlatform? platform}) {
  final target = platform ?? defaultTargetPlatform;
  final scheme = ColorScheme.fromSeed(
    seedColor: const Color(0xFF276A56),
    brightness: brightness,
  );
  final base = ThemeData(
    brightness: brightness,
    platform: target,
    colorScheme: scheme,
    fontFamily: 'HermodSansSC',
    fontFamilyFallback: const ['Noto Sans', 'sans-serif'],
  );
  final text = base.textTheme.copyWith(
    headlineMedium: base.textTheme.headlineMedium?.copyWith(
      fontWeight: FontWeight.w600,
      fontVariations: const [FontVariation('wght', 600)],
      letterSpacing: -.5,
      height: 1.25,
    ),
    headlineSmall: base.textTheme.headlineSmall?.copyWith(
      fontWeight: FontWeight.w600,
      fontVariations: const [FontVariation('wght', 600)],
      letterSpacing: -.3,
      height: 1.3,
    ),
    titleLarge: base.textTheme.titleLarge?.copyWith(
      fontWeight: FontWeight.w600,
      fontVariations: const [FontVariation('wght', 600)],
      letterSpacing: -.2,
      height: 1.35,
    ),
    titleMedium: base.textTheme.titleMedium?.copyWith(
      fontWeight: FontWeight.w600,
      fontVariations: const [FontVariation('wght', 600)],
      letterSpacing: 0,
      height: 1.45,
    ),
    titleSmall: base.textTheme.titleSmall?.copyWith(
      fontWeight: FontWeight.w600,
      fontVariations: const [FontVariation('wght', 600)],
      letterSpacing: 0,
      height: 1.4,
    ),
    bodyLarge: base.textTheme.bodyLarge?.copyWith(
      letterSpacing: 0,
      height: 1.55,
    ),
    bodyMedium: base.textTheme.bodyMedium?.copyWith(
      letterSpacing: 0,
      height: 1.5,
    ),
    bodySmall: base.textTheme.bodySmall?.copyWith(
      letterSpacing: .1,
      height: 1.5,
    ),
    labelLarge: base.textTheme.labelLarge?.copyWith(
      fontWeight: FontWeight.w600,
      fontVariations: const [FontVariation('wght', 600)],
      letterSpacing: .1,
    ),
  );
  final shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(28));
  return base.copyWith(
    textTheme: text,
    scaffoldBackgroundColor: scheme.surface,
    appBarTheme: AppBarTheme(
      backgroundColor: scheme.surface,
      foregroundColor: scheme.onSurface,
      elevation: 0,
      scrolledUnderElevation: 0,
      centerTitle: false,
      titleTextStyle: text.titleLarge,
      toolbarHeight: 56,
    ),
    cardTheme: CardThemeData(
      color: scheme.surfaceContainerLow,
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: shape,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: scheme.surfaceContainerLowest,
      contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 18),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: scheme.outlineVariant),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: scheme.outlineVariant),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: scheme.primary, width: 2),
      ),
      labelStyle: text.bodyMedium?.copyWith(color: scheme.onSurfaceVariant),
    ),
    iconButtonTheme: const IconButtonThemeData(
      variant: StyleVariant.material3Expressive,
    ),
    elevatedButtonTheme: ElevatedButtonThemeData(
      variant: StyleVariant.material3Expressive,
      style: ElevatedButton.styleFrom(
        sizeVariant: ButtonSizeVariant.medium,
        shapeVariant: ButtonShapeVariant.round,
        textStyle: text.titleSmall,
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size(56, 56),
        padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 18),
        shape: const StadiumBorder(),
        textStyle: text.titleSmall,
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        minimumSize: const Size(48, 48),
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 14),
        side: BorderSide(color: scheme.outlineVariant),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        textStyle: text.labelLarge,
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        minimumSize: const Size(48, 48),
        textStyle: text.labelLarge,
      ),
    ),
    navigationRailTheme: NavigationRailThemeData(
      backgroundColor: scheme.surfaceContainerLow,
      indicatorColor: scheme.primaryContainer,
      selectedLabelTextStyle: text.labelLarge?.copyWith(
        color: scheme.onPrimaryContainer,
      ),
      unselectedLabelTextStyle: text.labelLarge?.copyWith(
        color: scheme.onSurfaceVariant,
      ),
      selectedIconTheme: IconThemeData(color: scheme.onPrimaryContainer),
      unselectedIconTheme: IconThemeData(color: scheme.onSurfaceVariant),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: scheme.surfaceContainerLow,
      indicatorColor: scheme.primaryContainer,
      elevation: 0,
    ),
    dividerTheme: DividerThemeData(
      color: scheme.outlineVariant,
      thickness: 1,
      space: 24,
    ),
    listTileTheme: ListTileThemeData(
      contentPadding: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
      titleTextStyle: text.titleMedium,
      subtitleTextStyle: text.bodyMedium?.copyWith(
        color: scheme.onSurfaceVariant,
      ),
    ),
  );
}
