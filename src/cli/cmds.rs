//! CLI 子命令实现。

use super::{fmt_hms, get, jstr, logger, make_client, now_ms, parse_flags, parse_pace, print_rows};
use crate::api::ai::AiMode;
use crate::api::client::ApiClient;
use crate::api::model;

pub fn dispatch(args: Vec<String>) -> i32 {
    let cmd = args.first().map(|x| x.as_str()).unwrap_or("help");
    let rest = args.iter().skip(1).map(|x| x.as_str()).collect::<Vec<_>>();
    match cmd {
        "login" => cmd_login(&rest),
        "logout" => cmd_logout(),
        "accounts" => cmd_accounts(),
        "account-use" => cmd_account_use(&rest),
        "account-remove" => cmd_account_remove(&rest),
        "run" => cmd_run(&rest),
        "run-all" => cmd_run_all(&rest),
        "template" => cmd_template(&rest),
        "ai" => cmd_ai(&rest),
        "ai-list" => cmd_ai_list(),
        "records" => cmd_records(),
        "records-raw" => cmd_records_raw(&rest),
        "record-info" => cmd_record_info(&rest),
        "obs-get" => cmd_obs_get(&rest),
        "obs-sample" => cmd_obs_sample(&rest),
        "track-sample" => cmd_track_sample(&rest),
        "probe" => cmd_probe(&rest),
        "network-build" => cmd_network_build(&rest),
        "network-edit" => cmd_network_edit(&rest),
        "network-connect" => cmd_network_connect(&rest),
        "network-validate" => cmd_network_validate(&rest),
        "ai-records" => cmd_ai_records(&rest),
        "ai-info" => cmd_ai_info(&rest),
        "semester" => cmd_semester(),
        "score" => cmd_score(),
        "cheat" => cmd_cheat(&rest),
        "rank" => cmd_rank(&rest),
        "update" => cmd_update(&rest),
        "help" | "--help" | "-h" => {
            usage();
            0
        }
        _ => {
            eprintln!("未知命令: {cmd}");
            super::usage();
            1
        }
    }
}

fn usage() {
    super::usage();
}

fn cmd_login(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let (Some(user), Some(pw)) = (get(&flags, "user"), get(&flags, "pass")) else {
        eprintln!("缺少 --user / --pass");
        return 1;
    };
    let remember = get(&flags, "remember").is_some()
        || model::load_accounts()
            .find(user)
            .map(|account| account.remember)
            .unwrap_or(false);
    let account = match model::prepare_login_account(user, pw, remember) {
        Ok(account) => account,
        Err(error) => {
            eprintln!("账号准备失败: {error}");
            return 1;
        }
    };
    let identity = account.identity.clone();
    let mut client = ApiClient::new(identity, None);
    let mut log = logger();
    match crate::api::login::login(&mut client, user, pw, &mut log) {
        Ok(s) => {
            if remember {
                println!("凭据已保存（会话失效时自动重登）");
            }
            println!(
                "登录成功 uid={} unid={} name={} DeviceId={}",
                s.uid, s.unid, s.name, s.device_id
            );
            0
        }
        Err(e) => {
            eprintln!("登录失败: {e}");
            1
        }
    }
}

fn cmd_accounts() -> i32 {
    let store = model::load_accounts();
    println!(
        "活动账号：{}（共 {} 个）",
        if store.active.is_empty() {
            "<新账号草稿>"
        } else {
            &store.active
        },
        store.accounts.len()
    );
    for account in &store.accounts {
        let marker = if account.username == store.active {
            "*"
        } else {
            " "
        };
        let username = if account.username.is_empty() {
            "<新账号草稿>"
        } else {
            &account.username
        };
        let name = if account.display_name.is_empty() {
            "-"
        } else {
            &account.display_name
        };
        let logged = if account
            .session
            .as_ref()
            .map(|session| session.is_logged_in())
            .unwrap_or(false)
        {
            "已登录"
        } else {
            "未登录"
        };
        println!(
            "{marker} {username}\t{name}\t{logged}\t{}{}",
            account.identity.device_id,
            if account.remember {
                "\t已记密码"
            } else {
                ""
            }
        );
    }
    0
}

fn cmd_account_use(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(user) = get(&flags, "user") else {
        eprintln!("缺少 --user <账号>");
        return 1;
    };
    match model::switch_account(user) {
        Ok(account) => {
            println!(
                "已切换活动账号：{}，DeviceId={}",
                account.username, account.identity.device_id
            );
            0
        }
        Err(error) => {
            eprintln!("切换失败: {error}");
            1
        }
    }
}

fn cmd_account_remove(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(user) = get(&flags, "user") else {
        eprintln!("缺少 --user <账号>");
        return 1;
    };
    match model::remove_account(user) {
        Ok(store) => {
            println!(
                "已删除账号：{}；当前活动账号：{}",
                user,
                if store.active.is_empty() {
                    "<新账号草稿>"
                } else {
                    &store.active
                }
            );
            0
        }
        Err(error) => {
            eprintln!("删除失败: {error}");
            1
        }
    }
}

fn cmd_logout() -> i32 {
    let identity = model::load_identity();
    let sess = model::load_session();
    let mut client = ApiClient::new(identity, sess.is_logged_in().then_some(sess));
    let mut log = logger();
    crate::api::login::logout(&mut client, &mut log);
    0
}

