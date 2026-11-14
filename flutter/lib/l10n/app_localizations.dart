import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:intl/intl.dart' as intl;

import 'app_localizations_en.dart';
import 'app_localizations_zh.dart';

// ignore_for_file: type=lint

/// Callers can lookup localized strings with an instance of AppLocalizations
/// returned by `AppLocalizations.of(context)`.
///
/// Applications need to include `AppLocalizations.delegate()` in their app's
/// `localizationDelegates` list, and the locales they support in the app's
/// `supportedLocales` list. For example:
///
/// ```dart
/// import 'l10n/app_localizations.dart';
///
/// return MaterialApp(
///   localizationsDelegates: AppLocalizations.localizationsDelegates,
///   supportedLocales: AppLocalizations.supportedLocales,
///   home: MyApplicationHome(),
/// );
/// ```
///
/// ## Update pubspec.yaml
///
/// Please make sure to update your pubspec.yaml to include the following
/// packages:
///
/// ```yaml
/// dependencies:
///   # Internationalization support.
///   flutter_localizations:
///     sdk: flutter
///   intl: any # Use the pinned version from flutter_localizations
///
///   # Rest of dependencies
/// ```
///
/// ## iOS Applications
///
/// iOS applications define key application metadata, including supported
/// locales, in an Info.plist file that is built into the application bundle.
/// To configure the locales supported by your app, you’ll need to edit this
/// file.
///
/// First, open your project’s ios/Runner.xcworkspace Xcode workspace file.
/// Then, in the Project Navigator, open the Info.plist file under the Runner
/// project’s Runner folder.
///
/// Next, select the Information Property List item, select Add Item from the
/// Editor menu, then select Localizations from the pop-up menu.
///
/// Select and expand the newly-created Localizations item then, for each
/// locale your application supports, add a new item and select the locale
/// you wish to add from the pop-up menu in the Value field. This list should
/// be consistent with the languages listed in the AppLocalizations.supportedLocales
/// property.
abstract class AppLocalizations {
  AppLocalizations(String locale)
    : localeName = intl.Intl.canonicalizedLocale(locale.toString());

  final String localeName;

  static AppLocalizations of(BuildContext context) {
    return Localizations.of<AppLocalizations>(context, AppLocalizations)!;
  }

  static const LocalizationsDelegate<AppLocalizations> delegate =
      _AppLocalizationsDelegate();

  /// A list of this localizations delegate along with the default localizations
  /// delegates.
  ///
  /// Returns a list of localizations delegates containing this delegate along with
  /// GlobalMaterialLocalizations.delegate, GlobalCupertinoLocalizations.delegate,
  /// and GlobalWidgetsLocalizations.delegate.
  ///
  /// Additional delegates can be added by appending to this list in
  /// MaterialApp. This list does not have to be used at all if a custom list
  /// of delegates is preferred or required.
  static const List<LocalizationsDelegate<dynamic>> localizationsDelegates =
      <LocalizationsDelegate<dynamic>>[
        delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
      ];

  /// A list of this localizations delegate's supported locales.
  static const List<Locale> supportedLocales = <Locale>[
    Locale('en'),
    Locale('zh'),
  ];

  /// No description provided for @licenses.
  ///
  /// In en, this message translates to:
  /// **'Open-source licenses'**
  String get licenses;

  /// No description provided for @appTitle.
  ///
  /// In en, this message translates to:
  /// **'Hermod'**
  String get appTitle;

  /// No description provided for @pairedDevices.
  ///
  /// In en, this message translates to:
  /// **'Paired devices'**
  String get pairedDevices;

  /// No description provided for @receivePairedHint.
  ///
  /// In en, this message translates to:
  /// **'Connect to a paired device’s current sending task. Pairing does not indicate that a device is online.'**
  String get receivePairedHint;

  /// No description provided for @receiveCodeHint.
  ///
  /// In en, this message translates to:
  /// **'Have a transfer code? Enter it to connect to the sender.'**
  String get receiveCodeHint;

  /// No description provided for @paired.
  ///
  /// In en, this message translates to:
  /// **'Paired'**
  String get paired;

  /// No description provided for @pairingPending.
  ///
  /// In en, this message translates to:
  /// **'Pairing pending'**
  String get pairingPending;

  /// No description provided for @nextTransfer.
  ///
  /// In en, this message translates to:
  /// **'Next transfer'**
  String get nextTransfer;

