# Hermod-rs 开发文档 — WireGuard + HTTP 直连传输

| 项目 | 内容 |
| --- | --- |
| 状态 | 设计定稿（v3，crate 布局已按 D17 修订为双 crate），尚未开始编码 |
| 目标 | 用 Rust 重写 hermod，**只保留 server 那套认证与会合**，数据面完全重新设计为「用户态 WireGuard 隧道 + 隧道内 HTTP over TCP」 |
| 参考实现 | `hermod-rs/hermod-go` v1.2.0（只读；仅作为 server 行为参考与测试向量来源，**不是移植模板**） |
| 遵循规则 | `.opencode/RULES.md`（代码、注释、commit 用英文；本文档用中文） |
| 日期 | 2026-10-11 |

## 阅读指南

本文档不是移植方案。Go 实现只被参考两件事：**signaling server 的认证与会合行为**，以及**测试向量**。两端之间字节怎么流动，全部重新设计，设计依据是已确认的架构方向：

> 用传输码在 server 上会合 → 双方交换 IPv6 地址 → 建立 WireGuard 隧道 → 隧道内用 HTTP 传文件。

---

## 0. 决策清单

| # | 决策 | 理由 |
| --- | --- | --- |
| D1 | **保留 Go 版的 server 认证与会合契约**：传输码、channel allocate/join、经 server 中继的 CPace 密文 blob、server 证书 pinning（`trust`）、限速与配额。 | 这套东西工作正常，是用户直接接触的部分；保留意味着现成的 Go server 可以原样服务 Rust 客户端。 |
| D2 | **丢弃**：QUIC、数据面 TLS、peer 间临时证书与 ALPN、四条 QUIC 流、打洞协议、UDP reflector、ML-KEM 与 X25519 两个 pillar。 | WireGuard 承载载荷之后这些都不需要；留着就是移植而非重设计。 |
| D3 | **数据面 = 进程内用户态 WireGuard + 进程内用户态 TCP，永不需要特权。** | 与已验证可行的 `wireproxy` 形态一致（wireguard-go + 用户态 netstack），保持「装上就能跑」的体验。 |
| D4 | **对端地址来自 server 观测 + 双方本地枚举，不靠打洞。** 每方在 PAKE 加密的 bundle 里公布自己绑定的 UDP 端口与 IPv6 地址。 | IPv6 没有 NAT，交换地址即可。 |
| D5 | **每会话临时 X25519 密钥对，PSK 由 PAKE 密钥派生。** | 传输码仍然完成对端认证；PSK 把隧道绑定到该认证上，同时为将来恢复后量子 pillar 留出接口。 |
| D6 | **v1 不做后量子。** CPace（P-256）是唯一的密钥协商 pillar。 | 已接受的回归；PSK 派生处即是恢复路径。 |
| D7 | **v1 只做 IPv6。** IPv4 传输、IPv4 打洞、中继全部出范围；Go 版打洞算法作为**后续回退路径**加回，且不改变 v1 任何接口。 | 本次确认的范围。CPE 有状态防火墙靠「双方同时对发握手」解决，剩下打不通的场景留给打洞阶段。 |
| D8 | **传输层不做第二层加密。** WireGuard 已在两端之间提供端到端的机密性与完整性，传输层只做完整性校验（SHA-256）。 | 隧道端点就是两个 peer，中间没有中继；再加一层只烧 CPU。**前提**：一旦未来数据面出现「隧道在中间终止」（例如加 TCP 中继），应用层加密立刻变为必需。 |
| D9 | **单一顺序流传输：不做断点续传、不写状态文件、不留块位图。** 中断即重来。 | 本次确认的范围。去掉这一整类「续传状态出错」的 bug，接收端保持无状态。 |
| D10 | **只做单条隧道：不做多 lane、不做并行分片传输。** | 本次确认的范围。单隧道 = 单数据线程，吞吐上限 1–3 Gbit/s 量级，对 1 GbE 及以下完全够用。 |
| D11 | **数据面用 HTTP/1.1**：tx 起 HTTP server，rx 用**一次 GET** 拉完整个文件，再用 `POST /done` 回裁决。 | 复用成熟协议：Content-Length、文件名、状态码、chunked 都是现成的，且可以用 curl 在隧道里手工调试。 |
| D12 | **删除 SAS 与 `--verify/-v` 参数**（明确打破与 Go 版的参数一致性）；**保留 CPace 与传输码词表**。 | 本次确认的范围。注意 CPace 不是 SAS：CPace 是传输码的密码学认证，必须留；SAS 只是带外人工核对。 |
| D13 | **Rust 版 server 可以延后**，v1 先用 Go server。 | signaling 协议兼容，客户端工作才是解锁用户的关键。 |
| D14 | **覆盖网固定地址**：tx = 10.66.66.1/32，rx = 10.66.66.2/32。 | 每条会话拥有自己的进程内接口，不存在冲突，也不在宿主机装任何东西。 |
| D15 | **配置文件与 Go 版双向兼容**（能读 Go 写的 schema，也按同样 key 写回）。 | 两个工具可以共存于同一台机器。 |
| D16 | **传输码默认 3 词**，敏感场景用 `--words 5` 提高熵。 | 保持与 Go 版一致；删除 SAS 后码的强度成为唯一防线，需要给用户一个加强手段（3 词约 31 bit，5 词约 52 bit）。 |
| D17 | **单一库 crate（`hermod-core`）+ 薄 CLI**：signaling 与 tunnel 不再单独成 crate，全部作为 core 的模块；`hermod-cli` 只做参数解析、进度渲染与信号处理，直接调用 core 的公共 API。 | 用户要求：后续要包装其他客户端（GUI、脚本等），一个库 crate 提供完整公共 API，避免跨 crate 依赖；core 内部仍按 signal/tunnel 分模块，职责边界不变。 |

---

## 1. 总体架构

### 1.1 一句话

保留 Go 版的 **server 认证与会合**，数据面换成 **进程内用户态 WireGuard 单隧道 + 隧道内 HTTP/1.1 over TCP**，只走 **IPv6 直连**，零特权、无续传、无并行、无带外校验。

### 1.2 分层

    应用层   HTTP/1.1：GET 拉文件 + POST /done 回裁决
    ───────────────────────────────────────────────
    传输层   TCP（smoltcp，进程内）
    ───────────────────────────────────────────────
    隧道层   WireGuard（boringtun::noise::Tunn，进程内，单会话）
    ───────────────────────────────────────────────
    承载层   UDP socket（进程独占，单条）
    ───────────────────────────────────────────────
    网络层   IPv6 underlay（全局单播 / ULA / 链路本地）

宿主机除了一个普通用户账号和一条 UDP socket 之外，什么都不需要。

### 1.3 三个组成部分

