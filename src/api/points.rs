//! 校园点位：POST /api/v560/get/...（sportType=4/1）。
//!
//! runec = 信封(f"{uid}{经度6位}{纬度6位}{起始时间秒级ms整}"，insert→observed 序列化)；
//! sign = MD5(http版URL + 盐)。带 300s 缓存（限流 10603：5 分钟 3 次），失败回退最近缓存。
//!
//! 端点版本/参数会随发版与学校配置漂移（实测端点表里有 `v560/get`、`v560/get/{type}/distance`、
//! `v560/get/destinationRun/points` 三种形态）。旧版写死 `v560/get/1/distance/1`，对部分学校
//! （按距离出点、多区域校区、目的地跑）会返回空，表现就是「打卡点拉不起来」。
//! 现在按候选顺序逐个试，命中非空即返回并记住来源；全部为空时再回退电子围栏的 passPoints。

use super::client::ApiClient;
use super::model;
use crate::crypto::envelope::{build_envelope, OuterOrder};
use crate::crypto::sign::md5_url_sign;
use crate::location::Coordinate;
use crate::track::geom::{bd09_to_gcj02, gcj02_to_bd09};
use serde_json::{json, Value};

/// 兼容常量：旧版固定路径（保留给 CLI/文档引用）。
pub const POINTS_PATH: &str = "/api/v560/get/1/distance/1";

/// 点位请求参数。
#[derive(Debug, Clone, PartialEq)]
pub struct PointsQuery {
    /// 计分跑 sportType（默认 1）。
    pub sport_type: i64,
    /// 本次跑步目标距离（米）；新版接口按距离返回该点位集。
    pub distance_m: i64,
    /// 校区区域 id：学校配置了多个跑步区域时 App 会附带；None 表示不传（与旧版一致）。
    pub run_area_id: Option<String>,
}

impl Default for PointsQuery {
    fn default() -> Self {
        PointsQuery {
            sport_type: 1,
            distance_m: 1,
            run_area_id: None,
        }
    }
}

