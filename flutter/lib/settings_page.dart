import 'package:flutter/material.dart';

import 'app_controller.dart';

class EditableSettingsPage extends StatefulWidget {
  const EditableSettingsPage({super.key, required this.controller});
  final HermodController controller;

  @override
  State<EditableSettingsPage> createState() => _EditableSettingsPageState();
}

class _EditableSettingsPageState extends State<EditableSettingsPage> {
  final server = TextEditingController();
  bool loaded = false;
  String transport = 'quic';
  String receiveTransport = 'auto';
  String profile = 'auto';
  String theme = 'system';
  String language = 'system';
  String verbose = 'none';
  int words = 3;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (loaded || widget.controller.settings == null) return;
    final settings = widget.controller.settings!;
    final connection = settings['connection'] as Map<String, dynamic>;
    server.text = settings['server'] as String;
    transport = connection['transport'] as String? ?? 'quic';
    profile = connection['profile'] as String? ?? 'auto';
    receiveTransport = settings['receive_transport'] as String? ?? 'auto';
    theme = settings['theme'] as String? ?? 'system';
    language = settings['language'] as String? ?? 'system';
    verbose = settings['verbose'] as String? ?? 'none';
    words = settings['words'] as int? ?? 3;
    loaded = true;
  }

  @override
  void dispose() {
    server.dispose();
    super.dispose();
  }

  Future<void> save() async {
    final settings = Map<String, dynamic>.from(widget.controller.settings!);
    final connection = Map<String, dynamic>.from(
      settings['connection'] as Map<String, dynamic>,
    );
    connection['transport'] = transport;
    connection['profile'] = profile;
    settings
      ..['connection'] = connection
      ..['receive_transport'] = receiveTransport
      ..['server'] = server.text.trim()
      ..['theme'] = theme
      ..['language'] = language
      ..['verbose'] = verbose
      ..['words'] = words;
    await widget.controller.saveSettings(settings);
  }

  Future<void> verifyServer() async {
    final fingerprint = await widget.controller.probeServer(server.text.trim());
    if (!mounted || fingerprint == null) return;
    final trusted = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('验证服务器指纹'),
        content: SelectableText('请通过独立渠道核对 SHA-256 SPKI 指纹：\n\n$fingerprint'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('信任'),
          ),
        ],
      ),
    );
    if (trusted == true) {
      await widget.controller.trustServer(server.text.trim(), fingerprint);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (!loaded) return const Center(child: CircularProgressIndicator());
    return ListView(
      children: [
        Text('设置', style: Theme.of(context).textTheme.headlineMedium),
        const SizedBox(height: 6),
        const Text('所有值由 Rust Core 验证并持久化；正在进行的传输不会被修改。'),
        const SizedBox(height: 24),
        Card(
          child: Padding(
            padding: const EdgeInsets.all(20),
            child: Column(
              children: [
                DropdownButtonFormField(
                  initialValue: transport,
                  decoration: const InputDecoration(labelText: '发送协议'),
                  items: const [
                    DropdownMenuItem(value: 'quic', child: Text('QUIC')),
                    DropdownMenuItem(
                      value: 'wireguard',
                      child: Text('WireGuard'),
                    ),
                  ],
                  onChanged: (value) => setState(() => transport = value!),
                ),
                const SizedBox(height: 12),
                DropdownButtonFormField(
                  initialValue: receiveTransport,
                  decoration: const InputDecoration(labelText: '接收协议'),
                  items: const [
                    DropdownMenuItem(value: 'auto', child: Text('自动')),
                    DropdownMenuItem(value: 'quic', child: Text('QUIC')),
                    DropdownMenuItem(
                      value: 'wireguard',
                      child: Text('WireGuard'),
                    ),
                  ],
                  onChanged: (value) =>
                      setState(() => receiveTransport = value!),
                ),
                const SizedBox(height: 12),
                DropdownButtonFormField(
                  initialValue: profile,
                  decoration: const InputDecoration(
                    labelText: '链路档位',
                    helperText: 'WiFi 局域网选“局域网优先”可显著提升吞吐',
                  ),
                  items: const [
                    DropdownMenuItem(value: 'auto', child: Text('自动（按链路质量）')),
                    DropdownMenuItem(value: 'lan', child: Text('局域网优先（并行流）')),
                    DropdownMenuItem(value: 'compatible', child: Text('兼容优先（最稳）')),
                  ],
                  onChanged: (value) => setState(() => profile = value!),
                ),
                const SizedBox(height: 12),
                TextFormField(
                  controller: server,
                  decoration: InputDecoration(
                    labelText: '信令服务器',
                    suffixIcon: IconButton(
                      onPressed: verifyServer,
                      tooltip: '获取并验证指纹',
                      icon: const Icon(Icons.verified_user_outlined),
                    ),
                  ),
                ),
                const SizedBox(height: 12),
                DropdownButtonFormField(
                  initialValue: theme,
                  decoration: const InputDecoration(labelText: '外观'),
                  items: const [
                    DropdownMenuItem(value: 'system', child: Text('跟随系统')),
                    DropdownMenuItem(value: 'light', child: Text('亮色')),
                    DropdownMenuItem(value: 'dark', child: Text('暗色')),
                  ],
                  onChanged: (value) => setState(() => theme = value!),
                ),
                const SizedBox(height: 12),
                DropdownButtonFormField(
                  initialValue: language,
                  decoration: const InputDecoration(labelText: '语言'),
                  items: const [
                    DropdownMenuItem(value: 'system', child: Text('跟随系统')),
                    DropdownMenuItem(value: 'zh-CN', child: Text('简体中文')),
                    DropdownMenuItem(value: 'en-US', child: Text('English')),
                  ],
                  onChanged: (value) => setState(() => language = value!),
                ),
                const SizedBox(height: 12),
                DropdownButtonFormField(
                  initialValue: verbose,
                  decoration: const InputDecoration(labelText: '日志级别'),
                  items: const [
                    DropdownMenuItem(value: 'none', child: Text('关闭')),
                    DropdownMenuItem(value: 'error', child: Text('错误')),
                    DropdownMenuItem(value: 'warning', child: Text('警告')),
                    DropdownMenuItem(value: 'info', child: Text('信息')),
                    DropdownMenuItem(value: 'debug', child: Text('调试')),
                  ],
                  onChanged: (value) => setState(() => verbose = value!),
                ),
                const SizedBox(height: 12),
                Row(
                  children: [
                    const Expanded(child: Text('传输码单词数量')),
                    Text('$words'),
                    Expanded(
                      child: Slider(
                        value: words.toDouble(),
                        min: 1,
                        max: 12,
                        divisions: 11,
                        onChanged: (value) =>
                            setState(() => words = value.round()),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 16),
                Align(
                  alignment: Alignment.centerRight,
                  child: FilledButton.icon(
                    onPressed: save,
                    icon: const Icon(Icons.save_outlined),
                    label: const Text('保存设置'),
                  ),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}
