//! 界面主题：深色 Win11 卡片风（XieRun Dark）
//!
//! 设计基线：
//! - 页面底 `#14141A`，面板 `#1B1B22`，卡片 `#212129`，次卡片 `#262630`
//! - 描边 `#32323E`（卡片内部 1px，弱对比）
//! - 主色（蓝紫）`#6C8CFF`，深 `#4E6BF5`，浅底 14% 透明，描边 50% 透明
//! - 文字 `#F3F4F8` / 次 `#9EA2B2` / 弱 `#6E7280`
//! - 渐变：`#5E8BFF → #A56BFF`（品牌条、强调块）
//! - 控件：按钮/输入高 34～38，圆角 9；卡片圆角 14
//!
//! 说明：页面层只通过这些函数取色，改主题不必动页面代码。

use eframe::egui;
use egui::Color32;

// ── 色板 ─────────────────────────────────────────────────
const PAGE: Color32 = Color32::from_rgb(20, 20, 26);
const PANEL: Color32 = Color32::from_rgb(27, 27, 34);
const CARD: Color32 = Color32::from_rgb(33, 33, 41);
const CARD_SOFT: Color32 = Color32::from_rgb(38, 38, 48);
const BORDER: Color32 = Color32::from_rgb(50, 50, 62);
const INK: Color32 = Color32::from_rgb(243, 244, 248);
const INK_DIM: Color32 = Color32::from_rgb(158, 162, 178);
const INK_WEAK: Color32 = Color32::from_rgb(110, 114, 128);
const ACCENT: Color32 = Color32::from_rgb(108, 140, 255);
const ACCENT_DEEP: Color32 = Color32::from_rgb(78, 107, 245);
const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(24, 32, 62, 255);
const ACCENT_BORDER: Color32 = Color32::from_rgb(64, 82, 140);
const GRAD_A: Color32 = Color32::from_rgb(94, 139, 255);
const GRAD_B: Color32 = Color32::from_rgb(165, 107, 255);

/// 成功（文字色 + 深色浅底）。
pub fn ok() -> Color32 {
    Color32::from_rgb(52, 211, 153)
}
pub fn ok_soft() -> Color32 {
    Color32::from_rgb(23, 51, 44)
}
/// 警告（文字色 + 深色浅底）。
pub fn warn() -> Color32 {
    Color32::from_rgb(251, 191, 36)
}
pub fn warn_soft() -> Color32 {
    Color32::from_rgb(56, 45, 22)
}
/// 错误（文字色 + 深色浅底）。
pub fn err() -> Color32 {
    Color32::from_rgb(248, 113, 113)
}
pub fn err_soft() -> Color32 {
    Color32::from_rgb(58, 30, 32)
}

pub fn accent() -> Color32 {
    ACCENT
}
pub fn accent_deep() -> Color32 {
    ACCENT_DEEP
}
pub fn accent_soft() -> Color32 {
    ACCENT_SOFT
}
pub fn accent_border() -> Color32 {
    ACCENT_BORDER
}
pub fn text() -> Color32 {
    INK
}
pub fn text_dim() -> Color32 {
    INK_DIM
}
pub fn text_weak() -> Color32 {
    INK_WEAK
}
pub fn card_soft() -> Color32 {
    CARD_SOFT
}
pub fn card_fill() -> Color32 {
    CARD
}
pub fn panel_fill() -> Color32 {
    PANEL
}
pub fn page_bg() -> Color32 {
    PAGE
}
pub fn border() -> Color32 {
    BORDER
}
/// 正文中性色（页面里当「强调性文字」用）。
pub fn plain() -> Color32 {
    Color32::from_rgb(214, 218, 230)
}

// ── 地图配色（深色底，与外壳一致）────────────────────────
/// 地图画布底色。
pub fn map_bg() -> Color32 {
    Color32::from_rgb(14, 20, 26)
}
/// 道路线。
pub fn map_road() -> Color32 {
    Color32::from_rgb(62, 84, 100)
}
/// 建筑填充。
pub fn map_building() -> Color32 {
    Color32::from_rgb(32, 44, 54)
}
/// 规划路线 / 轨迹主色。
pub fn map_route() -> Color32 {
    Color32::from_rgb(64, 160, 255)
}
/// 打卡点。
pub fn map_point() -> Color32 {
    Color32::from_rgb(255, 201, 60)
}
/// 起点。
pub fn map_start() -> Color32 {
    Color32::from_rgb(46, 212, 122)
}
/// 终点。
pub fn map_end() -> Color32 {
    Color32::from_rgb(255, 91, 91)
}
/// 电子围栏。
pub fn map_fence() -> Color32 {
    Color32::from_rgb(255, 150, 60)
}
/// 地图上的说明文字。
pub fn map_label() -> Color32 {
    Color32::from_rgb(150, 168, 180)
}

// ── 文本样式 ─────────────────────────────────────────────
/// 页面主标题。
pub fn h1(s: impl Into<String>) -> egui::RichText {
    egui::RichText::new(s.into()).size(19.0).strong().color(INK)
}
/// 卡片小标题。
pub fn h2(s: impl Into<String>) -> egui::RichText {
    egui::RichText::new(s.into()).size(14.5).strong().color(INK)
}
/// 说明文字。
pub fn dim(s: impl Into<String>) -> egui::RichText {
    egui::RichText::new(s.into()).size(12.5).color(INK_DIM)
}

