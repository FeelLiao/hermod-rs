import 'package:material_ui/material_ui.dart';

import 'app_controller.dart';
import 'core_bridge.dart';
import 'l10n/app_localizations.dart';

class EditableSettingsPage extends StatefulWidget {
  const EditableSettingsPage({super.key, required this.controller});
  final HermodController controller;
  @override
  State<EditableSettingsPage> createState() => _EditableSettingsPageState();
}

class _EditableSettingsPageState extends State<EditableSettingsPage> {
  final _form = GlobalKey<FormState>();
  final _fields = <String, TextEditingController>{};
  ApiSettings? _initial;
  String _transport = 'quic', _receiveTransport = 'auto', _profile = 'auto';
  String _theme = 'system', _language = 'system', _verbose = 'none';
  String _ipFamily = 'auto';
  bool _quiet = false;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final value = widget.controller.settings;
    if (_initial != null || value == null) return;
    _initial = value;
    final t = value.tuning;
    final values = <String, String>{
      'server': value.server,
      'mtu': value.connection.mtu,
      'congestion': value.connection.congestion,
      'words': '${value.words}',
      'streams': t.streams?.toString() ?? '',
      'chunkBytes': '${t.chunkBytes}',
      'windowBytes': '${t.windowBytes}',
      'udpBufferBytes': '${t.udpBufferBytes}',
      'sliceBytes': '${t.sliceBytes}',
      'prefetch': '${t.prefetch}',
      'keepaliveSecs': '${t.keepaliveSecs}',
      'idleTimeoutSecs': '${t.idleTimeoutSecs}',
    };
    for (final entry in values.entries) {
      _fields[entry.key] = TextEditingController(text: entry.value);
    }
    _transport = value.connection.transport;
    _receiveTransport = value.receiveTransport;
    _ipFamily = value.ipFamily;
    _profile = value.connection.profile;
    _theme = value.theme;
    _language = value.language;
    _verbose = value.verbose;
    _quiet = value.quiet;
  }

  @override
  void dispose() {
    for (final field in _fields.values) {
      field.dispose();
    }
    super.dispose();
  }

  String _text(String key) => _fields[key]!.text;
  int _number(String key) => int.parse(_text(key));
  Future<void> _save() async {
    if (!_form.currentState!.validate()) return;
    final initial = _initial!;
    final value = ApiSettings(
      version: initial.version,
      connection: ApiConnection(
        transport: _transport,
        mtu: _text('mtu'),
        congestion: _text('congestion'),
        profile: _profile,
      ),
      receiveTransport: _receiveTransport,
      ipFamily: _ipFamily,
      server: _text('server'),
      words: _number('words'),
      verbose: _verbose,
      quiet: _quiet,
      theme: _theme,
      language: _language,
      // Receiving remains in the private inbox; never introduce an automatic export target.
      destination: initial.destination,
      tuning: ApiTuning(
        streams: _text('streams').isEmpty ? null : _number('streams'),
        chunkBytes: _number('chunkBytes'),
        windowBytes: BigInt.parse(_text('windowBytes')),
        udpBufferBytes: _number('udpBufferBytes'),
        sliceBytes: BigInt.parse(_text('sliceBytes')),
        prefetch: _number('prefetch'),
        keepaliveSecs: _number('keepaliveSecs'),
        idleTimeoutSecs: _number('idleTimeoutSecs'),
      ),
    );
    final saved = await widget.controller.saveSettings(value);
    if (!mounted || !saved) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(AppLocalizations.of(context).saved)));
  }

  Future<void> _verify() async {
    final server = _text('server');
    final fingerprint = await widget.controller.probeServer(server);
    if (!mounted || fingerprint == null) return;
    final l = AppLocalizations.of(context);
    final trusted = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l.verifyFingerprint),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(l.fingerprintHint),
              const SizedBox(height: 16),
              SelectableText(fingerprint),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(l.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(l.trust),
          ),
        ],
      ),
    );
    if (trusted == true) {
      await widget.controller.trustServer(server, fingerprint);
    }
  }

  Widget _field(
    String key,
    String label, {
    bool numeric = false,
    bool optional = false,
    bool wide = false,
  }) => TextFormField(
    key: ValueKey('settings-$key'),
    controller: _fields[key],
    enabled: !widget.controller.operating,
    decoration: InputDecoration(labelText: label),
    keyboardType: numeric ? TextInputType.number : TextInputType.text,
    validator: numeric
        ? (value) {
            if (optional && (value == null || value.isEmpty)) return null;
            final parsed = BigInt.tryParse(value ?? '');
            final max = wide
                ? (BigInt.one << 64) - BigInt.one
                : BigInt.from(0xffffffff);
            return parsed == null || parsed < BigInt.zero || parsed > max
                ? AppLocalizations.of(context).invalidNumber
                : null;
          }
        : null,
  );
  Widget _dropdown(
    String label,
    String value,
    Map<String, String> choices,
    ValueChanged<String> change, {
    String? fieldKey,
  }) => DropdownMenu<String>(
    key: ValueKey('settings-${fieldKey ?? label}'),
    initialSelection: choices.containsKey(value) ? value : null,
    label: Text(label),
    expandedInsets: EdgeInsets.zero,
    selectOnly: true,
    enabled: !widget.controller.operating,
    dropdownMenuEntries: choices.entries
        .map((e) => DropdownMenuEntry(value: e.key, label: e.value))
        .toList(),
    onSelected: (value) {
      if (value != null) setState(() => change(value));
    },
  );

  Widget _grid(List<Widget> children) => LayoutBuilder(
    builder: (context, constraints) {
      final textScale = MediaQuery.textScalerOf(context).scale(16) / 16;
      final columns = constraints.maxWidth >= 640 * textScale ? 2 : 1;
      final width = (constraints.maxWidth - (columns - 1) * 20) / columns;
      return Wrap(
        spacing: 20,
        runSpacing: 20,
        children: [
          for (final child in children) SizedBox(width: width, child: child),
        ],
      );
    },
  );

  Widget _heading(String title, IconData icon) => Padding(
    padding: const EdgeInsets.only(top: 24, bottom: 20),
    child: Row(
      spacing: 12,
      children: [
        Icon(icon, color: Theme.of(context).colorScheme.onSurfaceVariant),
        Expanded(
          child: Text(title, style: Theme.of(context).textTheme.titleMedium),
        ),
      ],
    ),
  );
  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    if (_initial == null) return Text(l.errorTitle);
    return Form(
      key: _form,
      child: ListView(
        children: [
          Text(
            l.settingsHint,
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
          _heading(l.server, Icons.dns_outlined),
          _field('server', l.server),
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton.icon(
              onPressed: widget.controller.operating ? null : _verify,
              icon: const Icon(Icons.verified_user_outlined),
              label: Text(l.verifyFingerprint),
            ),
          ),
          _heading(l.theme, Icons.palette_outlined),
          _grid([
            _dropdown(
              l.theme,
              _theme,
              {'system': l.system, 'light': l.light, 'dark': l.dark},
              (v) => _theme = v,
              fieldKey: 'theme',
            ),
            _dropdown(
              l.language,
              _language,
              {'system': l.system, 'zh-CN': '简体中文', 'en-US': 'English'},
              (v) => _language = v,
              fieldKey: 'language',
            ),
          ]),
          const SizedBox(height: 24),
          ExpansionTile(
            key: const ValueKey('settings-advanced'),
            maintainState: true,
            tilePadding: EdgeInsets.zero,
            leading: const Icon(Icons.tune),
            title: Text(l.advanced),
            childrenPadding: const EdgeInsets.symmetric(vertical: 20),
            children: [
              _grid([
                _dropdown(l.sendTransport, _transport, const {
                  'quic': 'QUIC',
                }, (v) => _transport = v),
                _dropdown(l.receiveTransport, _receiveTransport, {
                  'auto': l.auto,
                  'quic': 'QUIC',
                }, (v) => _receiveTransport = v),
                _dropdown(
                  l.ipFamily,
                  _ipFamily,
                  {'auto': l.auto, 'ipv4': l.ipv4, 'ipv6': l.ipv6},
                  (v) => _ipFamily = v,
                  fieldKey: 'ip-family',
                ),
                _dropdown(l.profile, _profile, {
                  'auto': l.auto,
                  'lan': l.lan,
                  'wan': l.wan,
                }, (v) => _profile = v),
                _field('mtu', l.mtu),
                _field('congestion', l.congestion),
                _field('words', l.words, numeric: true),
                _field('streams', l.streams, numeric: true, optional: true),
                _field('chunkBytes', l.chunkBytes, numeric: true),
                _field('windowBytes', l.windowBytes, numeric: true, wide: true),
                _field('udpBufferBytes', l.udpBufferBytes, numeric: true),
                _field('sliceBytes', l.sliceBytes, numeric: true, wide: true),
                _field('prefetch', l.prefetch, numeric: true),
                _field('keepaliveSecs', l.keepaliveSecs, numeric: true),
                _field('idleTimeoutSecs', l.idleTimeoutSecs, numeric: true),
                // Raw string values deliberately pass through to Rust validation.
                _dropdown(l.verbose, _verbose, const {
                  'none': 'none',
                  'error': 'error',
                  'warning': 'warning',
                  'info': 'info',
                  'debug': 'debug',
                }, (v) => _verbose = v),
              ]),
              const SizedBox(height: 16),
              SwitchListTile.adaptive(
                key: const ValueKey('settings-quiet'),
                contentPadding: EdgeInsets.zero,
                title: Text(l.quiet),
                value: _quiet,
                onChanged: widget.controller.operating
                    ? null
                    : (v) => setState(() => _quiet = v),
              ),
            ],
          ),
          const SizedBox(height: 16),
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: ElevatedButton.icon(
              key: const ValueKey('settings-save'),
              onPressed: widget.controller.operating ? null : _save,
              icon: const Icon(Icons.save_outlined),
              label: Text(l.save),
            ),
          ),
          const SizedBox(height: 16),
          ExpansionTile(
            title: Text(l.diagnostics),
            onExpansionChanged: (open) {
              if (open) widget.controller.refreshLogs();
            },
            children: [
              Align(
                alignment: Alignment.centerRight,
                child: IconButton(
                  tooltip: l.refresh,
                  onPressed: widget.controller.refreshLogs,
                  icon: const Icon(Icons.refresh),
                ),
              ),
              SelectableText(
                widget.controller.logs.isEmpty
                    ? l.noLogs
                    : widget.controller.logs.join('\n'),
              ),
              Align(
                alignment: AlignmentDirectional.centerStart,
                child: TextButton.icon(
                  key: const ValueKey('settings-licenses'),
                  onPressed: () => showLicensePage(context: context),
                  icon: const Icon(Icons.description_outlined),
                  label: Text(l.licenses),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
