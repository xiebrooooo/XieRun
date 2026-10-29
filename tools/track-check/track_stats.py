# -*- coding: utf-8 -*-
"""对照真实真人轨迹（history_tracks/*.json）与本地生成轨迹的几何质量。

关心三件事：
 1. 相邻点间距分布（真人 vs 我们生成的"断崖/漂移"点）
 2. 丢星点（type=-1 / locType 非正常）的位移是不是"随机乱跳"
 3. 折线转角（是否平滑）与整体轮廓形态
"""
import json
import math
import os
import sys

LAT_M = 111132.0


def lon_m(lat):
    return 111320.0 * math.cos(math.radians(lat))


def _deep_get_points(obj, depth=0):
    """在（可能多层嵌套的）OBS 对象里找点数组。"""
    if depth > 4 or obj is None:
        return None
    if isinstance(obj, list):
        if obj and isinstance(obj[0], dict) and ("glat" in obj[0] or "gLat" in obj[0] or "lat" in obj[0]):
            return obj
        return None
    if isinstance(obj, dict):
        for key in ("run_data", "allLocJson", "locations", "data", "list"):
            if key in obj:
                v = obj[key]
                if isinstance(v, str):
                    s = v.strip()
                    if s.startswith("{") or s.startswith("["):
                        try:
                            got = _deep_get_points(json.loads(s), depth + 1)
                        except Exception:
                            got = None
                        if got:
                            return got
                else:
                    got = _deep_get_points(v, depth + 1)
                    if got:
                        return got
        for v in obj.values():
            if isinstance(v, str) and v.lstrip().startswith("[") and "glat" in v:
                try:
                    return json.loads(v)
                except Exception:
                    pass
    return None


def load_points(path):
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        raw = json.load(f)
    return _deep_get_points(raw), raw


def as_pts(points):
    out = []
    for p in points:
        if not isinstance(p, dict):
            continue
        lat = p.get("gLat", p.get("lat"))
        lon = p.get("gLng", p.get("gLon", p.get("lon")))
        if lat is None or lon is None:
            continue
        out.append((float(lat), float(lon), p))
    return out


def summarize(path):
    pts_raw, raw = load_points(path)
    if pts_raw is None:
        return None
    pts = as_pts(pts_raw)
    if len(pts) < 3:
        return None
    lat0 = pts[0][0]
    sx = lon_m(lat0)
    sy = LAT_M
    xy = [((p[1] - pts[0][1]) * sx, (p[0] - pts[0][0]) * sy) for p in pts]

    steps = [math.dist(xy[i], xy[i + 1]) for i in range(len(xy) - 1)]
    times = [p[2].get("totalTime", 0) for p in pts]
    dts = [max(1.0, times[i + 1] - times[i]) for i in range(len(times) - 1)]
    speeds = [s / t for s, t in zip(steps, dts)]

    # 转角（以点 i-1,i,i+1 折角角度）
    turns = []
    for i in range(1, len(xy) - 1):
        a = (xy[i][0] - xy[i - 1][0], xy[i][1] - xy[i - 1][1])
        b = (xy[i + 1][0] - xy[i][0], xy[i + 1][1] - xy[i][1])
        na, nb = math.hypot(*a), math.hypot(*b)
        if na < 1e-9 or nb < 1e-9:
            continue
        cosv = max(-1.0, min(1.0, (a[0] * b[0] + a[1] * b[1]) / (na * nb)))
        turns.append(math.degrees(math.acos(cosv)))

    kinds = {}
    for p in pts:
        k = (p[2].get("type"), p[2].get("locType"))
        kinds[k] = kinds.get(k, 0) + 1

    # 丢星点位移：type == -1 的点，相对上一点的距离
    drift = []
    for i, p in enumerate(pts):
        if p[2].get("type") != -1 or i == 0:
            continue
        drift.append(math.dist(xy[i - 1], xy[i]))

    def pct(vals, q):
        if not vals:
            return 0.0
        s = sorted(vals)
        return s[min(len(s) - 1, int(q * len(s)))]

    res = {
        "file": os.path.basename(path),
        "n": len(pts),
        "dur_s": times[-1] if times else 0,
        "totalDis": pts[-1][2].get("totalDis"),
        "step_p50": round(pct(steps, 0.5), 2),
        "step_p90": round(pct(steps, 0.9), 2),
        "step_p99": round(pct(steps, 0.99), 2),
        "step_max": round(max(steps), 2),
        "speed_p50": round(pct(speeds, 0.5), 2),
        "speed_max": round(max(speeds), 2),
        "turn_p50": round(pct(turns, 0.5), 1),
        "turn_p90": round(pct(turns, 0.9), 1),
        "turn_p99": round(pct(turns, 0.99), 1),
        "turn_max": round(max(turns), 1) if turns else 0,
        "kinds": kinds,
        "drift_n": len(drift),
        "drift_p50": round(pct(drift, 0.5), 2),
        "drift_max": round(max(drift), 2) if drift else 0,
    }
    return res


def main():
    paths = sys.argv[1:]
    rows = []
    for p in paths:
        r = summarize(p)
        if r:
            rows.append(r)
    if not rows:
        print("没有可解析的轨迹")
        return
    keys = [k for k in rows[0] if k != "kinds"]
    print(" | ".join(keys))
    for r in rows:
        print(" | ".join(str(r[k]) for k in keys))
    print()
    for r in rows:
        print(r["file"], "kinds(type,locType)=", r["kinds"])


if __name__ == "__main__":
    main()
