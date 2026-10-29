/*!
 * AmapSatelliteLayer —— 高德卫星瓦片底图（Canvas 图层，零依赖）
 * ---------------------------------------------------------------------------
 * 设计目标：**只做底图接入，不改动你现有的路网绘制逻辑**。
 *
 * 用法（三步）：
 *   1) 建一个底图对象，把你的「经纬度 → 屏幕像素」投影函数交给它：
 *        const base = new AmapSatelliteLayer({
 *          project: (lat, lon) => [x, y],   // 复用你已有的投影，保证两层完全对齐
 *          unproject: (x, y) => [lat, lon], // 屏幕像素 → 经纬度（用于算可见范围）
 *          zoom: () => currentTileZoom,     // 当前瓦片层级 3..18
 *          tileFrame: 'gcj02',              // 你的路网数据是什么坐标系：gcj02 / bd09 / wgs84
 *          onNeedRedraw: () => render(),    // 瓦片陆续到位时叫我们重画
 *        });
 *   2) 在你的 render() 里，**先画底图，再画你原来的路网**：
 *        base.draw(ctx);        // ① 高德卫星底图层
 *        drawMyRoadNetwork(ctx); // ② 你原来的代码，原封不动
 *   3) 视图（平移/缩放）变化后调用 base.invalidate()；
 *      页面卸载时 base.destroy()。
 *
 * 图层顺序由「调用顺序」保证：底图 → 路网，路网永远压在卫星图上面。
 *
 * 瓦片坐标系说明（重要，容易踩坑）：
 *   - 高德瓦片是 **GCJ-02** 的 Web 墨卡托切片；
 *   - 如果你的路网是 BD-09（百度系），底图必须先 GCJ→BD 再投影，
 *     否则两层会整体错开约 500~700 米；
 *   - 所以这里提供 tileFrame 选项，内部自动做 GCJ↔BD↔WGS 换算。
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) {
    module.exports = factory();            // Node / 打包器
  } else {
    root.AmapSatelliteLayer = factory();   // 浏览器 <script> 直接引
  }
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  // ── 瓦片地址 ────────────────────────────────────────────────────────────
  // style=6：卫星影像（JPEG）    style=8：路网+标注（PNG，带透明）
  // {s} 为镜像号 1..4，用于分摊请求；{x}/{y}/{z} 为瓦片行列号与层级。
  var URL_SAT = 'https://webst0{s}.is.autonavi.com/appmaptile?style=6&x={x}&y={y}&z={z}';
  var URL_OVL = 'https://webst0{s}.is.autonavi.com/appmaptile?style=8&x={x}&y={y}&z={z}';
  var MIRRORS = [1, 2, 3, 4];
  var MIN_Z = 3;
  var MAX_Z = 18;          // 高德影像层最高约 18 级
  var TILE = 256;
  var MAX_TILES = 120;     // 单帧最多拉多少张，防止视野过小时请求爆炸

  // ── 坐标系换算（GCJ-02 / BD-09 / WGS84）────────────────────────────────
  var X_PI = (Math.PI * 3000.0) / 180.0;
  var A = 6378245.0;
  var EE = 0.006693421622965943;

  function outOfChina(lat, lon) {
    return !(lon >= 72.004 && lon <= 137.8347 && lat >= 0.8293 && lat <= 55.8271);
  }
  function tLat(x, y) {
    var r = -100 + 2 * x + 3 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * Math.sqrt(Math.abs(x));
    r += ((20 * Math.sin(6 * x * Math.PI) + 20 * Math.sin(2 * x * Math.PI)) * 2) / 3;
    r += ((20 * Math.sin(y * Math.PI) + 40 * Math.sin((y / 3) * Math.PI)) * 2) / 3;
    r += ((160 * Math.sin((y / 12) * Math.PI) + 320 * Math.sin((y * Math.PI) / 30)) * 2) / 3;
    return r;
  }
  function tLon(x, y) {
    var r = 300 + x + 2 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * Math.sqrt(Math.abs(x));
    r += ((20 * Math.sin(6 * x * Math.PI) + 20 * Math.sin(2 * x * Math.PI)) * 2) / 3;
    r += ((20 * Math.sin(x * Math.PI) + 40 * Math.sin((x / 3) * Math.PI)) * 2) / 3;
    r += ((150 * Math.sin((x / 12) * Math.PI) + 300 * Math.sin((x / 30) * Math.PI)) * 2) / 3;
    return r;
  }
  function wgs2gcj(lat, lon) {
    if (outOfChina(lat, lon)) return [lat, lon];
    var dLat = tLat(lon - 105, lat - 35);
    var dLon = tLon(lon - 105, lat - 35);
    var radLat = (lat / 180) * Math.PI;
    var magic = Math.sin(radLat);
    magic = 1 - EE * magic * magic;
    var sq = Math.sqrt(magic);
    dLat = (dLat * 180) / (((A * (1 - EE)) / (magic * sq)) * Math.PI);
    dLon = (dLon * 180) / ((A / sq) * Math.cos(radLat) * Math.PI);
    return [lat + dLat, lon + dLon];
  }
  function gcj2wgs(lat, lon) {
    if (outOfChina(lat, lon)) return [lat, lon];
    var wLat = lat, wLon = lon;
    for (var i = 0; i < 6; i++) {
      var g = wgs2gcj(wLat, wLon);
      wLat += lat - g[0];
      wLon += lon - g[1];
    }
    return [wLat, wLon];
  }
  function gcj2bd(lat, lon) {
    var z = Math.sqrt(lon * lon + lat * lat) + 0.00002 * Math.sin(lat * X_PI);
    var theta = Math.atan2(lat, lon) + 0.000003 * Math.cos(lon * X_PI);
    return [z * Math.sin(theta) + 0.006, z * Math.cos(theta) + 0.0065];
  }
  function bd2gcj(lat, lon) {
    var x = lon - 0.0065, y = lat - 0.006;
    var z = Math.sqrt(x * x + y * y) - 0.00002 * Math.sin(y * X_PI);
    var theta = Math.atan2(y, x) - 0.000003 * Math.cos(x * X_PI);
    return [z * Math.sin(theta), z * Math.cos(theta)];
  }
  /** 把「瓦片自带的 GCJ 坐标」换算到「你项目路网使用的坐标系」。 */
  function fromGcj(lat, lon, frame) {
    if (frame === 'bd09') return gcj2bd(lat, lon);
    if (frame === 'wgs84') return gcj2wgs(lat, lon);
    return [lat, lon];                 // 默认 gcj02
  }
  /** 反向：你项目坐标系 → GCJ（算可见瓦片范围时用）。 */
  function toGcj(lat, lon, frame) {
    if (frame === 'bd09') return bd2gcj(lat, lon);
    if (frame === 'wgs84') return wgs2gcj(lat, lon);
    return [lat, lon];
  }

  // ── 标准 Web 墨卡托瓦片公式 ────────────────────────────────────────────
  function lon2tileX(lon, z) { return ((lon + 180) / 360) * Math.pow(2, z); }
  function lat2tileY(lat, z) {
    var r = (Math.max(-85.05112878, Math.min(85.05112878, lat)) / 180) * Math.PI;
    return ((1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2) * Math.pow(2, z);
  }
  function tileX2lon(x, z) { return (x / Math.pow(2, z)) * 360 - 180; }
  function tileY2lat(y, z) {
    var n = Math.PI - (2 * Math.PI * y) / Math.pow(2, z);
    return (180 / Math.PI) * Math.atan(0.5 * (Math.exp(n) - Math.exp(-n)));
  }

  // ── 主体 ────────────────────────────────────────────────────────────────
  /**
   * @param {Object} o
   * @param {(lat:number, lon:number)=>[number,number]} o.project   经纬度→屏幕像素（复用你的投影）
   * @param {(x:number, y:number)=>[number,number]}     o.unproject 屏幕像素→经纬度
   * @param {()=>number}                                o.zoom      当前瓦片层级（3..18）
   * @param {'gcj02'|'bd09'|'wgs84'}                   [o.tileFrame='gcj02'] 你的路网坐标系
   * @param {boolean}                                  [o.overlay=false]  是否叠加 style=8 路网标注
   * @param {string|null}                              [o.crossOrigin=null] 见 README「跨域」
   * @param {()=>void}                                 [o.onNeedRedraw]    瓦片到位后请重画
   * @param {number}                                   [o.maxTiles=120]
   * @param {boolean}                                  [o.disabled=false]
   */
  function AmapSatelliteLayer(o) {
    o = o || {};
    if (typeof o.project !== 'function') throw new Error('AmapSatelliteLayer: 需要 project(lat,lon)=>[x,y]');
    this.project = o.project;
    this.unproject = o.unproject || null;
    this.zoomFn = o.zoom || function () { return 16; };
    this.frame = o.tileFrame || 'gcj02';
    this.overlay = !!o.overlay;
    this.crossOrigin = o.crossOrigin === undefined ? null : o.crossOrigin;
    this.onNeedRedraw = o.onNeedRedraw || function () {};
    this.maxTiles = o.maxTiles || MAX_TILES;
    this.disabled = !!o.disabled;

    this.cache = Object.create(null);   // "z/x/y/layer" → {img, state}
    this.pending = 0;
    this.errors = 0;
    this._lastKey = null;
  }

  /** 视图变了（平移/缩放/尺寸变化）后调用，让下一帧重算可见瓦片。 */
  AmapSatelliteLayer.prototype.invalidate = function () { this._lastKey = null; };
  /** 图层开关。 */
  AmapSatelliteLayer.prototype.setEnabled = function (v) { this.disabled = !v; this.invalidate(); };
  AmapSatelliteLayer.prototype.setOverlay = function (v) { this.overlay = !!v; this.invalidate(); };

  /** 根据当前视图算出可见瓦片列表。需要 unproject 才能知道屏幕四角的世界坐标。 */
  AmapSatelliteLayer.prototype._visible = function (ctx) {
    if (!this.unproject) return [];
    var w = ctx.canvas.width, h = ctx.canvas.height;
    var z = Math.max(MIN_Z, Math.min(MAX_Z, Math.round(this.zoomFn())));
    var tl = this.unproject(0, 0);            // 屏幕左上 → [lat,lon]（你的坐标系）
    var br = this.unproject(w, h);
    var a = toGcj(tl[0], tl[1], this.frame);  // → GCJ 才能套瓦片公式
    var b = toGcj(br[0], br[1], this.frame);
    var latMax = Math.max(a[0], b[0]), latMin = Math.min(a[0], b[0]);
    var lonMin = Math.min(a[1], b[1]), lonMax = Math.max(a[1], b[1]);
    var n = Math.pow(2, z);
    var x0 = Math.max(0, Math.floor(lon2tileX(lonMin, z)) - 1);
    var x1 = Math.min(n - 1, Math.floor(lon2tileX(lonMax, z)) + 1);
    var y0 = Math.max(0, Math.floor(lat2tileY(latMax, z)) - 1);
    var y1 = Math.min(n - 1, Math.floor(lat2tileY(latMin, z)) + 1);
    var out = [];
    for (var y = y0; y <= y1; y++) {
      for (var x = x0; x <= x1; x++) out.push([x, y]);
    }
    return { z: z, tiles: out };
  };

  /** 取/建一张瓦片。返回 {img,ready}。 */
  AmapSatelliteLayer.prototype._tile = function (layer, z, x, y) {
    var key = z + '/' + x + '/' + y + '/' + layer;
    var hit = this.cache[key];
    if (hit) return hit;
    var self = this;
    var tpl = layer === 'sat' ? URL_SAT : URL_OVL;
    var s = MIRRORS[(x + y) % MIRRORS.length];
    var url = tpl.replace('{s}', s).replace('{x}', x).replace('{y}', y).replace('{z}', z);
    var img = new Image();
    // 跨域说明：只是 drawImage 显示的话不要设 crossOrigin（高德瓦片不带 CORS 头）；
    // 若你必须 canvas 读像素(toDataURL/getImageData)，见 README 的取舍。
    if (this.crossOrigin) img.crossOrigin = this.crossOrigin;
    var rec = { img: img, ready: false, failed: false };
    this.cache[key] = rec;
    this.pending++;
    img.onload = function () {
      rec.ready = true; self.pending--;
      self._lastKey = null; self.onNeedRedraw();
    };
    img.onerror = function () {
      rec.failed = true; self.pending--; self.errors++;
      self._lastKey = null; self.onNeedRedraw();
    };
    img.src = url;
    return rec;
  };

  /**
   * 把底图画到 ctx 上 —— 请在渲染循环里**最先**调用。
   * 返回 true 表示底图已参与绘制。
   */
  AmapSatelliteLayer.prototype.draw = function (ctx) {
    if (this.disabled) return false;
    var vis = this._visible(ctx);
    if (!vis || !vis.tiles.length) return false;
    var z = vis.z, tiles = vis.tiles;
    if (tiles.length > this.maxTiles) return false;   // 视野太大，先不画，等用户放大

    var layers = this.overlay ? ['sat', 'ovl'] : ['sat'];
    for (var li = 0; li < layers.length; li++) {
      var layer = layers[li];
      for (var i = 0; i < tiles.length; i++) {
        var x = tiles[i][0], y = tiles[i][1];
        var rec = this._tile(layer, z, x, y);
        if (!rec.ready) continue;
        // 瓦片左上/右下角的 GCJ 经纬度 → 你的坐标系 → 屏幕像素
        var tlG = [tileY2lat(y, z), tileX2lon(x, z)];
        var brG = [tileY2lat(y + 1, z), tileX2lon(x + 1, z)];
        var tl = fromGcj(tlG[0], tlG[1], this.frame);
        var br = fromGcj(brG[0], brG[1], this.frame);
        var p0 = this.project(tl[0], tl[1]);
        var p1 = this.project(br[0], br[1]);
        var w = p1[0] - p0[0], h = p1[1] - p0[1];
        if (!(w > 0) || !(h > 0)) continue;
        try {
          ctx.drawImage(rec.img, p0[0], p0[1], w, h);
        } catch (e) { /* 单张失败不影响整体 */ }
      }
    }
    return true;
  };

  /** 预取一批瓦片（可选，用于启动时预热）。 */
  AmapSatelliteLayer.prototype.prefetch = function (lat, lon, z, ring) {
    z = Math.max(MIN_Z, Math.min(MAX_Z, z));
    ring = ring || 1;
    var g = toGcj(lat, lon, this.frame);
    var cx = Math.floor(lon2tileX(g[1], z)), cy = Math.floor(lat2tileY(g[0], z));
    var n = Math.pow(2, z);
    for (var dy = -ring; dy <= ring; dy++) {
      for (var dx = -ring; dx <= ring; dx++) {
        var x = cx + dx, y = cy + dy;
        if (x >= 0 && y >= 0 && x < n && y < n) this._tile('sat', z, x, y);
      }
    }
  };

  /** 释放引用（图片会随 GC 回收）。 */
  AmapSatelliteLayer.prototype.destroy = function () {
    this.cache = Object.create(null);
    this.onNeedRedraw = function () {};
  };

  AmapSatelliteLayer.MIN_Z = MIN_Z;
  AmapSatelliteLayer.MAX_Z = MAX_Z;
  AmapSatelliteLayer.URL_SAT = URL_SAT;
  AmapSatelliteLayer.URL_OVL = URL_OVL;
  AmapSatelliteLayer.coord = {
    wgs2gcj: wgs2gcj, gcj2wgs: gcj2wgs, gcj2bd: gcj2bd, bd2gcj: bd2gcj,
  };
  return AmapSatelliteLayer;
});
