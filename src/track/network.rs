//! 从历史 OBS 轨迹自动生成候选校园路网，并做路网/打卡点校验。
//!
//! 输入轨迹中的 `gLat/gLng` 按 GCJ-02 处理，输出 OSM 统一为 WGS84；
//! 生成的是「候选路网」，上线前仍应结合卫星图或实地走一遍确认。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime, TimeZone};
use serde_json::{json, Value};

use super::geom::{bd09_to_wgs84, gcj02_to_wgs84};
use route_planner::{Coord, RoadGraph};

#[derive(Clone, Debug)]
pub struct NetworkBuildOptions {
    pub tracks: PathBuf,
    pub base_osm: Option<PathBuf>,
    pub out: PathBuf,
    pub report: Option<PathBuf>,
    pub manifest: Option<PathBuf>,
    pub snap_m: f64,
    pub simplify_m: f64,
    pub min_segment_m: f64,
    pub max_jump_m: f64,
    pub include_date: Vec<String>,
    pub exclude_date: Vec<String>,
    pub include_file: Vec<String>,
    pub exclude_file: Vec<String>,
}

impl Default for NetworkBuildOptions {
    fn default() -> Self {
        Self {
            tracks: PathBuf::new(),
            base_osm: None,
            out: PathBuf::from("campus_network.osm"),
            report: None,
            manifest: None,
            snap_m: 8.0,
            simplify_m: 6.0,
            min_segment_m: 2.0,
            max_jump_m: 200.0,
            include_date: Vec::new(),
            exclude_date: Vec::new(),
            include_file: Vec::new(),
            exclude_file: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NetworkEditOptions {
    pub network: PathBuf,
    pub edits: PathBuf,
    pub out: PathBuf,
    pub report: Option<PathBuf>,
    pub snap_m: f64,
    pub min_segment_m: f64,
}

impl Default for NetworkEditOptions {
    fn default() -> Self {
        Self {
            network: PathBuf::new(),
            edits: PathBuf::new(),
            out: PathBuf::from("campus_network_edited.osm"),
            report: None,
            snap_m: 8.0,
            min_segment_m: 2.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NetworkConnectOptions {
    pub network: PathBuf,
    pub out: PathBuf,
    pub report: Option<PathBuf>,
    pub max_gap_m: f64,
}

impl Default for NetworkConnectOptions {
    fn default() -> Self {
        Self {
            network: PathBuf::new(),
            out: PathBuf::from("campus_network_connected.osm"),
            report: None,
            max_gap_m: 50.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NetworkValidateOptions {
    pub network: PathBuf,
    pub points: Option<PathBuf>,
    pub point_frame: Option<String>,
    pub snap_m: f64,
}

impl Default for NetworkValidateOptions {
    fn default() -> Self {
        Self {
            network: PathBuf::new(),
            points: None,
            point_frame: None,
            snap_m: 30.0,
        }
    }
}

#[derive(Clone, Debug)]
struct NamedPoint {
    name: String,
    coord: Coord,
}

#[derive(Clone, Debug)]
struct ParsedTrack {
    points: Vec<Coord>,
    date: Option<String>,
    rrid: String,
    first_flag_ms: Option<i64>,
    gain_time: Option<String>,
}

#[derive(Clone, Debug)]
struct ManualEdit {
    points: Vec<Coord>,
    bidirectional: bool,
    note: String,
    frame: PointFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointFrame {
    Auto,
    Bd09,
    Gcj02,
    Wgs84,
}

impl PointFrame {
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "bd09" | "bd-09" | "bd_09" => Ok(Self::Bd09),
            "gcj02" | "gcj-02" | "gcj_02" => Ok(Self::Gcj02),
            "wgs84" | "wgs-84" | "wgs_84" => Ok(Self::Wgs84),
            other => Err(format!("未知坐标基准 {other}，可选 bd09|gcj02|wgs84|auto")),
        }
    }

    fn convert(self, lat: f64, lng: f64) -> (f64, f64) {
        match self {
            Self::Bd09 => bd09_to_wgs84(lat, lng),
            Self::Gcj02 => gcj02_to_wgs84(lat, lng),
            Self::Wgs84 | Self::Auto => (lat, lng),
        }
    }
}

pub fn build_network(opts: &NetworkBuildOptions) -> Result<Value, String> {
    if opts.tracks.as_os_str().is_empty() {
        return Err("缺少 --tracks <目录或文件>".into());
    }
    if opts.out.as_os_str().is_empty() {
        return Err("缺少 --out <osm>".into());
    }
    if !(0.1..=100.0).contains(&opts.snap_m) {
        return Err("--snap-m 必须在 0.1..100".into());
    }
    if !(0.1..=100.0).contains(&opts.simplify_m) {
        return Err("--simplify-m 必须在 0.1..100".into());
    }

    let include_dates = normalize_filter_values(&opts.include_date);
    let exclude_dates = normalize_filter_values(&opts.exclude_date);
    let include_files = normalize_filter_values(&opts.include_file);
    let exclude_files = normalize_filter_values(&opts.exclude_file);

    let files = collect_json_files(&opts.tracks)?;
    if files.is_empty() {
        return Err(format!(
            "{} 下没有可读取的 JSON 轨迹",
            opts.tracks.display()
        ));
    }

    let mut warnings = Vec::new();
    let mut raw_segments: Vec<Vec<Coord>> = Vec::new();
    let mut file_reports: Vec<Value> = Vec::new();
    let mut parsed_files = 0usize;
    let mut accepted_files = 0usize;
    let mut excluded_files = 0usize;
    let mut errored_files = 0usize;
    let mut points_total = 0usize;
    let mut points_used = 0usize;
    let mut split_segments = 0usize;

    for file in &files {
        if let Some(pattern) = first_match(&exclude_files, file) {
            excluded_files += 1;
            file_reports.push(json!({
                "path": file.display().to_string(),
                "status": "excluded",
                "reason": format!("匹配 --exclude-file: {pattern}"),
            }));
            continue;
        }
        if !include_files.is_empty() && first_match(&include_files, file).is_none() {
            excluded_files += 1;
            file_reports.push(json!({
                "path": file.display().to_string(),
                "status": "excluded",
                "reason": "不匹配 --include-file",
            }));
            continue;
        }
        match parse_track_points(file) {
            Ok(track) => {
                parsed_files += 1;
                points_total += track.points.len();
                let valid: Vec<Coord> = track
                    .points
                    .iter()
                    .copied()
                    .filter(|p| valid_coord(p.lat, p.lon))
                    .collect();
                let date_reason =
                    date_filter_reason(track.date.as_deref(), &include_dates, &exclude_dates);
                let base_report = json!({
                    "path": file.display().to_string(),
                    "date": track.date,
                    "rrid": track.rrid,
                    "first_flag_ms": track.first_flag_ms,
                    "gain_time": track.gain_time,
                    "points": track.points.len(),
                    "valid_points": valid.len(),
                });
                if let Some(reason) = date_reason {
                    excluded_files += 1;
                    let mut report = base_report;
                    report["status"] = json!("excluded");
                    report["reason"] = json!(reason);
                    file_reports.push(report);
                    continue;
                }
                accepted_files += 1;
                points_used += valid.len();
                let points: Vec<Coord> = valid.into_iter().collect();
                let parts = split_at_jumps(&points, opts.max_jump_m);
                split_segments += parts.len();
                for part in parts {
                    if part.len() < 2 {
                        continue;
                    }
                    let simplified = simplify_rdp(&part, opts.simplify_m);
                    if simplified.len() >= 2 {
                        raw_segments.push(simplified);
                    } else {
                        raw_segments.push(part);
                    }
                }
                let mut report = base_report;
                report["status"] = json!("accepted");
                report["reason"] = Value::Null;
                file_reports.push(report);
            }
            Err(e) => {
                errored_files += 1;
                warnings.push(format!("{}: {e}", file.display()));
                file_reports.push(json!({
                    "path": file.display().to_string(),
                    "status": "error",
                    "reason": e,
                }));
            }
        }
    }
    if raw_segments.is_empty() {
        return Err(
            "没有筛选到可用轨迹段；请检查日期/文件筛选、run_data/allLocJson 或坐标字段".into(),
        );
    }

    let mut builder = NetworkBuilder::new(opts.snap_m, opts.min_segment_m);
    let mut base_nodes = 0usize;
    let mut base_edges = 0usize;
    if let Some(base_path) = &opts.base_osm {
        let bytes = fs::read(base_path)
            .map_err(|e| format!("读取基础 OSM {} 失败: {e}", base_path.display()))?;
        let base = route_planner::load_osm(&bytes)
            .map_err(|e| format!("解析基础 OSM {} 失败: {e}", base_path.display()))?;
        base_nodes = base.graph.node_count();
        base_edges = base.graph.edge_count();
        builder.add_graph(&base);
    }
    let mut simplified_points = 0usize;
    for seg in &raw_segments {
        simplified_points += seg.len();
        builder.add_polyline(seg);
    }
    builder.connect_nearby_nodes(4);
    let (nodes, directed_edges, buildings, manual_edges) = builder.into_parts();
    if directed_edges.is_empty() {
        return Err("轨迹经过过滤/抽稀后没有形成任何道路边".into());
    }

    let xml = render_osm(&nodes, &directed_edges, &buildings, &manual_edges);
    if let Some(parent) = opts.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建输出目录 {} 失败: {e}", parent.display()))?;
        }
    }
    fs::write(&opts.out, xml).map_err(|e| format!("写入 {} 失败: {e}", opts.out.display()))?;

    let physical_edges = undirected_edge_count(&directed_edges);
    let sizes = {
        let bytes = fs::read(&opts.out).map_err(|e| e.to_string())?;
        let graph =
            route_planner::load_osm(&bytes).map_err(|e| format!("回读生成路网失败: {e}"))?;
        route_planner::component_sizes(&graph)
    };
    let report = json!({
        "schema": 1,
        "source": {
            "tracks": opts.tracks.display().to_string(),
            "files": files.len(),
            "parsed_files": parsed_files,
            "accepted_files": accepted_files,
            "excluded_files": excluded_files,
            "errored_files": errored_files,
            "points_total": points_total,
            "points_used": points_used,
            "segments_after_jump_split": split_segments,
            "simplified_points": simplified_points,
            "base_osm": opts.base_osm.as_ref().map(|p| p.display().to_string()),
            "base_nodes": base_nodes,
            "base_edges": base_edges,
            "files_detail": file_reports,
        },
        "network": {
            "nodes": nodes.len(),
            "directed_edges": directed_edges.len(),
            "undirected_edges": physical_edges,
            "buildings": buildings.len(),
            "components": sizes.len(),
            "largest_component_nodes": sizes.first().copied().unwrap_or(0),
            "bbox_wgs84": bbox_json(&nodes),
        },
        "parameters": {
            "snap_m": opts.snap_m,
            "simplify_m": opts.simplify_m,
            "min_segment_m": opts.min_segment_m,
            "max_jump_m": opts.max_jump_m,
        },
        "filters": {
            "include_date": include_dates,
            "exclude_date": exclude_dates,
            "include_file": include_files,
            "exclude_file": exclude_files,
        },
        "warnings": warnings,
        "output": opts.out.display().to_string(),
    });
    if let Some(report_path) = &opts.report {
        write_json(report_path, &report)?;
    }
    if let Some(manifest_path) = &opts.manifest {
        let accepted: Vec<Value> = report["source"]["files_detail"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| row["status"] == "accepted")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let excluded: Vec<Value> = report["source"]["files_detail"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| row["status"] != "accepted")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        write_json(
            manifest_path,
            &json!({
                "schema": 1,
                "generated_by": "network-build",
                "accepted": accepted,
                "excluded": excluded,
                "network": opts.out.display().to_string(),
            }),
        )?;
    }
    Ok(report)
}

pub fn edit_network(opts: &NetworkEditOptions) -> Result<Value, String> {
    if opts.network.as_os_str().is_empty() {
        return Err("缺少 --network <osm>".into());
    }
    if opts.edits.as_os_str().is_empty() {
        return Err("缺少 --edits <json>".into());
    }
    let bytes = fs::read(&opts.network)
        .map_err(|e| format!("读取路网 {} 失败: {e}", opts.network.display()))?;
    let graph = route_planner::load_osm(&bytes)
        .map_err(|e| format!("解析路网 {} 失败: {e}", opts.network.display()))?;
    let input_nodes = graph.graph.node_count();
    let input_edges = graph.graph.edge_count();
    let edits = parse_edit_file(&opts.edits)?;
    if edits.is_empty() {
        return Err("编辑文件中没有 paths/ways/edits".into());
    }

    let mut builder = NetworkBuilder::new(opts.snap_m, opts.min_segment_m);
    // 编辑已有路网时不能按手动编辑的吸附半径重新合并原节点，否则较大的
    // --snap-m 会把原本相隔几米到几十米的道路节点折叠成少量节点，破坏拓扑。
    builder.add_graph_preserving(&graph);
    let mut edit_reports = Vec::with_capacity(edits.len());
    for (i, edit) in edits.iter().enumerate() {
        builder.add_manual_polyline(&edit.points, edit.bidirectional);
        edit_reports.push(json!({
            "index": i + 1,
            "points": edit.points.len(),
            "length_m": polyline_length_m(&edit.points),
            "bidirectional": edit.bidirectional,
            "frame": format!("{:?}", edit.frame).to_ascii_lowercase(),
            "note": edit.note,
        }));
    }
    let (nodes, directed_edges, buildings, manual_edges) = builder.into_parts();
    let xml = render_osm(&nodes, &directed_edges, &buildings, &manual_edges);
    write_text(&opts.out, &xml)?;

    let report = json!({
        "schema": 1,
        "generated_by": "network-edit",
        "input": {
            "network": opts.network.display().to_string(),
            "nodes": input_nodes,
            "directed_edges": input_edges,
            "edits": opts.edits.display().to_string(),
        },
        "applied": edit_reports,
        "parameters": {
            "snap_m": opts.snap_m,
            "min_segment_m": opts.min_segment_m,
        },
        "network": {
            "nodes": nodes.len(),
            "directed_edges": directed_edges.len(),
            "undirected_edges": undirected_edge_count(&directed_edges),
            "manual_edges": manual_edges.len(),
            "buildings": buildings.len(),
        },
        "output": opts.out.display().to_string(),
    });
    if let Some(report_path) = &opts.report {
        write_json(report_path, &report)?;
    }
    Ok(report)
}

pub fn connect_components(opts: &NetworkConnectOptions) -> Result<Value, String> {
    if opts.network.as_os_str().is_empty() {
        return Err("缺少 --network <osm>".into());
    }
    if !(0.1..=10_000.0).contains(&opts.max_gap_m) {
        return Err("--max-gap-m 必须在 0.1..10000".into());
    }
    let bytes = fs::read(&opts.network)
        .map_err(|e| format!("读取路网 {} 失败: {e}", opts.network.display()))?;
    let graph = route_planner::load_osm(&bytes)
        .map_err(|e| format!("解析路网 {} 失败: {e}", opts.network.display()))?;

    let mut groups = component_node_groups(&graph);
    let initial_components = groups.len();
    let mut connections = Vec::new();
    while groups.len() > 1 {
        let mut best: Option<(usize, usize, Coord, Coord, usize, usize, f64)> = None;
        for i in 0..groups.len() {
            for j in i + 1..groups.len() {
                for &a in &groups[i] {
                    for &b in &groups[j] {
                        let d = approx_distance_m(graph.node_coord(a), graph.node_coord(b));
                        if best.map(|(_, _, _, _, _, _, bd)| d < bd).unwrap_or(true) {
                            best = Some((
                                i,
                                j,
                                graph.node_coord(a),
                                graph.node_coord(b),
                                a.index(),
                                b.index(),
                                d,
                            ));
                        }
                    }
                }
            }
        }
        let Some((i, j, a_coord, b_coord, a_id, b_id, distance)) = best else {
            break;
        };
        if distance > opts.max_gap_m {
            break;
        }
        let moved = groups.remove(j);
        groups[i].extend(moved);
        groups[i].sort_unstable_by_key(|n| n.index());
        connections.push(json!({
            "from_node": a_id + 1,
            "to_node": b_id + 1,
            "from_wgs84": {"lat": a_coord.lat, "lon": a_coord.lon},
            "to_wgs84": {"lat": b_coord.lat, "lon": b_coord.lon},
            "distance_m": distance,
        }));
    }

    let mut builder = NetworkBuilder::new(0.1, 0.1);
    builder.add_graph(&graph);
    for row in &connections {
        let a = Coord::new(
            row["from_wgs84"]["lon"].as_f64().unwrap_or(0.0),
            row["from_wgs84"]["lat"].as_f64().unwrap_or(0.0),
        );
        let b = Coord::new(
            row["to_wgs84"]["lon"].as_f64().unwrap_or(0.0),
            row["to_wgs84"]["lat"].as_f64().unwrap_or(0.0),
        );
        builder.add_manual_edge(a, b);
    }
    let (nodes, directed_edges, buildings, manual_edges) = builder.into_parts();
    if nodes.is_empty() || directed_edges.is_empty() {
        return Err("路网为空，无法连接分量".into());
    }
    let xml = render_osm(&nodes, &directed_edges, &buildings, &manual_edges);
    write_text(&opts.out, &xml)?;

    let out_bytes = fs::read(&opts.out).map_err(|e| e.to_string())?;
    let out_graph =
        route_planner::load_osm(&out_bytes).map_err(|e| format!("回读输出路网失败: {e}"))?;
    let final_components = route_planner::component_sizes(&out_graph).len();
    let remaining_gap = if groups.len() > 1 {
        nearest_cross_component_gap(&graph, &groups)
    } else {
        None
    };
    let report = json!({
        "schema": 1,
        "generated_by": "network-connect",
        "input": opts.network.display().to_string(),
        "initial_components": initial_components,
        "connections_added": connections.len(),
        "connections": connections,
        "final_components": final_components,
        "max_gap_m": opts.max_gap_m,
        "remaining_unconnected_gap_m": remaining_gap,
        "manual_edges": manual_edges.len(),
        "output": opts.out.display().to_string(),
    });
    if let Some(report_path) = &opts.report {
        write_json(report_path, &report)?;
    }
    Ok(report)
}

fn component_node_groups(graph: &RoadGraph) -> Vec<Vec<route_planner::NodeIndex>> {
    let n = graph.graph.node_count();
    let mut seen = vec![false; n];
    let mut groups = Vec::new();
    for start in graph.graph.node_indices() {
        if seen[start.index()] {
            continue;
        }
        let mut stack = vec![start];
        let mut group = Vec::new();
        seen[start.index()] = true;
        while let Some(cur) = stack.pop() {
            group.push(cur);
            for next in graph.graph.neighbors_undirected(cur) {
                if !seen[next.index()] {
                    seen[next.index()] = true;
                    stack.push(next);
                }
            }
        }
        group.sort_unstable_by_key(|n| n.index());
        groups.push(group);
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    groups
}

fn nearest_cross_component_gap(
    graph: &RoadGraph,
    groups: &[Vec<route_planner::NodeIndex>],
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for i in 0..groups.len() {
        for j in i + 1..groups.len() {
            for &a in &groups[i] {
                for &b in &groups[j] {
                    let d = approx_distance_m(graph.node_coord(a), graph.node_coord(b));
                    if best.map(|x| d < x).unwrap_or(true) {
                        best = Some(d);
                    }
                }
            }
        }
    }
    best
}

fn collect_json_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(format!("轨迹路径不存在: {}", path.display()));
    }
    let mut out = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("读取目录 {} 失败: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn normalize_filter_values(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        for part in value.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
            let part = part.trim().to_ascii_lowercase();
            if !part.is_empty() && !out.contains(&part) {
                out.push(part);
            }
        }
    }
    out
}

fn first_match(patterns: &[String], path: &Path) -> Option<String> {
    let text = path.to_string_lossy().to_ascii_lowercase();
    patterns.iter().find(|p| text.contains(p.as_str())).cloned()
}

fn date_filter_reason(
    date: Option<&str>,
    include_dates: &[String],
    exclude_dates: &[String],
) -> Option<String> {
    let Some(date) = date else {
        return (!include_dates.is_empty())
            .then(|| "缺少可识别日期，且设置了 --include-date".into());
    };
    let date = date.to_ascii_lowercase();
    if exclude_dates.iter().any(|d| d == &date) {
        return Some(format!("命中 --exclude-date: {date}"));
    }
    if !include_dates.is_empty() && !include_dates.iter().any(|d| d == &date) {
        return Some(format!("不匹配 --include-date: {date}"));
    }
    None
}

fn ms_date(ms: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
}

fn parse_track_points(path: &Path) -> Result<ParsedTrack, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    let root: Value = serde_json::from_str(&text).map_err(|e| format!("JSON 解析失败: {e}"))?;
    let run_value = root.get("run_data").unwrap_or(&root);
    let run = json_value(run_value)?;
    let all = run
        .get("allLocJson")
        .or_else(|| run.get("allLoc"))
        .ok_or_else(|| "缺少 allLocJson".to_string())?;
    let arr = json_value(all)?;
    let arr = arr
        .as_array()
        .ok_or_else(|| "allLocJson 不是数组".to_string())?;
    let first_flag_ms = arr
        .iter()
        .find_map(|p| p.get("flag").and_then(Value::as_i64).filter(|v| *v > 0));
    let gain_time = arr
        .iter()
        .find_map(|p| p.get("gainTime").and_then(Value::as_str))
        .or_else(|| run.get("gainTime").and_then(Value::as_str))
        .map(str::to_string);
    let rrid = root
        .get("rrid")
        .or_else(|| run.get("rrid"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => "0".into(),
        })
        .unwrap_or_else(|| "0".into());
    let date = first_flag_ms
        .and_then(ms_date)
        .or_else(|| gain_time.as_deref().and_then(parse_date_prefix))
        .or_else(|| {
            run.get("startTime")
                .and_then(Value::as_i64)
                .and_then(ms_date)
        });

    let mut out = Vec::with_capacity(arr.len());
    for p in arr {
        let Some(obj) = p.as_object() else {
            continue;
        };
        let lat = number_any(obj, &["gLat", "glat", "gLatitude", "latitude"]);
        let lng = number_any(obj, &["gLng", "glon", "gLongitude", "longitude", "lng"]);
        let (Some(lat), Some(lng)) = (lat, lng) else {
            continue;
        };
        let raw_frame = obj
            .get("coorType")
            .or_else(|| obj.get("coordType"))
            .and_then(|v| v.as_str())
            .unwrap_or("gcj02");
        let frame = match PointFrame::parse(raw_frame).unwrap_or(PointFrame::Gcj02) {
            PointFrame::Auto => PointFrame::Gcj02,
            f => f,
        };
        let (wlat, wlng) = frame.convert(lat, lng);
        out.push(Coord::new(wlng, wlat));
    }
    if out.is_empty() {
        return Err("没有有效 gLat/gLng 轨迹点".into());
    }
    Ok(ParsedTrack {
        points: out,
        date,
        rrid,
        first_flag_ms,
        gain_time,
    })
}

fn parse_date_prefix(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() >= 10 {
        let date = &value[..10];
        if NaiveDateTime::parse_from_str(&format!("{date} 00:00:00"), "%Y-%m-%d %H:%M:%S").is_ok() {
            return Some(date.to_string());
        }
    }
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
}

fn parse_edit_file(path: &Path) -> Result<Vec<ManualEdit>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取编辑文件失败: {e}"))?;
    let root: Value =
        serde_json::from_str(&text).map_err(|e| format!("编辑 JSON 解析失败: {e}"))?;
    let default_frame = root
        .get("frame")
        .and_then(Value::as_str)
        .map(PointFrame::parse)
        .transpose()?
        .unwrap_or(PointFrame::Wgs84);
    let rows = root
        .get("paths")
        .or_else(|| root.get("ways"))
        .or_else(|| root.get("edits"))
        .unwrap_or(&root)
        .as_array()
        .ok_or("编辑 JSON 顶层应为数组，或含 paths/ways/edits 数组")?;

    let mut out = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let obj = row
            .as_object()
            .ok_or_else(|| format!("第 {} 条编辑不是对象", i + 1))?;
        let frame = obj
            .get("frame")
            .and_then(Value::as_str)
            .map(PointFrame::parse)
            .transpose()?
            .unwrap_or(default_frame);
        let mut points = Vec::new();
        if let Some(Value::Array(values)) = obj.get("points") {
            for v in values {
                points.push(parse_edit_coord(v, frame)?);
            }
        } else {
            let from = obj
                .get("from")
                .or_else(|| obj.get("start"))
                .ok_or_else(|| format!("第 {} 条编辑缺少 from/start", i + 1))?;
            points.push(parse_edit_coord(from, frame)?);
            if let Some(Value::Array(values)) = obj.get("via") {
                for v in values {
                    points.push(parse_edit_coord(v, frame)?);
                }
            }
            let to = obj
                .get("to")
                .or_else(|| obj.get("end"))
                .ok_or_else(|| format!("第 {} 条编辑缺少 to/end", i + 1))?;
            points.push(parse_edit_coord(to, frame)?);
        }
        if points.len() < 2 {
            return Err(format!("第 {} 条编辑至少需要两个点", i + 1));
        }
        let bidirectional = obj
            .get("bidirectional")
            .or_else(|| obj.get("twoWay"))
            .or_else(|| obj.get("two_way"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let note = obj
            .get("note")
            .and_then(Value::as_str)
            .unwrap_or("manual path")
            .to_string();
        out.push(ManualEdit {
            points,
            bidirectional,
            note,
            frame,
        });
    }
    Ok(out)
}

fn parse_edit_coord(value: &Value, frame: PointFrame) -> Result<Coord, String> {
    let (lat, lon) = match value {
        Value::Array(values) if values.len() >= 2 => {
            let lat = values[0].as_f64().ok_or("坐标数组第 1 项不是数字")?;
            let lon = values[1].as_f64().ok_or("坐标数组第 2 项不是数字")?;
            (lat, lon)
        }
        Value::Object(obj) => {
            for (key, nested_frame) in [
                ("wgs84", PointFrame::Wgs84),
                ("gcj02", PointFrame::Gcj02),
                ("bd09", PointFrame::Bd09),
            ] {
                if let Some(nested) = obj.get(key) {
                    return parse_edit_coord(nested, nested_frame);
                }
            }
            let lat = obj
                .get("lat")
                .or_else(|| obj.get("latitude"))
                .and_then(Value::as_f64)
                .ok_or("坐标对象缺少 lat/latitude")?;
            let lon = obj
                .get("lon")
                .or_else(|| obj.get("lng"))
                .or_else(|| obj.get("longitude"))
                .and_then(Value::as_f64)
                .ok_or("坐标对象缺少 lon/lng/longitude")?;
            (lat, lon)
        }
        _ => return Err("坐标应为 [lat, lon] 或 {lat, lon}".into()),
    };
    if !valid_coord(lat, lon) {
        return Err(format!("无效坐标 lat={lat} lon={lon}"));
    }
    let frame = if frame == PointFrame::Auto {
        PointFrame::Wgs84
    } else {
        frame
    };
    let (wlat, wlon) = frame.convert(lat, lon);
    Ok(Coord::new(wlon, wlat))
}

fn polyline_length_m(points: &[Coord]) -> f64 {
    points
        .windows(2)
        .map(|pair| approx_distance_m(pair[0], pair[1]))
        .sum()
}

fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建输出目录 {} 失败: {e}", parent.display()))?;
        }
    }
    fs::write(path, text).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

fn json_value(v: &Value) -> Result<Value, String> {
    match v {
        Value::Array(_) | Value::Object(_) => Ok(v.clone()),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Err("空 JSON 字符串".into());
            }
            if let Ok(v) = serde_json::from_str(t) {
                return Ok(v);
            }
            let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, t)
                .or_else(|_| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE, t))
                .map_err(|_| "字符串不是 JSON，也不是 base64+gzip".to_string())?;
            let mut dec = flate2::read::GzDecoder::new(raw.as_slice());
            let mut text = String::new();
            dec.read_to_string(&mut text)
                .map_err(|e| format!("gzip 解压失败: {e}"))?;
            serde_json::from_str(&text).map_err(|e| format!("解压后 JSON 解析失败: {e}"))
        }
        _ => Err("不是 JSON 对象/数组/字符串".into()),
    }
}

