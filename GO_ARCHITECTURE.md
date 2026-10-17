# Hermod Go 版架构文档（对照 Rust 开发文档的里程碑阶段）

> 目的：给**不懂 Go** 的读者讲清楚 `hermod-go`（Go 版 hermod）的架构，重点回答两个问题——
> **server 是怎么让两台设备互信的**、**文件是怎么传的**。
>
> 结构：本文档按 `DEVELOPMENT.md` 的**里程碑 M0–M6** 逐阶段对照组织。每写一个阶段，先讲 Go 版在
> 对应阶段做了什么、再讲它与 Rust 版的关系，方便你拿两份文档并排阅读。
>
> 依据：`hermod-go/` v1.2.0 源码与 `docs/protocol.md`（Go 版协议规范），2026-10-11 核对。
> 本文档只描述 Go 版**实际实现**，不涉及 Rust 版的设计目标（那是 DEVELOPMENT.md 的事）。

---

## 0. 一分钟总览：Go 版是什么、有哪三件套

一句话：**server 只负责让两台不知道对方地址的设备"碰头"，两台设备碰头后用完全点对点的加密
通道直接传文件，文件永远不经过 server。**

Go 版由三个部分组成：

| 部分 | 命令 | 角色 |
| --- | --- | --- |
| **Server（信令服务器）** | `hermod serve` | 一台公网可达的机器，开一个 WebSocket 中继 + 一个 UDP 反射端口。只转交握手用的密文小块（blob），**永远看不到文件、也算不出会话密钥**。 |
| **发送端** | `hermod tx` | 生成传输码 → 在 server 上开频道 → 与接收端完成密码学握手 → 打洞 → 用 QUIC 把文件推给对方。 |
| **接收端** | `hermod rx` | 用传输码加入频道 → 与发送端完成密码学握手 → 打洞 → 收文件、校验、落盘。 |

整个流程分成 6 个阶段，正好和 DEVELOPMENT.md 的里程碑一一对应：

```
阶段1  信任 server          tx/rx 各自先 pin 住 server 的证书（hermod trust）
阶段2  会合 + 认证          tx 开频道、rx 加入；双方凭传输码经 server 中继跑密码学握手
阶段3  直连                 双方交换各自的地址清单 → UDP 打洞 → 建立点对点 QUIC
阶段4  文件传输             4 条 QUIC 流：元数据 → 载荷 → 哈希 → 确认；校验通过才改名落盘
阶段5  性能                 QUIC 窗口、批量、限速，尽量跑满 10 Gbit
阶段6  打包发布              静态二进制、三平台、文档
```

下面按里程碑逐段展开。

---

## M1 —— 工程骨架与核心（对应 DEVELOPMENT.md 第 8 节 M1）

Rust 版的 M1 要搭骨架、实现 CPace / AEAD / 词表 / 配置等"地基"。这些"地基"在 Go 版里**已经存在
且可测**，Rust 版正是以它们为参照（提取测试向量）。Go 版对应的模块是：

| 概念 | Go 文件 | 说明（不需要懂 Go 也能看懂） |
| --- | --- | --- |
| CPace 口令认证 | `internal/crypto/crypto.go` | P-256 曲线，RFC 9380 hash-to-curve（SSWU），把"传输码的口令"哈希成曲线上的点，双方据此做密钥交换 |
| 对称加密 | `internal/crypto/crypto.go` | AES-256-GCM，用于加密端点清单，频道号当"附加认证数据"（AAD） |
| 后量子 / 经典混合 | `internal/crypto/crypto.go` | X25519 ECDH + ML-KEM-768，与 CPace 合成一把混合密钥 |
| 传输码与词表 | `internal/crypto/crypto.go` | EFF 短词表（1296 个词），传输码格式 `<频道号>-word-word-word` |
| SAS / identicon | `internal/crypto/crypto.go` | 可选的带外校验：6 个词 + 对称图案 |
| 元数据、路径安全 | `pkg/transfer/transfer.go` | 元数据 JSON、防路径穿越、目标重名去重、边传边算哈希 |
| 配置与证书 | `internal/config/config.go` | `~/.config/hermod/config.yaml`、TLS、server 证书与指纹 |

几个必须记住的"地基"事实：

