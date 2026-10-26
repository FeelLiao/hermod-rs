//! 与 Web 界面共用翻译词典，原生日志和协议状态保持稳定的英文值。
use std::{collections::HashMap, sync::OnceLock};

pub fn translate<'a>(locale: &str, message: &'a str) -> &'a str {
    if locale != "zh-CN" {
        return message;
    }
    static CHINESE: OnceLock<HashMap<String, String>> = OnceLock::new();
    CHINESE
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../src/locales/zh-CN.json"))
                .expect("Invalid bundled Chinese translations")
        })
        .get(message)
        .map_or(message, String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_strings_share_the_gui_catalog() {
        assert_eq!(
            translate("zh-CN", "Choose a file to send"),
            "选择要发送的文件"
        );
        assert_eq!(
            translate("zh-CN", "Waiting for the receiver"),
            "等待接收方连接"
        );
        assert_eq!(translate("en-US", "Cancel"), "Cancel");
        assert_eq!(
            translate("zh-CN", "unknown diagnostic"),
            "unknown diagnostic"
        );
    }
}
