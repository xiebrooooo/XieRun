//! 应用外壳（XieRun Dark）：顶部品牌条 + 左侧导航栏 + 内容卡片。
//!
//! 分工：
//! - `theme.rs`：色板 / 控件基线；
//! - 本文件：外壳布局（顶栏、侧栏、页面容器）与外壳级组件。
//!
//! 窄窗口（< 760px）自动退回顶部横向导航，便于小屏与手机端。

use super::{mobile, theme, App};
use eframe::egui;
use egui::Color32;

/// 外壳总开关（false 则回退为无外壳的裸页面）。
pub const ENABLED: bool = true;

/// 导航项：标签 + 副标题（副标题只在侧栏显示）。
pub const NAV: [(&str, &str); 9] = [
    ("跑步", "规划与提交"),
    ("AI运动", "批量补跑"),
    ("运动记录", "历史明细"),
    ("数据", "学期进度"),
    ("我的", "账号资料"),
    ("设备信息", "机型与定位"),
    ("运行日志", "过程跟踪"),
    ("路网", "路线绘制"),
    ("关于", "版本与作者"),
];

/// 柔和投影。
pub fn shadow(blur: f32, y: f32, alpha: u8) -> egui::epaint::Shadow {
    theme::shadow(blur, y, alpha)
}

/// 主内容卡片容器。
pub fn card() -> egui::Frame {
    theme::card_frame()
}

/// 次级卡片容器。
pub fn soft_card() -> egui::Frame {
    theme::soft_frame()
}

/// 页面内容区（统一内边距 + 深色底）。
pub fn page_frame() -> egui::Frame {
    egui::Frame::none().fill(theme::page_bg()).inner_margin(egui::Margin {
        left: 16.0,
        right: 16.0,
        top: 12.0,
        bottom: 12.0,
    })
}

/// 主操作按钮。
pub fn primary_btn(text: &str) -> egui::Button<'static> {
    theme::primary_btn(text)
}

/// 覆盖 egui 全局样式为深色工作台外观。
pub fn apply(ctx: &egui::Context) {
    theme::apply(ctx);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(15.0, 8.0);
    style.spacing.interact_size = egui::vec2(46.0, 34.0);
    style.spacing.text_edit_width = 300.0;
    style.spacing.slider_width = 140.0;
    #[cfg(target_os = "android")]
    {
        style.spacing.interact_size = egui::vec2(46.0, 46.0);
        style.spacing.button_padding = egui::vec2(15.0, 11.0);
    }
    ctx.set_style(style);
}

/// 是否为窄窗口（走顶部导航）。
pub fn narrow(ctx: &egui::Context) -> bool {
    ctx.screen_rect().width() < 760.0
}

/// 页面标题区：大标题 + 副标题 + 分割线。
pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(theme::h1(title));
        if !subtitle.is_empty() {
            ui.add_space(2.0);
            ui.label(theme::dim(subtitle));
        }
    });
    ui.add_space(8.0);
    let stroke = egui::Stroke::new(1.0_f32, theme::border());
    let y = ui.cursor().top();
    let rect = egui::Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), y),
        egui::pos2(ui.max_rect().right(), y + 1.0),
    );
    ui.painter().hline(rect.x_range(), y, stroke);
    ui.add_space(10.0);
}

/// 圆角状态胶囊。
pub fn pill(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::none()
        .fill(bg)
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_white_alpha(12)))
        .rounding(egui::Rounding::same(99.0))
        .inner_margin(egui::Margin::symmetric(10.0, 5.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(fg).size(12.0).strong());
        });
}

/// 顶部横向导航（窄窗口 / 兼容入口）。
pub fn nav_bar(ui: &mut egui::Ui, selected: &mut usize) -> Vec<egui::Response> {
    let spacing = 6.0;
    let n = NAV.len() as f32;
    let cell_width = ((ui.available_width() - spacing * (n - 1.0)) / n).max(56.0);
    let mut responses = Vec::with_capacity(NAV.len());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        for (index, (label, _)) in NAV.into_iter().enumerate() {
            let active = *selected == index;
            let text = egui::RichText::new(label).strong().color(if active {
                Color32::WHITE
            } else {
                theme::text_dim()
            });
            let button = egui::Button::new(text)
                .fill(if active {
                    theme::accent_deep()
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(egui::Stroke::NONE)
                .rounding(egui::Rounding::same(9.0))
                .min_size(egui::vec2(cell_width, 32.0));
            let response = ui.add(button);
            if response.clicked() {
                *selected = index;
            }
            responses.push(response);
        }
    });
    responses
}