- **传输码 = 频道号 + 口令**。格式 `47832-apple-banana-cherry`：开头的数字是 16 位随机频道号
  （0–65535），后面的词是口令。默认 3 个词约 31.9 bit 熵；`tx --words 5` 可提到约 52 bit。
  这个码有双重身份：既在 server 上"开频道"，又作为 CPace 的共享口令。
- **CPace 是"口令认证密钥交换"**：只有双方都知道同一个口令，才能推出同一个密钥；口令本身
  **从不上网**，server 只中继密文，永远算不出这个密钥。
- 词表是**固定公开**的（EFF 短词表 1），所以熵全部来自随机选择，不靠保密。

> Rust 版对应关系：M1.2（CPace 向量）、M1.3（AEAD 与词表逐字节一致）、M1.5（配置双向兼容）
> 的验收标准，正是从 Go 版的这些模块里提取出来的。

---

## M2 —— 会合：server 如何让两台设备互信（第 1、2 层信任）

这是"server 做互信"的核心阶段。Go 版里它由 `internal/network/signaling.go`（客户端）和
`internal/server/`（服务端）实现。**互信分三层，M2 完成前两层，M3 完成第三层。**

### 2.1 第一层信任：先信任 server（TOFU + 证书 pinning）

server 是**自签证书**，没有公共 CA 帮它背书。所以客户端需要"自己记住"server 长什么样：

1. 运行一次 `hermod trust wss://你的server:4376`。
2. 客户端请求 server 的 HTTPS `/cert` 接口，抓回 server 的 TLS 证书。
3. 计算证书**公钥的指纹**（对证书里的公钥做 SHA-256，得 64 个 hex 字符），存进本地配置
   `~/.config/hermod/config.yaml` 的 `trusted_servers` 表，键是标准化后的地址 `wss://host:4376`。
4. 之后每次 `tx` / `rx` 连 server，TLS 握手时都要**校验 server 证书的公钥指纹与存的一致**
   （用常量时间比较，防计时侧信道）。不一致就拒绝连接，提示重新 `trust`。

两个细节值得注意：

- **存的是"公钥指纹"（SPKI），不是整张证书的指纹**。这样 server 换证书但沿用同一把私钥时，
  指纹不变，客户端**不用重新 trust**。
- 首次抓证书时是"先信任后验证"（TOFU）。所以 `trust` 最好在可信网络里跑；`--fingerprint` 参数
  可以让你在已知指纹的前提下严格校验后再存，防 TOFU 劫持。

> 这一层的意义：**两台设备先分别信任同一个 server**，才敢在它上面"对暗号"。自建 server 不需要
> 买 CA 证书，这就是 `trust` 命令存在的全部理由。

### 2.2 第二层信任：两台设备之间的认证（传输码 + CPace + 混合 KEM）

两台设备互不相识，唯一的共同点是**同一个传输码**。server 在这里只做一个动作：**转发密文**。
完整时序（消息都是 JSON，装在一个信封里：`{"type":..., "channel_id":..., "payload":...}`）：

```
发送端 tx                     server                       接收端 rx
   │ ① allocate(频道号) ────► │
   │ ◄── ok(观测到的IP) ───── │
   │ ②                        │ ◄── join(同一频道号) ─────  rx 输入传输码
   │ ◄── ready ────────────── │
   │ ③                        │ ──► ok(观测到的IP) ────────►
   │ ④ blob1(97B) ──────────► │ ──► blob1 ────────────────►
   │ ◄─ blob2(1281B) ──────── │ ◄─ blob2 ──────────────────
   │ ⑤ blob3(KEM密文+加密清单)►│ ──► blob3 ────────────────►
   │ ◄─ blob4(加密清单) ───── │ ◄─ blob4 ──────────────────
   │ ⑥ 信令到此结束，双方关闭 WebSocket
```

各 blob 里装什么（全部是二进制，server 只能转发、看不懂）：

| blob | 方向 | 内容 |
| --- | --- | --- |
| blob1 | tx → rx | 65 字节 CPace 公开点 + 32 字节 X25519 公钥（共 97 字节） |
| blob2 | rx → tx | 65 字节 CPace 公开点 + 32 字节 X25519 公钥 + 1184 字节 ML-KEM-768 封装公钥（共 1281 字节） |
| blob3 | tx → rx | 1088 字节 ML-KEM 密文 + 用混合密钥 AES-256-GCM 加密的"端点清单" |
| blob4 | rx → tx | 用混合密钥 AES-256-GCM 加密的"端点清单" |

