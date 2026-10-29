//! UI 层：App 状态机 + 顶栏 + 标签页 + 底部状态栏。
//!
//! 后台任务与消息处理在独立 impl App 块中。
//!
//! 线程模型：所有网络/长任务 spawn thread + mpsc channel<String> 回 UI，
//! 带 200ms 轮询重绘。带内消息协议：
//! __IP_DONE__ / __LOGIN_DONE__ / __RUN_DONE__ / __AI_DONE__ / __RECORDS__ / __AI_LIST__
//! __SEMESTER__ / __CHEAT__ / __RANK__ / __USER__ / __AI_RECORDS__

pub mod about;
pub mod accounts;
pub mod ai;
pub mod data;
pub mod device;
pub mod fonts;
pub mod jobs;
pub mod log;
pub mod map;
pub mod mobile;
pub mod msgs;
pub mod osm;
pub mod personal;
pub mod records;
pub mod run;
pub mod theme;
pub mod user;
pub mod workbench;

use crate::api::model::{self, AccountStore, Config, HeaderIdentity, Session};
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

pub const IP: &str = "__IP_DONE__";
pub const LOGIN_DONE: &str = "__LOGIN_DONE__";
pub const RUN_DONE: &str = "__RUN_DONE__";
pub const AI_DONE: &str = "__AI_DONE__";
pub const RECORDS: &str = "__RECORDS__";
pub const AI_LIST: &str = "__AI_LIST__";
pub const AI_DETAIL: &str = "__AI_DETAIL__";
pub const SEMESTER: &str = "__SEMESTER__";
pub const SCORE: &str = "__SCORE__";
pub const CHEAT: &str = "__CHEAT__";
pub const RANK: &str = "__RANK__";
pub const USER: &str = "__USER__";
pub const AI_RECORDS: &str = "__AI_RECORDS__";
pub const RUN_DETAIL: &str = "__RUN_DETAIL__";
pub const UPDATE_CHK: &str = "__UPDATE_CHK__";
pub const UPDATE_PROG: &str = "__UPDATE_PROG__";
pub const UPDATE_DONE: &str = "__UPDATE_DONE__";
pub const FENCE_DONE: &str = "__FENCE_DONE__";
pub const POINTS_DONE: &str = "__POINTS_DONE__";
pub const LOCATION_DONE: &str = "__LOCATION_DONE__";

/// 提交结果弹窗。
pub struct PopupInfo {
    pub title: String,
    pub lines: Vec<String>,
}

/// 各页面标题与副标题（外壳页头显示）。
pub const PAGE_TITLES: [(&str, &str); 9] = [
    ("跑步", "选好参数，剩下的交给程序"),
    ("AI 运动", "批量补跑与次数规划"),
    ("运动记录", "历史明细与有效性"),
    ("数据", "学期进度与记录自查"),
    ("我的", "账号资料与个人数据"),
    ("设备信息", "机型指纹与定位锚点"),
    ("运行日志", "完整过程跟踪"),
    ("路网", "校园路线绘制与导入"),
    ("关于", "版本信息与作者"),
];

/// 顶层 App。
pub struct App {
    /// 路网代次：每次路网加载成功 +1，用来驱动跑步页刷新
    pub net_version: u64,
    pub tx: Sender<String>,
    pub rx: Receiver<String>,
    pub log: log::LogStore,
    pub font_loaded: Option<String>,

    pub ip: String,
    pub identity: HeaderIdentity,
    pub device_buf: HeaderIdentity,
    pub session: Option<Session>,
    pub config: Config,
    pub accounts: AccountStore,
    pub new_account_mode: bool,

    pub username: String,
    pub password: String,
    pub remember: bool,
    pub login_busy: bool,
    pub run_busy: bool,
    pub ai_busy: bool,
    pub records_busy: bool,
    pub data_busy: bool,
    pub user_busy: bool,
    pub status: String,
    pub popup: Option<PopupInfo>,
    pub ai_confirm: Option<ai::AiBatchPlan>,

    pub tab: usize,
    pub run_page: run::RunPage,
    pub ai_page: ai::AiPage,
    pub records_page: records::RecordsPage,
    pub data_page: data::DataPage,
    pub user_page: user::UserPage,
    pub device_page: device::DevicePage,
    pub update: about::UpdateUi,

