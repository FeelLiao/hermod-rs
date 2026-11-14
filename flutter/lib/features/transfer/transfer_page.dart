import 'package:material_ui/material_ui.dart';

import '../../app_controller.dart';
import '../../l10n/app_localizations.dart';
import '../../transfer_status_widget.dart';
import '../../ui/formatters.dart';
import '../../ui/motion.dart';

class TransferPage extends StatefulWidget {
  const TransferPage({super.key, required this.controller});
  final HermodController controller;

  @override
  State<TransferPage> createState() => _TransferPageState();
}

class _TransferPageState extends State<TransferPage> {
  final code = TextEditingController();
  bool receiving = false;

  @override
  void dispose() {
    code.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.controller;
    final l = AppLocalizations.of(context);
    final text = Theme.of(context).textTheme;
    return ListView(
      key: const PageStorageKey('transfer-scroll'),
      children: [
        Text(l.transferHint, style: text.bodyLarge),
        const SizedBox(height: 24),
        if (c.status case final status?) ...[
          TransferStatusCard(
            status: status,
            quiet: c.taskSettings?.quiet ?? false,
            onCancel: c.operating ? null : c.cancel,
          ),
          const SizedBox(height: 24),
        ],
        // A running task owns this surface. Keep the input controller and the
        // selected direction mounted in State, not in an outgoing animation.
        if (!c.active)
          MotionSwap(
            identity: receiving,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: 20,
              children: [
                if (c.status?.done == true)
                  Text(l.nextTransfer, style: text.titleLarge),
                SegmentedButton<bool>(
                  key: const ValueKey('transfer-direction'),
                  segments: [
                    ButtonSegment(
                      value: false,
                      icon: const Icon(Icons.upload_file_outlined),
                      label: Text(l.send),
                    ),
                    ButtonSegment(
                      value: true,
                      icon: const Icon(Icons.download_outlined),
                      label: Text(l.receive),
                    ),
                  ],
                  selected: {receiving},
                  onSelectionChanged: c.canStart
                      ? (value) => setState(() => receiving = value.first)
                      : null,
                ),
                if (receiving) ..._receive(context) else _send(context),
                SwitchListTile.adaptive(
                  key: const ValueKey('request-pairing'),
                  contentPadding: const EdgeInsets.symmetric(horizontal: 8),
                  title: Text(l.pairRequest),
                  subtitle: Text(l.pairHint),
                  value: c.requestPairing,
                  onChanged: c.canStart ? c.setPairing : null,
                ),
              ],
            ),
          ),
      ],
    );
  }

  Widget _send(BuildContext context) {
    final c = widget.controller;
    final l = AppLocalizations.of(context);
    final theme = Theme.of(context);
    final selection = c.selection;
    return Card.filled(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          spacing: 20,
          children: [
            Align(
              alignment: Alignment.centerLeft,
              child: Icon(
                Icons.upload_file_outlined,
                size: 40,
                color: theme.colorScheme.primary,
              ),
            ),
            Text(
              selection?.name ?? l.send,
              style: theme.textTheme.headlineSmall,
            ),
            Text(
              selection?.size == null
                  ? l.chooseFileHint
                  : formatBytes(selection!.size!),
              style: theme.textTheme.bodyLarge,
            ),
            Wrap(
              spacing: 12,
              runSpacing: 12,
              children: [
                if (selection == null)
                  ElevatedButton.icon(
                    key: const ValueKey('choose-file'),
                    onPressed: c.canStart ? c.chooseFile : null,
                    icon: const Icon(Icons.add),
                    label: Text(l.chooseFile),
                  )
                else ...[
                  ElevatedButton.icon(
                    key: const ValueKey('generate-code'),
                    onPressed: c.canStart ? c.sendFile : null,
                    icon: const Icon(Icons.key_outlined),
                    label: Text(l.generateCode),
                  ),
                  OutlinedButton.icon(
                    key: const ValueKey('choose-file'),
                    onPressed: c.canStart ? c.chooseFile : null,
                    icon: const Icon(Icons.swap_horiz),
                    label: Text(l.chooseFile),
                  ),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }

  List<Widget> _receive(BuildContext context) {
    final c = widget.controller;
    final l = AppLocalizations.of(context);
    final theme = Theme.of(context);
    final paired = c.devices.where((device) => device.active).toList();
    return [
      Card.filled(
        key: const ValueKey('paired-receive-panel'),
        color: theme.colorScheme.primaryContainer,
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            spacing: 16,
            children: [
              Text(
                l.pairedDevices,
                style: theme.textTheme.titleLarge?.copyWith(
                  color: theme.colorScheme.onPrimaryContainer,
                ),
              ),
              Text(
                l.receivePairedHint,
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: theme.colorScheme.onPrimaryContainer,
                ),
              ),
              if (paired.isEmpty)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 8),
                  child: Text(
                    l.emptyDevices,
                    style: theme.textTheme.titleMedium,
                  ),
                ),
              for (final device in paired)
                Card.filled(
                  color: theme.colorScheme.surface,
                  child: Padding(
                    padding: const EdgeInsets.all(16),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      spacing: 8,
                      children: [
                        Text(device.name, style: theme.textTheme.titleMedium),
                        SelectableText(
                          device.deviceId,
                          style: theme.textTheme.bodySmall,
                        ),
                        Align(
                          alignment: Alignment.centerLeft,
                          child: ElevatedButton.icon(
                            key: ValueKey('receive-paired-${device.deviceId}'),
                            onPressed: c.canStart
                                ? () => c.receiveFromDevice(device.deviceId)
                                : null,
                            icon: const Icon(Icons.download_outlined),
                            label: Text(l.receiveDevice),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
      Card.outlined(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            spacing: 16,
            children: [
              Text(l.transferCode, style: theme.textTheme.titleLarge),
              Text(l.receiveCodeHint, style: theme.textTheme.bodyMedium),
              TextField(
                key: const ValueKey('transfer-code'),
                controller: code,
                enabled: c.canStart,
                autocorrect: false,
                textInputAction: TextInputAction.go,
                decoration: InputDecoration(
                  labelText: l.transferCode,
                  prefixIcon: const Icon(Icons.key_outlined),
                ),
                onChanged: (_) => setState(() {}),
                onSubmitted: c.canStart ? (_) => _receiveCode() : null,
              ),
              Align(
                alignment: Alignment.centerLeft,
                child: ElevatedButton.icon(
                  key: const ValueKey('connect-receive'),
                  onPressed: c.canStart && code.text.trim().isNotEmpty
                      ? _receiveCode
                      : null,
                  icon: const Icon(Icons.download_outlined),
                  label: Text(l.connectReceive),
                ),
              ),
            ],
          ),
        ),
      ),
    ];
  }

  void _receiveCode() {
    if (widget.controller.canStart && code.text.trim().isNotEmpty) {
      widget.controller.receive(code.text.trim());
    }
  }
}
