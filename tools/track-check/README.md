# tools/track-check —— 轨迹形态核对（纯本地、不联网）

用来回答一个问题：**生成的轨迹像不像真人跑出来的？**

方法是把「像不像」拆成可对比的数字：相邻点间距、丢星点（`type=-1`）位移、折线转角、类型分布。
对照基线来自 10 条真人历史记录（`history_tracks/` 与个人版 dist 里的 `history-*.json`）。

## 真人基线（10 条记录实测）

| 指标 | 范围 |
| --- | --- |
| 相邻点间距 | 中位 6–12m，99 分位 ≤37m，最大 ≤48m |
| 丢星点位移 | 42% 原地复制，50% 在 0.6–20m，8% 在 20–50m，0 个 >50m |
| 折线转角 | 中位 4–12°，99 分位 79–160° |
| 点数 / 时长 | 198–840 点 / 844–1183s |

## 用法

```powershell
# 1. 本地生成两条样本（不联网、不上传；--points 给点位 json，--net 给 OSM 路网）
.\target\release\nekosportsworldtool.exe track-sample --dist 2100 --dur 700 --seed 7 `
    --route legacy --points "_pytmp\points_辽源.json" --out "_pytmp\ours_legacy.json"

.\target\release\nekosportsworldtool.exe track-sample --dist 2100 --dur 700 --seed 7 `
    --route road --points "_pytmp\points_辽源.json" `
    --net "tools\road-network\辽源职业技术学院.osm" --out "_pytmp\ours_road.json"

# 2. 与真人记录同表对比（间距 / 转角 / 类型分布）
python tools\track-check\track_stats.py "_pytmp\ours_legacy.json" "_pytmp\ours_road.json" history_tracks\*.json

# 3. 只看丢星点位移明细
python tools\track-check\drift_detail.py "_pytmp\ours_legacy.json" history_tracks\1327075488.json
```

`track-sample` 自己也会打印一份摘要（点数 / 间距中位·99分位·最大 / 丢星点原地比例），日常看它就够了。

> 这两个脚本只读文件、不联网；轨迹 json 支持 OBS 对象（`run_data` / `allLocJson` 嵌套字符串）
> 或裸点数组。
