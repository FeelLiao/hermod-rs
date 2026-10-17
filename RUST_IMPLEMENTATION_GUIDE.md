# Hermod-rs Rust 实现指南（给 Rust 初学者的逐阶段动手文档）

> 目的：给**不懂 Go、也刚开始学 Rust** 的你一份"照着做"的施工指南。它把 `DEVELOPMENT.md`（设计文档）
> 和 `GO_ARCHITECTURE.md`（Go 版是怎么实现的）翻译成 **Rust 里每个阶段具体写什么、怎么写**。
>
> 三个文档怎么配合用：
> - **DEVELOPMENT.md**：决策（D1–D17）、详细设计（§3）、里程碑任务表（§8）。是"做什么"的最终依据。
> - **GO_ARCHITECTURE.md**：Go 版实际行为，是"参考实现"。Rust 版照着它的**行为**重写（不是照抄代码）。
> - **本文档**：把上面两个翻译成"我该在哪个文件里写什么 Rust 代码"。
>
> 阅读顺序建议：先读本文档第 1 节（Rust 速览）→ 然后按 M1–M6 逐段做，每段先看"要做什么"
> → 再看"怎么写" → 完成"验证"。
>
> 依据：DEVELOPMENT.md（2026-10-11 定稿）、hermod-go v1.2.0 源码、hermod-rs 现有骨架
> （`hermod-core` + `hermod-cli` 的 .rs 文件已按 §3.1 建好空壳）。

---

## 1. Rust 初学者速览：读懂下面代码前先花 20 分钟看这节

以下 10 条是写这个项目**最低限度**需要懂的 Rust。每条只讲本项目会用到的最小面。

### 1.1 Cargo 与 crate（包）

- Rust 的构建工具叫 **Cargo**。一个"包"叫 **crate**。
- 每个 crate 的配置在它的 `Cargo.toml` 里：`[dependencies]` 声明依赖哪些第三方库（类似
  Go 的 go.mod / go.sum）。用 `cargo add <名字>` 或手写。
- 本项目是**一个 Cargo workspace**（`/home/feelliao/hermod-rs/Cargo.toml` 里 `members` 列了两个
  crate）：`hermod-core`（库，所有逻辑）和 `hermod-cli`（可执行程序，薄壳）。clap 参数在 CLI，
  逻辑在 core —— 这是决策 D17。

### 1.2 module 与 pub（模块与可见性）

- 一个 `.rs` 文件是一个 **module**。`src/crypto/mod.rs` 里 `pub mod cpace;` 表示"有一个子模块
  cpace，公开"。`lib.rs` 是库的入口，集中声明所有 `pub mod`。
- `pub` = 对外可见；`pub fn` 对外可调；`pub struct` 的字段默认私有，要 `pub field` 才可读写。
- 模块路径：`hermod_core::crypto::cpace::xxx`。

### 1.3 Result / Option 与 `?`（错误处理）

- 可能失败的函数返回 `Result<T, E>`（成功 `Ok(T)`，失败 `Err(E)`）；可能没有值的返回 `Option<T>`
  （`Some(v)` / `None`）。
- `?` 运算符 = "如果出错，立刻把错误返回给上层"。写 `let x = f()?;` 比写一堆 if 简单得多。
- 本项目用两个惯用错误 crate：**thiserror**（定义自己的错误类型，适合库 hermod-core）和
  **anyhow**（程序里快速包一层错误消息，适合 CLI）。示例：

```rust
// thiserror：在 hermod-core 里定义统一错误类型（error.rs）
#[derive(thiserror::Error, Debug)]
pub enum HermodError {
    #[error("server certificate fingerprint mismatch: expected {expected}, got {got}")]
    FingerprintMismatch { expected: String, got: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// 用法：把任意错误转成 HermodError
fn connect() -> Result<(), HermodError> {
    let s = std::fs::read_to_string("a.txt")?; // 自动转成 HermodError::Io
    Ok(())
}
```

```rust
// anyhow：在 hermod-cli 里直接用
fn main() -> anyhow::Result<()> {
    let cfg = hermod_core::config::Config::load()?; // 任何错误都能 ? 出去
    Ok(())
}
```

### 1.4 struct / enum / match（自定义类型）

- `struct` 是"带名字的一堆字段"；`enum` 是"几种情况之一"。match 是"按情况分支"。
- 本项目里到处都是它们：`enum MsgType { Allocate, Join, ... }`、`struct Message {...}`。

