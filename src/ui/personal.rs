//! 专属定制：按当前账号（姓名/学校/院系/班级/学期目标）生成的个性化卡片。
//!
//! 这套东西不是通用模板：文案、问候语、推荐距离都取自本机 accounts.json 里的
//! profile 和「我的页」拉回来的学期数据，换账号会自动跟着变。

use super::{theme, workbench, App};
use eframe::egui;
use egui::RichText;

#[derive(Default, Clone)]
pub struct PersonalStats {
    /// 学期目标（米）
    pub goal_m: f64,
    /// 已完成的学期有效里程（米）
    pub done_m: f64,
    /// 完成百分比（0~100）
    pub percent: f64,
    /// 学期剩余天数
    pub days_left: i64,
}

impl PersonalStats {
    pub fn remain_m(&self) -> f64 {
        (self.goal_m - self.done_m).max(0.0)
    }

    /// 平均每天还得跑多少米
    pub fn per_day_m(&self) -> f64 {
        if self.days_left > 0 {
            self.remain_m() / self.days_left as f64
        } else {
            0.0
        }
    }

    /// 推荐今天跑多少公里（落在 2.5~5.0 的舒适区，保留一位小数）
    pub fn recommend_km(&self) -> f64 {
        let per = self.per_day_m() / 1000.0;
        let v = if per <= 0.0 {
            3.0
        } else {
            per.clamp(2.5, 5.0)
        };
        (v * 10.0).round() / 10.0
    }
}

/// 当前账号的画像（全部来自本机登录态）。
pub struct Profile {
    pub name: String,
    pub account: String,
    pub initial: String,
    pub school: String,
    pub depart: String,
    pub class: String,
    pub nickname: String,
    pub semester: String,
    pub campus_id: String,
    pub year: String,
}

fn s(v: &serde_json::Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn profile(app: &App) -> Option<Profile> {
    let sess = app.session.as_ref()?;
    let p = &sess.profile;
    let name = if sess.name.trim().is_empty() {
        sess.username.clone()
    } else {
        sess.name.clone()
    };
    let initial = name.chars().next().unwrap_or('我').to_string();
    let semester = app
        .user_page
        .info
        .as_ref()
        .map(|i| s(&i.personal_semester, "sname"))
        .filter(|x| !x.is_empty())
        .unwrap_or_default();
    Some(Profile {
        name,
        account: sess.username.clone(),
        initial,
        school: {
            let v = s(p, "campusName");
            if v.is_empty() {
                app.identity.city.clone()
            } else {
                v
            }
        },
        depart: s(p, "depart"),
        class: s(p, "gradeClass"),
        nickname: s(p, "nickname"),
        semester,
        campus_id: s(p, "campusId"),
        year: s(p, "enrollmentYear"),
    })
}

pub fn stats(app: &App) -> Option<PersonalStats> {
    let info = app.user_page.info.as_ref()?;
    let num = |v: &serde_json::Value, keys: &[&str]| -> Option<f64> {
        keys.iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_f64()))
    };
    let s = &info.summary;
    let c = &info.completed;
    let goal = num(s, &["semesterGoal"]).or_else(|| num(c, &["semesterGoal"]))?;
    if goal <= 0.0 {
        return None;
    }
    // 两个接口都可能给 0（recordssummary/semester 常年是 0），取非零的那个
    let d1 = num(s, &["semesterValidDis"]).unwrap_or(0.0);
    let d2 = num(c, &["semesterValidDis"]).unwrap_or(0.0);
    let d3 = num(c, &["curValidDis"]).unwrap_or(0.0);
    let done = if d1 > 0.0 { d1 } else if d2 > 0.0 { d2 } else { d3 };
    let percent = num(s, &["completedProportion"])
        .or_else(|| num(c, &["completedProportion"]))
        .unwrap_or(done / goal * 100.0);
    let days_left = num(c, &["requiredDays"])
        .or_else(|| num(s, &["requiredDays"]))
        .unwrap_or(0.0) as i64;
    Some(PersonalStats {
        goal_m: goal,
        done_m: done,
        percent,
        days_left,
    })
}

fn greeting() -> &'static str {
    use chrono::Timelike;
    match chrono::Local::now().hour() {
        5..=8 => "早上好",
        9..=11 => "上午好",
        12..=13 => "中午好",
        14..=17 => "下午好",
        18..=22 => "晚上好",
        _ => "夜深了",
    }
}

fn avatar(ui: &mut egui::Ui, letter: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size * 0.5, theme::accent_deep());
    painter.circle_stroke(
        rect.center(),
        size * 0.5,
        egui::Stroke::new(1.0, theme::accent_border()),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::proportional(size * 0.46),
        theme::text(),
    );
}

fn progress_bar(ui: &mut egui::Ui, pct: f32, height: f32) {
    let want = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(want, height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, height * 0.5, theme::card_soft());
    let w = (rect.width() * pct.clamp(0.0, 1.0)).max(height);
    let fill = egui::Rect::from_min_size(rect.min, egui::vec2(w, rect.height()));
    painter.rect_filled(fill, height * 0.5, theme::accent());
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.1}%", pct * 100.0),
        egui::FontId::proportional(11.0),
        theme::text(),
    );
}