双方收到对方的 CPace 点后各自算出**同一个**共享密钥 K（口令对才一致），再叠加两个 pillar，
合成一把**混合密钥**（这是 Go 版的安全核心）：

```
kClassical = CPace(双方口令)              // 32 字节，来自传输码，经典安全
ssX25519   = X25519 椭圆曲线密钥交换       // 32 字节，经典安全（纵深防御）
ssMLKEM    = ML-KEM-768 后量子封装/解封装   // 32 字节，抗量子

hybridKey  = SHA-256(kClassical ‖ ssX25519 ‖ ssMLKEM)   // 32 字节
```

这把混合密钥的用途：

- **加密"端点清单"（endpoint bundle）**：每个设备把自己要公布的信息用 `hybridKey` + 频道号
  （作 AAD）做 AES-256-GCM 加密，经 server 转发给对方。清单里装的是：
  - 自己 UDP socket 实际绑定的端口；
  - 本地枚举出的所有 IPv4 / IPv6 候选地址；
  - server 观测 / UDP 反射发现的公网地址；
  - **自己一次性 TLS 证书的公钥指纹**（留给 M3 用）；
  - 是否要求 SAS 带外校验（任一方要求则双方都做）。
- 因为只有持有正确传输码的双方能解出 `hybridKey`，**能解出清单 = 你是持码者**。server 只会转发
  密文，解不开也伪造不了。这把"两台设备的互信"和"传输码正确"绑死。

信令交换一结束，**WebSocket 立刻关闭**，释放 server 资源。此后 server 不再参与。

### 2.3 server 侧的保护措施（公共中继能不能活，全看这些）

Go 版 server 内置了一套配额与限速（`hermod serve` 的参数可调）：

| 项目 | 默认值 | 作用 |
| --- | --- | --- |
| 频道 TTL / 空闲超时 | 600 s（`--ttl`） | 频道与连接超时自动作废，防占坑 |
| 每频道 blob 上限 | 10 | 防中继被握手垃圾塞爆，超限踢连接 |
| 每频道 CPace 失败上限 | 3 | 连续协议违规即**整频道作废**，同时防暴力猜码 |
| 每 IP 前缀频道上限 | 100 | IPv4 /32、IPv6 /64 计数 |
| 令牌桶限速 | 5 次/秒，突发 15 | `/cert`、WebSocket 升级、join 三处独立限速 |
| WebSocket 消息大小上限 | 64 KiB | 防恶意 server / 对端撑爆内存 |

隐私细节：限速桶的键是 `HMAC-SHA256(每日盐, IP前缀)`，**不存明文 IP**，盐每天轮换。
channel ID 是 16 位整数，高频部署可能撞号，server 对已占用频道重复 `allocate` 一律拒绝。

> Rust 版对应关系：M2.1（线格式与 Go server 报文一致）、M2.2（pinning）、M2.3（经真实 Go
> server 得同一个 K 且 server 算不出）、M2.6（Rust server 可延后，v1 直接用 Go server）。这些
> 验收之所以成立，是因为 Go 版的信令契约原样保留（DEVELOPMENT.md 决策 D1）。

---

## M3 —— 直连：从"经 server 互信"到"设备直连互信"（第 3 层信任）

M2 结束时双方已互信、也知道对方地址了。M3 要做的是**抛开 server，建立一条点对点的加密通道**。
Go 版这里分三步：找自己的公网 UDP 地址 → UDP 打洞 → 建立 QUIC 双向 TLS 连接。

### 3.1 UDP 反射：先搞清"自己对外是什么 UDP 地址"

server 观测到的是 WebSocket（TCP）连接的来源地址，**TCP 端口对 UDP 没用**。尤其在有
**CGNAT（运营商级 NAT，会把 UDP 端口改掉）** 的环境里，必须让 server 亲眼看到自己的 UDP 包从
哪个地址出去。server 在**和 HTTPS 同一个端口**上还开了一个 UDP 反射端口：

