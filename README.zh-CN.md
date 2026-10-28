# Hermod

[English](README.md) · [简体中文](README.zh-CN.md)

**选一个文件，分享传输码，直接传到另一台设备。**
Hermod 是使用 Rust 编写的加密 P2P **IPv4 局域网 / IPv6** 文件传输工具。文件在设备之间直传，
信令服务器只帮助双方建立连接，不中转文件。

- **电脑端**：Tauri 2 + React + 微软官方 Fluent UI React v9，采用 Fluent 2 设计。
- **安卓端**：Tauri + Google 官方 Material Design 3 Web 控件，与桌面共用 React 页面和交互逻辑，支持动态配色、手机和平板。
- **Linux 命令行**：musl 静态可执行文件，适合终端和脚本。
- GUI 默认 **QUIC 发送、Auto 接收、不限速**；CLI 默认 WireGuard 发送，也可选择 QUIC。
- 支持文件、文本、取消、进度、限速、服务器指纹信任与 SHA-256 校验。
- 传输在用户态完成，不需要配置 VPN、创建 TUN 网卡或使用管理员权限。

## 下载哪个文件？

打开 [GitHub Releases](https://github.com/FeelLiao/hermod-rs/releases/latest)，根据接收或发送设备自己的架构选择：

| 设备 | 下载文件 |
| --- | --- |
| Windows 10 2004+ / Windows 11，x64 / ARM64 | `hermod-windows.msixbundle`（包含两种架构，仅供商店提交） |
| Linux 桌面 x64 | `hermod-linux-x64.AppImage` |
| Linux 命令行 x64 / ARM64 | `hermod-linux-x64-musl` / `hermod-linux-arm64-musl` |
| Android 14 及以上、ARM64 | `hermod-android-arm64.apk` |

不再构建 macOS、Linux ARM64 GUI、Windows CLI 和 Android CLI。Release 只上传 5 个程序或包，不上传证书、安装脚本、ZIP、deb、checksum、私钥、日志或调试符号。
Windows MSIXBundle 是包含 x64 和 ARM64 的未签名商店提交包；普通用户请在产品上线后通过微软商店安装 **HermodTransfer**，商店会选择对应架构。
GitHub 自动提供的源码下载链接属于平台功能。

## 连接前先确认网络

**双方处于同一 IPv4 局域网，或具有可达的 IPv6，并允许对端 UDP 通信。**
QUIC 和 WireGuard 均支持私有 / 链路本地 IPv4 地址及纯 IPv4 网络。配对仍需连接所选信令服务器；
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

收件箱只展示校验完成的文件。导出失败、目录不可写或 Android 授权过期时，已验证副本仍保留。
文本会保存为文本文件。取消和进程结束不会保留可续传任务；本版不提供断点续传。

Android 使用系统文件选择器和目录授权，不申请全盘访问。传输由前台服务持有，界面旋转、重建不会
重新发起传输；通知显示进度并提供取消。Android 17 需要局域网 / 附近设备权限，拒绝时会明确提示。
通知权限用于显示传输通知，和文件权限无关。

**语言**：电脑与安卓界面均支持简体中文和英文，默认跟随系统。可在 **设置 → 语言** 选择
**简体中文 / English / 跟随系统**，点击 **保存设置** 后下次打开仍使用此选择。界面语言立即切换，
技术日志和未识别的底层错误详情保留原文以便排查。

设置包括：发送与接收协议、统一 MTU、拥塞控制、限速、传输码长度、服务器、指纹信任、接收目录、
日志级别、减少提示、主题和语言。设置会持久化；每次开始传输时取得快照，修改只影响后续传输。
新安装的 GUI 默认 **QUIC 发送、Auto 接收、不限速（0）**。旧设置保持不变，可在设置中点击
**采用新默认设置**，再保存。需要为其他应用留出带宽时，仍可手动设置限速。

发送前的加密协商会预告文件名、类型和大小，接收端立即显示总大小，并校验后续 HTTP 信息是否一致。
旧客户端仍可通过 IPv6 互通，旧发送端的大小从 HTTP 响应头补齐；IPv4 局域网传输需要双方升级。

## 安装和升级

**Windows**：商店审核上线后，从 Microsoft Store 安装 **HermodTransfer**，发布者为 **FeelLiao**，
Store ID 为 **9PMPWN3HMR3M**。维护者只需把 GitHub 的 `hermod-windows.msixbundle` 上传到 Partner Center
的 **程序包 / Packages** 页面，其中已包含 x64 和 ARM64；审核通过后由微软签名并选择对应架构。GitHub 上的未签名包无法直接双击安装，
不再附带测试证书或安装脚本。旧 `Hermod.Transfer.Test` 版本迁移前，请先导出收件箱文件；
商店版使用不同包身份。详见[商店提交说明](APP_BUILD.md#microsoft-store-packages)。

**Linux**：以 Ubuntu 22.04 及兼容更新桌面为基线：

```bash
chmod +x hermod-linux-x64.AppImage
./hermod-linux-x64.AppImage
# 没有 FUSE 时：
APPIMAGE_EXTRACT_AND_RUN=1 ./hermod-linux-x64.AppImage
```

**Android**：允许下载应用安装 APK 后打开文件。仅发布 ARM64；最低 API 34，compileSdk/targetSdk 37。
后续更新必须保留相同签名密钥。旧临时密钥版本无法直接覆盖时，**先导出收件箱中的重要文件，再卸载重装**。
Windows 旧包身份或签名不同时同样先导出。卸载可能删除应用私有数据。

## 命令行快速开始

```bash
chmod +x hermod-linux-x64-musl
./hermod-linux-x64-musl tx ./archive.zip --limit-mbps 8
# 在另一端把 CODE 替换为完整传输码
./hermod-linux-x64-musl rx CODE --destination ./received
```

也可将程序安装为 `hermod`：

```bash
hermod tx 'Hello from Hermod'
printf 'Read from stdin\n' | hermod tx -
hermod tx ./archive.zip --transport quic --mtu 1280 --congestion bbr
hermod rx CODE --transport wireguard --limit-mbps 8
hermod --help
hermod tx --help
```

CLI 默认不限速，`0` 表示不限速；发送默认 WireGuard，接收默认 Auto。双方保持程序打开，直到摘要验证完成。

| 参数 | 作用 |
| --- | --- |
| `--transport wireguard\|quic` | 发送协议；接收还支持 `auto` 跟随发送端 |
| `--mtu auto\|数值` | 控制本次实际使用协议的 MTU |
| `--congestion auto\|cubic\|reno\|bbr\|newreno` | 只接受实际协议支持的算法 |
| `--limit-mbps 数值` | 十进制 Mbps 载荷速率，发送和接收限制取较小值 |
| `--server URL` | 信令服务器，需要预先建立信任 |
| `--words 数值` | 发送端传输码词数，1–32，默认 3 |
| `--destination 路径` | 接收目录 |
| `--verbose 级别`、`--quiet` | 全局日志级别和减少状态提示 |

**MTU**：WireGuard 表示内层 IP MTU，auto 为 1400，允许 576–65488。QUIC 表示 UDP 载荷上限，
允许 1200–1452；auto 从 1200 开始探测，最高 1452；显式设置后不会探测超过该值。WireGuard 的
较大 MTU 不保证更快，还需为外层 IPv6、UDP 和 WireGuard 头部预留空间。

**拥塞控制**：auto 在 WireGuard 下为 CUBIC，在 QUIC 下为 BBR。WireGuard 支持 CUBIC/Reno；
QUIC 支持 BBR/CUBIC/NewReno。不兼容组合会报错，不会静默忽略。Auto 接收在知道对端协议后同样校验。

**自建服务器信任**：从可信渠道取得服务器 SPKI SHA-256 指纹后，运行
`hermod trust SERVER --fingerprint HEX`。GUI 可以获取指纹并显式确认；仅从网络获取指纹本身不能证明服务器身份。
每个客户端配置只保留**一个信任的服务器**。验证成功后同时替换旧信任并切换服务器，失败则保留原配置。
旧信任列表按当前选中的服务器迁移；`--server` 必须与唯一信任的地址匹配。CLI 配置位于 `$XDG_CONFIG_HOME/hermod/config.yaml`，默认是
`~/.config/hermod/config.yaml`；GUI 配置和收件箱位于各平台应用数据目录。

## 从 0.3.x 迁移

| 已删除参数 / 功能 | 替代方法 |
| --- | --- |
| `--wg-mtu N` | `--transport wireguard --mtu N` |
| `--quic-mtu N` | `--transport quic --mtu N`，现在也是探测硬上限 |
| `--quic-cc 算法` | `--transport quic --congestion 算法` |
| `--listen 地址` / bind address | 自动监听 `[::]:0`，无需手动配置 |
| 未实现的 `serve` | 已删除，信令服务需要单独部署兼容实现 |

电脑端使用 Fluent 2，安卓使用 Material Design 3 Web。两端复用 `hermod-app` 中的 React 页面、交互逻辑与 Rust 业务层；Android 原生代码只适配系统文件授权、权限、配色和前台通知。
已有桌面收件箱保留，旧界面的限速和主题偏好在首次启动时尽可能迁移。

## 常见问题

- **传输码连不上**：检查双方 IPv4 局域网 / IPv6 路由、信令服务器、UDP 防火墙、完整传输码及强制协议是否一致。
- **发送时网页很卡**：降低 GUI 限速或 `--limit-mbps`；上行跑满会挤占其他请求，见[性能分析](NETWORK_PERFORMANCE.md)。
- **导出失败**：收件箱副本仍在，重新选择可写目录后导出；Android 重新授权目录。
- **Windows MSIXBundle 无法安装**：GitHub 文件仅供商店提交，尚未签名；请在审核上线后安装商店签名的版本。

Android 在 Actions 中只编译、签名并检查 ARM64 APK，不运行安卓模拟器或界面测试。

Actions 分为两个工作流：`CI` 负责分支推送和 PR 检查；`Build and release` 复用同一套检查，
通过后才打包。推送 `master` 只检查，推送 `v*` 标签才发布 Release；手动运行只生成 Actions 构建产物，不发布。

[构建与发布](APP_BUILD.md) · [架构与贡献指南](AGENTS.md) · [更新日志](CHANGELOG.md) · [隐私政策 / Privacy policy](PRIVACY.md) · [许可证](LICENSE)

项目灵感来自 [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole)。协议采用 CPace、
经过认证的端点交换和传输加密；与 Go 实现共享信令及传输码格式。
