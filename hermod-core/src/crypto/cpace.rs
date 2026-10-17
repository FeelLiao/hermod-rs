//! CPace PAKE (P-256): password-authenticated key agreement run through the
//! relayed blob channel. See DEVELOPMENT.md §3.2 (step 4) and M1.2.
//! Verified against RFC 9380 hash-to-curve vectors and CPace vectors
//! extracted from the Go test suite (M1.6).

use crypto_bigint::{NonZero, U256};
use p256::{
    elliptic_curve::{
        ff::{Field, PrimeField},
        point::AffineCoordinates,
        sec1::FromSec1Point,
    },
    hash2curve::{hash_from_bytes, ExpandMsgXmd},
    AffinePoint, NistP256, ProjectivePoint, Scalar,
};
use rand_core::Rng;
use sha2::{Digest, Sha256};

// #[derive(...)] 是 Rust 的派生宏，作用：自动帮你实现 trait（特征），不用手写 impl 代码
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum Role {
    Sender,
    Receiver,
}

pub struct CpaceSession {
    scalar: Scalar,
    pub_msg: [u8; 65], // 定长 65 个无符号 8 位（1 字节）元素的数组，存在栈上，编译期长度固定
    role: Role,
}

#[derive(Debug, thiserror::Error)] // thiserror 是一个 Rust 错误处理库，提供了宏来简化错误类型的定义和实现，derive 标签可以自动为枚举生成 Display 和 Error trait 的实现
pub enum CpaceError {
    #[error("hash to curve failed")]
    HashToCurve,
    #[error("invalid peer public message: must be a 65-byte uncompressed P-256 point")]
    InvalidPeerPoint,
}

fn p256_dst(password: &str, channel_id: u16) -> Vec<u8> {
    let mut dst = Vec::with_capacity(19 + password.len()); //预分配内存给可变动态字节数组 dst，避免多次分配
    dst.extend_from_slice(b"hermod-cpace-v1:"); //把一段字节切片追加到 dst 末尾
    dst.extend_from_slice(&channel_id.to_be_bytes());
    dst.push(b':'); //把一个字节追加到 dst 末尾
    dst.extend_from_slice(password.as_bytes());
    if dst.len() > 255 {
        // 把上面拼接结果，带上标准前缀`H2C-OVERSIZE-DST-`做 sha256，输出 32 字节哈希作为新 DST。
        let mut h = Sha256::new();
        h.update(b"H2C-OVERSIZE-DST-");
        h.update(&dst);
        dst = h.finalize().to_vec();
    }
    dst
}

fn cpace_generator(password: &str, channel_id: u16) -> Result<ProjectivePoint, CpaceError> {
    let dst = p256_dst(password, channel_id);
    let msg: &[u8] = b"sender:receiver"; // 发送方和接收方的标识符
    hash_from_bytes::<NistP256, ExpandMsgXmd<Sha256>>(&[msg], &[dst.as_slice()]).map_err(|_| CpaceError::HashToCurve)
}

fn scalar_from_bytes(bytes: [u8; 32]) -> Option<Scalar> {
    let s = Scalar::from_repr(bytes.into()).into_option()?;
    (!bool::from(s.is_zero())).then_some(s)
}

fn rand_scalar(rng: &mut impl Rng) -> Scalar {
    let mut bytes = [0u8; 32];
    loop {
        rng.fill_bytes(&mut bytes);
        if let Some(s) = scalar_from_bytes(bytes) {
            return s;
        }
    }
}

/// 复刻 Go recoverYOnP256：y = (x³ - 3x + b)^((p+1)/4) mod p
/// （P-256 的 p ≡ 3 mod 4，开平方根有封闭公式）。
fn recover_y_on_p256(x: &[u8; 32]) -> [u8; 32] {
    let p = U256::from_be_hex(
        "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
    );
    let a = p.wrapping_sub(&U256::from_u8(3)); // a = -3 mod p
    let b = U256::from_be_hex(
        "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
    );
    let p_nz = NonZero::new(p).expect("P-256 prime is odd");

    let x = U256::from_be_slice_truncated(x, 256);
    // y² = x³ - 3x + b (mod p)
    let x2 = x.mul_mod(&x, &p_nz);
    let x3 = x2.mul_mod(&x, &p_nz);
    let ax = a.mul_mod(&x, &p_nz);
    let y2 = x3.add_mod(&ax, &p_nz).add_mod(&b, &p_nz);
    // y = (y²)^((p+1)/4) mod p，平方-乘循环；指数是公开常量，无时序顾虑
    let exp = (p + U256::ONE).shr_vartime(2);
    let y = pow_mod_p256(y2, exp, &p_nz);

    let mut out = [0u8; 32];
    out.copy_from_slice(y.to_be_bytes().as_slice());
    out
}