1. **Server（保留，可用 Go 版）**：WebSocket 中继。用传输码认证并分配 channel，永远看不到载荷，也永远算不出会话密钥。自签证书由客户端首次使用时 pinning（`hermod trust`）。
2. **会合（保留并简化）**：双方加入同一 channel，通过中继的 blob 跑 CPace，得到只有两端知道的密钥 K；随后在 K 之下交换一份加密 bundle：WG 公钥、自己绑定的 UDP 端口、HTTP 端口、IPv6 候选地址。
3. **数据面（全新）**：双方各自建立进程内 WireGuard 会话（临时密钥 + 由 K 派生的 PSK），把对端端点指向对方公布的地址并完成 Noise 握手 —— 这个握手包同时充当防火墙 pinhole 探针。隧道建立后，tx 在隧道内起 HTTP server，rx 发一次 GET 把文件拉下来，边收边写临时文件边算摘要，最后通过 POST 交换裁决，摘要一致才原子改名。

### 1.4 必须一起接受的取舍

- **只支持 IPv6**：任一端没有可用 IPv6，或对端网络整体过滤入站，v1 传不了（5 秒内明确报错，不挂起）。
- **单线程吞吐上限约 1–3 Gbit/s**：单隧道单数据线程，10 GbE 级链路吃不满；巨帧链路可用 `--wg-mtu 8900` 把 WireGuard 每包开销从约 5% 压到 1% 以下。
- **没有带外校验**：SAS 已删除。猜中传输码的攻击者可以静默冒充对端；剩下的防线是码熵 + server 限速 + CPace。敏感传输请用 `--words 5`。
- **没有断点续传**：中断即重传整个文件。
- **传输层没有第二层加密**：只有 WireGuard 一层（这是设计，不是遗漏）。

---

## 2. 范围：保留、重新设计、丢弃

### 2.1 从 hermod-go 保留（server 认证与会合）

| 保留项 | 原因 |
| --- | --- |
| 传输码（EFF 短词表，默认 3 词）与相同格式 | 用户直接接触的契约，且与 Go server 的 channel 模型共享 |
| Channel 生命周期：allocate、join、ready、blob、ok、error（WebSocket） | 会合契约；正因为保留它，Rust 客户端才能使用 Go server |
| 经中继的 CPace PAKE（P-256），server 永远读不到也伪造不了 | 产品的安全核心；没有它传输码就只剩装饰作用 |
| Server 证书 pinning（首次使用即信任）+ 显式 `trust` 命令 | 自建部署不需要公共 CA |
| Server 侧配额与限速：channel TTL、每 channel blob 上限、CPace 失败上限、每 IP channel 上限、令牌桶 | 实现成本低，但决定公共中继能否存活 |
| 配置文件位置与 schema（`~/.config/hermod/config.yaml`）、`HERMOD_LISTEN` | 现有用户无需迁移 |
| CLI 命令名：tx、rx、serve、trust（参数见附录 E） | 用户输入的命令不变 |

### 2.2 重新设计（数据面）

| 组件 | 设计 |
| --- | --- |
| 对端地址 | 每方公布自己**实际绑定的 UDP 端口** + 本地枚举的 IPv6 地址，并与 server 观测到的源地址合并（仅当它是 IPv6 时作提示，见 §3.5） |
| 隧道 | 每会话一个进程内 WireGuard 会话：临时密钥、PSK 由 K 派生、MTU 1420、keepalive 25 s（§3.3） |
| 隧道内传输 | 一条 TCP 连接，跑 HTTP/1.1（§3.4） |
| 传输语义 | tx 在隧道内提供文件，rx 单次 GET 拉取；元数据走 HTTP 头；摘要通过 POST /done 交换裁决（§3.6） |
| 取消 | rx 直接关连接；tx 见 EOF/RST 即中止并清理（§3.7） |
| 完整性 | 双方各自边传边算 SHA-256，rx 的摘要在 POST /done 中提交，tx 与自己算出的比对后才允许改名（§3.6） |

### 2.3 丢弃清单及其替代

| 丢弃项 | 影响 / 替代 |
| --- | --- |
| QUIC、quic-go | 由 WireGuard + HTTP over TCP 数据面替代 |
| peer 间临时 TLS 证书、SPKI pinning、ALPN `hermod-p2p` | 对端身份改由 CPace 派生的密钥与 WireGuard 握手保证；peer 之间不再有任何证书 |
| 四条 QUIC 流（SAS、元数据、载荷、尾部哈希、ack） | 一条 HTTP 连接：GET 拉载荷，POST 回裁决 |
| 打洞协议（探针标记、IPv6/IPv4 双阶段、探针 nonce） | 直连；握手包兼任 pinhole 探针（§3.3）。算法作为后续回退路径加回（D7），v1 不含 |
| UDP reflector（opcode `0x10`、HMAC cookie、每日轮换） | 不需要：IPv6 无 NAT，没有端口改写要探测，客户端直接报告自己绑定的端口 |
| ML-KEM-768 与 X25519 ECDH pillar | 会话密钥仅由 CPace 派生，PSK 再由它派生 |
| SAS、identicon、`--verify/-v` | 全部删除（D12）。补偿手段是提高传输码熵（`--words`） |
| QUIC 流控窗口、idle timeout、keep-alive、CloseWithError 错误码 | TCP 窗口、TCP 空闲超时、连接关闭 + 退出码 |
| 接收端强制双向 TLS | 无需强制：WireGuard 握手已经认证了对端 |
| 断点续传、块位图、状态文件（Go 版本也没有，此处明确不做） | 中断即重传（D9） |
| 多 lane / 并行分片 | 单隧道单线程（D10） |
| 传输层加密 | 只保留完整性校验（D8） |

---

## 3. 详细设计

### 3.1 crate 布局（D17：双 crate）

    hermod-rs/
    ├── Cargo.toml                 # [workspace]（hermod-core + hermod-cli）
    ├── rust-toolchain.toml        # 固定工具链（MSRV 1.99.0 见 Cargo.toml）
    ├── DEVELOPMENT.md             # 本文档
    ├── docs/
    │   ├── adr/                   # 从第 0 节抽出的决策记录
    │   └── benchmarks/            # 性能与可达率实测结果
    ├── hermod-core/               # 唯一的库 crate：全部逻辑
    │   └── src/
    │       ├── lib.rs             # 公共 API 出口（其他客户端从这里接入）
    │       ├── config.rs          # 配置读写（与 Go 版 schema 双向兼容）
    │       ├── error.rs           # 统一错误类型
    │       ├── transfer.rs        # 路径安全、完整性、tx/rx 编排 API
    │       ├── http_meta.rs       # 隧道内 HTTP 头约定
    │       ├── crypto/{mod.rs,cpace.rs,kdf.rs,aead.rs,wordlist.rs}
    │       ├── signal/{mod.rs,protocol.rs,client.rs,tls.rs,server.rs,store.rs,ratelimit.rs}
    │       └── tunnel/
    │           ├── mod.rs
    │           ├── session.rs     # 会话编排（bundle → WG → HTTP）
    │           ├── addr.rs        # 本地 IPv6 枚举与候选合并
    │           ├── udp.rs         # socket 归属与批量收发
    │           ├── wg/{mod.rs,keys.rs,tunnel.rs,endpoint.rs}
    │           ├── stack/{mod.rs,device.rs,driver.rs,bridge.rs}   # smoltcp 与 tokio 桥接
    │           └── http/{mod.rs,server.rs,client.rs}             # 隧道内 HTTP
    └── hermod-cli/                # 薄可执行文件：只解析参数并调用 core 的 API
        └── src/{main.rs,cli.rs,tx.rs,rx.rs,serve.rs,trust.rs,progress.rs,tty.rs,verbosity.rs}

