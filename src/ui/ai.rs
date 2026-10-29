//! AI 运动页：多项目选择 / 按分钟与按次 / 单次提交 / 批量补签。

use super::{mobile, theme, App};
use crate::api::ai::AiMode;
use eframe::egui;

#[derive(Default)]
pub struct AiPage {
    pub list: Vec<crate::api::ai::AiSport>,
    pub selected: usize,
    /// 0=按分钟 1=按次
    pub mode: usize,
    /// 批量补签：天数（1-60）与每天次数
    pub days: i64,
    pub per_day: i64,
    /// 勾选的项目 id
    pub picked: Vec<i64>,
}

/// 批量计划（确认后执行）。
pub struct AiBatchPlan {
    pub sport_ids: Vec<i64>,
    pub days: i64,
    pub per_day: i64,
    pub mode: AiMode,
}

impl App {
    pub fn draw_ai(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.draw_ai_content(ui));
    }

    fn draw_ai_content(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);

        // 项目多选
        ui.label("项目（可多选）：");
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for s in &self.ai_page.list {
                    let picked = self.ai_page.picked.contains(&s.id);
                    let response = if mobile::compact_ui(ui) {
                        ui.add_sized(
                            [96.0, mobile::TOUCH_HEIGHT],
                            egui::SelectableLabel::new(picked, &s.name),
                        )
                    } else {
                        ui.selectable_label(picked, &s.name)
                    };
                    if response.clicked() {
                        if picked {
                            self.ai_page.picked.retain(|&x| x != s.id);
                        } else {
                            self.ai_page.picked.push(s.id);
                        }
                    }
                }
            });
        });

        ui.add_space(6.0);
        mobile::row(ui, |ui| {
            ui.radio_value(&mut self.ai_page.mode, 0, "按分钟");
            ui.radio_value(&mut self.ai_page.mode, 1, "按次");
        });

        egui::Grid::new("ai_grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                if self.ai_page.mode == 0 {
                    ui.label("时长（分钟）：");
                    mobile::drag_i64(
                        ui,
                        "ai_minutes",
                        &mut self.config.ai_minutes,
                        1..=30,
                        1.0,
                        "",
                        " 分钟",
                    );
                    ui.end_row();
                } else {
                    ui.label("个数：");
                    mobile::drag_i64(
                        ui,
                        "ai_reps",
                        &mut self.config.ai_reps,
                        5..=1000,
                        5.0,
                        "",
                        " 个",
                    );
                    ui.end_row();
                }
                ui.label("补签天数（含今天）：");
                mobile::drag_i64(
                    ui,
                    "ai_days",
                    &mut self.ai_page.days,
                    1..=60,
                    1.0,
                    "",
                    " 天",
                );
                ui.end_row();
                ui.label("每天次数：");
                mobile::drag_i64(
                    ui,
                    "ai_per_day",
                    &mut self.ai_page.per_day,
                    1..=10,
                    1.0,
                    "",
                    "",
                );
                ui.end_row();
            });

        ui.add_space(8.0);
        let logged = self.session.is_some();
        let one_enabled = !self.ai_busy && logged && !self.ai_page.list.is_empty();
        mobile::row(ui, |ui| {
            if ui
                .add_enabled(one_enabled, theme::primary_btn("提交成绩"))
                .clicked()
            {
                let sport_id = self
                    .ai_page
                    .list
                    .get(self.ai_page.selected)
                    .map(|s| s.id)
                    .unwrap_or(0);
                self.submit_ai_once(sport_id);
            }
            if self.ai_busy {
                ui.label("提交中…");
            }
        });

        self.draw_single_selector(ui);

        // ── 批量补签 ────────────────────────────────────────────
        ui.add_space(10.0);
        ui.separator();
        ui.label("批量补签（单线程顺序提交，每天随机时刻；提交后按天回读核对并补投）");
        let picked = self.ai_page.picked.clone();
        let days = self.ai_page.days.max(1);
        let per = self.ai_page.per_day.max(1);
        let total = picked.len() as i64 * days * per;
        mobile::row(ui, |ui| {
            let batch_label = format!(
                "批量提交（{}天 × {}次 × {}项 = {} 条）",
                days,
                per,
                picked.len(),
                total
            );
            let btn = theme::primary_btn(&batch_label);
            let response = if mobile::compact_ui(ui) {
                ui.add_enabled_ui(!self.ai_busy && logged && !picked.is_empty(), |ui| {
                    ui.add_sized([ui.available_width(), mobile::TOUCH_HEIGHT], btn)
                })
                .inner
            } else {
                ui.add_enabled(!self.ai_busy && logged && !picked.is_empty(), btn)
            };
            if response.clicked() {
                self.ai_confirm = Some(AiBatchPlan {
                    sport_ids: picked,
                    days,
                    per_day: per,
                    mode: self.ai_mode(),
                });
            }
        });

        if let Some(plan) = self.ai_confirm.take() {
            let total = plan.sport_ids.len() as i64 * plan.days * plan.per_day;
            let mut confirmed = false;
            let mut cancelled = false;
            egui::Window::new(
                egui::RichText::new("确认批量提交")
                    .strong()
                    .color(theme::text()),
            )
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "将顺序提交 {} 条：{} 天（含今天）× 每天 {} 次 × {} 个项目\n提交完成后会回读服务端记录按天核对，不足的自动补投一轮",
                        total,
                        plan.days,
                        plan.per_day,
                        plan.sport_ids.len()
                    ))
                    .color(theme::text()),
                );
                ui.add_space(6.0);
                mobile::row(ui, |ui| {
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                    if ui.add(theme::primary_btn("确认提交")).clicked() {
                        confirmed = true;
                    }
                });
            });
            if confirmed {
                self.run_ai_batch(plan);
            } else if cancelled {
                self.status = "已取消批量提交".into();
            } else {
                self.ai_confirm = Some(plan);
            }
        }
    }

    /// 当前提交意图。
    fn ai_mode(&mut self) -> AiMode {
        if self.ai_page.mode == 0 {
            let minutes = self.config.ai_minutes.clamp(1, 30);
            AiMode::Minutes { minutes }
        } else {
            let reps = (self.config.ai_reps.clamp(5, 1000) / 5) * 5;
            AiMode::Count { reps }
        }
    }

    fn submit_ai_once(&mut self, sport_id: i64) {
        let mode = self.ai_mode();
        let (mode_label, score_label, secs, per_min, consume) = ai_display(mode);
        let identity = self.identity.clone();
        let session = self.session.clone().unwrap_or_default();
        self.ai_busy = true;
        self.status = "AI 成绩提交中…".into();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let payload = match crate::api::flow::run_ai_submit(
                &mut client,
                sport_id,
                mode,
                &mut log,
            ) {
                Ok(biz) => {
                    log(&format!("[ai] 响应：{}", truncate_json(&biz)));
                    serde_json::json!({
                        "ok": true,
                        "sport_id": sport_id,
                        "mode": mode_label,
                        "score": score_label,
                        "secs": secs, "per_min": per_min, "consume": consume,
                        "resp_error": biz.get("error").cloned().unwrap_or(serde_json::Value::Null),
                        "resp_msg": biz.get("message").and_then(|m| m.as_str()).unwrap_or(""),
                    })
                }
                Err(e) => {
                    log(&format!("× AI 提交失败: {e}"));
                    serde_json::json!({ "ok": false, "message": e })
                }
            };
            tx.send(format!("__AI_DONE__{payload}")).ok();
        });
    }

    /// 批量补签：单线程顺序提交。
    ///
    /// 三天数问题都在这条链上：① 每一天用的是「当天确定的时刻」，重试沿用同一个时刻，
    /// 不会因为重试把记录挪到别的日子；② 提交前先按天生成完整清单（今天起往前 N-1 天）；
    /// ③ 提交后回读 `record/infos` 按天核对，缺的当天补投一轮，最后按天报结果。
    fn run_ai_batch(&mut self, plan: AiBatchPlan) {
        let identity = self.identity.clone();
        let session = self.session.clone().unwrap_or_default();
        self.ai_busy = true;
        self.status = "批量补签进行中…".into();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let days = plan.days.clamp(1, 60);
            let per = plan.per_day.clamp(1, 10);
            let sports = plan.sport_ids.clone();
            let total = sports.len() as i64 * days * per;

            // ① 每天每条的提交时刻（今天起往前 days-1 天；今天用今天已过去的时刻）
            let times: Vec<Vec<i64>> = (0..days)
                .map(|day| (0..per).map(|_| random_time_days_ago(day)).collect())
                .collect();

            let mut ok = 0i64;
            let mut done = 0i64;
            let mut day_ok: Vec<i64> = vec![0; days as usize]; // 每天成功条数（提交侧）
            for day in 0..days {
                for k in 0..per {
                    for &sport in &sports {
                        let at = times[day as usize][k as usize];
                        done += 1;
                        match crate::api::ai::upload(&mut client, sport, plan.mode, Some(at)) {
                            Ok(biz) if biz.get("error").and_then(|e| e.as_i64()) == Some(10000) => {
                                ok += 1;
                                day_ok[day as usize] += 1;
                            }
                            Ok(biz) => {
                                let msg =
                                    biz.get("message").and_then(|m| m.as_str()).unwrap_or("");
                                log(&format!(
                                    "× [ai] {} sport={sport} 失败: {msg}（{done}/{total}）",
                                    day_label(day)
                                ));
                            }
                            Err(e) => log(&format!(
                                "× [ai] {} sport={sport} 失败: {e}（{done}/{total}）",
                                day_label(day)
                            )),
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1200));
                    }
                }
                log(&format!(
                    "√ [ai] {} 提交完成（{done}/{total}）",
                    day_label(day)
                ));
            }

            // ② 回读核对：按「天 × 项目」统计服务端实际落库条数（服务器可能丢单/改期）
            let page_size = (total.max(50)).min(500);
            let mut have: std::collections::HashMap<(i64, i64), i64> = std::collections::HashMap::new();
            let mut verified = false;
            for &sport in &sports {
                match crate::api::ai::fetch_records(&mut client, sport, page_size) {
                    Ok(page) => {
                        verified = true;
                        for g in &page.groups {
                            for r in &g.records {
                                if let Some(day) = days_ago_of(r.score_date) {
                                    if (0..days).contains(&day) {
                                        *have.entry((day, sport)).or_insert(0) += 1;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => log(&format!("⚠ [ai] 记录回读失败（跳过核对）: {e}")),
                }
            }

            // ③ 缺的「天 × 项目」补投一轮（沿用当天已抽好的时刻，不会再乱）
            let mut repaired = 0i64;
            let want = per;
            if verified {
                let mut missing: Vec<(i64, i64, i64)> = Vec::new(); // (day, sport, 缺几条)
                for day in 0..days {
                    for &sport in &sports {
                        let got = have.get(&(day, sport)).copied().unwrap_or(0);
                        if got < want {
                            missing.push((day, sport, want - got));
                        }
                    }
                }
                if !missing.is_empty() {
                    log(&format!(
                        "[ai] 核对发现 {} 个「天×项目」不足，补投一轮…",
                        missing.len()
                    ));
                }
                for (day, sport, need) in missing {
                    let mut added = 0i64;
                    for k in 0..per {
                        if added >= need {
                            break;
                        }
                        let at = times[day as usize][k as usize];
                        if let Ok(biz) =
                            crate::api::ai::upload(&mut client, sport, plan.mode, Some(at))
                        {
                            if biz.get("error").and_then(|e| e.as_i64()) == Some(10000) {
                                added += 1;
                                repaired += 1;
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1200));
                    }
                    log(&format!(
                        "√ [ai] {} sport={sport} 补投 {added}/{need}",
                        day_label(day)
                    ));
                }
            }

            // ④ 按天汇总（UI 弹窗直接展示，天数对不上时能一眼看出是哪天）
            let mut server_by_day: Vec<i64> = vec![0; days as usize];
            if verified {
                for (&(day, _), &n) in have.iter() {
                    server_by_day[day as usize] += n;
                }
            }
            let mut lines: Vec<String> = Vec::new();
            let mut short_days = 0i64;
            let want_day = per * sports.len() as i64;
            for day in 0..days {
                let got = if verified {
                    server_by_day[day as usize]
                } else {
                    day_ok[day as usize]
                };
                if got < want_day {
                    short_days += 1;
                }
                lines.push(format!(
                    "{}：提交 {} · 服务端 {} / 要求 {}",
                    day_label(day),
                    day_ok[day as usize],
                    got,
                    want_day
                ));
            }
            if days > 14 {
                // 天数多时弹窗只留前 3 天 + 不足的天，避免刷屏
                let mut brief: Vec<String> = lines.iter().take(3).cloned().collect();
                for (i, l) in lines.iter().enumerate() {
                    if server_by_day[i] < want_day && i >= 3 {
                        brief.push(l.clone());
                    }
                }
                brief.push(format!("（共 {} 天，明细见运行日志）", days));
                lines = brief;
            }

            let payload = serde_json::json!({
                "ok": true, "batch": true,
                "success": ok, "total": total,
                "days": days, "per_day": per,
                "sports": sports.len(),
                "verified": verified,
                "repaired": repaired,
                "short_days": short_days,
                "detail": lines,
            });
            tx.send(format!("__AI_DONE__{payload}")).ok();
        });
    }

    fn draw_single_selector(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self
            .ai_page
            .list
            .iter()
            .map(|s| format!("id={}  {}", s.id, s.name))
            .collect();
        mobile::row(ui, |ui| {
            ui.label("单发项目：");
            let sel_text = names
                .get(self.ai_page.selected)
                .cloned()
                .unwrap_or_else(|| "-".into());
            egui::ComboBox::from_id_salt("ai_combo")
                .selected_text(sel_text)
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.ai_page.selected, i, name.clone());
                    }
                });
        });
    }
}