/// 侧栏导航项：整行圆角按钮，选中时左侧亮起渐变小竖条。
fn nav_item(ui: &mut egui::Ui, index: usize, selected: &mut usize, compact: bool) {
    let (label, hint) = NAV[index];
    let active = *selected == index;
    let height = if compact { 34.0 } else { 42.0 };
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, egui::Rounding::same(10.0), theme::accent_soft());
        painter.rect_stroke(
            rect,
            egui::Rounding::same(10.0),
            egui::Stroke::new(1.0_f32, theme::accent_border()),
        );
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.center().y - height * 0.28),
            egui::vec2(3.0, height * 0.56),
        );
        painter.rect_filled(bar, egui::Rounding::same(2.0), theme::accent());
    } else if response.hovered() {
        painter.rect_filled(rect, egui::Rounding::same(10.0), theme::card_soft());
    }
    let text_color = if active {
        Color32::WHITE
    } else {
        theme::text_dim()
    };
    let x = rect.left() + 14.0;
    if compact {
        painter.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.5),
            text_color,
        );
    } else {
        painter.text(
            egui::pos2(x, rect.center().y - 7.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.5),
            text_color,
        );
        painter.text(
            egui::pos2(x, rect.center().y + 9.0),
            egui::Align2::LEFT_CENTER,
            hint,
            egui::FontId::proportional(10.5),
            if active {
                theme::accent()
            } else {
                theme::text_weak()
            },
        );
    }
    if response.clicked() {
        *selected = index;
    }
}

