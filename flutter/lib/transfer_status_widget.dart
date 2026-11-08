import 'package:flutter/material.dart';

class TransferStatusCard extends StatelessWidget {
  const TransferStatusCard({
    super.key,
    required this.status,
    required this.onCancel,
  });
  final Map<String, dynamic> status;
  final VoidCallback onCancel;

  @override
  Widget build(BuildContext context) {
    final bytes = (status['bytes'] as num?)?.toDouble() ?? 0;
    final total = (status['total'] as num?)?.toDouble();
    final progress = total == null || total <= 0
        ? null
        : (bytes / total).clamp(0.0, 1.0);
    final done = status['done'] == true;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    status['phase_message'] as String? ?? '正在连接',
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                ),
                if (!done)
                  TextButton.icon(
                    onPressed: onCancel,
                    icon: const Icon(Icons.close),
                    label: const Text('取消'),
                  ),
              ],
            ),
            if (status['code'] case final String code) ...[
              const SizedBox(height: 12),
              SelectableText(
                code,
                style: Theme.of(context).textTheme.headlineSmall,
              ),
            ],
            const SizedBox(height: 14),
            LinearProgressIndicator(value: done ? 1 : progress),
            const SizedBox(height: 8),
            Text(
              total == null
                  ? '${bytes.toInt()} 字节'
                  : '${bytes.toInt()} / ${total.toInt()} 字节',
            ),
          ],
        ),
      ),
    );
  }
}
