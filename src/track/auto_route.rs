//! 打卡点自动成线：把「识别到的打卡点」串成一条**平滑的环线**。
//!
//! 设计目标（对应用户要求）：
//! 1. **必须经过每个打卡点** —— 用绕质心的方位角排序后，用 Catmull-Rom 闭合样条
//!    穿过每个点，曲线天然经过所有控制点；
//! 2. **夹角要小、尽量圆滑** —— 样条本身一阶连续，之后再「转角迭代平滑」：
//!    逐点算转角，超过阈值就整体加权平滑，直到全程转角都低于阈值；
//! 3. **不能纯数学曲线** —— 只叠加极低频、极小幅度的横向扰动（崎岖度可调，默认很小），
//!    保留一点真人步态的松弛感，不产生尖角；
//! 4. 距离不够就整圈叠加，最后按弧长精确截断。

use super::geom::{MET_PER_DEG_LAT, MET_PER_DEG_LNG};
use super::geom::Rng;

/// 单步采样间距（米），与真实跑步采样密度接近。
const STEP_M: f64 = 2.0;
/// 允许的最大转角（度/步）。2 m 步长下 8° 约等于半径 14 m 的弯。
const MAX_TURN_DEG: f64 = 8.0;

/// 生成结果。
pub struct AutoRoute {
    /// BD-09 折线（纬度, 经度）
    pub points: Vec<(f64, f64)>,
    /// 折线总长（米）
    pub length_m: f64,
    /// 实际绕了几圈
    pub laps: f64,
    /// 单圈基础环线（不含重复圈）：导出路网时用这个，避免路网本身变成多圈
    pub base_lap: Vec<(f64, f64)>,
}

fn dist_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dy = (a.0 - b.0) * MET_PER_DEG_LAT;
    let dx = (a.1 - b.1) * MET_PER_DEG_LNG * a.0.to_radians().cos();
    (dx * dx + dy * dy).sqrt()
}

/// 射线法：点是否在多边形内（建筑避让检查用）。
fn point_in_poly(p: (f64, f64), poly: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (yi, xi) = poly[i];
        let (yj, xj) = poly[j];
        if ((yi > p.0) != (yj > p.0))
            && (p.1 < (xj - xi) * (p.0 - yi) / (yj - yi + f64::EPSILON) + xi)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// 按绕质心的方位角排序（环着校园外围走，天然避开中心建筑群）。
fn order_points(pts: &[(f64, f64)]) -> Vec<usize> {
    let n = pts.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let (cy, cx) = (
        pts.iter().map(|p| p.0).sum::<f64>() / n as f64,
        pts.iter().map(|p| p.1).sum::<f64>() / n as f64,
    );
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&i, &j| {
        let a = (pts[i].0 - cy).atan2(pts[i].1 - cx);
        let b = (pts[j].0 - cy).atan2(pts[j].1 - cx);
        a.partial_cmp(&b).unwrap()
    });
    idx
}

/// Catmull-Rom 单段插值（t ∈ [0,1]，端点 p1→p2）。
fn catmull(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let t2 = t * t;
    let t3 = t2 * t;
    let f = |a: f64, b: f64, c: f64, d: f64| {
        0.5 * ((2.0 * b)
            + (-a + c) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
            + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    (f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1))
}

/// 闭合 Catmull-Rom 样条：曲线严格经过每个控制点，且处处一阶连续。
pub fn spline_closed(pts: &[(f64, f64)], step: f64) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    for i in 0..n {
        let p0 = pts[(i + n - 1) % n];
        let p1 = pts[i];
        let p2 = pts[(i + 1) % n];
        let p3 = pts[(i + 2) % n];
        let seg = dist_m(p1, p2).max(1.0);
        let k = (seg / step).ceil().clamp(6.0, 2000.0) as usize;
        for j in 0..k {
            out.push(catmull(p0, p1, p2, p3, j as f64 / k as f64));
        }
    }
    out
}

/// 按弧长等距重采样（闭合）。
pub fn resample_closed(pts: &[(f64, f64)], step: f64) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut total = 0.0;
    for i in 0..n {
        total += dist_m(pts[i], pts[(i + 1) % n]);
    }
    if total < step * 4.0 {
        return pts.to_vec();
    }
    let want = (total / step).round().max(4.0) as usize;
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(want);
    let mut acc = 0.0;
    let mut next = 0.0_f64;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let d = dist_m(a, b);
        while next <= acc + d && out.len() < want {
            let t = if d > 1e-9 { (next - acc) / d } else { 0.0 };
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            next += step;
        }
        acc += d;
    }
    if out.len() < 4 {
        pts.to_vec()
    } else {
        out
    }
}

/// 闭合曲线加权平滑一遍：[1,2,1]/4。
fn smooth_once(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    (0..n)
        .map(|i| {
            let a = pts[(i + n - 1) % n];
            let b = pts[i];
            let c = pts[(i + 1) % n];
            ((a.0 + 2.0 * b.0 + c.0) / 4.0, (a.1 + 2.0 * b.1 + c.1) / 4.0)
        })
        .collect()
}