fn number_any(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = obj.get(*k).and_then(Value::as_f64) {
            if v.is_finite() && v != 0.0 {
                return Some(v);
            }
        }
    }
    None
}

fn valid_coord(lat: f64, lng: f64) -> bool {
    lat.is_finite()
        && lng.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lng)
        && !(lat.abs() < 1e-9 && lng.abs() < 1e-9)
}

fn approx_distance_m(a: Coord, b: Coord) -> f64 {
    let mid = (a.lat.to_radians() + b.lat.to_radians()) * 0.5;
    let dx = (b.lon - a.lon) * 111_320.0 * mid.cos();
    let dy = (b.lat - a.lat) * 111_132.0;
    (dx * dx + dy * dy).sqrt()
}

fn split_at_jumps(points: &[Coord], max_jump_m: f64) -> Vec<Vec<Coord>> {
    if points.is_empty() {
        return Vec::new();
    }
    let limit = max_jump_m.max(10.0);
    let mut out = Vec::new();
    let mut cur = vec![points[0]];
    for &p in &points[1..] {
        let prev = *cur.last().unwrap();
        if approx_distance_m(prev, p) > limit {
            if cur.len() >= 2 {
                out.push(cur);
            }
            cur = vec![p];
        } else if approx_distance_m(prev, p) >= 0.35 {
            cur.push(p);
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

fn simplify_rdp(points: &[Coord], eps_m: f64) -> Vec<Coord> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let eps = eps_m.max(0.1);
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0usize, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let mut best = 0.0f64;
        let mut best_i = a + 1;
        for i in a + 1..b {
            let d = point_segment_distance_m(points[i], points[a], points[b]);
            if d > best {
                best = d;
                best_i = i;
            }
        }
        if best > eps {
            keep[best_i] = true;
            stack.push((a, best_i));
            stack.push((best_i, b));
        }
    }
    points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| keep[i].then_some(*p))
        .collect()
}