`hermod-core` 是唯一的库 crate，承载全部支持文件；`hermod-cli` 是薄壳，只做参数解析、进度渲染与信号处理，直接调用 core 的公共 API（D17）。其他客户端（GUI、脚本等）只需要依赖 `hermod-core`。模块职责与 §2/§3 的设计一一对应，只是不再跨 crate。

### 3.2 端到端流程

    tx                                            rx
     │ 1  WSS 连 server，pin 其证书（trust 首次完成）
     │ 2  allocate(传输码) ──► channel id + server 观测到的源地址
     │ 3                      server ◄── join(同一传输码)；tx 收到 ready
     │ 4  CPace PAKE（server 仅中继密文）──► 双方得到 K，server 算不出
     │ 5  加密 bundle 交换（AES-256-GCM，AAD = channel id）：
     │      wg 公钥 · 绑定的 UDP 端口 · HTTP 端口 · IPv6 候选列表
     │ 6  建 WG 会话：临时密钥对 + PSK = HKDF(K, salt=channel_id, info=hermod-wg-psk-v1)
     │      双方对全部候选同时对发握手 ◄── 握手包兼任 pinhole 探针，1 个 RTT
     │ 7  隧道建立：tx 在 10.66.66.1:port 起 HTTP server（URL 带一次性随机 token）
     │ 8  rx 连 10.66.66.1:<port>，发 GET /f/<token>
     │      tx 回 200 + Content-Length + Content-Disposition + X-Hermod-Kind
     │ 9  rx 边收边写临时文件边算 SHA-256；tx 边发边算 SHA-256
     │10  rx 发 POST /done {"sha256": ..., "size": N}
     │      tx 与自己算出的摘要比对 ──► 200 {"status":"ok"} 或 {"status":"mismatch"}
     │11  rx 收到 ok 才原子改名 → POST 完成 → tx 退出 0
     │      摘要不一致或长度不符：rx 删除临时文件并退出非 0，tx 同样退出非 0
     │12  关闭连接、销毁 WireGuard 会话、关闭 UDP socket、退出

整体时间预算：连接 server 10 s、等待对端到齐 600 s（server TTL 默认值）、PAKE 10 s、bundle 交换 10 s、WireGuard 握手 5 s（含两次抖动重试）、TCP 连接 10 s、载荷空闲 30 s、收尾 2 s。每个阶段都可取消，且都必须清理干净。

### 3.3 WireGuard 会话

- **密钥**：每会话一对全新的 X25519 临时密钥，从不复用、从不落盘。对端公钥来自 PAKE 加密的 bundle，因此它的可信来源是传输码，而不是传输层本身。
- **预共享密钥（PSK）**：`psk = HKDF-SHA256(ikm = K, salt = channel_id, info = hermod-wg-psk-v1)`，两端各自本地派生，**从不过网**。它混入 Noise 的 chaining key，因此拿不到传输码的一方即使偷到静态密钥也无法完成握手；同时它是将来恢复后量子 pillar 的接缝（把 K 的派生改成 CPace ‖ X25519 ‖ ML-KEM 即可，协议不变）。
- **会话创建**：`Tunn::new(static_private, peer_static_public, Some(psk), Some(25), index, None)`。UDP socket 由我们的进程持有，所以端点选择与漫游都是我们自己的代码（附录 B）。
- **地址族**：只走 IPv6。socket 绑 `[::]` 并置 `IPV6_V6ONLY`，行为确定，不会意外收到 v4-mapped 流量。
- **开路方式**：bundle 交换完成后，**双方立刻对全部候选地址同时对发握手**。这个出站包会在对端的有状态防火墙/CPE 上打开 pinhole，因此一个 RTT 同时完成「开路」与「认证」。WireGuard 协议本身处理同时发起的竞态；若实测发现 boringtun 处理不了，退路是固定角色：tx 发起、rx 响应，rx 仅在超时后再发起。
- **MTU**：隧道内 1420，TCP MSS 钳到 1380。用户态栈没有 PMTU 发现，固定且保守的 MTU 才是可预测的。1500 字节外层路径的算术是 1500 − 40（IPv6）− 8（UDP）− 32（WireGuard）= 1420；巨帧链路用 `--wg-mtu 8900` 把每包开销从约 5% 压到 1% 以下。
- **保活**：persistent keepalive 25 s，短于常见 NAT/防火墙老化时间，也短于 30 s 的载荷空闲超时，避免「活着的隧道看起来像死了」。
- **漫游**：任何一个成功解封装的报文，其源地址即成为新的对端端点。这是 WireGuard 的漫游规则，也是传输中途对端地址变化（例如 IPv6 临时地址被弃用）时连接能继续的原因。
- **覆盖网地址**：tx = 10.66.66.1/32，rx = 10.66.66.2/32。每条会话拥有自己的进程内接口，不会冲突，也不会在宿主机上安装任何路由或地址。

### 3.4 隧道内的 TCP 与 HTTP

smoltcp 与 WireGuard 跑在同一进程，由一个专用数据线程驱动。设备桥接是唯一「不常见」的部分：

    UDP socket ──recv──► Tunn::decapsulate ──内层 IP 包──► smoltcp Device::receive()
                                                                │
                                                        smoltcp Interface::poll()
                                                                │
    UDP socket ◄──send── Tunn::encapsulate ◄──内层 IP 包── Device::transmit()

- **监听方是 tx**：tx 在 10.66.66.1 上监听一个临时端口（在 bundle 中以 `http_port` 公布），rx 拨向 10.66.66.1:<http_port>。只有 tx 需要监听；rx 用临时源端口拨出。双方都会在 bundle 里公布自己准备的端口，角色决定谁真正使用。
- **TCP 参数**：每方向 8 MiB socket 缓冲，启用窗口缩放；拥塞控制用 smoltcp 的 CUBIC（该 crate 把拥塞控制建模为 Reno/Cubic 的 enum，BBR 类算法需要打补丁或向上游贡献，见风险 R2）。
- **桥接到异步**：smoltcp socket 通过有界通道封装成 `AsyncRead + AsyncWrite`，因此文件 I/O 与进度渲染永远不会阻塞数据线程。
- **HTTP/1.1 约定**：
  - 一条 TCP 连接、keep-alive：`GET /f/<token>` 与其后的 `POST /done` 复用同一条连接，隧道内只有这一个端口、这一条连接。
  - 响应头：`Content-Length`（文件与文本）或 `Transfer-Encoding: chunked`（stdin 流，长度未知）、`Content-Disposition: attachment; filename=...`、`X-Hermod-Kind: file|text|stream`。
  - **不主动使用 Range**：一次 GET 拉完整个文件。服务端若采用 `tower-http` 的 `ServeFile`，Range/206/416/ETag 会白送，仅作为手工 `curl` 调试的便利，不进入产品流程。
  - **不使用 HTTP/2**：多路复用会把所有流挤回一条 TCP，没有收益，反而增加复杂度。
  - URL 带一次性随机 token：隧道虽已认证，但 token 能防止误连与串流（例如同机多进程、异常重连）。
  - 隧道内是明文 `http://`，**不加 TLS** —— WireGuard 已经加密。这一条必须写死在代码与评审清单里，避免有人「顺手」补一层 TLS（D8）。