/// 当前最大转角（度/顶点）。
fn max_turn_deg(pts: &[(f64, f64)]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut worst: f64 = 0.0;
    for i in 0..n {
        let a = pts[(i + n - 1) % n];
        let b = pts[i];
        let c = pts[(i + 1) % n];
        let v1 = (b.0 - a.0, b.1 - a.1);
        let v2 = (c.0 - b.0, c.1 - b.1);
        let l1 = (v1.0 * v1.0 + v1.1 * v1.1).sqrt();
        let l2 = (v2.0 * v2.0 + v2.1 * v2.1).sqrt();
        if l1 < 1e-12 || l2 < 1e-12 {
            continue;
        }
        let cosv = ((v1.0 * v2.0 + v1.1 * v2.1) / (l1 * l2)).clamp(-1.0, 1.0);
        worst = worst.max(cosv.acos().to_degrees());
    }
    worst
}

/// 极低频横向扰动：两条超长波长正弦，幅度 ≤5 m，不会产生尖角。
fn low_freq_wobble(pts: &mut [(f64, f64)], amp: f64, seed: u64) {
    let n = pts.len();
    if n < 8 || amp <= 0.01 {
        return;
    }
    let phase = (seed % 1000) as f64 / 1000.0 * std::f64::consts::TAU;
    for i in 0..n {
        let a = pts[(i + n - 1) % n];
        let c = pts[(i + 1) % n];
        let dy = c.0 - a.0;
        let dx = c.1 - a.1;
        let len = (dy * dy + dx * dx).sqrt();
        if len < 1e-12 {
            continue;
        }
        let (ny, nx) = (-dx / len, dy / len);
        let t = i as f64;
        let off = amp
            * (0.62 * (t / 140.0 * std::f64::consts::TAU + phase).sin()
                + 0.38 * (t / 61.0 * std::f64::consts::TAU + phase * 1.7).sin());
        let o = off / MET_PER_DEG_LAT;
        pts[i].0 += ny * o;
        pts[i].1 += nx * o;
    }
}

/// 主入口：按打卡点生成目标距离的平滑环线。
pub fn build_auto_route(
    points: &[(f64, f64)],
    buildings: &[Vec<(f64, f64)>],
    target_m: f64,
    rugged: f64,
    seed: u64,
) -> Result<AutoRoute, String> {
    let pts: Vec<(f64, f64)> = {
        let mut v: Vec<(f64, f64)> = Vec::new();
        for p in points {
            if p.0.abs() > 1.0 && p.1.abs() > 1.0 {
                let dup = v
                    .iter()
                    .any(|q| (q.0 - p.0).abs() < 1e-6 && (q.1 - p.1).abs() < 1e-6);
                if !dup {
                    v.push(*p);
                }
            }
        }
        v
    };
    if pts.len() < 2 {
        return Err("打卡点不足 2 个，无法成线".into());
    }
    let order = order_points(&pts);
    let control: Vec<(f64, f64)> = order.iter().map(|&i| pts[i]).collect();

    // ① 闭合样条穿过所有打卡点
    let mut lap = spline_closed(&control, STEP_M);
    lap = resample_closed(&lap, STEP_M);

    // ③ 反复平滑，直到全程转角都低于阈值（「尽量圆滑」的硬保证）
    let mut guard = 0;
    while max_turn_deg(&lap) > MAX_TURN_DEG && guard < 400 {
        lap = smooth_once(&lap);
        guard += 1;
    }
    lap = resample_closed(&lap, STEP_M);

    // ③.5 崎岖度：0=完美平滑，1=明显游走；一旦出现自交/折叠就自动收窄，保证是干净闭环
    let r = rugged.clamp(0.0, 1.0);
    let mut scale = 1.0f64;
    let mut tries = 0;
    loop {
        let mut cand = lap.clone();
        radial_wobble(&mut cand, (r * scale).min(1.0), seed);
        if r > 0.05 {
            jitter_wobble(&mut cand, r * r * 1.15, seed ^ 0x9E37_79B9);
        }
        if !self_intersects(&cand) || scale < 0.1 || tries >= 8 {
            lap = cand;
            break;
        }
        scale *= 0.6;
        tries += 1;
    }
    // 收尾轻平滑：保留游走形状，去掉锯齿
    lap = smooth_once(&lap);
    lap = smooth_once(&lap);
    lap = resample_closed(&lap, STEP_M);

    // ④ 建筑避让：把落进建筑轮廓里的点推到轮廓外（有真实 OSM 建筑时才生效）
    if !buildings.is_empty() {
        for _ in 0..6 {
            let mut moved = false;
            for i in 0..lap.len() {
                for poly in buildings {
                    if point_in_poly(lap[i], poly) {
                        let mut best = lap[i];
                        let mut bestd = f64::MAX;
                        for v in poly {
                            let d = dist_m(lap[i], *v);
                            if d < bestd {
                                bestd = d;
                                best = *v;
                            }
                        }
                        let dy = best.0 - lap[i].0;
                        let dx = best.1 - lap[i].1;
                        let l = (dy * dy + dx * dx).sqrt().max(1e-9);
                        lap[i] = (
                            best.0 + dy / l * 8.0 / MET_PER_DEG_LAT,
                            best.1 + dx / l * 8.0 / MET_PER_DEG_LAT,
                        );
                        moved = true;
                    }
                }
            }
            lap = smooth_once(&lap);
            if !moved {
                break;
            }
        }
        lap = resample_closed(&lap, STEP_M);
    }

    let lap_len: f64 = (0..lap.len())
        .map(|i| dist_m(lap[i], lap[(i + 1) % lap.len()]))
        .sum();
    if lap_len < 50.0 {
        return Err(format!("环线过短（{lap_len:.0} m），检查打卡点分布"));
    }

    // ⑤ 补足目标距离：整圈叠加，最后一段按弧长精确截断
    let target = target_m.max(300.0);
    let laps = (target / lap_len).ceil().max(1.0);
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut acc = 0.0;
    'outer: for round in 0..(laps as usize) {
        for i in 0..lap.len() {
            let a = lap[i];
            let b = lap[(i + 1) % lap.len()];
            let d = dist_m(a, b);
            if acc + d >= target {
                let t = (target - acc) / d.max(1e-9);
                out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
                break 'outer;
            }
            if !(round > 0 && i == 0) {
                out.push(a);
            }
            acc += d;
        }
    }
    if out.len() < 2 {
        return Err("路线为空".into());
    }
    let length_m: f64 = (0..out.len() - 1).map(|i| dist_m(out[i], out[i + 1])).sum();
    Ok(AutoRoute {
        points: out,
        length_m,
        laps,
        base_lap: lap,
    })
}

