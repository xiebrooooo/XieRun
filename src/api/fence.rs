//! 电子围栏：POST /api/v1/getGeoFenceForRun。
//! 返回 data.geoFences[]，每个围栏 points[] 为围栏多边形（与打卡点同系，BD-09），
//! passPoints[] 为该围栏内的必经打卡点。
//!
//! 服务端点结构可能只填充 GCJ 字段（glat/glon）而把 BD 字段（lat/lon）置 0，
//! 故优先取 lat/lon，若为 0/缺失则回退 glat/glon 并做 GCJ→BD 转换，避免缓存全 0。

use super::client::{get_field, ApiClient};
use crate::track::geom::gcj02_to_bd09;
use serde_json::{json, Value};

pub const FENCE_PATH: &str = "/api/v1/getGeoFenceForRun";

/// 拉取 data.geoFences[] 原始数组。
fn fetch_geo_fences_raw(client: &mut ApiClient) -> Result<Vec<Value>, String> {
    let body = json!({ "geoFenceUpdateTime": 0 }).to_string();
    let biz = client.call("POST", FENCE_PATH, &body, &[])?;
    Ok(get_field(&biz, "geoFences")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

/// 拉取围栏多边形列表（每个围栏一个闭合环，(lat, lon)，BD-09）。
pub fn fetch_geo_fence(client: &mut ApiClient) -> Result<Vec<Vec<(f64, f64)>>, String> {
    let fences = fetch_geo_fences_raw(client)?;
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    for f in fences {
        let Some(pts) = f.get("points").and_then(|v| v.as_array()) else {
            continue;
        };
        let ring: Vec<(f64, f64)> = pts
            .iter()
            .filter_map(point_xy)
            .filter(|(lat, lon)| !(*lat == 0.0 && *lon == 0.0))
            .collect();
        if ring.len() >= 3 {
            out.push(ring);
        }
    }
    Ok(out)
}

/// 拉取围栏内的必经打卡点（`geoFences[].passPoints[]`）→ 点位对象（含 BD/GCJ 双系字段）。
///
/// 部分学校不在 `v560` 点位接口下发打卡点，只在围栏响应里给 `passPoints`；
/// 此时点位请求会返回空，表现为「打卡点拉不起来」。这里作为点位接口的兜底来源。
pub fn fetch_geo_fence_points(client: &mut ApiClient) -> Result<Vec<Value>, String> {
    let fences = fetch_geo_fences_raw(client)?;
    let mut out: Vec<Value> = Vec::new();
    let mut seen: Vec<(i64, i64)> = Vec::new();
    for f in fences {
        let name_hint = f
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        for key in ["passPoints", "passPointList", "mustPoints"] {
            let Some(arr) = f.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for p in arr {
                let Some((lat, lon)) = point_xy(p) else { continue };
                // 同一坐标在多个围栏里重复时只保留一个（约 1e-6 度去重）
                let key_q = ((lat * 1e6).round() as i64, (lon * 1e6).round() as i64);
                if seen.contains(&key_q) {
                    continue;
                }
                seen.push(key_q);
                let mut obj = json!({
                    "lat": lat,
                    "lon": lon,
                    "isFixed": p.get("isFixed").and_then(|v| v.as_i64()).unwrap_or(1),
                    "pointName": p
                        .get("name")
                        .or_else(|| p.get("pointName"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            if name_hint.is_empty() {
                                "围栏必经点".to_string()
                            } else {
                                name_hint.clone()
                            }
                        }),
                    "source": "fence.passPoints",
                });
                if let Some(id) = p.get("id").or_else(|| p.get("pointId")) {
                    obj["pointId"] = id.clone();
                }
                if let (Some(glat), Some(glon)) = (p.get("glat"), p.get("glon")) {
                    obj["glat"] = glat.clone();
                    obj["glon"] = glon.clone();
                }
                out.push(obj);
            }
        }
    }
    Ok(out)
}

/// 取一个点的 (lat, lon)（BD-09）。优先 lat/lon，缺失或全 0 时回退 glat/glon（GCJ→BD）。
/// 供围栏与 policy 必经点共用（两端点均实测存在「只填 GCJ、BD 置 0」的情况）。
pub(crate) fn point_xy(p: &Value) -> Option<(f64, f64)> {
    let lat = field_num(p, &["lat", "latitude"]);
    let lon = field_num(p, &["lon", "lng", "longitude"]);
    match (lat, lon) {
        (Some(a), Some(o)) if !(a == 0.0 && o == 0.0) => Some((a, o)),
        _ => {
            let (ga, go) = (field_num(p, &["glat"]), field_num(p, &["glon"]));
            match (ga, go) {
                (Some(a), Some(o)) if !(a == 0.0 && o == 0.0) => Some(gcj02_to_bd09(a, o)),
                _ => None,
            }
        }
    }
}

fn field_num(p: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| p.get(*k).and_then(|v| v.as_f64()))
}

#[cfg(test)]
mod tests {
    use super::point_xy;
    use serde_json::json;

    #[test]
    fn point_xy_prefers_bd_then_gcj_fallback() {
        // BD lat/lon 正常
        let p = json!({ "lat": 38.9, "lon": 121.5 });
        assert_eq!(point_xy(&p), Some((38.9, 121.5)));
        // 经纬度字段用 lng
        let p = json!({ "lat": 38.9, "lng": 121.5 });
        assert_eq!(point_xy(&p), Some((38.9, 121.5)));
        // BD 全 0，回退 glat/glon（GCJ）并转 BD
        let p =
            json!({ "lat": 0.0, "lon": 0.0, "glat": 38.8956025774013, "glon": 121.5337497718317 });
        let (lat, lon) = point_xy(&p).unwrap();
        assert!((lat - 38.901678).abs() < 1e-6, "lat={lat}");
        assert!((lon - 121.540241).abs() < 1e-6, "lon={lon}");
        // 全缺
        let p = json!({ "foo": 1 });
        assert_eq!(point_xy(&p), None);
    }
}