fn cmd_run(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let dist_km: f32 = get(&flags, "dist")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let pace: f32 = get(&flags, "pace").map(parse_pace).unwrap_or(0.0);
    let ago_min: i64 = get(&flags, "ago").and_then(|v| v.parse().ok()).unwrap_or(0);
    let days_ago: i64 = get(&flags, "days-ago")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .clamp(0, 3);
    let time_spec = get(&flags, "time").unwrap_or("");
    let face = get(&flags, "face")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(true);
    let (manual_altitude, manual_altitude_range) = match get(&flags, "altitude") {
        Some(value) => match crate::track::altitude::parse_spec(value) {
            Ok(Some(crate::track::altitude::AltitudeSpec::Single(value))) => (Some(value), None),
            Ok(Some(crate::track::altitude::AltitudeSpec::Range(range))) => (None, Some(range)),
            Ok(None) => (None, None),
            Err(e) => {
                eprintln!("--altitude {e}");
                return 1;
            }
        },
        None => (None, None),
    };
    let seed: u64 = get(&flags, "seed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let seed = if seed == 0 {
        (now_ms() % 2_147_483_647) as u64
    } else {
        seed
    };
    let cfg = model::load_config();
    let route_mode = crate::track::generate_road::RouteMode::from_str(
        get(&flags, "route").unwrap_or(&cfg.route_mode),
    );

    let dist = if dist_km > 0.0 {
        dist_km as f64 * 1000.0
    } else {
        (1.0 + rand::random::<f32>() * 0.5) as f64 * 1000.0
    };
    let pace_s = if pace > 0.0 {
        pace
    } else {
        360.0 + rand::random::<f32>() * 120.0
    };
    let dur = (dist as f32 / 1000.0 * pace_s) as i64;
    let start_ms = if !time_spec.is_empty() {
        // --time "HH:MM" 配合 --days-ago（0-3）
        let (h, m) = time_spec.split_once(':').unwrap_or((time_spec, "0"));
        let (h, m) = (
            h.parse::<u32>().unwrap_or(7) % 24,
            m.parse::<u32>().unwrap_or(0),
        );
        let base = chrono::Local::now() - chrono::Duration::days(days_ago);
        use chrono::{Datelike, TimeZone};
        chrono::Local
            .with_ymd_and_hms(base.year(), base.month(), base.day(), h, m, 0)
            .single()
            .map(|x| x.timestamp_millis())
            .unwrap_or(now_ms())
    } else if ago_min > 0 {
        now_ms() - ago_min * 60_000
    } else {
        now_ms() - 30 * 60_000 - (rand::random::<f64>() * 270.0 * 60_000.0) as i64
    };

    println!(
        "参数：{:.0}m / {}s / 配速 {}:{:02}/km / 开始 {}",
        dist,
        dur,
        pace_s as i64 / 60,
        pace_s as i64 % 60,
        fmt_hms(start_ms)
    );

    let mut log = logger();
    let params = crate::api::flow::RunParams {
        dist,
        dur,
        start_ms,
        face_check: face as i64,
        manual_altitude,
        manual_altitude_range,
        seed,
        route_mode,
    };
    match crate::api::flow::run_full_flow(&mut client, &params, &mut log) {
        Ok(out) => {
            println!(
                "跑步提交成功 rrid={} uuid={} obs={}/2 verify={}",
                out.result.rrid,
                out.result.uuid,
                out.obs_ok,
                if out.detail_ok { "通过" } else { "未通过" }
            );
            0
        }
        Err(e) => {
            eprintln!("跑步提交失败: {e}");
            1
        }
    }
}

fn cmd_run_all(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let only = account_filter(get(&flags, "only"));
    let exclude = account_filter(get(&flags, "exclude"));
    let stop_on_error = get(&flags, "stop-on-error").is_some();
    let store = model::load_accounts();
    let original_active = store.active.clone();
    let targets: Vec<String> = store
        .accounts
        .iter()
        .filter(|account| !account.is_blank_draft())
        .map(|account| account.username.clone())
        .filter(|username| only.is_empty() || only.contains(username))
        .filter(|username| !exclude.contains(username))
        .collect();

    if targets.is_empty() {
        eprintln!("没有可运行的账号；请先用 login --user <账号> --pass <密码> --remember 添加账号");
        return 1;
    }

    println!(
        "批量运行开始：{} 个账号（失败后{}）",
        targets.len(),
        if stop_on_error {
            "立即停止"
        } else {
            "继续下一个"
        }
    );

    let mut succeeded = 0usize;
    let mut failed = Vec::new();
    for (index, username) in targets.iter().enumerate() {
        println!("\n[{}/{}] 切换账号：{}", index + 1, targets.len(), username);
        if let Err(error) = model::switch_account(username) {
            eprintln!("切换账号失败：{error}");
            failed.push(username.clone());
            if stop_on_error {
                break;
            }
            continue;
        }
        if cmd_run(rest) == 0 {
            succeeded += 1;
        } else {
            failed.push(username.clone());
            if stop_on_error {
                break;
            }
        }
    }

    if let Err(error) = model::switch_account(&original_active) {
        eprintln!("恢复活动账号失败：{error}");
    }
    println!(
        "\n批量运行结束：成功 {} / {}，失败 {}",
        succeeded,
        targets.len(),
        failed.len()
    );
    if !failed.is_empty() {
        eprintln!("失败账号：{}", failed.join(", "));
        return 1;
    }
    0
}

