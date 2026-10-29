# 高德卫星底图接入 + 网页版路网编辑器

这个目录是「把高德卫星影像当底图、把路网矢量画在上面」的最小完整实现，以及基于它的
一个网页版路网编辑器。**不需要任何构建工具，双击 html 就能用。**

| 文件 | 作用 |
| --- | --- |
| `amap-satellite-layer.js` | 底图图层模块（零依赖，纯 canvas）。只负责在最底层画卫星瓦片，不碰你的绘制逻辑 |
| `index.html` | 最小接入示例：卫星底图 + 路网矢量叠加 + 平移缩放 |
| `路网编辑器.html` | 网页版路网编辑器：打开/导出 OSM、删边/连线/移点、经纬度定位、视图锁定 |

---

## 1. 快速开始

双击 `路网编辑器.html`（Chrome / Edge）。

1. **打开 .osm** → 选 `assets\builtin-campus-network.osm`
2. **载入打卡点…** → 选 App 目录里的 `points_cache.json`（打卡圈会落到正确位置）
3. 勾上 **卫星**，对着影像修路网：
   - **删边**：左键点路段 = 标记删除（红虚线），再点一次 = 恢复
   - **连线**：左键点两个节点连线；点空白处 = 新建节点并连上
   - **移点**：左键按住节点拖动
   - **打卡点**：左键点一下 = 在该处加一个打卡点
4. **保存 .osm** → 导出到 `assets\builtin-campus-network.osm`，然后重新编译程序

快捷键：`Ctrl+Z` 撤销、`Ctrl+Y` 重做、`Ctrl+S` 导出、`Esc` 取消选中。
右键拖动（或 Shift+左键）平移，滚轮缩放。

---

## 2. 瓦片地址

高德 Web 端栅格瓦片（**不需要 Key**）：

```
卫星影像   https://webst0{1..4}.is.autonavi.com/appmaptile?style=6&x={x}&y={y}&z={z}
路网标注   https://webst0{1..4}.is.autonavi.com/appmaptile?style=8&x={x}&y={y}&z={z}
```

- `{1..4}` = 镜像号，代码里按 `(x+y)%4` 轮询，分摊请求压力
- `style=6` = 卫星影像（JPEG）；`style=8` = 路网+注记（PNG，带透明）
- 瓦片 256×256，标准 Web 墨卡托切片，`z` 建议 3–18（影像层最高约 18 级）
- 另有 `style=7`（无注记路网），需要时把 `URL_OVL` 里的 `style` 改掉即可

---

## 3. 高德 Key 配置说明

**用本目录的栅格瓦片方案不需要 Key**，直接粘 URL 就能出图。

只有当你改用**官方 JS API**（`AMap.TileLayer.Satellite`）时才需要 Key：

1. 去 <https://console.amap.com/dev/key/app> 注册/登录 → 创建应用 → 添加 Key
2. Key 类型选 **Web端(JS API)**（不要选 Web服务/Android/iOS，那几种不能在前端加载瓦片）
3. 给这个 Key 绑定 **域名白名单**；本地调试加 `127.0.0.1`、`localhost`
   （用 `file://` 打开时白名单不生效，建议起个本地服务，见第 6 节）
4. 页面里引入并初始化：

```html
<script src="https://webapi.amap.com/load?key=你的Key&v=2.0&plugin=AMap.TileLayer"></script>
<script>
  var map = new AMap.Map('container', {
    zoom: 17,
    center: [113.1589562, 29.2650505],   // 注意：高德是 [经度, 纬度]
  });
  map.add(new AMap.TileLayer.Satellite());   // 卫星底图
  map.add(new AMap.TileLayer.RoadNet());     // 可选：路网注记
</script>
```

两种方案怎么选：

| | 栅格瓦片（本目录） | 官方 JS API |
| --- | --- | --- |
| 需要 Key | 否 | 是（+域名白名单） |
| 依赖 | 无 | 高德 SDK |
| 自定义矢量图层 | 直接画在同一张 canvas 上，天然对齐 | 需要 `AMap.CustomLayer`/`CanvasLayer` |
| 适用 | 纯展示 + 自己画矢量 | 需要高德的地图交互/POI/搜索能力 |

---

## 4. 图层顺序与接入方式

图层顺序由**绘制顺序**保证，代码里就三行：

```js
function render() {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  base.draw(ctx);            // ① 高德卫星底图（最底层）
  drawMyRoadNetwork(ctx);    // ② 你原来的路网绘制逻辑，原封不动
}
```

接入到现有项目的三步：

```html
<script src="amap-satellite-layer.js"></script>
```

```js
const base = new AmapSatelliteLayer({
  project:   (lat, lon) => [x, y],   // ← 复用你已有的「经纬度→屏幕像素」函数
  unproject: (x, y) => [lat, lon],   // ← 复用你已有的反投影函数
  zoom:      () => currentZoom,      // 当前瓦片层级(3..18)
  tileFrame: 'gcj02',                // 你的路网数据坐标系：gcj02 / bd09 / wgs84
  onNeedRedraw: () => render(),      // 瓦片陆续到位后叫我们重画
});
```

