# Hermod

[English](README.md) · [简体中文](README.zh-CN.md)

**选一个文件，分享传输码，直接传到另一台设备。**
Hermod 是使用 Rust 编写的加密 P2P **IPv4 局域网 / IPv6** 文件传输工具。文件在设备之间直传，
信令服务器只帮助双方建立连接，不中转文件。

- **Windows GUI**：Flutter Material 3 + Rust Core 子进程，通过 Microsoft Store 的 x64/ARM64 MSIXBundle 发布。
- **Android**：Flutter runner 已保留用于后续平台接入，目前不发布 Android 安装包。
- **Linux 命令行**：musl 静态可执行文件，适合终端和脚本。
- 唯一数据面协议为 **QUIC**（TLS 1.3，默认 BBR）；GUI 与 CLI 一致，接收端自动跟随。
- 支持文件、文本、取消、进度、服务器指纹信任与 SHA-256 校验。
- 传输在用户态完成，不需要配置 VPN、创建 TUN 网卡或使用管理员权限。

## 下载哪个文件？

打开 [GitHub Releases](https://github.com/FeelLiao/hermod-rs/releases/latest)，根据接收或发送设备自己的架构选择：

| 设备 | 下载文件 |
| --- | --- |
| Windows 10 2004+ / Windows 11，x64 / ARM64 | [Microsoft Store 中的 HermodTransfer](https://apps.microsoft.com/detail/9PMPWN3HMR3M) |
| Linux 命令行 x64 / ARM64 | `hermod-linux-x64-musl` / `hermod-linux-arm64-musl` |

不再构建 macOS、Linux GUI、Windows CLI 或 Android 安装包。Release 只上传两个 Linux CLI 程序，不上传证书、安装脚本、ZIP、deb、checksum、私钥、日志或调试符号。
Windows MSIX/MSIXBundle 仅用于商店提交，不放入 GitHub Release。用户通过微软商店安装 **HermodTransfer**，商店会选择对应架构。
GitHub 自动提供的源码下载链接属于平台功能。

## 连接前先确认网络

**双方处于同一 IPv4 局域网，或具有可达的 IPv6，并允许对端 UDP 通信。**
QUIC 支持私有 / 链路本地 IPv4 地址及纯 IPv4 网络。配对仍需连接所选信令服务器；
本版不提供离线发现、互联网 IPv4 NAT 打洞或文件中转。
有 IPv6 地址不代表路由器、防火墙一定放行。默认信令地址是
`wss://oracle.stariverfeel.eu.org:4376`，已内置其 SPKI 指纹。传输码只在服务器成功分配频道后显示。

WSL NAT 通常没有可用 IPv6。支持镜像网络的 Windows 11 / WSL 可在 Windows 用户目录的
`.wslconfig` 中设置 `networkingMode=mirrored`，然后从 Windows 执行 `wsl --shutdown`。
前提是 Windows 所在网络已经支持 IPv6；随意添加一个 IPv6 地址不能获得公网路由。
详见[网络与性能说明](NETWORK_PERFORMANCE.md)。

## 图形界面：三步传输

1. 发送端在 **文件** 选择文件，或在 **文本** 输入内容，开始发送。
2. 私下分享完整传输码；接收端在 **接收** 输入它并连接。
3. 等待 **SHA-256 校验通过**，在 **收件箱 → 另存副本** 导出，也可提前选择默认导出目录。

收件箱只展示校验完成的文件。导出失败或目录不可写时，已验证副本仍保留。文本会保存为文本文件。取消和进程结束不会保留可续传任务；本版不提供断点续传。传输状态由 Rust Core 持有，Flutter 仅负责界面。

**语言**：Windows 界面支持简体中文和英文，默认跟随系统。可在 **设置 → 语言** 选择
**简体中文 / English / 跟随系统**，点击 **保存设置** 后下次打开仍使用此选择。界面语言立即切换，
技术日志和未识别的底层错误详情保留原文以便排查。

GUI 设置会本地持久化，每次开始传输时取得快照，修改只影响后续传输。
下述 CLI 调优工作不修改 GUI/bridge 设置，也不保证旧 `compatible` 档位值兼容；
新增 CLI 调优参数不持久化，也不从 YAML 读取。

发送前的加密协商会预告文件名、类型和大小。当前 payload v3 直接使用 QUIC 流并校验 SHA-256，
不再支持 HTTP 或 WireGuard 载荷。请双方使用升级后的客户端；协议版本不兼容时明确报错。

## 安装和升级

**Windows**：从 [Microsoft Store](https://apps.microsoft.com/detail/9PMPWN3HMR3M) 安装已上架的 **HermodTransfer**，发布者为 **FeelLiao**，
Store ID 为 **9PMPWN3HMR3M**。以后推送版本标签，全部检查和打包、GitHub Release 成功后，Actions 会自动提交
包含 x64 和 ARM64 的商店包；微软完成审核后按现有发布设置分发更新。Windows 程序包仅保留为 Actions 内部产物并提交商店，不附加到 GitHub Release。
旧 `Hermod.Transfer.Test` 版本迁移前，请先导出收件箱文件；
商店版使用不同包身份。详见[商店提交说明](APP_BUILD.md#microsoft-store-packages)。

Windows 更新必须保留相同的商店包身份。旧包身份或签名不同时，先导出收件箱中的重要文件再卸载；卸载可能删除应用私有数据。

## 命令行快速开始

```bash
chmod +x hermod-linux-x64-musl
./hermod-linux-x64-musl tx ./archive.zip
# 在另一端把 CODE 替换为完整传输码
./hermod-linux-x64-musl rx CODE -d ./received
```

也可将程序安装为 `hermod`：

```bash
hermod tx 'Hello from Hermod'
printf 'Read from stdin\n' | hermod tx -
hermod -4 tx ./archive.zip --mtu 1280 --congestion bbr
hermod rx CODE --streams 4 --output ./received
hermod serve -p 4376
hermod --help
hermod tx --help
```

**CLI 调优状态：实施中，等待用户验证。** 唯一数据面是 QUIC，接收端自动跟随，目前没有载荷限速器。
双方保持程序打开，直到摘要验证完成。调优仅用于本次调用：显式值覆盖内建默认，未输入项使用默认；
不持久化，不从 YAML 读取。服务器信任配置仍独立持久化。详见[已确认实施方案](docs/cli-transport-tuning.md)。

| 参数 | 默认 / 作用 |
| --- | --- |
| `-4/--ipv4`、`-6/--ipv6` | 互斥；严格约束全部四个命令，不跨族回退 |
| `-m/--mtu auto\|数值` | auto；QUIC UDP 载荷上限 1200–1452 字节；auto 从 1200 探测到 1452 |
| `-c/--congestion auto\|bbr\|cubic\|newreno` | bbr；auto 同样选择 BBR |
| `-P/--profile auto\|lan\|wan` | auto；不保留 compatible 别名 |
| `--streams auto\|1..8` | auto；rx 决定实际并发，仍受源能力与文件大小限制 |
| `--chunk 大小` | 1MiB；256KiB–4MiB |
| `--window 大小` | 单流 64MiB；8MiB–256MiB；连接接收/发送预算 = window × 8 |
| `--udp-buffer 大小` | 16MiB；1MiB–64MiB，请求的 socket 缓冲 |
| `--slice-size 大小` | 4MiB；4MiB–64MiB，切片目标大小 |
| `--prefetch 数量` | 16；4–64 个预读块 |
| `--keepalive 秒` | 10；0–60，0 关闭 keepalive |
| `--idle-timeout 秒` | 30；10–300，且必须严格大于 keepalive |
| rx `-d/--output 目录` | `.`；接收目录 |
| serve `-p/--port 端口` | 纯数字 0–65535；CLI > HERMOD_PORT > 4376 |
| `--verbose 级别`、`-q/--quiet` | 全局日志级别（默认 none）和减少状态提示 |

枚举值大小写不敏感。大小仅接受整数字节或整数 KiB/MiB，例如 `1048576`、`1024KiB`、`1MiB`；
不接受小数、MB 或内嵌空格。非法值在联网前报错。全部数据面参数（mtu、congestion、profile 和八项调优）
在 serve/trust 中显式传入即错误，即使传的是默认值。

接收端拥有开流计划，tx 调优不能强迫 rx 并发。auto 按对端地址分类，有有效 RTT 时还需 ≤25ms 才选择 LAN 多流。
keepalive/idle-timeout 用于 QUIC，idle-timeout 也用于 CLI 载荷无进展看门狗，不改变独立的握手预算。
较大窗口、chunk 与预读队列会消耗更多资源，不保证吞吐提升，详见[内存权衡与测量](NETWORK_PERFORMANCE.md)。

**自建服务器信任**：从可信渠道取得服务器 SPKI SHA-256 指纹后，运行
`hermod trust SERVER --fingerprint HEX`。GUI 可以获取指纹并显式确认；仅从网络获取指纹本身不能证明服务器身份。
每个客户端配置只保留**一个信任的服务器**。验证成功后同时替换旧信任并切换服务器，失败则保留原配置。
旧信任列表按当前选中的服务器迁移；tx/rx 使用该服务器，不再支持本次传输的 `--server` 覆盖。CLI 配置位于 `$XDG_CONFIG_HOME/hermod/config.yaml`，默认是
`~/.config/hermod/config.yaml`；GUI 配置和收件箱位于各平台应用数据目录。

## 从 0.3.x 迁移

| 已删除参数 / 功能 | 替代方法 |
| --- | --- |
| `--wg-mtu`、`--transport`、`--limit-mbps` | 已移除；仅 QUIC，没有载荷限速器 |
| `--quic-mtu N` | `--mtu N`，探测硬上限 |
| `--quic-cc 算法` | `--congestion 算法` |
| tx/rx `--server`、tx `--words` | 用 trust 选择服务器；CLI 固定三词码 |
| rx `--destination 目录` | `-d/--output 目录` |
| `--profile compatible` | 按需求使用 wan；旧值与别名均删除 |
| serve `-l/--listen`、HERMOD_LISTEN | `serve -p/--port`、HERMOD_PORT；CLI > 环境变量 > 4376 |

`serve` 现为实际的信令/NAT 辅助服务，默认双栈监听，可用全局 -4/-6 限制地址族；它不提供文件中转。

Windows 使用 Flutter Material 3。GUI 经 flutter_rust_bridge 以 FFI 加载 `hermod_flutter_bridge.dll`，其 `gui_api` 门面持有设置、任务和私有收件箱，传输能力全部依赖 `hermod-core`；私有收件箱路径不会进入 Flutter 层。

## 常见问题

- **传输码连不上**：检查双方 IPv4 局域网 / IPv6 路由、信令服务器、UDP 防火墙、完整传输码、客户端版本及 -4/-6 限制。
- **发送时网页很卡**：上行跑满会挤占其他请求；当前没有载荷限速器，可考虑路由器 SQM/AQM 或外部流量整形，见[性能分析](NETWORK_PERFORMANCE.md)。
- **导出失败**：收件箱副本仍在，重新选择可写目录后导出。
- **Windows MSIXBundle 无法安装**：GitHub 文件仅供商店提交，尚未签名；请在审核上线后安装商店签名的版本。


Actions 分为两个工作流：`CI` 负责分支推送和 PR 检查；`Build and release` 复用同一套检查，
通过后才打包。推送 `master` 只检查，推送 `v*` 标签才发布 Release；手动运行只生成 Actions 构建产物，不发布。

[构建与发布](APP_BUILD.md) · [架构与贡献指南](AGENTS.md) · [更新日志](CHANGELOG.md) · [隐私政策 / Privacy policy](PRIVACY.md) · [许可证](LICENSE)

项目灵感来自 [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole)。协议采用 CPace、
经过认证的端点交换和传输加密；与 Go 实现共享信令及传输码格式。