```rust
// 示意：信令消息（M2 会用到，参照 Go server.Message）
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub type_: MsgType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

### 1.5 impl 与 trait（方法、接口）

- `impl Type { fn ... }` 给类型加方法（类似 Go 的 `func (t Type) Xxx()`）。
- **trait** 是"接口/能力"：声明一组方法，多个类型可以实现。例子：`trait AsyncRead`、`trait Digest`、
  `trait ServerCertVerifier`。本项目大量工作 = "实现某某 trait"。

```rust
// 示意：给 Message 加一个构造方法
impl Message {
    pub fn ok(channel_id: u16) -> Self {
        Message { type_: MsgType::Ok, channel_id: Some(channel_id), payload: None, error: None }
    }
}
```

### 1.6 所有权、借用、生命周期（最容易劝退，但本项目只需三条规则）

1. **一个值同一时刻只有一个"主人"**；把值传给函数 = 所有权转移（move），原变量失效。
2. 想"借用"（不转移）就用 `&`（只读借用）或 `&mut`（可写借用）。**同一时刻要么多个 `&`，
   要么一个 `&mut`，不能混**——这就是 borrow checker 报错的根源。
3. 生命周期 `'a` 是"这个引用能活多久"的标签，多数时候编译器能自己推断，你只要在函数签名里
   偶尔写 `<T: Trait + 'a>` 或 `&'a str`。

应付策略：**传引用优先**、**数据往 struct 里存、方法用 `&mut self`**、报 borrow 错时先想"是不是
可以克隆/拷贝一份，或把借用的值先取出"。这个项目量不大，卡住了改结构比硬扛借用简单。

### 1.7 async / await 与 tokio（异步）

- Go 用 goroutine + channel；Rust 用 **async/await + tokio**（一个异步运行时库）。
- `async fn` 是可暂停的函数；`.await` 是"等它完成"。tokio 提供 `tokio::net`（TCP/UDP）、
  channel、`tokio::spawn`（起一个任务，类似开 goroutine）。
- 入口习惯：`#[tokio::main]`。

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    loop {
        let (mut sock, _) = listener.accept().await?;
        tokio::spawn(async move { /* 每个连接一个任务 */ });
    }
    Ok(())
}
```

⚠️ 注意：async 世界里**不能调用阻塞函数**（比如 std 的读文件、DNS），要用 tokio 提供的版本，
否则会卡住整个运行时。本项目数据线程刻意**不用 async**（§3.8），控制面才用 tokio。

### 1.8 String vs &str（文本）

- `String` 是拥有的可变文本；`&str` 是只读的文本切片。函数参数尽量用 `&str`，返回值用 `String`。
- 拼接：`format!("{} {}", a, b)`；`s.push_str("...")`。

### 1.9 测试：`#[test]` 与 `cargo test`

- 在文件里写 `#[cfg(test)] mod tests { #[test] fn xxx() { ... } }`。`cargo test` 跑所有测试。
- 断言：`assert_eq!(a, b)`、`assert!(cond)`。本项目质量门禁 = 所有 `cargo test` 通过 +
  `cargo clippy -- -D warnings` 无警告 + `cargo fmt --check`。

### 1.10 常用命令

```bash
cargo build            # 编译（workspace 两个 crate 一起）
cargo build -p hermod-core   # 只编 core
cargo test             # 跑所有测试
cargo test -p hermod-core crypto::   # 只跑名字含 crypto:: 的测试
cargo clippy -- -D warnings   # 静态检查，警告当错误（本项目强制）
cargo fmt              # 格式化；cargo fmt --check 检查
cargo add p256         # 往当前 crate 加依赖（在哪个目录跑就加给哪个 crate）
```

> 建议：装 **rust-analyzer**（VS Code 插件），改代码时实时标错误、点一下就能看类型，对新手帮助极大。

---

## M1 —— 工程骨架与核心（对应 DEVELOPMENT.md §8 M1，2–4 天）

### 要做什么（一句话）

把"地基"搭好：能让 `cargo build` 干净通过、CPace / AEAD / 词表 / 配置 / 路径安全这些基础模块
能算对。**这一步不碰网络，全是本机计算**，是练 Rust 手感最好的阶段。

### 先学什么

`struct`/`enum`/`impl`/`trait`（§1.4–1.5）、thiserror（§1.3）、单元测试（§1.9）。

### 任务清单与写法

**M1.1 骨架（已基本完成，你只验证）**
- 现有文件：workspace 配置、两个 crate 空壳、`lib.rs` 的模块声明、统一 lint（clippy 全开 +
  `unsafe_code = "forbid"`，见 S5）。
- 你要做的：`cargo build --workspace && cargo clippy -- -D warnings` 确认干净。
- 结果：以后每写完一段代码，都跑这两个命令。**lint 是硬门禁**，不是可选。

**M1.2 CPace（`hermod-core/src/crypto/cpace.rs`、`kdf.rs`）——本阶段最难的模块**
- 依赖（加到 `hermod-core/Cargo.toml` 的 `[dependencies]`）：

```toml
[dependencies]
p256 = { version = "0.14", features = ["ecdh", "hash2curve"] }
sha2 = "0.10"
hkdf = "0.12"
rand_core = { version = "0.6", features = ["getrandom"] }
thiserror = "2"
```