    /// 已加载的 OSM 路网（真实道路路由 + 地图显示共用）。
    pub network: Option<Arc<route_planner::RoadGraph>>,
    pub osm_page: osm::OsmPage,
    net_tx: Sender<Result<Arc<route_planner::RoadGraph>, String>>,
    net_rx: Receiver<Result<Arc<route_planner::RoadGraph>, String>>,
    /// 打卡点重新获取结果（后台线程 → UI）
    pts_tx: Sender<(usize, Vec<(String, f64, f64)>, String)>,
    pts_rx: Receiver<(usize, Vec<(String, f64, f64)>, String)>,
}

impl eframe::App for App {
    #[cfg(target_os = "android")]
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        crate::platform::apply_safe_area(ctx, input);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let n = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static FRAMES: AtomicU64 = AtomicU64::new(0);
            FRAMES.fetch_add(1, Ordering::Relaxed) + 1
        };
        let trace = n <= 3 || n % 60 == 0;
        let _ = trace;
        if n % 600 == 1 {
            crate::gui_log(&format!(
                "帧={n} tab={} 日志行={} 内存={}MB 运行中(run/ai/login)={}/{}/{}",
                self.tab,
                self.log.len(),
                crate::platform::rss_mb(),
                self.run_busy,
                self.ai_busy,
                self.login_busy
            ));
        }
        self.poll_messages();
        #[cfg(target_os = "android")]
        self.poll_device_info();
        crate::platform::set_keep_screen_on(self.run_busy || self.ai_busy || self.login_busy);
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        // ── 顶栏（外壳皮肤，见 ui/workbench.rs）─────────────────
        self.workbench_top_bar(ctx);

        // ── 左侧导航（宽窗口；窄窗口改在顶栏）──────────────────
        self.workbench_sidebar(ctx);

        // ── 底部状态栏 ──────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar")
            .frame(
                egui::Frame::none()
                    .fill(theme::panel_fill())
                    .inner_margin(egui::Margin::symmetric(16.0, 6.0)),
            )
            .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("日志 {} 行", self.log.len()));
                ui.separator();
                if let Some(s) = &self.data_page.semester {
                    ui.label(format!(
                        "学期 {}：有效 {}/{} 次",
                        s.sname, s.semester_valid_count, s.semester_count
                    ));
                }
                if let Some(c) = &self.data_page.cheat {
                    if c.is_clean() {
                        ui.colored_label(theme::ok(), "自查正常");
                    } else {
                        ui.colored_label(theme::err(), format!("已标记：{}", c.self_brief()));
                    }
                }
                ui.separator();
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("v{}", crate::platform::version_name()))
                            .small()
                            .color(theme::text_dim()),
                    );
                });
            });
            ui.add_space(2.0);
        });

        // ── 内容区（导航在侧栏，这里渲染当前页）──────────────────
        egui::CentralPanel::default()
            .frame(workbench::page_frame())
            .show(ctx, |ui| {
                let (title, subtitle) = PAGE_TITLES[self.tab.min(PAGE_TITLES.len() - 1)];
                workbench::page_header(ui, title, subtitle);
                match self.tab {
                    0 => {
                        personal::hero(self, ui);
                        self.draw_run(ui)
                    }
                    1 => self.draw_ai(ui),
                    2 => self.draw_records(ui),
                    3 => self.draw_data(ui),
                    4 => self.draw_user(ui),
                    5 => self.draw_device(ui),
                    6 => self.log.render(ui),
                    7 => self.draw_osm_page(ui),
                    _ => self.draw_about(ui),
                }
            });


        if let Some(p) = &self.popup {
            let mut open = true;
            let mut close = false;
            egui::Window::new(egui::RichText::new(&p.title).strong())
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width((ctx.screen_rect().width() - 32.0).min(520.0));
                    for line in &p.lines {
                        ui.label(line);
                    }
                    ui.add_space(6.0);
                    if ui.button("关闭").clicked() {
                        close = true;
                    }
                });
            if !open || close {
                self.popup = None;
            }
        }
        crate::platform::sync_clipboard(ctx);
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (net_tx, net_rx) = std::sync::mpsc::channel();
    let (pts_tx, pts_rx) = std::sync::mpsc::channel();
        let font_loaded = fonts::install(&cc.egui_ctx);
        workbench::apply(&cc.egui_ctx);
        let accounts = model::load_accounts();
        let active_account = accounts.active_account().cloned();
        let identity = active_account
            .as_ref()
            .map(|account| account.identity.clone())
            .unwrap_or_else(model::load_identity);
        let device_buf = identity.clone();
        let session = active_account
            .as_ref()
            .and_then(|account| account.session.clone())
            .unwrap_or_default();
        let config = model::load_config();
        let username = active_account
            .as_ref()
            .map(|account| account.username.clone())
            .unwrap_or_default();
        let password = active_account
            .as_ref()
            .map(|account| account.password.clone())
            .unwrap_or_default();
        let remember = active_account
            .as_ref()
            .map(|account| account.remember)
            .unwrap_or(false);
        let logged_in = session.is_logged_in();
        let new_account_mode = active_account
            .as_ref()
            .map(|account| account.is_blank_draft())
            .unwrap_or(true)
            && !logged_in;

        let mut app = Self {
            tx: tx.clone(),
            rx,
            log: log::LogStore::default(),
            font_loaded,
            ip: String::new(),
            identity,
            device_buf,
            session: logged_in.then_some(session),
            config: config.clone(),
            accounts,
            new_account_mode,
            username,
            password,
            remember,
            login_busy: false,
            run_busy: false,
            ai_busy: false,
            net_version: 0,
            records_busy: false,
            data_busy: false,
            user_busy: false,
            status: "就绪".into(),
            popup: None,
            ai_confirm: None,
            tab: 0,
            run_page: run::RunPage {
                net_version: 0,
                dist_min: config.dist_min,
                dist_max: config.dist_max,
                pace_min: config.pace_min,
                pace_max: config.pace_max,
                manual_altitude: config
                    .manual_altitude_range
                    .map(|r| format!("{}-{}", r.min_m, r.max_m))
                    .or_else(|| config.manual_altitude.map(|v| v.to_string()))
                    .unwrap_or_default(),
                start_mode: 0,
                days_ago: 0,
                hour: 12,
                minute: 0,
                face_check: config.face_check,
                plan: None,
                route_mode: crate::track::generate_road::RouteMode::from_str(&config.route_mode),
                map: map::MapState::default(),
                preview: None,
                legacy_preview: None,
                preview_stale: true,
                points_requested: false,
                map_fitted: false,
                map_canvas: [0.0, 0.0],
            },
            ai_page: ai::AiPage {
                days: 1,
                per_day: 1,
                ..Default::default()
            },
            records_page: records::RecordsPage::default(),
            data_page: data::DataPage::default(),
            user_page: user::UserPage::default(),
            device_page: device::DevicePage::default(),
            update: about::UpdateUi::default(),
            network: None,
            osm_page: osm::OsmPage {
                path: config.effective_osm_path(),
                ..Default::default()
            },
            net_tx,
            net_rx,
            pts_tx,
            pts_rx,
        };
        if app.font_loaded.is_none() {
            app.log
                .push("未找到中文字体（msyh/simhei/simsun），界面中文可能显示为方块");
        }
        // 上次更新残留的 .old/.new 顺手清掉
        crate::update::cleanup_residue();
        app.fetch_ip();
        #[cfg(target_os = "android")]
        crate::android::request_device_info(true);
        // AI 项目列表先上缓存，网络刷新后覆盖
        app.ai_page.list = crate::api::model::load_ai_sports().unwrap_or_default();
        if app.session.is_some() {
            app.refresh_data_page();
            app.refresh_records();
            app.refresh_user_page();
            app.refresh_ai_list();
            app.refresh_fence();
            app.refresh_points();
            app.start_school_location_detection(true);
        }
        // 已配置 OSM 路网则启动时后台加载
        let osm_start = app.config.effective_osm_path();
        if !osm_start.is_empty() {
            let path = osm_start;
            app.load_osm_async(path);
        }
        app
    }

    /// 后台任务（线程 + 日志 channel）。
    pub fn spawn_job<F>(&self, f: F)
    where
        F: FnOnce(Sender<String>) + Send + 'static,
    {
        let tx = self.tx.clone();
        std::thread::spawn(move || f(tx));
    }

    pub fn logger(tx: Sender<String>) -> impl FnMut(&str) {
        move |s: &str| {
            tx.send(s.to_string()).ok();
        }
    }
/// 当前路网是不是「本软件自己导出的 route.osm」。
///
/// 是的话就不要再把它当道路底图画一遍——否则路线旁边会多出一条重复的细线。
pub fn network_is_self_route(&self) -> bool {
    let p = self.config.effective_osm_path().to_lowercase();
    p.ends_with("route.osm")
}
}
