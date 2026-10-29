//! 真实道路路由轨迹生成器（模式 B）。
//!
//! 与经典模式（`generator::build`）的分工：
//! - 几何源为路网闭环路线（而非 Catmull-Rom 环）；
//! - 配速为运动学速度剖面：OU 均值回归 + 弯道预判刹车（`kinematics::pace_profile`）；
//! - 步频/步幅为生物力学 OU 耦合（`biomech::ou_cadence_series` + `gait`）；
//! - GPS 抖动为 AR(1) 相关漂移 + 高斯测量噪声 + 建筑 SDF 衰减（`noise::GpsJitter`）。
//!
//! 协议字段/哨兵/断崖/吸附/10s 窗与经典模式完全一致。

use super::generator::{SPEED_CEIL, SPEED_FLOOR};
use super::geom::{
    fmt_gain_time, lost_fix_offset, ring_point_at, round_to, to_bd, wgs84_to_bd09, Rng,
    MET_PER_DEG_LAT, MET_PER_DEG_LNG,
};
use super::model::{GenPoint, Segment, TenWindow, Track};
use super::postfix::apply_post_fixes;
use route_planner::{
    fatigue_from_km, gait, ou_cadence_series, pace_profile, plan_route_split, point_in_polygon,
    Coord, GpsJitter, KinParams, RoadGraph, Route, RouteOptions, RoutePoint, Sdf,
};

/// 路线算法模式（二选一）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RouteMode {
    /// 经典打卡点环（Catmull-Rom）。
    #[default]
    Legacy,
    /// 真实道路拓扑路由。
    Road,
}

/// 路由参数：只由「目标距离 + 时长」决定（转弯半径来自向心加速度上限 v²/a_c）。
///
/// 预览与提交必须用同一套参数，否则同一种子也会算出两条不同路线（所见非所得）。
pub fn route_options(dist: f64, dur: i64) -> RouteOptions {
    let a_c_max = 2.5;
    // 防御：dur 异常（0/负数）时按 2 m/s 估算，避免 avg_v 爆炸把转弯半径推到天文数字。
    let dur = if dur > 0 { dur } else { (dist / 2.0).max(1.0) as i64 };
    let avg_v = if dur > 0 { dist / dur as f64 } else { 2.0 };
    // 校园跑步的转弯半径上限 200 m：超过就是参数异常，钳住防止曲率重采样点数爆炸。
    let radius = (avg_v * avg_v / a_c_max).clamp(6.0, 200.0);
    RouteOptions {
        a_c_max,
        min_radius_m: radius,
        ..Default::default()
    }
}

/// 规划闭环路线：预览与提交的唯一入口。
///
/// `waypoints[0]` 为起点，其余为软引导点（按质心角序）；`must` 为强制必经点（不可达即报错）。
pub fn plan_route_for_run(
    net: &RoadGraph,
    waypoints: &[(f64, f64)],
    must: &[(f64, f64)],
    dist: f64,
    dur: i64,
    seed: u64,
) -> Result<Route, String> {
    // 自绘 / 自动成线的环线：线路本身就是要跑的路径，直接沿线跑圈，不在图上乱绕。
    if let Some(ring) = outline_ring() {
        let mut route = outline_route(&ring, dist);
        let amp = (0.8 + 18.0 * rugged_level()).min(6.0);
        humanize_path(&mut route.points, seed, amp);
        return Ok(route);
    }
    let opts = route_options(dist, dur);
    let wps: Vec<Coord> = waypoints.iter().map(|p| Coord::new(p.1, p.0)).collect();
    // 必经点之间插入崎岖偏移点：路线因此沿岔路来回走，而不是拉成平滑大圆环。
    let rugged_must = if rugged_waypoints_enabled() {
        rugged_waypoints(must, rugged_level(), seed)
    } else {
        must.to_vec()
    };
    let musts: Vec<Coord> = rugged_must.iter().map(|p| Coord::new(p.1, p.0)).collect();
    let mut route = plan_route_split(net, &wps, &musts, dist, seed, &opts)?;
    // 真人不会贴着路中心线跑：叠加横向游走，避免轨迹是一条数学光滑曲线。
    // 摇摆幅度跟整条路线的尺度挂钩：小环（手绘）不会被放成刺团
    let mut total = 0.0;
    for w in route.points.windows(2) {
        let dy = (w[0].lat - w[1].lat) * crate::track::geom::MET_PER_DEG_LAT;
        let dx = (w[0].lon - w[1].lon) * crate::track::geom::MET_PER_DEG_LNG;
        total += (dx * dx + dy * dy).sqrt();
    }
    let (mut la_min, mut la_max, mut lo_min, mut lo_max) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &route.points {
        la_min = la_min.min(p.lat);
        la_max = la_max.max(p.lat);
        lo_min = lo_min.min(p.lon);
        lo_max = lo_max.max(p.lon);
    }
    let span = (((la_max - la_min) * crate::track::geom::MET_PER_DEG_LAT).powi(2)
        + ((lo_max - lo_min) * crate::track::geom::MET_PER_DEG_LNG).powi(2))
    .sqrt();
    let amp = (0.8 + 20.0 * rugged_level())
        .min(total.max(1.0) / 80.0)
        .min(span.max(20.0) * 0.03);
    humanize_path(&mut route.points, seed, amp);
    Ok(route)
}