fn point_segment_distance_m(p: Coord, a: Coord, b: Coord) -> f64 {
    let mid = ((a.lat + b.lat + p.lat) / 3.0).to_radians();
    let scale_x = 111_320.0 * mid.cos().abs().max(0.2);
    let scale_y = 111_132.0;
    let px = (p.lon - a.lon) * scale_x;
    let py = (p.lat - a.lat) * scale_y;
    let bx = (b.lon - a.lon) * scale_x;
    let by = (b.lat - a.lat) * scale_y;
    let denom = bx * bx + by * by;
    if denom <= 1e-12 {
        return (px * px + py * py).sqrt();
    }
    let t = ((px * bx + py * by) / denom).clamp(0.0, 1.0);
    let dx = px - t * bx;
    let dy = py - t * by;
    (dx * dx + dy * dy).sqrt()
}

struct NetworkBuilder {
    nodes: Vec<Coord>,
    directed_edges: BTreeSet<(usize, usize)>,
    manual_edges: BTreeSet<(usize, usize)>,
    buildings: Vec<Vec<Coord>>,
    snap_m: f64,
    min_segment_m: f64,
    base_lat: f64,
    cell_m: f64,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

impl NetworkBuilder {
    fn new(snap_m: f64, min_segment_m: f64) -> Self {
        Self {
            nodes: Vec::new(),
            directed_edges: BTreeSet::new(),
            manual_edges: BTreeSet::new(),
            buildings: Vec::new(),
            snap_m,
            min_segment_m: min_segment_m.max(0.1),
            base_lat: 0.0,
            cell_m: snap_m.max(1.0),
            grid: HashMap::new(),
        }
    }

