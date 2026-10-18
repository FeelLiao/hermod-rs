//! 传输语义：载荷分类、路径安全、完整性校验（M1.5）。
//!
//! 对应 Go 版 `hermod-go/pkg/transfer/transfer.go`：
//! - `Kind`（第 16 行）             → [`Kind`]
//! - `ClassifyInput`（第 34 行）     → [`classify_input`]
//! - `HashBytes`（第 61 行）         → [`hash_bytes`]
//! - `SafeDestinationPath`（第 83 行）→ [`safe_destination_path`]
//! - `TempPath`（第 109 行）         → [`temp_path`]
//!
//! `Metadata` 的 JSON 编解码（`EncodeMetadata`/`DecodeMetadata`）依赖
//! serde_json，安装后补充（见 M1.6 对拍）。高层 tx/rx 编排 API 属 M4。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 载荷类型（对应 Go `transfer.Kind`）。取值与 HTTP 头 `X-Hermod-Kind` 一致。
/// JSON/YAML 序列化为小写字符串："file" | "text" | "stream"（与 Go 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Text,
    Stream,
}

impl Kind {
    /// 字符串值："file" | "text" | "stream"（与 Go/HTTP 头一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Text => "text",
            Kind::Stream => "stream",
        }
    }

    /// 解析字符串为 Kind；非法输入返回 `None`。
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "file" => Some(Kind::File),
            "text" => Some(Kind::Text),
            "stream" => Some(Kind::Stream),
            _ => None,
        }
    }
}

/// 载荷元数据（对应 Go `transfer.Metadata`），经 JSON 在两端交换：
/// 类型、原始文件名、大小、SHA-256。
///
/// JSON 字段序 `kind/name/size/sha256` 与 Go `json.Marshal` 一致；
/// `name` 为空时省略（对应 Go 的 `omitempty`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub kind: Kind,
    /// 原始文件名（仅 `KindFile` 有意义）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub size: i64,
    /// 载荷原始字节的 SHA-256（十六进制小写）。
    pub sha256: String,
}

/// 序列化元数据为 JSON 字节（对应 Go `EncodeMetadata`）。
pub fn encode_metadata(m: &Metadata) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(m)
}

/// 反序列化 JSON 字节为元数据（对应 Go `DecodeMetadata`）。
pub fn decode_metadata(data: &[u8]) -> Result<Metadata, serde_json::Error> {
    serde_json::from_slice(data)
}