```
设备                           server（UDP 反射端口，同 4376）
  │── [0x10] 索要 cookie（1B）─►
  │◄─ [0x10][HMAC cookie 8B] ─── 9B
  │── [0x10][cookie] 回显（9B）─►   cookie 绑来源 IP，只有真能收包的人才拿得到
  │◄─ [族][IP][端口]（7–19B）───── 返回外部 UDP 地址
```

两阶段 HMAC cookie 是为了**防反射放大攻击**：外部地址只发给"证明自己能在这个地址收到包"的对端。
发现失败就退回"server 观测 IP + 本地端口"。

### 3.2 UDP 打洞：让两边都能互相"敲门"

双方都拿到对方在清单里公布的候选地址后，**同时向对方全部候选地址发探测包**：

```
探测包 = [0x01][nonce 前 7 字节]       ack 包 = [0x01][nonce 中间 7 字节]
nonce  = SHA-256(hybridKey ‖ "hermod-holepunch-v1")   // 每会话唯一，64 位熵
```

- 双方每 200 ms 向所有候选地址打一轮探测；收到探测的一方回 ack。
- 先打 **IPv6（5 秒超时）**，失败再打 **IPv4（总预算 10 秒）**；`-4` / `-6` 可强制单族。
- **第一个拿到正确 ack 的地址赢**，用它建 QUIC。
- 打洞成功后**继续打**直到 QUIC 建起来（防止 NAT 映射在空档期过期）。

为什么需要打洞：双方若都在 NAT / 有状态防火墙后面，彼此的入站包默认进不来。各自发出去的出站
探测包会在自己的 NAT / 防火墙上"开一个洞"，对方的包顺着这个洞就能进来。纯 IPv6 无 NAT 的场景
其实不需要打洞，直接连即可。

### 3.3 QUIC：点对点加密通道 + 第三层互信

打洞成功后：**发送端当 QUIC 客户端去拨号，接收端当 QUIC 服务器监听接受**。

```
发送端 tx                               接收端 rx
   │ 打洞成功，拿到对端地址                    │
   │── QUIC 拨号（TLS 1.3）────────────────►│
   │◄───────────────────────────────────────│ QUIC 接受
   │  从此数据完全点对点，不再经过 server        │
```

通道安全（**第三层互信**，即"设备与设备之间的直接互信"）：

- 每台设备现场生成一张 **2 小时有效的一次性自签 ECDSA P-256 证书**，用完即弃。
- 双方在 QUIC 握手时**互相校验对端证书的公钥指纹**，要求它与 M2 加密清单里公布的一致
  （`VerifyPeerCertificate` 回调 + 常量时间比较）。接收端还强制要求客户端证书（双向 TLS）。
- 因为清单只有持码者能解密，所以"对端证书指纹可信"最终追溯到"传输码正确"——**信任链闭环**。
- ALPN = `hermod-p2p`；空闲 30 秒超时、5 秒保活；TLS 1.3。
- **可选 SAS 带外校验（`--verify`）**：把 TLS 会话导出 32 字节密钥，生成 6 个词的 SAS + 对称
  图案，双方各自显示，人工通过电话 / IM 比对，再各按一个字节确认。防的是"中间人劫持 QUIC 握手"
  这种极端情况。默认关闭。

> ⚠️ **这是 Go 版与 Rust 版最大的分叉点**（DEVELOPMENT.md 决策 D2/D3/D7）：
> - Go 版数据面 = **QUIC + 打洞 + 双向 TLS + 临时证书**，支持 IPv4/IPv6 双栈。
> - Rust 版 v1 数据面被**彻底重设计** = 进程内用户态 **WireGuard 隧道 + 隧道内 HTTP over TCP**，
>   且**只走 IPv6、不打洞**。
> 两版**不能互传文件**（协议完全不同），但都能用同一个 Go server 完成会合。Rust 版 M3 验收里
> 的"同时发起握手、端点管理、漫游"，在 Go 版里对应的是"打洞 + QUIC 拨号/监听"。

---

## M4 —— 传输与 CLI：文件是怎么传的

这是第二个核心问题。Go 版用**一条 QUIC 连接上的几条流（stream）**传文件，全部由 `tx.go` /
`rx.go` / `pkg/transfer` 实现。

### 4.1 一次文件传输的完整数据流

