//! 学校位置自动检测。
//!
//! 坐标优先取当前校区的电子围栏中心，围栏不可用时回退到打卡点中心；两组数据均为
//! BD-09，可直接写入设备锚点。城市名优先复用该账号跑步记录里的服务端地址，记录
//! 不可用时仅在当前锚点确实靠近学校中心的情况下沿用已配置城市。

use super::client::ApiClient;
use crate::crypto::header::HeaderIdentity;

#[derive(Debug, Clone)]
pub struct SchoolLocation {
    pub campus_id: String,
    pub campus_name: String,
    pub city: Option<String>,
    pub bd_lat: f64,
    pub bd_lon: f64,
    pub source: String,
}

pub fn detect(
    client: &mut ApiClient,
    current: &HeaderIdentity,
    log: &mut dyn FnMut(&str),
) -> Result<SchoolLocation, String> {
    let (campus_id, campus_name) = campus_identity(client);
    let (bd_lat, bd_lon, source) = school_center(client, current, log)?;
    let city =
        record_city_hint(client, log).or_else(|| nearby_current_city(current, bd_lat, bd_lon));
    if let Some(city) = city.as_deref() {
        log(&format!(
            "√ [school] 学校位置已识别：{city} ({bd_lat:.6}, {bd_lon:.6})"
        ));
    } else {
        log("⚠ [school] 已识别学校坐标，但暂无历史记录可确认城市名");
    }
    Ok(SchoolLocation {
        campus_id,
        campus_name,
        city,
        bd_lat,
        bd_lon,
        source: source.into(),
    })
}

fn school_center(
    client: &mut ApiClient,
    current: &HeaderIdentity,
    log: &mut dyn FnMut(&str),
) -> Result<(f64, f64, &'static str), String> {
    match crate::api::fence::fetch_geo_fence(client) {
        Ok(fences) if !fences.is_empty() => {
            let _ = crate::api::model::save_fence_cache(&fences);
            if let Some((lat, lon)) = fence_center(&fences) {
                log(&format!(
                    "√ [school] 电子围栏中心 ({lat:.6}, {lon:.6})，共 {} 个围栏",
                    fences.len()
                ));
                return Ok((lat, lon, "电子围栏"));
            }
            log("⚠ [school] 电子围栏没有有效坐标，尝试打卡点");
        }
        Ok(_) => log("⚠ [school] 电子围栏为空，尝试打卡点"),
        Err(error) => log(&format!("⚠ [school] 电子围栏获取失败，尝试打卡点: {error}")),
    }

    let anchor = current
        .anchor_coordinate()
        .unwrap_or_else(|_| crate::location::default_anchor());
    let points = crate::api::points::fetch_points(client, anchor, log)?;
    let (lat, lon) = crate::api::points::center_bd(&points);
    if valid_ll(lat, lon) {
        log(&format!(
            "√ [school] 打卡点中心 ({lat:.6}, {lon:.6})，共 {} 个有效点",
            points
                .iter()
                .filter(|point| crate::api::points::point_bd(point).is_some())
                .count()
        ));
        return Ok((lat, lon, "打卡点"));
    }
    Err("学校电子围栏和打卡点都没有可用坐标".into())
}

fn campus_identity(client: &ApiClient) -> (String, String) {
    let Some(session) = client.login.as_ref() else {
        return (String::new(), String::new());
    };
    let profile = &session.profile;
    let campus_id = profile
        .get("campusId")
        .and_then(value_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| session.unid.trim().to_string());
    let campus_name = profile
        .get("campusName")
        .and_then(value_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    (campus_id, campus_name)
}

fn value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.trim().to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn record_city_hint(client: &mut ApiClient, log: &mut dyn FnMut(&str)) -> Option<String> {
    let rows = match crate::api::records::fetch_records(client) {
        Ok(rows) => rows,
        Err(error) => {
            log(&format!("⚠ [school] 历史记录地址读取失败: {error}"));
            return None;
        }
    };
    for row in rows.iter().take(3) {
        let Ok(detail) = crate::api::records::fetch_detail(client, row.rrid) else {
            continue;
        };
        if let Some(city) = normalize_prefecture(&detail.address) {
            log(&format!(
                "√ [school] 从历史记录确认城市：{city}（rrid={}）",
                row.rrid
            ));
            return Some(city);
        }
    }
    None
}

fn nearby_current_city(
    current: &HeaderIdentity,
    detected_lat: f64,
    detected_lon: f64,
) -> Option<String> {
    let city = normalize_prefecture(&current.city)?;
    let anchor = current.anchor_coordinate().ok()?;
    if distance_km(
        anchor.latitude,
        anchor.longitude,
        detected_lat,
        detected_lon,
    ) <= 30.0
    {
        Some(city)
    } else {
        None
    }
}

pub(crate) fn normalize_prefecture(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_start_matches("中国");
    if raw.is_empty() {
        return None;
    }
    for suffix in ["自治州", "地区", "盟", "市"] {
        let Some(end) = raw.find(suffix).map(|index| index + suffix.len()) else {
            continue;
        };
        let mut city = raw[..end].trim();
        if let Some(index) = city.rfind("自治区") {
            city = &city[index + "自治区".len()..];
        } else if let Some(index) = city.rfind('省') {
            city = &city[index + '省'.len_utf8()..];
        }
        let city = city.trim();
        if !city.is_empty() {
            return Some(city.to_string());
        }
    }
    None
}

fn fence_center(fences: &[Vec<(f64, f64)>]) -> Option<(f64, f64)> {
    let mut min_lat = f64::INFINITY;
    let mut min_lon = f64::INFINITY;
    let mut max_lat = f64::NEG_INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut count = 0usize;
    for &(lat, lon) in fences.iter().flatten() {
        if !valid_ll(lat, lon) {
            continue;
        }
        min_lat = min_lat.min(lat);
        min_lon = min_lon.min(lon);
        max_lat = max_lat.max(lat);
        max_lon = max_lon.max(lon);
        count += 1;
    }
    (count >= 3).then_some(((min_lat + max_lat) * 0.5, (min_lon + max_lon) * 0.5))
}

fn valid_ll(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat.abs() < 1e-9 && lon.abs() < 1e-9)
}

fn distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let mean_lat = ((lat1 + lat2) * 0.5).to_radians();
    let north = (lat1 - lat2) * 111.132;
    let east = (lon1 - lon2) * 111.320 * mean_lat.cos();
    (north * north + east * east).sqrt()
}

#[cfg(test)]
mod tests {
    use super::{fence_center, normalize_prefecture};

    #[test]
    fn fence_center_uses_bounding_box() {
        let ring = vec![
            (42.840, 125.120),
            (42.840, 125.140),
            (42.860, 125.140),
            (42.860, 125.120),
        ];
        let (lat, lon) = fence_center(&[ring]).unwrap();
        assert!((lat - 42.850).abs() < 1e-9);
        assert!((lon - 125.130).abs() < 1e-9);
    }

    #[test]
    fn normalizes_prefecture_addresses() {
        assert_eq!(normalize_prefecture("辽源市").as_deref(), Some("辽源市"));
        assert_eq!(
            normalize_prefecture("吉林省辽源市龙山区").as_deref(),
            Some("辽源市")
        );
        assert_eq!(
            normalize_prefecture("广西壮族自治区南宁市青秀区").as_deref(),
            Some("南宁市")
        );
        assert_eq!(
            normalize_prefecture("内蒙古自治区锡林郭勒盟").as_deref(),
            Some("锡林郭勒盟")
        );
        assert_eq!(normalize_prefecture("龙山区"), None);
    }
}