/// 候选请求（路径, body 里的 sportType）。第一项与旧版完全一致，保证已跑通的学校行为不变。
///
/// 后两条来自 7.3.30 真包端点表：本地线上实测对当前学校返回 404（可能只在特定模式/参数下有效），
/// 因此只放在最后，且不计入运行期候选（见 `runtime_candidates`），给 `probe points` 留个入口。
pub fn candidates(q: &PointsQuery) -> Vec<(String, i64)> {
    let sport = q.sport_type.max(1);
    let dist = q.distance_m.max(1);
    let raw: Vec<(String, i64)> = vec![
        // ① 旧版实测可用请求（body sportType=4）：优先保留，避免回退破坏已跑通的学校
        (POINTS_PATH.to_string(), 4),
        // ② 参考实现形态：/{sportType}/distance/{目标距离}（按距离出点的学校只有这个能拉起）
        (format!("/api/v560/get/{sport}/distance/{dist}"), sport),
        // ③ 同路径换计分跑 sportType（部分学校按 sportType 返回不同点位集）
        (format!("/api/v560/get/{sport}/distance/1"), sport),
        // ④ 基路径（端点表实测存在，当前学校 404）
        ("/api/v560/get".to_string(), sport),
        // ⑤ 目的地跑点位变体（端点表实测存在，当前学校 404）
        ("/api/v560/get/destinationRun/points".to_string(), sport),
    ];
    // 按 (路径, sportType) 去重：同路径不同 sportType 是两种请求，必须都保留
    let mut out: Vec<(String, i64)> = Vec::new();
    for item in raw {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// 点位接口限流：5 分钟内最多 3 次（服务端 error=10603）。
pub const POINTS_RATE_LIMIT_CODE: i64 = 10603;

/// 一次运行最多试几个候选：把配额（3 次/5 分钟）留给下一次运行，
/// 也避免「第一个端点通了但被限流」时把剩下的配额全烧在无效端点上。
pub const MAX_CANDIDATES_PER_RUN: usize = 3;

/// 运行期候选：只取前 `MAX_CANDIDATES_PER_RUN` 个（探测命令不受此限制）。
pub fn runtime_candidates(q: &PointsQuery) -> Vec<(String, i64)> {
    let mut list = candidates(q);
    list.truncate(MAX_CANDIDATES_PER_RUN);
    list
}

/// 点位请求失败原因：带服务端错误码，便于区分「限流」与「该校真的没有点位」。
#[derive(Debug, Clone)]
pub struct PointsError {
    pub code: Option<i64>,
    pub message: String,
}

impl std::fmt::Display for PointsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// 经纬度六位小数字符串。
fn six_digit(v: f64) -> String {
    format!("{v:.6}")
}

/// 拉取实时点位（带缓存回退）。anchor=(lat,lng) 请求锚点。
pub fn fetch_points(
    client: &mut ApiClient,
    anchor: Coordinate,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    fetch_points_ext(client, anchor, None, log)
}

/// run_area_id：学校配置了区域时 App 会附带；未配置则不传（与 App 一致）。
pub fn fetch_points_ext(
    client: &mut ApiClient,
    anchor: Coordinate,
    run_area_id: Option<String>,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    fetch_points_query(
        client,
        anchor,
        &PointsQuery {
            run_area_id,
            ..Default::default()
        },
        log,
    )
}

/// 按完整查询参数拉点位：候选端点轮试 → 围栏必经点 → 缓存。
///
/// 用「目标距离」而不是固定 1 的学校、以及带 runAreaId 的学校，都靠这里的多候选覆盖。
pub fn fetch_points_query(
    client: &mut ApiClient,
    anchor: Coordinate,
    query: &PointsQuery,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    anchor.validate()?;
    // ① TTL 内命中缓存直接返回（缓存点统一补全 BD/GCJ 双系字段）
    if let Some((ts, pts)) = model::load_points_cache_for(anchor) {
        if !pts.is_empty() && crate::crypto::envelope::now_ms() - ts < model::POINTS_TTL_MS {
            log(&format!(
                "[points] 缓存命中（{} 秒前，{} 点）",
                (crate::crypto::envelope::now_ms() - ts) / 1000,
                pts.len()
            ));
            return Ok(normalize_all(pts));
        }
    }

    // ② 逐个候选端点请求（受 5 分钟 3 次的限流约束，最多试 3 个）
    let cands = runtime_candidates(query);
    let mut last_error = String::new();
    for (idx, (path, sport_type)) in cands.iter().enumerate() {
        match request_points(client, anchor, path, *sport_type, query.run_area_id.as_deref()) {
            Ok(pts) if !pts.is_empty() => {
                let pts = normalize_all(pts);
                log(&format!(
                    "√ [points] {} 点（端点 {}，sportType={}{}）",
                    pts.len(),
                    path,
                    sport_type,
                    if query.run_area_id.is_some() {
                        "，含 runAreaId"
                    } else {
                        ""
                    }
                ));
                let _ = model::save_points_cache(anchor, &pts);
                return Ok(pts);
            }
            Ok(_) => {
                log(&format!(
                    "[points] 端点 {path} 无点位（{}/{}）",
                    idx + 1,
                    cands.len()
                ));
            }
            Err(e) if e.code == Some(POINTS_RATE_LIMIT_CODE) => {
                // 限流时换个端点也一样被拒，继续试只会把下一次运行的配额也烧掉
                log(&format!(
                    "⚠ [points] 点位接口限流（5 分钟 3 次）：{}；本轮不再尝试其它端点",
                    e.message
                ));
                last_error = e.message;
                break;
            }
            Err(e) => {
                last_error = e.message.clone();
                log(&format!("[points] 端点 {path} 请求失败：{}", e.message));
            }
        }
    }

    // ③ 电子围栏里的必经打卡点（passPoints）：部分学校点位只在这里下发
    match crate::api::fence::fetch_geo_fence_points(client) {
        Ok(pts) if !pts.is_empty() => {
            let pts = normalize_all(pts);
            log(&format!(
                "√ [points] 点位端点全部为空，改用电子围栏必经点 {} 个",
                pts.len()
            ));
            let _ = model::save_points_cache(anchor, &pts);
            return Ok(pts);
        }
        Ok(_) => log("[points] 电子围栏也没有必经点"),
        Err(e) => log(&format!("[points] 电子围栏必经点获取失败：{e}")),
    }

    // ④ 回退最近一次缓存
    if let Some((_ts, pts)) = model::load_points_cache_for(anchor) {
        if !pts.is_empty() {
            log("[points] 全部接口失败，回退最近一次接口结果缓存");
            return Ok(normalize_all(pts));
        }
    }
    Err(if last_error.is_empty() {
        "点位接口返回空且无缓存（该校可能未配置打卡点/电子围栏）".into()
    } else {
        format!("点位接口全部失败且无缓存：{last_error}")
    })
}

/// 单次点位请求（不落缓存）。`probe points` 也用它逐个候选试探。
///
/// 每次调用都会消耗服务端「5 分钟 3 次」的点位配额，返回的错误里带 `code`，
/// 便于调用方区分限流（10603）与「该校确实没有点位」。
pub fn request_points(
    client: &mut ApiClient,
    anchor: Coordinate,
    path: &str,
    sport_type: i64,
    run_area_id: Option<&str>,
) -> Result<Vec<Value>, PointsError> {
    let uid = client.login.as_ref().map(|s| s.uid).unwrap_or(0);
    let unid = client
        .login
        .as_ref()
        .map(|s| s.unid.clone())
        .unwrap_or_else(|| "0".into());
    let lat = anchor.latitude;
    let lon = anchor.longitude;
    let url = format!("{}{}", model::HOST, path);

    let start_ms = crate::crypto::envelope::now_ms();
    let runec_input = format!(
        "{uid}{}{}{}",
        six_digit(lon),
        six_digit(lat),
        (start_ms / 1000) * 1000
    );
    // runec：共用会话 Four，外层 observed 序
    let runec_env = build_envelope(&mut client.session, &runec_input, OuterOrder::Observed);
    let runec = runec_env.json;

    let mut body = json!({
        "sportType": sport_type,
        "longitude": lon,
        "latitude": lat,
        "sign": md5_url_sign(&url),
        "uuid": uuid::Uuid::new_v4().to_string(),
        "selectedUnid": unid,
        "runec": runec,
    });
    if let Some(area) = run_area_id {
        body["runAreaId"] = json!(area);
    }
    let body = body.to_string();

    let fail = |message: String, code: Option<i64>| PointsError { code, message };
    let out = client
        .envelope_request("POST", &url, &body, crate::crypto::header::UA_IOS, &[])
        .map_err(|e| fail(format!("{path} {e}"), None))?;
    let Some(dec) = out.decrypted else {
        return Err(fail(
            format!("{path} 响应无法解密（可能被风控拦截）"),
            None,
        ));
    };
    let payload = &dec.business;
    if let Some(pts) = extract_points(payload) {
        return Ok(pts);
    }
    let code = payload.get("error").and_then(|e| e.as_i64());
    let message = payload
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("");
    Err(fail(
        format!("{path} 无点位 error={} {message}", code.unwrap_or(0)),
        code,
    ))
}

/// 从响应里取点位数组：顶层 pointsResModels，其次 data 内同名键、再退化到 list/points。
fn extract_points(payload: &Value) -> Option<Vec<Value>> {
    for key in ["pointsResModels", "pointResModels"] {
        if let Some(arr) = super::client::get_field(payload, key).and_then(|m| m.as_array()) {
            if !arr.is_empty() {
                return Some(arr.clone());
            }
        }
    }
    for key in ["list", "points", "records"] {
        if let Some(arr) = payload.get(key).and_then(|m| m.as_array()) {
            let valid: Vec<Value> = arr
                .iter()
                .filter(|p| point_bd(p).is_some())
                .cloned()
                .collect();
            if !valid.is_empty() {
                return Some(valid);
            }
        }
    }
    None
}

/// 统一补全 BD/GCJ 双系字段（缓存里可能是旧版只存了一种坐标系的点）。
fn normalize_all(pts: Vec<Value>) -> Vec<Value> {
    pts.into_iter().map(|p| normalize_point(&p)).collect()
}

/// 点位中心（BD 系）。只统计能解析出的有效点，缺失坐标不会稀释平均值。
#[allow(dead_code)]
pub fn center_bd(points: &[Value]) -> (f64, f64) {
    let valid: Vec<(f64, f64)> = points.iter().filter_map(point_bd).collect();
    if valid.is_empty() {
        return (0.0, 0.0);
    }
    let n = valid.len() as f64;
    (
        valid.iter().map(|(lat, _)| lat).sum::<f64>() / n,
        valid.iter().map(|(_, lon)| lon).sum::<f64>() / n,
    )
}

/// 点位 → (lat, lon) BD 系数组（轨迹输入）。
pub fn points_bd(points: &[Value]) -> Vec<(f64, f64)> {
    points.iter().filter_map(point_bd).collect()
}

/// 统一解析一个点的 BD-09 坐标。
///
/// 服务端在不同版本/场景中可能只填 `lat/lon`（BD）或只填 `glat/glon`（GCJ）。
/// 因此这里优先 BD；缺失或全 0 时再由 GCJ-02 转换，避免打卡点静默变成 (0,0)。
pub fn point_bd(p: &Value) -> Option<(f64, f64)> {
    let bd = read_pair(p, &["lat", "latitude"], &["lon", "lng", "longitude"]);
    if let Some((lat, lon)) = bd.filter(|(lat, lon)| valid_ll(*lat, *lon)) {
        return Some((lat, lon));
    }
    let gcj = read_pair(p, &["glat", "gLat"], &["glon", "gLng"]);
    gcj.filter(|(lat, lon)| valid_ll(*lat, *lon))
        .map(|(lat, lon)| gcj02_to_bd09(lat, lon))
}

/// 统一解析一个点的 GCJ-02 坐标。优先原始 GCJ 字段，缺失时由 BD 反向转换。
pub fn point_gcj(p: &Value) -> Option<(f64, f64)> {
    let gcj = read_pair(p, &["glat", "gLat"], &["glon", "gLng"]);
    if let Some((lat, lon)) = gcj.filter(|(lat, lon)| valid_ll(*lat, *lon)) {
        return Some((lat, lon));
    }
    let bd = read_pair(p, &["lat", "latitude"], &["lon", "lng", "longitude"]);
    bd.filter(|(lat, lon)| valid_ll(*lat, *lon))
        .map(|(lat, lon)| bd09_to_gcj02(lat, lon))
}

/// 规范化为同时含 BD 和 GCJ 两组字段的点对象；无法解析时返回原对象。
pub fn normalize_point(p: &Value) -> Value {
    let bd = point_bd(p);
    let gcj = point_gcj(p);
    let Some((bd_lat, bd_lon)) = bd else {
        return p.clone();
    };
    let (gcj_lat, gcj_lon) = gcj.unwrap_or_else(|| bd09_to_gcj02(bd_lat, bd_lon));
    let mut out = p.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.insert("lat".into(), json!(bd_lat));
        obj.insert("lon".into(), json!(bd_lon));
        obj.insert("glat".into(), json!(gcj_lat));
        obj.insert("glon".into(), json!(gcj_lon));
    }
    out
}

fn read_pair(p: &Value, lat_keys: &[&str], lon_keys: &[&str]) -> Option<(f64, f64)> {
    let lat = lat_keys
        .iter()
        .find_map(|k| p.get(*k).and_then(Value::as_f64))?;
    let lon = lon_keys
        .iter()
        .find_map(|k| p.get(*k).and_then(Value::as_f64))?;
    Some((lat, lon))
}

fn valid_ll(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat.abs() < 1e-9 && lon.abs() < 1e-9)
}

