# CLI 参数重构与传输调优 —— 设计文档

- 状态：已定方案，待实现
- 日期：2026-11-13
- 范围：`cli`（参数面与输出）、`core`（IPv4/IPv6 选择、档位识别、QUIC 调优项）
- 不涉及：`flutter-bridge` 与 `flutter` 的界面改动（见 §9，本方案保证零改动兼容）
- 关联：`AGENTS.md`（逻辑只住 `core`）、`core/src/transport/quic/transport.rs`、
  `core/src/orchestrate.rs`、`core/src/transport/network.rs`、`core/src/session.rs`

---

## 1. 背景

当前 CLI 参数有三类问题：

1. **默认路径不可用**：`tx`/`rx` 的 `--congestion` 默认值是字符串 `"BBR"`，而
   `Congestion::from_str` 只接受小写。实测不传 `--congestion` 时程序直接退出：

   ```
   error: invalid value 'BBR' for '--congestion <CONGESTION>'
   ```

   `cli::tests::defaults_and_removed_options` 目前因此 FAILED。同类问题还有
   `--profile Lan`（大小写敏感）。

2. **参数分层错位**：`--mtu`/`--congestion` 属于数据面传输调优，却被定义在
   `tx`/`rx` 各自子命令里，重复声明两次；`serve` 与 `trust` 完全用不到。

3. **历史残留**：`serve` 子命令与完整 server 实现都在，但 `README` 仍写着
   "`serve` 已删除、信令服务需单独部署"；`cli/Cargo.toml` 注释仍提"对标 Go 的 slog"；
   `main.rs` 注释仍讲"WireGuard/smoltcp 会爆栈"。迁移表里的 `--transport wireguard`、
   `--limit-mbps` 两个参数都已不存在。

本方案同时处理参数分层与"把 QUIC 传输里已经硬编码的调优项按需开放给用户"。

## 2. 目标与非目标

### 目标

1. 全局参数只保留**跨子命令有效**的项；`--mtu`/`--congestion`/`--profile` 上提为全局。
2. 消除默认值解析失败；所有枚举参数大小写不敏感，帮助文本自动列出取值。
3. 新增 `-4`/`-6` 地址族限制，贯穿"监听 / 信令连接 / 打洞"三条链路。
4. `serve` 只暴露端口，内部固定双栈；删除 `-l/--listen` 与 `HERMOD_LISTEN` 魔法比较。
5. `--profile` 语义从 "auto/lan/compatible" 收敛为 "auto/lan/wan"，`auto` 按
   **对端地址类型**判定，而不是只看 RTT。
6. 把真正影响吞吐的 QUIC 参数（窗口、切片、缓冲）按 A/B 两档开放，默认值等于现状。
7. 修正一处实测到的窗口配置缺陷（§7）。

### 非目标

- 不改 payload 协议（v3 的 `MODE_SINGLE`/`MODE_MULTI` 帧、切片计划格式）。
- 不引入 0-RTT；会话密钥来自 CPace，且 QUIC 证书每会话临时生成，协议上不成立。
- 不暴露丢包检测阈值（`packet_threshold`/`time_threshold`）等"改错会误判拥塞"的进阶项。
- 不重构 GUI 设置页；`compatible` 保留为别名（§6）。

## 3. 目标参数表

```
hermod [GLOBAL] <tx|rx|trust|serve>
```

### 3.1 全局

| 参数 | 短 | 默认 | 取值 | 说明 |
| --- | --- | --- | --- | --- |
| `--verbose` | — | `none` | none/error/warning/info/debug | tracing 级别；默认不再刷 INFO |
| `--quiet` | `-q` | off | — | 只压状态行，错误始终显示 |
| `--ipv4` | `-4` | off | — | 仅 IPv4：UDP 绑 0.0.0.0、TCP 只连 A 记录、不发布 IPv6 候选 |
| `--ipv6` | `-6` | off | — | 仅 IPv6：UDP 绑 `[::]` 且 `only_v6(true)`、只连 AAAA、不发布 IPv4 候选 |
| `--mtu` | `-m` | `auto` | auto 或 1200–1452 | QUIC UDP 载荷上限；`auto` 从 1200 探测到 1452 |
| `--congestion` | `-c` | `bbr` | auto/bbr/cubic/newreno | `auto` 即 bbr（QUIC 唯一数据面） |
| `--profile` | `-P` | `auto` | auto/lan/wan | 链路档位，见 §5 |
| `--version` | `-V` | — | — | |