    fn xy(&self, c: Coord) -> (f64, f64) {
        let lat = if self.nodes.is_empty() {
            c.lat
        } else {
            self.base_lat
        };
        (
            c.lon * 111_320.0 * lat.to_radians().cos().abs().max(0.2),
            c.lat * 111_132.0,
        )
    }

    fn intern(&mut self, c: Coord) -> usize {
        if self.nodes.is_empty() {
            self.base_lat = c.lat;
        }
        let (x, y) = self.xy(c);
        let cx = (x / self.cell_m).floor() as i32;
        let cy = (y / self.cell_m).floor() as i32;
        let mut best: Option<(usize, f64)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ids) = self.grid.get(&(cx + dx, cy + dy)) {
                    for &id in ids {
                        let d = approx_distance_m(self.nodes[id], c);
                        if d <= self.snap_m && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                            best = Some((id, d));
                        }
                    }
                }
            }
        }
        if let Some((id, _)) = best {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(c);
        self.grid.entry((cx, cy)).or_default().push(id);
        id
    }

    fn add_graph(&mut self, graph: &RoadGraph) {
        let mut map: HashMap<usize, usize> = HashMap::new();
        for n in graph.graph.node_indices() {
            map.insert(n.index(), self.intern(graph.node_coord(n)));
        }
        for e in graph.graph.edge_indices() {
            let Some((a, b)) = graph.graph.edge_endpoints(e) else {
                continue;
            };
            let (Some(&a), Some(&b)) = (map.get(&a.index()), map.get(&b.index())) else {
                continue;
            };
            if a == b {
                continue;
            }
            let data = &graph.graph[e];
            self.directed_edges.insert((a, b));
            if !data.oneway {
                self.directed_edges.insert((b, a));
            }
        }
        self.buildings.extend(graph.buildings.iter().cloned());
        for ring in graph.buildings.iter().cloned() {
            for c in ring {
                self.intern(c);
            }
        }
    }

    fn add_graph_preserving(&mut self, graph: &RoadGraph) {
        let mut map: HashMap<usize, usize> = HashMap::new();
        for n in graph.graph.node_indices() {
            map.insert(n.index(), self.push_exact(graph.node_coord(n)));
        }
        for e in graph.graph.edge_indices() {
            let Some((a, b)) = graph.graph.edge_endpoints(e) else {
                continue;
            };
            let (Some(&a), Some(&b)) = (map.get(&a.index()), map.get(&b.index())) else {
                continue;
            };
            if a == b {
                continue;
            }
            let data = &graph.graph[e];
            self.directed_edges.insert((a, b));
            if !data.oneway {
                self.directed_edges.insert((b, a));
            }
        }
        self.buildings.extend(graph.buildings.iter().cloned());
        for ring in graph.buildings.iter().cloned() {
            for c in ring {
                self.intern(c);
            }
        }
    }

    fn push_exact(&mut self, c: Coord) -> usize {
        if self.nodes.is_empty() {
            self.base_lat = c.lat;
        }
        let (x, y) = self.xy(c);
        let cx = (x / self.cell_m).floor() as i32;
        let cy = (y / self.cell_m).floor() as i32;
        let id = self.nodes.len();
        self.nodes.push(c);
        self.grid.entry((cx, cy)).or_default().push(id);
        id
    }

    fn add_polyline(&mut self, points: &[Coord]) {
        self.add_polyline_with_source(points, false, true);
    }

    fn add_manual_polyline(&mut self, points: &[Coord], bidirectional: bool) {
        self.add_polyline_with_source(points, true, bidirectional);
    }

    fn add_polyline_with_source(&mut self, points: &[Coord], manual: bool, bidirectional: bool) {
        if points.len() < 2 {
            return;
        }
        let mut prev = self.intern(points[0]);
        for &p in &points[1..] {
            let next = self.intern(p);
            if next == prev {
                continue;
            }
            let len = approx_distance_m(self.nodes[prev], self.nodes[next]);
            if len < self.min_segment_m {
                continue;
            }
            self.directed_edges.insert((prev, next));
            if bidirectional {
                self.directed_edges.insert((next, prev));
            }
            if manual {
                self.manual_edges.insert(edge_key(prev, next));
            }
            prev = next;
        }
    }

    fn add_manual_edge(&mut self, a: Coord, b: Coord) {
        let a = self.intern(a);
        let b = self.intern(b);
        if a == b || approx_distance_m(self.nodes[a], self.nodes[b]) < self.min_segment_m {
            return;
        }
        self.directed_edges.insert((a, b));
        self.directed_edges.insert((b, a));
        self.manual_edges.insert(edge_key(a, b));
    }

    fn connect_nearby_nodes(&mut self, rounds: usize) {
        for _ in 0..rounds {
            let edges: Vec<(usize, usize)> = self.directed_edges.iter().copied().collect();
            let mut changed = false;
            for (a, b) in edges {
                if !self.directed_edges.contains(&(a, b)) {
                    continue;
                }
                let seg_a = self.nodes[a];
                let seg_b = self.nodes[b];
                let mut hit: Option<(usize, f64)> = None;
                for c in 0..self.nodes.len() {
                    if c == a || c == b {
                        continue;
                    }
                    let d = point_segment_distance_m(self.nodes[c], seg_a, seg_b);
                    if d <= self.snap_m && hit.map(|(_, best)| d < best).unwrap_or(true) {
                        hit = Some((c, d));
                    }
                }
                let Some((c, _)) = hit else {
                    continue;
                };
                let len1 = approx_distance_m(seg_a, self.nodes[c]);
                let len2 = approx_distance_m(self.nodes[c], seg_b);
                if len1 < self.min_segment_m || len2 < self.min_segment_m {
                    continue;
                }
                let was_manual = self.manual_edges.contains(&edge_key(a, b));
                self.directed_edges.remove(&(a, b));
                let reverse = self.directed_edges.remove(&(b, a));
                self.directed_edges.insert((a, c));
                self.directed_edges.insert((c, b));
                if reverse {
                    self.directed_edges.insert((c, a));
                    self.directed_edges.insert((b, c));
                }
                if was_manual {
                    self.manual_edges.remove(&edge_key(a, b));
                    self.manual_edges.insert(edge_key(a, c));
                    self.manual_edges.insert(edge_key(c, b));
                }
                changed = true;
            }
            if !changed {
                break;
            }
        }
    }

    fn into_parts(
        self,
    ) -> (
        Vec<Coord>,
        BTreeSet<(usize, usize)>,
        Vec<Vec<Coord>>,
        BTreeSet<(usize, usize)>,
    ) {
        (
            self.nodes,
            self.directed_edges,
            self.buildings,
            self.manual_edges,
        )
    }
}