QUIC 连接建好后，发送端按顺序开流。以下按**默认无 `--verify`** 的情况列（开了 `--verify` 则
最前面多一条 SAS 协调流，其余整体后移）：

| 流 | 打开方 | 内容 |
| --- | --- | --- |
| 元数据流 | 发送端 | `4 字节长度前缀 + JSON`：`{"kind":"file","name":"报告.pdf","size":204800,"sha256":""}`。`kind` 是 `file` / `text` / `stream`；`sha256` 恒为空（真正的哈希在传输中并行算）。 |
| 载荷流 | 发送端 | 文件 / 文本 / 标准输入的**原始字节**，无任何分帧。发送端边读边算 SHA-256；接收端边收边算 SHA-256，同时写临时文件。 |
| 尾部哈希流 | 发送端 | `4 字节长度前缀 + 64 位 hex 的 SHA-256`。发送端把上面并行算出的哈希在此流发出。 |
| 完成确认流 | 接收端 | 接收端存盘 + 校验全过后，**开一条流立即关闭**；发送端等收到这条流才关 QUIC 连接，防止连接过早拆掉。 |

文件字节流的落地流程（接收端）：

```
收到元数据 ──► 边收边写 .hermod_tmp 临时文件（O_EXCL 创建、0600 权限）
             边收边算 SHA-256、边数字节
收到尾部哈希 ─► ① 收到的字节数 == 元数据里的 size？
             ② 算出的 SHA-256 == 尾部哈希？
             两步全过 ──► 原子改名 os.Rename 成最终文件名
             任何一步不过 ──► 删除临时文件、报错、非零退出
```

落盘的安全细节：

- **先写临时文件、验证通过才改名**，绝不出现"半截文件假装成功"。
- 文件名先用 `filepath.Base` 剥掉所有目录成分，**防路径穿越**（对端改名 `../x` 想写别处，被剥掉）。
- 目标已存在时自动加去重后缀 `report(1).pdf`、`report(2).pdf` …（最多 9999 个）。
- `O_EXCL` 防止静默覆盖上次崩溃留下的旧临时文件（有则先删再建）。

三种载荷的处理：`file`（存盘，默认当前目录）、`text`（直接打印到 stdout）、`stream`
（标准输入管道直通 stdout 或存盘）。文本/流默认直出 stdout，进度条画在 stderr，互不干扰。

### 4.2 取消 / 中断

任一方按 Ctrl+C（SIGINT/SIGTERM）：

```
取消方：以 QUIC 应用错误码 1 关闭连接，消息 "cancelled:sender" 或 "cancelled:receiver"
对端： 立刻感知（被阻塞的读写被解阻塞），识别出是谁取消的，打印 "Transfer cancelled by X"
接收端：删除 .hermod_tmp 临时文件，绝不留半截文件
双方：  非零退出码
```

取消在 SAS 等待输入时也生效；发送端在载荷流中途看到对方关闭即中止并清理。

### 4.3 CLI 命令一览

| 命令 | 作用 | 主要参数 |
| --- | --- | --- |
| `tx [INPUT]` | 发送文件/文本/标准输入 | `-s/--server`、`-w/--words`（默认 3）、`-v/--verify`、`-l/--listen` |
| `rx [CODE]` | 用传输码接收 | `-d/--destination`、`-s/--server`、`-v/--verify`、`-l/--listen` |
| `serve` | 起信令服务器 | `-l/--listen`（默认 :4376）、`-T/--ttl`（600）、`--rate-limit/--rate-burst`、`--max-*` 系列 |
| `trust <server>` | 抓取并 pin server 证书 | `--fingerprint`（预知指纹时严格校验） |

> Rust 版对应关系：M4.1–M4.5 在 Go 版里由上面这套 QUIC 流与落盘逻辑承担。差异点见附录 E：
> **Rust 版删掉了 `--verify`（SAS）**（决策 D12），新增 `--wg-mtu`；Rust 版传输语义是"隧道内
> HTTP GET 拉文件 + POST /done 回裁决"，与 Go 版的"四条 QUIC 流"完全不同——所以两版不能互传文件。

---

## M5 —— 性能：Go 版做了什么

Go 版面向"尽量跑满 10 Gbit/s"做了这些（`internal/network/network.go`）：