`-4` 与 `-6` 互斥（`conflicts_with`）。短命令冲突已核对：`-P`（profile）与
`-p`（serve port）大小写不同，clap 视为不同 id。

**`serve`/`trust` 对数据面参数的处理**：三个参数在 `serve`/`trust` 下可见但无意义。
按"禁止静默忽略"原则（`options.rs` 现有约定），显式传入即报错。实现上这些字段必须是
`Option<T>`，否则 clap 无法区分"用户传了"与"用了默认值"。

### 3.2 子命令

| 子命令 | 参数 | 短 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `tx` | `<INPUT>` | — | — | 文件路径 / 文本 / `-`（需 stdin 管道） |
| `rx` | `<CODE>` | — | — | 传输码，至少 3 词 |
| `rx` | `--output <DIR>` | `-d` | `.` | 落盘目录 |
| `trust` | `<SERVER>` | — | — | URL 或 host:port；成功后设为默认服务器 |
| `trust` | `--fingerprint` | — | — | 期望的 SPKI SHA-256 hex |
| `serve` | `--port` | `-p` | `4376` | 监听端口；内部双栈 |
| `serve` | `--ttl` | `-T` | `600` | 通道 TTL 与空闲超时（秒） |
| `serve` | `--rate-limit` | — | `5` | 每 IP 前缀令牌桶速率 |
| `serve` | `--rate-burst` | — | `15` | 令牌桶容量 |
| `serve` | `--max-blobs-per-channel` | — | `10` | 每通道中继 blob 上限 |
| `serve` | `--max-cpace-failures` | — | `3` | CPace 失败上限 |
| `serve` | `--max-channels-per-ip` | — | `100` | 每 IP 前缀通道上限 |

### 3.3 删除的参数

| 已删 | 理由 |
| --- | --- |
| `tx -w/--words` | 码长恒为 3 词（core 本身对 <3 静默提升为 3） |
| `tx -s/--server`、`rx -s/--server` | 服务器只来自 `config.yaml`（先用 `trust` 建立信任） |
| `--profile compatible` | 收敛为 `wan`；`compatible` 仅作解析别名保留（§6） |
| `serve -l/--listen`、`HERMOD_LISTEN` | 换成 `-p/--port`；地址族由 `-4/-6` 决定 |
| `--transport`、`--wg-mtu`、`--quic-mtu`、`--quic-cc`、`--limit-mbps` | 早已移除，仅剩文档与测试残留 |

## 4. `-4`/`-6` 的实现落点

新增 `core/src/options.rs::IpFamily { Auto, V4, V6 }`，挂在 `SessionOptions.family`，
向三条链路下发。

| 链路 | 位置 | Auto（现状） | V4 | V6 |
| --- | --- | --- | --- | --- |
| UDP 绑定 | `transport/network.rs::bind_udp` | `[::]:0`，失败回落 `0.0.0.0:0` | `0.0.0.0:0` | `[::]:0` 且 `only_v6(true)` |
| 候选发布 | `network.rs::candidates` | v6 + `local_ipv4()` | 只 v4 | 只 v6 |
| 打洞目标 | `network.rs::peer_addresses` | v4 优先、再 v6 | 只接受 v4 候选 | 只接受 v6 候选 |
| 信令连接 | `signal/client.rs::connect_pinned` | 系统默认顺序 | 只连 A 记录 | 只连 AAAA |
| 证书抓取 | `signal/tls.rs::fetch_server_spki` | 同上 | 同上 | 同上 |
| 服务监听 | `network.rs::bind_tcp` | 双栈、`only_v6(false)` | `0.0.0.0:port` | `[::]:port` 且 `only_v6(true)` |
| UDP 反射 | `server/reflector.rs::start` | 跟随监听地址 | 跟随 | 跟随 |

两条实现要求：

- **信令连接需要重写**。`connect_async_tls_with_config` 内部固定用
  `TcpStream::connect("host:port")`，无法按族过滤。改为自行 `lookup_host` →
  按族筛 → `TcpStream::connect(addr)` → `client_async_tls_with_config(request, stream, None, connector)`。