/// 跑步页顶部的专属卡片：问候 + 学校身份 + 学期进度 + 今日建议距离。
pub fn hero(app: &mut App, ui: &mut egui::Ui) {
    let Some(p) = profile(app) else {
        return;
    };
    let st = stats(app);
    let mut fill_km: Option<f64> = None;
    workbench::card().show(ui, |ui| {
        ui.horizontal(|ui| {
            avatar(ui, &p.initial, 42.0);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!("{}，{}", p.name, greeting()))
                        .size(16.5)
                        .strong()
                        .color(theme::text()),
                );
                let mut bits: Vec<String> = Vec::new();
                if !p.school.is_empty() {
                    bits.push(p.school.clone());
                }
                if !p.depart.is_empty() {
                    bits.push(p.depart.clone());
                }
                if !p.class.is_empty() {
                    bits.push(p.class.clone());
                }
                if !p.semester.is_empty() {
                    bits.push(format!("学期 {}", p.semester));
                }
                ui.label(
                    RichText::new(bits.join(" · "))
                        .size(11.5)
                        .color(theme::text_dim()),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(st) = &st {
                    let rec = st.recommend_km();
                    let btn = egui::Button::new(
                        RichText::new(format!("按推荐填 {rec:.1} km"))
                            .size(13.0)
                            .color(theme::text()),
                    )
                    .fill(theme::accent())
                    .rounding(egui::Rounding::same(9.0))
                    .min_size(egui::vec2(0.0, 34.0));
                    if ui
                        .add(btn)
                        .on_hover_text("按学期剩余里程和剩余天数算出来的今天该跑的距离")
                        .clicked()
                    {
                        fill_km = Some(rec);
                    }
                }
            });
        });
        ui.add_space(8.0);
        match &st {
            Some(st) => {
                progress_bar(ui, (st.percent / 100.0) as f32, 16.0);
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "学期目标 {:.1} km · 已刷 {:.1} km · 还差 {:.1} km · 剩 {} 天 · 平均每天 {:.1} km",
                        st.goal_m / 1000.0,
                        st.done_m / 1000.0,
                        st.remain_m() / 1000.0,
                        st.days_left,
                        st.per_day_m() / 1000.0
                    ))
                    .size(11.5)
                    .color(theme::text_dim()),
                );
            }
            None => {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("学期数据还没同步，点右侧「同步」拉一次")
                            .size(11.5)
                            .color(theme::text_weak()),
                    );
                    if ui.small_button("同步").clicked() {
                        app.refresh_user_page();
                    }
                });
            }
        }
    });
    if let Some(km) = fill_km {
        let km = km as f32;
        app.run_page.dist_min = km;
        app.run_page.dist_max = km;
        app.run_page.preview_stale = true;
    }
    ui.add_space(6.0);
}

/// 「我的」页顶部的名片：把学校、班级、账号、学期进度一次性摆出来。
pub fn profile_card(app: &mut App, ui: &mut egui::Ui) {
    let Some(p) = profile(app) else {
        return;
    };
    let st = stats(app);
    workbench::card().show(ui, |ui| {
        ui.horizontal(|ui| {
            avatar(ui, &p.initial, 52.0);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&p.name).size(19.0).strong().color(theme::text()));
                    if !p.nickname.is_empty() {
                        workbench::pill(ui, &p.nickname, theme::accent(), theme::accent_soft());
                    }
                });
                ui.label(
                    RichText::new(format!("账号 {}", p.account))
                        .size(11.5)
                        .color(theme::text_dim()),
                );
                let mut bits: Vec<String> = Vec::new();
                if !p.school.is_empty() {
                    bits.push(p.school.clone());
                }
                if !p.depart.is_empty() {
                    bits.push(p.depart.clone());
                }
                if !p.class.is_empty() {
                    bits.push(p.class.clone());
                }
                if !p.year.is_empty() {
                    bits.push(format!("{} 级", p.year));
                }
                ui.label(
                    RichText::new(bits.join(" · "))
                        .size(11.5)
                        .color(theme::text_weak()),
                );
            });
        });
        ui.add_space(8.0);
        if let Some(st) = &st {
            progress_bar(ui, (st.percent / 100.0) as f32, 18.0);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for (k, v) in [
                    ("学期", if p.semester.is_empty() { "—".to_string() } else { p.semester.clone() }),
                    ("目标", format!("{:.1} km", st.goal_m / 1000.0)),
                    ("已完成", format!("{:.1} km", st.done_m / 1000.0)),
                    ("还差", format!("{:.1} km", st.remain_m() / 1000.0)),
                    ("剩余天数", format!("{} 天", st.days_left)),
                    ("每天建议", format!("{:.1} km", st.per_day_m() / 1000.0)),
                ] {
                    workbench::pill(ui, &format!("{k} {v}"), theme::text_dim(), theme::card_soft());
                }
            });
        } else {
            ui.label(
                RichText::new("学期目标未同步：点下面的「刷新」拉取最新数据")
                    .size(11.5)
                    .color(theme::text_weak()),
            );
        }
    });
    ui.add_space(6.0);
}

/// 关于页底部的一行专属说明。
pub fn signature(app: &App) -> String {
    match profile(app) {
        Some(p) => {
            let mut bits = vec![format!("{}（{}）", p.name, p.account)];
            if !p.school.is_empty() {
                bits.push(p.school.clone());
            }
            if !p.class.is_empty() {
                bits.push(p.class.clone());
            }
            format!("本版本为 {} 专属定制", bits.join(" · "))
        }
        None => "本版本支持通用账号登录".to_string(),
    }
}
