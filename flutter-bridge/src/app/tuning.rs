//! 持久化表示只负责单位转换；有效范围由 core 唯一校验。
use hermod_core::options::TransferTuning;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[flutter_rust_bridge::frb(non_opaque)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TuningSettings {
    pub streams: Option<u32>,
    pub chunk_bytes: u32,
    pub window_bytes: u64,
    pub udp_buffer_bytes: u32,
    pub slice_bytes: u64,
    pub prefetch: u32,
    pub keepalive_secs: u32,
    pub idle_timeout_secs: u32,
}
impl Default for TuningSettings {
    fn default() -> Self {
        let t = TransferTuning::default();
        Self {
            streams: t.streams.map(|n| n as u32),
            chunk_bytes: t.chunk as u32,
            window_bytes: t.window,
            udp_buffer_bytes: t.udp_buffer as u32,
            slice_bytes: t.slice_size,
            prefetch: t.prefetch as u32,
            keepalive_secs: t.keepalive.as_secs() as u32,
            idle_timeout_secs: t.idle_timeout.as_secs() as u32,
        }
    }
}
impl TuningSettings {
    pub fn validated(&self) -> Result<TransferTuning, String> {
        let t = TransferTuning {
            streams: self.streams.map(|n| n as usize),
            chunk: self.chunk_bytes as usize,
            window: self.window_bytes,
            udp_buffer: self.udp_buffer_bytes as usize,
            slice_size: self.slice_bytes,
            prefetch: self.prefetch as usize,
            keepalive: Duration::from_secs(self.keepalive_secs.into()),
            idle_timeout: Duration::from_secs(self.idle_timeout_secs.into()),
        };
        t.validate()?;
        Ok(t)
    }
}
