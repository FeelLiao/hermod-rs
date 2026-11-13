# CLI 参数重构与传输调优 —— 已确认实施方案

- 状态：**实现完成，Rust 自动验证通过；真实链路性能待测**。依赖策略和 Flutter 检查受本机工具环境限制，详见 §8。
- 日期：2026-11-13
- 范围：CLI 参数与输出，以及 core 的地址族选择、链路档位、QUIC 与载荷搬运预算。
- 不涉及：GUI / flutter-bridge 改动、无关 deadcode 清理、提交或发布。
- 参数来源以 [CLI 定义](../cli/src/cli.rs) 与 [预算校验](../core/src/options.rs) 为准。

## 1. 已确认边界与决策说明

1. **调优只用于本次 CLI 调用**：显式输入覆盖对应默认值，其余使用内建默认；不持久化，不从 YAML 读取调优字段，也不新增 `transfer_tuning` 配置组。服务器地址与指纹信任仍使用原有配置，并非取消全部配置文件。
2. **不做旧 `compatible` 兼容**：`LinkProfile` 只有 `Auto / Lan / Wan`，删除 `Compatible` 和解析别名。相对原案，取消了“GUI 零改动兼容”的承诺；本任务不改 GUI/bridge，旧设置或调用者若仍传 `compatible` 必须另行迁移，不能宣称无缝兼容。
3. **地址族是全局严格约束**：`-4/-6` 对 tx、rx、trust、serve 全部生效；两者互斥，不能静默跨族回退。serve 默认双栈，也允许显式限制地址族。
4. **数据面参数不静默忽略**：它们虽在全局声明，但只适用于 tx/rx；serve/trust 显式传入任意一项即错误，包括显式传默认值或 `auto`。
5. **接收端决定实际并发**：rx 的 streams/profile 选择载荷形态并开启数据流；tx 参数只是本端资源预算，不能强迫 rx 使用相同并发。
6. **连接窗暂按单流窗 × 8**：给对端合法的最多八条数据流预留流控预算，不按本端 streams 缩小。相对原案，不采用“64 MiB 总窗必然损失大半吞吐”的论断，也不声称此次修改已经提高性能；需要实测。
7. payload v3 保持不变；不引入 0-RTT，不开放 `--adv`、`--initial-rtt`、`--ack-delay`。无关 deadcode 清理暂缓。

## 2. 命令与全局参数

```text
hermod [GLOBAL] <tx|rx|trust|serve>
```

全局参数可放在子命令前后。枚举值大小写不敏感；大小单位后缀规则见 §4。

| 参数 | 短选项 | 默认 | 取值 / 语义 |
| --- | --- | --- | --- |
| `--verbose` | — | none | none/error/warning/info/debug |
| `--quiet` | -q | off | 压制状态输出，错误仍显示 |
| `--ipv4` | -4 | off | 仅 IPv4 |
| `--ipv6` | -6 | off | 仅 IPv6 |
| `--mtu` | -m | auto | auto 或 1200–1452 字节的 QUIC UDP 载荷上限；auto 从 1200 探测到 1452 |
| `--congestion` | -c | bbr | auto/bbr/cubic/newreno；auto 解析为 BBR |
| `--profile` | -P | auto | auto/lan/wan，见 §5 |

`--mtu`、`--congestion`、`--profile` 与 §4 的八项均属于只允许 tx/rx 使用的数据面参数。字段保留 `Option<T>` 以区分未输入与显式输入；默认值只在构建本次会话选项时应用。

| 子命令 | 参数 | 默认 / 语义 |
| --- | --- | --- |
| tx | `<INPUT>` | 文件路径、文本或 `-`（stdin） |
| rx | `<CODE>` | 完整传输码 |
| rx | `-d/--output <DIR>` | `.`；接收目录 |
| trust | `<SERVER>` | URL 或 host:port；成功验证后选为唯一信任服务器 |
| trust | `--fingerprint <HEX>` | 期望的 SPKI SHA-256 指纹 |
| serve | `-p/--port <PORT>` | 数字 u16；CLI > HERMOD_PORT > 4376 |
| serve | `-T/--ttl` | 600 秒；通道 TTL 与服务空闲超时，不是数据面 idle-timeout |
| serve | `--rate-limit` / `--rate-burst` | 每 IP 前缀令牌桶速率 5 / 容量 15 |
| serve | `--max-blobs-per-channel` | 10 |
| serve | `--max-cpace-failures` | 3 |
| serve | `--max-channels-per-ip` | 100 |