fn edge_key(a: usize, b: usize) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}
fn render_osm(
    nodes: &[Coord],
    directed_edges: &BTreeSet<(usize, usize)>,
    buildings: &[Vec<Coord>],
    manual_edges: &BTreeSet<(usize, usize)>,
) -> String {
    let mut s = String::with_capacity(nodes.len() * 90 + directed_edges.len() * 130);
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<osm version=\"0.6\" generator=\"xierun-network-build\">\n");
    if let Some(bb) = bbox_values(nodes) {
        s.push_str(&format!(
            "  <bounds minlat=\"{:.7}\" minlon=\"{:.7}\" maxlat=\"{:.7}\" maxlon=\"{:.7}\"/>\n",
            bb.0, bb.1, bb.2, bb.3
        ));
    }
    for (i, c) in nodes.iter().enumerate() {
        s.push_str(&format!(
            "  <node id=\"{}\" lat=\"{:.7}\" lon=\"{:.7}\"/>\n",
            i + 1,
            c.lat,
            c.lon
        ));
    }

    let mut used: HashSet<(usize, usize)> = HashSet::new();
    let mut way_id = 1usize;
    for &(a, b) in directed_edges {
        if a == b || used.contains(&(a, b)) {
            continue;
        }
        let reverse = directed_edges.contains(&(b, a));
        if reverse && a > b {
            continue;
        }
        used.insert((a, b));
        if reverse {
            used.insert((b, a));
        }
        let manual = manual_edges.contains(&edge_key(a, b));
        s.push_str(&format!("  <way id=\"{}\">\n", way_id));
        way_id += 1;
        s.push_str(&format!("    <nd ref=\"{}\"/>\n", a + 1));
        s.push_str(&format!("    <nd ref=\"{}\"/>\n", b + 1));
        s.push_str("    <tag k=\"highway\" v=\"footway\"/>\n");
        if manual {
            s.push_str("    <tag k=\"source\" v=\"manual\"/>\n");
            s.push_str("    <tag k=\"note\" v=\"manually added or component connector; verify on site\"/>\n");
        } else {
            s.push_str("    <tag k=\"source\" v=\"nekosports_gps_track\"/>\n");
            s.push_str(
                "    <tag k=\"note\" v=\"auto-generated candidate path; verify on site\"/>\n",
            );
        }
        s.push_str(&format!(
            "    <tag k=\"oneway\" v=\"{}\"/>\n",
            if reverse { "no" } else { "yes" }
        ));
        s.push_str("  </way>\n");
    }

    let mut coord_id: HashMap<(i64, i64), usize> = HashMap::new();
    for (i, c) in nodes.iter().enumerate() {
        coord_id.insert(coord_key(*c), i + 1);
    }
    let mut building_id = way_id + 100_000;
    for ring in buildings {
        if ring.len() < 3 {
            continue;
        }
        let ids: Vec<usize> = ring
            .iter()
            .filter_map(|c| coord_id.get(&coord_key(*c)).copied())
            .collect();
        if ids.len() < 3 {
            continue;
        }
        s.push_str(&format!("  <way id=\"{}\">\n", building_id));
        building_id += 1;
        for id in ids {
            s.push_str(&format!("    <nd ref=\"{}\"/>\n", id));
        }
        s.push_str("    <tag k=\"building\" v=\"yes\"/>\n");
        s.push_str("  </way>\n");
    }
    s.push_str("</osm>\n");
    s
}

