# 设备 ID 配对与按 ID 直传 —— 设计文档

- 状态：草案（未实现）
- 日期：2026-11-08
- 范围：`core`、信令服务器（hermod-go）协议扩展；CLI/GUI 仅列衔接点，本轮不实现
- 备注：本文写作时数据面还有 WireGuard；现已移除（QUIC-only）。文中提到
  WireGuard/`TransportKind::Quic` 与 "另一实现" 的段落请按 QUIC-only 现状阅读，
  流程本身不受影响。
- 关联：`AGENTS.md` 架构约定（逻辑只住 core）、`core/src/session.rs`（现有会话编排）

---

## 1. 背景与动机

当前两台设备的互通依赖"传输码"的人工同步：

```
发方生成  "4660-acid-acorn-acre"  → 人肉递给收方 → 收方输入 → 会合
```

问题：每次传输都要念/粘一次码，对同一台设备反复重复。本设计为每台设备引入
一个唯一、可验证的 **设备 ID（device_id）**，配对一次之后，收方既可以通过
传输码接收（原路径保留），也可以通过设备 ID 直接发起接收。

## 2. 目标与非目标

### 目标

1. 每台设备拥有一个**长期**的 `device_id`，内部绑定一台长期密钥（可验证、防冒充），对外显示为随机样式的 32 位 hex（与收件箱条目 ID 同风格）。
2. **配对**复用现有传输码流程：第一次正常传一次文件，握手成功时两端交换"身份证"并各自记账（对方 ID、公钥、配对口令）。
3. 之后每次传输：**发方仍生成传输码**（保留原体验与兼容性），但等待状态同时监听
   两扇门——对方可用传输码 join（原路径），或用发方 device_id 呼叫（新路径）。
4. 信任判断 100% 端到端：信令服务器是纯路由器，不持久化任何设备信息，
   不做任何身份验证。
5. core 只提供能力与存储原语；接收策略（自动接/弹窗）、备注名、国际化等留给 CLI/GUI。

### 非目标（v0 不做）

- TURN/STUN 类中继兜底（打洞失败维持现状：明确报错）。
- 纯无码模式（"选 ID 就传"可作为后续版本的收方侧开关，属于本设计的严格子集扩展）。
- contacts.json / device.json 的 OS 级加密（DPAPI/Keychain），列为后续加固。
- 双向并发传输（一台设备同时服务多个呼叫）。

## 3. 核心概念

| 概念 | 定义 | 放置 |
|---|---|---|
| 长期身份 | 每台设备一个 P-256 ECDSA 密钥对，只存本机，长期使用 | core `identity` 模块 |
| `device_id` | `hex(SHA-256(公钥)的前 16 字节)`，32 个小写 hex 字符 | 由公钥派生，非独立存储 |
| 身份证（`DeviceAnnounce`） | `{id, pubkey, sig}`；`sig` = 对该字段 + 会话绑定材料的 ECDSA 签名。随加密端点信封交换 | core `identity` 模块 |
| 配对（Pairing） | 两台设备互信状态：双方 contacts 各持一条记录 `{peer_id, peer_pubkey, pairing_secret, name, added_at, last_seen}` | core `contacts` 模块 |
| `pairing_secret` | `HKDF-SHA256(ikm = 配对那次会话的 K, salt = channel_id_be16, info = "hermod-pairing-v1")`，32 字节；之后每对该设备的 CPace 口令 | 会话派生、contacts 存储 |
| 双门等待 | 发方选完文件后：`allocate(C1) + register(device_id)`，同时等待 `join(C1)` 或 `incoming` 呼叫；一次只接待一个客人 | core `signal` / `session` |

**信任的正确性说明**：`device_id` 是公钥的指纹，任何对 ID 的冒充都要在握手后
出示对应私钥的签名才能通过；信令路由错误/作弊只会失败，不会降级。

## 4. 总体流程

### 4.1 配对（每对设备一次，= 现有流程 + 记账）

```text
A(发): 选文件 → 生成传输码 C1+词 ──人肉递码──► B(收): 输入传输码
       allocate(C1)  ⇄  join(C1)               [现状，零改动]
       CPace(blob1/blob2)                      ⇒ K        [现状，零改动]
       blob3: A 端点+offer+【DeviceAnnounce_A】 AEAD(K)     [+身份证，可选字段]
       blob4: B 端点+【DeviceAnnounce_B】       AEAD(K)
       互验签名（ID↔公钥↔签发私钥一致）                    [+新增]
       ⇒ 双方 contacts.upsert(对方ID, 公钥, HKDF(K, "hermod-pairing-v1"))
       打洞直连 → HTTP + SHA-256 对账（与今天完全一致）     [现状，零改动]
```