serve 端口只接受 0–65535 的数字，不接受 host:port；CLI 显式端口优先于环境变量，即使环境变量无效。未传 CLI 端口时，无效或空的 HERMOD_PORT 报错。

## 3. 删除与迁移

| 删除项 | 当前用法 |
| --- | --- |
| tx/rx `-s/--server` | 先运行 `trust SERVER --fingerprint HEX`，会话使用已选中的信任服务器 |
| tx `-w/--words` | CLI 使用固定三词传输码 |
| `--profile compatible` | 按需求改用 wan；无别名，无自动兼容承诺 |
| rx `--destination` | `-d/--output` |
| serve `-l/--listen`、HERMOD_LISTEN | `-p/--port`、HERMOD_PORT；地址族由 -4/-6 决定 |
| 早已移除的 `--transport`、`--wg-mtu`、`--quic-mtu`、`--quic-cc`、`--limit-mbps` | 仅 QUIC，使用统一 mtu/congestion；当前没有载荷限速参数 |

serve 是实际可用的信令/NAT 辅助服务命令，不能再写成“已删除的占位命令”。历史 CHANGELOG 保留当时的事实，不做全仓关键字盲删。

## 4. 八项调优参数：A6 + B2

所有范围含端点，联网前校验，越界报错而非 clamp。大小仅接受**纯字节整数**或**整数 + KiB/MiB**（区分后缀大小写），例如 `1048576`、`1024KiB`、`1MiB`；不接受小数、MB、空格、正负号或溢出值。

| 分组 | 参数 | 默认 | 范围 | 实施语义 |
| --- | --- | --- | --- | --- |
| A | `--streams` | auto | auto 或 1–8 | rx 决定实际并发；仍受对端可 seek 能力与小文件闸门约束 |
| A | `--chunk` | 1MiB | 256KiB–4MiB | 本端搬运块大小，不是协议帧大小 |
| A | `--window` | 64MiB | 8MiB–256MiB | 单流接收窗；连接接收/发送窗暂为 window × 8 |
| A | `--udp-buffer` | 16MiB | 1MiB–64MiB | 请求的 UDP 收发 socket 缓冲；实际受 OS 限制 |
| A | `--slice-size` | 4MiB | 4MiB–64MiB | rx 切片计划的目标大小，不等于 chunk |
| A | `--prefetch` | 16 | 4–64 | 本端预读队列深度 |
| B | `--keepalive` | 10 秒 | 0–60 秒 | 0 关闭 QUIC keepalive |
| B | `--idle-timeout` | 30 秒 | 10–300 秒 | QUIC idle timeout 与 CLI 载荷无进展看门狗预算 |

idle-timeout 必须严格大于 keepalive。握手/配对预算独立，不随此参数变化；不能把它当作建立连接的总超时。

### 4.1 资源与内存 tradeoffs

- 连接窗 ×8 默认是 512MiB，最大 2GiB。这是流控/发送预算，不意味着启动即分配同量内存，但允许更多在途/缓冲数据，会增加压力；控制流还使用独立的协议开流额度。
- chunk × prefetch 的单队列载荷预算默认约 16MiB，范围组合约 1–256MiB；另有活动块、QUIC 缓冲、多流任务、文件和 OS page cache，不能把乘积当作进程总内存上限。
- tx 必须接受合法 rx 发起的最多八条数据流；本端 `--streams 1` 不得造成对端八流开流饥饿，也不意味着限制对端并发。rx 的显式并发仍会被可用切片数、文件尺寸和源能力限制。
- 增大 udp-buffer 会增加内核内存请求，可能被 OS 限制或拒绝；应记录实际结果，不保证请求值完全生效。
- slice-size 改变任务粒度、磁盘访问及按序读回哈希的开销；较大值不是必然更快，也不是一次分配同尺寸重组内存。
- 同时增大窗口、chunk、prefetch 与并发会叠加内存/调度压力。默认或更大预算是否提升吞吐尚未验证。

## 5. 链路档位与并发所有权