- CPace 是什么（对照 GO_ARCHITECTURE.md §2.2）：双方用同一个口令（传输码的单词），各自算出一个
  P-256 曲线上的点发给对方，再用对方的点 + 自己的私钥推出同一个共享密钥 K。**要点：口令永远不上网**。
- 实现骨架（`CPaceSession` 两端同构，只差 role）：

```rust
// 示意：cpace.rs（真正的曲线运算用 p256 crate，不要自己手写大数）
pub struct CpaceSession {
    scalar: p256::ecdh::EphemeralSecret,
    pub_msg: [u8; 65],   // 0x04 || X(32) || Y(32)，未压缩点
    role: Role,
}
pub enum Role { Sender, Receiver }

/// 第一步：由口令 + 频道号生成"生成元"，再随机取一个标量，得到公开点。
pub fn init(password: &str, channel_id: u16, role: Role) -> Result<CpaceSession, HermodError> {
    let g = hash_to_curve_generator(password, channel_id)?; // p256 的 RFC 9380 SSWU
    let scalar = p256::ecdh::EphemeralSecret::random(&mut rand_core::OsRng);
    let pub_msg = /* scalar * g，编码成 65 字节 */;
    Ok(CpaceSession { scalar, pub_msg, role })
}

/// 第二步：收到对方点后完成协商，得到 32 字节 K。
pub fn finish(&self, peer_pub: &[u8; 65]) -> Result<[u8; 32], HermodError> {
    // ISK = SHA-256(iskX || pubSender || pubReceiver)，role 决定排序
    // （照抄 Go crypto.go 的 CPaceFinish 的字节排序，保证兼容）
}
```

- **关键提示**：字节格式必须与 Go 版逐字节一致（M1.6 会从 Go 测试里提取向量来对）。所以
  `cpace.rs` 里的点编码（65 字节未压缩）、`pubSender/pubReceiver` 的排序，都**照抄 Go 的
  `crypto.go`**，不要自己发明格式。
- 验证：RFC 9380 hash-to-curve 官方向量 + 从 Go 测试提取的 CPace 向量全过（M1.6 提供工具）。

**M1.3 AEAD 与词表（`crypto/aead.rs`、`crypto/wordlist.rs`）**
- 依赖：`aes-gcm = "0.11"`。
- aead.rs：封装"加密/解密 + 附加认证数据（AAD）"，对照 Go 的 `SealAAD`/`OpenAAD`——返回
  `nonce ‖ 密文 ‖ tag` 的拼接格式，AAD 不进密文但参与认证：

```rust
// 示意：aead.rs
pub fn seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).unwrap();
    let nonce = /* 随机 12 字节 */;
    let ct = cipher.encrypt(aes_gcm::Nonce::from_slice(&nonce),
                            aes_gcm::aad::Aad::from(aad), plaintext).unwrap();
    [nonce.as_slice(), ct.as_slice()].concat()
}
```

- wordlist.rs：1296 词 EFF 短词表 + 传输码生成/解析（格式 `<uint16>-word-word-word`，见
  GO_ARCHITECTURE.md M1）。词表直接照抄 Go 的 `effShortWordlist`（保证逐字节一致）。传输码生成
  用随机数 + 拒绝采样消除模偏差（照抄 Go `randomWordIndex`）。
- 验证：加解密往返正确；AAD 不匹配必须报错；词表与 Go 逐字节一致（M1.6）。

**M1.4 HTTP 头约定（`http_meta.rs`）**
- 定义隧道内 HTTP 用的头：`X-Hermod-Kind`（file|text|stream）、`Content-Disposition`、
  `Content-Length` 语义（见 DEVELOPMENT.md §3.4）。本阶段只做**编解码 + 长度上限 + 模糊测试**，
  不真正传 HTTP。
- 验证：头字段编解码往返；超过上限拒绝；用 `cargo-fuzz` 或简单随机输入保证不 panic（S4）。

**M1.5 传输辅助与配置（`transfer.rs`、`config.rs`）**
- 依赖：`serde`、`serde_json`、`serde_yaml`。
- transfer.rs：路径安全（拒绝 `../`、绝对路径、符号链接逃逸）+ 目标已存在自动加去重后缀。
  逻辑**照抄 Go 的 `pkg/transfer/transfer.go`**（`SafeDestinationPath`、`TempPath`），是纯函数，很好测。
- config.rs：读写 `~/.config/hermod/config.yaml`，**与 Go 版 schema 双向兼容**（决策 D15）：
  结构体字段名对齐 Go 的 `config.go`（`server_url`、`trusted_servers` 等）。用 serde 的
  `#[serde(rename = "server_url")]` 控制 YAML 键名。
- 验证：从 Go 版写出的配置文件能被本 Rust 版原样解析（拿一份真实 config.yaml 做测试夹具）。

