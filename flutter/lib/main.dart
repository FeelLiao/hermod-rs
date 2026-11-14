import 'dart:async';

import 'package:material_ui/material_ui.dart';

import 'app_controller.dart';
import 'features/library/library_pages.dart';
import 'features/transfer/transfer_page.dart';
import 'l10n/app_localizations.dart';
import 'settings_page.dart';
import 'ui/font_licenses.dart';
import 'ui/motion.dart';
import 'ui/theme.dart';

export 'features/library/library_pages.dart';
export 'features/transfer/transfer_page.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  registerFontLicenses();
  runApp(const HermodApp());
}

class HermodApp extends StatefulWidget {
  const HermodApp({super.key, this.controller});
  final HermodController? controller;
  @override
  State<HermodApp> createState() => _HermodAppState();
}

class _HermodAppState extends State<HermodApp> {
  late final controller = widget.controller ?? HermodController();
  @override
  void initState() {
    super.initState();
    unawaited(controller.initialize());
  }

  @override
  void dispose() {
    if (widget.controller == null) controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final settings = controller.settings;
      final language = settings?.language;
      return MaterialApp(
        debugShowCheckedModeBanner: false,
        onGenerateTitle: (context) => AppLocalizations.of(context).appTitle,
        localizationsDelegates: const [
          AppLocalizations.delegate,
          ...GlobalMaterialLocalizations.delegates,
        ],
        supportedLocales: AppLocalizations.supportedLocales,
        locale: language == 'zh-CN' || language == 'zh'
            ? const Locale('zh')
            : language == 'en-US' || language == 'en'
            ? const Locale('en')
            : null,
        themeMode: switch (settings?.theme) {
          'light' => ThemeMode.light,
          'dark' => ThemeMode.dark,
          _ => ThemeMode.system,
        },
        theme: buildHermodTheme(Brightness.light),
        darkTheme: buildHermodTheme(Brightness.dark),
        home: HermodShell(controller: controller),
      );
    },
  );
}

class HermodShell extends StatefulWidget {
  const HermodShell({super.key, required this.controller});
  final HermodController controller;
  @override
  State<HermodShell> createState() => _HermodShellState();
}

class _HermodShellState extends State<HermodShell> {
  int destination = 0;
  bool _closing = false;
  @override
  void initState() {
    super.initState();
    widget.controller.configureLifecycle(() async {
      if (_closing || !mounted) return false;
      _closing = true;
      try {
        if (!widget.controller.active) return true;
        final l = AppLocalizations.of(context);
        return await showDialog<bool>(
              context: context,
              builder: (context) => AlertDialog(
                title: Text(l.closeTitle),
                content: Text(l.closeHint),
                actions: [
                  TextButton(
                    onPressed: () => Navigator.pop(context, false),
                    child: Text(l.cancel),
                  ),
                  FilledButton(
                    onPressed: () => Navigator.pop(context, true),
                    child: Text(l.stopClose),
                  ),
                ],
              ),
            ) ??
            false;
      } finally {
        _closing = false;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.controller;
    final l = AppLocalizations.of(context);
    final labels = [l.transfer, l.devices, l.inbox, l.settings];
    const icons = [
      Icons.swap_horiz,
      Icons.devices_outlined,
      Icons.inbox_outlined,
      Icons.tune,
    ];
    return LayoutBuilder(
      builder: (context, constraints) {
        final wide = constraints.maxWidth >= 840;
        return Scaffold(
          appBar: wide ? null : AppBar(title: Text(labels[destination])),
          body: SafeArea(
            child: Row(
              children: [
                if (wide)
                  NavigationRail(
                    key: const ValueKey('desktop-navigation'),
                    minExtendedWidth: 224,
                    selectedIndex: destination,
                    extended: constraints.maxWidth >= 1000,
                    onDestinationSelected: (index) =>
                        setState(() => destination = index),
                    leading: Padding(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 16,
                        vertical: 8,
                      ),
                      child: Text(l.appTitle),
                    ),
                    destinations: [
                      for (var i = 0; i < labels.length; i++)
                        NavigationRailDestination(
                          icon: Icon(icons[i]),
                          label: Text(labels[i]),
                        ),
                    ],
                  ),
                Expanded(
                  key: const ValueKey('page-content'),
                  child: Center(
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 1000),
                      child: Padding(
                        padding: EdgeInsets.fromLTRB(
                          wide ? 28 : 16,
                          12,
                          wide ? 28 : 16,
                          16,
                        ),
                        child: Column(
                          children: [
                            if (c.error case final error?)
                              MaterialBanner(
                                leading: const Icon(Icons.error_outline),
                                content: Text(
                                  '${l.errorTitle}\n${l.errorDetails(error.code, error.message)}${error.retryable ? '\n${l.retryable}' : ''}',
                                ),
                                actions: [
                                  TextButton(
                                    onPressed: c.operating
                                        ? null
                                        : () => c.bootstrap == null
                                              ? c.initialize()
                                              : c.refreshAll(),
                                    child: Text(l.retry),
                                  ),
                                ],
                              ),
                            if (c.fileOperating)
                              Align(
                                alignment: Alignment.centerRight,
                                child: TextButton.icon(
                                  onPressed: c.cancelFileOperation,
                                  icon: const Icon(Icons.close),
                                  label: Text(l.cancelFileOperation),
                                ),
                              ),
                            Expanded(
                              child: c.busy
                                  ? const Center(
                                      child: CircularProgressIndicator(),
                                    )
                                  : MotionSwap(
                                      identity: destination,
                                      child: IndexedStack(
                                        index: destination,
                                        children: [
                                          TransferPage(
                                            key: const ValueKey(
                                              'transfer-page',
                                            ),
                                            controller: c,
                                          ),
                                          DevicesPage(
                                            key: const ValueKey('devices-page'),
                                            controller: c,
                                          ),
                                          InboxPage(
                                            key: const ValueKey('inbox-page'),
                                            controller: c,
                                          ),
                                          EditableSettingsPage(
                                            key: const ValueKey(
                                              'settings-page',
                                            ),
                                            controller: c,
                                          ),
                                        ],
                                      ),
                                    ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
          bottomNavigationBar: wide
              ? null
              : NavigationBar(
                  key: const ValueKey('compact-navigation'),
                  selectedIndex: destination,
                  onDestinationSelected: (index) =>
                      setState(() => destination = index),
                  destinations: [
                    for (var i = 0; i < labels.length; i++)
                      NavigationDestination(
                        icon: Icon(icons[i]),
                        label: labels[i],
                      ),
                  ],
                ),
        );
      },
    );
  }
}
