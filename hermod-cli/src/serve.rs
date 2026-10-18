//! `hermod serve`：Rust 信令服务器（D13 延后，v1 用 Go 版）。

/// Rust server 尚未实现，明确报错。
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("Rust 信令服务器尚未实现（D13）；请使用 Go 版 server 或远程服务器".into())
}
