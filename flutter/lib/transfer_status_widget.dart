import 'package:material_ui/material_ui.dart';

import 'core_bridge.dart';
import 'l10n/app_localizations.dart';
import 'ui/formatters.dart';
import 'ui/motion.dart';

class TransferStatusCard extends StatelessWidget {
  const TransferStatusCard({
    super.key,
    required this.status,
    required this.onCancel,
    this.quiet = false,
  });
  final bool quiet;
  final TransferSnapshot status;
  final VoidCallback? onCancel;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final theme = Theme.of(context);
    final colors = theme.colorScheme;
    final reducedMotion =
        MediaQuery.disableAnimationsOf(context) ||
        MediaQuery.accessibleNavigationOf(context);
    final success =
        status.phase == TransferPhase.completed &&
        !status.cancelled &&
        status.error == null;
    final total = status.total;
    final progress = success
        ? 1.0
        : total == null || total <= BigInt.zero
        ? null
        : (status.bytes.toDouble() / total.toDouble()).clamp(0.0, .99);
    final phase = switch (status.phase) {
      TransferPhase.preparingSource => l.preparingSource,
      TransferPhase.connectingSignaling => l.connectingSignaling,
      TransferPhase.waitingForPeer => l.waitingForPeer,
      TransferPhase.sending => l.sending,
      TransferPhase.receiving => l.receiving,
      TransferPhase.completed => l.completed,
      TransferPhase.cancelled => l.cancelled,
      TransferPhase.failed => l.failed,
    };
    final icon = switch (status.phase) {
      TransferPhase.preparingSource => Icons.insert_drive_file_outlined,
      TransferPhase.connectingSignaling => Icons.hub_outlined,
      TransferPhase.waitingForPeer => Icons.devices_outlined,
      TransferPhase.sending => Icons.north_east,
      TransferPhase.receiving => Icons.south_west,
      TransferPhase.completed => Icons.check_circle_outline,
      TransferPhase.cancelled => Icons.cancel_outlined,
      TransferPhase.failed => Icons.error_outline,
    };
    final rate = status.elapsed > 0
        ? status.bytes.toDouble() / status.elapsed
        : 0.0;
    final numberStyle = theme.textTheme.bodyMedium?.copyWith(
      fontFeatures: const [FontFeature.tabularFigures()],
      color: colors.onSurfaceVariant,
    );
    return Card.filled(
      margin: EdgeInsets.zero,
      color: colors.surfaceContainerLow,
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          spacing: 16,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              spacing: 12,
              children: [
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Icon(
                    icon,
                    color: status.error != null ? colors.error : colors.primary,
                  ),
                ),
                Expanded(
                  child: MotionSwap(
                    identity: status.phase,
                    enabled: !quiet,
                    child: Semantics(
                      liveRegion: true,
                      child: Text(phase, style: theme.textTheme.titleMedium),
                    ),
                  ),
                ),
              ],
            ),
            if (status.code case final code?)
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                spacing: 8,
                children: [
                  Text(
                    l.transferCode,
                    style: theme.textTheme.labelMedium?.copyWith(
                      color: colors.onSurfaceVariant,
                    ),
                  ),
                  SelectableText(
                    code,
                    style: theme.textTheme.headlineSmall?.copyWith(
                      fontFeatures: const [FontFeature.tabularFigures()],
                    ),
                  ),
                ],
              ),
            if (!quiet) ...[
              // Bytes are facts, not spring positions: only a true success is
              // 100%, immediately. Effects motion is confined to the phase label.
              LinearProgressIndicator(
                value: progress ?? (reducedMotion || status.done ? 0 : null),
                semanticsLabel: phase,
              ),
              Wrap(
                spacing: 16,
                runSpacing: 8,
                children: [
                  Text(
                    total == null
                        ? formatBytes(status.bytes)
                        : '${formatBytes(status.bytes)} / ${formatBytes(total)}',
                    style: numberStyle,
                  ),
                  if (!status.done) Text(formatRate(rate), style: numberStyle),
                  if (status.transport case final transport?)
                    Text(transport, style: numberStyle),
                ],
              ),
            ],
            if (status.error case final error?)
              Text(
                error,
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: colors.error,
                ),
              ),
            if (status.digest case final digest?)
              SelectableText(
                'SHA-256: $digest',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: colors.onSurfaceVariant,
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
            if (!status.done)
              Align(
                alignment: AlignmentDirectional.centerStart,
                child: TextButton.icon(
                  key: const ValueKey('transfer-cancel'),
                  onPressed: onCancel,
                  icon: const Icon(Icons.close),
                  label: Text(l.cancel),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
