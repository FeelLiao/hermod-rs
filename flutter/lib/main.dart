import 'package:flutter/material.dart';

import 'app_controller.dart';
import 'settings_page.dart';
import 'transfer_status_widget.dart';

void main() {
  runApp(const HermodApp());
}

class HermodApp extends StatelessWidget {
  const HermodApp({super.key});

  @override
  Widget build(BuildContext context) {
    const seed = Color(0xFF006C4C);
    return MaterialApp(
      title: 'HermodTransfer',
      debugShowCheckedModeBanner: false,
      themeMode: ThemeMode.system,
      theme: _theme(Brightness.light, seed),
      darkTheme: _theme(Brightness.dark, seed),
      home: const HermodShell(),
    );
  }

  ThemeData _theme(Brightness brightness, Color seed) {
    final scheme = ColorScheme.fromSeed(
      seedColor: seed,
      brightness: brightness,
      dynamicSchemeVariant: DynamicSchemeVariant.fidelity,
    );
    return ThemeData(
      useMaterial3: true,
      brightness: brightness,
      colorScheme: scheme,
      scaffoldBackgroundColor: scheme.surface,
      cardTheme: CardThemeData(
        elevation: 0,
        margin: EdgeInsets.zero,
        color: scheme.surfaceContainerLow,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(24)),
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: scheme.surfaceContainerHighest.withValues(alpha: .55),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(16),
          borderSide: BorderSide.none,
        ),
      ),
    );
  }
}

enum AppDestination { transfer, inbox, settings, logs }

enum TransferMode { file, text, receive }

class HermodShell extends StatefulWidget {
  const HermodShell({super.key});

  @override
  State<HermodShell> createState() => _HermodShellState();
}

class _HermodShellState extends State<HermodShell> {
  AppDestination destination = AppDestination.transfer;
  final controller = HermodController();

  @override
  void initState() {
    super.initState();
    controller.initialize();
  }

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final wide = constraints.maxWidth >= 840;
        final body = SafeArea(
          child: Padding(
            padding: EdgeInsets.fromLTRB(
              wide ? 20 : 16,
              16,
              wide ? 28 : 16,
              16,
            ),
            child: ListenableBuilder(
              listenable: controller,
              builder: (context, _) {
                if (controller.busy) {
                  return const Center(child: CircularProgressIndicator());
                }
                return Column(
                  children: [
                    if (controller.error case final error?)
                      Padding(
                        padding: const EdgeInsets.only(bottom: 12),
                        child: MaterialBanner(
                          content: Text(error),
                          leading: const Icon(Icons.error_outline),
                          actions: [
                            TextButton(
                              onPressed: () => controller.refreshAll(),
                              child: const Text('刷新'),
                            ),
                          ],
                        ),
                      ),
                    Expanded(
                      child: switch (destination) {
                        AppDestination.transfer => TransferPage(
                          controller: controller,
                        ),
                        AppDestination.inbox => InboxPage(
                          controller: controller,
                        ),
                        AppDestination.settings => EditableSettingsPage(
                          controller: controller,
                        ),
                        AppDestination.logs => LogsPage(controller: controller),
                      },
                    ),
                  ],
                );
              },
            ),
          ),
        );
        return Scaffold(
          body: Row(
            children: [
              if (wide)
                SafeArea(
                  child: Padding(
                    padding: const EdgeInsets.all(12),
                    child: NavigationRail(
                      extended: constraints.maxWidth >= 1180,
                      selectedIndex: destination.index,
                      onDestinationSelected: (index) => setState(
                        () => destination = AppDestination.values[index],
                      ),
                      leading: const _BrandMark(),
                      destinations: const [
                        NavigationRailDestination(
                          icon: Icon(Icons.swap_horiz_rounded),
                          selectedIcon: Icon(Icons.swap_horiz),
                          label: Text('传输'),
                        ),
                        NavigationRailDestination(
                          icon: Icon(Icons.inbox_outlined),
                          selectedIcon: Icon(Icons.inbox),
                          label: Text('收件箱'),
                        ),
                        NavigationRailDestination(
                          icon: Icon(Icons.tune_outlined),
                          selectedIcon: Icon(Icons.tune),
                          label: Text('设置'),
                        ),
                        NavigationRailDestination(
                          icon: Icon(Icons.receipt_long_outlined),
                          selectedIcon: Icon(Icons.receipt_long),
                          label: Text('日志'),
                        ),
                      ],
                    ),
                  ),
                ),
              Expanded(child: body),
            ],
          ),
          bottomNavigationBar: wide
              ? null
              : NavigationBar(
                  selectedIndex: destination.index,
                  onDestinationSelected: (index) => setState(
                    () => destination = AppDestination.values[index],
                  ),
                  destinations: const [
                    NavigationDestination(
                      icon: Icon(Icons.swap_horiz),
                      label: '传输',
                    ),
                    NavigationDestination(
                      icon: Icon(Icons.inbox_outlined),
                      label: '收件箱',
                    ),
                    NavigationDestination(icon: Icon(Icons.tune), label: '设置'),
                    NavigationDestination(
                      icon: Icon(Icons.receipt_long),
                      label: '日志',
                    ),
                  ],
                ),
        );
      },
    );
  }
}