impl RouteMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "road" => RouteMode::Road,
            _ => RouteMode::Legacy,
        }
    }
}

/// 从本地文件加载 OSM 路网。
pub fn load_network_path(path: &str) -> Result<RoadGraph, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取 {path} 失败: {e}"))?;
    route_planner::load_osm(&bytes)
}

/// 路网预览计划（供地图显示）：对齐后的路网/建筑 + 生成的路线（BD 系）。
pub struct RoadPlan {
    pub route: Vec<(f64, f64)>,
    pub length_m: f64,
    pub edges: Vec<Vec<(f64, f64)>>,
    pub buildings: Vec<Vec<(f64, f64)>>,
    pub checkpoints: Vec<(f64, f64)>,
    pub fences: Vec<Vec<(f64, f64)>>,
}

/// 用围栏（BD 系，与打卡点同系）裁剪路网：仅保留围栏内道路；裁剪后无路则退回原网。
pub fn apply_fences(net: &RoadGraph, fences: &[Vec<(f64, f64)>]) -> RoadGraph {
    if fences.is_empty() {
        return net.clone();
    }
    let mut g = net.clone();
    let polys: Vec<Vec<Coord>> = fences
        .iter()
        .map(|r| r.iter().map(|p| Coord::new(p.1, p.0)).collect())
        .collect();
    g.retain_inside_any(&polys);
    // 进一步裁剪：移除「穿越围栏边界」的边（端点在内但中段跑出围栏，如跨校区道路）。
    g.graph.retain_edges(|graph, e| {
        let (a, b) = graph.edge_endpoints(e).unwrap();
        edge_inside(&polys, graph[a].coord, graph[b].coord)
    });
    g.rebuild_index();
    if g.graph.edge_count() == 0 {
        return net.clone();
    }
    g
}

/// 边是否整体落在任一围栏内：沿边等距采样，所有采样点（含端点）必须在围栏内。
fn edge_inside(polys: &[Vec<Coord>], a: Coord, b: Coord) -> bool {
    const SEGS: usize = 8;
    for k in 0..=SEGS {
        let t = k as f64 / SEGS as f64;
        let p = Coord::new(a.lon + (b.lon - a.lon) * t, a.lat + (b.lat - a.lat) * t);
        if !polys.iter().any(|poly| point_in_polygon(poly, p)) {
            return false;
        }
    }
    true
}

/// 生成路网预览（对齐 + 围栏裁剪 + 闭环路由），不产生协议字段。
///
/// 与 `build_road` 共用 `plan_route_for_run`：同一份路网/围栏/打卡点/距离/时长/种子
/// 必然得到同一条路线，预览即提交（旧版预览用 `RouteOptions::default()`、提交用
/// 弯道半径，两边路线不同，用户看到「预览和实际提交不一样」）。
///
/// 默认崎岖度：0 时打卡点之间直接沿路连（接近一个大圆环），1 时到处拐小弯。
pub const RUGGED_DEFAULT: f64 = 0.62;

/// 当前崎岖度（0.62 表示默认，存 v*1000+1，0 表示未设置）。
static RUGGED_LEVEL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 设置崎岖度（路网页滑块调用；预览与提交共用，保证所见即所得）。
pub fn set_rugged(v: f64) {
    let q = (v.clamp(0.0, 1.0) * 1000.0).round() as u64;
    RUGGED_LEVEL.store(q + 1, std::sync::atomic::Ordering::Relaxed);
}

