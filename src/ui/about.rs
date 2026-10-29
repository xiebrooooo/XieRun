//! 关于页与自动更新流程：检查 / 下载进度 / 确认弹窗 / 重启或安装收尾。
//!
//! 桌面端替换 exe 后走 relaunch 重启；Android 端下载 APK 到私有目录后
//! 交给系统安装器（platform::install_apk）。

use super::{theme, App};
use crate::update::ReleaseInfo;
use eframe::egui;

/// 更新流程的 UI 状态（随 App 存活，消息协议回填）。
#[derive(Default)]
pub struct UpdateUi {
    pub checking: bool,
    pub latest: Option<ReleaseInfo>,
    pub up_to_date: bool,
    pub check_error: Option<String>,
    pub downloading: bool,
    pub done: u64,
    pub total: Option<u64>,
    /// 等待用户确认开始下载（启动检查发现新版本时也置此）。
    pub confirm: Option<ReleaseInfo>,
    /// 下载替换完成后的收尾动作。
    pub finish: Option<FinishAction>,
    /// 启动询问模式：是否检查更新。
    pub ask_startup: bool,
}

#[derive(Clone)]
pub enum FinishAction {
    /// 桌面：替换完成，等待重启。
    Restart { tag: String },
    /// Android：APK 已就绪，等待调起安装器（桌面构建下不会构造）。
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    InstallApk { tag: String },
}

impl App {
    pub fn draw_about(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(super::personal::signature(self)).size(12.0).color(theme::accent()));
        ui.add_space(4.0);
        let mut _sig_used = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("XieRun")
                    .size(26.0)
                    .strong()
                    .color(theme::accent()),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!("v{}", crate::platform::version_name()))
                    .size(14.0)
                    .color(theme::text_dim()),
            );
        });
        ui.label(
            egui::RichText::new("校园跑助手 · 运动世界校园版自动化工具")
                .size(13.0)
                .color(theme::text_dim()),
        );
        ui.add_space(10.0);

        let row = |ui: &mut egui::Ui, k: &str, v: &str| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [72.0, 18.0],
                    egui::Label::new(egui::RichText::new(k).color(theme::text_dim())),
                );
                ui.label(egui::RichText::new(v));
            });
        };
        row(ui, "作者", "xiebro");
        row(ui, "项目", "XieRun 校园跑助手");
        row(ui, "运行平台", "Windows / Android（同一份源码）");

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("工作方式").strong());
        ui.colored_label(
            theme::text_dim(),
            "一份源码，运行期自适应：登录哪个学校的账号，就按哪个学校识别位置与打卡点。",
        );
        ui.add_space(8.0);
        ui.label(egui::RichText::new("数据与隐私").strong());
        ui.colored_label(
            theme::text_dim(),
            "账号、密码、会话只保存在本机（exe 同目录的 json 文件）；换电脑时把这些 json 一起拷走即可。",
        );
        ui.add_space(8.0);
        ui.label(egui::RichText::new("更新方式").strong());
        ui.colored_label(
            theme::text_dim(),
            "拉最新源码后重新 cargo build --release，用新产物替换旧 exe；配置文件不要删。",
        );
    }

    fn draw_download_progress(&self, ui: &mut egui::Ui) {
        let bar = egui::ProgressBar::new(self.update.fraction()).text(self.update.progress_text());
        ui.add(bar);
    }

    /// 全局窗口：启动询问 / 确认更新 / 更新收尾（不依赖当前标签页）。
    pub fn draw_update_windows(&mut self, ctx: &egui::Context) {
        if self.update.ask_startup {
            let mut done: Option<&str> = None;
            egui::Window::new(egui::RichText::new("检查更新").strong())
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width(320.0);
                    ui.label("是否检查新版本？\n协议变更时旧版本可能异常，建议检查。");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_btn("检查")).clicked() {
                            done = Some("check");
                        }
                        if ui.button("本次跳过").clicked() {
                            done = Some("skip");
                        }
                        if ui.button("不再询问").clicked() {
                            done = Some("off");
                        }
                    });
                });
            if let Some(choice) = done {
                self.update.ask_startup = false;
                match choice {
                    "check" => self.check_update(false),
                    "off" => {
                        self.config.update_check = "off".into();
                        let _ = crate::api::model::save_config(&self.config);
                    }
                    _ => {}
                }
            }
        }

        if let Some(rel) = self.update.confirm.clone() {
            let mut close = false;
            let mut go = false;
            egui::Window::new(egui::RichText::new(format!("发现新版本 {}", rel.tag)).strong())
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width((ctx.screen_rect().width() - 32.0).min(460.0));
                    if !rel.title.is_empty() {
                        ui.label(&rel.title);
                    }
                    ui.label(format!(
                        "大小 {:.1} MB · 来源 {}",
                        rel.asset_size as f64 / 1024.0 / 1024.0,
                        rel.repo
                    ));
                    let notes: Vec<&str> = rel.notes.lines().take(8).collect();
                    if !notes.is_empty() {
                        ui.add_space(4.0);
                        for line in notes {
                            ui.label(egui::RichText::new(line).small());
                        }
                    }
                    ui.add_space(4.0);
                    ui.colored_label(theme::warn(), "更新会替换为官方 Release 版本并重启程序");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_btn("立即更新")).clicked() {
                            go = true;
                        }
                        if ui.button("忽略").clicked() {
                            close = true;
                        }
                    });
                });
            if go || close {
                self.update.confirm = None;
            }
            if go {
                if self.any_busy() {
                    self.status = "有任务进行中，请稍后再试".into();
                } else {
                    self.start_update_download(rel);
                }
            }
        }

        if let Some(fin) = self.update.finish.clone() {
            let mut done = false;
            let (title, body, button) = match fin {
                FinishAction::Restart { tag } => (
                    format!("已更新到 {tag}"),
                    "程序已替换完成，重启后使用新版本。".to_string(),
                    "重启程序".to_string(),
                ),
                FinishAction::InstallApk { tag } => (
                    format!("已下载 {tag}"),
                    "点击安装，在系统弹窗中确认；若提示禁止安装，请允许本应用安装未知应用。"
                        .to_string(),
                    "安装".to_string(),
                ),
            };
            egui::Window::new(egui::RichText::new(&title).strong())
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width(360.0);
                    ui.label(&body);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_btn(&button)).clicked() {
                            done = true;
                        }
                        if ui.button("稍后").clicked() {
                            self.update.finish = None;
                        }
                    });
                });
            if done {
                match self.update.finish.take() {
                    Some(FinishAction::Restart { .. }) => match crate::relaunch::relaunch_self() {
                        Ok(()) => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(e) => {
                            self.status = e.clone();
                            self.popup = Some(super::PopupInfo {
                                title: "更新完成".into(),
                                lines: vec![e],
                            });
                        }
                    },
                    Some(FinishAction::InstallApk { .. }) => {
                        crate::platform::install_apk(
                            &crate::platform::data_dir()
                                .join(crate::update::APK_NAME)
                                .to_string_lossy(),
                        );
                        self.update.finish = None;
                    }
                    None => {}
                }
            }
        }
    }
}

impl UpdateUi {
    fn fraction(&self) -> f32 {
        match self.total {
            Some(t) if t > 0 => (self.done as f32 / t as f32).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    fn progress_text(&self) -> String {
        let mb = |n: u64| format!("{:.1} MB", n as f64 / 1024.0 / 1024.0);
        match self.total {
            Some(t) if t > 0 => format!("{}/{}", mb(self.done), mb(t)),
            _ => format!("已下载 {}", mb(self.done)),
        }
    }
}