fn ai_display(mode: AiMode) -> (&'static str, String, f64, i64, f64) {
    // 展示预估：与提交侧同源参数（计次 90 个/分，计时 240 毫秒/个）
    match mode {
        AiMode::Minutes { minutes } => (
            "按分钟",
            format!("{minutes} 分钟"),
            (minutes * 60) as f64,
            (90.0 * minutes as f64).round() as i64,
            (minutes * 60) as f64 * 0.2,
        ),
        AiMode::Count { reps } => {
            let secs = ((reps as f64 / 90.0) * 60.0).round().max(30.0);
            (
                "按次",
                format!("{reps} 个"),
                secs,
                (reps as f64 / secs * 60.0).round() as i64,
                reps as f64 * 0.07,
            )
        }
    }
}

/// days_ago 天前（0=今天）的随机时刻。
///
/// 过去的天取当天 7:00-21:00；今天取「今天 0 点 ~ 现在」之间（不要生成未来时刻，
/// 否则服务端会把它归到别的日期，批量补签的天数就会乱）。
fn random_time_days_ago(days_ago: i64) -> i64 {
    use chrono::{Datelike, Duration, Local, TimeZone};
    let now = crate::crypto::envelope::now_ms();
    let base = Local::now() - Duration::days(days_ago.clamp(0, 60));
    let midnight = Local
        .with_ymd_and_hms(base.year(), base.month(), base.day(), 0, 0, 0)
        .single()
        .map(|t| t.timestamp_millis())
        .unwrap_or(now);
    let (lo, hi) = if days_ago <= 0 {
        // 今天：可用区间是 [0 点, 现在]；有 7:00 以后的部分就优先用 7:00-现在
        let seven = Local
            .with_ymd_and_hms(base.year(), base.month(), base.day(), 7, 0, 0)
            .single()
            .map(|t| t.timestamp_millis())
            .unwrap_or(midnight);
        (seven.min(now), now)
    } else {
        let seven = Local
            .with_ymd_and_hms(base.year(), base.month(), base.day(), 7, 0, 0)
            .single()
            .map(|t| t.timestamp_millis())
            .unwrap_or(midnight);
        let twenty_one = Local
            .with_ymd_and_hms(base.year(), base.month(), base.day(), 21, 0, 0)
            .single()
            .map(|t| t.timestamp_millis())
            .unwrap_or(seven);
        (seven, twenty_one)
    };
    let (lo, hi) = if hi < lo { (midnight.min(now), now.max(midnight)) } else { (lo, hi) };
    let span = (hi - lo).max(0) as f64;
    let ms = lo + (rand::random::<f64>() * span) as i64;
    // 兜底：绝不生成未来时刻
    let t = Local.timestamp_millis_opt(ms.min(now)).single();
    t.map(|t| t.timestamp_millis()).unwrap_or(now)
}

