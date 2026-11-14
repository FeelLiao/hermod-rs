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
商店版使用不同包身份。详见[商店提交说明](APP_BUILD.md#microsoft-store-identity)。

Windows 更新必须保留相同的商店包身份。旧包身份或签名不同时，先导出收件箱中的重要文件再卸载；卸载可能删除应用私有数据。

## 编译 CLI 和服务器

在仓库根目录执行；需要项目固定的 Rust 工具链（1.99 或更新版本）和本地 C/C++ 构建工具：

```text
cargo build -p hermod-cli --release --locked
```

Windows 生成 `target/release/hermod.exe`，Linux 生成 `target/release/hermod`。
客户端和 `serve` 使用同一个可执行程序。验证当前运行的版本：

```powershell
# Windows
.\target\release\hermod.exe --version
```

```bash
# Linux
./target/release/hermod --version
```

当前源码版本输出 `hermod 0.5.0`。虽然不发布 Windows CLI 安装包，但可以在 Windows
本地从源码编译使用。Linux 还需要 `cc` 等本地链接器，仅安装 Rust 不够。详见[构建说明](APP_BUILD.md)。

## CLI 设备配对与备注

0.5.0 已支持设备命令。双方须选择同一个可信信令服务器；按 ID 呼叫需要更新后的 Rust server，
不能仅凭客户端版本就认定默认公共服务器已经升级。

首次建立长期信任时，**双方都必须显式启用配对**：

```text
# 发送端
hermod tx ./hello.txt --pair
# 接收端：使用发送端显示的完整传输码
hermod rx CODE --pair
```

普通 `tx` / `rx CODE` 不自动保存联系人。配对确认与文件传输结果独立：配对确认后，
即使 QUIC 或文件传输失败，配对仍可保留。重复已有匹配配对不轮换密钥，也不覆盖本机备注。

```text
hermod device id
hermod device list
hermod device rename DEVICE_ID "我的笔记本"
hermod device remove DEVICE_ID
```

设备 ID 是公钥派生的 32 位小写十六进制字符串。`rename` 只修改本机备注，不改变 ID，
也不修改对端名称。允许同名备注，但 `--from` 必须精确且唯一匹配；重名时使用完整 ID。
符合规范的完整 ID 优先按 ID 解析，找不到时不会再按备注查找。

后续传输：

```text
# 发送端仍需主动选定本次发送内容
hermod tx ./archive.zip
# 接收端按 ID 或本机备注选择
hermod rx --from DEVICE_ID -d ./received
hermod rx --from "我的笔记本" -d ./received
```

`rx CODE` 与 `rx --from DEVICE` 互斥，`--from` 不能与 `--pair` 同时使用。
没有客户端后台常驻、远程文件浏览或任意文件拉取；同一本机身份只能发布一个发送任务，
第二个 `tx` 会立即报忙。发送时传输码和独立设备控制连接并行等待，认证成功的候选才能获胜。
未知或认证失败的 ID 呼叫不会作废传输码。服务器不支持设备控制或控制连接断开时，码入口可以继续；
需要查看该诊断时使用 `--verbose warning`。ID 认证失败不自动降级为码认证；显式配对要求双方支持并同意。

## 配置与私密文件

| 平台 | CLI 配置根目录 |
| --- | --- |
| Windows | `%APPDATA%\Hermod` |
| Linux，设置了绝对路径的 XDG_CONFIG_HOME | `$XDG_CONFIG_HOME/hermod` |
| Linux，其他情况 | `~/.config/hermod` |

无法取得对应配置位置时，当前实现回退到临时目录；这不是适合长期部署的位置。
配置目录与可执行文件位置、当前工作目录无关。

```text
配置根目录/
├── config.yaml             # 当前服务器与其 SPKI 信任
├── device-state/
│   ├── device.json         # 长期身份，包含私钥
│   ├── contacts.json       # 配对秘密、备注、pending/active 状态
│   └── *.lock              # 身份、联系人事务和发送租约
└── server/                 # serve 创建
    ├── server-cert.pem     # 公开 TLS 证书
    ├── server-key.der      # 私钥，PKCS#8 DER
    └── server-cert.json    # 证书到期元数据
```

没有 `config.yaml` 时使用内建默认值，成功执行 `trust` 后保存。
字段包括 `trust_version`、`server_url`、`trusted_servers`（目前仅保留选中的一个服务器）
及保留的 `tls_configuration` 字段。旧 PEM 字段可能保留，新 Rust server 的证书和私钥
实际位于 `server/`，不使用这些 YAML 字段。传输调优和接收目录不持久化。
同一配置目录的设备 ID 和联系人不随服务器切换而改变，但双方必须连接同一服务器；
稳定 ID 也意味着不同服务器可能关联同一设备的活动。

身份、联系人和服务端私钥都是敏感数据，不要提交到 Git 或分享。
Linux 设备存储验证所有者和 0700/0600 权限；Windows 设备存储通过系统 PowerShell
设置并验证仅当前用户和 SYSTEM 可访问的私密权限，尚未实现 DPAPI/Keychain 加密。
父目录须可信，不承诺防御同用户恶意进程。身份损坏或权限不安全时明确失败，不静默重建身份。

匹配的待确认配对可由双方再次显式 `--pair` 核对恢复；一端已 active 而另一端没有记录，
或两端记录冲突时，需要双方删除旧联系人后重新配对。删除只撤销本机信任，
不会远程删除对端记录，也不会自动恢复信任。

## 运行 Rust 信令服务器

```text
hermod serve -p 4376 --verbose info
# 按需要限制监听地址族
hermod -4 serve -p 4376 --verbose info
hermod -6 serve -p 4376 --verbose info
```

放行 TCP 4376（TLS/WSS）和 UDP 4376（地址反射），客户端之间还须允许对端 UDP 通信。
默认双栈监听，端口优先级为显式 `--port` > `HERMOD_PORT` > 4376。
`--port 0` 用于本地测试时动态分配端口；长期部署应使用固定端口并持久保存 `server/`。
删除服务端私钥会改变 SPKI 指纹，客户端必须重新建立信任。
服务在启动时检查证书续期，正常续期复用现有密钥。

读取启动时显示的指纹，并让每个客户端通过可信渠道核对后执行：

```text
hermod trust your-server.example.com:4376 --fingerprint HEX
```

注册证明只证明连接持有对应私钥，设备间的联系人信任仍由两端验证。
设备路由状态只放内存，但服务器能观察 ID、呼叫关系和源地址。
它不提供文件中转，也不能解决所有 NAT/防火墙可达性问题。本机集成测试不等于公网长期运行或负载认证；
正式部署应配置进程守护，并安全备份私钥，不泄露到日志或仓库。

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

## CLI 传输参数

以下功能已在 0.5.0 实现。唯一数据面是 QUIC，接收端自动跟随，目前没有载荷限速器。
双方保持程序打开，直到摘要验证完成。调优仅用于本次调用：显式值覆盖内建默认，未输入项使用默认；
不持久化，不从 YAML 读取。服务器信任配置仍独立持久化。

| 参数 | 默认 / 作用 |
| --- | --- |
| `-4/--ipv4`、`-6/--ipv6` | 互斥；严格约束 tx/rx/trust/serve 的网络访问（含设备控制），不跨族回退 |
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
在 serve/trust/device 中显式传入即错误，即使传的是默认值。

接收端拥有开流计划，tx 调优不能强迫 rx 并发。auto 按对端地址分类，有有效 RTT 时还需 ≤25ms 才选择 LAN 多流。
keepalive/idle-timeout 用于 QUIC，idle-timeout 也用于 CLI 载荷无进展看门狗，不改变独立的握手预算。
较大窗口、chunk 与预读队列会消耗更多资源，不保证吞吐提升。默认连接窗口预算为 512MiB，
最大设置为 2GiB，但不表示启动时立即分配这么多内存。单个 chunk × prefetch 队列预算约为
1–256MiB，此外还有其他缓冲。应在实际链路上测量吞吐、延迟和内存，目前没有前后提速对照结论。

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
- **设备 ID 连不上**：确认双方选择同一个已升级 Rust server、发送端正在运行 `tx`、双方保留匹配的 active 配对。设备 ID 不是密码。
- **发送时网页很卡**：上行跑满会挤占其他请求；当前没有载荷限速器，可考虑路由器 SQM/AQM 或外部流量整形。
- **导出失败**：收件箱副本仍在，重新选择可写目录后导出。
- **Windows MSIXBundle 无法安装**：GitHub 文件仅供商店提交，尚未签名；请在审核上线后安装商店签名的版本。


Actions 分为两个工作流：`CI` 负责分支推送和 PR 检查；`Build and release` 复用同一套检查，
通过后才打包。推送 `master` 只检查，推送 `v*` 标签才发布 Release；手动运行只生成 Actions 构建产物，不发布。

[构建与发布](APP_BUILD.md) · [core 架构说明](core/README.md) · [更新日志](CHANGELOG.md) · [隐私政策 / Privacy policy](PRIVACY.md) · [许可证](LICENSE)

项目灵感来自 [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole)。协议采用 CPace、
经过认证的端点交换和传输加密；与旧 Go 实现共享原有信令及传输码格式，
不代表 Go 支持新增设备控制协议或当前载荷版本。