| 项 | 值 | 说明 |
| --- | --- | --- |
| UDP 读缓冲 | 1 MiB / 次读 | 吸收突发，减少逐包拷贝 |
| socket 收发缓冲 | 各 2 MiB | 提升高带宽链路吞吐 |
| QUIC 流控窗口 | 初始 64 MiB → 最大 256 MiB | 接收端信用额度，按需才占内存 |
| mux 队列 | 16384 槽 | 吸收约 20 ms 的突发不丢包 |
| 哈希 | 与传输并行（TeeReader） | 边传边算，不预读文件 |

> 背景：**Go 版的 QUIC 数据面在丢包链路上跑不满**——这正是 Rust 重写（DEVELOPMENT.md M0）的
> 动机之一。Rust 版换用用户态 WireGuard + TCP 后，M5 的目标是"丢包路径至少为 Go 基线 2 倍"。
> 所以 M5 对 Go 版而言是"现状基线"，对 Rust 版是"要超越的对象"。

---

## M6 —— 打包与发布

Go 版发布形态：单一静态二进制、Linux / macOS / Windows 三平台、自带 `README.md` /
`CHANGELOG.md` / 协议文档 / e2e 测试（`e2e/`）。server 端配置全在 `~/.config/hermod/config.yaml`，
证书到期前 14 天自动续期（沿用同一把私钥，客户端无需重新 trust）。

> Rust 版对应关系：M6.1 静态 musl Linux + macOS 双架构 + Windows；M6.2 文档要写明 IPv6 范围限制
> 与"无带外校验"的安全姿态。

---

## 7. 专题小结：三层信任链怎么串起来的

两台设备从互不相识到放心传文件，靠的是**一条完整信任链**，任何一环断了都中止：

| 层 | 互信的对象 | 机制 | 断链后果 |
| --- | --- | --- | --- |
| 第 1 层 | 设备 ↔ server | TOFU + 证书公钥 pinning（`hermod trust`） | 连不上 / 拒绝连接 |
| 第 2 层 | 设备 ↔ 设备（凭传输码） | 传输码 + CPace + X25519 + ML-KEM → 混合密钥；AES-GCM 加密端点清单 | 口令错 → 解不开清单，中止 |
| 第 3 层 | 设备 ↔ 设备（直连） | QUIC 双向 TLS + 临时证书 SPKI 互相 pinning | 指纹不符 → 握手失败 |
| （可选） | 人类确认 | SAS 6 词 + 图案，带外人工比对 | 任一方拒绝 → 中止 |

要点：**第 3 层（设备直连）的信任最终追溯到第 2 层（传输码）**，因为证书指纹是放在只有持码者
能解密的清单里传的；**第 2 层的信任又建立在第 1 层（双方都信任同一个 server）** 上。server 全程
只看到密文，既看不了文件也算不出密钥。

⚠️ 诚实的安全姿态（README 明示）：**默认没有带外校验**。谁猜中传输码，谁就能静默冒充对端。
补偿手段：传输码熵（敏感场景 `--words 5`）+ server 的 CPace 失败上限（3 次）+ 码本身强度。
这是 Go 版的设计取舍，不是漏洞。

---

## 8. 专题小结：一份文件从发送到落盘的完整旅程

```
① tx 生成传输码 → 连 server → allocate 频道 → 把码显示给用户
② rx 输入码 → 连 server → join 同一频道
③ 双方经 server 中继 4 个 blob，完成 CPace + X25519 + ML-KEM 握手
   → 各自得到混合密钥 → 交换加密的端点清单 → 关闭 WebSocket
④ 双方找自己的外部 UDP 地址（反射）→ 互相打洞 → 建立 QUIC（双向 TLS + pinning）
⑤ tx 开元数据流 → 开载荷流，边读边算 SHA-256 边推
⑥ rx 边收边写 .hermod_tmp、边算 SHA-256、边数字节
⑦ tx 发尾部哈希流；rx 对"字节数 + 哈希"双重校验
⑧ 通过 → 原子改名落盘；不过 → 删临时文件报错
⑨ rx 开完成确认流；tx 收到后关闭 QUIC；双方退出 0
```

整个过程：**只有 ③ 经过 server（且只是握手密文），⑤⑥⑦⑧ 全部点对点直连**。

---

## 9. 与 Rust 版差异对照表（合并自 DEVELOPMENT.md 附录 E，并补充）