/// 批量里的天标签（与「补签天数（含今天）」下拉一致）。
fn day_label(days_ago: i64) -> String {
    match days_ago {
        0 => "今天".to_string(),
        1 => "昨天".to_string(),
        n => format!("{n} 天前"),
    }
}

/// 毫秒时间戳 → 「距今天几天」（0=今天；未来或超出范围返回 None）。
fn days_ago_of(ms: i64) -> Option<i64> {
    use chrono::{Local, TimeZone};
    let t = Local.timestamp_millis_opt(ms).single()?;
    let today = Local::now().date_naive();
    let days = (today - t.date_naive()).num_days();
    (0..=60).contains(&days).then_some(days)
}

fn truncate_json(v: &serde_json::Value) -> String {
    let s = v.to_string();
    s.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::{day_label, days_ago_of, random_time_days_ago};
    use chrono::{Duration, Local};

    /// 批量用的时刻必须落在目标那一天（跨天 = 天数错乱的头号原因）。
    #[test]
    fn batch_time_stays_on_target_day() {
        for days_ago in 0..=60 {
            let ms = random_time_days_ago(days_ago);
            let want = (Local::now() - Duration::days(days_ago)).date_naive();
            let got = chrono::TimeZone::timestamp_millis_opt(&Local, ms)
                .single()
                .unwrap()
                .date_naive();
            assert_eq!(got, want, "days_ago={days_ago} 落到了 {got}");
        }
    }

    /// 绝不能生成未来时刻（未来的 scoreDate 会被服务端算到别的日期）。
    #[test]
    fn batch_time_is_never_in_future() {
        let now = crate::crypto::envelope::now_ms();
        for days_ago in 0..=3 {
            for _ in 0..40 {
                let ms = random_time_days_ago(days_ago);
                assert!(ms <= now, "days_ago={days_ago} 生成未来时刻 {ms} > {now}");
            }
        }
    }

    /// 回读核对把「毫秒时间戳」折算成「距今天几天」。
    #[test]
    fn record_timestamp_maps_back_to_days_ago() {
        let now = crate::crypto::envelope::now_ms();
        assert_eq!(days_ago_of(now), Some(0));
        let three = (Local::now() - Duration::days(3))
            .date_naive()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let ms = chrono::TimeZone::from_local_datetime(&Local, &three)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(days_ago_of(ms), Some(3));
        // 未来时间戳 / 太旧的时间戳都不计入
        assert_eq!(days_ago_of(now + 86_400_000), None);
        assert_eq!(
            days_ago_of((Local::now() - Duration::days(400)).timestamp_millis()),
            None
        );
    }

    #[test]
    fn day_labels_match_dropdown() {
        assert_eq!(day_label(0), "今天");
        assert_eq!(day_label(1), "昨天");
        assert_eq!(day_label(30), "30 天前");
    }
}
