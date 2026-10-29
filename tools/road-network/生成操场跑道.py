#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""按既有跑道环生成「多圈跑道」OSM。

用途：运动世界要求在学校操场跑几圈。校方路网如果只有一条中心线，
生成的路线只会在同一条线上反复走；这里把中心线按跑道人行道宽
（默认 1.5 m）向外/向内平移，生成 N 条同心闭合跑道线，写进 OSM。

用法：
    python 生成操场跑道.py <输入.osm> <输出.osm> [--lanes 6] [--spacing 1.5]
    python 生成操场跑道.py 进.osm 出.osm --lanes 8 --spacing 1.22

说明：
  - 只处理「闭合环」型的路网（操场）：把全部节点当作一个环，
    按质心方向做径向平移；
  - 原有点/路段原样保留，新跑道用新的节点与 way id 追加；
  - 输入输出都是 WGS84 的标准 OSM（与项目和 App 一致）。
"""

import math
import re
import sys

MET_LAT = 111132.0


def read_osm(path):
    with open(path, "r", encoding="utf-8") as f:
        text = f.read()
    nodes = {}
    for m in re.finditer(r'<node id="(\d+)" lat="([-\d.]+)" lon="([-\d.]+)"', text):
        nodes[m.group(1)] = (float(m.group(2)), float(m.group(3)))
    ways = []
    for m in re.finditer(r"<way id=\"(\d+)\">(.*?)</way>", text, re.S):
        refs = re.findall(r'<nd ref="(\d+)"', m.group(2))
        if len(refs) >= 2:
            ways.append((m.group(1), refs, m.group(0)))
    return text, nodes, ways


def to_plane(lat, lon, clat, clon, m_lon):
    return ((lon - clon) * m_lon, (lat - clat) * MET_LAT)


def to_deg(x, y, clat, clon, m_lon):
    return (clat + y / MET_LAT, clon + x / m_lon)


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 1
    src, dst = argv[1], argv[2]
    lanes = 6
    spacing = 1.5
    if "--lanes" in argv:
        lanes = max(1, int(argv[argv.index("--lanes") + 1]))
    if "--spacing" in argv:
        spacing = float(argv[argv.index("--spacing") + 1])

    text, nodes, ways = read_osm(src)
    if not nodes:
        print("输入 OSM 没有节点")
        return 1

    # 质心（度）
    clat = sum(p[0] for p in nodes.values()) / len(nodes)
    clon = sum(p[1] for p in nodes.values()) / len(nodes)
    m_lon = 111320.0 * math.cos(math.radians(clat))

    # 找最大闭合环：这里把全部节点按角度排序当作一个环（操场就是这种情况）
    ordered = sorted(nodes.items(),
                     key=lambda kv: math.atan2(kv[1][0] - clat, kv[1][1] - clon))
    ring = [p for _id, p in ordered]

    def ring_len(pts):
        s = 0.0
        for i in range(len(pts)):
            a = pts[i]
            b = pts[(i + 1) % len(pts)]
            pa = to_plane(a[0], a[1], clat, clon, m_lon)
            pb = to_plane(b[0], b[1], clat, clon, m_lon)
            s += math.hypot(pb[0] - pa[0], pb[1] - pa[1])
        return s

    base_r = []
    for (lat, lon) in ring:
        x, y = to_plane(lat, lon, clat, clon, m_lon)
        base_r.append(math.hypot(x, y))
    print("环节点 %d，半径 %.1f~%.1f m，中心线一圈约 %.0f m"
          % (len(ring), min(base_r), max(base_r), ring_len(ring)))

    out = [text.rstrip()]
    out = out[0].replace("</osm>", "").rstrip()
    out += "\n  <!-- 自动生成的同心跑道：%d 圈，间距 %.2f m -->\n" % (lanes, spacing)

    next_node = 13000000000000
    next_way = 13000003000000
    mid = (lanes - 1) / 2.0
    summary = []
    for li in range(lanes):
        off = (li - mid) * spacing            # 以中心线为基准向外/向内平移
        ids = []
        pts = []
        for (lat, lon) in ring:
            x, y = to_plane(lat, lon, clat, clon, m_lon)
            r = math.hypot(x, y)
            if r < 1e-6:
                nx, ny = x, y
            else:
                k = (r + off) / r
                nx, ny = x * k, y * k
            nlat, nlon = to_deg(nx, ny, clat, clon, m_lon)
            nid = str(next_node)
            next_node += 1
            ids.append(nid)
            pts.append((nlat, nlon))
            out += '  <node id="%s" lat="%.7f" lon="%.7f"/>\n' % (nid, nlat, nlon)
        # 闭合 way
        out += '  <way id="%d">\n' % next_way
        for nid in ids + [ids[0]]:
            out += '    <nd ref="%s"/>\n' % nid
        out += '    <tag k="highway" v="residential"/>\n'
        out += '    <tag k="name" v="操场跑道 %d"/>\n' % (li + 1)
        out += '    <tag k="neko:support" v="9"/>\n'
        out += "  </way>\n"
        next_way += 1
        summary.append(ring_len(pts))

    out += "</osm>\n"
    with open(dst, "w", encoding="utf-8", newline="\n") as f:
        f.write(out)
    print("已写出 %s" % dst)
    for i, s in enumerate(summary, 1):
        print("  跑道 %d: 一圈约 %.1f m" % (i, s))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