### 3.5 对端地址发现：server 能给什么、不能给什么

这是最容易实现错的部分，因此精确定义如下。

- server 观测到的是 **WebSocket（TCP）连接的源地址**。它给出的 IP 可用，端口无用：TCP 源端口不是 WireGuard 将要使用的 UDP 端口。Go 版 server 已经在 allocate/ok 回复里返回这个观测地址，该行为保留。
- 客户端因此上报 **自己实际绑定的 UDP 端口**（用 `getsockname` 读回，临时端口也可以），以及 **本地枚举出的全部可用 IPv6 地址**：全局单播优先，其次 ULA，最后是带 scope id 的链路本地（同链路场景）。完全不枚举 IPv4。
- 一台主机往往同时持有多个 IPv6 地址（RFC 8981 临时地址、多前缀、多网卡），server 观测到的那个不一定就是 socket 实际使用的那个。因此 server 观测地址只当 **提示**，而且**仅当它本身是 IPv6 时**才采纳（客户端经 IPv4 连上 server 的话，这个地址对 v1 毫无用处）。本地枚举结果为准。
- 候选顺序（双方一致）：全局单播 → ULA → 链路本地。全部候选都会被尝试；先建立起来的会话获胜，之后的地址变化由 WireGuard 漫游吸收。
- **地址轮换是常态而非异常**：临时地址可能在传输中途被弃用，正常路径是靠对端漫游把会话接续下去；接不住就走 30 s 空闲超时（§3.7）。
- **不使用 UDP reflector**：IPv6 没有 NAT，也就没有需要探测的端口改写。若某部署确实会改写 UDP 源端口，可以日后把 Go 版的 reflector 加回来，不影响其他部分。

### 3.6 传输与完整性

**为什么没有第二层加密（D8）。** WireGuard 已经在两个 peer 之间提供机密性与完整性，而 peer 就是隧道端点，中间没有中继 —— 这层保护本身就是端到端的。再套一层加密只会增加每字节 CPU 开销而没有任何新增防护面。传输层做的是另一件事：**完整性校验**，用来发现截断、顺序错乱、对端（或自己）谎报成功这类问题。

**摘要为什么走 POST 而不是响应头。** 发送方是边读边发的，而 HTTP 响应头必须在正文之前发出。若要把摘要放进响应头，就必须先把整个文件预读一遍算出 SHA-256 —— 多一次全量磁盘读，而且文件在预读与发送之间被修改会造成假失败；stdin 这种流更没有预读的可能。因此流程设计成双方各自只读一遍、只算一遍，最后用一次 POST 交换裁决：

| 步骤 | 动作 |
| --- | --- |
| 1 | rx 连上 10.66.66.1:<http_port>，发 `GET /f/<token>` |
| 2 | tx 回 `200` + `Content-Length` + `Content-Disposition` + `X-Hermod-Kind`，随后开始流式发送，边发边算 SHA-256 |
| 3 | rx 边收边写入临时文件（`O_EXCL`、0600），边算 SHA-256，并核对收到的字节数 |
| 4 | rx 发 `POST /done`，正文 `{"sha256":"<hex>","size":N}` |
| 5 | tx 把自己算出的摘要与长度和提交值比对，回 `200 {"status":"ok"}` 或 `200 {"status":"mismatch"}` |
| 6 | rx 仅在收到 `ok` 后原子改名（必要时加去重后缀）；`mismatch` 则删除临时文件并退出非 0 |

其他约定：

- **长度校验**：`Content-Length` 与实际收到的字节数不一致即失败。chunked 流没有长度，只有摘要这一道校验。
- **进度显示**：有 `Content-Length` 时显示确定进度，否则显示不确定进度（与 Go 版行为一致）。
- **路径安全**：沿用 Go 版 `transfer` 模块语义 —— 路径穿越防护（拒绝 `../`、绝对路径、符号链接逃逸）+ 目标已存在时自动加去重后缀。
- **协议开销**：HTTP 头几百字节；对任何值得用这个工具传的文件都可忽略。真正的大头是 WireGuard 每包 80 字节（约 5%），其杠杆是 MTU 而不是协议设计（§5.2）。

### 3.7 失败模式、超时与清理

| 失败情形 | 检测方式 | 行为 |
| --- | --- | --- |
| 对端没有可用 IPv6 | bundle 中候选列表为空 | 握手前即失败：`peer advertised no IPv6 address` |
| 本机没有 IPv6 连接 | 枚举不到可用地址，或 socket 绑定失败 | 本地准备阶段失败：`this host has no IPv6 connectivity` |
| 对端不可达（CPE 连出站后仍拦入站，或网络整体过滤入站） | 5 s 内任何候选都没有握手响应 | 快速失败：`peer is not reachable over IPv6 from this network`，并附上尝试过的候选列表；不挂起、不静默重试 |
| server 不可达或证书变化 | TLS 连接失败或 pinning 不匹配 | 走既有 `trust` 流程 |
| 传输码错误 | CPace 无法达成一致的 K | 干净中止，并计入 server 侧的 CPace 失败上限 |
| 握手包丢失 | 5 s 内未建立会话 | 加抖动重试两次，仍失败则报错退出 |
| 传输中途隧道停滞 | 连续 30 s 无数据 | 中止、删除临时文件、退出非 0 |
| 摘要不一致 | rx 提交的摘要与 tx 计算值不同 | 删除临时文件，打印期望与实际摘要 |
| 长度不符 | 收到字节数与 `Content-Length` 不同 | 同上一行 |
| 任一方取消或收到 SIGINT | 连接关闭、EOF 或信号 | 关闭连接；tx 见 EOF 即中止并清理，双方都干净退出 |
| 传输中途对端地址变化 | WireGuard 漫游接管新源地址 | 传输继续；接不住则走 30 s 空闲超时 |

每一条路径都必须做到：删除临时文件、关闭 TCP 连接、销毁 WireGuard 会话、关闭 UDP socket。宿主机上没有任何需要清理的状态，因为整个设计从未在宿主机上安装过接口、路由或地址。

