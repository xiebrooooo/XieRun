//! 轨迹层：生成器 / OBS 组装 / 官方卡路里。

pub mod altitude;
pub mod auto_route;
pub mod calorie;
pub mod generate_road;
pub mod generator;
pub mod geom;
pub mod model;
pub mod network;
pub mod postfix;
pub mod wire;

use geom::{MET_PER_DEG_LAT, MET_PER_DEG_LNG};

/// 轨迹几何统计（本地核对用，不参与上传）。
///
/// 对照依据：10 条真人历史记录实测 —— 相邻点间距中位 6–12m、99 分位 ≤37m、最大 ≤48m；
/// 丢星点 42% 原地复制、位移上限 47.9m。
pub struct TrackDensityStats {
    /// 相邻点间距分位（米）
    pub step_p50: f64,
    pub step_p99: f64,
    pub step_max: f64,
    /// 丢星点（type=-1）数量与其中「原地复制」的数量
    pub lost_total: usize,
    pub lost_copies: usize,
    pub lost_p50: f64,
    pub lost_max: f64,
}

fn planar_m(a: &model::GenPoint, b: &model::GenPoint) -> f64 {
    (((b.gLat - a.gLat) * MET_PER_DEG_LAT).powi(2) + ((b.gLng - a.gLng) * MET_PER_DEG_LNG).powi(2))
        .sqrt()
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

/// 计算一条轨迹的间距/丢星点统计。
pub fn track_density_stats(track: &model::Track) -> TrackDensityStats {
    let mut steps: Vec<f64> = track
        .locations
        .windows(2)
        .map(|w| planar_m(&w[0], &w[1]))
        .collect();
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut lost: Vec<f64> = track
        .locations
        .windows(2)
        .filter(|w| w[1].ptype == -1)
        .map(|w| planar_m(&w[0], &w[1]))
        .collect();
    lost.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    TrackDensityStats {
        step_p50: quantile(&steps, 0.50),
        step_p99: quantile(&steps, 0.99),
        step_max: steps.last().copied().unwrap_or(0.0),
        lost_total: lost.len(),
        lost_copies: lost.iter().filter(|d| **d <= 0.6).count(),
        lost_p50: quantile(&lost, 0.50),
        lost_max: lost.last().copied().unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::generate_road::build_road;
    use super::generator::build;
    use super::geom::{MET_PER_DEG_LAT, MET_PER_DEG_LNG};

    use super::wire::*;

    fn sample_points() -> Vec<(f64, f64)> {
        // 某校园 5 个打卡点（BD 系）
        vec![
            (38.901678, 121.540241),
            (38.902564, 121.541233),
            (38.900921, 121.542310),
            (38.899823, 121.541010),
            (38.900455, 121.539512),
        ]
    }

    /// 在打卡点质心周围构建 7x7 网格路网（双向，约 111m 间距）。
    fn road_grid() -> route_planner::RoadGraph {
        let pts = sample_points();
        let n = pts.len() as f64;
        let (clat, clng) = (
            pts.iter().map(|p| p.0).sum::<f64>() / n,
            pts.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let mut g = route_planner::RoadGraph::new();
        let step = 0.0012;
        let mut nodes = vec![];
        for j in 0..7i64 {
            for i in 0..7i64 {
                let lat = clat + (j as f64 - 3.0) * step;
                let lon = clng + (i as f64 - 3.0) * step;
                nodes.push(g.add_node(route_planner::Coord::new(lon, lat)));
            }
        }
        let mut wid = 1u64;
        let add_edge = |g: &mut route_planner::RoadGraph, a, b, wid| {
            let d = route_planner::graph::EdgeData {
                way_id: wid,
                oneway: false,
                highway: "residential".into(),
                maxspeed_kmh: None,
                length_m: 111.0,
            };
            g.add_edge(a, b, d.clone());
            g.add_edge(b, a, d);
        };
        for j in 0..7usize {
            for i in 0..6usize {
                let a = nodes[j * 7 + i];
                let b = nodes[j * 7 + i + 1];
                add_edge(&mut g, a, b, wid);
                wid += 1;
            }
        }
        for i in 0..7usize {
            for j in 0..6usize {
                let a = nodes[j * 7 + i];
                let b = nodes[(j + 1) * 7 + i];
                add_edge(&mut g, a, b, wid);
                wid += 1;
            }
        }
        g.compute_anchor();
        g.rebuild_index();
        g
    }

    /// 模式 B（真实道路路由）：距离精确、哨兵/断崖语义与模式 A 一致。
    #[test]
    fn test_build_road_distribution() {
        let pts = sample_points();
        let net = road_grid();
        let start = 1_788_958_186_123i64;
        let track = build_road(3300.0, 1220, 42, start, &pts, &pts[1..], &net).expect("build_road");
        assert!(
            (track.totalDistance - 3300.0).abs() < 0.5,
            "dist={}",
            track.totalDistance
        );
        assert_eq!(track.totalTime, 1220);
        assert!(!track.locations.is_empty());
        assert!([0, 7].contains(&track.locations[0].ptype));
        assert_eq!(track.locations[1].ptype, 5);
        assert_eq!(track.locations.last().unwrap().ptype, 6);
        // 步数为正
        assert!(track.totalSteps > 500, "steps={}", track.totalSteps);
    }

    /// 轨迹生成抽样断言：距离精确、采样间隔分布、哨兵/断崖/位移语义。
    #[test]
    fn test_generator_distribution() {
        let pts = sample_points();
        let start = 1_788_958_186_123i64;
        let track = build(3300.0, 1220, 42, (38.9, 121.54), start, &pts);
        // 总距离精确等于目标（±0.5m 舍入容差）
        assert!(
            (track.totalDistance - 3300.0).abs() < 0.5,
            "dist={}",
            track.totalDistance
        );
        assert_eq!(track.totalTime, 1220);
        // 点数合理（主 5s 采样）
        let n = track.locations.len();
        assert!((200..320).contains(&n), "n={n}");
        // 哨兵：索引0 type∈{0,7}/totalTime=0/state=1；索引1 type=5 全零；末点 type=6
        assert!([0, 7].contains(&track.locations[0].ptype));
        assert_eq!(track.locations[0].totalTime, 0);
        assert_eq!(track.locations[0].state, 1);
        assert_eq!(track.locations[1].ptype, 5);
        assert_eq!(track.locations[1].totalDis, 0.0);
        assert_eq!(track.locations[1].steps, 0);
        assert_eq!(track.locations.last().unwrap().ptype, 6);
        // 累计距离单调不减、末点 ≈ 总距离
        let mut prev = 0.0;
        for p in &track.locations {
            assert!(p.totalDis >= prev - 1e-6, "totalDis 回退");
            prev = p.totalDis;
        }
        // 距离只由正常点承担：终点哨兵(type=6)携带全程累计距离
        assert!(
            (track.locations.last().unwrap().totalDis - 3300.0).abs() < 2.0,
            "末点={}",
            track.locations.last().unwrap().totalDis
        );
        // 采样间隔：5s 占比 ≥ 60%
        let mut fives = 0;
        let mut total = 0;
        for w in track.locations.windows(2) {
            let dt = w[1].totalTime - w[0].totalTime;
            if dt > 0 {
                total += 1;
                if dt == 5 {
                    fives += 1;
                }
            }
        }
        assert!(fives as f64 / total as f64 > 0.6, "5s 占比不足");
        // 10s 窗非空、结构合法
        assert!(!track.speedPerTenSec.is_empty());
        assert_eq!(track.speedPerTenSec.len(), track.stepsPerTenSec.len());
        // 首点 lat/lng 占位 -1.0，coorType gcj02
        assert_eq!(track.locations[0].lat, -1.0);
        assert_eq!(track.locations[0].coorType, "gcj02");
        // 步数为正、步频在合理范围
        assert!(track.totalSteps > 500, "steps={}", track.totalSteps);
    }

    /// 打卡点吸附：轨迹必过点位（<40m 落位）。
    #[test]
    fn test_point_snapping() {
        let pts = sample_points();
        let track = build(2200.0, 900, 7, (38.9, 121.54), 1_788_958_186_123, &pts);
        for pl in &pts {
            let min_m = track
                .locations
                .iter()
                .map(|p| {
                    (((p.gLat - pl.0) * MET_PER_DEG_LAT).powi(2)
                        + ((p.gLng - pl.1) * MET_PER_DEG_LNG).powi(2))
                    .sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            assert!(min_m < 1.0, "点位吸附失败: {min_m}m");
        }
    }

    /// 轨迹连续性：真人样本（10 条历史记录）相邻点间距最大 5–48m，没有断崖。
    ///
    /// 旧实现给「丢星点」按随机方向甩 2–300m，肉眼就是一条条断断续续的折线；
    /// 这里按同一判据守住：最大间距 ≤ 60m，99 分位 ≤ 55m。
    #[test]
    fn test_tracks_are_continuous_like_real_samples() {
        let pts = sample_points();
        let net = road_grid();
        for seed in 0..8u64 {
            for &(dist, dur) in &[(2100.0, 700i64), (3300.0, 1220), (1050.0, 480)] {
                let mut tracks = vec![build(dist, dur, seed, (38.9, 121.54), 1_788_958_186_123, &pts)];
                if let Ok(t) = build_road(dist, dur, seed, 1_788_958_186_123, &pts, &[], &net) {
                    tracks.push(t);
                }
                for track in tracks {
                    let stats = super::track_density_stats(&track);
                    assert!(
                        stats.step_max <= 60.0,
                        "seed={seed} dist={dist} 出现断崖 {:.1}m",
                        stats.step_max
                    );
                    assert!(
                        stats.step_p99 <= 55.0,
                        "seed={seed} dist={dist} 99分位 {:.1}m",
                        stats.step_p99
                    );
                }
            }
        }
    }

    /// 丢星点（type=-1）位移分布对齐真人样本：多数原地复制，最大 ≤50m。
    #[test]
    fn test_lost_fix_points_match_real_samples() {
        let pts = sample_points();
        let net = road_grid();
        // 真人样本：42% 位移 0（原地）、8% 在 20-50m、0 个 >50m
        let (mut copies, mut total) = (0usize, 0usize);
        for seed in 0..12u64 {
            let mut tracks = vec![build(2100.0, 700, seed, (38.9, 121.54), 1_788_958_186_123, &pts)];
            if let Ok(t) = build_road(2100.0, 700, seed, 1_788_958_186_123, &pts, &[], &net) {
                tracks.push(t);
            }
            for track in tracks {
                let stats = super::track_density_stats(&track);
                total += stats.lost_total;
                copies += stats.lost_copies;
                assert!(
                    stats.lost_max <= super::geom::LOST_MAX_OFFSET_M + 1.0,
                    "seed={seed} 丢星位移 {:.1}m 超出真人上限",
                    stats.lost_max
                );
            }
        }
        assert!(total > 20, "丢星点样本太少：{total}");
        // 真人原地比例 42%，这里按 25% 兜底（抽样波动）
        assert!(
            copies * 4 >= total,
            "原地复制占比过低：{copies}/{total}（真人 42%）"
        );
    }

    /// 预览即提交：同一份路网/打卡点/距离/时长/种子，提交轨迹必须落在预览路线上。
    ///
    /// 生产流程：预览 `plan_road_view(未对齐 OSM)`（内部对齐），提交 `align_network` 后
    /// `build_road`；两条路径必须得到同一条路线。旧版预览用 `RouteOptions::default()`、
    /// 提交用弯道半径，同一种子也会得到两条不同路线。
    #[test]
    fn test_road_preview_matches_submission() {
        use super::generate_road::{align_network, build_road, plan_road_view};
        let pts = sample_points();
        let raw = road_grid();
        let mut aligned = raw.clone();
        align_network(&mut aligned);
        let (dist, dur, seed) = (2100.0, 700i64, 20261015u64);
        let view = plan_road_view(&raw, &pts, dist, dur, seed, &[]).expect("预览");
        let track = build_road(
            dist,
            dur,
            seed,
            1_788_958_186_123,
            &pts,
            &[],
            &aligned,
        )
        .expect("提交");
        // 轨迹上的每个正常点都应落在预览路线上（抖动 + 建筑 SDF 放大）
        let mut worst = 0.0f64;
        for p in track.locations.iter().filter(|p| p.ptype != -1) {
            let min = view
                .route
                .iter()
                .map(|(la, lo)| {
                    (((p.gLat - la) * MET_PER_DEG_LAT).powi(2)
                        + ((p.gLng - lo) * MET_PER_DEG_LNG).powi(2))
                    .sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            worst = worst.max(min);
        }
        assert!(worst < 12.0, "轨迹偏离预览路线 {worst:.1}m");
        // 时长参与弯道半径（min_radius = max(6, v²/a_c)）：把平均速度拉到 12m/s，
        // 转弯半径必须变大、路线几何必须跟着变（否则说明参数没进规划器）
        let view2 = plan_road_view(&raw, &pts, dist, dur / 4, seed, &[]).expect("预览2");
        let deviation = view2
            .route
            .iter()
            .map(|(la, lo)| {
                view.route
                    .iter()
                    .map(|(a, b)| {
                        (((la - a) * MET_PER_DEG_LAT).powi(2)
                            + ((lo - b) * MET_PER_DEG_LNG).powi(2))
                        .sqrt()
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0f64, f64::max);
        assert!(
            view2.route.len() != view.route.len() || deviation > 1.0,
            "时长未参与路线规划（两侧路线完全一致）"
        );
    }

    /// 10 秒窗均值配速全部落在有效窗口内（判定规则 2'21"-10'00"/km），且总距精确。
    /// 逐点 avgSpeed 允许越界（真人爬坡期同样低于窗口，见 OBS 样本）。
    #[test]
    fn test_speeds_within_valid_pace_window() {
        let pts = sample_points();
        let combos = [
            (1050.0, 480i64),
            (1440.0, 661),
            (1920.0, 719),
            (2100.0, 900),
            (3300.0, 1220),
        ];
        for seed in 0..16u64 {
            for &(dist, dur) in &combos {
                let t = build(dist, dur, seed, (38.9, 121.54), 1_788_958_186_123, &pts);
                for (i, w) in t.speedPerTenSec.iter().enumerate() {
                    let pace = 1000.0 / (w.value / 10.0); // 秒/km
                    assert!(
                        (141.0..=600.0).contains(&pace),
                        "seed={seed} dist={dist} 窗{i} 配速 {}/km 越界",
                        format_args!("{}:{:02}", pace as i64 / 60, (pace as i64) % 60)
                    );
                }
                assert!(
                    (t.totalDistance - dist).abs() < 2.0,
                    "seed={seed} dist={}: {}",
                    dist,
                    t.totalDistance
                );
            }
        }
    }

    /// BD→GCJ 实测向量。
    #[test]
    fn test_bd09_to_gcj02_vector() {
        let (lat, lng) = bd09_to_gcj02(38.901678, 121.540241);
        assert!((lat - 38.8956025774013).abs() < 1e-9, "lat={lat}");
        assert!((lng - 121.5337497718317).abs() < 1e-9, "lng={lng}");
    }

    /// 坐标基准变换：gcj02_to_bd09 是 bd09_to_gcj02 的逆；WGS84→BD 有境内偏移。
    #[test]
    fn test_coord_datum_roundtrip() {
        use super::geom::{gcj02_to_bd09, wgs84_to_bd09};
        let bd = (38.901678, 121.540241);
        let gcj = bd09_to_gcj02(bd.0, bd.1);
        let back = gcj02_to_bd09(gcj.0, gcj.1);
        assert!((back.0 - bd.0).abs() < 1e-6, "lat={} 期望 {}", back.0, bd.0);
        assert!((back.1 - bd.1).abs() < 1e-6, "lng={} 期望 {}", back.1, bd.1);
        // WGS84 校园坐标转 BD 后应带明显偏移（约 0.004~0.006 度）
        let wgs = (38.8956, 121.5337);
        let bd2 = wgs84_to_bd09(wgs.0, wgs.1);
        assert!((bd2.0 - wgs.0).abs() > 0.003, "lat 偏移过小 {}", bd2.0);
        assert!((bd2.1 - wgs.1).abs() > 0.003, "lng 偏移过小 {}", bd2.1);
    }

    /// 围栏裁剪：保留边整体都在围栏内，不允许「端点在内、中段越出」的穿越边。
    #[test]
    fn test_apply_fences_clips_crossing_edges() {
        use super::generate_road::apply_fences;
        use route_planner::{point_in_polygon, Coord};
        let net = road_grid();
        let pts = sample_points();
        let n = pts.len() as f64;
        let (clat, clng) = (
            pts.iter().map(|p| p.0).sum::<f64>() / n,
            pts.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let s = 0.0020;
        let fence: Vec<(f64, f64)> = vec![
            (clat - s, clng - s),
            (clat - s, clng + s),
            (clat + s, clng + s),
            (clat + s, clng - s),
        ];
        let polys: Vec<Coord> = fence.iter().map(|p| Coord::new(p.1, p.0)).collect();
        let g = apply_fences(&net, &[fence]);
        assert!(g.graph.edge_count() > 0, "裁剪后仍应有道路");
        for e in g.edges_deg() {
            let a = e[0];
            let b = e[1];
            for k in 0..=8 {
                let t = k as f64 / 8.0;
                let c = Coord::new(a.lon + (b.lon - a.lon) * t, a.lat + (b.lat - a.lat) * t);
                assert!(point_in_polygon(&polys, c), "边越出围栏");
            }
        }
    }

    /// OBS 对象：10 键、gzip+base64 可解、run_data 27 键点集。
    #[test]
    fn test_obs_object_structure() {
        let pts: Vec<serde_json::Value> = sample_points()
            .iter()
            .enumerate()
            .map(|(i, (la, lo))| {
                serde_json::json!({
                    "lon": lo, "lat": la, "isFixed": 0,
                    "pointName": format!("P{i}"), "glon": lo - 0.006,
                    "glat": la - 0.006,
                })
            })
            .collect();
        let track = build(
            3300.0,
            1220,
            42,
            (38.9, 121.54),
            1_788_958_186_123,
            &sample_points(),
        );
        let obj = build_obs_object(&track, 1320403809, "UUID-TEST", 13056447, &pts);
        let keys: Vec<&str> = obj
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "rrid",
                "uuid",
                "uid",
                "run_data",
                "fixed_point_json",
                "segment_json",
                "speed_json",
                "step_freq_json",
                "laps_json",
                "runFaceCheck"
            ]
        );
        // rrid gzip 可解
        let raw = crate::crypto::envelope::b64_decode(obj["rrid"].as_str().unwrap()).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        use std::io::Read;
        let mut s = String::new();
        dec.read_to_string(&mut s).unwrap();
        assert_eq!(s, "1320403809");
        // run_data 解包 → 27 键点集
        let raw = crate::crypto::envelope::b64_decode(obj["run_data"].as_str().unwrap()).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        let mut s = String::new();
        dec.read_to_string(&mut s).unwrap();
        let wrap: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(wrap["useZip"], false);
        let pts: Vec<serde_json::Value> =
            serde_json::from_str(wrap["allLocJson"].as_str().unwrap()).unwrap();
        assert_eq!(pts[0].as_object().unwrap().len(), 27, "点键数必须 27");
        // segment_json 是空串 gzip
        let raw =
            crate::crypto::envelope::b64_decode(obj["segment_json"].as_str().unwrap()).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        let mut s = String::new();
        dec.read_to_string(&mut s).unwrap();
        assert_eq!(s, "");
        // obs keys 两个
        let ks = obs_keys(&track, 1320403809, "UUID-TEST");
        assert_eq!(ks.len(), 2);
        assert!(ks[0].contains("run_data/"));
        assert!(ks[1].starts_with("run_data/1320/1320403809.json"));
    }
    /// 提交用的路网只能「对齐一次」：生产路径是 `load_network_path`(WGS84 osm) →
    /// `align_network`(WGS84→BD-09) → `build_road`（不再对齐）。若把已对齐的路网
    /// 再对齐一次，路网会整体再平移数百米，打卡点只能靠公里级直线连边接上 —— 线上
    /// 表现就是「守卫报距路网最近道路 944m → 回退经典算法 → 轨迹变成椭圆或从别处
    /// 拉直线」（2026-10-16 事故）。
    #[test]
    fn submit_network_is_aligned_exactly_once() {
        use super::generate_road::{align_network, build_road, radial_order};
        // 模拟 osm 源文件：与打卡点同布局、坐标却是 WGS84 的路网 —— 网格按 BD-09 布好后
        // 逐点反算成 WGS84，等价于「路网采集时拿到的原始文件」。网格取 15×15 / 约 33m，
        // 保证每个打卡点到最近路网边 < 40m（与轨迹吸附阈值同量级）。
        let cps = sample_points();
        let n = cps.len() as f64;
        let (clat, clng) = (
            cps.iter().map(|p| p.0).sum::<f64>() / n,
            cps.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let mut raw = route_planner::RoadGraph::new();
        let step = 0.0003;
        let mut nodes = vec![];
        for j in 0..15i64 {
            for i in 0..15i64 {
                let lat = clat + (j as f64 - 7.0) * step;
                let lon = clng + (i as f64 - 7.0) * step;
                nodes.push(raw.add_node(route_planner::Coord::new(lon, lat)));
            }
        }
        let mut wid = 1u64;
        let add_edge = |g: &mut route_planner::RoadGraph, a, b, wid| {
            let d = route_planner::graph::EdgeData {
                way_id: wid,
                oneway: false,
                highway: "residential".into(),
                maxspeed_kmh: None,
                length_m: 33.0,
            };
            g.add_edge(a, b, d.clone());
            g.add_edge(b, a, d);
        };
        for j in 0..15usize {
            for i in 0..14usize {
                let a = nodes[j * 15 + i];
                let b = nodes[j * 15 + i + 1];
                add_edge(&mut raw, a, b, wid);
                wid += 1;
            }
        }
        for i in 0..15usize {
            for j in 0..14usize {
                let a = nodes[j * 15 + i];
                let b = nodes[(j + 1) * 15 + i];
                add_edge(&mut raw, a, b, wid);
                wid += 1;
            }
        }
        for node in raw.graph.node_weights_mut() {
            let (lat, lon) = super::geom::bd09_to_wgs84(node.coord.lat, node.coord.lon);
            node.coord.lat = lat;
            node.coord.lon = lon;
        }
        raw.compute_anchor();
        raw.rebuild_index();
        let ordered = radial_order(&cps);
        // 「每个必经点到路网最近边」的最大值
        let gap = |g: &route_planner::RoadGraph, pts: &[(f64, f64)]| -> f64 {
            let nodes: Vec<(f64, f64)> = g
                .graph
                .node_weights()
                .map(|w| (w.coord.lat, w.coord.lon))
                .collect();
            let mut worst = 0.0f64;
            for p in pts {
                let mut best = f64::INFINITY;
                for e in g.graph.edge_indices() {
                    let Some((a, b)) = g.graph.edge_endpoints(e) else {
                        continue;
                    };
                    let (la, lo) = nodes[a.index()];
                    let (lb, lob) = nodes[b.index()];
                    let ax = (lo - p.1) * MET_PER_DEG_LNG;
                    let ay = (la - p.0) * MET_PER_DEG_LAT;
                    let bx = (lob - p.1) * MET_PER_DEG_LNG;
                    let by = (lb - p.0) * MET_PER_DEG_LAT;
                    let (dx, dy) = (bx - ax, by - ay);
                    let l2 = dx * dx + dy * dy;
                    let t = if l2 <= 0.0 {
                        0.0
                    } else {
                        (-(ax * dx + ay * dy) / l2).clamp(0.0, 1.0)
                    };
                    let d = ((ax + dx * t).powi(2) + (ay + dy * t).powi(2)).sqrt();
                    if d < best {
                        best = d;
                    }
                }
                worst = worst.max(best);
            }
            worst
        };
        // 生产路径：对齐一次 → 路网必须贴住打卡点
        let mut net = raw.clone();
        align_network(&mut net);
        assert!(
            gap(&net, &ordered) < 40.0,
            "align_network 后的路网必须贴住打卡点（实测最大间距 {:.0} m）",
            gap(&net, &ordered)
        );
        // 提交路径：build_road（已对齐路网，不再对齐）→ 每个必经点都要被轨迹经过
        let track = build_road(2000.0, 840, 7, 1_791_441_899_000, &ordered, &ordered, &net)
            .expect("提交");
        for (i, p) in ordered.iter().enumerate() {
            let near = track
                .locations
                .iter()
                .map(|q| {
                    (((q.gLat - p.0) * MET_PER_DEG_LAT).powi(2)
                        + ((q.gLng - p.1) * MET_PER_DEG_LNG).powi(2))
                    .sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            assert!(near < 40.0, "必经点 {i} 距轨迹 {near:.0} m");
        }
        // 反面样本：把已对齐的路网再对齐一次，间距必然掉到几百米开外（旧 bug 的指纹）
        let mut twice = net.clone();
        align_network(&mut twice);
        assert!(
            gap(&twice, &ordered) > 300.0,
            "「对齐两次」这种错误必须能被这条断言抓到"
        );
    }
}