- **双栈监听必须显式设 `only_v6(false)`**。Windows 默认值不可靠，`socket2` 已在依赖树中。

过滤后若候选为空，**明确报错**（提示放宽 `-4/-6`），不得静默退回双栈。

## 5. `--profile` 语义

现状：`LinkProfile::{Auto, Lan, Compatible}`，`Auto` 只看实测 RTT 是否 ≤ 25ms。

问题：RTT 判据被"首包之前的估计"和跨网段链路污染；且 `compatible` 命名与"兼容性"
无关，实际含义是"弱网保守档"。

新语义：

| 值 | 判定 | 数据面 | 初始 RTT |
| --- | --- | --- | --- |
| `lan` | 强制 | 多流切片（≤8 条） | 25 ms |
| `wan` | 强制 | 单流大块流水线 | 100 ms |
| `auto` | **按对端地址类型**：私有/链路本地/回环/ULA → lan，global unicast → wan | 同左 | 同左 |

地址判定的单一实现放 `network.rs`：

```rust
pub enum LinkKind { Lan, Wan }
pub fn classify_link(addr: &SocketAddr) -> LinkKind
```

规则：`is_private() || is_link_local() || is_loopback()` → Lan；IPv6 `fc00::/7`(ULA)
→ Lan；其余 global unicast → Wan。

两端获取地址的时机不同：

- **tx（QUIC client）**：`establish` 前已有 `peer_addresses()` 结果，用它预判，
  无需等握手。
- **rx（QUIC server）**：需在连接建立后取对端地址。**当前 `TransportConnection`
  没有暴露对端地址**，`QuicConnection` 内部持有 `conn` 但未外露，需新增
  `fn peer_addr(&self) -> Option<SocketAddr>` 到 trait 与 `AnyConnection` 转发。

`auto` 下若同时拿到 RTT，用 `地址类型 ∧ RTT ≤ 25ms` 双条件，避免跨网段的
"低 RTT 假象"被误判为局域网。

## 6. 与 GUI 的兼容

`LinkProfile` 同时被 `flutter-bridge/src/api/mod.rs`、`flutter/lib/settings_page.dart`
使用，界面下拉项含 `compatible`。

处理方式：**保留 `Compatible` 变体，解析时接受 `"compatible"` 并映射为 `Wan`；
序列化输出仍为 `"wan"`，但 GUI 传入 `compatible` 不会报错**。这样 GUI 零改动，
CLI 只文档化 `auto/lan/wan`。

> 若不接受别名而彻底删除 `Compatible`，必须同步改
> `flutter-bridge/src/api/mod.rs`（字符串映射）、`flutter/lib/settings_page.dart`
> 与 `flutter/lib/core_bridge.dart`，超出"只改 cli 和 core"的范围。

## 7. 现状核对：已有调优 vs 仍缺的

对照"quinn 调优清单"逐项核对 `quic/transport.rs::transport_config()`：

| 建议项 | 项目现状 | 结论 |
| --- | --- | --- |
| BBR 切换 | 按 `--congestion` 注入 `BbrConfig`/`CubicConfig`/`NewRenoConfig` | 已做 |
| `initial_mtu(1200)` + PMTUD | `min_mtu(1200)`、`initial_mtu`、`upper_bound(mtu_limit)` | 已做 |
| `initial_rtt` / `keep_alive` | 100ms(WAN)/25ms(LAN)、10s | 已做，值不同 |
| 大收发窗口 | `stream_receive_window = receive_window = send_window = 64 MiB` | **有缺陷，见下** |
| socket2 16 MiB 缓冲 | `network.rs::UDP_BUFFER = 16 MiB` | 已做 |
| `[::]:0` + `only_v6(false)` | `bind_udp` 已按此实现 | 已做（但未受 `-4/-6` 控制） |
| UDP GSO | quinn 默认 `enable_segmentation_offload = true`，未显式关 | 已做 |
| SPKI pinning | `signal/tls.rs` 抓 SPKI；QUIC 用临时证书 DER 指纹 | 已做（比建议更严） |
| ALPN 单一 | `hermod/1` | 已做 |
| `max_concurrent_uni_streams(128)` | 只设 bidi=1024，uni 仍默认 100；项目只用 bidi | 无收益 |
| 0-RTT | `Resumption::disabled()`，会话密钥来自 CPace | 协议上不成立 |
| `read_chunk`/`write_chunk` 零拷贝 | 仍是 `Vec<u8>` + `write_all`/`read_exact`，1 MiB 一块 | 可优化，属代码改动 |
| 8 MB 流窗 / 32 MB 连接窗 | 统一 64 MiB | 已超出建议值 |