| 维度 | Go 版（本文档） | Rust 版 v1（DEVELOPMENT.md） |
| --- | --- | --- |
| 数据面 | QUIC（quic-go） | 进程内用户态 WireGuard + 隧道内 HTTP over TCP |
| 传输语义 | 4 条 QUIC 流 | HTTP/1.1：GET 拉文件 + POST /done 回裁决 |
| 网络族 | IPv4 + IPv6 双栈 | 仅 IPv6（决策 D7） |
| NAT | UDP 打洞 + 反射 | v1 不打洞（握手包兼作 pinhole 探针） |
| 对端互信 | QUIC 双向 TLS + 临时证书 SPKI pinning | WireGuard 握手，PSK 由 CPace 密钥派生 |
| 后量子 | 有（ML-KEM-768） | v1 无，保留恢复接缝（决策 D6） |
| 带外校验 | 有（`--verify`，SAS） | 删除（决策 D12） |
| 断点续传 | 无 | 无（决策 D9） |
| 与对方互传文件 | — | 不支持（协议完全不同，报明确错误） |
| 会合契约 | — | 保留（决策 D1），可用 Go server |
| 服务器 | Go 版 `serve` | Rust server 延后，v1 用 Go server（决策 D13） |

---

## 附录 A — 关键常量表

| 常量 | 取值 |
| --- | --- |
| server 端口 | 4376（TCP/WSS 与 UDP 反射同一端口） |
| 传输码格式 | `<uint16>-word-word-word`（词来自 1296 词 EFF 短词表） |
| 默认词数 / 熵 | 3 词 ≈ 31.9 bit；`--words 5` ≈ 52 bit |
| CPace | P-256，RFC 9380 SSWU hash-to-curve |
| 混合密钥 | SHA-256(CPace ‖ X25519 ‖ ML-KEM-768) |
| 清单加密 | AES-256-GCM，AAD = 频道号（2 字节大端） |
| blob 大小 | blob1 97B / blob2 1281B / blob3 KEM 1088B+清单 / blob4 清单 |
| 打洞 | 探测 200ms 一轮；IPv6 5s → IPv4 10s；nonce = SHA-256(hybridKey‖"hermod-holepunch-v1") |
| QUIC | TLS 1.3、ALPN `hermod-p2p`、空闲 30s、保活 5s、临时 ECDSA P-256 证书 2h |
| 流控窗口 | 初始 64 MiB → 最大 256 MiB |
| server 配额 | TTL 600s、每频道 blob 10、CPace 失败 3、每 IP 频道 100、限速 5/s 突发 15 |
| 临时文件 | `目标名.hermod_tmp`，O_EXCL、0600 |

## 附录 B — 代码文件索引（概念 → Go 文件）

| 想查什么 | 看这里 |
| --- | --- |
| 信令客户端（allocate/join/blob/ready/ok/error、证书抓取与 pinning） | `internal/network/signaling.go` |
| CPace / 混合 KEM / AES-GCM / SAS / 词表 / 传输码 | `internal/crypto/crypto.go` |
| 握手 blob 编码 / 端点清单结构 / 候选地址 | `internal/network/handshake.go` |
| UDP 复用 / 打洞 / QUIC dial & listen / SPKI pinning | `internal/network/network.go` |
| server（频道、blob 中继、限速、配额） | `internal/server/server.go`、`store.go`、`ratelimit.go` |
| UDP 反射（CGNAT 外部地址发现） | `internal/server/udp_reflect.go` |
| 发送端主流程（4 条流、SAS、取消） | `internal/cli/tx.go` |
| 接收端主流程（接收、校验、落盘改名） | `internal/cli/rx.go` |
| 元数据 / 路径安全 / 去重 / 哈希流 | `pkg/transfer/transfer.go` |
| 配置 / TLS / server 证书与指纹 / trust 存储 | `internal/config/config.go` |
| trust 命令 / tx/rx 强制校验已 trust | `internal/cli/trust.go`、`server_trust.go` |
| server 命令与证书自动续期 | `internal/cli/serve.go` |
| 完整协议规范（Go 版） | `hermod-go/docs/protocol.md` |
| Rust 版开发文档（本文档的对照对象） | `DEVELOPMENT.md` |