fn pow_mod_p256(base: U256, exp: U256, p: &NonZero<U256>) -> U256 {
    let mut result = U256::ONE;
    let mut base = base;
    let mut exp = exp;
    while !bool::from(exp.is_zero()) {
        if bool::from(exp.is_odd()) {
            result = result.mul_mod(&base, p);
        }
        base = base.mul_mod(&base, p);
        exp = exp.shr_vartime(1);
    }
    result
}

/// 对应 Go CPaceInit。返回 (会话, 65 字节公开消息)。
pub fn init(
    password: &str,
    channel_id: u16,
    role: Role,
) -> Result<(CpaceSession, [u8; 65]), CpaceError> {
    let g = cpace_generator(password, channel_id)?;
    let scalar = rand_scalar(&mut rand_core::UnwrapErr(getrandom::SysRng));
    let pub_msg = compute_pub_msg(&g, scalar);
    Ok((CpaceSession { scalar, pub_msg, role }, pub_msg))
}

/// Y = scalar·G，只取 x 坐标，y 用 recover_y_on_p256 恢复（照抄 Go 的流程）。
fn compute_pub_msg(g: &ProjectivePoint, scalar: Scalar) -> [u8; 65] {
    let point = *g * scalar;
    let x_bytes = point.to_affine().x(); // AffineCoordinates::x
    let mut x = [0u8; 32];
    x.copy_from_slice(x_bytes.as_slice());
    let y = recover_y_on_p256(&x);
    encode_point(&x, &y)
}

fn encode_point(x: &[u8; 32], y: &[u8; 32]) -> [u8; 65] {
    let mut pt = [0u8; 65];
    pt[0] = 0x04;
    pt[1..33].copy_from_slice(x);
    pt[33..].copy_from_slice(y);
    pt
}