### 7.1 必须修正的窗口缺陷

quinn 的 `receive_window` **默认是 `VarInt::MAX`（不限）**，`send_window` 默认是
`8 * STREAM_RWND`。项目把三者统一设为 64 MiB，等于把连接级总接收窗口从"无限"
收紧到 64 MiB。

后果：多流模式 8 条流各需 64 MiB 才能不阻塞，合计 512 MiB，但连接总窗只有 64 MiB
——单流之外的数据全部被卡在 `MAX_DATA` 上，**多流吞吐收益被这条设置吃掉大半**。

修正：`receive_window = stream_window × streams`（或 MULTI 模式下保持
`VarInt::MAX`），`send_window` 同步放大。这是本方案里最值钱的一处改动，优先级高于
新增任何参数。

## 8. 开放给用户的调优参数

判据：QUIC 参数基本是**单边生效**的——接收窗是"通告我能收多少"，发送窗/PMTU/切片
是本地约束，所以用户单手调参即可，不必两端一致。唯一例外是
`max_concurrent_bidi_streams`（通告值），必须 ≥ 并发度，否则对端 `open_bi` 阻塞；
本方案按其与 `--streams` 联动，不单独暴露。

### A 档：直接加速（全部开放）

| 参数 | 默认＝现状 | 范围 | 落到 |
| --- | --- | --- | --- |
| `--streams` | auto（lan=8 / wan=1） | 1–8 | `orchestrate::TransferTuning`，联动 bidi 上限 |
| `--chunk` | 1 MiB | 256 KiB–4 MiB | `orchestrate::CHUNK` |
| `--window` | 64 MiB | 8–256 MiB | `stream_receive_window`/`send_window`，`receive_window = 基数×streams` |
| `--udp-buffer` | 16 MiB | 1–64 MiB | `network::UDP_BUFFER` |
| `--slice-size` | 4 MiB | 4–64 MiB | `orchestrate::SLICE_MIN` |
| `--prefetch` | 16 | 4–64 | `orchestrate::PREFETCH_QUEUE` |

### B 档：连接韧性（只开放前两个）

| 参数 | 默认＝现状 | 范围 | 落到 |
| --- | --- | --- | --- |
| `--keepalive` | 10 s | 0（关）–60 s | `TransportConfig::keep_alive_interval` |
| `--idle-timeout` | 30 s | 10–300 s | `max_idle_timeout`，须 > keepalive |
| `--initial-rtt` | 按 profile | 10–500 ms | `initial_rtt` |
| `--ack-delay` | 5 ms | 1–50 ms | `AckFrequencyConfig::max_ack_delay` |

`--initial-rtt` 与 `--ack-delay` 收进 `--adv` 或暂不开放：前者被 profile 覆盖，
后者改动收益小且影响对端 ACK 行为。

### 不开放

`packet_threshold`(3)、`time_threshold`(1.125)、`persistent_congestion_threshold`(3)、
`mtu_discovery.interval`(600s)、`black_hole_cooldown`(60s)、`gso`(true)、
`pad_to_mtu`(隐私换带宽)、uni stream 上限(项目不用)、0-RTT(协议不成立)、
`send_fairness`(默认已最优)。

### 范围校验

所有新参数在**任何网络访问之前**校验（沿用 `options.rs` 的 `validate_for` 约定）：
越界即报错，不做 clamp。`--idle-timeout` 必须大于 `--keepalive`，否则报错。

## 9. config.yaml 持久化

新增调优参数**同时支持 `config.yaml`**，字段组 `transfer_tuning`：

```yaml
transfer_tuning:
  streams: auto        # auto | 1-8
  chunk_mib: 1
  window_mib: 64
  udp_buffer_mib: 16
  slice_size_mib: 4
  prefetch: 16
  keepalive_secs: 10
  idle_timeout_secs: 30
```

