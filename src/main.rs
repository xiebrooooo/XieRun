//! XieRun：无参数启动 GUI，带子命令进入 CLI（青龙等定时任务用）。
//! full 构建（默认）含 GUI + CLI；`--no-default-features` 仅 CLI。

use xierun::cli;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        #[cfg(feature = "gui")]
        run_gui();
        #[cfg(not(feature = "gui"))]
        {
            eprintln!("当前为 CLI 构建，子命令用法：XieRun help");
            std::process::exit(1);
        }
    } else {
        let code = cli::main(args);
        std::process::exit(code);
    }
}

#[cfg(all(windows, feature = "gui"))]
fn free_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn FreeConsole() -> i32;
    }
    unsafe {
        FreeConsole();
    }
}

#[cfg(not(all(windows, feature = "gui")))]
fn free_console() {}

#[cfg(feature = "gui")]
fn install_panic_logger() {
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "未知错误（payload 无法解析）".to_string()
        };
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "未知位置".to_string());
        let bt = std::backtrace::Backtrace::force_capture();
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let text =
            format!("===== [{stamp}] PANIC =====\n消息: {msg}\n位置: {loc}\n回溯:\n{bt}\n\n");
        let path = xierun::platform::data_dir().join("panic.log");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = f.write_all(text.as_bytes());
        }
        show_crash_dialog(&format!(
            "程序发生内部错误，已记录到 panic.log：\n\n消息：{msg}\n位置：{loc}\n\n请把程序目录下的 panic.log 反馈给作者。"
        ));
    }));
}

#[cfg(windows)]
fn show_crash_dialog(text: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            hwnd: *mut core::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            u_type: u32,
        ) -> i32;
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let t = wide(text);
    let c = wide("XieRun 崩溃");
    unsafe {
        MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), 0x10);
    }
}

#[cfg(not(windows))]
fn show_crash_dialog(_text: &str) {}

#[cfg(feature = "gui")]
fn run_gui() {
    free_console();
    install_panic_logger();
    xierun::gui_log("=== GUI 启动开始（渲染后端 = WGPU/DX12）===");
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        vsync: true,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([880.0, 720.0])
            .with_title("XieRun 校园跑助手"),
        ..Default::default()
    };
    xierun::gui_log("调用 run_native，等待创建窗口…");
    let result = xierun::run_gui(options);
    xierun::gui_log(&format!("run_native 返回：{:?}", result.is_ok()));
    if let Err(e) = result {
        eprintln!("GUI 启动失败: {e}");
        show_crash_dialog(&format!(
            "界面初始化失败，程序无法启动：\n\n{e}\n\n常见原因：显卡驱动 / 远程桌面 / 缺少图形环境。"
        ));
        std::process::exit(1);
    }
}
