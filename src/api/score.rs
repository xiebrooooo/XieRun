//! 有效成绩：学期有效次数 / 有效里程 / 目标与达标状态。
//!
//! 与 App「有效成绩」同源的三份数据（端点均来自 7.3.30 实测端点表）：
//! - `POST /api/v55/runnings/recordssummary/semester`    学期汇总（有效次数 / 有效里程）
//! - `POST /api/v41/running/getPersonalSemesterInfo`     个人完成度（学期起止 / 考核方式 / 有效次数兜底）
//! - `POST /api/v44/runnings/personalSemesterCompleted`  完成状态（目标次数，需最近一条 rrid）
//!
//! 注意：`/api/v1/runnings/getUserScore` 是**信用分**，不是有效成绩，别混用。
//! 各校字段有漂移，解析走「多候选键 + 兜底」，原始响应一并返回供核对。

use super::client::{parse_data_field, ApiClient};
use serde_json::{json, Value};

pub const SUMMARY_PATH: &str = super::semester::SUMMARY_PATH;
pub const PERSONAL_PATH: &str = super::user::PERSONAL_SEMESTER_PATH;
pub const COMPLETED_PATH: &str = super::user::SEMESTER_COMPLETED_PATH;

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Score {
    pub semester: String,
    /// 学期起止（格式化后的日期，取不到为空串）
    pub start: String,
    pub end: String,
    pub valid_count: i64,
    pub goal_count: i64,
    pub valid_dis_m: f64,
    pub total_dis_m: f64,
    pub mode: String,
    pub passed: bool,
    #[serde(default)]
    pub raw_personal: Value,
    #[serde(default)]
    pub raw_completed: Value,
}

impl Score {
    /// 有效次数进度（0..=1；目标未知时为 0）。
    pub fn progress(&self) -> f32 {
        if self.goal_count <= 0 {
            0.0
        } else {
            (self.valid_count as f32 / self.goal_count as f32).clamp(0.0, 1.0)
        }
    }

    /// 达标还差几次（已达标或目标未知时为 0）。
    pub fn remaining(&self) -> i64 {
        if self.goal_count <= 0 {
            0
        } else {
            (self.goal_count - self.valid_count).max(0)
        }
    }
}

pub fn fetch(client: &mut ApiClient, log: &mut dyn FnMut(&str)) -> Result<Score, String> {
    let mut score = Score::default();

    // ① 学期汇总：有效次数 / 有效里程 / 总里程
    match client.call("POST", SUMMARY_PATH, "{}", &[]) {
        Ok(biz) => {
            let data = parse_data_field(&biz);
            score.semester = s_of(&data, &["sname", "semesterName"]).unwrap_or_default();
            score.valid_count = i_of(&data, &["semesterValidCount", "validCount"]).unwrap_or(0);
            score.valid_dis_m = f_of(&data, &["semesterValidDis", "validDis"]).unwrap_or(0.0);
            score.total_dis_m = f_of(&data, &["semesterDis", "totalDis"]).unwrap_or(0.0);
            score.goal_count = i_of(
                &data,
                &["semesterGoalCount", "semesterCount", "goalCount"],
            )
            .unwrap_or(0);
        }
        Err(e) => log(&format!("[score] 学期汇总失败: {e}")),
    }

    // ② 个人完成度：学期起止 / 考核方式 / 次数兜底
    match client.call(
        "POST",
        PERSONAL_PATH,
        &json!({ "runMode": 1 }).to_string(),
        &[],
    ) {
        Ok(biz) => {
            let data = parse_data_field(&biz);
            if score.semester.is_empty() {
                score.semester = s_of(&data, &["sname", "semesterName"]).unwrap_or_default();
            }
            score.start = date_of(
                &data,
                &[
                    "semesterStart",
                    "startDate",
                    "startTime",
                    "beginDate",
                    "semesterBeginTime",
                ],
            )
            .unwrap_or_default();
            score.end = date_of(
                &data,
                &[
                    "semesterEnd",
                    "endDate",
                    "endTime",
                    "finishDate",
                    "semesterEndTime",
                ],
            )
            .unwrap_or_default();
            if score.valid_count == 0 {
                score.valid_count = i_of(&data, &["semesterValid", "validCount"]).unwrap_or(0);
            }
            if score.goal_count == 0 {
                score.goal_count = i_of(
                    &data,
                    &["semesterGoalCount", "semesterCount", "goalCount"],
                )
                .unwrap_or(0);
            }
            score.mode = s_of(&data, &["examMode", "assessmentMode", "checkMode"])
                .unwrap_or_default();
            score.raw_personal = data;
        }
        Err(e) => log(&format!("[score] 个人完成度失败（不影响汇总）: {e}")),
    }

    // ③ 完成状态：目标次数（需要最近一条记录的 rrid）
    if let Ok(rows) = super::records::fetch_records(client) {
        if let Some(rrid) = rows.first().map(|r| r.rrid) {
            let body = json!({ "rrid": rrid }).to_string();
            if let Ok(biz) = client.call("POST", COMPLETED_PATH, &body, &[]) {
                let data = parse_data_field(&biz);
                if let Some(n) = i_of(&data, &["semesterGoalCount", "goalCount", "semesterCount"])
                {
                    score.goal_count = n;
                }
                if score.valid_count == 0 {
                    score.valid_count =
                        i_of(&data, &["semesterValidCount", "validCount"]).unwrap_or(0);
                }
                if let Some(p) = bool_of(&data, &["pass", "isPass", "complete", "isComplete"]) {
                    score.passed = p;
                }
                score.raw_completed = data;
            }
        }
    }

    if score.goal_count > 0 && score.valid_count >= score.goal_count {
        score.passed = true;
    }
    Ok(score)
}

fn i_of(v: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|k| match v.get(*k)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    })
}

fn f_of(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| match v.get(*k)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    })
}

fn s_of(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match v.get(*k)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn bool_of(v: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|k| match v.get(*k)? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|i| i != 0),
        Value::String(s) => match s.trim() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

/// 时间字段：毫秒时间戳 → `YYYY-MM-DD`；字符串原样返回（截到日期部分）。
fn date_of(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match v.get(*k)? {
        Value::Number(n) => {
            let ms = n.as_i64()?;
            if ms > 100_000_000_000 {
                use chrono::TimeZone;
                chrono::Local
                    .timestamp_millis_opt(ms)
                    .single()
                    .map(|t| t.format("%Y-%m-%d").to_string())
            } else {
                None
            }
        }
        Value::String(s) if !s.trim().is_empty() => Some(s.chars().take(10).collect()),
        _ => None,
    })
}
