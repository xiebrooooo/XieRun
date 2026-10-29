//! 跑步策略：POST /api/v70103/runModePolicy。
//! 返回 data.runRuleModel.minDistance（提交时 selDistance 用它）与 data.policy。

use super::client::{get_field, ApiClient};
use super::fence::point_xy;
use serde_json::{json, Value};

pub const POLICY_PATH: &str = "/api/v70103/runModePolicy";

pub struct PolicyInfo {
    pub timestamp: i64,
    pub policy: i64,
    pub min_distance: i64,
    pub valid_time: i64,
    /// 必经点（BD 系，与打卡点同系），policy 响应里若有则返回。
    pub must_points: Vec<(f64, f64)>,
    /// 学校配置的跑步区域 id：多区域校区拉点位/提交都要带（`runAreaId`），无则 None。
    pub run_area_id: Option<String>,
}

/// 从 policy 响应 `data` 中防御式提取必经点列表（字段名不确定，逐个尝试）。
fn extract_must_points(v: &serde_json::Value) -> Vec<(f64, f64)> {
    let data = v.get("data").unwrap_or(v);
    for name in [
        "pointList",
        "runPointList",
        "mustPointList",
        "passPointList",
        "mustPoints",
        "passPoints",
        "runPoints",
        "points",
        "nodeList",
        "checkPointList",
    ] {
        let Some(arr) = data.get(name).and_then(|x| x.as_array()) else {
            continue;
        };
        // 复用 fence 的坐标系解析：BD 优先，缺失/全 0 回退 glat/glon（GCJ→BD）并过滤 (0,0)。
        let pts: Vec<(f64, f64)> = arr.iter().filter_map(point_xy).collect();
        if !pts.is_empty() {
            return pts;
        }
    }
    Vec::new()
}

/// 从 policy 响应里提取学校的跑步区域 id（字段名不确定，逐个尝试）。
///
/// 学校未配置区域时返回 None（不传 `runAreaId`，与 App 行为一致）；只认非 0 数值/非空字符串。
fn extract_run_area_id(v: &serde_json::Value) -> Option<String> {
    let data = v.get("data").unwrap_or(v);
    for name in ["runAreaId", "areaId", "zoneId", "runArea", "zone"] {
        let Some(raw) = data.get(name) else { continue };
        if let Some(id) = raw
            .as_i64()
            .filter(|n| *n > 0)
            .map(|n| n.to_string())
            .or_else(|| {
                raw.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && *s != "0")
                    .map(str::to_string)
            })
        {
            return Some(id);
        }
    }
    None
}

/// body：{"runMode":1,"ruleUpdateTime":0,"geoFenceUpdateTime":0,"selectUnid":<unid>,"operateType":0}
pub fn fetch_policy(client: &mut ApiClient) -> Result<PolicyInfo, String> {
    let unid = client
        .login
        .as_ref()
        .map(|s| s.unid.clone())
        .unwrap_or_default();
    let select_unid = unid.parse::<i64>().unwrap_or(0);
    let body = json!({
        "runMode": 1,
        "ruleUpdateTime": 0,
        "geoFenceUpdateTime": 0,
        "selectUnid": select_unid,
        "operateType": 0,
    })
    .to_string();
    let biz = client.call("POST", POLICY_PATH, &body, &[])?;
    let timestamp = get_field(&biz, "timestamp")
        .and_then(|t| t.as_i64())
        .ok_or("policy 响应缺 timestamp")?;
    let policy = get_field(&biz, "policy")
        .and_then(|t| t.as_i64())
        .unwrap_or(0);
    let rule = get_field(&biz, "runRuleModel")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(PolicyInfo {
        timestamp,
        policy,
        min_distance: rule
            .get("minDistance")
            .and_then(|t| t.as_i64())
            .unwrap_or(1000),
        valid_time: rule.get("validTime").and_then(|t| t.as_i64()).unwrap_or(0),
        must_points: extract_must_points(&biz),
        run_area_id: extract_run_area_id(&biz),
    })
}

#[cfg(test)]
mod tests {
    use super::{extract_must_points, extract_run_area_id};
    use serde_json::json;

    #[test]
    fn extracts_run_area_id_defensively() {
        assert_eq!(
            extract_run_area_id(&json!({"data": {"runAreaId": 12}})).as_deref(),
            Some("12")
        );
        assert_eq!(
            extract_run_area_id(&json!({"runAreaId": "9"})).as_deref(),
            Some("9")
        );
        // 0 / 空字符串 / 缺字段都视为「未配置区域」
        assert_eq!(extract_run_area_id(&json!({"data": {"runAreaId": 0}})), None);
        assert_eq!(extract_run_area_id(&json!({"data": {"areaId": ""}})), None);
        assert_eq!(extract_run_area_id(&json!({"data": {}})), None);
    }

    #[test]
    fn extracts_must_points_from_any_known_field() {
        let v = json!({"data": {"pointList": [{"lat": 42.1, "lon": 125.1}]}});
        assert_eq!(extract_must_points(&v).len(), 1);
        let v = json!({"data": {"passPoints": [{"glat": 42.1, "glon": 125.1}]}});
        assert_eq!(extract_must_points(&v).len(), 1);
        let v = json!({"data": {}});
        assert!(extract_must_points(&v).is_empty());
    }
}