#[cfg(test)]
mod tests {
    use super::{
        candidates, normalize_point, point_bd, point_gcj, points_bd, runtime_candidates,
        PointsQuery, MAX_CANDIDATES_PER_RUN, POINTS_PATH,
    };
    use serde_json::json;

    #[test]
    fn parses_only_bd_points() {
        let p = json!({"lat": 42.859622, "lon": 125.139564});
        let (lat, lon) = point_bd(&p).unwrap();
        assert!((lat - 42.859622).abs() < 1e-9);
        assert!((lon - 125.139564).abs() < 1e-9);
        let (glat, glon) = point_gcj(&p).unwrap();
        assert!((glat - 42.853558).abs() < 1e-6, "glat={glat}");
        assert!((glon - 125.133072).abs() < 1e-6, "glon={glon}");
    }

    #[test]
    fn parses_only_gcj_points() {
        let p = json!({"glat": 42.853558, "glon": 125.133072});
        let (lat, lon) = point_bd(&p).unwrap();
        assert!((lat - 42.859622).abs() < 1e-6, "lat={lat}");
        assert!((lon - 125.139564).abs() < 1e-6, "lon={lon}");
        let (glat, glon) = point_gcj(&p).unwrap();
        assert!((glat - 42.853558).abs() < 1e-9);
        assert!((glon - 125.133072).abs() < 1e-9);
    }

