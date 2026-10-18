//! 日志封装：基于 `tracing`（第三方结构化日志，对标 Go 的 `slog`）。
//!
//! core 内部统一用 [`log_error!`](crate::log_error!) / [`log_warn!`]
//! / [`log_info!`] / [`log_debug!`] 打日志；由 `hermod-cli` 安装 subscriber，
//! 级别对应 `--verbose`（none/error/warning/info/debug）。
//!
//! `tracing` 的宏在级别未开启时**不做 `format!`**，热路径无额外开销；
//! 也支持结构化字段：`log_info!(channel_id, "频道已分配")`。

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { ::tracing::error!($($arg)*) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { ::tracing::warn!($($arg)*) };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { ::tracing::info!($($arg)*) };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { ::tracing::debug!($($arg)*) };
}