### 3.8 并发与线程模型

- **数据线程（每会话一个）**：持有 UDP socket、WireGuard 会话对象、smoltcp interface 与其 socket。阻塞 poll + 唤醒 fd，另有 100 ms 定时器 tick 驱动 WireGuard 的 update_timers。这个线程里没有 async，也不与控制面共享可变状态。
- **tokio 运行时（控制面）**：WebSocket、CPace、配置读写、进度渲染、信号处理、文件 I/O 调度。
- **桥接**：两侧通过有界通道连接，背压真实存在、内存有界（目标：传 1 GiB 时 RSS 低于 256 MiB）。
- **哈希**：独立线程。单线程 SHA-256 在 SHA-NI 下约 1.5–2 GB/s，超过约 10 Gbit/s 会成为瓶颈；独立线程同时避免哈希抖动干扰 TCP。
- **吞吐上限**：单隧道即单数据线程，本版上限在 1–3 Gbit/s 量级（D10）。1 GbE 及以下完全够用；要更高需要改架构（多 lane 或内核路径），不在本版范围。

---

## 4. IPv6 可达范围与后续打洞计划

v1 按决策 D7 只做 IPv6，本节列清它覆盖哪些场景。

| 场景 | v1 是否支持 | 原因 |
| --- | --- | --- |
| 双方都有全局单播 IPv6，跨网络 | 支持 | 无 NAT；同时发握手，一个 RTT 打开有状态防火墙的 pinhole |
| 双方在同一链路（全局、ULA 或链路本地） | 支持 | 本地候选带 scope id 被公布，直接连上 |
| 对端在 CPE 有状态防火墙之后（拦截非主动入站） | 通常支持 | 出站握手包建立了 pinhole，对端同时对发的握手因此被接受 |
| CPE 在出站后仍拦入站，或网络整体过滤入站 IPv6 | **v1 不支持** | 这正是后续打洞阶段要解决的场景 |
| 任一端没有 IPv6 | **不支持** | 在会合阶段就报错，不会走到握手 |
| IPv4 场景（含 CGNAT、双端 NAT） | **不支持，属范围外** | IPv4 传输与 IPv4 打洞整体出范围（D7），不是漏做的功能 |

后续阶段（不属于 v1，也不需要 v1 为其预留接口）：

1. **把 Go 版打洞算法作为回退路径加回**：探针与确认机制、候选排序、双阶段时序，以及传输期间持续探测以维持 pinhole 与映射。结果由 endpoint manager 吸收，其他部分不变。
2. **IPv4 仍出范围**，除非未来反转 D7。若真要支持，必须连打洞一起做 —— 没有打洞的 IPv4 版几乎没有可连通的组合。

---

## 5. 性能方案

### 5.1 成本模型

| 环节 | 成本所在 | 手段 |
| --- | --- | --- |
| UDP 系统调用 | 未批量时每个报文一次 syscall | recvmmsg / sendmmsg；Linux 上可用时叠加 UDP_SEGMENT（GSO） |
| WireGuard 加密 | boringtun 内的 ChaCha20-Poly1305（AVX2 / NEON） | 批量处理；单会话单核 |
| IP 与 TCP | smoltcp 每个包都要过环形缓冲拷贝 | 大缓冲、窗口缩放、批量 poll |
| 应用层拷贝 | 缓冲池复用 | 1 MiB 块，避免逐包分配 |
| SHA-256 | SHA-NI 下单核约 1.5–2 GB/s | 独立线程 |
| 内核加速 | **完全不可用**（D3） | 无 —— 这就是「全平台零特权」的代价 |

### 5.2 开销透视（1500 字节外层 MTU）

| 层 | 开销 | 说明 |
| --- | --- | --- |
| **WireGuard + UDP + IPv6 外层头** | **每包 80 字节，约 5%** | 承载层固有开销。巨帧链路用 `--wg-mtu 8900` 可压到 1% 以下 |
| HTTP 响应/请求头 | 每次请求几百字节 | 对 1 GiB 文件可忽略 |

结论：**真正值得动的是 MTU，而不是协议设计**。本层的头部开销在任何合理实现下都可以忽略。

### 5.3 调优顺序

1. 批量 UDP I/O（recvmmsg / sendmmsg）。
2. 固定 1 MiB 的缓冲池复用。
3. 8 MiB TCP socket 缓冲 + 窗口缩放校验。
4. 哈希放到独立线程。
5. 顺序读提示（posix_fadvise）与 4 MiB 读粒度。
6. 进度渲染只用原子计数。
7. 仅在实测需要时才做：busy-poll、io_uring、拥塞控制改进。

### 5.4 基准方法

两种 profile，均不需要 root：

- **干净路径**：loopback 或同 LAN，1 GiB 载荷，目标达到同机 `wireproxy` 参考值的 90% 以上。
- **丢包路径（用户实际问题）**：进程内丢包/延迟 shim 包住 UDP socket，1% 丢包、20 ms RTT、0.1% 乱序，带随机种子以保证可复现。

之所以用进程内 shim 而不是 `tc netem`：本开发机是 WSL2，`sudo` 被 `no_new_privs` 拦住，CI 同样没有权限；用户态 shim 可移植、确定性强，并且能进入测试套件。

同机同 profile 的参考点：`hermod-go`（基线）、`iperf3` TCP/UDP、`wireproxy` 隧道、`hermod-rs`。报告吞吐、每 GiB 的 CPU 秒数、p99 停顿。**可行性已由用户实测确认**：wireproxy 隧道内的 TCP 连接实测基本达到链路带宽极限（wireguard-go + gVisor netstack，纯用户态）。因此这里的数字用于调优与回归，不作为准入判断；wireproxy 的实测值同时作为 smoltcp 的对照基准。

---

## 6. 安全模型与回归登记

### 6.1 保留与丢失

| 保留 | 丢失（均已接受） |
| --- | --- |
| CPace PAKE：传输码完成认证，server 读不到也伪造不了会话密钥 | 后量子（ML-KEM 移出，PSK 派生处保留恢复接缝） |
| Server 自签证书 pinning（TOFU + `trust`） | peer 间 TLS / 证书 / ALPN |
| channel id 作为 AEAD 的 AAD、SHA-256 完整性、原子写、路径穿越防护、限速 | 带外校验（SAS 已删除，D12） |
| 传输层不做第二层加密的前提是隧道端点即 peer | 猜中传输码者可静默冒充（补偿：码熵 + server 限速） |

### 6.2 回归登记