  /// No description provided for @chooseFileHint.
  ///
  /// In en, this message translates to:
  /// **'Choose a file, then create a code for the receiver.'**
  String get chooseFileHint;

  /// No description provided for @inboxChecksum.
  ///
  /// In en, this message translates to:
  /// **'SHA-256 checksum'**
  String get inboxChecksum;

  /// No description provided for @transfer.
  ///
  /// In en, this message translates to:
  /// **'Transfer'**
  String get transfer;

  /// No description provided for @devices.
  ///
  /// In en, this message translates to:
  /// **'Devices'**
  String get devices;

  /// No description provided for @inbox.
  ///
  /// In en, this message translates to:
  /// **'Inbox'**
  String get inbox;

  /// No description provided for @settings.
  ///
  /// In en, this message translates to:
  /// **'Settings'**
  String get settings;

  /// No description provided for @send.
  ///
  /// In en, this message translates to:
  /// **'Send file'**
  String get send;

  /// No description provided for @receive.
  ///
  /// In en, this message translates to:
  /// **'Receive'**
  String get receive;

  /// No description provided for @chooseFile.
  ///
  /// In en, this message translates to:
  /// **'Choose file'**
  String get chooseFile;

  /// No description provided for @generateCode.
  ///
  /// In en, this message translates to:
  /// **'Generate transfer code'**
  String get generateCode;

  /// No description provided for @transferCode.
  ///
  /// In en, this message translates to:
  /// **'Transfer code'**
  String get transferCode;

  /// No description provided for @connectReceive.
  ///
  /// In en, this message translates to:
  /// **'Connect and receive'**
  String get connectReceive;

  /// No description provided for @transferHint.
  ///
  /// In en, this message translates to:
  /// **'Encrypted peer-to-peer file transfer. Received files remain private until you export them.'**
  String get transferHint;

  /// No description provided for @pairRequest.
  ///
  /// In en, this message translates to:
  /// **'Request device pairing'**
  String get pairRequest;

  /// No description provided for @pairHint.
  ///
  /// In en, this message translates to:
  /// **'Both peers must explicitly agree. If the peer does not support pairing, turn this off only for a one-time transfer.'**
  String get pairHint;

  /// No description provided for @cancel.
  ///
  /// In en, this message translates to:
  /// **'Cancel'**
  String get cancel;

  /// No description provided for @refresh.
  ///
  /// In en, this message translates to:
  /// **'Refresh'**
  String get refresh;

  /// No description provided for @delete.
  ///
  /// In en, this message translates to:
  /// **'Delete'**
  String get delete;

  /// No description provided for @rename.
  ///
  /// In en, this message translates to:
  /// **'Rename'**
  String get rename;

  /// No description provided for @exportFile.
  ///
  /// In en, this message translates to:
  /// **'Export'**
  String get exportFile;

  /// No description provided for @save.
  ///
  /// In en, this message translates to:
  /// **'Save settings'**
  String get save;

  /// No description provided for @saved.
  ///
  /// In en, this message translates to:
  /// **'Settings saved'**
  String get saved;

  /// No description provided for @confirmDelete.
  ///
  /// In en, this message translates to:
  /// **'Delete this item?'**
  String get confirmDelete;

  /// No description provided for @deleteHint.
  ///
  /// In en, this message translates to:
  /// **'This cannot be undone. Export any files you want to keep first.'**
  String get deleteHint;

  /// No description provided for @emptyInbox.
  ///
  /// In en, this message translates to:
  /// **'No verified files'**
  String get emptyInbox;

  /// No description provided for @privateInboxHint.
  ///
  /// In en, this message translates to:
  /// **'Only SHA-256 verified files appear here. Export files to keep them outside the app; uninstalling may remove private copies.'**
  String get privateInboxHint;

  /// No description provided for @ownDevice.
  ///
  /// In en, this message translates to:
  /// **'This device'**
  String get ownDevice;

  /// No description provided for @emptyDevices.
  ///
  /// In en, this message translates to:
  /// **'No trusted devices'**
  String get emptyDevices;

  /// No description provided for @deviceName.
  ///
  /// In en, this message translates to:
  /// **'Device name'**
  String get deviceName;

  /// No description provided for @receiveDevice.
  ///
  /// In en, this message translates to:
  /// **'Receive from device'**
  String get receiveDevice;

  /// No description provided for @deviceHint.
  ///
  /// In en, this message translates to:
  /// **'Trusted devices are added only after authenticated protocol pairing. This is not remote file browsing.'**
  String get deviceHint;