fn coord_key(c: Coord) -> (i64, i64) {
    (
        (c.lat * 10_000_000.0).round() as i64,
        (c.lon * 10_000_000.0).round() as i64,
    )
}

fn undirected_edge_count(edges: &BTreeSet<(usize, usize)>) -> usize {
    let mut seen = HashSet::new();
    for &(a, b) in edges {
        let k = if a < b { (a, b) } else { (b, a) };
        seen.insert(k);
    }
    seen.len()
}

fn bbox_values(nodes: &[Coord]) -> Option<(f64, f64, f64, f64)> {
    let mut it = nodes.iter().filter(|c| valid_coord(c.lat, c.lon));
    let first = *it.next()?;
    let (mut min_lat, mut max_lat) = (first.lat, first.lat);
    let (mut min_lon, mut max_lon) = (first.lon, first.lon);
    for c in it {
        min_lat = min_lat.min(c.lat);
        max_lat = max_lat.max(c.lat);
        min_lon = min_lon.min(c.lon);
        max_lon = max_lon.max(c.lon);
    }
    Some((min_lat, min_lon, max_lat, max_lon))
}

fn bbox_json(nodes: &[Coord]) -> Value {
    match bbox_values(nodes) {
        Some((min_lat, min_lon, max_lat, max_lon)) => json!({
            "min_lat": min_lat,
            "min_lon": min_lon,
            "max_lat": max_lat,
            "max_lon": max_lon,
        }),
        None => Value::Null,
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
        }
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| format!("JSON 序列化失败: {e}"))?;
    fs::write(path, text).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

