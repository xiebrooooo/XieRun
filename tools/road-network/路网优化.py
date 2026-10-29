#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""路网优化：把手工描/自动生成的 OSM 清理成适合路线规划的干净路网。

做四件事（都可单独关掉）：
  1) 节点吸附合并：距离小于 --snap 米的节点并成一个（消除手抖造成的重影点）
  2) 去掉重边：同一对节点之间的重复 way 只保留一条
  3) 断点桥接：两条 way 的端点相距小于 --bridge 米时并到同一个节点（补上"断断续续"）
  4) 去掉短岔支：长度小于 --stub 米、且一端没有其它连接的悬空小段

用法：
    python 路网优化.py 输入.osm 输出.osm
    python 路网优化.py 输入.osm 输出.osm --snap 0.3 --bridge 6 --stub 3
    python 路网优化.py 输入.osm 输出.osm --dry-run      # 只报告不写文件

坐标系：WGS84（与项目和 App 的 OSM 一致），不做任何坐标变换。

注意：`--snap` 必须明显小于「路网里最近的两条平行线间距」。
跑道是多圈同心线（真实道宽 1.22 m），所以默认取 0.3 m 只合并手抖重影点；
如果给的是普通校区道路网，可以放大到 0.8~1.5 m 把描歪的点并起来。
"""

import math
import re
import sys

MET_LAT = 111132.0


def m_per_deg_lon(lat):
    return 111320.0 * math.cos(math.radians(lat))


def dist_m(a, b):
    mlon = m_per_deg_lon((a[0] + b[0]) / 2.0)
    dx = (b[1] - a[1]) * mlon
    dy = (b[0] - a[0]) * MET_LAT
    return math.hypot(dx, dy)


def read_osm(path):
    text = open(path, "r", encoding="utf-8").read()
    nodes = {}
    order = []
    for m in re.finditer(r'<node id="(\d+)" lat="([-\d.]+)" lon="([-\d.]+)"', text):
        nid = m.group(1)
        nodes[nid] = (float(m.group(2)), float(m.group(3)))
        order.append(nid)
    ways = []
    for m in re.finditer(r"<way id=\"(\d+)\">(.*?)</way>", text, re.S):
        refs = re.findall(r'<nd ref="(\d+)"', m.group(2))
        sup = re.search(r'neko:support"\s+v="(\d+)"', m.group(2))
        name = re.search(r'<tag k="name" v="([^"]*)"', m.group(2))
        if len(refs) >= 2:
            ways.append({
                "id": m.group(1), "refs": refs,
                "support": int(sup.group(1)) if sup else 0,
                "name": name.group(1) if name else "",
            })
    header = text.split("<node", 1)[0].rstrip()
    if not header.endswith(">"):
        header = '<?xml version="1.0" encoding="UTF-8"?>\n<osm version="0.6" generator="路网优化.py">'
    return header, nodes, order, ways


def way_len(nodes, refs):
    s = 0.0
    for i in range(len(refs) - 1):
        a, b = nodes.get(refs[i]), nodes.get(refs[i + 1])
        if a and b:
            s += dist_m(a, b)
    return s


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 1
    src, dst = argv[1], argv[2]
    snap = float(argv[argv.index("--snap") + 1]) if "--snap" in argv else 0.3
    bridge = float(argv[argv.index("--bridge") + 1]) if "--bridge" in argv else 6.0
    stub = float(argv[argv.index("--stub") + 1]) if "--stub" in argv else 3.0
    dry = "--dry-run" in argv

    header, nodes, order, ways = read_osm(src)
    n0, w0 = len(nodes), len(ways)
    len0 = sum(way_len(nodes, w["refs"]) for w in ways)

    # ── 1) 节点吸附合并 ────────────────────────────────────────────────
    remap = {}
    kept = []
    grid = {}
    tol = max(snap, 1e-6)
    for nid in order:
        p = nodes[nid]
        cell = (int(p[0] / 0.00002), int(p[1] / 0.00002))   # ~2m 网格
        found = None
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                for cand in grid.get((cell[0] + dx, cell[1] + dy), []):
                    if dist_m(p, nodes[cand]) <= tol:
                        found = cand
                        break
                if found:
                    break
            if found:
                break
        if found:
            remap[nid] = found
        else:
            remap[nid] = nid
            kept.append(nid)
            grid.setdefault(cell, []).append(nid)

    # ── 3) 断点桥接：端点互相靠近的 way 并到同一节点 ──────────────────
    endpoints = set()
    for w in ways:
        endpoints.add(remap[w["refs"][0]])
        endpoints.add(remap[w["refs"][-1]])
    eps = sorted(endpoints)
    for i in range(len(eps)):
        for j in range(i + 1, len(eps)):
            a, b = remap[eps[i]], remap[eps[j]]
            if a == b:
                continue
            if dist_m(nodes[a], nodes[b]) <= bridge:
                for k, v in list(remap.items()):
                    if v == b:
                        remap[k] = a

    new_ways = []
    seen = set()
    for w in ways:
        refs = [remap[r] for r in w["refs"]]
        # 去掉连续重复节点
        dedup = [refs[0]]
        for r in refs[1:]:
            if r != dedup[-1]:
                dedup.append(r)
        if len(dedup) < 2:
            continue
        # ── 2) 去重边 ──────────────────────────────────────────────────
        key = tuple(sorted((dedup[0], dedup[-1]))) if len(dedup) == 2 else tuple(dedup)
        if key in seen:
            continue
        seen.add(key)
        new_ways.append({"id": w["id"], "refs": dedup, "support": w["support"], "name": w["name"]})

    # ── 4) 去掉短岔支 ──────────────────────────────────────────────────
    degree = {}
    for w in new_ways:
        for r in (w["refs"][0], w["refs"][-1]):
            degree[r] = degree.get(r, 0) + 1
    final_ways = []
    dropped_stub = 0
    for w in new_ways:
        ends = (w["refs"][0], w["refs"][-1])
        dangling = any(degree.get(e, 0) <= 1 for e in ends)
        if dangling and way_len(nodes, w["refs"]) < stub:
            dropped_stub += 1
            continue
        final_ways.append(w)

    used = {r for w in final_ways for r in w["refs"]}
    len1 = sum(way_len(nodes, w["refs"]) for w in final_ways)

    print("=== 路网优化报告 ===")
    print("节点: %d -> %d（吸附 %.2f m）" % (n0, len(used), snap))
    print("路段: %d -> %d（去重边 + 去短岔 %d 条）" % (w0, len(final_ways), dropped_stub))
    print("总长: %.0f m -> %.0f m" % (len0, len1))
    if w0 and len(final_ways) < w0:
        lost = w0 - len(final_ways)
        if lost > max(2, w0 * 0.1):
            print("⚠ 路段少了 %d 条（>10%%）——如果路网含多圈跑道，把 --snap 调到 0.2~0.3 再试" % lost)
    if dry:
        return 0

    out = ['<?xml version="1.0" encoding="UTF-8"?>',
           '<osm version="0.6" generator="路网优化.py">',
           '  <!-- 优化后：节点 %d / 路段 %d -->' % (len(used), len(final_ways))]
    for nid in order:
        if remap[nid] == nid and nid in used:
            p = nodes[nid]
            out.append('  <node id="%s" lat="%.7f" lon="%.7f"/>' % (nid, p[0], p[1]))
    for w in final_ways:
        out.append('  <way id="%s">' % w["id"])
        for r in w["refs"]:
            out.append('    <nd ref="%s"/>' % r)
        out.append('    <tag k="highway" v="residential"/>')
        if w["name"]:
            out.append('    <tag k="name" v="%s"/>' % w["name"])
        out.append('    <tag k="neko:support" v="%d"/>' % w["support"])
        out.append('  </way>')
    out.append('</osm>')
    with open(dst, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print("已写出 %s" % dst)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
