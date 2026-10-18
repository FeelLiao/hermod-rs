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

    let fp = tls::fetch_server_spki(&url).await.map_err(util::boxed)?;
    let hex = transfer::hex(&fp);
    if let Some(expect) = &args.fingerprint
        && !expect.eq_ignore_ascii_case(&hex)
    {
        return Err(format!("fingerprint mismatch: got {hex}, expected {expect}").into());
    }

    // stdout 只放指纹，便于脚本取值与人工核对
    println!("{hex}");
    if args.fingerprint.is_none() {
        verbosity::warn("首次信任（TOFU）：请在带外核对上面的指纹");
    }

    let mut cfg = config::load().unwrap_or_default();
    let normalized = config::normalize_server_url(&url).map_err(util::boxed)?;
    cfg.trusted_servers.insert(normalized.clone(), hex);
    config::save(&cfg).map_err(util::boxed)?;
    verbosity::info(format!("trusted {normalized}"));
    Ok(())
}
