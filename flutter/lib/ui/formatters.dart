/// Binary units for file sizes; no locale-dependent parsing of persisted values.
String formatBytes(BigInt bytes) {
  if (bytes < BigInt.zero) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  var amount = bytes.toDouble();
  var unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit++;
  }
  final precision = unit == 0 || amount >= 100 ? 0 : 1;
  return '${amount.toStringAsFixed(precision)} ${units[unit]}';
}

String formatRate(double bytesPerSecond) =>
    '${formatBytes(BigInt.from(bytesPerSecond.isFinite && bytesPerSecond > 0 ? bytesPerSecond.round() : 0))}/s';