| # | 变更 | 影响 | 缓解 |
| --- | --- | --- | --- |
| S1 | 移除后量子 pillar，CPace（P-256）成为唯一协商 pillar，WireGuard 静态密钥为 X25519 | 具备量子计算机的攻击者可「先记录后解密」 | 已接受（D6）。恢复路径清晰：改 K 的派生（加入 ML-KEM 输出），协议不变 |
| S2 | 对端 WireGuard 公钥经中继传递 | 恶意中继可尝试替换 | bundle 由 CPace 密钥 AEAD 加密（server 算不出），PSK 又把握手绑到该密钥上；需测试「替换公钥导致握手失败」 |
| S3 | **删除 SAS，不再有带外校验** | 猜中传输码的攻击者可静默冒充对端，双方察觉不到 | 码熵是唯一防线：`--words 5`（约 52 bit）用于敏感传输；server 侧保留 CPace 失败上限（每 channel 3 次）、每 IP channel 上限与令牌桶。README 必须明确写出这一点 |
| S4 | 隧道内自研 HTTP 元数据与头解析 | 新增解析面，持有传输码者可触达 | 长度上限、checked 运算、对头解析与 URL token 解析做模糊测试 |
| S5 | 隧道在进程内、无特权 | 无内核面，但进程要处理对端全部流量 | Rust 规范：尽量 forbid unsafe、密钥用 zeroize、任何 verbosity 下都不打印密钥 |
| S6 | 新增依赖面（boringtun、smoltcp、p256、aes-gcm、HTTP 栈） | 供应链风险 | 固定版本、CI 跑依赖审计、密码学继续用 RustCrypto（与 Go 版同算法） |
| S7 | 隧道内是明文 `http://` | 一旦有人加 TLS 或在隧道外暴露该端口，保护假设就变了 | 代码评审清单写死：不允许在数据面添加 TLS；HTTP server 只绑覆盖网地址，绝不绑宿主机地址 |

### 6.3 一条必须写进代码注释的硬前提

「传输层不加密」成立的唯一原因是 **WireGuard 隧道的两个端点就是两个 peer**。一旦未来数据面出现「隧道在中间某点终止」的形态（例如为打不通的组合加 TCP 中继），**应用层端到端加密立刻变成必需**，而不是可选优化。

---

## 7. 风险清单

| # | 风险 | 概率 | 影响 | 缓解与预警信号 |
| --- | --- | --- | --- | --- |
| R1 | 用户态 TCP 栈成为全部性能故事，且没有特权退路（D3） | 中 | 高 | 形态已由用户自己的 `wireproxy` 实测验证 —— 隧道内 TCP 实测基本达到链路带宽极限（同样是用户态 Go 栈），说明无特权可达；M5 针对该参考值测量，差距在栈内解决：缓冲、批量、拥塞控制 |
| R2 | smoltcp 把拥塞控制建模为 Reno/Cubic 的 enum，BBR 无法插拔 | 高 | 中 | 先在丢包 profile 下测 CUBIC；若丢包恢复是瓶颈，作为 M5 任务向上游贡献或本地打补丁 |
| R3 | IPv6 实际可达率低于预期（严格 CPE、整体过滤入站、对端无 IPv6） | 中 | 高（产品范围） | 快速失败并给出明确诊断；M3 记录真实可达率与失败组合及原因，该报告就是启动打洞阶段的依据 |
| R4 | server 观测到的地址/端口与 UDP socket 实际使用的不同（临时地址、多前缀、改写型 CPE） | 高 | 中 | 公布全部本地候选，server 观测地址仅作提示且必须是 IPv6（§3.5）；M3 用多地址主机验证 |
| R5 | boringtun 在「双方同时发起」时的行为与协议规定不一致 | 中 | 中 | M3 做显式竞态测试；退路是固定角色（tx 发起、rx 响应，超时后再发起） |
| R6 | 批量收发与 GSO 的可用性因平台而异，Windows/macOS 可能慢于 Linux | 中 | 中 | M5 分平台测量并如实记录，不假设三平台一致 |
| R7 | 范围蔓延：IPv4、reflector、VPN、并行 lane、断点续传被陆续塞回 v1 | 中 | 中 | 第 4 节与第 0 节划定了边界；打洞阶段单独规划，不需要提前塞进来。数据面仍保持在小接口之后，将来加回不需要重设计 |
| R8 | 安全姿态静默退化（无后量子、无 SAS、无第二层加密） | 低 | 高 | 第 6 节登记表 + README 明确写出当前姿态 |
| R9 | smoltcp socket 与 tokio 流 API 之间的适配器是数据面最容易出错的地方（帧边界、背压、EOF） | 中 | 中 | 适配器保持极小，用内存假设备 + 边界模糊测试覆盖，不依赖网络即可测试异步管线 |

---

## 8. 里程碑与任务

### M0 —— 不适用

可行性在本计划之前已经验证：用户实测过「用户态 WireGuard + TCP」这套形态能够跑满链路，而 Go 的 QUIC 实现跑不满。因此不设 spike 门禁、不做 go/no-go 判断。剩下的工作是实现，以及 M5 里的调优与回归测量。

**TDD 路线记录**

    模式（Mode）: off
    结论（Decision）: skipped
    强制依据（Strict authority）: 不适用（用户与项目均未要求严格 TDD）
    测试姿态（Test posture）: M1 对齐从 Go 版提取的一致性向量；M4 起使用丢包 shim 做端到端传输测试；M5 做吞吐回归
    原因（Reason）: 用户先要设计文档；上游项目也是先实现后测试，而非测试先行
    验证方式（Verification）: 每个里程碑的完成标准都要求 cargo test、cargo clippy（warnings 视为错误）、
                              帧解析模糊测试、端到端测试全部通过

### M1 —— 工程骨架与核心（2–4 天）

| 任务 | 文件 | 验证 |
| --- | --- | --- |
| M1.1 骨架 | Cargo workspace、toolchain 固定、两个 crate 空壳、统一 lint | `cargo build --workspace` 与 `cargo clippy -- -D warnings` 干净 |
| M1.2 CPace | `hermod-core/src/crypto/{cpace,kdf}.rs` | RFC 9380 hash-to-curve 向量与从 Go 测试提取的 CPace 向量全部通过 |
| M1.3 AEAD 与词表 | `crypto/{aead,wordlist}.rs` | channel id 作 AAD 的加解密往返正确；词表与 Go 版逐字节一致 |
| M1.4 元数据与头部约定 | `hermod-core/src/http_meta.rs` | 头部字段编解码往返；长度上限生效；模糊测试不得崩溃 |
| M1.5 传输辅助与配置 | `transfer.rs`、`config.rs` | 从 Go 版移植的路径安全用例通过；Go 版写出的配置文件可原样解析 |
| M1.6 向量提取工具 | `scripts/extract-vectors.sh` | Go 测试输出 JSON 向量，`cargo test` 能消费 |

### M2 —— 会合（2–4 天）