**M1.6 向量提取工具（`scripts/extract-vectors.sh`）**
- 作用：跑 Go 版测试，把 CPace 中间值、词表、配置样本输出成 JSON，让 `cargo test` 能消费。
- 做法：脚本 `cd hermod-go && go test ./internal/crypto -run Xxx -json > vectors.json`，再把
  vectors.json 拷进 `hermod-core/tests/vectors/`，Rust 测试读它比对。
- 这是"M1.2/1.3 凭什么说我写对了"的裁判，**先做脚本再写实现**（测试先行精神）。

### M1 完成标准

`cargo build --workspace` 与 `cargo clippy -- -D warnings` 干净；CPace/AEAD/词表/配置/路径安全
的单测 + 向量测试全绿。

---

## M2 —— 会合（对应 §8 M2，2–4 天）

### 要做什么（一句话）

实现"连上 server → 开/加频道 → 经 server 中继 CPace 与加密清单"这一整套信令，以及**证书 pinning**
（M2.2）。做完后两个 Rust 进程能经一个真实 Go server 完成整个会合，server 还偷看不了密钥。
（"server 如何让两台设备互信"的 Rust 侧实现，行为见 GO_ARCHITECTURE.md §2。）

### 先学什么

`serde_json` 序列化、`async fn` + tokio（§1.7）、trait 实现（§1.5）、channel。

### 任务清单与写法

**M2.1 线格式（`signal/protocol.rs`）**
- 定义消息枚举与 JSON 信封，对照 Go `server.go` 的 `Message` 与 `MsgType`（allocate/join/blob/
  ok/ready/error）。Go 版的 payload 是 base64 的字节数组，serde 里 `Vec<u8>` 默认就是 base64，
  天然一致。
- 验证：`cargo test` 里写编解码往返 + **从 Go server 抓的真实报文**做 golden 测试（M1.6 延伸）。

**M2.2 客户端 + 证书 pinning（`signal/client.rs`、`signal/tls.rs`）**
- 依赖：

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "io-util", "sync"] }
tokio-tungstenite = "0.26"
rustls = "0.23"
webpki-roots = "0.26"   # 若想顺便信任公共 CA；本项目是自签，主要靠自写 verifier
```

- client.rs 用 `tokio_tungstenite::connect_async` 连 `wss://host:4376/ws`。但要先能传自定义 TLS
  配置做 pinning，因此需要 `connect_async_tls_with_config` + 自定义的 rustls `ClientConfig`。
- tls.rs 是**本阶段最容易卡住的点**：实现 rustls 的 `ServerCertVerifier` trait，在
  `verify_server_cert` 里取对端证书、算 SPKI 指纹、与配置里 pin 的指纹做**常量时间比较**。

```rust
// 示意：tls.rs —— 实现 rustls 的 ServerCertVerifier trait（细节以 rustls 0.23 API 为准）
pub struct PinnedVerifier { expected: [u8; 32] } // 存 server 公钥指纹

impl rustls::client::ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::server_verifier::ServerCertVerified, rustls::Error> {
        use subtle::ConstantTimeEq; // 让 [u8] 有 .ct_eq() 方法（常量时间比较）
        let got = spki_fingerprint(end_entity)?; // SHA-256(SPKI)
        if got.as_slice().ct_eq(self.expected.as_slice()) == subtle::Choice::from(1) {
            Ok(rustls::client::server_verifier::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("server fingerprint mismatch".into()))
        }
    }
}
```

- 提示：rustls 0.23 的 verifier 接口细节较多，**先读 rustls 自带示例
  （`examples/src/bin/tlsclient-mio.rs` 里的 custom verifier）**，照抄结构再改。别裸写。
- 验证：能连上真实 Go server；pin 对能连、pin 错拒绝；证书变了拒绝（M2.2 验收）。

**M2.3 经中继的 PAKE（`signal/client.rs` + `crypto/cpace.rs`）**
- 串起流程：allocate → 打印传输码 → 等 ready → 发 blob1 → 收 blob2 → CPaceFinish → 派生混合密钥
  （对照 GO_ARCHITECTURE.md §2.2 的 blob 表和混合密钥公式）。
- 注意：**Rust 版是 IPv6 优先，但会合协议不变**——blob 结构、消息类型照抄 Go，才能连 Go server。
- 验证：两个 Rust 进程经真实 Go server 得到**同一个 K**，且 server 抓包也算不出（M2.3 验收）。

**M2.4 Bundle 加密（`tunnel/session.rs`）**
- 定义 `EndpointBundle`（对照 Go `handshake.go`）：本地 IPv6 候选、绑定的 UDP 端口、WG 公钥（Rust
  版改成公布 WireGuard 公钥，见 M3）、一次性凭证指纹、版本。
- 用 M1.3 的 aead seal/open，AAD = 频道号（2 字节大端）。加解密往返测试 + AAD 不匹配拒绝测试。
- 提示：Rust 版清单内容与 Go 不同（Go 是临时 TLS 证书指纹，Rust 是 WG 公钥），因为数据面不同
  ——**清单格式跟随 DEVELOPMENT.md §3.2 步骤 5，不跟随 Go**。

