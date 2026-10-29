//! Platform services shared by the existing UI and the Android host.

#[derive(Clone, Copy, Debug)]
pub enum InputKind {
    Text,
    Password,
    Integer,
    Decimal,
}

pub fn data_dir() -> std::path::PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_DATA_DIR.with(|p| p.borrow().clone()) {
        return path;
    }
    #[cfg(target_os = "android")]
    {
        crate::android::data_dir()
    }
    #[cfg(not(target_os = "android"))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }
}

pub fn edit_text(id: u64, value: &str, kind: InputKind) {
    #[cfg(target_os = "android")]
    crate::android::edit_text(id, value, kind);
    #[cfg(not(target_os = "android"))]
    let _ = (id, value, kind);
}

pub fn take_edited_text(id: u64) -> Option<String> {
    #[cfg(target_os = "android")]
    {
        crate::android::take_edited_text(id)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id;
        None
    }
}

pub fn set_keep_screen_on(enabled: bool) {
    #[cfg(target_os = "android")]
    crate::android::set_keep_screen_on(enabled);
    #[cfg(not(target_os = "android"))]
    let _ = enabled;
}

/// 当前程序版本：桌面读 Cargo 版本，Android 读 manifest versionName
/// （形如 0.2.5-android.3，与 Release tag 对齐）。
pub fn version_name() -> String {
    #[cfg(target_os = "android")]
    {
        crate::android::version_name()
    }
    #[cfg(not(target_os = "android"))]
    {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

/// Android：把已下载的 APK 交给系统安装器（用户在系统弹窗确认安装）。
pub fn install_apk(path: &str) {
    #[cfg(target_os = "android")]
    crate::android::install_apk(path);
    #[cfg(not(target_os = "android"))]
    let _ = path;
}

#[cfg(feature = "gui")]
pub fn sync_clipboard(context: &egui::Context) {
    #[cfg(target_os = "android")]
    {
        let text = context.output(|output| output.copied_text.clone());
        if !text.is_empty() {
            crate::android::copy_text(&text);
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = context;
}

#[cfg(feature = "gui")]
pub fn apply_safe_area(context: &egui::Context, input: &mut egui::RawInput) {
    #[cfg(target_os = "android")]
    if let Some(rect) = input.screen_rect.as_mut() {
        let [left, top, right, bottom] = crate::android::safe_insets();
        let scale = context.pixels_per_point();
        rect.min.x += left as f32 / scale;
        rect.min.y += top as f32 / scale;
        rect.max.x -= right as f32 / scale;
        rect.max.y -= bottom as f32 / scale;
    }
    #[cfg(not(target_os = "android"))]
    let _ = (context, input);
}

/// 当前进程工作集（MB）。仅 Windows 返回真实值，其他平台返回 0。
pub fn rss_mb() -> u64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Pmc {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        #[link(name = "psapi")]
        extern "system" {
            fn GetProcessMemoryInfo(h: *mut core::ffi::c_void, c: *mut Pmc, cb: u32) -> i32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut core::ffi::c_void;
        }
        let mut c = Pmc {
            cb: core::mem::size_of::<Pmc>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
        };
        unsafe {
            if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
                return (c.working_set_size / (1024 * 1024)) as u64;
            }
        }
        0
    }
    #[cfg(not(windows))]
    {
        0
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_DATA_DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}