class _BrandMark extends StatelessWidget {
  const _BrandMark();

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.only(bottom: 20),
      child: Tooltip(
        message: 'HermodTransfer',
        child: CircleAvatar(
          radius: 24,
          backgroundColor: scheme.primaryContainer,
          foregroundColor: scheme.onPrimaryContainer,
          child: const Icon(Icons.lock_person_rounded),
        ),
      ),
    );
  }
}

class PageHeading extends StatelessWidget {
  const PageHeading({
    super.key,
    required this.title,
    required this.subtitle,
    this.trailing,
  });
  final String title;
  final String subtitle;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: Theme.of(context).textTheme.headlineMedium),
              const SizedBox(height: 6),
              Text(subtitle, style: Theme.of(context).textTheme.bodyLarge),
            ],
          ),
        ),
        ?trailing,
      ],
    );
  }
}

class TransferPage extends StatefulWidget {
  const TransferPage({super.key, required this.controller});
  final HermodController controller;

  @override
  State<TransferPage> createState() => _TransferPageState();
}

class _TransferPageState extends State<TransferPage> {
  TransferMode mode = TransferMode.file;
  final textController = TextEditingController();
  final codeController = TextEditingController();

  @override
  void dispose() {
    textController.dispose();
    codeController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        PageHeading(
          title: '设备之间，直接相连。',
          subtitle: '端到端加密传输；信令服务器只帮助设备相遇，不中转文件。',
          trailing: _ConnectionBadge(
            connected: widget.controller.bootstrap != null,
          ),
        ),
        const SizedBox(height: 28),
        Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 920),
            child: Card(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    SegmentedButton<TransferMode>(
                      segments: const [
                        ButtonSegment(
                          value: TransferMode.file,
                          icon: Icon(Icons.file_present),
                          label: Text('文件'),
                        ),
                        ButtonSegment(
                          value: TransferMode.text,
                          icon: Icon(Icons.notes),
                          label: Text('文本'),
                        ),
                        ButtonSegment(
                          value: TransferMode.receive,
                          icon: Icon(Icons.download_rounded),
                          label: Text('接收'),
                        ),
                      ],
                      selected: {mode},
                      onSelectionChanged: (value) =>
                          setState(() => mode = value.first),
                    ),
                    const SizedBox(height: 28),
                    AnimatedSwitcher(
                      duration: const Duration(milliseconds: 220),
                      child: switch (mode) {
                        TransferMode.file => _FileComposer(
                          key: const ValueKey('file'),
                          controller: widget.controller,
                        ),
                        TransferMode.text => _TextComposer(
                          key: const ValueKey('text'),
                          controller: textController,
                          onSend: () =>
                              widget.controller.sendText(textController.text),
                        ),
                        TransferMode.receive => _ReceiveComposer(
                          key: const ValueKey('receive'),
                          controller: codeController,
                          onReceive: () => widget.controller.receive(
                            codeController.text.trim(),
                          ),
                        ),
                      },
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
        const SizedBox(height: 20),
        if (widget.controller.status case final status?) ...[
          TransferStatusCard(
            status: status,
            onCancel: widget.controller.cancel,
          ),
          const SizedBox(height: 20),
        ],
        const Center(child: _SecurityNote()),
      ],
    );
  }
}

