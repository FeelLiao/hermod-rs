# HermodTransfer / Hermod Privacy Policy · 隐私政策

[简体中文](#简体中文) · [English](#english)

## 简体中文

更新及生效日期：2026 年 10 月 28 日。

本政策适用于 FeelLiao 发布的 HermodTransfer（Microsoft Store ID：`9PMPWN3HMR3M`），
以及同一项目的 Hermod 桌面端、Android 端和命令行客户端。下文“应用”指这些客户端，
“我们”指发布者 FeelLiao。隐私联系邮箱：[feel2027@outlook.com](mailto:feel2027@outlook.com)。

### 1. 应用如何工作

应用通过传输码让两台设备建立加密的 IPv4 局域网或 IPv6 点对点连接，用于发送你选择的文件或输入的文本。
信令服务器协助双方配对和交换连接信息；文件内容在两台设备之间直传，不经过信令服务器中转。
应用不要求注册账户。我们没有在应用中集成广告、用户行为分析，或自动向开发者上传日志、文件及崩溃报告的功能。
这不表示应用“完全不处理个人信息”：你选择的内容、文件名和连接地址本身可能包含个人信息。

### 2. 处理哪些数据，以及处理目的

- **发送和接收内容**：应用读取你选择的文件、输入的文本，或 CLI 接收的标准输入，并处理文件名、大小、内容类型和 SHA-256 摘要，用于传输、进度显示、完整性校验及保存。接收方会得到内容及传输所需的文件元数据。
- **连接数据**：应用处理本机与对端的 IP 地址、端口、传输码、配对频道编号、协议和握手数据，用于找到对方、验证双方掌握相同传输码并建立连接。对端可以获知建立直连所需的网络地址；传输码不是实名身份认证。
- **本地偏好及授权**：应用保存语言、主题、限速、协议、MTU、日志级别、信令服务器地址、信任指纹、默认接收位置等设置。位置可能是桌面文件路径或 Android 授权 URI。旧版本的界面本地存储可能保有限速及主题偏好。
- **本地诊断**：启用日志时，应用可能记录连接地址、频道编号、文件名或路径、大小、进度及错误信息，用于排查问题。应用不会自动把这些日志发送给我们。
- **你主动发送的反馈**：如果你通过邮件或 GitHub 联系我们，我们会收到你的邮箱地址或 GitHub 账号，以及你提供的问题描述和附件，用于回复、排错和处理隐私请求。请仅提供处理问题所需的信息。

### 3. 信令服务器和其他接收方

默认信令地址是 `wss://oracle.stariverfeel.eu.org:4376`。这是第三方服务，并非由本应用发布者管理。
服务器能够看到你的连接来源地址、配对频道编号、请求时间和信令消息；它中继密码协商消息，以及加密的端点与文件预告信息（文件名、类型、大小）。
客户端不会向信令服务器直接发送传输码的密码单词、明文文件内容或文本正文。
我们无法确认该第三方服务器的实际日志内容、保存期限或服务器所在地，因此不承诺其“零日志”或立即删除数据。
你可以在设置中改用自行信任的兼容信令服务器，并独立核对其指纹和隐私安排。
获取或验证自定义服务器指纹也会连接该服务器，使其看到连接来源地址。

文件及文本会发送给掌握完整传输码并成功连接的对端。请只与预期接收方分享传输码；
对方收到内容后可能自行保存或转发，应用无法远程撤回或删除对方的副本。
连接数据可能发送到信令服务器所在国家或地区，文件会发送到接收方所在地区；应用不限制这些服务或设备的地理位置。

如果你选择系统文件选择器中的云盘或云端文档，或将文件另存到云同步目录，相关提供方可能处理该文件。
Microsoft Store、操作系统、WebView2、系统剪贴板或备份服务也可能按各自设置和政策处理运行、诊断或同步数据；
这些服务的处理不等于应用向开发者上传文件。使用邮件咨询时，邮箱服务商会处理邮件。
访问 GitHub 上的项目、政策或提交问题时，适用 [GitHub 隐私声明](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement)。

### 4. 保存位置和保留期限

- **已验证收件**：GUI 将收到的文件和对应文件名、大小、摘要保存在设备的应用私有收件箱，直到你删除或清除相应应用数据。另存副本或导出失败不会自动删除收件箱副本。CLI 将接收结果保存到你指定的位置；这些文件由你自行管理。
- **临时数据**：未知长度的发送来源可能先缓存到应用私有目录，接收过程中也会写入临时文件。应用在任务结束时尝试清理临时数据；异常退出遗留的未完成数据在下次启动、成功取得存储锁后清理。清理不会删除已有校验收据的收件。
- **设置和授权**：配置、信任记录及默认位置保留在设备上，直到你修改或清除相应数据。Android 的持久化目录授权由系统保管。卸载或重置后的残留、备份和同步副本取决于相应平台，不能承诺安全擦除。
- **日志和当前任务**：GUI 诊断日志保存在进程内存中，最多保留最近 500 条，退出进程后不再由该内存缓冲保存。当前任务状态主要保存在内存中。CLI 输出、终端历史、你保存的日志以及操作系统生成的诊断文件，由你和相应系统管理。
- **联系记录**：我们仅在回复问题、处理请求及满足适用法律要求所必需的期间保留你主动提交的联系记录。你可以通过下方邮箱询问或请求删除。第三方信令服务的记录不由我们控制。

### 5. 安全措施及其边界

信令连接采用 TLS 和服务器公钥指纹校验；双方使用基于传输码的密码协商，并通过 WireGuard 或 QUIC 加密传输。
应用使用 SHA-256 检查传输完整性。这些措施保护传输过程，不能阻止已获授权的接收方使用收到的内容。
“私有收件箱”指应用的数据目录，并不表示文件经过应用额外的静态加密。
本地文件、缓存和设置依赖操作系统的权限和设备安全措施；请保护设备和传输码。

### 6. 权限、剪贴板和通知

Windows 版本以普通桌面程序运行；其网络及完整信任能力用于 Rust/Tauri 程序、网络通信和你发起的文件操作。
桌面 GUI 使用系统文件选择器；应用读写你选择的文件、导出位置及自身应用数据。
点击“复制传输码”时，应用将该码写入系统剪贴板，不会持续读取剪贴板。剪贴板历史或跨设备同步由系统设置控制。

Android 使用系统 Storage Access Framework 授权读取选定文件、写入选定目录，不申请全盘文件访问权限。
前台服务和保持唤醒权限用于继续传输，通知显示任务状态、进度及取消入口；
系统要求时，局域网访问权限用于对端通信。你可在系统设置中管理通知和网络权限；拒绝相关权限可能影响传输。
选择默认导出目录会保存目录授权；在应用内清除默认目录仅停止使用该默认位置，
不等于撤销系统保存的授权。需要彻底清除时，可使用系统提供的授权管理或清除应用数据功能，并先导出重要文件。
应用不要求通讯录、麦克风、摄像头或精确位置权限来传输文件。

### 7. 你的选择和请求

你可以不选择文件或不开始传输，取消正在进行的任务，修改网络和日志设置，另存或删除收件箱文件，
以及在系统设置中管理应用权限或清除数据。取消传输不会撤回已经发送的数据；
删除收件箱条目不会删除已经导出的副本。原始发送文件、导出副本、系统备份和接收方副本需分别管理。
卸载或清除应用数据前，请先导出你需要保留的收件。

如需查询、更正或请求删除我们持有的联系记录，或行使你所在地适用的其他隐私权利，
请发送邮件至 [feel2027@outlook.com](mailto:feel2027@outlook.com)。我们会根据请求及适用要求处理，
并在必要时核对请求来源。我们不能远程读取或删除仅保存在你的设备、对端设备或独立第三方服务上的数据。
请不要在公开 GitHub Issue 中发布私人文件、完整传输码或未脱敏日志。

### 8. 未成年人和政策更新

应用是通用文件传输工具，没有面向儿童的账户或画像功能。未成年人应在适用法律和监护人指导下使用，
不要分享没有权限披露的个人信息。监护人如担心儿童已向我们提交信息，可通过上述邮箱联系。
功能或数据处理方式变化时，我们会更新本页面及日期；需要额外告知或同意时，将按适用要求处理。

## English

Updated and effective: October 28, 2026.

This policy covers HermodTransfer, published by FeelLiao (Microsoft Store ID:
`9PMPWN3HMR3M`), and the same project's Hermod desktop, Android and command-line
clients. “App” means these clients; “we” means the publisher FeelLiao.
Privacy contact: [feel2027@outlook.com](mailto:feel2027@outlook.com).

### 1. How the app works

The app uses a transfer code to establish an encrypted IPv4 LAN or IPv6 peer-to-peer connection
for a file you select or text you enter. A signaling server helps pair devices and
exchange connection information. File contents travel directly between the devices,
not through the signaling server. No app account is required. We have not integrated
advertising, behavioral analytics, or automatic uploads of logs, files or crash reports
to the developer. This does not mean that the app processes no personal information:
selected content, filenames and network addresses can themselves be personal information.

### 2. Data processed and purposes

- **Content sent or received:** the app reads selected files, entered text or CLI standard input, and processes filenames, sizes, content types and SHA-256 digests to transfer, show progress, verify integrity and save data. The recipient receives the content and file metadata needed for the transfer.
- **Connection data:** local and peer IP addresses, ports, transfer codes, pairing channel numbers, protocol choices and handshake data are processed to find a peer, verify possession of the same code and connect. Peers learn network addresses needed for direct communication. A transfer code does not verify a person's real-world identity.
- **Local preferences and permissions:** language, theme, bandwidth, protocol, MTU, logging, signaling address, trusted fingerprints and default destination are stored locally. A destination may be a desktop path or an authorized Android URI. Older UI local storage may retain bandwidth and theme preferences.
- **Local diagnostics:** when enabled, logs may include network addresses, channel numbers, filenames or paths, sizes, progress and errors for troubleshooting. The app does not automatically send these logs to us.
- **Feedback you send:** if you contact us by email or GitHub, we receive your email address or GitHub account and the description and attachments you provide, to respond, troubleshoot and handle privacy requests. Please provide only information needed to address the issue.

### 3. Signaling servers and other recipients

The default signaling address is `wss://oracle.stariverfeel.eu.org:4376`. This is a
third-party service, not managed by the app publisher. The server can see connection
source addresses, pairing channel numbers, request timing and signaling messages.
It relays password-agreement messages and encrypted endpoint information and file offers (name, type and size). The client
does not directly send the code's password words, plaintext file contents or message
text to the signaling server. We cannot confirm that server's actual logging,
retention periods or hosting location, and do not promise zero logging or immediate
deletion by it. You can choose a compatible server you trust in Settings and
independently check its fingerprint and privacy practices. Probing or verifying a
custom server's fingerprint also connects to that server and exposes the source address.

Content is sent to the peer that possesses the complete code and successfully
connects. Share the code only with your intended recipient. Recipients may keep or
forward received content; the app cannot remotely recall or delete their copies.
Connection data may reach the country or region hosting the signaling server, and
files reach the recipient's location. The app does not restrict their geographic locations.

Cloud drives or document providers selected through the system picker, and cloud-synced
export folders, may process the corresponding files. Microsoft Store, operating systems,
WebView2, system clipboard and backup services may separately process runtime,
diagnostic or synchronized data under their own settings and policies; this is distinct
from an app upload of files to the developer. Email providers process support mail.
Visiting the project or policy on GitHub, or submitting an issue there, is subject to
the [GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).

### 4. Storage and retention

- **Verified receipts:** the GUI keeps received files and their names, sizes and digests in the device's private app inbox until you delete them or clear the corresponding app data. Exporting, or an export failure, does not automatically remove the inbox copy. CLI downloads are saved to your chosen destination and managed by you.
- **Temporary data:** sources with unknown lengths may be staged in private storage; receiving also writes temporary files. The app attempts cleanup when the task ends. Incomplete data left by a crash is cleaned on the next startup after the storage lock is acquired. Cleanup preserves receipts with completed verification records.
- **Settings and authorizations:** configuration, trust records and destinations remain until changed or cleared. Android keeps persistent folder permissions in the system. Residual files, backups or synchronized copies after uninstall/reset depend on the platform; secure erasure is not promised.
- **Logs and current tasks:** GUI diagnostic logs use an in-memory buffer of up to 500 recent entries, which is no longer retained by that buffer after process exit. Current task state is mainly held in memory. CLI output, terminal history, saved logs and OS-generated diagnostics are managed by you and the relevant systems.
- **Contact records:** we retain correspondence you submit only as long as needed to answer questions, handle requests and meet applicable legal requirements. You can inquire or request deletion using the contact below. Third-party signaling records are outside our control.

### 5. Security and its limits

Signaling uses TLS and server public-key fingerprint verification. Peers use
password agreement based on the transfer code and WireGuard or QUIC encryption.
SHA-256 verifies transfer integrity. These measures protect transfers, but do not
prevent an authorized recipient from using received content. A “private inbox” is
an app data directory, not additional app-level encryption at rest. Local files,
cache and settings rely on operating-system permissions and device security.
Protect your device and transfer codes.

### 6. Permissions, clipboard and notifications

On Windows, the app runs as a desktop program. Its network and full-trust capabilities
support the Rust/Tauri process, communications and file operations you initiate.
Desktop GUI operations use native pickers and access your selected files, export
locations and the app's own data. Choosing “Copy code” writes the code to the system
clipboard; the app does not continuously read it. Clipboard history and cross-device
sync are controlled by the system.

Android uses the Storage Access Framework to access selected documents and folders,
without requesting all-files access. A foreground service and wake lock support
ongoing transfers; notifications show task status, progress and cancellation. Where
required, local-network permission supports peer communication. System settings
control notification and network permissions; denial can affect transfers. Choosing
a default export folder saves a folder authorization. Clearing the default in the
app stops using that destination but does not revoke the system's persisted grant.
Use available system permission controls or clear app data to remove it, exporting
important files first. File transfers do not require contacts, microphone, camera
or precise-location permissions.

### 7. Your choices and requests

You can refrain from selecting content or starting a transfer, cancel a task,
change network/log settings, export or delete inbox items, and manage permissions
or clear data through the operating system. Cancellation does not recall data
already sent. Deleting an inbox item does not delete exported copies. Original
source files, exports, backups and recipient copies must be managed separately.
Export important receipts before uninstalling or clearing app data.

To ask about, correct or request deletion of contact records we hold, or exercise
other privacy rights available in your location, email
[feel2027@outlook.com](mailto:feel2027@outlook.com). We will handle the request under
applicable requirements and verify its source when necessary. We cannot remotely
read or erase data held only on your device, a peer's device or an independent
third-party service. Do not post private files, complete transfer codes or
unredacted logs in public GitHub issues.

### 8. Minors and changes

The app is a general-purpose transfer tool, without child-specific accounts or
profiling features. Minors should use it under applicable laws and parental guidance
and avoid sharing personal information they are not authorized to disclose.
Guardians concerned about information a child submitted to us can contact the email
above. We will update this page and its date when features or data practices change,
and provide additional notice or obtain consent where required.
