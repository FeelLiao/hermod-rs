//! 有界内存日志；级别动态生效，不把文件内容写入日志。
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};
static LINES: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
static LEVEL: AtomicU8 = AtomicU8::new(0);
fn buffer() -> &'static Mutex<VecDeque<String>> {
    LINES.get_or_init(|| Mutex::new(VecDeque::new()))
}
pub fn set_level(level: &str) {
    LEVEL.store(
        match level {
            "error" => 1,
            "warning" => 2,
            "info" => 3,
            "debug" => 4,
            _ => 0,
        },
        Ordering::Relaxed,
    );
    tracing::callsite::rebuild_interest_cache();
}
pub fn initialize() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(|| Writer(Vec::new()))
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                let needed = match *metadata.level() {
                    tracing::Level::ERROR => 1,
                    tracing::Level::WARN => 2,
                    tracing::Level::INFO => 3,
                    tracing::Level::DEBUG => 4,
                    _ => 5,
                };
                needed <= LEVEL.load(Ordering::Relaxed)
            }));
        let _ = tracing_subscriber::registry().with(layer).try_init();
    });
}
pub fn read() -> Vec<String> {
    buffer().lock().unwrap().iter().cloned().collect()
}
struct Writer(Vec<u8>);
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .extend_from_slice(&bytes[..bytes.len().min(8192usize.saturating_sub(self.0.len()))]);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0).trim().to_string();
        if !text.is_empty() {
            let mut lines = buffer().lock().unwrap();
            if lines.len() >= 500 {
                lines.pop_front();
            }
            lines.push_back(text);
        }
    }
}