配对记账是**自动的**：传输码本身已是"两个人当面共同完成一次传输"的人类确认，
无需再弹"是否保存对方？"（记录若冗余可由上层 UI 提供删除/清理入口）。

### 4.2 之后每次（传输码保留，设备 ID 并行）

```text
A(发): 选文件 → 生成并展示传输码(C1) → allocate(C1) + register(A_id)
        【双门等待】一次只接待一个客人
B(收): 两选一
   门1  输入传输码 → join(C1)                      [现状路径]
   门2  点选/输入 A 的 device_id → 查 contacts → call{A_id, C2}（C2 随机生成）
        └─ A 收到 incoming → 扩展：闲 → 接 join(C2)；忙 → decline
        ⇒ CPace（口令 = contacts.secret）⇒ K → 信封（身份证三重校验）
门2 被用 → 展示中的 C1 传输码立即作废（UI 提示"已通过设备 ID 接上"）
       → 打洞直连 → HTTP + SHA-256 对账（与今天完全一致）
```

### 4.3 门 2 的三重校验（任何一层失败 = 明确报错，不降级）

1. contacts 里存在该 `peer_id`（否则拿不出 CPace 口令，直接到不了握手）。
2. 验证 `DeviceAnnounce` 签名（冒充者无 A 的长期私钥）。
3. 宣告中的 `id` 必须等于"我呼叫的那个 `device_id`"（防张三顶替李四接会话）。

## 5. core 层改动清单

Everything 一览；【现有】= 一行不改。

```text
core/src/
├── identity.rs             ★ 新增   本机长期身份 + 身份证签验
├── contacts.rs             ★ 新增   配对台账（读写原语，不做策略）
├── signal/protocol.rs      ✎ 小改   +4 个消息类型
├── signal/client.rs        ✎ 中改   组合握手函数 + 3 个新动作
├── session.rs              ✎ 中改   Rendezvous 抽象 + 新入口 + announce 夹带
├── crypto/*                【现有】 CPace/KDF/AEAD/wordlist 全部不动
├── transport/*             【现有】 WG/QUIC/bundle/iface/network 全部不动
├── orchestrate.rs          【现有】 传输编排不动
└── options.rs/transfer.rs  【现有】 不动
```

### 5.1 identity.rs（新增 —— 本节签名均为草案）

```rust
pub struct Identity { /* P-256 长期私钥 + 派生 id */ }

/// config 同目录的 device.json；0600；首次运行的原子：不存在则生成
pub fn load_or_create(config_dir: &Path) -> io::Result<Identity>;

pub fn device_id(&self) -> [u8; 16];                    // 显示：32 位小写 hex
pub fn public_key(&self) -> [u8; 32];                   // SEC1 压缩点

/// 身份证：绑定了本会话材料，防止跨会话重放
pub fn sign(&self, announce: &DeviceAnnounce, cid: u16, session_k: &[u8; 32]) -> Vec<u8>;
pub fn verify(announce: &DeviceAnnounce, cid: u16, peer_pubkey: &[u8; 32],
              session_k: &[u8; 32]) -> Result<(), AnnounceError>;
```

`DeviceAnnounce` 夹带方式复用 [transfer_offer](../../core/src/session.rs) 的
"端点 JSON 可选字段"兼容模式（`serde` 派生默认容忍未知字段；旧端忽略）：

```json
{ "version": 1, "udp_port":…, "ipv6_candidates":[…],
  "transfer_offer": {…},
  "device_announce": { "id":"32hex", "pubkey":"<b64>", "sig":"<b64>" } }
```

`sig` 的被签材料（草案）： `SHA-256("hermod-device-announce-v1" ‖ id ‖ pubkey ‖ channel_id_be ‖ K)`。
它同时在 AEAD(K) 信封内（双重保护）。

### 5.2 contacts.rs（新增）

```rust
pub struct Pairing {
    pub peer_id:     [u8; 16],
    pub peer_pubkey: [u8; 32],
    pub secret:      [u8; 32],     // pairing_secret，之后门 2 的 CPace 口令
    pub name:        Option<String>,   // 上层填备注；core 不做 i18n
    pub added_at:    u64,
    pub last_seen:   u64,
}

pub struct Contacts { /* contacts.json + Mutex + 原子写(临时文件+rename) */ }
pub fn load_or_create(config_dir: &Path) -> io::Result<Contacts>;
pub fn get(&self, peer_id: &[u8; 16]) -> Option<Pairing>;
pub fn upsert(&self, pairing: Pairing) -> Result<()>;
pub fn remove(&self, peer_id: &[u8; 16]) -> Result<()>;
pub fn list(&self) -> Vec<Pairing>;
```

文件格式草案：`{ "version": 1, "pairings": [ {…} ] }`，ID/secret 均小写 hex 字符串。

