//! 后台任务与消息处理：IP 获取 / 登录 / 刷新 / 消息协议分发。

use super::{
    App, AI_DETAIL, AI_LIST, AI_RECORDS, CHEAT, FENCE_DONE, IP, LOCATION_DONE, LOGIN_DONE,
    POINTS_DONE, RANK, RECORDS, RUN_DETAIL, SCORE, SEMESTER, UPDATE_CHK, UPDATE_DONE, UPDATE_PROG,
    USER,
};
use crate::api::model;
use crate::track::generate_road::RouteMode;
use crate::update::ReleaseInfo;

/// 提交前预检登录态。
///
/// 服务端单会话：手机 App 或另一台机器登录后，本机 token 立即失效，
/// 直接提交只会拿到 `HTTP 401 / token有误或缓存中token不存在`。
/// 这里先探一次轻量接口（runModePolicy），失效则用本机保存的密码静默重登。
///
/// 返回 `Ok(Some(新会话))` 表示已续期，调用方需据此重建 `ApiClient`。
pub(crate) fn refresh_session_if_needed(
    client: &mut crate::api::client::ApiClient,
    log: &mut dyn FnMut(&str),
) -> Result<Option<model::Session>, String> {
    match crate::api::policy::fetch_policy(client) {
        Ok(_) => Ok(None),
        Err(e) if crate::api::client::is_auth_error(&e) => {
            log(&format!("登录态已失效（{e}），正在自动重新登录…"));
            let fresh = model::relogin_active(log)?;
            log(&format!("自动重登成功 uid={}", fresh.uid));
            Ok(Some(fresh))
        }
        Err(e) => {
            log(&format!("预检接口异常（不影响提交）: {e}"));
            Ok(None)
        }
    }
}


impl App {
    /// 任一后台任务进行中（更新前的打断保护）。
    pub fn any_busy(&self) -> bool {
        self.login_busy
            || self.run_busy
            || self.ai_busy
            || self.records_busy
            || self.data_busy
            || self.user_busy
            || self.device_page.detect_busy
    }

    /// 检查更新；manual=true 时切到关于页并显示状态。
    pub fn check_update(&mut self, manual: bool) {
        if self.update.checking || self.update.downloading {
            return;
        }
        self.update.checking = true;
        self.update.check_error = None;
        if manual {
            self.tab = 8;
            self.status = "正在检查更新…".into();
        }
        self.spawn_job(move |tx| {
            let mut log = |s: &str| {
                tx.send(s.to_string()).ok();
            };
            let payload = match crate::update::check_latest(&mut log) {
                Ok(Some(rel)) => serde_json::json!({ "ok": true, "newer": true, "release": rel }),
                Ok(None) => serde_json::json!({ "ok": true, "newer": false }),
                Err(e) => serde_json::json!({ "ok": false, "message": e }),
            };
            tx.send(format!("{UPDATE_CHK}{payload}")).ok();
        });
    }

    /// 下载并应用更新（桌面：解包替换 exe；Android：APK 落私有目录）。
    pub fn start_update_download(&mut self, rel: ReleaseInfo) {
        if self.update.downloading {
            return;
        }
        self.update.downloading = true;
        self.update.done = 0;
        self.update.total = None;
        self.status = format!("正在下载 {}…", rel.tag);
        self.spawn_job(move |tx| {
            let outcome: Result<serde_json::Value, String> = (|| {
                let mut report = |done: u64, total: Option<u64>| {
                    tx.send(format!("{UPDATE_PROG}{}/{}", done, total.unwrap_or(0)))
                        .ok();
                };
                let bytes = crate::update::download(&rel.asset_url, &mut report)?;
                #[cfg(target_os = "android")]
                {
                    let path = crate::platform::data_dir().join(crate::update::APK_NAME);
                    std::fs::write(&path, &bytes).map_err(|e| format!("保存安装包失败: {e}"))?;
                    Ok(serde_json::json!({ "ok": true, "tag": rel.tag }))
                }
                #[cfg(not(target_os = "android"))]
                {
                    let bin = crate::update::extract(&rel.asset_name, &bytes)?;
                    crate::update::apply(&bin)?;
                    Ok(serde_json::json!({ "ok": true, "tag": rel.tag }))
                }
            })();
            let payload = match outcome {
                Ok(v) => v,
                Err(e) => serde_json::json!({ "ok": false, "message": e }),
            };
            tx.send(format!("{UPDATE_DONE}{payload}")).ok();
        });
    }

