//! 「路网」导入页：选择 OSM 文件 → 后台解析 → 画布预览 + 统计。

use eframe::egui;
use std::sync::Arc;

use route_planner::{Coord, RoadGraph};

use super::map::{collect_bounds, MapState};
use super::{theme, App};

pub struct OsmPage {
    pub path: String,
    pub busy: bool,
    pub status: String,
    pub map: MapState,
    pub fitted: bool,
    /// 已识别的打卡点（名称, BD-09 纬度, BD-09 经度）
    pub points: Vec<(String, f64, f64)>,
    /// 是否尝试过读取点库
    pub points_loaded: bool,
    /// 自动生成路线的目标距离（km）
    pub target_km: f32,
    /// 崎岖度 0~1
    pub rugged: f32,
    /// 生成的路线（BD-09 折线）
    pub route: Vec<(f64, f64)>,
    /// 生成路线长度（米）
    pub route_len: f64,
    /// 手动画线模式
    pub manual: bool,
    /// 手动点的 (BD-09 纬度, 经度)
    pub manual_pts: Vec<(f64, f64)>,
}

impl Default for OsmPage {
    fn default() -> Self {
        OsmPage {
            path: String::new(),
            busy: false,
            status: String::new(),
            map: MapState::default(),
            fitted: false,
            points: Vec::new(),
            points_loaded: false,
            target_km: 3.5,
            rugged: std::env::var("XIE_RUGGED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.62),
            route: Vec::new(),
            route_len: 0.0,
            manual: false,
            manual_pts: Vec::new(),
        }
    }
}

fn ring_to_pts(ring: &[Coord]) -> Vec<(f64, f64)> {
    ring.iter().map(|c| (c.lat, c.lon)).collect()
}

impl App {
    pub fn draw_osm_page(&mut self, ui: &mut egui::Ui) {
        let mut load_now = false;

        ui.horizontal(|ui| {
            ui.label("OSM 文件：");
            ui.add_sized(
                [340.0, 26.0],
                egui::TextEdit::singleline(&mut self.osm_page.path)
                    .hint_text("拖入 .osm 文件或填写路径"),
            );
            #[cfg(not(target_os = "android"))]
            if ui.button("浏览…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("OSM", &["osm", "xml", "pbf"])
                    .pick_file()
                {
                    self.osm_page.path = p.display().to_string();
                }
            }
            let can_load = !self.osm_page.path.is_empty() && !self.osm_page.busy;
            if ui
                .add_enabled(can_load, theme::primary_btn("加载路网"))
                .clicked()
            {
                load_now = true;
            }
        });

        // ── 打卡点：来自软件自动识别的点库 ──────────────────
        if !self.osm_page.points_loaded {
            self.load_checkpoints();
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("打卡点：{} 个", self.osm_page.points.len()));
            if ui.small_button("刷新").clicked() {
                self.load_checkpoints();
            }
            if ui
                .add_enabled(!self.osm_page.busy, theme::ghost_btn("重新获取"))
                .on_hover_text("向服务端重新拉取一次打卡点（约 5 分钟限 3 次）")
                .clicked()
            {
                self.refetch_points_from_server();
            }
            ui.separator();
            ui.label("目标距离：");
            ui.add(
                egui::DragValue::new(&mut self.osm_page.target_km)
                    .speed(0.05)
                    .range(0.5..=20.0)
                    .suffix(" km"),
            );
            ui.label("崎岖度：");
            ui.add_sized(
                [120.0, 20.0],
                egui::Slider::new(&mut self.osm_page.rugged, 0.0..=1.0).show_value(false),
            );
            let can_gen = self.network.is_some() && !self.osm_page.points.is_empty();
            if ui
                .add_enabled(can_gen, theme::primary_btn("按打卡点生成路线"))
                .clicked()
            {
                self.generate_route_from_points();
            }
            ui.separator();
            ui.checkbox(&mut self.osm_page.manual, "手动画线");
            if self.osm_page.manual {
                if ui.add(theme::ghost_btn("撤销")).clicked() {
                    self.osm_page.manual_pts.pop();
                }
                if ui.add(theme::ghost_btn("闭合生成")).clicked() {
                    self.finish_manual_route();
                }
                if ui.add(theme::ghost_btn("清空")).clicked() {
                    self.osm_page.manual_pts.clear();
                    self.osm_page.status = "已清空手绘点".into();
                }
            }
            if !self.osm_page.route.is_empty() {
                ui.colored_label(
                    theme::ok(),
                    format!("路线 {:.0} m · {} 点", self.osm_page.route_len, self.osm_page.points.len()),
                );
            }
            if self.network.is_none() {
                ui.colored_label(theme::warn(), "（先加载 OSM 路网）");
            }
        });

