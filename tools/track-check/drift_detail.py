# -*- coding: utf-8 -*-
"""列出轨迹里 type=-1（丢星/漂移）点的位移明细，并给出分布统计。

用法:
    python drift_detail.py <轨迹 json ...>

轨迹 json 可以是 OBS 对象（`run_data` / `allLocJson` 嵌套字符串都认），
也可以直接是点数组；真人的历史记录与 `track-sample` 生成的样本都适用。
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from track_stats import LAT_M, as_pts, load_points, lon_m  # noqa: E402


def main():
    paths = sys.argv[1:]
    if not paths:
        print(__doc__)
        return
    all_disp = []
    for path in paths:
        pts_raw, _ = load_points(path)
        if pts_raw is None:
            print("%-24s 无法解析" % os.path.basename(path))
            continue
        pts = as_pts(pts_raw)
        if len(pts) < 2:
            continue
        sx = lon_m(pts[0][0])
        xy = [((p[1] - pts[0][1]) * sx, (p[0] - pts[0][0]) * LAT_M) for p in pts]
        drift = []
        for i in range(1, len(pts)):
            if pts[i][2].get("type") == -1:
                drift.append(round(math.dist(xy[i - 1], xy[i]), 2))
        all_disp.extend(drift)
        print("%-24s 丢星点 %d 个，位移 %s" % (os.path.basename(path), len(drift), drift))
    if all_disp:
        s = sorted(all_disp)
        print(
            "\n合计 %d 个丢星点；中位 %.2f m，均值 %.2f m，最大 %.2f m"
            % (len(s), s[len(s) // 2], sum(s) / len(s), s[-1])
        )
        zero = sum(1 for v in s if v <= 0.6)
        mid = sum(1 for v in s if 0.6 < v <= 20.0)
        big = sum(1 for v in s if 20.0 < v <= 50.0)
        over = sum(1 for v in s if v > 50.0)
        print(
            "≤0.6m（原地）: %d (%.0f%%) | 0.6-20m: %d | 20-50m: %d | >50m: %d"
            % (zero, 100.0 * zero / len(s), mid, big, over)
        )
        print("真人基线：原地 42% / 0.6-20m 50% / 20-50m 8% / >50m 0%")


if __name__ == "__main__":
    main()