  /// No description provided for @server.
  ///
  /// In en, this message translates to:
  /// **'Signaling server'**
  String get server;

  /// No description provided for @verifyFingerprint.
  ///
  /// In en, this message translates to:
  /// **'Verify fingerprint'**
  String get verifyFingerprint;

  /// No description provided for @fingerprintHint.
  ///
  /// In en, this message translates to:
  /// **'Compare this SHA-256 SPKI fingerprint through an independent channel before trusting it.'**
  String get fingerprintHint;

  /// No description provided for @trust.
  ///
  /// In en, this message translates to:
  /// **'Trust'**
  String get trust;

  /// No description provided for @settingsHint.
  ///
  /// In en, this message translates to:
  /// **'Rust validates and persists these values. Each new task uses a frozen snapshot of saved settings.'**
  String get settingsHint;

  /// No description provided for @sendTransport.
  ///
  /// In en, this message translates to:
  /// **'Send transport'**
  String get sendTransport;

  /// No description provided for @receiveTransport.
  ///
  /// In en, this message translates to:
  /// **'Receive transport'**
  String get receiveTransport;

  /// No description provided for @profile.
  ///
  /// In en, this message translates to:
  /// **'Network profile'**
  String get profile;

  /// No description provided for @mtu.
  ///
  /// In en, this message translates to:
  /// **'MTU (auto or bytes)'**
  String get mtu;

  /// No description provided for @congestion.
  ///
  /// In en, this message translates to:
  /// **'Congestion control'**
  String get congestion;

  /// No description provided for @words.
  ///
  /// In en, this message translates to:
  /// **'Transfer-code words'**
  String get words;

  /// No description provided for @verbose.
  ///
  /// In en, this message translates to:
  /// **'Log level'**
  String get verbose;

  /// No description provided for @quiet.
  ///
  /// In en, this message translates to:
  /// **'Compact transfer status'**
  String get quiet;

  /// No description provided for @theme.
  ///
  /// In en, this message translates to:
  /// **'Theme'**
  String get theme;

  /// No description provided for @language.
  ///
  /// In en, this message translates to:
  /// **'Language'**
  String get language;

  /// No description provided for @system.
  ///
  /// In en, this message translates to:
  /// **'System'**
  String get system;

  /// No description provided for @light.
  ///
  /// In en, this message translates to:
  /// **'Light'**
  String get light;

  /// No description provided for @dark.
  ///
  /// In en, this message translates to:
  /// **'Dark'**
  String get dark;

  /// No description provided for @auto.
  ///
  /// In en, this message translates to:
  /// **'Automatic'**
  String get auto;

  /// No description provided for @lan.
  ///
  /// In en, this message translates to:
  /// **'LAN'**
  String get lan;

  /// No description provided for @wan.
  ///
  /// In en, this message translates to:
  /// **'WAN'**
  String get wan;

  /// No description provided for @advanced.
  ///
  /// In en, this message translates to:
  /// **'Advanced network tuning'**
  String get advanced;

  /// No description provided for @streams.
  ///
  /// In en, this message translates to:
  /// **'Streams (empty = automatic)'**
  String get streams;

  /// No description provided for @chunkBytes.
  ///
  /// In en, this message translates to:
  /// **'Chunk bytes'**
  String get chunkBytes;

  /// No description provided for @windowBytes.
  ///
  /// In en, this message translates to:
  /// **'Window bytes'**
  String get windowBytes;

  /// No description provided for @udpBufferBytes.
  ///
  /// In en, this message translates to:
  /// **'UDP buffer bytes'**
  String get udpBufferBytes;

  /// No description provided for @sliceBytes.
  ///
  /// In en, this message translates to:
  /// **'Slice bytes'**
  String get sliceBytes;

  /// No description provided for @prefetch.
  ///
  /// In en, this message translates to:
  /// **'Prefetch'**
  String get prefetch;

  /// No description provided for @keepaliveSecs.
  ///
  /// In en, this message translates to:
  /// **'Keepalive seconds'**
  String get keepaliveSecs;

  /// No description provided for @idleTimeoutSecs.
  ///
  /// In en, this message translates to:
  /// **'Idle timeout seconds'**
  String get idleTimeoutSecs;

  /// No description provided for @invalidNumber.
  ///
  /// In en, this message translates to:
  /// **'Enter a whole number within the supported range.'**
  String get invalidNumber;

