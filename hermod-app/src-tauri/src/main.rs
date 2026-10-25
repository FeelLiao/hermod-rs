#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    {
        // WSLg 的部分驱动在 WebKit GPU 合成初始化时挂起；仅对子进程设置回退。
        let wsl = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"));
        if wsl && std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
            let status = std::process::Command::new(
                std::env::current_exe().expect("Application path unavailable"),
            )
            .args(std::env::args_os().skip(1))
            .env("WEBKIT_DISABLE_COMPOSITING_MODE", "1")
            .env(
                "GDK_BACKEND",
                std::env::var_os("GDK_BACKEND").unwrap_or_else(|| "x11".into()),
            )
            .status()
            .expect("Unable to launch WSLg compatibility mode");
            std::process::exit(status.code().unwrap_or(1));
        }
    }
    hermod_app_lib::run();
}