class _ConnectionBadge extends StatelessWidget {
  const _ConnectionBadge({required this.connected});
  final bool connected;
  @override
  Widget build(BuildContext context) => Chip(
    avatar: Icon(
      connected ? Icons.check_circle_outline : Icons.lan_outlined,
      size: 18,
    ),
    label: Text(connected ? 'Rust Core 已连接' : 'Core 接口待连接'),
    side: BorderSide.none,
  );
}

class _FileComposer extends StatelessWidget {
  const _FileComposer({super.key, required this.controller});
  final HermodController controller;
  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Column(
      key: key,
      children: [
        Container(
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 44),
          decoration: BoxDecoration(
            color: scheme.secondaryContainer.withValues(alpha: .45),
            borderRadius: BorderRadius.circular(20),
            border: Border.all(color: scheme.outlineVariant),
          ),
          child: Column(
            children: [
              Icon(Icons.upload_file_rounded, size: 48, color: scheme.primary),
              const SizedBox(height: 16),
              Text(
                controller.selection?['name'] as String? ?? '选择要发送的文件',
                style: Theme.of(context).textTheme.titleLarge,
              ),
              const SizedBox(height: 8),
              Text(
                controller.selection == null
                    ? '文件内容由 Rust Core 直接读取，不经过 Dart 内存。'
                    : '${controller.selection?['size'] ?? '未知'} 字节',
              ),
              const SizedBox(height: 20),
              FilledButton.icon(
                onPressed: controller.chooseFile,
                icon: const Icon(Icons.folder_open),
                label: const Text('选择文件'),
              ),
            ],
          ),
        ),
        const SizedBox(height: 20),
        _PrimaryAction(
          icon: Icons.key_rounded,
          label: '生成传输码',
          onPressed: controller.selection == null ? null : controller.sendFile,
        ),
      ],
    );
  }
}

class _TextComposer extends StatelessWidget {
  const _TextComposer({
    super.key,
    required this.controller,
    required this.onSend,
  });
  final TextEditingController controller;
  final VoidCallback onSend;
  @override
  Widget build(BuildContext context) => Column(
    key: key,
    children: [
      TextField(
        controller: controller,
        minLines: 7,
        maxLines: 12,
        maxLength: 1024 * 1024,
        decoration: const InputDecoration(
          labelText: '文本内容',
          alignLabelWithHint: true,
          hintText: '输入要安全发送的文本……',
          helperText: '最多 1 MiB；接收为经过校验的文本文件。',
        ),
      ),
      const SizedBox(height: 12),
      _PrimaryAction(
        icon: Icons.key_rounded,
        label: '生成传输码',
        onPressed: onSend,
      ),
    ],
  );
}

class _ReceiveComposer extends StatelessWidget {
  const _ReceiveComposer({
    super.key,
    required this.controller,
    required this.onReceive,
  });
  final TextEditingController controller;
  final VoidCallback onReceive;
  @override
  Widget build(BuildContext context) => Column(
    key: key,
    children: [
      TextField(
        controller: controller,
        decoration: const InputDecoration(
          labelText: '传输码',
          hintText: '请输入发送方提供的完整传输码',
          prefixIcon: Icon(Icons.password_rounded),
        ),
      ),
      const SizedBox(height: 20),
      _PrimaryAction(
        icon: Icons.link_rounded,
        label: '连接并接收',
        onPressed: onReceive,
      ),
    ],
  );
}