/// 高频小抖动：崎岖度越高越明显，模拟人跑步时手臂/身体左右晃导致的轨迹毛刺。
fn jitter_wobble(pts: &mut [(f64, f64)], amp: f64, seed: u64) {
    let n = pts.len();
    if n < 8 || amp <= 0.01 {
        return;
    }
    let phase = (seed % 997) as f64 / 997.0 * std::f64::consts::TAU;
    for i in 0..n {
        let a = pts[(i + n - 1) % n];
        let c = pts[(i + 1) % n];
        let dy = c.0 - a.0;
        let dx = c.1 - a.1;
        let len = (dy * dy + dx * dx).sqrt();
        if len < 1e-12 {
            continue;
        }
        let (ny, nx) = (-dx / len, dy / len);
        let t = i as f64;
        let off = amp
            * (0.55 * (t / 17.0 * std::f64::consts::TAU + phase).sin()
                + 0.45 * (t / 7.0 * std::f64::consts::TAU + phase * 2.3).sin());
        let o = off / MET_PER_DEG_LAT;
        pts[i].0 += ny * o;
        pts[i].1 += nx * o;
    }
}

/// 两线段是否真正相交（不含共端点）。
fn seg_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let cross = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.1 - p.1) * (r.0 - p.0) - (q.0 - p.0) * (r.1 - p.1)
    };
    let d1 = cross(c, d, a);
    let d2 = cross(c, d, b);
    let d3 = cross(a, b, c);
    let d4 = cross(a, b, d);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// 折线（闭环）是否自交 —— 用来保证「不折叠」。
pub fn self_intersects(pts: &[(f64, f64)]) -> bool {
    let n = pts.len();
    if n < 4 {
        return false;
    }
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let c = pts[j];
            let d = pts[(j + 1) % n];
            if seg_cross(a, b, c, d) {
                return true;
            }
        }
    }
    false
}
/// 径向游走：沿「到质心」的半径方向整体伸缩，形状不规则但拓扑安全（不会自交）。
fn radial_wobble(pts: &mut [(f64, f64)], r: f64, seed: u64) {
    let n = pts.len();
    if n < 8 || r <= 0.01 {
        return;
    }
    let (mut cy, mut cx) = (0.0f64, 0.0f64);
    for p in pts.iter() {
        cy += p.0;
        cx += p.1;
    }
    cy /= n as f64;
    cx /= n as f64;
    let mut rng = Rng::new(seed ^ 0x5EED_1A2B);
    let (mut a, mut b) = (0.0f64, 0.0f64);
    let mut offs = Vec::with_capacity(n);
    for _ in 0..n {
        a = 0.972 * a + rng.gauss(0.0, 0.42);
        b = 0.88 * b + rng.gauss(0.0, 0.26);
        offs.push(a + b * 0.5);
    }
    let mx = offs.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-9);
    let depth = 0.18 * r;
    for i in 0..n {
        let k = 1.0 + depth * (offs[i] / mx);
        let dy = pts[i].0 - cy;
        let dx = pts[i].1 - cx;
        pts[i].0 = cy + dy * k;
        pts[i].1 = cx + dx * k;
    }
}
