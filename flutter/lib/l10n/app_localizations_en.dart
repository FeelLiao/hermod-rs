// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for English (`en`).
class AppLocalizationsEn extends AppLocalizations {
  AppLocalizationsEn([String locale = 'en']) : super(locale);

  @override
  String get licenses => 'Open-source licenses';

  @override
  String get appTitle => 'Hermod';

  @override
  String get pairedDevices => 'Paired devices';

  @override
  String get receivePairedHint =>
      'Connect to a paired device’s current sending task. Pairing does not indicate that a device is online.';

  @override
  String get receiveCodeHint =>
      'Have a transfer code? Enter it to connect to the sender.';

  @override
  String get paired => 'Paired';

  @override
  String get pairingPending => 'Pairing pending';

  @override
  String get nextTransfer => 'Next transfer';

  @override
  String get chooseFileHint =>
      'Choose a file, then create a code for the receiver.';

  @override
  String get inboxChecksum => 'SHA-256 checksum';

  @override
  String get transfer => 'Transfer';

  @override
  String get devices => 'Devices';

  @override
  String get inbox => 'Inbox';

  @override
  String get settings => 'Settings';

  @override
  String get send => 'Send file';

  @override
  String get receive => 'Receive';

  @override
  String get chooseFile => 'Choose file';

  @override
  String get generateCode => 'Generate transfer code';

  @override
  String get transferCode => 'Transfer code';

  @override
  String get connectReceive => 'Connect and receive';

  @override
  String get transferHint =>
      'Encrypted peer-to-peer file transfer. Received files remain private until you export them.';

  @override
  String get pairRequest => 'Request device pairing';

  @override
  String get pairHint =>
      'Both peers must explicitly agree. If the peer does not support pairing, turn this off only for a one-time transfer.';

  @override
  String get cancel => 'Cancel';

  @override
  String get refresh => 'Refresh';

  @override
  String get delete => 'Delete';

  @override
  String get rename => 'Rename';

  @override
  String get exportFile => 'Export';

  @override
  String get save => 'Save settings';

  @override
  String get saved => 'Settings saved';

  @override
  String get confirmDelete => 'Delete this item?';

  @override
  String get deleteHint =>
      'This cannot be undone. Export any files you want to keep first.';

  @override
  String get emptyInbox => 'No verified files';

  @override
  String get privateInboxHint =>
      'Only SHA-256 verified files appear here. Export files to keep them outside the app; uninstalling may remove private copies.';

  @override
  String get ownDevice => 'This device';

  @override
  String get emptyDevices => 'No trusted devices';

  @override
  String get deviceName => 'Device name';

  @override
  String get receiveDevice => 'Receive from device';

  @override
  String get deviceHint =>
      'Trusted devices are added only after authenticated protocol pairing. This is not remote file browsing.';

  @override
  String get server => 'Signaling server';

  @override
  String get verifyFingerprint => 'Verify fingerprint';

  @override
  String get fingerprintHint =>
      'Compare this SHA-256 SPKI fingerprint through an independent channel before trusting it.';

  @override
  String get trust => 'Trust';

  @override
  String get settingsHint =>
      'Rust validates and persists these values. Each new task uses a frozen snapshot of saved settings.';

  @override
  String get sendTransport => 'Send transport';

  @override
  String get receiveTransport => 'Receive transport';

  @override
  String get profile => 'Network profile';

  @override
  String get mtu => 'MTU (auto or bytes)';

  @override
  String get congestion => 'Congestion control';

  @override
  String get words => 'Transfer-code words';

  @override
  String get verbose => 'Log level';

  @override
  String get quiet => 'Compact transfer status';

  @override
  String get theme => 'Theme';

  @override
  String get language => 'Language';

  @override
  String get system => 'System';

  @override
  String get light => 'Light';

  @override
  String get dark => 'Dark';

  @override
  String get auto => 'Automatic';

  @override
  String get lan => 'LAN';

  @override
  String get wan => 'WAN';

  @override
  String get advanced => 'Advanced network tuning';

  @override
  String get streams => 'Streams (empty = automatic)';

  @override
  String get chunkBytes => 'Chunk bytes';

  @override
  String get windowBytes => 'Window bytes';

  @override
  String get udpBufferBytes => 'UDP buffer bytes';

  @override
  String get sliceBytes => 'Slice bytes';

  @override
  String get prefetch => 'Prefetch';

  @override
  String get keepaliveSecs => 'Keepalive seconds';

  @override
  String get idleTimeoutSecs => 'Idle timeout seconds';

  @override
  String get invalidNumber =>
      'Enter a whole number within the supported range.';

  @override
  String get cancelFileOperation => 'Cancel file operation';

  @override
  String get ipFamily => 'IP address family';

  @override
  String get ipv4 => 'IPv4 only';

  @override
  String get ipv6 => 'IPv6 only';

  @override
  String get diagnostics => 'Diagnostics';

  @override
  String get noLogs => 'No logs available.';

  @override
  String get retry => 'Retry';

  @override
  String get errorTitle => 'Operation failed';

  @override
  String get retryable => 'You may retry this operation.';

  @override
  String get preparingSource => 'Preparing source';

  @override
  String get connectingSignaling => 'Connecting to signaling';

  @override
  String get waitingForPeer => 'Waiting for peer';

  @override
  String get sending => 'Sending';

  @override
  String get receiving => 'Receiving';

  @override
  String get completed => 'Verified and complete';

  @override
  String get cancelled => 'Cancelled';

  @override
  String get failed => 'Failed';

  @override
  String get verifying => 'Awaiting completion and verification';

  @override
  String get closeTitle => 'Stop the active transfer?';

  @override
  String get closeHint =>
      'Closing stops the current transfer. Verified inbox files remain stored.';

  @override
  String get stopClose => 'Stop and close';

  @override
  String bytesValue(String value) {
    return '$value bytes';
  }

  @override
  String progressValue(String bytes, String total, String rate) {
    return '$bytes / $total bytes · $rate bytes/s';
  }

  @override
  String errorDetails(String code, String message) {
    return '$code: $message';
  }
}