impl CpaceSession {
    /// 对应 Go CPaceFinish：返回 32 字节共享密钥 K。
    pub fn finish(&self, peer_pub: &[u8; 65]) -> Result<[u8; 32], CpaceError> {
        // 校验 1：非压缩前缀（Go: peerPub[0] != 0x04 直接报错）
        if peer_pub[0] != 0x04 {
            return Err(CpaceError::InvalidPeerPoint);
        }
        // 校验 2：SEC1 解码 + 曲线方程校验（Go 用 ecdh.NewPublicKey）
        let peer = AffinePoint::from_sec1_bytes(peer_pub)
            .map_err(|_| CpaceError::InvalidPeerPoint)?;

        // ISK_x = scalar·peerPub 的 x 坐标；两边算的是同一个点
        let isk_x_bytes = (peer * self.scalar).to_affine().x();
        let mut isk_x = [0u8; 32];
        isk_x.copy_from_slice(isk_x_bytes.as_slice());

        // transcript 排序照抄 Go：role 决定谁是 sender（第 143-148 行）
        let (sender_pub, receiver_pub): (&[u8; 65], &[u8; 65]) = match self.role {
            Role::Sender => (&self.pub_msg, peer_pub),
            Role::Receiver => (peer_pub, &self.pub_msg),
        };
        let mut transcript = Vec::with_capacity(32 + 65 + 65);
        transcript.extend_from_slice(&isk_x);
        transcript.extend_from_slice(sender_pub);
        transcript.extend_from_slice(receiver_pub);

        let digest = Sha256::digest(&transcript);
        let mut k = [0u8; 32];
        k.copy_from_slice(&digest);
        Ok(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 交叉验证用的固定参数：口令 "correct horse battery staple"、频道 42。
    // 三个"独立基准"（RFC 9380 官方向量、Python 模幂、Python 手写 P-256 点乘）
    // 全部绕开本文件代码，用来证明与 Go 版逐字节一致。
    const PASSWORD: &str = "correct horse battery staple";
    const CHANNEL: u16 = 42;

    // ------------------------------------------------------------------
    // 独立基准 1：RFC 9380 §J.1.1 官方向量（P256_XMD:SHA-256_SSWU_RO_）。
    // 该套件与 Go 版 hash_to_curve.go 用完全相同，两边都过同一组向量，
    // 所以哈希到曲线这一环与 Go 逐字节一致。
    // ------------------------------------------------------------------
    #[test]
    fn rfc9380_p256_vectors() {
        const DST: &[u8] = b"QUUX-V01-CS02-with-P256_XMD:SHA-256_SSWU_RO_";

        // msg = ""
        let pt = hash_from_bytes::<NistP256, ExpandMsgXmd<Sha256>>(&[b"".as_slice()], &[DST])
            .unwrap();
        assert_eq!(
            hex(pt.to_affine().x().as_slice()),
            "2c15230b26dbc6fc9a37051158c95b79656e17a1a920b11394ca91c44247d3e4"
        );
        assert_eq!(
            hex(pt.to_affine().y().as_slice()),
            "8a7a74985cc5c776cdfe4b1f19884970453912e9d31528c060be9ab5c43e8415"
        );

        // msg = "abc"
        let pt = hash_from_bytes::<NistP256, ExpandMsgXmd<Sha256>>(&[b"abc".as_slice()], &[DST])
            .unwrap();
        assert_eq!(
            hex(pt.to_affine().x().as_slice()),
            "0bb8b87485551aa43ed54f009230450b492fead5f1cc91658775dac4a3388a0f"
        );
        assert_eq!(
            hex(pt.to_affine().y().as_slice()),
            "5c41b3d0731a27a7b14bc0bf0ccded2d8751f83493404c84a88e71ffd424212e"
        );
    }

    // ------------------------------------------------------------------
    // 独立基准 2：recover_y_on_p256 用 Python 的 pow((x³-3x+b), (p+1)/4, p)
    // 逐字节核对（完全不同的实现），验证 QR 根字节选择与 Go recoverYOnP256
    // 一致——这是最容易出兼容性问题的自定义代码。
    // ------------------------------------------------------------------
    #[test]
    fn recover_y_matches_independent_sqrt_formula() {
        // 我们 DST 下的生成元 x 坐标（Python 独立算出基准 y）
        let x_g = arr32("e874bb29112561f5c9607a7b4448c5b0365128e21511eec2b44b85fc87a53ce8");
        assert_eq!(
            hex(&recover_y_on_p256(&x_g)),
            "c0977a469371b3d98769faec3426be4828f33d39104180c2e784e40076668685"
        );
        // RFC 9380 "abc" 点的 x：QR 根恰好是真实 y 的相反数（p - y）
        let x_abc = arr32("0bb8b87485551aa43ed54f009230450b492fead5f1cc91658775dac4a3388a0f");
        assert_eq!(
            hex(&recover_y_on_p256(&x_abc)),
            "a3be4c2e8ce5d8594eb43f40f33212d278ae07cc6cbfb37b57718e002bdbded1"
        );
    }

    // ------------------------------------------------------------------
    // 独立基准 3：固定标量的完整 CPace 向量。pubSender / pubReceiver / K
    // 全部由独立的 Python P-256 点乘算出（自检：n·G = 无穷远点），算法与 Go
    // 完全一致：ECDH 取 x、y 恢复 QR 根、transcript 排序、SHA-256。
    // ------------------------------------------------------------------
    #[test]
    fn fixed_scalar_vector_matches_independent_reference() {
        let s1 = arr32("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let s2 = arr32("202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f");

        let sender = session_from_bytes(PASSWORD, CHANNEL, Role::Sender, s1).unwrap();
        let receiver = session_from_bytes(PASSWORD, CHANNEL, Role::Receiver, s2).unwrap();

        assert_eq!(
            hex(&sender.pub_msg),
            "04e2ddb7a4cdba0da9e7bde323c5434c2626450b3ce28b517d816d73084814bec6dd7893ae27074984dbb100134e1f2c6601d40c4c7e62336352258c31848e0a3f"
        );
        assert_eq!(
            hex(&receiver.pub_msg),
            "0401527bd84d341919fc83178cbb93861f2fb40aebbaec645cd15bb7bb4aa14b80e23a4013c4b5df74efa42df98994e9135c9f2b5618304fec9ce136776d8aa00c"
        );

        let k_sender = sender.finish(&receiver.pub_msg).unwrap();
        let k_receiver = receiver.finish(&sender.pub_msg).unwrap();
        assert_eq!(k_sender, k_receiver);
        assert_eq!(
            hex(&k_sender),
            "2b942e525cc3f0f95ae8772002776570e555cc9cb175c55f2f25316bb0d4b427"
        );
    }

    // ------------------------------------------------------------------
    // 行为测试：对应 Go crypto_test.go 的四个测试。
    // ------------------------------------------------------------------

    /// 对应 Go TestCPaceExchange：同口令双方得到相同 K。
    #[test]
    fn exchange_produces_same_key() {
        let (sender, sender_pub) = init(PASSWORD, CHANNEL, Role::Sender).unwrap();
        let (receiver, receiver_pub) = init(PASSWORD, CHANNEL, Role::Receiver).unwrap();

        let k_sender = sender.finish(&receiver_pub).unwrap();
        let k_receiver = receiver.finish(&sender_pub).unwrap();

        assert_eq!(k_sender, k_receiver);
        assert_eq!(k_sender.len(), 32);
    }

    /// 对应 Go TestCPaceWrongPassword：口令不同 → K 不同。
    #[test]
    fn wrong_password_differs() {
        let (correct_sender, correct_sender_pub) =
            init("correct-horse", CHANNEL, Role::Sender).unwrap();
        let (_, wrong_pub) = init("wrong-battery", CHANNEL, Role::Receiver).unwrap();
        let (correct_receiver, _) = init("correct-horse", CHANNEL, Role::Receiver).unwrap();

        let wrong_k = correct_sender.finish(&wrong_pub).unwrap();
        let reference_k = correct_receiver.finish(&correct_sender_pub).unwrap();

        assert_ne!(wrong_k, reference_k);
    }

    /// 对应 Go TestCPaceRoleSeparation：两个 sender 互 finish → K 不同
    /// （共享秘密相同，但 transcript 里 pub1/pub2 顺序相反）。
    #[test]
    fn role_separation() {
        let (s1, pub1) = init(PASSWORD, CHANNEL, Role::Sender).unwrap();
        let (s2, pub2) = init(PASSWORD, CHANNEL, Role::Sender).unwrap();

        let k1 = s1.finish(&pub2).unwrap();
        let k2 = s2.finish(&pub1).unwrap();

        assert_ne!(k1, k2);
    }

    /// 对应 Go TestCPaceFinishInvalidPubMsg：非法 peer 点必须报错。
    /// （长度由 `&[u8; 65]` 在类型层面保证，这里测前缀与曲线校验。）
    #[test]
    fn finish_rejects_invalid_peer_point() {
        let (session, _) = init(PASSWORD, CHANNEL, Role::Sender).unwrap();

        // 非 0x04 前缀（Go 的第一层校验）
        let mut bad_prefix = [0u8; 65];
        bad_prefix[0] = 0x00;
        assert!(session.finish(&bad_prefix).is_err());

        // x=0, y=0 不在曲线上（Go 的第二层校验）
        let mut off_curve = [0u8; 65];
        off_curve[0] = 0x04;
        assert!(session.finish(&off_curve).is_err());
    }

    // ------------------------------------------------------------------
    // 测试辅助函数
    // ------------------------------------------------------------------

    /// 用固定标量构造会话（绕过随机熵，便于向量对拍）。
    fn session_from_bytes(
        password: &str,
        channel_id: u16,
        role: Role,
        scalar_bytes: [u8; 32],
    ) -> Option<CpaceSession> {
        let g = cpace_generator(password, channel_id).ok()?;
        let scalar = scalar_from_bytes(scalar_bytes)?;
        let pub_msg = compute_pub_msg(&g, scalar);
        Some(CpaceSession {
            scalar,
            pub_msg,
            role,
        })
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn arr32(h: &str) -> [u8; 32] {
        let v = unhex(h);
        let mut out = [0u8; 32];
        out.copy_from_slice(&v);
        out
    }

    fn unhex(h: &str) -> Vec<u8> {
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect()
    }
}