**M2.5 IPv6 地址枚举（`tunnel/addr.rs`）**
- 枚举本机非回环 IPv6 地址，排序：全局单播 → ULA → 链路本地（带 scope id）。**不枚举 IPv4**（D7）。
- 提示：跨平台枚举地址，可考虑 `if-addrs` crate 或直接调平台 API；先用本机 stub 接口列表做单测。
- server 观测地址**仅作提示**且必须是 IPv6 才采纳（§3.5）。

**M2.6 Rust server（延后，可先不做）**
- DEVELOPMENT.md 决策 D13：v1 直接用 Go server，Rust server 延后。所以 `signal/server.rs`、
  `store.rs`、`ratelimit.rs` 本阶段**只需要能通过 clippy 的空实现**，不要花时间。等 M4 后再回来填。

### M2 完成标准

两个 Rust 进程经真实 Go server 完成 allocate/join/blob 全流程、得到同一把 K；pinning 正反向都正确；
`cargo test` + clippy 干净。

---

## M3 —— WireGuard 直连（对应 §8 M3，3–5 天）

### 要做什么（一句话）

**抛开 server**，让两端各自建一个进程内 WireGuard 会话，用 M2 得到的密钥派生 PSK，互相握手后
形成一条点对点加密隧道。这是 Rust 版和 Go 版**差别最大的阶段**（Go 是打洞 + QUIC；Rust 是
WireGuard + IPv6 直连，见 GO_ARCHITECTURE.md M3 的⚠️对比框）。

### 先学什么

`&mut self` 状态机、`Option<[u8; 32]>` 这类密钥类型、`subtle` 常量时间比较、简单的无锁/单线程循环。

### 任务清单与写法

**M3.1 密钥与 PSK（`tunnel/wg/keys.rs`）**
- 依赖：

```toml
[dependencies]
x25519-dalek = { version = "3", features = ["static_secrets", "reusable_secrets"] }
hkdf = "0.12"
sha2 = "0.10"
subtle = "2.6"
boringtun = { version = "0.7.1", default-features = false }
```

- 每会话**新生成一对 X25519 临时密钥**，不落盘（对照 Go 每会话临时证书的思路，但 Rust 用 WG 密钥
  承担身份）。对端公钥来自 M2.4 的加密清单。
- PSK 派生（DEVELOPMENT.md §3.3）：`psk = HKDF-SHA256(ikm=K, salt=channel_id, info="hermod-wg-psk-v1")`：

```rust
// 示意：keys.rs
pub fn derive_psk(k: &[u8; 32], channel_id: u16) -> [u8; 32] {
    let mut out = [0u8; 32];
    let salt = channel_id.to_be_bytes();          // [u8; 2]
    hkdf::Hkdf::<sha2::Sha256>::new(Some(salt.as_slice()), k)
        .expand(b"hermod-wg-psk-v1", &mut out)
        .expect("expand into 32 bytes");
    out
}
```

- 验证：两端各自派生结果一致；PSK 不匹配时握手失败（M3.1 验收）。

**M3.2/M3.3 隧道与会话（`tunnel/wg/tunnel.rs`、`tunnel/session.rs`）**
- 用 boringtun 的 `Tunn::new(...)`（API 见 DEVELOPMENT.md 附录 B）。核心循环（对照 DEVELOPMENT.md
  §3.4 的图）：

```rust
// 示意：数据线程主循环（单线程、无 async；见 §3.8）
loop {
    // 1) 从 UDP socket 读包（本机绑定 [::]，IPV6_V6ONLY）
    let n = udp.recv(&mut buf).await?;            // 外层 WireGuard 包
    // 2) 交给 boringtun 解封装
    match tunnel.decapsulate(Some(src_ip), &buf[..n], &mut inner) {
        TunnResult::Packet(pkt) => stack.receive(&pkt),      // 内层 IP 交给 smoltcp
        TunnResult::Err(e) => { /* 丢包计数 */ }
        _ => {}
    }
    // 3) smoltcp poll 出要发的内层 IP 包
    while let Some(pkt) = stack.poll() {
        // 4) 封装成 WireGuard 包再发回 UDP
        if let TunnResult::WriteTo(out, dest) = tunnel.encapsulate(&pkt, &mut out_buf) {
            udp.send_to(&out_buf[..out], dest).await?;
        }
    }
    // 5) 定时 update_timers（约 100ms 一次，驱动 keepalive/rekey）
}
```

- **同时发起**：双方都对全部候选地址"同时发握手"。boringtun 的 `decapsulate` 能接受来自尚未
  确定地址的报文，天然支持（附录 B）。M3.3 要写一个显式竞态测试验证只产生一个会话；若 boringtun
  处理不了，退路是固定角色：tx 发起、rx 响应（§3.3）。