### 5.3 signal 扩展

protocol.rs 新增（JSON payload，沿用 base64 blob 约定与 64 KiB 上限）：

```text
register { device_id }                                  收发双方在等待时挂"我在"牌
call     { caller_id, target_id, channel_id }           发起呼叫（caller 自选新信道号）
incoming { caller_id, channel_id }                      server → 被呼叫方
decline  { channel_id, reason: busy|rejected|offline }  接受与否的明确回执
```

client.rs：`sender_handshake / receiver_handshake` 合并为按参数分流的组合函数，
旧签名退化为薄包装（零回归）：

```rust
pub async fn rendezvous_handshake(
    ws, channel_id, password,
    local_role: Role,        // = 文件方向角色：tx→Sender / rx→Receiver（不变）
    initiator: bool,         // = 信令角色：码路 tx 为 true；ID 路呼叫者(收方)为 true
) -> Result<([u8; 32], ObservedAddrs), ClientError>;

pub async fn register_device(&mut ws, &Identity) -> Result<(), ClientError>;
pub async fn call_device(&mut ws, target_id, device_id, channel_id) -> Result<(), ClientError>;
pub async fn wait_incoming(&mut ws) -> Result<Incoming, ClientError>;  // Incoming{caller_id, channel_id}
```

### 5.4 session.rs

```rust
pub enum Rendezvous {
    Code(String),                        // 门 1：现有路径（parse_transfer_code）
    Peer { peer_id: [u8;16], secret: [u8;32] },   // 门 2：查 contacts 得到
}

/// 发方：双门等待（会话内部 select!：wait_ready(C1) 或 incoming）
pub async fn sender_session_with_ready_dual_door(config, code, opts, identity,
                                                 on_ready) -> Result<TxSession, SessionError>;

/// 收方门 2：按 ID 主动呼叫
pub async fn receiver_session_by_device(config, peer_id, opts) -> Result<RxSession, SessionError>;

/// 现有入口签名与行为不变（= Rendezvous::Code 路径的包装），全部零回归
pub async fn sender_session_with_ready(...) /* 原样 */
pub async fn receiver_session_with(...)     /* 原样 */

pub struct TxSession { /* conn, token, */ pub paired: Option<NewPairing> }
pub struct RxSession { /* conn, token, offer, */ pub paired: Option<NewPairing> }
pub struct NewPairing   { /* peer_id, peer_pubkey, secret: 配对那次非空，供上层/本层自动 upsert */ }
```

门 2 呼叫到来时使用既有中继：`wait_ready` 与 `next_incoming` 由发方会话
内部 `select!` 竞争，先到先得，胜出后码（若未被用）作废。

## 6. 信令服务器（hermod-go）改动

服务器**不持久化、不验证、不做任何信任判断**；新增的全部为易失映射与转发：

| 消息 | 服务器行为 |
|---|---|
| `register { device_id }` | 内存表 `device_id → 本 WS`；同一 ID 重复注册 → 新连接顶替旧连接；连接断开 → 注销 |
| `call { caller_id, target_id, channel_id }` | 目标在线 → 给目标发 `incoming`，回呼叫方 `ok`；离线 → `error("peer offline")`。接受进信道（成员 = 呼叫方） |
| `incoming { caller_id, channel_id }` | 纯转发 |
| `decline { channel_id, reason }` | 纯转发；呼叫方收到明确错误（busy/拒绝/不在线） |
| 既有 `allocate/join/blob/ok/ready/error` | **零改动**（信道表 + blob 中继原样） |

一般性建议：对 `call` 做每连接限速（骚扰缓解），对消息维持现有 64 KiB 上限。

### 兼容与发布次序

1. **先升服务器**：全部新增行为对旧客户端透明（旧客户端不认识新消息即可）。
2. 再发新版客户端：旧客户端 + 新服务器 = 完全正常（只少门 2）。
3. 新客户端 + 旧服务器：门 2 应优雅报"服务器版本过旧，暂不支持按 ID 直传"，
   门 1 照常。协议消息类型建议新客户端先探测（或依赖 `error("unknown type")` 提示）。
4. `device_announce` 可选字段：旧 Rust 端点解析默认容忍未知字段（serde 派生未声明
   `deny_unknown_fields`）；Go 端点解析的容忍行为发布前需对拍确认。

## 7. 安全模型

### 7.1 谁看得到什么