pub fn validate_network(opts: &NetworkValidateOptions) -> Result<Value, String> {
    if opts.network.as_os_str().is_empty() {
        return Err("缺少 --network <osm>".into());
    }
    let bytes = fs::read(&opts.network)
        .map_err(|e| format!("读取路网 {} 失败: {e}", opts.network.display()))?;
    let graph = route_planner::load_osm(&bytes)
        .map_err(|e| format!("解析路网 {} 失败: {e}", opts.network.display()))?;
    let sizes = route_planner::component_sizes(&graph);
    let largest = sizes.first().copied().unwrap_or(0);
    let node_count = graph.graph.node_count();
    let edge_count = graph.graph.edge_count();
    let mut endpoints = 0usize;
    for n in graph.graph.node_indices() {
        if graph.graph.neighbors_undirected(n).count() == 1 {
            endpoints += 1;
        }
    }
    let mut warnings = Vec::new();
    if sizes.len() > 1 {
        warnings.push(format!(
            "路网存在 {} 个连通分量，最大分量仅 {} 个节点",
            sizes.len(),
            largest
        ));
    }
    if node_count > 1 && largest < node_count / 2 {
        warnings.push("最大连通分量不足节点总数一半，请检查轨迹拼接".into());
    }
    if edge_count == 0 {
        warnings.push("路网没有可用边".into());
    }
    if endpoints > 2 {
        warnings.push(format!(
            "发现 {endpoints} 个度为 1 的端点，可能存在断头或轨迹缺口"
        ));
    }

    let frame = match opts.point_frame.as_deref() {
        Some(s) => PointFrame::parse(s)?,
        None => PointFrame::Auto,
    };
    let mut point_reports = Vec::new();
    let mut route_ok: Option<bool> = None;
    let mut route_error: Option<String> = None;
    if let Some(points_path) = &opts.points {
        let points = parse_points_file(points_path, frame)?;
        if points.is_empty() {
            warnings.push("点位文件中没有解析到坐标".into());
        }
        let coords: Vec<Coord> = points.iter().map(|p| p.coord).collect();
        let mut g = graph.clone();
        let projected = route_planner::project::add_virtual_nodes(&mut g, &coords).ok();
        for (i, p) in points.iter().enumerate() {
            let nearest = route_planner::nearest_edge_nodes(&graph, p.coord);
            let (dist_m, component, comp_size, on_largest) = match nearest {
                Some((a, _b, d, _proj)) => {
                    let info = route_planner::component_info(&graph, a);
                    let (cid, csize) = info.unwrap_or((usize::MAX, 0));
                    (Some(d), Some(cid), csize, csize == largest && csize > 0)
                }
                None => (None, None, 0, false),
            };
            let reachable = projected
                .as_ref()
                .and_then(|nodes| nodes.get(i).copied())
                .map(|n| route_planner::route::shortest_path(&g, n, n).is_some())
                .unwrap_or(false);
            point_reports.push(json!({
                "name": p.name,
                "wgs84": {"lat": p.coord.lat, "lon": p.coord.lon},
                "nearest_distance_m": dist_m,
                "component": component,
                "component_nodes": comp_size,
                "on_largest_component": on_largest,
                "snap_ok": dist_m.map(|d| d <= opts.snap_m).unwrap_or(false),
                "projected": reachable,
            }));
        }
        if points.len() >= 2 {
            let waypoints = vec![coords[0]];
            let must = coords[1..].to_vec();
            match route_planner::plan_route_split(
                &graph,
                &waypoints,
                &must,
                2000.0,
                1,
                &route_planner::RouteOptions::default(),
            ) {
                Ok(route) => {
                    route_ok = Some(true);
                    warnings.push(format!("路线规划自检通过，长度 {:.0}m", route.length_m));
                }
                Err(e) => {
                    route_ok = Some(false);
                    route_error = Some(e.clone());
                    warnings.push(format!("路线规划自检失败: {e}"));
                }
            }
        }
    } else {
        warnings.push("未提供 --points，未校验打卡点吸附与规划可达性".into());
    }

    Ok(json!({
        "schema": 1,
        "network": opts.network.display().to_string(),
        "nodes": node_count,
        "directed_edges": edge_count,
        "components": sizes.len(),
        "component_sizes": sizes,
        "largest_component_nodes": largest,
        "endpoint_nodes": endpoints,
        "bbox_wgs84": bbox_json(&(0..graph.graph.node_count()).filter_map(|i| {
            graph.graph.node_indices().find(|n| n.index() == i).map(|n| graph.node_coord(n))
        }).collect::<Vec<_>>()),
        "points_checked": point_reports.len(),
        "points": point_reports,
        "route_self_check": route_ok,
        "route_error": route_error,
        "snap_m": opts.snap_m,
        "warnings": warnings,
    }))
}
fn parse_points_file(path: &Path, frame: PointFrame) -> Result<Vec<NamedPoint>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取点位文件失败: {e}"))?;
    let root: Value =
        serde_json::from_str(&text).map_err(|e| format!("点位 JSON 解析失败: {e}"))?;
    let container = extract_point_container(&root);
    let mut out = Vec::new();
    collect_points(&container, frame, &mut out, 0);
    let mut seen = HashSet::new();
    out.retain(|p| {
        let key = (
            (p.coord.lat * 10_000_000.0).round() as i64,
            (p.coord.lon * 10_000_000.0).round() as i64,
            p.name.clone(),
        );
        seen.insert(key)
    });
    Ok(out)
}