impl App {
    /// 外壳：顶部品牌条（含登录/连接状态），窄窗口附带横向导航。
    pub fn workbench_top_bar(&mut self, ctx: &egui::Context) {
        let frame = egui::Frame::none()
            .fill(theme::panel_fill())
            .inner_margin(egui::Margin::symmetric(16.0, 10.0));
        egui::TopBottomPanel::top("workbench_top")
            .frame(frame)
            .show(ctx, |ui| {
                // 顶部 3px 品牌渐变条
                let top = ui.cursor().top();
                let bar = egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().left() - 16.0, top - 10.0),
                    egui::pos2(ui.max_rect().right() + 16.0, top - 7.0),
                );
                theme::gradient_rect(ui.painter(), bar);

                let compact = narrow(ctx);
                self.brand_row(ui, compact);
                ui.add_space(9.0);
                if self.session.is_some() {
                    self.session_row(ui, compact);
                } else {
                    self.login_row(ui, compact);
                }
                if compact {
                    ui.add_space(8.0);
                    nav_bar(ui, &mut self.tab);
                }
            });
    }

    /// 左侧导航栏（宽窗口）。
    pub fn workbench_sidebar(&mut self, ctx: &egui::Context) {
        if narrow(ctx) {
            return;
        }
        let compact = ctx.screen_rect().height() < 620.0;
        let frame = egui::Frame::none()
            .fill(theme::panel_fill())
            .inner_margin(egui::Margin::symmetric(10.0, 12.0));
        egui::SidePanel::left("workbench_nav")
            .frame(frame)
            .exact_width(if compact { 116.0 } else { 136.0 })
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("功能导航")
                        .size(11.0)
                        .color(theme::text_weak()),
                );
                ui.add_space(6.0);
                for index in 0..NAV.len() {
                    nav_item(ui, index, &mut self.tab, compact);
                    ui.add_space(2.0);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new("by xiebro")
                            .size(10.5)
                            .color(theme::text_weak()),
                    );
                    ui.label(
                        egui::RichText::new(format!("v{}", crate::platform::version_name()))
                            .size(10.5)
                            .color(theme::text_weak()),
                    );
                });
            });
    }

    /// 品牌 + 连接/登录状态。
    fn brand_row(&mut self, ui: &mut egui::Ui, compact: bool) {
        let logged = self.session.is_some();
        let mut ip_retry = false;
        ui.horizontal(|ui| {
            // 品牌标记：圆角渐变方块 + 白色首字母。
            let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
            theme::gradient_block(ui.painter(), rect, egui::Rounding::same(10.0));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "X",
                egui::FontId::proportional(19.0),
                Color32::WHITE,
            );
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.add_space(1.0);
                ui.label(
                    egui::RichText::new("XieRun 校园跑助手")
                        .size(16.5)
                        .strong()
                        .color(theme::text()),
                );
                ui.label(
                    egui::RichText::new(format!("自定义版 · v{}", crate::platform::version_name()))
                        .size(11.0)
                        .color(theme::text_dim()),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if logged {
                    pill(ui, "● 已登录", theme::ok(), theme::ok_soft());
                } else {
                    pill(ui, "● 未登录", theme::text_dim(), theme::card_soft());
                }
                if !compact {
                    ip_retry = self.ip_pill(ui);
                }
            });
        });
        if ip_retry {
            // 重试时先回到「获取中」，避免旧结果一直显示「失败」。
            self.ip.clear();
            self.fetch_ip();
        }
    }

    /// 连接状态胶囊（IP）；获取失败/进行中时附带「重试」，返回是否请求重试。
    fn ip_pill(&self, ui: &mut egui::Ui) -> bool {
        let bad = self.ip.is_empty() || self.ip == "失败";
        if self.ip.is_empty() {
            pill(ui, "IP 获取中…", theme::text_dim(), theme::card_soft());
        } else if self.ip == "失败" {
            pill(ui, "IP 获取失败", theme::err(), theme::err_soft());
        } else {
            pill(ui, &format!("IP {}", self.ip), theme::ok(), theme::ok_soft());
        }
        if bad && ui.small_button("重试").clicked() {
            return true;
        }
        false
    }

    /// 已登录时的顶栏账号行：当前账号 + 定位 + 登出。
    fn session_row(&mut self, ui: &mut egui::Ui, _compact: bool) {
        let label = self
            .session
            .as_ref()
            .map(|s| {
                if s.name.is_empty() {
                    s.username.clone()
                } else {
                    format!("{}（{}）", s.name, s.username)
                }
            })
            .unwrap_or_default();
        let location = if self.identity.city.trim().is_empty() {
            "定位：未配置".to_string()
        } else {
            format!(
                "定位：{}（{:.5}, {:.5}）",
                self.identity.city, self.identity.anchor_lat, self.identity.anchor_lon
            )
        };
        let mut logout = false;
        let mut edit_location = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("当前账号").color(theme::text_dim()));
            ui.label(egui::RichText::new(label).strong().color(theme::text()));
            ui.label(
                egui::RichText::new(location)
                    .size(11.5)
                    .color(theme::text_dim()),
            );
            if ui.small_button("修改定位").clicked() {
                edit_location = true;
            }
            ui.add_space(8.0);
            logout = ui
                .add_enabled(!self.login_busy, primary_btn("登出"))
                .clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("凭据仅保存在本机 accounts.json")
                        .size(11.5)
                        .color(theme::text_weak()),
                );
            });
        });
        if edit_location {
            self.tab = 5;
        }
        if logout {
            self.do_logout();
        }
    }

    /// 未登录时的顶栏登录行：账号库 + 账号/密码/记住密码 + 登录。
    fn login_row(&mut self, ui: &mut egui::Ui, compact: bool) {
        self.draw_account_bar(ui);
        ui.add_space(6.0);
        let btn_text = if self.new_account_mode {
            "登录并绑定设备"
        } else {
            "登录"
        };
        if compact {
            ui.label("账号：");
            mobile::text_edit(
                ui,
                "login_username",
                &mut self.username,
                crate::platform::InputKind::Text,
                ui.available_width(),
            );
            ui.label("密码：");
            mobile::text_edit(
                ui,
                "login_password",
                &mut self.password,
                crate::platform::InputKind::Password,
                ui.available_width(),
            );
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.remember, "记住密码");
                if ui
                    .add_enabled(!self.login_busy, primary_btn(btn_text))
                    .clicked()
                {
                    self.do_login();
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label("账号：");
                mobile::text_edit(
                    ui,
                    "login_username",
                    &mut self.username,
                    crate::platform::InputKind::Text,
                    220.0,
                );
                ui.label("密码：");
                mobile::text_edit(
                    ui,
                    "login_password",
                    &mut self.password,
                    crate::platform::InputKind::Password,
                    200.0,
                );
                ui.checkbox(&mut self.remember, "记住密码");
                if ui
                    .add_enabled(!self.login_busy, primary_btn(btn_text))
                    .clicked()
                {
                    self.do_login();
                }
            });
        }
    }
}