| 观察者 | 可见 | 不可见 |
|---|---|---|
| 信令服务器 | channel_id、两个 65B 公钥点、AES-GCM 密文信封、双方出口 v6、在线状态（device_id 在线与否） | 口令词、K、端点信息、文件内容 |
| 链路窃听 | WG Noise / QUIC TLS 下的密文 | 一切 |
| 恶意 peer 冒充 device_id | 可无限发起 call（骚扰面） | 完成握手（无 pairing_secret）或通过身份证验签（无私钥） |
| 拿到收方 contacts.json 的攻击者 | 持某对设备的 pairing_secret 可用该次口令完成握手 | 过不了身份证验签；无法从中提取任一方的长期私钥 |

### 7.2 已知取舍（接受）

- 门 1 口令每次新鲜（熵新鲜 + 独立失效）；门 2 复用长期 secret（高熵 + PAKE 适合长期口令），
  换取免念码的便利。两者互补：日常对手机用门 2，陌生人/首次用门 1。
- contacts.json / device.json 的落盘仅 0600 权限（进程内无额外加密）；设备失窃 = 身份与
  台账同时丢失。缓解路线（后续版本）：OS Keychain/DPAPI 锁 `device.json`、
  或"删除设备"升级为双向协议消息。
- 信任是**单向可撤回**的：A 删除 B 后，B 仍可发起呼叫，A 侧表现为"未知设备"，
  默认不自动接受。是否静默忽略/仅记录日志由上层策略决定。
- `call` 可被目标 ID 冒名占用线路（服务器不验签）；后果限于按钮噪音，由限速 +
  decline 缓解。

## 8. 存储

| 文件 | 内容 | 权限/写入 | 拥有者 |
|---|---|---|---|
| `device.json` | 本机私钥、公钥、派生 `device_id` | 0600；临时文件 + 原子 rename | core |
| `contacts.json` | 配对台账（peer_id/公钥/secret/备注/时间） | 0600；临时文件 + 原子 rename | core |
| `config.yaml` | SPKI 信任等 | 现状不变 | core（既有） |
| UI 偏好 | 自动接受开关、界面文案 | UI 自行存储 | CLI/GUI |

位置：与现有 `config::dir()`/应用支持目录同一根（跨平台语义不变）。

## 9. CLI / GUI 衔接（本轮不实现，仅列接口）

- CLI：`hermod rx` 的参数位从"传输码"扩为"传输码或 --from <device-id>"；
  `hermod tx` 不变（双门无需 CLI 用户感知）。
- GUI：收文件页输入框在"传输码"之外提供"已配对设备"下拉；发文件页文本
  "等待接收…（可选：也可以告诉我我的 ID）"；
  新增"设备"页：台账列表 + 删除/改名（调用 contacts 原语）。
- 二者一律不得实现：身份签验、口令派生、台账文件操作 —— 全部经 core API。

## 10. 实施顺序与测试

实施顺序（core 内部依赖顺序）：

1. `identity.rs`（生成/签验 + 向量测试）
2. `contacts.rs`（读写/损坏容错/权限）
3. `signal` 4 消息 + `rendezvous_handshake(role, initiator)`（本地模拟中继）
4. `Rendezvous::Peer` 新会话入口 + 双门 `select!` + 传输码路径回归
5. 集成测试：两假设备 + 本地假服务器

测试要点（沿用现有测试风格，如 signal::client 的本地 relay 测试）：

- 配对：码传输 + 身份证互验 + 双方各自 upsert 幂等
- 门 2 全流程：call → incoming → join → CPace(secret) → 直连 → SHA-256 对账通过
- 双门竞争：先到先得、第二个敲门者收 `busy`；门 2 建立后展示码作废
- 负路径：离线呼叫、未配对呼叫（拒/确认）、签名不符、ID↔呼叫目标不一致、
  端点信封篡改（现有 AEAD 测试延伸）
- 回归：现有全部测试零改动通过（[interop.rs](../../cli/tests/interop.rs) 等）

## 11. 决策记录

| # | 决策 | 理由 |
|---|---|---|
| D1 | 配对制（先配对后直传），不引入无码直连 | 安全性与现有 CPace 口令模型一致，改面最小 |
| D2 | 发方继续生成/展示传输码，等待状态双门 | 渐进兼容：旧路径字节级不变，新路径并行演进 |
| D3 | `device_id` 绑长期密钥（指纹），显示为 hex | 可验证、防冒充；显示友好且与收件箱 ID 风格同类 |
| D4 | 身份/台账/签验全在 core，策略在 CLI/GUI | 两前端一致执行同一验证规则；与 AGENTS.md 界线一致 |
| D5 | 配对成功后 core 自动记账 | 传输码本身即人类确认；CLI 也自动获得完整体验 |
| D6 | 服务器不做验证、不持久化设备数据 | 信任完全端到端；服务器保持哑路由器 |
| D7 | 一次传输只接待一个客人；`decline` 回执 | 避免单 WS 双会话状态机；呼叫方获得明确错误 |

—— 完 ——
