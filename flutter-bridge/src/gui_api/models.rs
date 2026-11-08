//! GUI 稳定数据模型；不暴露传输实现或私有存储路径。
use crate::app::{Bootstrap, Selection, Settings, Status, storage::Received};
use serde::{Deserialize, Serialize};

pub use crate::app::storage::Received as InboxItem;
pub use crate::app::worker::TransferPhase;
pub use crate::app::{
    Bootstrap as LegacyBootstrap, Selection as SourceSelection, Settings as GuiSettings,
};

/// GUI 可用于恢复页面状态的类型化任务快照。
#[derive(Clone, Serialize, Deserialize)]
pub struct TransferSnapshot {
    pub phase: TransferPhase,
    pub phase_message: String,
    pub code: Option<String>,
    pub bytes: u64,
    pub total: Option<u64>,
    pub elapsed: f64,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub digest: Option<String>,
    pub rtt_ms: Option<f64>,
    pub transport: Option<String>,
    pub received_id: Option<String>,
}

impl TransferSnapshot {
    pub(crate) fn from_parts(phase: TransferPhase, status: Status) -> Self {
        Self {
            phase,
            phase_message: status.phase,
            code: status.code,
            bytes: status.bytes,
            total: status.total,
            elapsed: status.elapsed,
            done: status.done,
            cancelled: status.cancelled,
            error: status.error,
            digest: status.digest,
            rtt_ms: status.rtt_ms,
            transport: status.transport,
            received_id: status.received_id,
        }
    }
}

/// 启动传输时恰好设置一个字段。
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct StartRequest {
    pub source_id: Option<String>,
    pub code: Option<String>,
    pub text: Option<String>,
}

// 保留显式别名，桥接代码无需依赖 app 模块路径。
pub type BootstrapDto = Bootstrap;
pub type SettingsDto = Settings;
pub type SelectionDto = Selection;
pub type ReceivedDto = Received;