/// 计算字节切片的 SHA-256，返回十六进制小写（对应 Go `HashBytes`）。
pub fn hash_bytes(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// 对输入参数分类（对应 Go `ClassifyInput`）。
///
/// - `"-"` 或（空参数 + stdin 是管道）→ `Stream`
/// - 参数是已存在的文件 → `File`（返回文件基名）
/// - 其余 → `Text`
pub fn classify_input(arg: &str, is_stdin_piped: bool) -> (Kind, String) {
    if arg == "-" || (arg.is_empty() && is_stdin_piped) {
        return (Kind::Stream, String::new());
    }
    if std::path::Path::new(arg).is_file() {
        return (Kind::File, file_base(arg).to_string());
    }
    (Kind::Text, String::new())
}

/// 解析输出路径：拒绝目录穿越（剥掉所有目录分量），已存在时自动加去重后缀。
///
/// 对应 Go `SafeDestinationPath`。示例：`a.pdf` 已存在 → `a(1).pdf`。
/// `name` 来自不可信的对端，所以必须先剥掉目录分量，绝不能让对端把文件写到
/// 目标目录之外（如 `../../etc/passwd`）。
pub fn safe_destination_path(dir: &std::path::Path, name: &str) -> Result<std::path::PathBuf, TransferError> {
    // 1. 只取最后一段文件名，剥掉所有目录分量（兼容 / 和 \）。
    let mut base = file_base(name).to_string();
    // 2. 空 / "." / ".." 都归为占位名 "received"（与 Go 一致）。
    if base.is_empty() || base == "." || base == ".." {
        base = "received".to_string();
    }

    // 3. 目标不存在 → 直接用。
    let mut candidate = dir.join(&base);
    if !candidate.exists() {
        return Ok(candidate);
    }

    // 4. 存在 → 尝试 base(i)ext，i = 1..=9999。
    let ext = file_ext(&base);
    let stem = base.strip_suffix(ext).unwrap_or(&base);
    for i in 1..=9999 {
        candidate = dir.join(format!("{stem}({i}){ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    // 5. 全部 9999 个都占满 → 放弃，避免静默覆盖。
    if candidate.exists() {
        return Err(TransferError::NoFreeName {
            name: base,
            dir: dir.display().to_string(),
        });
    }
    Ok(candidate)
}

/// 临时下载路径：目标路径 + `.hermod_tmp`（对应 Go `TempPath`）。
/// 先写到临时文件，校验通过后原子改名，失败则删临时文件。
pub fn temp_path(dest: &std::path::Path) -> std::path::PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".hermod_tmp");
    std::path::PathBuf::from(s)
}

/// 取最后一段路径分量（对应 Go `filepath.Base`，兼容 `/` 与 `\`）。
fn file_base(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or("")
}

/// 取含点的扩展名（对应 Go `filepath.Ext`）：`a.pdf` → `.pdf`，`noext` → `""`。
fn file_ext(name: &str) -> &str {
    match name.rfind('.') {
        Some(idx) => &name[idx..],
        None => "",
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 路径安全错误。
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error(
        "all 9,999 numbered copies of \"{name}\" already exist in {dir}; \
         delete some files or choose a different destination"
    )]
    NoFreeName { name: String, dir: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 每个测试一个独立临时目录（放系统 temp 下，避免污染工作区）。
    fn temp_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("hermod-transfer-{tag}-{n}"));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// `hash_bytes` 用已知 SHA-256 基准验证（SHA-256("abc") 是公开标准值）。
    #[test]
    fn hash_bytes_known_value() {
        assert_eq!(
            hash_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash_bytes(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    /// Metadata JSON 编解码：字节格式与 Go `json.Marshal` 一致（无 HTML 转义时）+ 往返。
    #[test]
    fn metadata_json_matches_go() {
        let m = Metadata {
            kind: Kind::File,
            name: "report.pdf".to_string(),
            size: 11,
            sha256: hash_bytes(b"hello world"),
        };
        let encoded = encode_metadata(&m).unwrap();
        assert_eq!(
            String::from_utf8(encoded.clone()).unwrap(),
            r#"{"kind":"file","name":"report.pdf","size":11,"sha256":"b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"}"#
        );
        assert_eq!(decode_metadata(&encoded).unwrap(), m);
    }

    /// `name` 为空时省略（对应 Go `omitempty`）；Go 的 HTML 转义输出也能解析。
    #[test]
    fn metadata_json_omitempty_and_parse_go_output() {
        let m = Metadata {
            kind: Kind::Stream,
            name: String::new(),
            size: 0,
            sha256: hash_bytes(b""),
        };
        let encoded = encode_metadata(&m).unwrap();
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            r#"{"kind":"stream","size":0,"sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}"#
        );
        // Go 默认会把 < > & 转义成 \u003c 等，也必须能解析
        let go_escaped = r#"{"kind":"file","name":"a\u003cb","size":3,"sha256":"aa"}"#;
        let m = decode_metadata(go_escaped.as_bytes()).unwrap();
        assert_eq!(m.name, "a<b");
    }

    #[test]
    fn classify_input_basic() {
        assert_eq!(classify_input("-", false), (Kind::Stream, String::new()));
        assert_eq!(classify_input("", true), (Kind::Stream, String::new()));
        assert_eq!(classify_input("hello world", false), (Kind::Text, String::new()));
        // 不存在的路径按文本处理
        assert_eq!(classify_input("/nonexistent/xyz", false), (Kind::Text, String::new()));

        // 真实文件 → File + 基名
        let dir = temp_dir("classify");
        let f = dir.join("note.txt");
        fs::write(&f, "hi").unwrap();
        assert_eq!(
            classify_input(f.to_str().unwrap(), false),
            (Kind::File, "note.txt".to_string())
        );
    }

    /// 目录穿越必须被剥掉（安全核心）。
    #[test]
    fn strips_directory_traversal() {
        let dir = temp_dir("traversal");
        let p = safe_destination_path(&dir, "../../etc/passwd").unwrap();
        assert_eq!(p, dir.join("passwd"));
        assert_eq!(safe_destination_path(&dir, "a/b/c.txt").unwrap(), dir.join("c.txt"));
        // Windows 反斜杠同样要剥
        assert_eq!(safe_destination_path(&dir, r"a\b\evil.sh").unwrap(), dir.join("evil.sh"));
    }

    /// "." / ".." / 空 → 占位名 "received"。
    #[test]
    fn dots_become_received() {
        let dir = temp_dir("dots");
        assert_eq!(safe_destination_path(&dir, "..").unwrap(), dir.join("received"));
        assert_eq!(safe_destination_path(&dir, ".").unwrap(), dir.join("received"));
        assert_eq!(safe_destination_path(&dir, "").unwrap(), dir.join("received"));
    }

    /// 已存在 → 自动去重后缀 a(1).txt, a(2).txt……
    #[test]
    fn dedups_existing_file() {
        let dir = temp_dir("dedup");
        fs::write(dir.join("a.txt"), "1").unwrap();
        let p1 = safe_destination_path(&dir, "a.txt").unwrap();
        assert_eq!(p1, dir.join("a(1).txt"));
        fs::write(&p1, "2").unwrap();
        let p2 = safe_destination_path(&dir, "a.txt").unwrap();
        assert_eq!(p2, dir.join("a(2).txt"));
        // 无扩展名的情况
        fs::write(dir.join("report"), "x").unwrap();
        assert_eq!(safe_destination_path(&dir, "report").unwrap(), dir.join("report(1)"));
    }

    /// 名字不存在 → 原样返回（含扩展名保留）。
    #[test]
    fn fresh_name_unchanged() {
        let dir = temp_dir("fresh");
        assert_eq!(
            safe_destination_path(&dir, "report.pdf").unwrap(),
            dir.join("report.pdf")
        );
    }

    #[test]
    fn temp_path_suffix() {
        assert_eq!(
            temp_path(Path::new("/tmp/x/a.pdf")),
            PathBuf::from("/tmp/x/a.pdf.hermod_tmp")
        );
    }
}