| 任务 | 文件 | 验证 |
| --- | --- | --- |
| M2.1 线格式 | `hermod-core/src/signal/protocol.rs` | 编解码与从 Go server 抓取的报文一致 |
| M2.2 客户端与 pinning | `hermod-core/src/signal/{client,tls}.rs` | 能连上 Go server、pin 证书、证书变化时拒绝 |
| M2.3 经中继的 PAKE | `hermod-core/src/{crypto/cpace, signal/client}.rs` | 两个 Rust 进程经真实 server 得到同一个 K，且 server 无法算出 |
| M2.4 Bundle | `hermod-core/src/tunnel/session.rs` | 加解密往返；AAD 不匹配时拒绝；版本不符时给出明确错误 |
| M2.5 IPv6 地址枚举 | `hermod-core/src/tunnel/addr.rs` | 用桩接口列表做单测；server 观测地址仅作提示；候选排序正确 |
| M2.6 Rust server（可延后） | `hermod-core/src/signal/{server,store,ratelimit}.rs` | Go 客户端能完成 allocate、join 与 blob 中继 |

### M3 —— WireGuard 直连（3–5 天）

| 任务 | 文件 | 验证 |
| --- | --- | --- |
| M3.1 密钥与 PSK | `hermod-core/src/tunnel/wg/{keys,tunnel}.rs` | 两个进程内 peer 握手成功；PSK 不匹配时握手失败 |
| M3.2 端点管理 | `hermod-core/src/tunnel/wg/endpoint.rs` | 候选顺序被遵守；模拟地址变化后一个已认证报文即可跟随 |
| M3.3 同时发起 | `hermod-core/src/tunnel/wg/tunnel.rs` | 竞态测试：双方同时发起时只产生一个会话，且数据面两种情形都可用 |
| M3.4 真实网络可达性 | `hermod-core/src/tunnel/session.rs` | 跨网络与同链路的 IPv6 传输均成功；无 IPv6 与不可达两种情况都在 5 s 内给出明确诊断 |
| M3.5 可达率报告 | `docs/benchmarks/reachability.md` | 记录测试集内每种 IPv6 网络组合的成功率与失败原因，作为启动打洞阶段的依据 |

### M4 —— HTTP 传输与 CLI（4–6 天）

| 任务 | 文件 | 验证 |
| --- | --- | --- |
| M4.1 smoltcp 桥接 | `hermod-core/src/tunnel/stack/{device,driver}.rs` | 隧道内 ICMP echo 与 TCP 连接均可用 |
| M4.2 异步流适配 | `hermod-core/src/tunnel/stack/bridge.rs` | `AsyncRead` / `AsyncWrite` 具备真实背压，无忙等；用内存假设备跑通边界用例 |
| M4.3 隧道内 HTTP | `hermod-core/src/tunnel/http/{server,client}.rs` | tx 能用 curl（经隧道）取到文件；200/404/长度/chunked 各分支正确 |
| M4.4 传输主流程 | `hermod-core`（编排 API）＋ `hermod-cli/src/{tx,rx}.rs`（薄壳） | 1 GiB 文件、文本、stdin 管道三种载荷端到端成功；摘要一致才改名 |
| M4.5 取消与超时 | `hermod-cli/src/{tx,rx}.rs` | 任一方取消与 SIGINT 均不留临时文件、退出码正确 |
| M4.6 CLI 对齐 | `hermod-cli/src/cli.rs` | 与 Go 版 help 逐参数对照（差异仅附录 E 列出的部分） |

### M5 —— 性能（3–5 天）

| 任务 | 验证 |
| --- | --- |
| M5.1 批量 UDP I/O 与缓冲池 | 两个 profile 上都有可测收益，干净 profile 无回归 |
| M5.2 8 MiB 窗口与窗口缩放 | 丢包 profile 吞吐提升，内存有界 |
| M5.3 哈希独立线程 | 每 GiB CPU 时间下降，摘要结果不变 |
| M5.4 拥塞控制评估 | CUBIC 与打补丁后的替代算法实测对比，结论写入 ADR |
| M5.5 分平台测量 | Linux / macOS / Windows 数字齐全，差异有解释 |

### M6 —— 打包与发布（2–3 天）

| 任务 | 验证 |
| --- | --- |
| M6.1 发布产物 | 静态 musl Linux、macOS 双架构、Windows |
| M6.2 文档 | README 写清用法、IPv6 范围限制、以及无带外校验带来的安全姿态 |
| M6.3 CI | 构建、测试、clippy、依赖审计、丢包 profile 端到端任务 |

---

## 9. 验收标准

1. **可达性（仅 IPv6）**：跨网络与同链路传输成功，成功率有记录；对端无 IPv6 与不可达两种情况都在 5 s 内给出文档化的诊断，而不是挂起。
2. **吞吐（干净路径）**：同机达到 `wireproxy` 参考值的 90% 以上。
3. **吞吐（丢包路径：1% 丢包、20 ms RTT）**：至少为 Go 版基线的 2 倍，且 p99 停顿不超过 1 s。这是最初的痛点，也是首要指标。
4. **只有一层加密**：数据面仅由 WireGuard 保护，传输层贡献的是校验而不是第二套密码（D8）。
5. **接收端无状态**：除传输的文件本身外不留任何东西；中断只留下临时文件，两次运行之间无需匹配任何状态（D9）。
6. **无特权**：任何路径都不需要 root、管理员权限、驱动或 TUN 设备（D3）。
7. **平台**：Linux、macOS、Windows 均可构建并完成传输。
8. **会合兼容**：Rust 客户端能对 Go server 完成整个会合流程，`trust` 流程行为不变。
9. **行为一致性**：tx、rx、serve、trust 的命令名、参数、默认值与退出行为保持一致；差异仅限附录 E 明确列出的部分；配置文件双向兼容。
10. **质量门禁**：单元与端到端测试、clippy（warnings 为错）、格式化检查、依赖审计、头部与 URL token 解析的模糊测试全部通过。
11. **清净性**：任何取消或崩溃路径都不残留临时文件、socket、线程或任务；任何 verbosity 下都不打印密钥；传 1 GiB 时 RSS 低于 256 MiB。

---

## 10. 待定事项

| # | 事项 | 需要时间点 | 建议 |
| --- | --- | --- | --- |
| 1 | Rust 版 server 何时实现 | M4 之后 | 延后：Go server 兼容，先不阻塞客户端 |
| 2 | 打洞阶段何时启动 | M3 报告出来后 | 依据真实可达率与失败组合决定，不预设 |
| 3 | 是否引入 BLAKE3 | M5 | 仅在实测显示 SHA-256 成为瓶颈时 |
| 4 | 巨帧场景是否调整 `--wg-mtu` 默认值 | M5 | 默认保持 1420；文档给出 8900 的用法与收益 |
| 5 | 传输码默认词数是否从 3 提到 4 | M1 | 保持 3 与 Go 版一致；由 README 引导敏感场景用 `--words 5` |
| 6 | 是否支持 IPv4 | 未定 | 需先反转 D7，且必须与打洞同时做 |

---

## 附录 A — 依赖清单（2026-10-11 核实）