| 档位 | 自动并发选择 | 初始 RTT |
| --- | --- | --- |
| lan | 目标最多八流 | 25ms |
| wan | 单流流水线 | 100ms |
| auto | 按实际对端地址分类；私有 IPv4、链路本地、回环、IPv6 ULA 为 LAN，其余为 WAN；有有效 RTT 时再要求 ≤25ms 才选 LAN 多流 | 按地址类型预判 |

IPv4-mapped IPv6 按原 IPv4 分类。地址分类是链路启发式，不是“同一物理局域网”的证明。实际并发由接收端解析，显式 rx streams 优先于档位的自动并发；对端不支持 seek 或文件过小仍为单流。文件切片计划与 `MODE_SINGLE/MODE_MULTI` 沿用 payload v3，不要求两端 chunk、window、prefetch 等本地预算一致。

## 6. 严格地址族与服务监听

- tx/rx：UDP 绑定、候选发布、对端候选筛选、信令 DNS/TCP 连接遵守同一地址族限制。
- trust：证书/SPKI 抓取同样按族筛选 DNS/TCP 地址。
- serve：默认显式双栈；-4 仅 `0.0.0.0`，-6 仅 `[::]` 且 only-v6。UDP 反射与 TCP 使用同端口同族；反射失败时仍允许仅 TCP 服务。
- 显式族过滤后没有可用地址即清晰报错，不跨族重试。未指定时才使用自动双栈/可用回退路径。

## 7. 不开放与暂缓事项

不开放 `--adv`、initial-rtt、ack-delay、丢包判定阈值、PMTUD 间隔、GSO、uni-stream 上限等进阶选项。不引入 0-RTT；当前会话使用 CPace 配对与每会话临时证书，不建立恢复协议。payload v3 和安全校验不变，不恢复 HTTP/WireGuard 或旧客户端兼容路径。

无关 deadcode、旧注释与依赖清理暂缓；不能仅凭搜索“无引用”就扩展本次改动范围。

## 8. 实施与验收记录

2026-11-13，Windows x64 / Rust 1.99.1 验证记录：

- 通过 `cargo fmt --all -- --check`。
- 通过 `cargo check --workspace --all-targets --locked`。
- 通过 `cargo clippy --workspace --all-targets --locked -- -D warnings`。
- 提交前重新通过 `cargo test --workspace --locked`：139 项通过，1 项吞吐基准按既定设置忽略，0 失败。用户确认保留删除编排私有单元测试模块的当前状态，因此比上轮验证减少 6 项测试；公开 API 集成测试保留。
- 通过 `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`、手写 unsafe 检查及 CLI release 构建。
- 实际覆盖双族 TCP/UDP、TLS SNI/SPKI、真实信令服务、CLI 进程契约，以及不同 chunk/切片、2/8 并发的 48 MiB QUIC 传输。现有 CLI/桥接层默认传输互通通过，不代表旧 `compatible` 设置继续可用。
- `cargo deny check` 未执行成功：本机未安装 cargo-deny。
- Flutter 质量门未能启动：FVM 报 Git 不可用及 `CreateFile failed 5`；本次未修改 GUI/bridge。
- 未做真实 LAN/WAN 的修改前后性能对照，不宣称吞吐提升，也不把本机回环正确性测试当作性能验收。

以下为实现覆盖范围与后续验收项：

- CLI：默认路径、大小写枚举、八项边界/单位/溢出、互斥地址族、serve/trust 显式数据面参数错误。
- 迁移：旧参数和 compatible 被拒绝，rx output 与 serve port 优先级正确。
- 网络：四命令在 -4/-6 下严格筛选，双栈服务与同族 UDP 反射、空候选错误。
- 数据面：rx 1–8 流实际生效；tx/rx 预算不一致不阻塞；payload v3、摘要、取消和临时文件行为保持。
- 资源：日志可核对有效预算和 connection_window=window×8；idle-timeout 覆盖 QUIC 与 CLI 看门狗，握手预算独立。
- 性能：后续比较吞吐、CPU、内存与延迟；没有结果前不写“提升”“必然退化”或“不低于单流”的保证。
- 文档：仅更新本设计、双语 README、NETWORK_PERFORMANCE 与 CHANGELOG；保留历史版本条目。

仓库质量门仍按 [AGENTS](../AGENTS.md) 执行：fmt、workspace tests、clippy、rustdoc 与依赖策略检查。最终完成状态由用户验证后确认。