// ── 阴影 / 渐变 ──────────────────────────────────────────
/// 柔和投影：深色底上只做轻微层次。
pub fn shadow(blur: f32, y: f32, alpha: u8) -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: egui::vec2(0.0, y),
        blur,
        spread: 0.0,
        color: Color32::from_black_alpha(alpha),
    }
}

/// 蓝紫渐变矩形（无圆角，用于顶部品牌条等矩形区域）。
pub fn gradient_rect(painter: &egui::Painter, rect: egui::Rect) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_top(), GRAD_A);
    mesh.colored_vertex(rect.right_top(), GRAD_B);
    mesh.colored_vertex(rect.right_bottom(), GRAD_B);
    mesh.colored_vertex(rect.left_bottom(), GRAD_A);
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// 带圆角的品牌渐变色块（先圆角实底，再在内侧铺渐变）。
pub fn gradient_block(painter: &egui::Painter, rect: egui::Rect, rounding: egui::Rounding) {
    painter.rect_filled(rect, rounding, GRAD_A);
    let inner = rect.shrink2(egui::vec2(1.0, 1.0));
    if inner.width() > 0.0 && inner.height() > 0.0 {
        painter.with_clip_rect(rect).add(egui::Shape::mesh({
            let mut mesh = egui::Mesh::default();
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(inner.left_top(), GRAD_A);
            mesh.colored_vertex(inner.right_top(), GRAD_B);
            mesh.colored_vertex(inner.right_bottom(), GRAD_B);
            mesh.colored_vertex(inner.left_bottom(), GRAD_A);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
            mesh
        }));
    }
}

// ── 组件 ─────────────────────────────────────────────────
/// 卡片容器：深色底、1px 描边、圆角 14、内边距 14、轻投影。
pub fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .rounding(egui::Rounding::same(14.0))
        .inner_margin(egui::Margin::same(14.0))
        .shadow(shadow(18.0, 3.0, 60))
}

/// 次级卡片（嵌在卡片里的分区块）。
pub fn soft_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD_SOFT)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::same(10.0))
}

/// 分区标题（左侧渐变小竖条 + 标题文字）。
pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 15.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, egui::Rounding::same(2.0), ACCENT);
        ui.label(h2(text));
    });
}

/// 主按钮：蓝紫渐变实底（圆角 9）、白字、高 38。
pub fn primary_btn(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .color(Color32::WHITE)
            .strong()
            .size(14.0),
    )
    .fill(ACCENT_DEEP)
    .stroke(egui::Stroke::new(1.0_f32, ACCENT))
    .rounding(egui::Rounding::same(9.0))
    .min_size(egui::vec2(0.0, 38.0))
}

/// 次按钮：深底、1px 描边、高 34、次文字色。
pub fn ghost_btn(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).color(INK).size(13.0))
        .fill(CARD_SOFT)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .rounding(egui::Rounding::same(9.0))
        .min_size(egui::vec2(0.0, 34.0))
}

/// 状态徽章：胶囊底 + 12px 文字（成功/警告/错误/中性四种色由调用方给）。
pub fn badge(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::none()
        .fill(bg)
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_white_alpha(10)))
        .rounding(egui::Rounding::same(999.0))
        .inner_margin(egui::Margin::symmetric(9.0, 4.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(fg).size(12.0));
        });
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();

    v.panel_fill = PAGE;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(16, 16, 21);
    v.faint_bg_color = CARD_SOFT;
    v.code_bg_color = Color32::from_rgb(24, 24, 30);
    v.override_text_color = Some(INK);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_BORDER;
    v.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);
    v.window_stroke = egui::Stroke::new(1.0_f32, BORDER);
    v.window_rounding = egui::Rounding::same(14.0);
    v.window_shadow = shadow(28.0, 8.0, 120);
    v.popup_shadow = shadow(22.0, 6.0, 110);
    v.menu_rounding = egui::Rounding::same(10.0);

    v.widgets.noninteractive.bg_fill = CARD_SOFT;
    v.widgets.noninteractive.weak_bg_fill = CARD_SOFT;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, INK_DIM);

    v.widgets.inactive.bg_fill = CARD_SOFT;
    v.widgets.inactive.weak_bg_fill = CARD_SOFT;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, INK);

    v.widgets.hovered.bg_fill = Color32::from_rgb(44, 44, 58);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 44, 58);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT_BORDER);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.1_f32, Color32::WHITE);

    v.widgets.active.bg_fill = ACCENT_DEEP;
    v.widgets.active.weak_bg_fill = ACCENT_DEEP;
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = egui::Stroke::new(1.2_f32, Color32::WHITE);

    v.widgets.open.bg_fill = CARD_SOFT;
    v.widgets.open.weak_bg_fill = CARD_SOFT;
    v.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT_BORDER);
    v.widgets.open.fg_stroke = egui::Stroke::new(1.1_f32, INK);

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = egui::Rounding::same(9.0);
    }

    style.spacing.item_spacing = egui::vec2(11.0, 9.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.menu_margin = egui::Margin::same(9.0);
    style.spacing.window_margin = egui::Margin::same(14.0);
    #[cfg(target_os = "android")]
    {
        style.spacing.interact_size = egui::vec2(46.0, 46.0);
        style.spacing.button_padding = egui::vec2(15.0, 11.0);
    }

    // 页面文字仅作展示：不可选中、不可复制，输入框不受影响
    style.interaction.selectable_labels = false;
    style.interaction.multi_widget_text_select = false;

    ctx.set_style(style);
}
