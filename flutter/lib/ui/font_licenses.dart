import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

bool _registered = false;

/// OFL text is embedded offline and available through the app license registry.
void registerFontLicenses() {
  if (_registered) return;
  _registered = true;
  LicenseRegistry.addLicense(() async* {
    final license = await rootBundle.loadString(
      'assets/fonts/noto-sans-sc/OFL.txt',
    );
    yield LicenseEntryWithLineBreaks(const ['Noto Sans SC'], license);
  });
}