fn account_filter(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split([',', ';', '，', '；'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn cmd_template(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(path) = get(&flags, "file") else {
        eprintln!("缺少 --file <GPX/JSON>");
        return 1;
    };
    match crate::template::load(path) {
        Ok(samples) => {
            let summary = crate::template::summarize(path, &samples);
            println!("本地模板分析（仅预览，不上传）");
            println!("文件：{}", summary.source);
            println!("采样点：{}", summary.samples);
            println!("海拔范围：{:.1}–{:.1} m", summary.min_m, summary.max_m);
            println!(
                "累计上升：{:.1} m；累计下降：{:.1} m",
                summary.gain_m, summary.loss_m
            );
            0
        }
        Err(e) => {
            eprintln!("模板读取失败：{e}");
            1
        }
    }
}

fn cmd_ai(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(sport) = get(&flags, "sport").and_then(|v| v.parse().ok()) else {
        eprintln!("缺少 --sport");
        return 1;
    };
    // 按分钟（--minutes 1-30）或按次（--count 5-1000 步长 5）
    let mode = match get(&flags, "mode").unwrap_or("min") {
        "count" => {
            let reps = get(&flags, "score")
                .or_else(|| get(&flags, "count"))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(5)
                .clamp(5, 1000);
            AiMode::Count {
                reps: (reps / 5) * 5,
            }
        }
        _ => {
            let minutes = get(&flags, "score")
                .or_else(|| get(&flags, "minutes"))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(1)
                .clamp(1, 30);
            AiMode::Minutes { minutes }
        }
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::flow::run_ai_submit(&mut client, sport, mode, &mut log) {
        Ok(biz) => {
            println!(
                "AI 提交成功 服务器={} {}",
                biz.get("error").and_then(|e| e.as_i64()).unwrap_or(0),
                biz.get("message").and_then(|m| m.as_str()).unwrap_or("")
            );
            0
        }
        Err(e) => {
            eprintln!("AI 提交失败: {e}");
            1
        }
    }
}

fn cmd_ai_list() -> i32 {
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::flow::run_ai_list(&mut client, &mut log) {
        Ok(list) => {
            for s in list {
                println!("id={:<4} {}", s.id, s.name);
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

fn cmd_records() -> i32 {
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::flow::run_records(&mut client, &mut log) {
        Ok(rows) => {
            print_rows(&rows);
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

/// 记录列表原始探测：支持自定义 body，打印条数与日期范围。
fn cmd_records_raw(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let body = get(&flags, "body").unwrap_or("{}").to_string();
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match client.call("POST", crate::api::records::RECORDS_PATH, &body, &[]) {
        Ok(biz) => {
            let data = crate::api::client::parse_data_field(&biz);
            let arr = data.as_array().cloned().unwrap_or_default();
            let n = arr.len();
            println!("条数: {n}");
            if let Some(out) = get(&flags, "out") {
                std::fs::write(out, serde_json::to_string_pretty(&arr).unwrap_or_default())
                    .map_err(|e| eprintln!("写入失败: {e}"))
                    .ok();
                println!("已保存 {out}");
            } else if n > 0 {
                println!(
                    "首条: {}",
                    &arr[0].to_string()[..arr[0].to_string().len().min(300)]
                );
                println!(
                    "末条: {}",
                    &arr[n - 1].to_string()[..arr[n - 1].to_string().len().min(300)]
                );
            } else {
                println!(
                    "data: {}",
                    data.to_string()[..data.to_string().len().min(500)].to_string()
                );
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

/// 拉取 OBS 对象（签名 GET），解码 gzip+base64 字段后保存。
fn cmd_obs_get(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(key) = get(&flags, "key") else {
        eprintln!("缺少 --key");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = |s: &str| eprintln!("{s}");
    match crate::api::obs::fetch_object(&mut client, key, &mut log) {
        Ok(v) => {
            let out = get(&flags, "out").unwrap_or("obs_real.json");
            // 解码 gzip+base64 值便于直接阅读
            let decoded = decode_gz_fields(&v);
            std::fs::write(
                out,
                serde_json::to_string_pretty(&decoded).unwrap_or_default(),
            )
            .map_err(|e| eprintln!("写入失败: {e}"))
            .ok();
            println!("已保存 {out}");
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

/// 解码 OBS 对象中 gzip+base64 的字段值（尽力而为）。
fn decode_gz_fields(v: &serde_json::Value) -> serde_json::Value {
    use base64::Engine;
    let mut out = v.clone();
    if let Some(obj) = out.as_object_mut() {
        for (_k, val) in obj.iter_mut() {
            if let Some(s) = val.as_str() {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(s)
                    .ok()
                    .and_then(|raw| {
                        let mut dec = flate2::read::GzDecoder::new(raw.as_slice());
                        use std::io::Read;
                        let mut txt = String::new();
                        dec.read_to_string(&mut txt).ok()?;
                        Some(txt)
                    });
                if let Some(txt) = decoded {
                    let parsed = serde_json::from_str::<serde_json::Value>(&txt)
                        .map(|x| x.to_string())
                        .unwrap_or(txt);
                    *val = serde_json::Value::String(parsed);
                }
            }
        }
    }
    out
}

/// 本地生成一份 OBS 对象样本（不提交；结构对照用）。
fn cmd_obs_sample(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let dist = get(&flags, "dist")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(1050.0);
    let dur = get(&flags, "dur")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(480);
    let rrid = get(&flags, "rrid")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1320000000);
    let seed = get(&flags, "seed")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(7);
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = |s: &str| eprintln!("{s}");
    let anchor = match client.identity.anchor_coordinate() {
        Ok(value) => value,
        Err(e) => {
            eprintln!("定位锚点无效: {e}");
            return 1;
        }
    };
    let pts = match crate::api::points::fetch_points(&mut client, anchor, &mut log) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("点位拉取失败: {e}");
            return 1;
        }
    };
    let pts_bd = crate::api::points::points_bd(&pts);
    let start_ms = crate::crypto::envelope::now_ms() - dur * 1000;
    let track = crate::track::generator::build(
        dist,
        dur,
        seed,
        (anchor.latitude, anchor.longitude),
        start_ms,
        &pts_bd,
    );
    let sess = client.login.clone().unwrap_or_default();
    let uuid = uuid::Uuid::new_v4().to_string().to_uppercase();
    let obj = crate::track::wire::build_obs_object(&track, rrid, &uuid, sess.uid, &pts);
    let out = get(&flags, "out").unwrap_or("obs_ours.json");
    let decoded = decode_gz_fields(&obj);
    std::fs::write(
        out,
        serde_json::to_string_pretty(&decoded).unwrap_or_default(),
    )
    .map_err(|e| eprintln!("写入失败: {e}"))
    .ok();
    println!("已保存 {out}");
    0
}

/// 本地生成一条轨迹样本（不联网、不上传）：用于核对轨迹几何是否符合真人形态。
///
/// 用法：track-sample [--dist m] [--dur s] [--seed n] [--route legacy|road]
///                    [--points <json>] [--net <osm>] [--out <file>] [--rrid n]
fn cmd_track_sample(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let dist = get(&flags, "dist")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2100.0);
    let dur = get(&flags, "dur")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(700);
    let seed = get(&flags, "seed")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20261015);
    let route = crate::track::generate_road::RouteMode::from_str(
        get(&flags, "route").unwrap_or("legacy"),
    );
    let (pts, anchor) = match get(&flags, "points") {
        Some(path) => match load_points_file(path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("读取点位文件失败: {e}");
                return 1;
            }
        },
        None => {
            // 不带点位文件时用一组内置示例点位（仅用于几何检查）
            let sample = vec![
                (38.901678, 121.540241),
                (38.902564, 121.541233),
                (38.900921, 121.542310),
                (38.899823, 121.541010),
                (38.900455, 121.539512),
            ];
            let n = sample.len() as f64;
            let anchor = (
                sample.iter().map(|p| p.0).sum::<f64>() / n,
                sample.iter().map(|p| p.1).sum::<f64>() / n,
            );
            println!("未指定 --points，使用内置示例点位（{} 个）", sample.len());
            (sample, anchor)
        }
    };
    let points_value: Vec<serde_json::Value> = pts
        .iter()
        .map(|(lat, lon)| serde_json::json!({ "lat": lat, "lon": lon }))
        .collect();
    let start_ms = now_ms() - dur * 1000;
    let track = match route {
        crate::track::generate_road::RouteMode::Road => {
            let Some(osm) = get(&flags, "net") else {
                eprintln!("--route road 需要 --net <osm 文件>");
                return 1;
            };
            let mut net = match crate::track::generate_road::load_network_path(osm) {
                Ok(net) => net,
                Err(e) => {
                    eprintln!("加载路网失败: {e}");
                    return 1;
                }
            };
            crate::track::generate_road::align_network(&mut net);
            match crate::track::generate_road::build_road(
                dist, dur, seed, start_ms, &pts, &[], &net,
            ) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("道路路由生成失败: {e}");
                    return 1;
                }
            }
        }
        crate::track::generate_road::RouteMode::Legacy => {
            crate::track::generator::build(dist, dur, seed, anchor, start_ms, &pts)
        }
    };
    let density = crate::track::track_density_stats(&track);
    println!(
        "轨迹：{} 点 · {:.0}m · {}s · 步数 {}",
        track.locations.len(),
        track.totalDistance,
        track.totalTime,
        track.totalSteps
    );
    println!(
        "相邻点间距：中位 {:.1}m · 99分位 {:.1}m · 最大 {:.1}m（真人样本 6-12 / ≤37 / ≤48）",
        density.step_p50, density.step_p99, density.step_max
    );
    println!(
        "丢星点 {} 个：原地 {} · 位移中位 {:.1}m · 最大 {:.1}m（真人 42% 原地、最大 47.9m）",
        density.lost_total,
        density.lost_copies,
        density.lost_p50,
        density.lost_max
    );

    let rrid = get(&flags, "rrid")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1320000000);
    let uuid = uuid::Uuid::new_v4().to_string().to_uppercase();
    let obj = crate::track::wire::build_obs_object(&track, rrid, &uuid, 0, &points_value);
    let out = get(&flags, "out").unwrap_or("track_sample.json");
    let decoded = decode_gz_fields(&obj);
    std::fs::write(
        out,
        serde_json::to_string_pretty(&decoded).unwrap_or_default(),
    )
    .map_err(|e| eprintln!("写入失败: {e}"))
    .ok();
    println!("已保存 {out}");
    0
}

/// 读点位文件：支持 points_cache.json 形态（{anchor, points[]}）或裸数组。
/// 返回 (BD 点位, 锚点)。
fn load_points_file(path: &str) -> Result<(Vec<(f64, f64)>, (f64, f64)), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{path} 不是合法 JSON: {e}"))?;
    let arr = match &json {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Object(_) => json
            .get("points")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| "缺少 points 数组".to_string())?,
        _ => return Err("点位文件既不是数组也不是对象".into()),
    };
    let pts: Vec<(f64, f64)> = arr.iter().filter_map(crate::api::points::point_bd).collect();
    if pts.is_empty() {
        return Err("点位文件里没有可用坐标".into());
    }
    let anchor = json
        .get("anchor")
        .and_then(|a| {
            Some((
                a.get("latitude")?.as_f64()?,
                a.get("longitude")?.as_f64()?,
            ))
        })
        .unwrap_or_else(|| {
            let n = pts.len() as f64;
            (
                pts.iter().map(|p| p.0).sum::<f64>() / n,
                pts.iter().map(|p| p.1).sum::<f64>() / n,
            )
        });
    Ok((pts, anchor))
}

/// 只读探测：把某个协议域的真实响应整段打印出来（不提交、不写缓存）。
///
/// 排查「某些学校打卡点拉不起来」时用它：逐个候选端点会打印各自的返回，
/// 一眼能看出是哪个路径/参数能拿到点位、还是服务端本身就是空的。
fn cmd_probe(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let domain = rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("points");
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = |s: &str| eprintln!("{}", crate::textlog::clean(s));
    match domain {
        "points" => {
            // --lat/--lon 可覆写锚点（用于网格扫描点位池）；不传则用账号定位
            let anchor = match (
                get(&flags, "lat").and_then(|v| v.parse::<f64>().ok()),
                get(&flags, "lon").and_then(|v| v.parse::<f64>().ok()),
            ) {
                (Some(la), Some(lo)) => match crate::location::Coordinate::new(la, lo, 0.0) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("锚点参数无效: {e}");
                        return 1;
                    }
                },
                _ => match client.identity.anchor_coordinate() {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("定位锚点无效: {e}");
                        return 1;
                    }
                },
            };
            let query = crate::api::points::PointsQuery {
                sport_type: get(&flags, "sport")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                distance_m: get(&flags, "dist")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2000),
                run_area_id: get(&flags, "area").map(str::to_string),
            };
            println!(
                "锚点 BD=({:.6},{:.6}) sportType={} distance={} runAreaId={:?}",
                anchor.latitude, anchor.longitude, query.sport_type, query.distance_m, query.run_area_id
            );
            println!(
                "提示：点位接口限流 5 分钟 3 次，每个候选都算一次；出现 10603 就是配额已用完，等 5 分钟再试。"
            );
            // --max 限制候选数（默认 5）；限流时用 --max 1 换取「每个 5 分钟窗口能多打几次」
            let max_c = get(&flags, "max")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(5);
            for (path, sport) in crate::api::points::candidates(&query)
                .into_iter()
                .take(max_c)
            {
                match crate::api::points::request_points(
                    &mut client,
                    anchor,
                    &path,
                    sport,
                    query.run_area_id.as_deref(),
                ) {
                    Ok(pts) => {
                        println!("  {path} (sportType={sport}) -> {} 点", pts.len());
                        for p in &pts {
                            let (lat, lon) =
                                crate::api::points::point_bd(p).unwrap_or((0.0, 0.0));
                            println!(
                                "      {} isFixed={} BD=({lat:.6},{lon:.6})",
                                p.get("pointName").and_then(|v| v.as_str()).unwrap_or("-"),
                                p.get("isFixed").and_then(|v| v.as_i64()).unwrap_or(-1)
                            );
                        }
                    }
                    Err(e) => println!("  {path} (sportType={sport}) -> 失败: {e}"),
                }
            }
        }
        "fence" => match crate::api::fence::fetch_geo_fence(&mut client) {
            Ok(rings) => {
                println!("围栏 {} 个：", rings.len());
                for (i, r) in rings.iter().enumerate() {
                    println!("  [{i}] {} 点，首点 {:?}", r.len(), r.first());
                }
                match crate::api::fence::fetch_geo_fence_points(&mut client) {
                    Ok(pts) => {
                        println!("必经点 {} 个：", pts.len());
                        for p in pts.iter().take(20) {
                            println!("  {p}");
                        }
                    }
                    Err(e) => println!("必经点获取失败: {e}"),
                }
            }
            Err(e) => {
                eprintln!("围栏获取失败: {e}");
                return 1;
            }
        },
        "policy" => match crate::api::policy::fetch_policy(&mut client) {
            Ok(p) => println!(
                "policy={} ts={} minDistance={} validTime={} must_points={} runAreaId={:?}",
                p.policy,
                p.timestamp,
                p.min_distance,
                p.valid_time,
                p.must_points.len(),
                p.run_area_id
            ),
            Err(e) => {
                eprintln!("策略获取失败: {e}");
                return 1;
            }
        },
        "zones" => {
            // 多校区/多区域学校用到的校区区域列表（字段结构未知，整段打印）
            for path in ["/api/v61101/campus/zonelist", "/api/v560/running/campusZoneList"] {
                for method in ["POST", "GET"] {
                    match client.call(method, path, "{}", &[]) {
                        Ok(v) => println!(
                            "{method} {path} ->\n{}",
                            serde_json::to_string_pretty(&v).unwrap_or_default()
                        ),
                        Err(e) => println!("{method} {path} -> 失败: {e}"),
                    }
                }
            }
        }
        "gmttime" => match client.call("GET", "/api/v3/getserver/gmttime", "{}", &[]) {
            Ok(v) => println!(
                "服务端时间：{}\n本地时间：{}",
                serde_json::to_string_pretty(&v).unwrap_or_default(),
                fmt_hms(now_ms())
            ),
            Err(e) => {
                eprintln!("时间获取失败: {e}");
                return 1;
            }
        },
        "ai" => {
            let sport: i64 = get(&flags, "sport")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            match client.call("GET", &format!("/api/v1/sport/ai/info?sportId={sport}"), "{}", &[]) {
                Ok(v) => println!(
                    "sport {sport} 详情：\n{}",
                    serde_json::to_string_pretty(&v).unwrap_or_default()
                ),
                Err(e) => {
                    eprintln!("项目详情获取失败: {e}");
                    return 1;
                }
            }
            match client.call(
                "GET",
                &format!("/api/v66/sport/ai/record/infos?sportId={sport}&pageSize=100&pageNum=1"),
                "{}",
                &[],
            ) {
                Ok(v) => println!(
                    "sport {sport} 记录（按天分组）：\n{}",
                    serde_json::to_string_pretty(&v).unwrap_or_default()
                ),
                Err(e) => eprintln!("记录获取失败: {e}"),
            }
        }
        other => {
            let _ = &mut log;
            eprintln!("未知探测域: {other}（可用：points / fence / policy / zones / gmttime / ai）");
            return 1;
        }
    }
    0
}

/// 单条跑步记录原始详情。
fn cmd_record_info(rest: &[&str]) -> i32 {    let flags = parse_flags(rest);
    let Some(rrid) = get(&flags, "rrid").and_then(|v| v.parse::<i64>().ok()) else {
        eprintln!("缺少 --rrid");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match crate::api::records::fetch_one_record(&mut client, rrid) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => {
            eprintln!("拉取失败: {e}");
            return 1;
        }
    }
    0
}

/// 单条 AI 记录全量详情。
fn cmd_ai_info(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(id) = get(&flags, "id").and_then(|v| v.parse::<i64>().ok()) else {
        eprintln!("缺少 --id");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match crate::api::ai::fetch_record_detail(&mut client, id) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => {
            eprintln!("拉取失败: {e}");
            return 1;
        }
    }
    0
}

fn cmd_ai_records(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(sport) = get(&flags, "sport").and_then(|v| v.parse().ok()) else {
        eprintln!("缺少 --sport");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match crate::api::ai::fetch_records(&mut client, sport, 50) {
        Ok(page) => {
            use chrono::TimeZone;
            for g in page.groups {
                let date = chrono::Local
                    .timestamp_millis_opt(g.score_date)
                    .single()
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_default();
                println!("{date} ×{}：", g.frequency);
                for r in g.records {
                    let grade = if r.rtype == 2 {
                        format!("{:.1} 秒", r.score.parse::<f64>().unwrap_or(0.0) / 1000.0)
                    } else {
                        format!("{} 个", r.score)
                    };
                    let finish = chrono::Local
                        .timestamp_millis_opt(r.score_date)
                        .single()
                        .map(|t| t.format("%H:%M:%S").to_string())
                        .unwrap_or_default();
                    let video = if r.has_video { "有" } else { "-" };
                    println!("  {} {} 完成 {finish} 视频 {video}", r.name, grade);
                }
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

fn cmd_semester() -> i32 {
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::semester::query(&mut client, &mut log) {
        Ok(r) => {
            if let Some(s) = r.summary {
                println!(
                    "学期：{}  有效次数：{}/{}  有效里程：{:.2} km（总 {:.2} km）",
                    s.sname,
                    s.semester_valid_count,
                    s.semester_count,
                    s.semester_valid_dis / 1000.0,
                    s.semester_dis / 1000.0
                );
            }
            if !r.personal_raw.is_null() {
                println!("个人完成度：{}", r.personal_raw);
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

/// 有效成绩：学期有效次数 / 有效里程 / 达标状态（与 App「有效成绩」同源）。
fn cmd_score() -> i32 {
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::score::fetch(&mut client, &mut log) {
        Ok(s) => {
            println!(
                "学期：{}  有效次数：{}/{}  有效里程：{:.2} km",
                if s.semester.is_empty() { "-" } else { &s.semester },
                s.valid_count,
                s.goal_count,
                s.valid_dis_m / 1000.0
            );
            if !s.start.is_empty() || !s.end.is_empty() {
                println!("起止：{} ~ {}", s.start, s.end);
            }
            if !s.mode.is_empty() {
                println!("考核：{}", s.mode);
            }
            if s.goal_count > 0 {
                println!(
                    "达标：{}（还差 {} 次）",
                    if s.passed { "是" } else { "否" },
                    s.remaining()
                );
            } else {
                println!("达标：目标次数未下发");
            }
            if !s.raw_completed.is_null() {
                println!("完成状态原始响应：{}", s.raw_completed);
            }
            0
        }
        Err(e) => {
            eprintln!("有效成绩拉取失败: {e}");
            1
        }
    }
}

fn cmd_cheat(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let page: i64 = get(&flags, "page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::cheat::query(&mut client, page, &mut log) {
        Ok(r) => {
            if r.is_clean() {
                println!("自查：干净（self=null）");
            } else {
                println!("已被标记：{}", r.self_brief());
            }
            println!("全校违规 {} 条", r.list.len());
            for item in r.list.iter().take(20) {
                println!(
                    "  {} | {} | {}",
                    jstr(item, &["name", "userName"]),
                    jstr(item, &["reason", "punishReason", "cause"]),
                    jstr(item, &["createTime", "time", "date"]),
                );
            }
            0
        }
        Err(e) => {
            eprintln!("检查失败: {e}");
            1
        }
    }
}

fn cmd_rank(rest: &[&str]) -> i32 {
    let kind = rest.first().copied().unwrap_or("main");
    let flags = parse_flags(&rest.iter().skip(1).copied().collect::<Vec<_>>());
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let res = match kind {
        "indoor" => {
            let range: i64 = get(&flags, "range")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            crate::api::rank::indoor_rank(&mut client, range, -1)
        }
        "history" => {
            let sort: i64 = get(&flags, "sort")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            crate::api::rank::history_rank(&mut client, sort, -1)
        }
        _ => {
            let rtype: i64 = get(&flags, "type")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let sort: i64 = get(&flags, "sort")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let gender = get(&flags, "gender").and_then(|v| v.parse().ok());
            let date = get(&flags, "date").map(|v| v.to_string());
            crate::api::rank::main_rank(&mut client, rtype, sort, gender, date)
        }
    };
    match res {
        Ok(rows) => {
            for r in rows {
                println!("{:<4} {}  {:.2} km", r.sort, r.name, r.length / 1000.0);
            }
            0
        }
        Err(e) => {
            eprintln!("查询失败: {e}");
            1
        }
    }
}

fn cmd_update(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let check_only = get(&flags, "check").is_some();
    crate::update::cleanup_residue();
    let mut log = logger();
    let release = match crate::update::check_latest(&mut log) {
        Ok(r) => r,
        Err(e) => {
            println!("× 检查更新失败: {e}");
            return 1;
        }
    };
    let Some(rel) = release else {
        println!("√ 已是最新版本（v{}）", crate::update::current_version());
        return 0;
    };
    println!(
        "发现新版本 {}（当前 v{}，{:.1} MB）",
        rel.tag,
        crate::update::current_version(),
        rel.asset_size as f64 / 1024.0 / 1024.0
    );
    if !rel.notes.is_empty() {
        for line in rel.notes.lines().take(8) {
            println!("  {line}");
        }
    }
    if check_only {
        println!("仅检查（--check），未下载。下载地址：{}", rel.asset_url);
        return 0;
    }
    #[cfg(target_os = "android")]
    {
        println!("Android 版请在界面（关于页）中下载并安装更新");
        0
    }
    #[cfg(not(target_os = "android"))]
    {
        println!("开始下载 {}…", rel.asset_name);
        let mut last_pct = u64::MAX;
        let bytes = match crate::update::download(&rel.asset_url, |done, total| {
            if let Some(t) = total.filter(|t| *t > 0) {
                let pct = done * 100 / t;
                if pct != last_pct && pct % 5 == 0 {
                    println!("  {pct}%");
                    last_pct = pct;
                }
            }
        }) {
            Ok(b) => b,
            Err(e) => {
                println!("× 下载失败: {e}");
                return 1;
            }
        };
        println!("下载完成（{} 字节），解包替换…", bytes.len());
        let bin = match crate::update::extract(&rel.asset_name, &bytes) {
            Ok(b) => b,
            Err(e) => {
                println!("× {e}");
                return 1;
            }
        };
        match crate::update::apply(&bin) {
            Ok(()) => {
                println!("√ 已更新到 {}，请重新运行命令", rel.tag);
                0
            }
            Err(e) => {
                println!("× 替换失败: {e}");
                1
            }
        }
    }
}

fn cmd_network_build(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(tracks) = get(&flags, "tracks") else {
        eprintln!("缺少 --tracks <目录或文件>");
        return 1;
    };
    let out = get(&flags, "out").unwrap_or("campus_network.osm");
    let opts = crate::track::network::NetworkBuildOptions {
        tracks: std::path::PathBuf::from(tracks),
        base_osm: get(&flags, "base-osm").map(std::path::PathBuf::from),
        out: std::path::PathBuf::from(out),
        report: get(&flags, "report").map(std::path::PathBuf::from),
        manifest: get(&flags, "manifest").map(std::path::PathBuf::from),
        snap_m: get(&flags, "snap-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(8.0),
        simplify_m: get(&flags, "simplify-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(6.0),
        min_segment_m: get(&flags, "min-segment-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(2.0),
        max_jump_m: get(&flags, "max-jump-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(200.0),
        include_date: get(&flags, "include-date")
            .map(|v| vec![v.to_string()])
            .unwrap_or_default(),
        exclude_date: get(&flags, "exclude-date")
            .map(|v| vec![v.to_string()])
            .unwrap_or_default(),
        include_file: get(&flags, "include-file")
            .map(|v| vec![v.to_string()])
            .unwrap_or_default(),
        exclude_file: get(&flags, "exclude-file")
            .map(|v| vec![v.to_string()])
            .unwrap_or_default(),
    };
    match crate::track::network::build_network(&opts) {
        Ok(report) => {
            let net = &report["network"];
            println!(
                "路网已生成: {} 节点 / {} 有向边 / {} 无向边 / {} 连通分量",
                net["nodes"].as_u64().unwrap_or(0),
                net["directed_edges"].as_u64().unwrap_or(0),
                net["undirected_edges"].as_u64().unwrap_or(0),
                net["components"].as_u64().unwrap_or(0),
            );
            if let Some(path) = get(&flags, "report") {
                println!("报告: {path}");
            }
            0
        }
        Err(e) => {
            eprintln!("路网生成失败: {e}");
            1
        }
    }
}

fn cmd_network_edit(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let (Some(network), Some(edits)) = (get(&flags, "network"), get(&flags, "edits")) else {
        eprintln!("缺少 --network <osm> / --edits <json>");
        return 1;
    };
    let out = get(&flags, "out").unwrap_or("campus_network_edited.osm");
    let opts = crate::track::network::NetworkEditOptions {
        network: std::path::PathBuf::from(network),
        edits: std::path::PathBuf::from(edits),
        out: std::path::PathBuf::from(out),
        report: get(&flags, "report").map(std::path::PathBuf::from),
        snap_m: get(&flags, "snap-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(8.0),
        min_segment_m: get(&flags, "min-segment-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(2.0),
    };
    match crate::track::network::edit_network(&opts) {
        Ok(report) => {
            println!(
                "手动路网已保存: {} 条编辑 / {} 节点 / {} 有向边 / {} 手动边",
                report["applied"].as_array().map(Vec::len).unwrap_or(0),
                report["network"]["nodes"].as_u64().unwrap_or(0),
                report["network"]["directed_edges"].as_u64().unwrap_or(0),
                report["network"]["manual_edges"].as_u64().unwrap_or(0),
            );
            if let Some(path) = get(&flags, "report") {
                println!("报告: {path}");
            }
            0
        }
        Err(e) => {
            eprintln!("手动路网生成失败: {e}");
            1
        }
    }
}

fn cmd_network_connect(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(network) = get(&flags, "network") else {
        eprintln!("缺少 --network <osm>");
        return 1;
    };
    let out = get(&flags, "out").unwrap_or("campus_network_connected.osm");
    let opts = crate::track::network::NetworkConnectOptions {
        network: std::path::PathBuf::from(network),
        out: std::path::PathBuf::from(out),
        report: get(&flags, "report").map(std::path::PathBuf::from),
        max_gap_m: get(&flags, "max-gap-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(50.0),
    };
    match crate::track::network::connect_components(&opts) {
        Ok(report) => {
            println!(
                "路网连接完成: {} -> {} 个连通分量，新增 {} 条连接",
                report["initial_components"].as_u64().unwrap_or(0),
                report["final_components"].as_u64().unwrap_or(0),
                report["connections_added"].as_u64().unwrap_or(0),
            );
            if let Some(path) = get(&flags, "report") {
                println!("报告: {path}");
            }
            0
        }
        Err(e) => {
            eprintln!("路网连接失败: {e}");
            1
        }
    }
}

fn cmd_network_validate(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(network) = get(&flags, "network") else {
        eprintln!("缺少 --network <osm>");
        return 1;
    };
    let opts = crate::track::network::NetworkValidateOptions {
        network: std::path::PathBuf::from(network),
        points: get(&flags, "points").map(std::path::PathBuf::from),
        point_frame: get(&flags, "point-frame").map(str::to_string),
        snap_m: get(&flags, "snap-m")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30.0),
    };
    match crate::track::network::validate_network(&opts) {
        Ok(report) => {
            let text = serde_json::to_string_pretty(&report).unwrap_or_default();
            if let Some(out) = get(&flags, "out") {
                if let Err(e) = std::fs::write(out, &text) {
                    eprintln!("写入 {out} 失败: {e}");
                    return 1;
                }
                println!("校验报告: {out}");
            } else {
                println!("{text}");
            }
            println!(
                "路网校验: {} 节点 / {} 有向边 / {} 连通分量 / 检查点位 {}",
                report["nodes"].as_u64().unwrap_or(0),
                report["directed_edges"].as_u64().unwrap_or(0),
                report["components"].as_u64().unwrap_or(0),
                report["points_checked"].as_u64().unwrap_or(0),
            );
            0
        }
        Err(e) => {
            eprintln!("路网校验失败: {e}");
            1
        }
    }
}