- **漫游**：任何一个成功解封装的报文的源地址 = 新的对端端点（WireGuard 规则，写进 endpoint.rs）。
- **MTU**：隧道内 1420、TCP MSS 1380；`--wg-mtu` 可调巨帧（§3.3）。keepalive 25 s。
- 覆盖网地址：tx=10.66.66.1/32、rx=10.66.66.2/32（进程内，不装宿主机路由，D14）。

**M3.4 真实网络可达性 + M3.5 报告**
- 用两台有 IPv6 的机器（或同机 loopback + 真实网卡）测跨网络/同链路传输；无 IPv6、不可达要在
  5 秒内给出明确诊断，不挂起。
- 把成功/失败组合记进 `docs/benchmarks/reachability.md`，这是将来决定"要不要加打洞"的依据。

### M3 完成标准

两个进程内 peer 握手成功、PSK 错则失败；同时发起只出一个会话；真实 IPv6 环境连通；不可达场景
5 秒报错。

---

## M4 —— HTTP 传输与 CLI（对应 §8 M4，4–6 天）

### 要做什么（一句话）

把隧道变成"能传文件"的完整工具：隧道内跑 HTTP（GET 拉文件 + POST /done 回裁决），CLI 四个命令
(tx/rx/serve/trust) 可用、取消干净、摘要一致才落盘。（对照 GO_ARCHITECTURE.md §4，但 Rust 版
传输语义是 HTTP 而不是 QUIC 四条流。）

### 先学什么

smoltcp 设备抽象（`Device` trait 的 receive/transmit）、tokio 流适配（把 socket 桥成
`AsyncRead + AsyncWrite`）、clap 参数解析、信号处理（`tokio::signal`）。

### 任务清单与写法

**M4.1 smoltcp 桥接（`tunnel/stack/`）**
- 依赖：`smoltcp = "0.14"`（features 开 `medium-ip` 等，见附录 A）。smoltcp 是一个**进程内
  TCP/IP 栈**：它给你提供 TCP 连接，数据包实际走你给的 UDP+WireGuard。
- `device.rs`：实现 smoltcp 的 `phy::Device` trait（`receive()` / `transmit()`），内部是一个队列，
  M3 的数据线程往里灌包、往外取包。
- `driver.rs`：`Interface::new(config, device)` + 周期 `poll()`；配置 IPv6、TCP socket 缓冲
  （每方向 8 MiB）、启用窗口缩放、CUBIC 拥塞控制。
- 验证：隧道内能 `ping` 通对端（ICMP），TCP 能连（M4.1 验收）。

**M4.2 异步流适配（`tunnel/stack/bridge.rs`）**
- 把 smoltcp 的一个 TCP socket 封装成 `tokio::io::AsyncRead + AsyncWrite`，中间用有界 channel
  做缓冲（真实背压、内存有界，目标：传 1 GiB 时 RSS < 256 MiB）。
- 这是数据面**最容易出错**的地方（帧边界、背压、EOF），DEVELOPMENT.md 风险 R9 点名过：适配器
  保持极小，用**内存假设备**写边界测试，不依赖真网络。
- 验证：`AsyncRead`/`AsyncWrite` 往返正确、无忙等；EOF 和半包都测（M4.2 验收）。

**M4.3 隧道内 HTTP（`tunnel/http/`）**
- 设计（DEVELOPMENT.md §3.4、§3.6）：tx 在 10.66.66.1 上起 HTTP server（URL 带一次性随机 token），
  rx 发一次 `GET /f/<token>` 拉整个文件，再 `POST /done` 回 `{"sha256":..., "size":N}` 裁决。
- 实现两条路，**建议先走 A 再换 B**：
  - **A. 教学版**：直接用 M4.2 的 `AsyncRead/AsyncWrite`，手写极简 HTTP/1.1 解析（读请求行/头、
    写 `Content-Length`/`Content-Disposition`/`X-Hermod-Kind`）。几十行就能跑通，且符合 S4 说的
    "自研头解析要限长度"。
  - **B. 生产版**：换 `hyper`（+`hyper-util`）做标准 HTTP server/client。hyper 1.x 的
    `Service`/body 抽象对新手陡，故放后面。
- 验证：tx 能在隧道里用 `curl` 拉到文件；200/404/长度不符/chunked 各分支正确（M4.3 验收）。
  **隧道内是明文 http，绝不加 TLS**（S7：评审清单写死）。

**M4.4 传输主流程（`tunnel/session.rs` 编排 + `hermod-cli/src/tx.rs`、`rx.rs`）**
- 端到端（DEVELOPMENT.md §3.2 步骤 1–12）：会合 → 建隧道 → tx 起 HTTP server → rx GET → 双端
  边传边算 SHA-256 → POST /done 比对 → 一致才原子改名。