理由：调优是"一次设定、反复使用"的偏好，每次都敲 6 个参数不现实；且 CLI 与 GUI
可共享同一份配置。命令行显式传入时**覆盖**配置文件，不合并。

配置文件当前用 `#[serde(default, deny_unknown_fields)]`，新增该组字段不破坏旧文件；
旧文件缺该组时取默认值（等于现状），行为不变。

## 10. `serve` 的改动

- `-l/--listen` → `-p/--port <u16>`，只接受数字；`parse_listen` 与其测试删除。
- 监听地址由 `-p` 与全局 `-4/-6` 合成：`bind_tcp(port, family)`。
  `only_v6` 显式设为 `false`（双栈）或 `true`（`-6`）。
- `HERMOD_LISTEN` → `HERMOD_PORT`（仅数字），去掉 `args.listen == ":4376"` 的魔法比较。
- UDP 反射与 TCP 同端口同族；反射失败仍降级为仅 TCP。

## 11. 实施阶段

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| 1 | `cli.rs` 重写：全局区、`ValueEnum` + `ignore_case`、`Option<T>`、`-4/-6` 互斥、`serve -p`、`rx -d/--output` | `cargo test -p hermod-cli` 全绿；`--help` 文本核对 |
| 2 | `options.rs` 加 `IpFamily` 与调优字段；`validate_for` 扩展校验 | 单测：越界/冲突组合在联网前报错 |
| 3 | `network.rs`：`bind_udp(family)`、`candidates(family)`、`bind_tcp`、`classify_link`、`UDP_BUFFER` 参数化 | 单测覆盖四族组合 |
| 4 | `signal/client.rs` 与 `signal/tls.rs` 改自建 TCP + 按族过滤 | `-4`/`-6` 下 `trust` 与 `tx` 可连 |
| 5 | `transport/quic/transport.rs`：修窗口缺陷；调优字段注入 `TransportConfig`；显式 `only_v6` | 多流吞吐不低于单流；`--adv` 值可见于 debug 日志 |
| 6 | `orchestrate.rs`：`CHUNK`/`SLICE_MIN`/`PREFETCH_QUEUE`/`MAX_STREAMS` 参数化；`TransferTuning` 接 `classify_link` | 多流切片计划随 `--slice-size` 变化 |
| 7 | 文档与残留清理：`README.md`/`README.zh-CN.md` 参数表与 0.3.x 迁移表、`NETWORK_PERFORMANCE.md`、`main.rs` 与 `cli/Cargo.toml` 注释 | 全仓 `grep` 无 `--limit-mbps`/`--transport wireguard`/`--wg-mtu` |

质量门（每阶段）：`cargo fmt --all -- --check`、`cargo test --workspace --locked`、
`cargo clippy --workspace --all-targets --locked -- -D warnings`、
`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`。

## 12. 待确认

1. `--output/-d` 只给 `rx`（`tx` 不需要），确认？
2. `compatible` 作为 `wan` 的解析别名保留（GUI 零改动），确认？
3. A 档 6 个全部开放 + B 档 `--keepalive`/`--idle-timeout` 两个，确认？
4. 调优参数是否随 `config.yaml` 持久化（§9 建议持久化），确认？

## 13. 遗留清理（core 无用代码）

调研中确认**零生产引用**、可安全删除：

- `transfer::{Metadata, encode_metadata, decode_metadata}`（协议 v2 后元数据只走 `TransferOffer`）
- `transfer::temp_path`、`orchestrate::TransferTuning` 外部无使用者
- `reflector::decode_external_address`（只被自身测试调用）
- `store::fetch_blob`、`signal::client::sender_handshake`、`quic::transport::set_initial_mtu`
- `log_error!` 宏（全仓零调用）
- 依赖 `tokio-util`（`core/Cargo.toml` 声明，源码零引用）

需确认后再动：`session::{sender_session, receiver_session}`（无参便捷入口）、
`Congestion::Reno`（QUIC 不支持，仅 GUI 字符串映射还留着）、
`transfer.rs` 顶部"对应 Go 第 N 行"的对照注释（15 处）。