/// 读取当前崎岖度。
pub fn rugged_level() -> f64 {
    let v = RUGGED_LEVEL.load(std::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        RUGGED_DEFAULT
    } else {
        (v - 1) as f64 / 1000.0
    }
}

/// 在相邻必经点之间插入「横向偏移」的中间点，让路线走岔路、绕建筑，而不是拉成平滑大圆环。
///
/// 插入点会被 `add_virtual_nodes` 投影到最近的道路边上，因此路线仍然只沿道路拓扑走，
/// 不会穿建筑；偏移越大、插入越多，轨迹就越「崎岖」。
/// 输入按环序（末点接回首个点），返回同样闭环的点列。
pub fn rugged_waypoints(must: &[(f64, f64)], level: f64, seed: u64) -> Vec<(f64, f64)> {
    if must.len() < 2 || level <= 0.0 {
        return must.to_vec();
    }
    let mut rng = Rng::new(seed ^ 0x5EED_2A17);
    let per_seg = if level > 0.7 { 3 } else if level > 0.45 { 2 } else { 1 };
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(must.len() * (per_seg + 1));
    for i in 0..must.len() {
        let a = must[i];
        let b = must[(i + 1) % must.len()];
        out.push(a);
        let (dlat, dlon) = (b.0 - a.0, b.1 - a.1);
        let len = (dlat * dlat + dlon * dlon).sqrt();
        if len <= 1e-9 {
            continue;
        }
        // 法向量（度空间近似正交即可，校园尺度误差可忽略）
        let (ny, nx) = (-dlon / len, dlat / len);
        for k in 1..=per_seg {
            let t = k as f64 / (per_seg as f64 + 1.0);
            // 偏移上限约 30 m，且不超过段长的 22%，避免投影到别的路上
            let amp = (level * 0.22 * len).min(0.00028);
            let sign = if rng.random() < 0.5 { -1.0 } else { 1.0 };
            let off = amp * sign * (0.55 + 0.9 * rng.random());
            out.push((a.0 + dlat * t + ny * off, a.1 + dlon * t + nx * off));
        }
    }
    out
}
/// 预览阶段拿不到 policy 必经点，故仅做「软引导 + 吸附」，不强制经过任何打卡点。
pub fn plan_road_view(
    net: &RoadGraph,
    points_bd: &[(f64, f64)],
    dist: f64,
    dur: i64,
    seed: u64,
    fences: &[Vec<(f64, f64)>],
) -> Result<RoadPlan, String> {
    let mut g = net.clone();
    align_network(&mut g);
    g = apply_fences(&g, fences);
    // 只保留真正落在路网附近（≤120 m）的打卡点：路网覆盖不到的点会把路线拉出十几公里
    let near: Vec<(f64, f64)> = points_bd
        .iter()
        .copied()
        .filter(|p| {
            net.graph.node_weights().any(|n| {
                let dy = (n.coord.lat - p.0) * crate::track::geom::MET_PER_DEG_LAT;
                let dx = (n.coord.lon - p.1)
                    * crate::track::geom::MET_PER_DEG_LNG
                    * p.0.to_radians().cos();
                (dx * dx + dy * dy).sqrt() <= 120.0
            })
        })
        .collect();
    let src: &[(f64, f64)] = if near.len() >= 2 { &near } else { points_bd };
    let ordered = radial_order(src);
    // 打卡点必须是「必经点」，不能只当方向锚点，否则路线会退化成一个平滑大圆环。
    let must: Vec<(f64, f64)> = ordered.iter().skip(1).copied().collect();
    let route = plan_route_for_run(&g, &ordered, &must, dist, dur, seed)?;
    let route_pts: Vec<(f64, f64)> = route.points.iter().map(|p| (p.lat, p.lon)).collect();
    let edges: Vec<Vec<(f64, f64)>> = g
        .edges_deg()
        .iter()
        .map(|e| e.iter().map(|c| (c.lat, c.lon)).collect())
        .collect();
    let buildings: Vec<Vec<(f64, f64)>> = g
        .buildings
        .iter()
        .map(|r| r.iter().map(|c| (c.lat, c.lon)).collect())
        .collect();
    Ok(RoadPlan {
        route: route_pts,
        length_m: route.length_m,
        edges,
        buildings,
        checkpoints: ordered,
        fences: fences.to_vec(),
    })
}