- rx 落盘：写 `.hermod_tmp`（O_EXCL、0600）→ 校验通过 → `std::fs::rename`；路径安全与去重复用
  M1.5 的 transfer.rs（逻辑照抄 Go）。
- 载荷三类：文件 / 文本 / stdin 流；文本与流直接进隧道，不预读。
- 验证：1 GiB 文件、文本、stdin 管道三种载荷端到端成功；摘要一致才改名（M4.4 验收）。

**M4.5 取消与超时（`hermod-cli/src/tx.rs`、`rx.rs`）**
- `tokio::signal::ctrl_c()` 监听 Ctrl+C；任一方取消即关连接，rx 删临时文件，双方非零退出。
- 各阶段超时预算：连接 server 10 s、等对端 600 s（server TTL）、PAKE 10 s、bundle 10 s、握手
  5 s、TCP 10 s、载荷空闲 30 s、收尾 2 s（§3.2）。每个超时都必须清理干净。
- 验证：任一方取消与 SIGINT 都不留临时文件、退出码正确（M4.5 验收）。

**M4.6 CLI 对齐（`hermod-cli/src/cli.rs`）**
- 依赖：`clap = { version = "4", features = ["derive"] }`。
- 四个命令 tx / rx / serve / trust，参数与 Go 版逐一对齐，**差异只允许附录 E 列出的**：
  - Go 的 `--verify/-v`（SAS）→ Rust **删除**，传了报 `error: unrecognized option`（D12）。
  - 新增 `--wg-mtu`（默认 1420）。
- 进度条画在 stderr（`progress.rs`），文本输出走 stdout；verbosity 逻辑在 `verbosity.rs`。
- 验证：与 Go 版 `--help` 逐参数对照（M4.6 验收）。

### M4 完成标准

三种载荷端到端成功、取消干净、CLI 对齐、摘要一致才改名、`cargo test` + clippy 全绿。
（Rust server 可继续延后。）

---

## M5 —— 性能（对应 §8 M5，3–5 天）

### 要做什么（一句话）

把吞吐调上去，重点是**丢包链路上至少达到 Go 版基线的 2 倍**（这是当初重写的首要动机）。
按 DEVELOPMENT.md §5.3 的调优顺序做。

### 写法（按顺序，每步都用基准验证，别拍脑袋）

1. **批量 UDP I/O**：用 `socket2` 的 `recvmmsg`/`sendmmsg` 一次收发多包，减少系统调用
   （§5.1 的第一项）。
2. **缓冲池复用**：固定 1 MiB 块，避免逐包分配（`Vec::with_capacity` + 复用）。
3. **8 MiB TCP socket 缓冲 + 窗口缩放**（smoltcp 配置里）。
4. **哈希独立线程**：SHA-256 单核 ~1.5–2 GB/s，放独立线程避免拖累 TCP（§3.8）。
5. 顺序读提示（`posix_fadvise`）与 4 MiB 读粒度。
6. 进度渲染只用原子计数。
7. 仅在实测需要时才做：busy-poll、io_uring、拥塞控制改进。

### 基准方法（§5.4，不需要 root）

- **干净路径**：同机或同 LAN，1 GiB 载荷，目标 ≥ 同机 `wireproxy` 参考值的 90%。
- **丢包路径**：进程内丢包/延迟 shim 包住 UDP socket（1% 丢包、20 ms RTT、0.1% 乱序、随机种子
  可复现）。不能依赖 `tc netem`（本机 sudo 被 `no_new_privs` 拦）。
- 报告：吞吐、每 GiB 的 CPU 秒、p99 停顿。同机对照 `hermod-go`、`iperf3`、`wireproxy`。

### M5 完成标准

验收标准 §9：丢包路径 ≥ Go 基线 2 倍且 p99 停顿 ≤ 1 s；干净路径 ≥ wireproxy 90%；内存有界
（1 GiB 时 RSS < 256 MiB）。

---

## M6 —— 打包与发布（对应 §8 M6，2–3 天）

### 要做什么（一句话）

产出三平台二进制 + 文档 + CI，让用户"装上就能跑"。

### 写法

1. **M6.1 发布产物**：
   - Linux：静态 musl 二进制（`x86_64-unknown-linux-musl` target，避免 glibc 依赖）。
   - macOS：双架构（`aarch64-apple-darwin` + `x86_64-apple-darwin`）。
   - Windows：`x86_64-pc-windows-msvc`。
   - 用 GitHub Actions 的 `cross` 或矩阵构建，产物打 tar/zip。
2. **M6.2 文档**：README 写清——用法、**只支持 IPv6** 的范围限制、**无带外校验**的安全姿态
   （和 Go 版一样的诚实警告，见 GO_ARCHITECTURE.md §7）。
3. **M6.3 CI**：构建 + 测试 + clippy + 依赖审计 + 丢包 profile 端到端任务，全部进 `.github/workflows`。