fn extract_point_container(root: &Value) -> Value {
    for key in ["fixed_point_json", "fixedPointJson"] {
        if let Some(v) = root.get(key) {
            if let Ok(inner) = json_value(v) {
                for inner_key in ["fivePointJson", "fivePoint", "points"] {
                    if let Some(points) = inner.get(inner_key) {
                        if let Ok(v) = json_value(points) {
                            return v;
                        }
                    }
                }
                return inner;
            }
        }
    }
    for key in ["fivePointJson", "pointsResModels", "points", "fixedPoints"] {
        if let Some(v) = root.get(key) {
            if let Ok(v) = json_value(v) {
                return v;
            }
        }
    }
    if let Some(data) = root.get("data") {
        if let Ok(v) = json_value(data) {
            return v;
        }
    }
    root.clone()
}

fn collect_points(v: &Value, frame: PointFrame, out: &mut Vec<NamedPoint>, depth: usize) {
    if depth > 12 {
        return;
    }
    match v {
        Value::Array(arr) => {
            for item in arr {
                collect_points(item, frame, out, depth + 1);
            }
        }
        Value::Object(obj) => {
            if let Some(p) = point_from_object(obj, frame, out.len() + 1) {
                out.push(p);
                return;
            }
            for child in obj.values() {
                collect_points(child, frame, out, depth + 1);
            }
        }
        Value::String(s) => {
            let t = s.trim();
            if (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'))
            {
                if let Ok(parsed) = serde_json::from_str::<Value>(t) {
                    collect_points(&parsed, frame, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

fn point_from_object(
    obj: &serde_json::Map<String, Value>,
    frame: PointFrame,
    fallback_id: usize,
) -> Option<NamedPoint> {
    let glat = number_any(obj, &["gLat", "glat", "gLatitude"]);
    let glng = number_any(obj, &["gLng", "glon", "gLongitude"]);
    let blat = number_any(obj, &["lat", "latitude", "bdLat"]);
    let blng = number_any(obj, &["lon", "lng", "longitude", "bdLng"]);
    let (lat, lng, used_frame) = if let (Some(lat), Some(lng)) = (glat, glng) {
        let f = if frame == PointFrame::Auto {
            PointFrame::Gcj02
        } else {
            frame
        };
        (lat, lng, f)
    } else if let (Some(lat), Some(lng)) = (blat, blng) {
        let f = if frame == PointFrame::Auto {
            PointFrame::Bd09
        } else {
            frame
        };
        (lat, lng, f)
    } else {
        return None;
    };
    let (wlat, wlng) = used_frame.convert(lat, lng);
    if !valid_coord(wlat, wlng) {
        return None;
    }
    let name = obj
        .get("pointName")
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("point{fallback_id}"));
    Some(NamedPoint {
        name,
        coord: Coord::new(wlng, wlat),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-7, "a={a} b={b}");
    }

    #[test]
    fn nested_manual_coordinate_frame_overrides_default() {
        let input = json!({"gcj02": {"lat": 42.853558, "lon": 125.133072}});
        let got = parse_edit_coord(&input, PointFrame::Bd09).unwrap();
        let (lat, lon) = PointFrame::Gcj02.convert(42.853558, 125.133072);
        assert_close(got.lat, lat);
        assert_close(got.lon, lon);
    }

    #[test]
    fn auto_manual_array_defaults_to_wgs84() {
        let input = json!([42.853558, 125.133072]);
        let got = parse_edit_coord(&input, PointFrame::Auto).unwrap();
        assert_close(got.lat, 42.853558);
        assert_close(got.lon, 125.133072);
    }

    #[test]
    fn manual_polyline_can_be_one_way_and_is_marked() {
        let mut builder = NetworkBuilder::new(0.1, 0.1);
        let a = Coord::new(125.133072, 42.853558);
        let b = Coord::new(125.134072, 42.853558);
        builder.add_manual_polyline(&[a, b], false);
        assert!(builder.directed_edges.contains(&(0, 1)));
        assert!(!builder.directed_edges.contains(&(1, 0)));
        assert!(builder.manual_edges.contains(&edge_key(0, 1)));
    }

    #[test]
    fn preserving_edit_graph_does_not_collapse_existing_nodes() {
        let osm = br#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
  <node id="1" lat="42.000000" lon="125.000000"/>
  <node id="2" lat="42.000050" lon="125.000000"/>
  <way id="1">
    <nd ref="1"/>
    <nd ref="2"/>
    <tag k="highway" v="footway"/>
  </way>
</osm>"#;
        let graph = route_planner::load_osm(osm).unwrap();
        let input_nodes = graph.graph.node_count();
        let mut builder = NetworkBuilder::new(50.0, 0.1);
        builder.add_graph_preserving(&graph);
        assert_eq!(builder.nodes.len(), input_nodes);
        assert!(!builder.directed_edges.is_empty());
    }
}