    /// 后台拉取电子围栏并落盘（供预览裁剪/居中复用），完成后触发预览刷新。
    /// 仅在真实道路路由模式下才需要围栏，避免经典模式用户也进入该端点请求指纹。
    pub(crate) fn refresh_fence(&self) {
        if self.run_page.route_mode != RouteMode::Road {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let identity = self.identity.clone();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            match crate::api::fence::fetch_geo_fence(&mut client) {
                Ok(f) => {
                    let _ = crate::api::model::save_fence_cache(&f);
                    log(&format!("√ [fence] 电子围栏已缓存 {} 个", f.len()));
                    tx.send(FENCE_DONE.to_string()).ok();
                }
                Err(e) => log(&format!("⚠ [fence] 电子围栏获取失败: {e}")),
            }
        });
    }

    /// 后台拉取实时点位并落盘（供路线预览使用），完成后触发预览刷新。
    /// 两种路线模式都需要点位；缓存仍新鲜时不重复请求（点位接口 5 分钟限 3 次）。
    pub(crate) fn refresh_points(&self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let identity = self.identity.clone();
        if identity.has_unconfigured_default_location() {
            return;
        }
        let Ok(anchor) = identity.anchor_coordinate() else {
            return;
        };
        if let Some((ts, pts)) = crate::api::model::load_points_cache_for(anchor) {
            if !pts.is_empty() && crate::crypto::envelope::now_ms() - ts < crate::api::model::POINTS_TTL_MS
            {
                return;
            }
        }
        // 与提交同参数：目标距离用当前方案，区域 id 由 policy 提供（没有就不传）
        let distance_m = self
            .run_page
            .plan
            .as_ref()
            .map(|p| (p.dist * 1000.0).round().max(1.0) as i64)
            .unwrap_or(1);
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let query = crate::api::points::PointsQuery {
                sport_type: 1,
                distance_m,
                run_area_id: None,
            };
            match crate::api::points::fetch_points_query(&mut client, anchor, &query, &mut log) {
                Ok(pts) => {
                    log(&format!(
                        "√ [points] 点位已缓存 {} 个（供路线预览）",
                        pts.len()
                    ));
                    tx.send(POINTS_DONE.to_string()).ok();
                }
                Err(e) => log(&format!("⚠ [points] 点位获取失败: {e}")),
            }
        });
    }

    /// 自动检测当前校区的电子围栏/打卡点，完成后由消息处理回填城市与 BD-09 锚点。
    pub(crate) fn start_school_location_detection(&mut self, automatic: bool) {
        if self.device_page.detect_busy {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            if !automatic {
                self.status = "请先登录后再检测学校位置".into();
            }
            return;
        };
        if automatic
            && !self
                .identity
                .needs_school_location_for(&session.campus_id())
        {
            return;
        }
        self.device_page.detect_busy = true;
        self.device_page.detect_message = Some(if automatic {
            "正在自动检测学校位置…".into()
        } else {
            "正在重新检测学校位置…".into()
        });
        self.status = "正在检测学校位置…".into();
        self.spawn_school_location_detection(automatic);
    }

    pub(crate) fn spawn_school_location_detection(&self, automatic: bool) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let identity = self.identity.clone();
        let mut username = self.active_account_username();
        if username.is_empty() {
            username = session.username.trim().to_string();
        }
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity.clone(), Some(session));
            let payload =
                match crate::api::school_location::detect(&mut client, &identity, &mut log) {
                    Ok(location) => serde_json::json!({
                        "ok": true,
                        "automatic": automatic,
                        "username": username,
                        "campus_id": location.campus_id,
                        "campus_name": location.campus_name,
                        "city": location.city,
                        "lat": location.bd_lat,
                        "lon": location.bd_lon,
                        "source": location.source,
                    }),
                    Err(error) => serde_json::json!({
                        "ok": false,
                        "automatic": automatic,
                        "username": username,
                        "message": error,
                    }),
                };
            tx.send(format!("{LOCATION_DONE}{payload}")).ok();
        });
    }

    pub(crate) fn fetch_ip(&mut self) {
        self.spawn_job(|tx| {
            let agent = crate::api::client::make_agent();
            let mut log = App::logger(tx.clone());
            let mut ip = String::new();
            for url in [
                "https://api.ipify.org?format=json",
                "https://httpbin.org/ip",
            ] {
                if let Ok(resp) = agent.get(url).call() {
                    if let Ok(text) = resp.into_string() {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            let got = v
                                .get("ip")
                                .or_else(|| v.get("origin"))
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !got.is_empty() {
                                ip = got;
                                break;
                            }
                        }
                    }
                }
            }
            if ip.is_empty() {
                log("公网 IP 获取失败");
                tx.send(format!("{IP}失败")).ok();
            } else {
                log(&format!("公网 IP：{ip}"));
                tx.send(format!("{IP}{ip}")).ok();
            }
        });
    }

    pub(crate) fn do_login(&mut self) {
        let username = self.username.trim().to_string();
        if username.is_empty() {
            self.status = "请填写账号".into();
            return;
        }
        // 已勾选“记住密码”的账号可凭保存的密码免密登录。
        if self.password.is_empty() {
            if let Some(saved) = model::saved_password(&username) {
                self.password = saved;
                self.remember = true;
            }
        }
        if self.password.is_empty() {
            self.status = "请填写密码".into();
            return;
        }
        let password = self.password.clone();
        let remember = self.remember;
        let account = match model::prepare_login_account(&username, &password, remember) {
            Ok(account) => account,
            Err(error) => {
                self.status = format!("× 无法创建账号记录：{error}");
                return;
            }
        };
        self.username = account.username.clone();
        self.identity = account.identity.clone();
        self.device_buf = account.identity.clone();
        self.accounts = model::load_accounts();
        self.new_account_mode = false;
        let identity = account.identity;
        self.login_busy = true;
        self.status = format!("登录中（{username}）…");
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, None);
            let msg = match crate::api::login::login(&mut client, &username, &password, &mut log) {
                Ok(sess) => {
                    log(&format!(
                        "√ 登录成功 uid={} unid={} name={}",
                        sess.uid, sess.unid, sess.name
                    ));
                    let payload = serde_json::json!({
                        "ok": true,
                        "session": {
                            "uid": sess.uid, "token": sess.token,
                            "unid": sess.unid, "name": sess.name,
                            "weight": sess.weight, "username": username,
                            "device_id": sess.device_id,
                            "profile": sess.profile,
                        },
                    });
                    format!("{LOGIN_DONE}{payload}")
                }
                Err(e) => {
                    log(&format!("× 登录失败: {e}"));
                    format!(
                        "{LOGIN_DONE}{}",
                        serde_json::json!({"ok": false, "message": e})
                    )
                }
            };
            tx.send(msg).ok();
        });
    }

    pub(crate) fn do_logout(&mut self) {
        let identity = self.identity.clone();
        let session = self.session.clone();
        self.persist_active_account_fields();
        let mut username = self.active_account_username();
        if username.is_empty() {
            username = session
                .as_ref()
                .map(|session| session.username.trim().to_string())
                .unwrap_or_default();
        }
        if !username.is_empty() {
            model::clear_session_for(&username);
        }
        self.session = None;
        self.accounts = model::load_accounts();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            if let Some(sess) = session {
                let mut client = crate::api::client::ApiClient::new(identity, Some(sess));
                crate::api::login::logout(&mut client, &mut log);
            }
        });
        self.status = "已登出".into();
    }

    pub fn refresh_ai_list(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.ai_busy = true;
        self.status = "AI 列表拉取中…".into();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let payload = match crate::api::flow::run_ai_list(&mut client, &mut log) {
                Ok(list) => serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()),
                Err(e) => {
                    log(&format!("× AI 列表拉取失败: {e}"));
                    "[]".to_string()
                }
            };
            tx.send(format!("{AI_LIST}{payload}")).ok();
        });
    }

    pub fn refresh_records(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.records_busy = true;
        self.status = "记录拉取中…".into();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let payload = match crate::api::flow::run_records(&mut client, &mut log) {
                Ok(rows) => serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
                Err(e) => {
                    log(&format!("× 记录拉取失败: {e}"));
                    "[]".to_string()
                }
            };
            tx.send(format!("{RECORDS}{payload}")).ok();
        });
    }

    /// 我的页数据聚合。
    pub fn refresh_user_page(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.user_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let info = crate::api::user::fetch_my_info(&mut client, &mut log);
            let payload = serde_json::json!({
                "profile": info.profile,
                "home_page": info.home_page,
                "personal_semester": info.personal_semester,
                "summary": info.summary,
                "completed": info.completed,
            });
            log("√ [user] 我的页数据已更新");
            tx.send(format!("{USER}{payload}")).ok();
            tx.send(format!("{USER}done")).ok();
        });
    }

    /// 拉取单条跑步详情。
    pub fn fetch_run_detail(&mut self, rrid: i64) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let identity = self.identity.clone();
        self.records_page.detail_loading = true;
        self.spawn_job(move |tx| {
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            match crate::api::records::fetch_detail(&mut client, rrid) {
                Ok(d) => {
                    let payload = serde_json::to_string(&d).unwrap_or_default();
                    tx.send(format!("{RUN_DETAIL}{payload}")).ok();
                }
                Err(e) => {
                    let payload = serde_json::json!({ "statusInfo": format!("详情拉取失败：{e}") });
                    tx.send(format!("{RUN_DETAIL}{payload}")).ok();
                }
            }
        });
    }

    /// 拉取单条 AI 记录全量详情。
    pub fn fetch_ai_detail(&mut self, id: i64) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let identity = self.identity.clone();
        self.records_page.ai_detail_loading = true;
        self.spawn_job(move |tx| {
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let payload = match crate::api::ai::fetch_record_detail(&mut client, id) {
                Ok(d) => d.to_string(),
                Err(e) => serde_json::json!({ "fetchError": e }).to_string(),
            };
            tx.send(format!("{AI_DETAIL}{payload}")).ok();
        });
    }

    /// AI 记录：遍历全部项目合并（按天排序）。
    pub fn refresh_ai_records(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.records_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            // 先拉项目列表
            let list = match crate::api::ai::fetch_list(&mut client) {
                Ok(l) => l,
                Err(e) => {
                    log(&format!("× [ai] 项目列表拉取失败: {e}"));
                    tx.send(format!("{AI_RECORDS}{{\"groups\":[],\"total\":0}}"))
                        .ok();
                    return;
                }
            };
            let mut all_groups: Vec<crate::api::ai::AiRecordGroup> = Vec::new();
            let mut total = 0i64;
            for sport in &list {
                match crate::api::ai::fetch_records(&mut client, sport.id, 50) {
                    Ok(page) => {
                        for mut g in page.groups {
                            for r in &mut g.records {
                                r.name = format!("{} {}", sport.name, r.name);
                            }
                            all_groups.push(g);
                        }
                        total += page.total_count;
                    }
                    Err(e) => log(&format!("× [ai] {} 拉取失败: {e}", sport.name)),
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            all_groups.sort_by(|a, b| b.score_date.cmp(&a.score_date));
            let count: usize = all_groups.iter().map(|g| g.records.len()).sum();
            log(&format!("√ [ai] {} 个项目共 {} 条记录", list.len(), count));
            let payload = serde_json::json!({ "groups": all_groups, "total": total });
            tx.send(format!("{AI_RECORDS}{payload}")).ok();
            tx.send("__AI_RECORDS_DONE__".to_string()).ok();
        });
    }

    /// 学期 + 违规自查（登录 / 跑步提交成功后自动调用）。
    pub fn refresh_data_page(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let identity = self.identity.clone();
        self.data_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            match crate::api::semester::query(&mut client, &mut log) {
                Ok(r) => {
                    if let Some(s) = r.summary {
                        log(&format!(
                            "√ [semester] {} 有效 {}/{} 次，有效里程 {:.2} km",
                            s.sname,
                            s.semester_valid_count,
                            s.semester_count,
                            s.semester_valid_dis / 1000.0
                        ));
                        let payload = serde_json::to_string(&s).unwrap_or_default();
                        tx.send(format!("{SEMESTER}{payload}")).ok();
                    } else {
                        tx.send(format!("{SEMESTER}{{}}")).ok();
                    }
                }
                Err(e) => {
                    log(&format!("× [semester] 拉取失败: {e}"));
                    tx.send(format!("{SEMESTER}{{}}")).ok();
                }
            }
            match crate::api::cheat::query(&mut client, 1, &mut log) {
                Ok(r) => {
                    let payload = serde_json::json!({ "self": r.self_info, "list": r.list });
                    tx.send(format!("{CHEAT}{}", payload)).ok();
                }
                Err(e) => {
                    log(&format!("× [cheat] 检查失败: {e}"));
                    tx.send(format!("{CHEAT}{{\"self\":null,\"list\":[]}}"))
                        .ok();
                }
            }
            // 有效成绩：学期有效次数 / 有效里程 / 达标状态
            match crate::api::score::fetch(&mut client, &mut log) {
                Ok(s) => {
                    log(&format!(
                        "√ [score] 有效 {}/{} 次，有效里程 {:.2} km{}",
                        s.valid_count,
                        s.goal_count,
                        s.valid_dis_m / 1000.0,
                        if s.passed { "（已达标）" } else { "" }
                    ));
                    let payload = serde_json::to_string(&s).unwrap_or_default();
                    tx.send(format!("{SCORE}{payload}")).ok();
                }
                Err(e) => log(&format!("× [score] 拉取失败: {e}")),
            }
        });
    }

    pub fn refresh_cheat_only(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.data_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            match crate::api::cheat::query(&mut client, 1, &mut log) {
                Ok(r) => {
                    let payload = serde_json::json!({ "self": r.self_info, "list": r.list });
                    tx.send(format!("{CHEAT}{}", payload)).ok();
                }
                Err(e) => {
                    log(&format!("× [cheat] 检查失败: {e}"));
                    tx.send(format!("{CHEAT}{{\"self\":null,\"list\":[]}}"))
                        .ok();
                }
            }
        });
    }

    /// 榜单查询：kind=main|indoor|history。
    pub fn refresh_rank(&mut self, kind: &str, subtype: i64) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        let kind = kind.to_string();
        self.data_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let res: Result<Vec<crate::api::rank::RankRow>, String> = (|| {
                match kind.as_str() {
                    "main" => {
                        // 当天无数据时自动回退最近 3 天（当日累计榜凌晨常为空）
                        let (rtype, sort_type) = (subtype / 10, subtype % 10);
                        let mut rows =
                            crate::api::rank::main_rank(&mut client, rtype, sort_type, None, None)?;
                        let mut back = 0;
                        while rows.is_empty() && back < 3 {
                            back += 1;
                            let d = (chrono::Local::now() - chrono::Duration::days(back))
                                .format("%Y-%m-%d")
                                .to_string();
                            log(&format!("[rank] 当天暂无数据，回退查询 {d}"));
                            rows = crate::api::rank::main_rank(
                                &mut client,
                                rtype,
                                sort_type,
                                None,
                                Some(d),
                            )?;
                        }
                        Ok(rows)
                    }
                    "indoor" => crate::api::rank::indoor_rank(&mut client, subtype, -1),
                    _ => crate::api::rank::history_rank(&mut client, subtype, -1),
                }
            })();
            let rows = match res {
                Ok(r) => r,
                Err(e) => {
                    log(&format!("× [rank] 查询失败: {e}"));
                    Vec::new()
                }
            };
            log(&format!("[rank] {} 行", rows.len()));
            tx.send(format!(
                "{RANK}{}",
                serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
            ))
            .ok();
        });
    }
}