### M6 完成标准

三平台可构建并完成一次传输；README 完整；CI 全绿（验收标准 §9 的 4–11 条）。

---

## 7. 每阶段要新学的 Rust 知识点（速查）

| 阶段 | 新知识点 | 用到哪 |
| --- | --- | --- |
| 速览 | struct/enum/match、impl/trait、Result/?、thiserror/anyhow、async+tokio | 全程 |
| M1 | 单元测试 `#[test]`、cargo 依赖、serde 序列化 | CPace/AEAD/配置 |
| M2 | 自定义 trait 实现（ServerCertVerifier）、async 网络、channel | 信令/pinning |
| M3 | `&mut self` 状态机、subtle 常量时间、单线程数据循环 | WireGuard |
| M4 | smoltcp Device trait、AsyncRead/AsyncWrite 适配、clap、信号 | 传输/CLI |
| M5 | 基准测试、原子计数（AtomicUsize）、批量 I/O | 性能 |
| M6 | 交叉编译、CI 脚本 | 发布 |

## 8. 初学者最常踩的坑（提前打预防针）

1. **borrow checker 报错** → 先想"能不能 clone 一份 / 把值先取出再借用"，别硬扛。
2. **async 里调了阻塞函数**（std 读文件、DNS）→ 卡死整个运行时。用 tokio 版或用 `spawn_blocking`。
3. **clippy 全开 + unsafe forbid** → 别写 `unsafe`，别让警告攒着，每改一段就 `cargo clippy`。
4. **证书 pinning 卡住** → 先照 rustls 官方示例改，别自己发明 API 调用。
5. **字节格式不一致** → 凡是"要与 Go 对齐"的地方（blob、词表、配置、CPace 排序），**直接照抄
   Go 的字节处理**，别凭感觉重排。M1.6 的向量测试就是抓这个的。
6. **测试跑不过** → 先确认是"实现错"还是"依赖/环境错"；本项目测试必须 `-- -D warnings` 干净地过。

## 附录 A — 按里程碑的依赖清单（合并自 DEVELOPMENT.md 附录 A）

| crate | 版本 | 用到 | 里程碑 |
| --- | --- | --- | --- |
| `p256` | 0.14 | CPace（ecdh + hash2curve） | M1 |
| `x25519-dalek` | 3 | WG 静态密钥 | M3 |
| `aes-gcm` | 0.11 | bundle 加密 | M1 |
| `sha2` / `hmac` / `hkdf` | 当前 | 摘要、派生 | M1 |
| `subtle` | 2.6 | 常量时间比较 | M2/M3 |
| `zeroize` | 当前 | 密钥擦除 | M1/M3 |
| `thiserror` | 2 | core 错误类型 | M1 |
| `anyhow` | 1 | CLI 错误 | M4 |
| `serde` / `serde_json` / `serde_yaml` | 当前 | 信令/配置 | M1/M2 |
| `tokio` | 1 | 控制面运行时 | M2 |
| `tokio-tungstenite` | 当前 | WebSocket 客户端 | M2 |
| `rustls` | 0.23 | TLS + pinning verifier | M2 |
| `boringtun` | 0.7.1（default-features=false） | 用户态 WireGuard | M3 |
| `smoltcp` | 0.14 | 进程内 TCP/IP | M4 |
| `hyper` / `hyper-util` / `tower-http` | 当前 | 隧道内 HTTP（生产版） | M4 |
| `clap` | 4 | CLI 参数 | M4 |
| `socket2` | 当前 | 批量 UDP I/O | M5 |
| `ml-kem` | 0.3.2 | **v1 不用**（D6，S1 恢复路径） | — |

明确不用：`quinn`（QUIC，D2 丢弃）、`wireguard-go`/`wireproxy`（Go，仅 M5 测量参考）。

## 附录 B — 常用命令速查

```bash
cargo build --workspace                 # 全编译
cargo test -p hermod-core               # 只测 core
cargo clippy -- -D warnings             # 门禁 1
cargo fmt --check                       # 门禁 2
cargo add <crate>                       # 在哪个目录跑就给哪个 crate 加依赖
cargo run -p hermod-cli -- --help       # 跑 CLI（当前只有占位输出）
```

## 附录 C — 三份文档速查对照

| 你想知道的 | 看 |
| --- | --- |
| "为什么这么设计"（决策/取舍） | DEVELOPMENT.md §0、§1、§6 |
| "Go 版实际怎么做的"（参考行为） | GO_ARCHITECTURE.md |
| "Rust 版该在哪写什么" | 本文档（各里程碑任务清单） |
| "每个文件承担什么职责" | DEVELOPMENT.md §3.1 + hermod-core/src/lib.rs 头注释 |
| "Go ↔ Rust 参数差异" | DEVELOPMENT.md 附录 E |
| "依赖和常量" | DEVELOPMENT.md 附录 A / D |