class _PrimaryAction extends StatelessWidget {
  const _PrimaryAction({
    required this.icon,
    required this.label,
    required this.onPressed,
  });
  final IconData icon;
  final String label;
  final VoidCallback? onPressed;
  @override
  Widget build(BuildContext context) => SizedBox(
    width: double.infinity,
    child: FilledButton.icon(
      onPressed: onPressed,
      icon: Icon(icon),
      label: Padding(padding: const EdgeInsets.all(14), child: Text(label)),
    ),
  );
}

class _SecurityNote extends StatelessWidget {
  const _SecurityNote();
  @override
  Widget build(BuildContext context) => const Wrap(
    spacing: 18,
    runSpacing: 8,
    alignment: WrapAlignment.center,
    children: [
      _Note(icon: Icons.enhanced_encryption_outlined, text: 'CPace 密钥协商'),
      _Note(icon: Icons.verified_user_outlined, text: 'SHA-256 校验'),
      _Note(icon: Icons.speed_outlined, text: 'QUIC / WireGuard'),
    ],
  );
}

class _Note extends StatelessWidget {
  const _Note({required this.icon, required this.text});
  final IconData icon;
  final String text;
  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [Icon(icon, size: 18), const SizedBox(width: 6), Text(text)],
  );
}

class InboxPage extends StatelessWidget {
  const InboxPage({super.key, required this.controller});
  final HermodController controller;
  @override
  Widget build(BuildContext context) => ListView(
    children: [
      PageHeading(
        title: '已验证的文件',
        subtitle: '文件完成 SHA-256 校验后才会出现在私有收件箱。',
        trailing: IconButton(
          onPressed: controller.refreshInbox,
          icon: const Icon(Icons.refresh),
        ),
      ),
      const SizedBox(height: 28),
      if (controller.inbox.isEmpty)
        Card(
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 56, horizontal: 24),
            child: Column(
              children: [
                Icon(
                  Icons.inbox_outlined,
                  size: 52,
                  color: Theme.of(context).colorScheme.primary,
                ),
                const SizedBox(height: 16),
                Text('收件箱为空', style: Theme.of(context).textTheme.titleLarge),
                const SizedBox(height: 8),
                const Text('传输完成并校验通过的文件将显示在这里。'),
              ],
            ),
          ),
        )
      else
        ...controller.inbox.map(
          (item) => Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: Card(
              child: ListTile(
                leading: const Icon(Icons.verified_outlined),
                title: Text(item['name'] as String? ?? '文件'),
                subtitle: Text('${item['size']} 字节 · ${item['sha256']}'),
                trailing: Wrap(
                  children: [
                    IconButton(
                      onPressed: () => controller.exportItem(item),
                      icon: const Icon(Icons.save_alt),
                    ),
                    IconButton(
                      onPressed: () => controller.removeItem(item),
                      icon: const Icon(Icons.delete_outline),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
    ],
  );
}

class LogsPage extends StatelessWidget {
  const LogsPage({super.key, required this.controller});
  final HermodController controller;
  @override
  Widget build(BuildContext context) => ListView(
    children: [
      PageHeading(
        title: '连接日志',
        subtitle: '日志仅限量保存在内存中，技术内容保持原文。',
        trailing: IconButton(
          onPressed: controller.refreshLogs,
          icon: const Icon(Icons.refresh),
        ),
      ),
      const SizedBox(height: 24),
      Card(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: SelectionArea(
            child: Text(
              controller.logs.isEmpty
                  ? '暂无日志。请在设置中启用日志。'
                  : controller.logs.join('\n'),
              style: Theme.of(
                context,
              ).textTheme.bodyMedium?.copyWith(fontFamily: 'Consolas'),
            ),
          ),
        ),
      ),
    ],
  );
}