| crate | 版本 | 核实方式 | 用途 |
| --- | --- | --- | --- |
| `boringtun` | 0.7.1（2026-05-01） | crates.io 与 docs.rs API | 用户态 WireGuard 协议实现，不需要 TUN（`noise::Tunn`） |
| `smoltcp` | 0.14.0（2026-08-17） | docs.rs | 进程内 TCP/IP 栈，`Medium::Ip`，CUBIC 与 Reno，支持 SACK 与窗口缩放 |
| `tokio` | 1.53.2 | crates.io | 控制面运行时 |
| `rustls` | 0.23.45 | crates.io | 与 server 之间的 WebSocket TLS，通过自定义 verifier 实现证书 pinning |
| `tokio-tungstenite` | 当前版本 | crates.io | WebSocket 客户端与服务端 |
| `hyper`、`http-body-util`、`tower-http` | 当前版本 | crates.io | 隧道内 HTTP/1.1（D11）；`ServeFile` 负责静态文件与头部 |
| `clap` | 4.6.7 | crates.io | CLI 参数（与 Go 版的 flag 面对齐） |
| `p256` | 0.14.0 | crates.io | CPace：P-256 与 `hash2curve`（SSWU） |
| `x25519-dalek` | 3.0.0 | crates.io | WireGuard 静态密钥 |
| `aes-gcm` | 0.11.1 | crates.io | bundle 加密，channel id 作 AAD |
| `sha2`、`hmac`、`hkdf`、`zeroize`、`subtle` | 当前版本 | — | 摘要、MAC、派生、密钥清理、常量时间比较 |
| `ml-kem` | 0.3.2 | crates.io | **v1 不使用**（D6）；这是 S1 的恢复路径 |
| `defguard_wireguard_rs`、`wireguard-uapi` | 0.12.2、3.0.1 | crates.io | **已评估、不使用**：它们管理 WireGuard *接口*（内核或 TUN），D3 已将其排除 |

明确不使用：`quinn`（QUIC，D2 已丢弃）、`wireproxy` / `wireguard-go`（Go，无法链接；仅作为 M5 的测量参考）。

## 附录 B — boringtun API 说明（依据 docs.rs 0.7.1 核实）

```rust
// boringtun 0.7.1，模块 "noise"。用 default-features = false 构建可跳过默认特性引入的 TUN 设备层。
pub fn Tunn::new(
    static_private: StaticSecret,
    peer_static_public: PublicKey,
    preshared_key: Option<[u8; 32]>,
    persistent_keepalive: Option<u16>,
    index: u32,
    rate_limiter: Option<Arc<RateLimiter>>,
) -> Self;

pub fn encapsulate<'a>(&mut self, src: &[u8], dst: &'a mut [u8]) -> TunnResult<'a>;
// dst 至少为 src.len() + 32，且不小于 148 字节，否则会 panic。

pub fn decapsulate<'a>(&mut self, src_addr: Option<IpAddr>, datagram: &[u8],
                       dst: &'a mut [u8]) -> TunnResult<'a>;
// src_addr 就是实现漫游的入口：返回值告诉我们该发什么、发到哪；
// 一个成功解封装的报文即确定当前对端端点。

pub fn update_timers<'a>(&mut self, dst: &'a mut [u8]) -> TunnResult<'a>; // 约 100 ms 一次
pub fn time_since_last_handshake(&self) -> Option<Duration>;
pub fn is_expired(&self) -> bool;
```

结论：**端点管理、socket 归属、批量收发与保活策略都是我们自己的代码**，这正是点对点会话需要的，也正是 `wireproxy` 从 `wireguard-go` 设备层白拿的部分。`decapsulate` 能接受来自尚未确定地址的报文，也是「双方同时发起」可行的基础。

## 附录 C — 开发环境实测

| 事实 | 数值 | 影响 |
| --- | --- | --- |
| 宿主 | WSL2，内核 6.18.40.1-microsoft-standard-WSL2，14 核 | 用户态数据面的开发与测试没问题 |
| WireGuard 内核模块 | 存在，版本 1.0.0 | 与本项目无关：D3 已排除特权路径 |
| /dev/net/tun | 不存在 | 印证 TUN 方案不能作为依赖，本机与受限主机都一样 |
| sudo | 被 no_new_privs 拦住 | 无法用 tc netem，因此采用进程内丢包 shim（§5.4） |
| 工具 | 有 tc、iperf3、ip、ss；缺 dig、jq、ncat、socat | 基准用 iperf3 + 进程内 shim；脚本不要依赖 jq |
| IPv6 | 主机同时持有多个全局地址与临时地址 | 正好用于验证多地址候选合并（§3.5） |
| 工具链 | cargo / rustc 1.99.0，edition 2024 | 与现有 hermod-core、hermod-cli 骨架一致 |
| 外网 | crates.io 可达（JSON API 需要 User-Agent 头） | 正常依赖解析可用 |

## 附录 D — 常量表

| 常量 | 取值 | 说明 |
| --- | --- | --- |
| Server 端口 | 4376 | Go 与 Rust server 共用 |
| 覆盖网地址 | 10.66.66.1（tx）、10.66.66.2（rx），/32 | 进程内接口，不会冲突 |
| 覆盖网 HTTP 端口 | 临时端口，在 bundle 中公布 | 不需要固定端口，也不需要特权端口 |
| 覆盖网 MTU / MSS | 1420 / 1380 | 用户态栈没有 PMTU 发现；外层为 IPv6 时算术为 1500 − 40 − 8 − 32 = 1420 |
| WireGuard keepalive | 25 s | 短于 30 s 的载荷空闲超时 |
| WireGuard rekey / reject | 120 s / 180 s | 协议常量 |
| 握手预算 | 5 s，含两次带抖动重试 | 超时即给出不可达诊断 |
| TCP 连接预算 | 10 s | |
| 载荷空闲超时 | 30 s | |
| TCP socket 缓冲 | 每方向 8 MiB | 启用窗口缩放 |
| 传输码 | EFF 短词表，默认 3 词 | 与 Go 版一致；敏感场景用 --words 5 |
| Channel TTL / blob 上限 / CPace 失败上限 | 600 s / 10 / 3 | server 侧，与 Go 版一致 |

## 附录 E — 与 Go 版 CLI 的差异

| 项目 | Go 版 | Rust 版 | 原因 |
| --- | --- | --- | --- |
| `--verify` / `-v` | 存在，启用 SAS 带外校验 | **删除**（D12） | SAS 不保留。传该参数会得到 `error: unrecognized option`，而不是被静默忽略 |
| `--server` / `--words` / `--listen` / `--destination` | 存在 | 保持一致（新增 `--wg-mtu`） | 保持用户习惯 |
| `serve` 的 `--ttl`、`--rate-limit`、`--rate-burst`、`--max-*` | 存在 | 保持一致（Rust server 延后实现） | server 行为契约不变 |
| 与 Go 客户端互传文件 | — | **不支持** | 数据面完全不同（QUIC vs WireGuard + HTTP）；失败时给出明确错误，不静默挂起 |
| 与 Go server 会合 | — | **支持** | signaling 契约保留（D1） |






