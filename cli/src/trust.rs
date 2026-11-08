//! `hermod trust`：抓取服务器证书并 pin 它的 SPKI 指纹。

use crate::cli::TrustArgs;
use crate::util;
use crate::verbosity;
use hermod_core::config;
use hermod_core::signal::tls;
use hermod_core::transfer;

/// 执行 `trust`。
pub async fn run(args: TrustArgs) -> Result<(), Box<dyn std::error::Error>> {
    let url = if args.server.contains("://") {
        args.server.clone()
    } else {
        format!("wss://{}", args.server)
    };

    // 先规范化（无端口时补 4376），保证 trust 与 tx/rx 连的是同一地址。
    let normalized = config::normalize_server_url(&url).map_err(util::boxed)?;
    let fp = tls::fetch_server_spki(&normalized)
        .await
        .map_err(util::boxed)?;
    let hex = transfer::hex(&fp);
    if let Some(expect) = &args.fingerprint
        && !expect.eq_ignore_ascii_case(&hex)
    {
        return Err(format!("fingerprint mismatch: got {hex}, expected {expect}").into());
    }

    let mut cfg = config::load().map_err(util::boxed)?;
    let normalized = config::normalize_server_url(&url).map_err(util::boxed)?;
    // 对齐 Go：pin 指纹，**并把它设为默认 server**（否则 tx/rx 仍用旧默认）。
    cfg.replace_trust(&normalized, &hex).map_err(util::boxed)?;
    config::save(&cfg).map_err(util::boxed)?;

    // 对齐 Go 的 printStatus 输出
    verbosity::status(format!("Pinned {normalized}"));
    verbosity::status(format!("  fingerprint: {hex}"));
    verbosity::status("  set as default server");
    Ok(())
}