/// 将 WGS84 路网逐节点转换为 BD-09（与打卡点同系）。
///
/// OSM 为 WGS84，打卡点为 BD-09。两者不是常数平移关系（境内 GCJ/BD 偏移随位置
/// 变化），故用标准 WGS84→GCJ-02→BD-09 变换逐节点转换，校园尺度下与真实道路
/// 对齐（优于旧版质心常数平移的米级~数十米残差）。
pub fn align_network(net: &mut RoadGraph) {
    if net.graph.node_count() == 0 {
        return;
    }
    for node in net.graph.node_weights_mut() {
        let (lat, lon) = wgs84_to_bd09(node.coord.lat, node.coord.lon);
        node.coord.lat = lat;
        node.coord.lon = lon;
    }
    for ring in net.buildings.iter_mut() {
        for c in ring.iter_mut() {
            let (lat, lon) = wgs84_to_bd09(c.lat, c.lon);
            c.lat = lat;
            c.lon = lon;
        }
    }
    net.compute_anchor();
    net.rebuild_index();
}

/// 按质心角度排序打卡点，形成自然环序。
pub fn radial_order(points_bd: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = points_bd.len();
    if n == 0 {
        return Vec::new();
    }
    let (cx, cy) = (
        points_bd.iter().map(|q| q.0).sum::<f64>() / n as f64,
        points_bd.iter().map(|q| q.1).sum::<f64>() / n as f64,
    );
    let mut ordered = points_bd.to_vec();
    ordered.sort_by(|a, b| {
        (a.0 - cx)
            .atan2(a.1 - cy)
            .partial_cmp(&(b.0 - cx).atan2(b.1 - cy))
            .unwrap()
    });
    ordered
}

/// 路线 → 平面折线 + 弧长表。
///
/// `arcs[k]` 必须与 `dense[k]` 一一对应（`ring_point_at` 按同下标取弧长），
/// 因此直接取每个采样点的累计弧长 `p.s`（`dense` 末点已闭合回起点）。
fn route_ring(route: &Route, c_lat: f64, c_lng: f64) -> (Vec<(f64, f64)>, Vec<f64>) {
    let dense: Vec<(f64, f64)> = route
        .points
        .iter()
        .map(|p| {
            (
                (p.lon - c_lng) * MET_PER_DEG_LNG,
                (p.lat - c_lat) * MET_PER_DEG_LAT,
            )
        })
        .collect();
    let arcs: Vec<f64> = route.points.iter().map(|p| p.s).collect();
    (dense, arcs)
}

