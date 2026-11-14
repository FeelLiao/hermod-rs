import 'package:material_ui/material_ui.dart';

import '../../app_controller.dart';
import '../../l10n/app_localizations.dart';
import '../../ui/formatters.dart';

Future<bool> confirmDelete(BuildContext context) async {
  final l = AppLocalizations.of(context);
  return await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text(l.confirmDelete),
          content: Text(l.deleteHint),
          actions: [
            TextButton(
              key: const ValueKey('cancel-delete'),
              onPressed: () => Navigator.pop(context, false),
              child: Text(l.cancel),
            ),
            FilledButton(
              key: const ValueKey('confirm-delete'),
              onPressed: () => Navigator.pop(context, true),
              child: Text(l.delete),
            ),
          ],
        ),
      ) ??
      false;
}

class InboxPage extends StatelessWidget {
  const InboxPage({super.key, required this.controller});
  final HermodController controller;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final theme = Theme.of(context);
    return ListView.separated(
      key: const PageStorageKey('inbox-scroll'),
      itemCount: controller.inbox.isEmpty ? 2 : controller.inbox.length + 1,
      separatorBuilder: (_, _) => const SizedBox(height: 12),
      itemBuilder: (context, index) {
        if (index == 0) {
          return _PageHeader(
            title: l.inbox,
            hint: l.privateInboxHint,
            refreshKey: 'refresh-inbox',
            onRefresh: controller.refreshInbox,
          );
        }
        if (controller.inbox.isEmpty) {
          return _EmptyState(icon: Icons.inbox_outlined, message: l.emptyInbox);
        }
        final item = controller.inbox[index - 1];
        return Card.filled(
          key: ValueKey('inbox-item-${item.id}'),
          child: Padding(
            padding: const EdgeInsets.all(20),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: 8,
              children: [
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  spacing: 16,
                  children: [
                    Icon(
                      Icons.insert_drive_file_outlined,
                      color: theme.colorScheme.primary,
                    ),
                    Expanded(
                      child: Text(
                        item.name,
                        style: theme.textTheme.titleMedium,
                      ),
                    ),
                  ],
                ),
                Text(formatBytes(item.size), style: theme.textTheme.bodyMedium),
                ExpansionTile(
                  key: PageStorageKey('inbox-details-${item.id}'),
                  tilePadding: EdgeInsets.zero,
                  title: Text(
                    l.inboxChecksum,
                    style: theme.textTheme.labelLarge,
                  ),
                  children: [
                    Align(
                      alignment: Alignment.centerLeft,
                      child: SelectableText(item.sha256),
                    ),
                  ],
                ),
                Wrap(
                  spacing: 12,
                  runSpacing: 8,
                  children: [
                    OutlinedButton.icon(
                      key: ValueKey('export-inbox-${item.id}'),
                      onPressed: controller.operating
                          ? null
                          : () => controller.exportItem(item),
                      icon: const Icon(Icons.save_alt),
                      label: Text(l.exportFile),
                    ),
                    TextButton.icon(
                      key: ValueKey('delete-inbox-${item.id}'),
                      onPressed: controller.operating
                          ? null
                          : () async {
                              if (await confirmDelete(context)) {
                                await controller.removeItem(item);
                              }
                            },
                      icon: const Icon(Icons.delete_outline),
                      label: Text(l.delete),
                    ),
                  ],
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class DevicesPage extends StatelessWidget {
  const DevicesPage({super.key, required this.controller});
  final HermodController controller;

  Future<void> _rename(BuildContext context, String id, String name) async {
    final result = await showDialog<String>(
      context: context,
      builder: (_) => _RenameDialog(name: name),
    );
    if (result != null) await controller.renameDevice(id, result);
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final theme = Theme.of(context);
    return ListView.separated(
      key: const PageStorageKey('devices-scroll'),
      itemCount: controller.devices.isEmpty ? 3 : controller.devices.length + 2,
      separatorBuilder: (_, _) => const SizedBox(height: 12),
      itemBuilder: (context, index) {
        if (index == 0) {
          return _PageHeader(
            title: l.devices,
            hint: l.deviceHint,
            refreshKey: 'refresh-devices',
            onRefresh: controller.refreshDevices,
          );
        }
        if (index == 1) {
          return Card.filled(
            color: theme.colorScheme.secondaryContainer,
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                spacing: 12,
                children: [
                  Icon(
                    Icons.devices_outlined,
                    color: theme.colorScheme.onSecondaryContainer,
                  ),
                  Text(
                    l.ownDevice,
                    style: theme.textTheme.titleLarge?.copyWith(
                      color: theme.colorScheme.onSecondaryContainer,
                    ),
                  ),
                  SelectableText(
                    controller.bootstrap?.deviceId ?? '',
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.onSecondaryContainer,
                    ),
                  ),
                ],
              ),
            ),
          );
        }
        if (controller.devices.isEmpty) {
          return _EmptyState(
            icon: Icons.devices_outlined,
            message: l.emptyDevices,
          );
        }
        final device = controller.devices[index - 2];
        return Card.filled(
          key: ValueKey('device-item-${device.deviceId}'),
          child: Padding(
            padding: const EdgeInsets.all(20),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: 12,
              children: [
                Text(device.name, style: theme.textTheme.titleLarge),
                Row(
                  spacing: 8,
                  children: [
                    Icon(
                      device.active
                          ? Icons.verified_user_outlined
                          : Icons.pending_outlined,
                      size: 20,
                      color: theme.colorScheme.primary,
                    ),
                    Expanded(
                      child: Text(
                        device.active ? l.paired : l.pairingPending,
                        style: theme.textTheme.labelLarge,
                      ),
                    ),
                  ],
                ),
                SelectableText(
                  device.deviceId,
                  style: theme.textTheme.bodySmall,
                ),
                Wrap(
                  spacing: 12,
                  runSpacing: 8,
                  children: [
                    OutlinedButton.icon(
                      key: ValueKey('receive-device-${device.deviceId}'),
                      onPressed: controller.canStart && device.active
                          ? () => controller.receiveFromDevice(device.deviceId)
                          : null,
                      icon: const Icon(Icons.download_outlined),
                      label: Text(l.receiveDevice),
                    ),
                    TextButton.icon(
                      key: ValueKey('rename-device-${device.deviceId}'),
                      onPressed: controller.operating
                          ? null
                          : () =>
                                _rename(context, device.deviceId, device.name),
                      icon: const Icon(Icons.edit_outlined),
                      label: Text(l.rename),
                    ),
                    TextButton.icon(
                      key: ValueKey('delete-device-${device.deviceId}'),
                      onPressed: controller.operating
                          ? null
                          : () async {
                              if (await confirmDelete(context)) {
                                await controller.removeDevice(device.deviceId);
                              }
                            },
                      icon: const Icon(Icons.delete_outline),
                      label: Text(l.delete),
                    ),
                  ],
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.name});
  final String name;
  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final input = TextEditingController(text: widget.name);
  @override
  void dispose() {
    input.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return AlertDialog(
      title: Text(l.rename),
      content: TextField(
        key: const ValueKey('device-name-input'),
        controller: input,
        autofocus: true,
        decoration: InputDecoration(labelText: l.deviceName),
      ),
      actions: [
        TextButton(
          key: const ValueKey('cancel-rename'),
          onPressed: () => Navigator.pop(context),
          child: Text(l.cancel),
        ),
        FilledButton(
          key: const ValueKey('confirm-rename'),
          onPressed: () => Navigator.pop(context, input.text),
          child: Text(l.rename),
        ),
      ],
    );
  }
}

class _PageHeader extends StatelessWidget {
  const _PageHeader({
    required this.title,
    required this.hint,
    required this.refreshKey,
    required this.onRefresh,
  });
  final String title;
  final String hint;
  final String refreshKey;
  final VoidCallback onRefresh;
  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 12),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: 8,
      children: [
        Row(
          spacing: 12,
          children: [
            Expanded(
              child: Text(hint, style: Theme.of(context).textTheme.bodyLarge),
            ),
            IconButton(
              key: ValueKey(refreshKey),
              tooltip: AppLocalizations.of(context).refresh,
              onPressed: onRefresh,
              icon: const Icon(Icons.refresh),
            ),
          ],
        ),
      ],
    ),
  );
}

class _EmptyState extends StatelessWidget {
  const _EmptyState({required this.icon, required this.message});
  final IconData icon;
  final String message;
  @override
  Widget build(BuildContext context) => Card.outlined(
    child: Padding(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 48),
      child: Column(
        spacing: 16,
        children: [
          Icon(icon, size: 48, color: Theme.of(context).colorScheme.primary),
          Text(
            message,
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.titleLarge,
          ),
        ],
      ),
    ),
  );
}