  /// No description provided for @cancelFileOperation.
  ///
  /// In en, this message translates to:
  /// **'Cancel file operation'**
  String get cancelFileOperation;

  /// No description provided for @ipFamily.
  ///
  /// In en, this message translates to:
  /// **'IP address family'**
  String get ipFamily;

  /// No description provided for @ipv4.
  ///
  /// In en, this message translates to:
  /// **'IPv4 only'**
  String get ipv4;

  /// No description provided for @ipv6.
  ///
  /// In en, this message translates to:
  /// **'IPv6 only'**
  String get ipv6;

  /// No description provided for @diagnostics.
  ///
  /// In en, this message translates to:
  /// **'Diagnostics'**
  String get diagnostics;

  /// No description provided for @noLogs.
  ///
  /// In en, this message translates to:
  /// **'No logs available.'**
  String get noLogs;

  /// No description provided for @retry.
  ///
  /// In en, this message translates to:
  /// **'Retry'**
  String get retry;

  /// No description provided for @errorTitle.
  ///
  /// In en, this message translates to:
  /// **'Operation failed'**
  String get errorTitle;

  /// No description provided for @retryable.
  ///
  /// In en, this message translates to:
  /// **'You may retry this operation.'**
  String get retryable;

  /// No description provided for @preparingSource.
  ///
  /// In en, this message translates to:
  /// **'Preparing source'**
  String get preparingSource;

  /// No description provided for @connectingSignaling.
  ///
  /// In en, this message translates to:
  /// **'Connecting to signaling'**
  String get connectingSignaling;

  /// No description provided for @waitingForPeer.
  ///
  /// In en, this message translates to:
  /// **'Waiting for peer'**
  String get waitingForPeer;

  /// No description provided for @sending.
  ///
  /// In en, this message translates to:
  /// **'Sending'**
  String get sending;

  /// No description provided for @receiving.
  ///
  /// In en, this message translates to:
  /// **'Receiving'**
  String get receiving;

  /// No description provided for @completed.
  ///
  /// In en, this message translates to:
  /// **'Verified and complete'**
  String get completed;

  /// No description provided for @cancelled.
  ///
  /// In en, this message translates to:
  /// **'Cancelled'**
  String get cancelled;

  /// No description provided for @failed.
  ///
  /// In en, this message translates to:
  /// **'Failed'**
  String get failed;

  /// No description provided for @verifying.
  ///
  /// In en, this message translates to:
  /// **'Awaiting completion and verification'**
  String get verifying;

  /// No description provided for @closeTitle.
  ///
  /// In en, this message translates to:
  /// **'Stop the active transfer?'**
  String get closeTitle;

  /// No description provided for @closeHint.
  ///
  /// In en, this message translates to:
  /// **'Closing stops the current transfer. Verified inbox files remain stored.'**
  String get closeHint;

  /// No description provided for @stopClose.
  ///
  /// In en, this message translates to:
  /// **'Stop and close'**
  String get stopClose;

  /// No description provided for @bytesValue.
  ///
  /// In en, this message translates to:
  /// **'{value} bytes'**
  String bytesValue(String value);

  /// No description provided for @progressValue.
  ///
  /// In en, this message translates to:
  /// **'{bytes} / {total} bytes · {rate} bytes/s'**
  String progressValue(String bytes, String total, String rate);

  /// No description provided for @errorDetails.
  ///
  /// In en, this message translates to:
  /// **'{code}: {message}'**
  String errorDetails(String code, String message);
}

class _AppLocalizationsDelegate
    extends LocalizationsDelegate<AppLocalizations> {
  const _AppLocalizationsDelegate();

  @override
  Future<AppLocalizations> load(Locale locale) {
    return SynchronousFuture<AppLocalizations>(lookupAppLocalizations(locale));
  }

  @override
  bool isSupported(Locale locale) =>
      <String>['en', 'zh'].contains(locale.languageCode);

  @override
  bool shouldReload(_AppLocalizationsDelegate old) => false;
}

AppLocalizations lookupAppLocalizations(Locale locale) {
  // Lookup logic when only language code is specified.
  switch (locale.languageCode) {
    case 'en':
      return AppLocalizationsEn();
    case 'zh':
      return AppLocalizationsZh();
  }

  throw FlutterError(
    'AppLocalizations.delegate failed to load unsupported locale "$locale". This is likely '
    'an issue with the localizations generation tool. Please file an issue '
    'on GitHub with a reproducible sample app and the gen-l10n configuration '
    'that was used.',
  );
}
