//! Shared desktop and Android application.

mod api;
pub mod cli;
mod crypto;
pub mod location;
pub mod platform;
mod relaunch;
pub mod template;
mod textlog;
mod track;
mod update;

#[cfg(feature = "gui")]
mod ui;

#[cfg(all(target_os = "android", feature = "android"))]
mod android;

/// 启动阶段日志：写入数据目录 gui.log，用于定位死机发生在哪一步。
#[cfg(feature = "gui")]
pub fn gui_log(msg: &str) {
    use std::io::Write;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let path = platform::data_dir().join("gui.log");
    // 超过 2 MB 就滚一次，避免长期运行把磁盘写满。
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 2 * 1024 * 1024 {
            let _ = std::fs::rename(&path, platform::data_dir().join("gui.old.log"));
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{stamp}] {msg}");
    }
}

#[cfg(feature = "gui")]
pub fn run_gui(options: eframe::NativeOptions) -> eframe::Result<()> {
    eframe::run_native(
        "XieRun",
        options,
        Box::new(|cc| {
            gui_log("窗口与渲染后端已就绪（已越过图形初始化）");
            #[cfg(target_os = "android")]
            android::set_context(&cc.egui_ctx);
            let app = ui::App::new(cc);
            gui_log("App 构造完成，进入事件循环");
            Ok(Box::new(app))
        }),
    )
}