/// 模式 B：真实道路拓扑路由轨迹生成。
///
/// `route_bd[0]` 为起点，`route_bd[1..]` 为软引导点（用于方向锚点，不必全经过）；
/// `must_bd` 为强制必经点（按序，可为空，最终必达）；吸附在 `route_bd` 全量上做。
pub fn build_road(
    dist: f64,
    dur: i64,
    seed: u64,
    start_ms: i64,
    route_bd: &[(f64, f64)],
    must_bd: &[(f64, f64)],
    net: &RoadGraph,
) -> Result<Track, String> {
    let mut rng = Rng::new(seed);
    let dur_f = dur as f64;
    let avg_v = dist / dur_f;
    // 运动学参数与预览共用同一套（弯道半径来自向心加速度上限）
    let a_c_max = route_options(dist, dur).a_c_max;

    // 与预览共用同一规划入口：同样的输入必然得到同一条路线
    let route = plan_route_for_run(net, route_bd, must_bd, dist, dur, seed)?;

    let n_pts = route_bd.len().max(1);
    let (c_lat, c_lng) = (
        route_bd.iter().map(|q| q.0).sum::<f64>() / n_pts as f64,
        route_bd.iter().map(|q| q.1).sum::<f64>() / n_pts as f64,
    );
    let (dense, arcs) = route_ring(&route, c_lat, c_lng);

    // 时间网格（同经典模式）
    let mut times = Vec::new();
    let mut t = 0.0;
    while t < dur_f {
        times.push(t);
        t += if rng.random() < 0.80 {
            5.0
        } else {
            rng.choice(&[1.0, 2.0, 3.0, 4.0, 6.0, 7.0, 8.0])
        };
    }
    let n = times.len();
    let mut dts: Vec<f64> = (0..n - 1).map(|i| times[i + 1] - times[i]).collect();
    dts.push(1.0f64.max(dur_f - times[n - 1]));

    // 运动学速度剖面：OU 配速 + 弯道预判刹车 + 精确命中目标距离
    let kin = KinParams {
        a_c_max,
        ..Default::default()
    };
    let speed_seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let w = pace_profile(
        &route,
        avg_v,
        &dts,
        dist,
        speed_seed,
        &kin,
        SPEED_FLOOR,
        SPEED_CEIL,
    );
    let speeds: Vec<f64> = w.clone();
    let seg_dist: Vec<f64> = (0..n).map(|i| w[i] * dts[i]).collect();

    // 点位类型（同经典模式）
    let tl_w = [
        ((-1i64, 4i64), 54u32),
        ((-1, 1), 27),
        ((-1, 12), 13),
        ((-1, 5), 3),
        ((-1, 6), 2),
    ];
    let mut kinds: Vec<(i64, i64)> = Vec::with_capacity(n);
    for _ in 0..n {
        let u = rng.random();
        if u < 0.39 {
            kinds.push((3, 1));
        } else if u < 0.93 {
            kinds.push((0, 1));
        } else if u < 0.96 {
            kinds.push(rng.weighted(&tl_w));
        } else {
            kinds.push((rng.choice(&[1, 1, 1, 1, 2, 2]), 1));
        }
    }
    for i in 1..kinds.len() {
        let prev_drift = (-1 == kinds[i - 1].0) || kinds[i - 1].0 == 5 || kinds[i - 1].0 == 6;
        if kinds[i].0 != -1 && prev_drift && rng.random() < 0.08 {
            kinds[i] = (if rng.random() < 0.75 { 7 } else { 8 }, 1);
        }
    }
    for i in 1..n {
        if kinds[i].0 == -1 && kinds[i - 1].0 == -1 {
            kinds[i] = (rng.choice(&[3, 0]), 1);
        }
    }
    let normal_idx: Vec<usize> = (0..n).filter(|&i| kinds[i].0 != -1 && i > 1).collect();
    let share: f64 = normal_idx.iter().map(|&i| seg_dist[i]).sum();
    let share = if share == 0.0 { 1.0 } else { share };
    let mut disp_of = vec![0.0f64; n];
    for &i in &normal_idx {
        disp_of[i] = seg_dist[i] * (dist / share);
    }

    // 建筑 SDF（自适应放大漂移）
    let sdf = Sdf::from_buildings(
        &net.buildings,
        |c| {
            [
                (c.lon - c_lng) * MET_PER_DEG_LNG,
                (c.lat - c_lat) * MET_PER_DEG_LAT,
            ]
        },
        2.0,
        30.0,
    );

    // 生物力学：步频在目标值附近 OU 波动，步幅反解严格满足 v=(cad/60)·stride
    let targets: Vec<f64> = (0..n)
        .map(|i| {
            let km_done = (times[i] / dur_f) * dist / 1000.0;
            gait(speeds[i], fatigue_from_km(km_done)).cadence
        })
        .collect();
    let cadence_seed = seed.wrapping_add(0xC6A4_A793_5BD1_E995);
    let cadences = ou_cadence_series(&targets, &dts, cadence_seed, 0.25, 0.5, 110.0, 220.0);

    // GPS 抖动：AR(1) 相关漂移 + 高斯测量噪声（SDF 逐点放大）
    let mut jitter = GpsJitter::new(0.90, 1.55, 1.05);
    // 丢星点（type=-1）的 AR(1) 漂移向量状态（米）
    let mut lost_drift = (0.0f64, 0.0f64);

    let mut locs: Vec<GenPoint> = Vec::with_capacity(n);
    let mut s = 0.0f64;
    let mut t_acc = 0.0f64;
    let mut dist_acc = 0.0f64;
    let mut steps_acc = 0.0f64;
    let mut alt = 82.0 + rng.uniform(-1.0, 1.0);
    let n_est = 1.max((dur_f / 5.0) as i64);
    let alt_sigma = rng.uniform(3.8, 6.2) / (0.40 * n_est as f64);
    let mut ten_t = 0.0f64;
    let mut ten_d = 0.0f64;
    let mut ten_st = 0.0f64;
    let mut ten_speed: Vec<TenWindow> = Vec::new();
    let mut ten_steps: Vec<TenWindow> = Vec::new();

    for i in 0..n {
        let dt = dts[i];
        let (typ, lt) = kinds[i];
        t_acc += dt;
        let pos = |ss: f64| ring_point_at(&dense, &arcs, ss);
        let mut d_step = 0.0f64;
        let px;
        let py;
        let x;
        let y;
        let rad;
        let state;
        if typ != -1 {
            d_step = disp_of[i];
            s += d_step;
            let (bx, by) = pos(s);
            x = bx;
            y = by;
            // AR(1) 相关漂移 + 高斯测量噪声，SDF 自适应放大
            let sc = sdf.sigma_scale([bx, by]);
            let z = [
                rng.gauss(0.0, 1.0),
                rng.gauss(0.0, 1.0),
                rng.gauss(0.0, 1.0),
                rng.gauss(0.0, 1.0),
            ];
            let (jxd, jyd) = jitter.step(sc, z);
            px = bx + jxd;
            py = by + jyd;
            rad = round_to(
                if typ == 3 {
                    rng.uniform(1.4, 5.1)
                } else {
                    rng.uniform(1.4, 2.4)
                },
                2,
            );
            state = if typ == 0 {
                rng.weighted(&[(1, 145), (2, 45), (3, 164)])
            } else {
                rng.weighted(&[(1, 145), (2, 256), (3, 151)])
            };
        } else {
            // 丢星点（type=-1）：真人样本是「原地复制上一点坐标」或几米的 AR(1) 漂移，
            // 不是随机方向的大位移（旧实现 2-300m 甩点 → 轨迹断断续续）。
            let (dx, dy) = lost_fix_offset(&mut rng, &mut lost_drift);
            let (bx, by) = pos(s);
            x = bx;
            y = by;
            if dx == 0.0 && dy == 0.0 {
                if let Some(last) = locs.last() {
                    py = (last.gLat - c_lat) * MET_PER_DEG_LAT;
                    px = (last.gLng - c_lng) * MET_PER_DEG_LNG;
                } else {
                    let (jx, jy) = jitter.state();
                    px = x + jx;
                    py = y + jy;
                }
            } else {
                px = x + dx;
                py = y + dy;
            }
            rad = if lt == 4 {
                round_to(
                    if rng.random() < 0.75 {
                        rng.uniform(30.0, 100.0)
                    } else {
                        rng.uniform(100.0, 550.0)
                    },
                    2,
                )
            } else if lt == 1 {
                round_to(
                    if rng.random() < 0.75 {
                        rng.uniform(1.6, 12.0)
                    } else {
                        rng.uniform(12.0, 95.0)
                    },
                    2,
                )
            } else if lt == 12 {
                round_to(rng.uniform(30.0, 125.0), 2)
            } else if lt == 5 {
                round_to(rng.uniform(25.0, 300.0), 2)
            } else {
                550.0
            };
            state = rng.weighted(&[(1, 102), (2, 124), (3, 136)]);
        }
        if typ != -1 {
            dist_acc += d_step;
        }
        let (lat, lng) = to_bd(px, py, c_lat, c_lng);
        alt += 0.04 * (82.0 - alt) + rng.gauss(0.0, alt_sigma);
        // 生物力学联动：v=(cad/60)·stride
        let v_now = speeds[i];
        let mut stride = v_now * 60.0 / cadences[i];
        if stride > 1.7 {
            stride = 1.7;
        } else if stride < 0.5 {
            stride = 0.5;
        }
        let cad = v_now * 60.0 / stride;
        steps_acc += cad / 60.0 * dt;
        let nxt = pos(s + 2.0);
        let brg =
            ((nxt.0 - x).atan2(nxt.1 - y).to_degrees() + rng.gauss(0.0, 35.0)).rem_euclid(360.0);
        let (avg_sp, gps_speed) = if typ == -1 {
            let avg = round_to(dist_acc / t_acc.max(1.0), 4);
            let gps = if rng.random() < 0.12 {
                rng.uniform(15.0, 46.0)
            } else {
                rng.uniform(0.5, 6.0)
            };
            (avg, round_to(gps, 4))
        } else {
            let avg = round_to(d_step / dt, 4);
            let kmh = avg * 3.6;
            let sigma = (kmh * 0.08).max(0.05);
            let gps = if rng.random() < 0.20 {
                0.0
            } else {
                round_to((kmh + rng.gauss(0.0, sigma)).max(0.0), 4)
            };
            (avg, gps)
        };

        ten_t += dt;
        ten_d += speeds[i] * dt;
        ten_st += cad / 60.0 * dt;
        while ten_t >= 10.0 {
            let k = 10.0 / ten_t;
            let (out_d, out_st) = (ten_d * k, ten_st * k);
            ten_speed.push(TenWindow {
                time: 10,
                value: round_to(out_d, 2),
            });
            ten_steps.push(TenWindow {
                time: 10,
                value: round_to(out_st, 0),
            });
            ten_t -= 10.0;
            ten_d -= out_d;
            ten_st -= out_st;
        }
        locs.push(GenPoint {
            id: i as i64 + 1,
            flag: start_ms,
            lat: -1.0,
            lng: -1.0,
            gLat: round_to(lat, 7),
            gLng: round_to(lng, 7),
            speed: round_to(gps_speed, 4),
            avgSpeed: avg_sp,
            radius: rad,
            accuracy: rad,
            ptype: typ,
            locType: lt,
            hasAltitude: true,
            totalTime: round_to(t_acc, 0) as i64,
            totalDis: round_to(dist_acc, 4),
            validDis: round_to(dist_acc, 4),
            validTime: round_to(t_acc, 0) as i64,
            steps: steps_acc as i64,
            stepDistance: 0.0,
            gainTime: fmt_gain_time(start_ms + (t_acc * 1000.0) as i64),
            gainTimeMs: start_ms + (t_acc * 1000.0) as i64,
            queueNum: 0,
            coorType: "gcj02".into(),
            bdA: round_to(alt, 2),
            bdD: round_to(brg, 2),
            bdS: round_to((avg_sp * rng.uniform(0.6, 0.95)).max(0.0), 3),
            bdG: rng.choice(&[1, 1, 1, -1]),
            count: rng.randint(20, 88),
            dtr: 0.0,
            state,
            locationId: String::new(),
        });
    }
    if ten_t > 1.0 {
        let k = (10.0 / ten_t).min(1.4);
        ten_speed.push(TenWindow {
            time: 10,
            value: round_to(ten_d * k, 2),
        });
        ten_steps.push(TenWindow {
            time: 10,
            value: round_to(ten_st * k, 0),
        });
    }

    let mut segments: Vec<Segment> = Vec::new();
    let (mut seg_t, mut seg_d) = (0.0f64, 0.0f64);
    let mut seg_v: Vec<f64> = Vec::new();
    for i in 0..n {
        seg_t += dts[i];
        seg_d += seg_dist[i];
        seg_v.push(speeds[i]);
        if seg_t >= 60.0 || i == n - 1 {
            segments.push(Segment {
                totalTime: round_to(seg_t, 0) as i64,
                distance: round_to(seg_d, 0) as i64,
                startTime: round_to((times[i] - seg_t) * 1000.0, 0) as i64,
                endTime: round_to(times[i] * 1000.0, 0) as i64,
                avgSpeed: round_to(seg_v.iter().sum::<f64>() / seg_v.len() as f64, 3),
                avgStep: round_to(steps_acc / 1.0f64.max(t_acc) * 60.0, 0) as i64,
                state: 0,
            });
            seg_t = 0.0;
            seg_d = 0.0;
            seg_v.clear();
        }
    }

    apply_post_fixes(&mut locs, &mut rng, start_ms);

    // 点位吸附：<40m 精确落位（全量打卡点，含必经点与普通打卡点）
    for pl in route_bd {
        let mut best_i = None;
        let mut best_d = 1e18f64;
        for (i, q) in locs.iter().enumerate() {
            let dd = ((q.gLat - pl.0) * MET_PER_DEG_LAT).powi(2)
                + ((q.gLng - pl.1) * MET_PER_DEG_LNG).powi(2);
            if dd < best_d {
                best_d = dd;
                best_i = Some(i);
            }
        }
        if let Some(i) = best_i {
            if best_d < 40.0 * 40.0 {
                locs[i].gLat = round_to(pl.0, 7);
                locs[i].gLng = round_to(pl.1, 7);
            }
        }
    }

    let total_dis = round_to(dist, 3);
    Ok(Track {
        totalTime: round_to(t_acc, 0) as i64,
        totalDistance: total_dis,
        validDistance: total_dis,
        validTime: round_to(t_acc, 0) as i64,
        startTime: start_ms,
        startLatitude: locs[0].gLat,
        startLongitude: locs[0].gLng,
        totalSteps: steps_acc as i64,
        locations: locs,
        speedPerTenSec: ten_speed,
        stepsPerTenSec: ten_steps,
        segments,
    })
}
/// 给闭环路线叠加「人跑出来的横向游走」。
///
/// 三档一阶自回归噪声叠加（波长分别约 25 / 45 / 90 m），幅度按崎岖度缩放。
/// 目的是让落地的 GPS 轨迹不再是贴着路中心线的数学光滑曲线——真人会走宽、
/// 会让路、会切内道，横向总有点来回。
pub fn humanize_path(pts: &mut [RoutePoint], seed: u64, amp_m: f64) {
    let n = pts.len();
    if n < 8 || amp_m <= 0.0 {
        return;
    }
    let mut rng = Rng::new(seed ^ 0x51ED_2A17);
    let (mut a, mut b) = (0.0f64, 0.0f64);
    let mut offs = Vec::with_capacity(n);
    for _ in 0..n {
        // 相关长度分别约 56 m / 13 m 的慢漂，真人跑偏就是这种低频游走
        a = 0.965 * a + rng.gauss(0.0, 0.40);
        b = 0.86 * b + rng.gauss(0.0, 0.30);
        offs.push((a + b * 0.55) * amp_m);
    }
    // 两次 9 点滑动平均，抹掉高频锯齿，只留缓慢横向游走
    for _ in 0..2 {
        let src = offs.clone();
        for i in 0..n {
            let mut s = 0.0;
            for k in 0..9 {
                s += src[(i + k + n - 4) % n];
            }
            offs[i] = s / 9.0;
        }
    }
    let mean = offs.iter().sum::<f64>() / n as f64;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = if i == 0 { n - 1 } else { i - 1 };
        let next = (i + 1) % n;
        let dlat = pts[next].lat - pts[prev].lat;
        let dlon = pts[next].lon - pts[prev].lon;
        let (my, mx) = (
            dlat * crate::track::geom::MET_PER_DEG_LAT,
            dlon * crate::track::geom::MET_PER_DEG_LNG,
        );
        let len = (mx * mx + my * my).sqrt();
        let off = offs[i] - mean;
        if len < 1e-9 {
            out.push(pts[i]);
            continue;
        }
        // 法向（米）
        let (nx, ny) = (-my / len, mx / len);
        out.push(RoutePoint {
            lon: pts[i].lon + off * nx / crate::track::geom::MET_PER_DEG_LNG,
            lat: pts[i].lat + off * ny / crate::track::geom::MET_PER_DEG_LAT,
            ..pts[i]
        });
    }
    pts.clone_from_slice(&out);
}
/// 自绘 / 自动成线的路网本身已带形状，再叠「崎岖必经点」会把路线拧成结。
fn rugged_waypoints_enabled() -> bool {
    let p = crate::api::model::load_config().effective_osm_path().to_lowercase();
    !(p.ends_with("route.osm") || p.ends_with("manual.osm"))
}
/// 读取自绘 / 自动成线的环线轮廓（BD-09，每行 lat,lon）。
fn outline_ring() -> Option<Vec<(f64, f64)>> {
    let p = crate::api::model::load_config().effective_osm_path().to_lowercase();
    if !(p.ends_with("route.osm") || p.ends_with("manual.osm")) {
        return None;
    }
    let txt = std::fs::read_to_string(crate::platform::data_dir().join("route.outline")).ok()?;
    let mut v: Vec<(f64, f64)> = Vec::new();
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((a, b)) = line.split_once(',') {
            if let (Ok(la), Ok(lo)) = (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
                v.push((la, lo));
            }
        }
    }
    if v.len() >= 8 {
        Some(v)
    } else {
        None
    }
}

/// 沿轮廓重复跑圈直到达到目标距离（不做图搜索，避免来回折返糊成一团）。
fn outline_route(ring: &[(f64, f64)], dist: f64) -> Route {
    let mut pts: Vec<RoutePoint> = Vec::new();
    let mut s = 0.0f64;
    let mut plat = ring[0].0;
    let mut plon = ring[0].1;
    let mut laps = 0;
    while s < dist && laps < 200 {
        for &(la, lo) in ring.iter().chain(ring.first()) {
            let dy = (la - plat) * crate::track::geom::MET_PER_DEG_LAT;
            let dx = (lo - plon) * crate::track::geom::MET_PER_DEG_LNG;
            s += (dx * dx + dy * dy).sqrt();
            pts.push(RoutePoint { lon: lo, lat: la, s, curvature: 0.0 });
            plat = la;
            plon = lo;
        }
        laps += 1;
    }
    Route { points: pts, length_m: s }
}