        // 拖拽导入
        let dropped: Vec<String> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.as_ref().map(|p| p.display().to_string()))
                .collect()
        });
        if let Some(p) = dropped.into_iter().find(|p| !p.is_empty()) {
            self.osm_page.path = p;
            if !self.osm_page.busy {
                load_now = true;
            }
        }

        if !self.osm_page.status.is_empty() {
            ui.colored_label(theme::plain(), &self.osm_page.status);
        }

        // 统计
        if let Some(net) = &self.network {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "节点 {} · 有向边 {} · 建筑 {}",
                    net.graph.node_count(),
                    net.graph.edge_count(),
                    net.buildings.len()
                ));
                if let Ok(meta) = std::fs::metadata(&self.osm_page.path) {
                    ui.label(format!("· 文件 {:.1} KB", meta.len() as f64 / 1024.0));
                }
            });
        }

        ui.add_space(4.0);
        if self.network.is_some() {
            super::map::legend(
                ui,
                &[
                    ("道路", theme::map_road()),
                    ("建筑", theme::map_building()),
                ],
            );
        }
        let rect = ui.available_rect_before_wrap();
        let (response, painter) = ui.allocate_painter(
            egui::Vec2::new(rect.width().max(200.0), rect.height().max(220.0)),
            egui::Sense::click_and_drag(),
        );
        let canvas = response.rect;

        // 手动画线：左键点地图加点，右键撤销一个
        if self.osm_page.manual {
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let (la, lo) = self.osm_page.map.from_screen(canvas, pos);
                    if la.abs() > 1.0 && lo.abs() > 1.0 {
                        self.osm_page.manual_pts.push((la, lo));
                        self.osm_page.status = format!(
                            "已加第 {} 个点（右键撤销）",
                            self.osm_page.manual_pts.len()
                        );
                    }
                }
            }
            if response.secondary_clicked() {
                self.osm_page.manual_pts.pop();
            }
        }

        if let Some(net) = &self.network {
            if !self.osm_page.fitted {
                let self_route = self.network_is_self_route();
                let mut bounds: Vec<(f64, f64)> = if self_route {
                    Vec::new()
                } else {
                    collect_bounds(
                        &net.edges_deg()
                            .iter()
                            .map(|e| ring_to_pts(e))
                            .collect::<Vec<_>>(),
                    )
                };
                for b in &net.buildings {
                    bounds.extend(ring_to_pts(b));
                }
                bounds.extend(self.osm_page.points.iter().map(|(_, la, lo)| (*la, *lo)));
                bounds.extend(self.osm_page.route.iter().copied());
                if !bounds.is_empty() {
                    self.osm_page.map.fit(canvas, &bounds);
                }
                self.osm_page.fitted = true;
            }
        }

        painter.rect_filled(
            canvas,
            egui::Rounding::ZERO,
            theme::map_bg(),
        );
        if let Some(net) = &self.network {
            let road_color = theme::map_road();
            for e in net.edges_deg().into_iter().filter(|_| !self.network_is_self_route()) {
                self.osm_page.map.draw_polyline(
                    &painter,
                    canvas,
                    &ring_to_pts(&e),
                    road_color,
                    1.0,
                );
            }
            let bld_color = theme::map_building();
            for b in &net.buildings {
                self.osm_page
                    .map
                    .draw_polygon(&painter, canvas, &ring_to_pts(b), bld_color, 1.0);
            }
            // 生成的路线
            if !self.osm_page.route.is_empty() {
                self.osm_page.map.draw_polyline(
                    &painter,
                    canvas,
                    &self.osm_page.route,
                    theme::map_route(),
                    2.5,
                );
            }
        } else {
            painter.text(
                canvas.center(),
                egui::Align2::CENTER_CENTER,
                "加载 OSM 路网后可预览道路与建筑",
                egui::FontId::proportional(14.0),
                theme::map_label(),
            );
        }

        // 手绘点预览
        if !self.osm_page.manual_pts.is_empty() {
            if self.osm_page.manual_pts.len() >= 2 {
                self.osm_page.map.draw_polyline(
                    &painter,
                    canvas,
                    &self.osm_page.manual_pts.clone(),
                    theme::warn(),
                    2.0,
                );
            }
            for (i, p) in self.osm_page.manual_pts.clone().iter().enumerate() {
                let sp = self.osm_page.map.to_screen(canvas, p.0, p.1);
                painter.circle_filled(sp, 3.5, theme::warn());
                painter.text(
                    sp + egui::vec2(6.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{}", i + 1),
                    egui::FontId::proportional(10.0),
                    theme::map_label(),
                );
            }
        }

        // 打卡点：圆点 + 名称
        for (name, la, lo) in &self.osm_page.points {
            self.osm_page
                .map
                .draw_point(&painter, canvas, *la, *lo, theme::map_point(), 4.0);
            let p = self.osm_page.map.to_screen(canvas, *la, *lo);
            painter.text(
                p + egui::vec2(6.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                name,
                egui::FontId::proportional(11.0),
                theme::map_label(),
            );
        }

        self.osm_page.map.interact(ui, canvas);

        if load_now {
            let path = self.osm_page.path.clone();
            self.load_osm_async(path);
        }
    }

    /// 手动画线闭合：把点连成闭环、轻度平滑去掉过冲，导出 route.osm 并设为当前路网。
    pub fn finish_manual_route(&mut self) {
        let pts = self.osm_page.manual_pts.clone();
        if pts.len() < 3 {
            self.osm_page.status = "至少要点 3 个点才能成环".into();
            return;
        }
        let step = 2.0;
        let mut ring = crate::track::auto_route::spline_closed(&pts, step);
        ring = crate::track::auto_route::resample_closed(&ring, step);
        let mut guard = 0;
        while crate::track::auto_route::self_intersects(&ring) && guard < 200 {
            let n = ring.len();
            let mut next = ring.clone();
            for i in 0..n {
                let a = ring[(i + n - 1) % n];
                let b = ring[(i + 1) % n];
                next[i] = (
                    (a.0 + 2.0 * ring[i].0 + b.0) / 4.0,
                    (a.1 + 2.0 * ring[i].1 + b.1) / 4.0,
                );
            }
            ring = next;
            guard += 1;
        }
        let mut len = 0.0;
        for i in 0..ring.len() {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            let dy = (a.0 - b.0) * crate::track::geom::MET_PER_DEG_LAT;
            let dx = (a.1 - b.1) * crate::track::geom::MET_PER_DEG_LNG;
            len += (dx * dx + dy * dy).sqrt();
        }
        let mut disp = ring.clone();
        if let Some(first) = ring.first().copied() {
            disp.push(first);
        }
        self.osm_page.route = disp;
        self.osm_page.route_len = len;
        self.osm_page.fitted = false;
        match self.export_route_osm(&ring) {
            Ok(p) => {
                self.osm_page.status = format!(
                    "手绘闭环：{:.0} m · 平滑后 {} 点（已导出路网 {}）",
                    len,
                    ring.len(),
                    p
                )
            }
            Err(e) => {
                self.osm_page.status =
                    format!("手绘闭环：{:.0} m（导出失败：{}）", len, e);
            }
        }
    }

    /// 后台线程加载 OSM 路网，成功后经 `net_rx` 回传。
    pub fn load_osm_async(&mut self, path: String) {
        self.osm_page.path = path.clone();
        self.osm_page.busy = true;
        self.osm_page.status = format!("加载中：{path} …");
        self.config.osm_path = path.clone();
        let _ = crate::api::model::save_config(&self.config);
        let tx = self.net_tx.clone();
        std::thread::spawn(move || {
            let res = (|| -> Result<Arc<RoadGraph>, String> {
                let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
                let net = route_planner::load_osm(&bytes)?;
                Ok(Arc::new(net))
            })();
            let _ = tx.send(res);
        });
    }
    /// 读取打卡点：软件缓存的 `points_cache.json` + 手工/扫描累积的 `checkpoints.json`。
    pub fn load_checkpoints(&mut self) {
        let dir = crate::platform::data_dir();
        let mut out: Vec<(String, f64, f64)> = Vec::new();
        let mut push = |out: &mut Vec<(String, f64, f64)>, name: String, lat: f64, lon: f64| {
            if !(lat.abs() > 1.0 && lon.abs() > 1.0) {
                return;
            }
            if out
                .iter()
                .any(|(_, la, lo)| (*la - lat).abs() < 1e-5 && (*lo - lon).abs() < 1e-5)
            {
                return;
            }
            out.push((name, lat, lon));
        };

        // 1) 运行期缓存（最近一次接口下发）
        if let Ok(text) = std::fs::read_to_string(dir.join("points_cache.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(list) = v.get("points").and_then(|p| p.as_array()) {
                    for p in list {
                        let name = p
                            .get("pointName")
                            .and_then(|s| s.as_str())
                            .unwrap_or("打卡点")
                            .to_string();
                        let lat = p.get("lat").and_then(|s| s.as_f64()).unwrap_or(0.0);
                        let lon = p.get("lon").and_then(|s| s.as_f64()).unwrap_or(0.0);
                        push(&mut out, name, lat, lon);
                    }
                }
            }
        }

        // 2) 累积点库（可手工编辑）
        if let Ok(text) = std::fs::read_to_string(dir.join("checkpoints.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(list) = v.get("points").and_then(|p| p.as_array()) {
                    for p in list {
                        let name = p
                            .get("name")
                            .and_then(|s| s.as_str())
                            .unwrap_or("打卡点")
                            .to_string();
                        let lat = p.get("lat").and_then(|s| s.as_f64()).unwrap_or(0.0);
                        let lon = p.get("lon").and_then(|s| s.as_f64()).unwrap_or(0.0);
                        push(&mut out, name, lat, lon);
                    }
                }
            }
        }

        self.osm_page.points = out;
        self.osm_page.points_loaded = true;
        self.osm_page.fitted = false;
        self.osm_page.status = format!("已载入 {} 个打卡点", self.osm_page.points.len());
    }

    /// 按打卡点自动生成环线：必经点 + 崎岖插点，沿道路拓扑走，绕开建筑。
    pub fn generate_route_from_points(&mut self) {
        let Some(net) = self.network.clone() else {
            self.osm_page.status = "请先加载 OSM 路网".into();
            return;
        };
        if self.osm_page.points.is_empty() {
            self.osm_page.status = "没有可用的打卡点".into();
            return;
        }
        let pts: Vec<(f64, f64)> = self
            .osm_page
            .points
            .iter()
            .map(|(_, la, lo)| (*la, *lo))
            .collect();
        let dist = (self.osm_page.target_km as f64 * 1000.0).max(500.0);
        let dur = (dist / 3.2).max(120.0) as i64;
        crate::track::generate_road::set_rugged(self.osm_page.rugged as f64);
        let seed = crate::crypto::envelope::now_ms() as u64;
        let _ = dur;
        // 建筑多边形只用于「绕障」，先把路网对齐到 BD-09
        let mut g = (*net).clone();
        crate::track::generate_road::align_network(&mut g);
        let buildings: Vec<Vec<(f64, f64)>> = g
            .buildings
            .iter()
            .map(|ring| ring.iter().map(|c| (c.lat, c.lon)).collect())
            .collect();
        match crate::track::auto_route::build_auto_route(
            &pts,
            &buildings,
            dist,
            self.osm_page.rugged as f64,
            seed,
        ) {
            Ok(r) => {
                // 画面上始终显示「闭合的一圈」（多圈叠加只影响提交里程）
                let mut ring = r.base_lap.clone();
                if let Some(first) = ring.first().copied() {
                    ring.push(first);
                }
                self.osm_page.route = ring;
                self.osm_page.route_len = r.length_m;
                self.osm_page.fitted = false;
                // 顺手导出成路网文件：跑步页「真实道路」模式就能直接沿这条环线跑
                let line = r.base_lap.clone();
                let exported = self.export_route_osm(&line);
                self.osm_page.status = match exported {
                    Ok(p) => format!(
                        "已生成路线：{:.0} m · {} 个打卡点 · 崎岖度 {:.0}% · {:.1} 圈（已导出路网 {}）",
                        r.length_m,
                        self.osm_page.points.len(),
                        self.osm_page.rugged * 100.0,
                        r.laps,
                        p
                    ),
                    Err(e) => format!(
                        "已生成路线：{:.0} m · {} 个打卡点 · {:.1} 圈（导出路网失败：{}）",
                        r.length_m,
                        self.osm_page.points.len(),
                        r.laps,
                        e
                    ),
                };
            }
            Err(e) => {
                self.osm_page.route.clear();
                self.osm_page.route_len = 0.0;
                self.osm_page.status = format!("生成失败：{e}");
            }
        }
    }
    /// 把生成的环线写成 `route.osm` 并设为当前路网：跑步页「真实道路」模式即刻可用。
    fn export_route_osm(&mut self, route: &[(f64, f64)]) -> Result<String, String> {
        if route.len() < 2 {
            return Err("路线为空".into());
        }
        let dir = crate::platform::data_dir();
        let path = dir.join("route.osm");
        let mut xml = String::with_capacity(route.len() * 90 + 512);
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<osm version=\"0.6\" generator=\"XieRun\">\n");
        // OSM 用 WGS84；软件内部是 BD-09，写盘前换算
        for (i, (lat, lon)) in route.iter().enumerate() {
            let (wlat, wlon) = crate::track::geom::bd09_to_wgs84(*lat, *lon);
            xml.push_str(&format!(
                "  <node id=\"{}\" lat=\"{:.8}\" lon=\"{:.8}\"/>\n",
                i + 1,
                wlat,
                wlon
            ));
        }
        xml.push_str("  <way id=\"1\">\n");
        for i in 0..route.len() {
            xml.push_str(&format!("    <nd ref=\"{}\"/>\n", i + 1));
        }
        xml.push_str("    <nd ref=\"1\"/>\n    <tag k=\"highway\" v=\"footway\"/>\n    <tag k=\"name\" v=\"XieRun 打卡点环线\"/>\n  </way>\n</osm>\n");
        std::fs::write(&path, xml).map_err(|e| format!("写入失败：{e}"))?;
        let mut ol = String::new();
        for (lat, lon) in route.iter() {
            ol.push_str(&format!("{:.8},{:.8}\n", lat, lon));
        }
        std::fs::write(dir.join("route.outline"), ol).ok();

        let p = path.to_string_lossy().to_string();
        self.osm_page.path = p.clone();
        let mut cfg = crate::api::model::load_config();
        cfg.osm_path = p.clone();
        crate::api::model::save_config(&cfg).ok();
        // 立刻加载成当前路网（预览与提交共用这一份）
        self.load_osm_async(p.clone());
        Ok(p)
    }
    /// 把当前点库写回 `checkpoints.json`。
    pub fn save_checkpoints(&self) {
        let dir = crate::platform::data_dir();
        let arr: Vec<serde_json::Value> = self
            .osm_page
            .points
            .iter()
            .map(|(name, lat, lon)| {
                serde_json::json!({ "name": name, "lat": lat, "lon": lon })
            })
            .collect();
        let doc = serde_json::json!({
            "note": "打卡点点位库（BD-09）。软件会与 points_cache.json 合并显示。",
            "points": arr,
        });
        if let Ok(text) = serde_json::to_string_pretty(&doc) {
            let _ = std::fs::write(dir.join("checkpoints.json"), text);
        }
    }

    /// 主动向服务端重新获取打卡点（后台线程，不卡界面）。
    pub fn refetch_points_from_server(&mut self) {
        let Some(session) = self.session.clone() else {
            self.osm_page.status = "未登录：请先登录再从服务端获取打卡点".into();
            return;
        };
        if !session.is_logged_in() {
            self.osm_page.status = "未登录：请先登录再从服务端获取打卡点".into();
            return;
        }
        let Ok(anchor) = self.identity.anchor_coordinate() else {
            self.osm_page.status = "定位未设置：请先到【设备信息】页设置城市与锚点".into();
            return;
        };
        let identity = self.identity.clone();
        let tx = self.pts_tx.clone();
        let want = (self.osm_page.target_km * 1000.0).round() as i64;
        self.osm_page.status = "正在从服务端重新获取打卡点…".into();
        std::thread::spawn(move || {
            // 丢掉缓存，强制走网络
            let _ = std::fs::remove_file(crate::platform::data_dir().join("points_cache.json"));
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let mut quiet = |_: &str| {};
            let mut last = String::from("服务端未返回打卡点");
            for dist in [want.max(1000), 3000, 1000] {
                let q = crate::api::points::PointsQuery {
                    sport_type: 1,
                    distance_m: dist,
                    run_area_id: None,
                };
                match crate::api::points::fetch_points_query(&mut client, anchor, &q, &mut quiet) {
                    Ok(list) if !list.is_empty() => {
                        let mut out: Vec<(String, f64, f64)> = Vec::new();
                        for p in &list {
                            let name = p
                                .get("pointName")
                                .and_then(|v| v.as_str())
                                .unwrap_or("打卡点")
                                .to_string();
                            if let Some((lat, lon)) = crate::api::points::point_bd(p) {
                                out.push((name, lat, lon));
                            }
                        }
                        let _ = tx.send((
                            out.len(),
                            out,
                            format!("已从服务端重新获取 {} 个打卡点（距离档 {dist} m）", list.len()),
                        ));
                        return;
                    }
                    Ok(_) => last = "服务端返回空点位列表".into(),
                    Err(e) => last = e,
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            let _ = tx.send((0, Vec::new(), format!("重新获取失败：{last}")));
        });
    }
}