平移/缩放后调用 `base.invalidate()`，页面卸载时 `base.destroy()`。
`base.prefetch(lat, lon, z, 2)` 可选，用于启动时预热中心 5×5 瓦片。

**关键点**：底图不自己维护投影，而是调用你的 `project()`。
这样底图和路网用的是同一套投影，天然像素级对齐，而你原有的绘制代码一行都不用改。

---

## 5. 坐标系（最容易踩的坑）

| 数据 | 坐标系 |
| --- | --- |
| 高德瓦片 | **GCJ-02** |
| App 打卡点 / 百度系 | BD-09 |
| OSM 文件（我们的 `builtin-campus-network.osm`） | WGS84 |

`tileFrame` 选错会整体错开：GCJ↔BD 在中国境内约 **500–700 米**。

- 本目录的编辑器**内部统一在 GCJ-02 下绘制**（和底图同系）；
  读 OSM 时 WGS84→GCJ，读打卡点时 BD-09→GCJ，导出 OSM 时 GCJ→WGS84。
- 你的项目如果路网本来就是 BD-09，就把 `tileFrame: 'bd09'`，模块内部会自动 GCJ→BD。
- 换算函数也导出在 `AmapSatelliteLayer.coord`：
  `wgs2gcj / gcj2wgs / gcj2bd / bd2gcj`

---

## 6. 跨域（CORS）说明

| 场景 | 是否需要 CORS |
| --- | --- |
| `ctx.drawImage(tileImg, …)` 只显示瓦片 | **不需要**。`<img>` 跨域加载不报错 |
| 之后 `canvas.toDataURL()` / `getImageData()` 读像素 | **需要**。跨域图会让 canvas 变「脏」，抛 `SecurityError` |
| `fetch()` 直接抓瓦片字节 | **需要**，且高德瓦片不带 `Access-Control-Allow-Origin`，必然失败 |
| `file://` 打开页面并用 `fetch()` 读本地 .osm | **需要**（被 CORS 拦）。本编辑器的做法是用 `<input type="file">` + `FileReader`，绕开限制 |

结论：
- 只做「底图 + 矢量叠加显示」→ 用默认配置（`crossOrigin: null`），零问题。
- 需要导出 canvas 图片 → 加一层**同源代理**（自己后端转发瓦片并补 CORS 头），
  然后 `crossOrigin: 'anonymous'`；不能直接读高德原始瓦片的像素。
- 本地调试如果要用 `fetch`，起个服务：`python -m http.server 8000`，
  然后访问 `http://127.0.0.1:8000/路网编辑器.html`。

---

## 7. 排错清单

| 现象 | 原因 / 处理 |
| --- | --- |
| 底图整块空白，但路网能画 | 没网或被代理拦。F12 → Network 看 `appmaptile` 是否为 403/超时；换 `webst02/03/04` 试；公司网络需放行 `*.is.autonavi.com` |
| 瓦片一直转圈、`pending` 不降 | 同上；模块里 `errors` 计数会增长，状态栏会提示 |
| 底图和路网整体错 500–700 米 | `tileFrame` 选错。路网是 BD-09 就设 `'bd09'`，WGS84 就设 `'wgs84'` |
| 底图四角错位、中间对得上 | 你的投影不是 Web 墨卡托（比如等距圆柱）。校园尺度可忽略；大范围请把 `project/unproject` 换成墨卡托 |
| 缩放后底图模糊 | 层级没跟着变。检查 `zoom()` 是否返回**整数**且随缩放更新（本编辑器用整数级 z） |
| 缩放时位置漂移 | 缩放应「以鼠标为锚点」：先记下鼠标下的经纬度，改 z 后再把它移回鼠标位置（本编辑器已实现） |
| 视野太大时不出底图 | 单帧瓦片数超过上限（默认 120）。放大一点，或调大 `maxTiles` |
| `toDataURL` 报 SecurityError | 见第 6 节，用同源代理 + `crossOrigin:'anonymous'` |
| 双击 html 打不开 .osm | 用页面上的 **打开 .osm** 按钮选文件，不要用 `fetch` 读本地路径 |
| 保存的 OSM 加载后位置不对 | 确认导出的是 WGS84（本编辑器已经转换），且程序侧没有再额外做一次 GCJ→BD |
| 高德 Key 提示 `INVALID_USER_DOMAIN` | Key 类型不对或域名白名单没加（仅 JS API 方案会遇到） |

---

## 8. 和本项目对接

1. 编辑器里 **保存 .osm**（导出的是 WGS84）
2. 把 .osm 放到自己方便的位置，之后在程序「路网」页加载它即可 ——
   路网是**运行期读取**的（记在 `config.json` 的 `osm_path` 里），**不需要重新编译程序**
3. 跑步页切到「真实道路」模式生效

> 旧版把路网编译期内置进 exe（`assets\builtin-campus-network.osm` + `include_bytes!`）的做法已废弃：
> 一份源码通用，路网按学校在运行时加载，详见 [docs/学校适配与路网.md](../../docs/学校适配与路网.md)。
