//! CLI 小工具。

/// 把任意 error 装箱，便于 handler 统一返回 `Box<dyn Error>`。
pub fn boxed<E: std::error::Error + 'static>(e: E) -> Box<dyn std::error::Error> {
    Box::new(e)
}

/// 共享的「中止句柄」槽：连接建立后由 tx/rx 填入，Ctrl+C 处理器取出并中止。
pub type AbortSlot = std::sync::Arc<std::sync::Mutex<Option<hermod_core::AbortHandle>>>;
