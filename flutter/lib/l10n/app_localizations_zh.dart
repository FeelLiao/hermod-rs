// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Chinese (`zh`).
class AppLocalizationsZh extends AppLocalizations {
  AppLocalizationsZh([String locale = 'zh']) : super(locale);

  @override
  String get licenses => '开源许可';

  @override
  String get appTitle => 'Hermod';

  @override
  String get pairedDevices => '已配对设备';

  @override
  String get receivePairedHint => '连接已配对设备当前的发送任务。已配对不代表设备在线。';

  @override
  String get receiveCodeHint => '有传输码？输入后连接发送方。';

  @override
  String get paired => '已配对';

  @override
  String get pairingPending => '配对待确认';

  @override
  String get nextTransfer => '下一次传输';

  @override
  String get chooseFileHint => '选择文件，然后生成传输码交给接收方。';

  @override
  String get inboxChecksum => 'SHA-256 校验和';

  @override
  String get transfer => '传输';

  @override
  String get devices => '设备';

  @override
  String get inbox => '收件箱';

  @override
  String get settings => '设置';

  @override
  String get send => '发送文件';

  @override
  String get receive => '接收';

  @override
  String get chooseFile => '选择文件';

  @override
  String get generateCode => '生成传输码';

  @override
  String get transferCode => '传输码';

  @override
  String get connectReceive => '连接并接收';

  @override
  String get transferHint => '端到端加密文件传输。接收到的文件在主动导出前仅保存在应用私有目录。';

  @override
  String get pairRequest => '请求设备配对';

  @override
  String get pairHint => '双方必须明确同意。若对端不支持配对，请仅在接受单次传输时主动关闭此项。';

  @override
  String get cancel => '取消';

  @override
  String get refresh => '刷新';

  @override
  String get delete => '删除';

  @override
  String get rename => '备注';

  @override
  String get exportFile => '导出';

  @override
  String get save => '保存设置';

  @override
  String get saved => '设置已保存';

  @override
  String get confirmDelete => '确认删除？';

  @override
  String get deleteHint => '此操作无法撤销。请先导出需要保留的文件。';

  @override
  String get emptyInbox => '暂无已验证文件';

  @override
  String get privateInboxHint =>
      '仅显示通过 SHA-256 校验的文件。请主动导出以在应用外保留；卸载应用可能删除私有副本。';

  @override
  String get ownDevice => '本机设备';

  @override
  String get emptyDevices => '暂无可信设备';

  @override
  String get deviceName => '设备备注';

  @override
  String get receiveDevice => '从此设备接收';

  @override
  String get deviceHint => '可信设备仅在协议认证配对完成后添加，不支持远程浏览文件。';

  @override
  String get server => '信令服务器';

  @override
  String get verifyFingerprint => '验证服务器指纹';

  @override
  String get fingerprintHint => '信任前，请通过独立渠道核对以下 SHA-256 SPKI 指纹。';

  @override
  String get trust => '信任';

  @override
  String get settingsHint => '所有值由 Rust 验证并持久化。新任务使用已保存设置的冻结快照。';

  @override
  String get sendTransport => '发送协议';

  @override
  String get receiveTransport => '接收协议';

  @override
  String get profile => '网络档位';

  @override
  String get mtu => 'MTU（auto 或字节数）';

  @override
  String get congestion => '拥塞控制';

  @override
  String get words => '传输码单词数量';

  @override
  String get verbose => '日志级别';

  @override
  String get quiet => '精简传输状态';

  @override
  String get theme => '主题';

  @override
  String get language => '语言';

  @override
  String get system => '跟随系统';

  @override
  String get light => '浅色';

  @override
  String get dark => '深色';

  @override
  String get auto => '自动';

  @override
  String get lan => '局域网';

  @override
  String get wan => '广域网';

  @override
  String get advanced => '高级网络调优';

  @override
  String get streams => '并行流（留空自动）';

  @override
  String get chunkBytes => '分块字节数';

  @override
  String get windowBytes => '窗口字节数';

  @override
  String get udpBufferBytes => 'UDP 缓冲字节数';

  @override
  String get sliceBytes => '切片字节数';

  @override
  String get prefetch => '预取数量';

  @override
  String get keepaliveSecs => '保活秒数';

  @override
  String get idleTimeoutSecs => '空闲超时秒数';

  @override
  String get invalidNumber => '请输入支持范围内的整数。';

  @override
  String get cancelFileOperation => '取消文件操作';

  @override
  String get ipFamily => 'IP 地址族';

  @override
  String get ipv4 => '仅 IPv4';

  @override
  String get ipv6 => '仅 IPv6';

  @override
  String get diagnostics => '诊断';

  @override
  String get noLogs => '暂无日志。';

  @override
  String get retry => '重试';

  @override
  String get errorTitle => '操作失败';

  @override
  String get retryable => '可重试此操作。';

  @override
  String get preparingSource => '准备文件';

  @override
  String get connectingSignaling => '连接信令服务';

  @override
  String get waitingForPeer => '等待对端';

  @override
  String get sending => '发送中';

  @override
  String get receiving => '接收中';

  @override
  String get completed => '校验完成';

  @override
  String get cancelled => '已取消';

  @override
  String get failed => '失败';

  @override
  String get verifying => '等待完成与校验';

  @override
  String get closeTitle => '停止当前传输？';

  @override
  String get closeHint => '关闭将停止当前传输，已校验收件仍会保留。';

  @override
  String get stopClose => '停止并关闭';

  @override
  String bytesValue(String value) {
    return '$value 字节';
  }

  @override
  String progressValue(String bytes, String total, String rate) {
    return '$bytes / $total 字节 · $rate 字节/秒';
  }

  @override
  String errorDetails(String code, String message) {
    return '$code：$message';
  }
}