    #[test]
    fn normalizes_and_preserves_both_frames() {
        let p = json!({"pointName": "点位9", "lat": 42.859622, "lon": 125.139564});
        let n = normalize_point(&p);
        assert!((n["lat"].as_f64().unwrap() - 42.859622).abs() < 1e-9);
        assert!((n["glat"].as_f64().unwrap() - 42.853558).abs() < 1e-6);
        assert_eq!(n["pointName"], "点位9");
    }

    #[test]
    fn rejects_missing_or_zero_coordinates() {
        assert!(point_bd(&json!({"lat": 0.0, "lon": 0.0})).is_none());
        assert!(point_bd(&json!({"foo": 1})).is_none());
        assert!(points_bd(&[json!({"lat": 0.0, "lon": 0.0})]).is_empty());
    }

    /// 候选表：第一条必须与旧版线上请求一致，且按距离出点的变体存在。
    #[test]
    fn candidate_order_keeps_legacy_first_and_covers_variants() {
        let q = PointsQuery {
            sport_type: 1,
            distance_m: 2000,
            run_area_id: None,
        };
        let cands = candidates(&q);
        assert_eq!(cands[0], ("/api/v560/get/1/distance/1".to_string(), 4));
        assert!(cands
            .iter()
            .any(|(p, s)| p == "/api/v560/get/1/distance/2000" && *s == 1));
        assert!(cands.iter().any(|(p, _)| p == "/api/v560/get"));
        assert!(cands
            .iter()
            .any(|(p, _)| p == "/api/v560/get/destinationRun/points"));
        // 去重按 (路径, sportType)：不应有完全重复的请求
        let mut seen = cands.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), cands.len());
        // 同路径不同 sportType 是两种请求，都要保留（旧版路径 + 计分跑 sportType）
        assert!(cands
            .iter()
            .any(|(p, s)| p == "/api/v560/get/1/distance/1" && *s == 1));
    }

    /// 运行期候选受配额限制：5 分钟只允许 3 次，绝不能一次运行打满 5 个端点。
    #[test]
    fn runtime_candidates_respect_rate_limit_quota() {
        let q = PointsQuery {
            sport_type: 1,
            distance_m: 2000,
            run_area_id: None,
        };
        let runtime = runtime_candidates(&q);
        assert_eq!(runtime.len(), MAX_CANDIDATES_PER_RUN);
        assert!(runtime.len() <= 3, "点位接口 5 分钟只允许 3 次");
        // 保底的两个变体必须在运行期候选里
        assert!(runtime.iter().any(|(p, s)| *p == POINTS_PATH && *s == 4));
        assert!(runtime
            .iter()
            .any(|(p, s)| p == "/api/v560/get/1/distance/2000" && *s == 1));
    }
}
