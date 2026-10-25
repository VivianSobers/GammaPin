/*
 * GammaPin monitor panels (G5.2).
 *
 * buildViewModel(snapshot) turns a validated C2 snapshot into display strings
 * and chart series. It is pure (no DOM), so it can be tested under Node.
 * createDashboard(document, uPlot) owns the DOM: header figures, the GEX by
 * strike chart, the per-convention table, data health and the status line.
 *
 * Classic script: window.GPPanels in the browser, module.exports under Node.
 */
(function (root, factory) {
  'use strict';
  var api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.GPPanels = api;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var TRADING_DAYS = 252;
  var MINUTES_PER_SESSION = 375; // 09:15-15:30 IST
  var STALE_MS = 3000;
  // C2 lag includes the wait for the next 1 s snapshot, so a healthy live feed runs p99 ~1.2 s.
  var LAG_WARN_MS = 2000;
  var CRORE = 1e7;
  var MINUS = '−';
  var DASH = '—';

  var CONVENTIONS = {
    S0: { desc: 'US convention', slot: 1 },
    S1: { desc: 'FII + Pro writers', slot: 2 },
    S2: { desc: 'opposite of clients', slot: 3 }
  };

  // ---------------------------------------------------------------- formatting (pure)

  var nfCache = {};
  function nf(dp) {
    if (!nfCache[dp]) nfCache[dp] = new Intl.NumberFormat('en-IN', { minimumFractionDigits: dp, maximumFractionDigits: dp });
    return nfCache[dp];
  }
  function isNum(x) { return typeof x === 'number' && isFinite(x); }

  /** Number with Indian grouping and a true minus sign; null/NaN -> em dash. */
  function fmtNum(x, dp) {
    if (!isNum(x)) return DASH;
    var s = nf(dp || 0).format(Math.abs(x));
    return (x < 0 && s.replace(/[0.,]/g, '') !== '' ? MINUS : '') + s;
  }
  function fmtSigned(x, dp) {
    if (!isNum(x)) return DASH;
    return (x > 0 ? '+' : '') + fmtNum(x, dp);
  }
  /** Strike or level: no decimals when integral, else one. */
  function fmtLevel(x) {
    if (!isNum(x)) return DASH;
    return fmtNum(x, Math.round(x) === x ? 0 : 1);
  }
  function toCrore(rupees) { return isNum(rupees) ? rupees / CRORE : null; }
  /** ₹ per 1% move -> "₹ cr" figure text (no unit). */
  function fmtCrore(rupees) {
    var cr = toCrore(rupees);
    if (cr === null) return DASH;
    return fmtNum(cr, Math.abs(cr) >= 1000 ? 0 : 1);
  }

  function tradingMinutes(tYears) { return isNum(tYears) ? tYears * TRADING_DAYS * MINUTES_PER_SESSION : null; }
  function fmtTradingMinutes(tYears) {
    var m = tradingMinutes(tYears);
    if (m === null) return DASH;
    if (m < 1) return '<1 min';
    var s = fmtNum(m, 0) + ' min';
    if (m > MINUTES_PER_SESSION) s += ' (' + fmtNum(m / MINUTES_PER_SESSION, 1) + ' sessions)';
    return s;
  }

  /** "MAE 61 pts over 18 expiries" or "no history yet". */
  function fmtAccuracy(h1, key) {
    var a = h1 && h1[key];
    if (!a || !isNum(a.mae) || !isNum(a.n) || a.n <= 0) return { text: 'no history yet', has: false };
    return {
      text: 'MAE ' + fmtNum(a.mae, 0) + ' pts over ' + fmtNum(a.n, 0) + (a.n === 1 ? ' expiry' : ' expiries'),
      has: true
    };
  }

  function regimeInfo(regime) {
    return regime === 'long_gamma'
      ? { cls: 'long', icon: '▲', label: 'Long gamma', short: 'long γ' }
      : { cls: 'short', icon: '▼', label: 'Short gamma', short: 'short γ' };
  }

  function fmtAge(ms) {
    if (!isNum(ms)) return { text: DASH, stale: true };
    var s = Math.max(0, ms) / 1000;
    return { text: (s < 10 ? s.toFixed(1) : fmtNum(s, 0)) + ' s ago', stale: ms > STALE_MS };
  }

  var timeFmt = null;
  function fmtTimeIST(tsMs) {
    if (!isNum(tsMs)) return DASH;
    try {
      timeFmt = timeFmt || new Intl.DateTimeFormat('en-GB', { timeZone: 'Asia/Kolkata', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
      return timeFmt.format(new Date(tsMs)) + ' IST';
    } catch (e) {
      return new Date(tsMs).toISOString().slice(11, 19) + ' UTC';
    }
  }

  function fmtExpiry(iso) {
    var m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso || '');
    if (!m) return String(iso || DASH);
    var months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
    return +m[3] + ' ' + (months[+m[2] - 1] || m[2]) + ' ' + m[1];
  }

  // ---------------------------------------------------------------- view model (pure)

  /** Convention keys in display order: S0, S1, S2, then anything else sorted. */
  function conventionKeys(snap) {
    var present = Object.keys((snap && snap.conventions) || {});
    var known = ['S0', 'S1', 'S2'].filter(function (k) { return present.indexOf(k) >= 0; });
    var other = present.filter(function (k) { return known.indexOf(k) < 0; }).sort();
    return known.concat(other);
  }

  /** Colour slot 1..3 for known conventions, then the next free slot, 0 (muted) past three. */
  function slotFor(key, index) {
    if (CONVENTIONS[key]) return CONVENTIONS[key].slot;
    var s = index + 1;
    return s <= 3 ? s : 0;
  }

  function strikeStep(strikes) {
    var step = Infinity;
    for (var i = 1; i < strikes.length; i++) step = Math.min(step, strikes[i] - strikes[i - 1]);
    if (!isFinite(step) || step <= 0) step = strikes.length ? Math.max(1, Math.abs(strikes[0]) * 0.002) : 1;
    return step;
  }

  function buildChartModel(snap, keys) {
    var strikes = snap.strikes || [];
    var step = strikeStep(strikes);
    var series = keys.map(function (k, i) {
      var c = snap.conventions[k];
      return { key: k, slot: slotFor(k, i), values: (c.gex || []).map(toCrore) };
    });
    var markers = [];
    if (isNum(snap.forward)) markers.push({ kind: 'F', key: null, slot: null, x: snap.forward, label: 'F' });
    series.forEach(function (s) {
      var c = snap.conventions[s.key];
      if (isNum(c.flip)) markers.push({ kind: 'flip', key: s.key, slot: s.slot, x: c.flip, label: s.key + ' flip' });
    });
    series.forEach(function (s) {
      var c = snap.conventions[s.key];
      if (isNum(c.pin)) markers.push({ kind: 'pin', key: s.key, slot: s.slot, x: c.pin, label: s.key + ' pin' });
    });
    var xs = strikes.concat(markers.map(function (m) { return m.x; }));
    var xMin = xs.length ? Math.min.apply(null, xs) : 0;
    var xMax = xs.length ? Math.max.apply(null, xs) : 1;
    var yMin = 0, yMax = 0;
    series.forEach(function (s) {
      s.values.forEach(function (v) { if (isNum(v)) { yMin = Math.min(yMin, v); yMax = Math.max(yMax, v); } });
    });
    if (yMin === yMax) { yMin = -1; yMax = 1; }
    var pad = (yMax - yMin) * 0.08;
    return {
      x: strikes.slice(),
      step: step,
      series: series,
      markers: markers,
      xRange: [xMin - step * 0.75, xMax + step * 0.75],
      yRange: [yMin < 0 ? yMin - pad : 0, yMax > 0 ? yMax + pad : 0],
      empty: strikes.length === 0
    };
  }

  function buildViewModel(snap) {
    var keys = conventionKeys(snap);
    var h1 = snap.h1_accuracy;
    var conventions = keys.map(function (k, i) {
      var c = snap.conventions[k];
      var acc = fmtAccuracy(h1, k);
      var ri = regimeInfo(c.regime);
      return {
        key: k,
        desc: CONVENTIONS[k] ? CONVENTIONS[k].desc : 'convention ' + k,
        slot: slotFor(k, i),
        netGexCr: toCrore(c.net_gex),
        netGexText: fmtCrore(c.net_gex),
        flipText: fmtLevel(c.flip),
        flipNote: isNum(c.flip) ? fmtSigned(c.flip - snap.forward, 1) + ' vs F' : 'no sign change within F ± 3%',
        pinText: fmtLevel(c.pin),
        pinNote: isNum(c.pin) ? fmtSigned(c.pin - snap.forward, 1) + ' vs F' : 'no positive GEX strike',
        regime: ri,
        accuracyText: acc.text,
        hasHistory: acc.has
      };
    });

    var h = snap.health || {};
    var nodes = Object.keys(h.nodes || {}).sort().map(function (k) {
      return { name: k, up: h.nodes[k] === 'up', text: h.nodes[k] === 'up' ? 'up' : 'down' };
    });
    var used = h.quotes_used, total = h.quotes_total;
    var frac = isNum(used) && isNum(total) && total > 0 ? used / total : null;

    return {
      schema: 'v' + snap.v,
      source: snap.source,
      tsText: fmtTimeIST(snap.ts_ms),
      header: {
        title: String(snap.underlying) + ' · ' + fmtExpiry(snap.expiry),
        underlying: String(snap.underlying),
        expiry: snap.expiry,
        expiryText: fmtExpiry(snap.expiry),
        spotText: fmtNum(snap.spot, 2),
        forwardText: fmtNum(snap.forward, 2),
        basisText: fmtSigned(snap.forward - snap.spot, 2) + ' vs spot',
        minutesText: fmtTradingMinutes(snap.t_years),
        tYearsText: 't = ' + (isNum(snap.t_years) ? snap.t_years.toPrecision(3) : DASH) + ' y',
        regimes: conventions.map(function (c) { return { key: c.key, slot: c.slot, regime: c.regime }; })
      },
      conventions: conventions,
      health: {
        p50Text: fmtNum(h.lag_ms_p50, 0) + ' ms',
        p99Text: fmtNum(h.lag_ms_p99, 0) + ' ms',
        p99Warn: isNum(h.lag_ms_p99) && h.lag_ms_p99 > LAG_WARN_MS,
        gapsText: fmtNum(h.gaps_today, 0),
        gapsWarn: isNum(h.gaps_today) && h.gaps_today > 0,
        nodes: nodes,
        quotesText: fmtNum(used, 0) + ' / ' + fmtNum(total, 0),
        quotesFrac: frac,
        quotesPctText: frac === null ? DASH : fmtNum(frac * 100, 0) + '% used'
      },
      chart: buildChartModel(snap, keys)
    };
  }

  // ---------------------------------------------------------------- DOM (browser only)

  function el(doc, tag, cls, text) {
    var e = doc.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined && text !== null) e.textContent = text;
    return e;
  }

  function cssVar(doc, name, fallback) {
    var v = getComputedStyle(doc.documentElement).getPropertyValue(name).trim();
    return v || fallback;
  }

  function seriesColor(doc, slot) {
    return slot ? cssVar(doc, '--series-' + slot, '#888') : cssVar(doc, '--text-3', '#888');
  }

  function roundedBar(p, x, w, y0, yv, r) {
    var h = Math.abs(yv - y0);
    if (h < 0.5) return;
    r = Math.max(0, Math.min(r, w / 2, h));
    if (yv < y0) { // positive bar, rounded at the top
      p.moveTo(x, y0);
      p.lineTo(x, yv + r);
      p.arcTo(x, yv, x + r, yv, r);
      p.lineTo(x + w - r, yv);
      p.arcTo(x + w, yv, x + w, yv + r, r);
      p.lineTo(x + w, y0);
    } else { // negative bar, rounded at the bottom
      p.moveTo(x, y0);
      p.lineTo(x, yv - r);
      p.arcTo(x, yv, x + r, yv, r);
      p.lineTo(x + w - r, yv);
      p.arcTo(x + w, yv, x + w, yv - r, r);
      p.lineTo(x + w, y0);
    }
    p.closePath();
  }

  function GexChart(doc, uPlot, mount, legend, markerKey) {
    this.doc = doc;
    this.uPlot = uPlot;
    this.mount = mount;
    this.legend = legend;
    this.markerKey = markerKey;
    this.u = null;
    this.model = null;
    this.shapeKey = '';
    this.hoverIdx = null;
    var self = this;
    if (typeof ResizeObserver !== 'undefined') {
      this.ro = new ResizeObserver(function () { self.resize(); });
      this.ro.observe(mount);
    }
  }

  GexChart.prototype.size = function () {
    var w = Math.max(240, Math.floor(this.mount.clientWidth));
    var h = w < 520 ? 280 : 340;
    return { width: w, height: h };
  };

  GexChart.prototype.resize = function () {
    if (this.u) {
      var s = this.size();
      if (s.width !== this.u.width || s.height !== this.u.height) this.u.setSize(s);
    }
  };

  GexChart.prototype.destroy = function () {
    if (this.u) { this.u.destroy(); this.u = null; }
    this.shapeKey = '';
  };

  GexChart.prototype.update = function (model) {
    this.model = model;
    this.renderMarkerKey();
    if (!this.uPlot) { this.mount.textContent = 'Chart library (uPlot) failed to load.'; return; }
    if (model.empty) {
      this.destroy();
      this.mount.textContent = 'No strikes in this snapshot.';
      this.renderLegend();
      return;
    }
    var shapeKey = model.series.map(function (s) { return s.key; }).join(',') + '|' + model.x.length;
    var data = [model.x].concat(model.series.map(function (s) { return s.values; }));
    if (this.u && shapeKey === this.shapeKey) {
      this.u.setData(data, true);
    } else {
      this.destroy();
      this.mount.textContent = '';
      this.create(data);
      this.shapeKey = shapeKey;
    }
    this.renderLegend();
  };

  GexChart.prototype.create = function (data) {
    var self = this, doc = this.doc, uPlot = this.uPlot;
    var ink2 = cssVar(doc, '--text-2', '#555');
    var ink3 = cssVar(doc, '--text-3', '#888');
    var grid = cssVar(doc, '--grid', '#e1e0d9');
    var axis = cssVar(doc, '--axis', '#c3c2b7');
    var ink1 = cssVar(doc, '--text-1', '#111');
    var surface = cssVar(doc, '--surface', '#fff');
    var font = '11px system-ui, -apple-system, "Segoe UI", sans-serif';
    var model = this.model;
    var n = model.series.length;

    function pr() { return uPlot.pxRatio || (typeof devicePixelRatio === 'number' ? devicePixelRatio : 1); }

    function bars(gi) {
      return function (u, sidx, i0, i1) {
        var m = self.model, xs = u.data[0], ys = u.data[sidx], p = new Path2D(), r = pr();
        var stepPx = Math.abs(u.valToPos(xs[0] + m.step, 'x', true) - u.valToPos(xs[0], 'x', true));
        var groupW = stepPx * 0.84, slotW = groupW / n;
        var gap = Math.min(2 * r, slotW * 0.25), w = Math.max(1, slotW - gap);
        var y0 = u.valToPos(0, 'y', true);
        for (var i = i0; i <= i1; i++) {
          var v = ys[i];
          if (v === null || v === undefined || v === 0) continue;
          var x = u.valToPos(xs[i], 'x', true) - groupW / 2 + gi * slotW + gap / 2;
          roundedBar(p, x, w, y0, u.valToPos(v, 'y', true), 4 * r);
        }
        return { stroke: null, fill: p, clip: null, band: null, gaps: null, flags: 0 };
      };
    }

    function drawMarkers(u) {
      var ctx = u.ctx, r = pr(), b = u.bbox, m = self.model;
      ctx.save();
      // zero baseline
      var y0 = Math.round(u.valToPos(0, 'y', true)) + 0.5;
      ctx.strokeStyle = axis; ctx.lineWidth = r; ctx.setLineDash([]);
      ctx.beginPath(); ctx.moveTo(b.left, y0); ctx.lineTo(b.left + b.width, y0); ctx.stroke();

      ctx.font = (11 * r) + 'px system-ui, -apple-system, "Segoe UI", sans-serif';
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'left';
      var rowH = 14 * r, rows = [[], [], []];
      m.markers.forEach(function (mk) {
        var x = Math.round(u.valToPos(mk.x, 'x', true)) + 0.5;
        if (x < b.left - 1 || x > b.left + b.width + 1) return;
        var color = mk.kind === 'F' ? ink1 : seriesColor(doc, mk.slot);
        ctx.strokeStyle = color;
        ctx.lineWidth = (mk.kind === 'F' ? 2 : 1.5) * r;
        ctx.setLineDash(mk.kind === 'flip' ? [6 * r, 4 * r] : mk.kind === 'pin' ? [2 * r, 3 * r] : []);
        ctx.beginPath(); ctx.moveTo(x, b.top); ctx.lineTo(x, b.top + b.height); ctx.stroke();
        ctx.setLineDash([]);
        if (mk.kind === 'pin') { // diamond on the zero line, ringed by the surface
          var d = 5 * r;
          ctx.beginPath();
          ctx.moveTo(x, y0 - d); ctx.lineTo(x + d, y0); ctx.lineTo(x, y0 + d); ctx.lineTo(x - d, y0); ctx.closePath();
          ctx.fillStyle = color; ctx.fill();
          ctx.lineWidth = 2 * r; ctx.strokeStyle = surface; ctx.stroke();
        }
        // label above the plot: first row where it does not overlap
        var sw = mk.kind === 'F' ? 0 : 8 * r;
        var tw = ctx.measureText(mk.label).width + sw;
        var lx = Math.min(Math.max(x - tw / 2, b.left), b.left + b.width - tw);
        for (var ri = 0; ri < rows.length; ri++) {
          var free = rows[ri].every(function (seg) { return lx + tw + 4 * r < seg[0] || lx > seg[1] + 4 * r; });
          if (!free) continue;
          rows[ri].push([lx, lx + tw]);
          var ly = b.top - (ri + 0.5) * rowH - 2 * r;
          if (sw) { ctx.fillStyle = color; ctx.fillRect(lx, ly - 3 * r, 6 * r, 6 * r); }
          ctx.fillStyle = mk.kind === 'F' ? ink1 : ink2;
          ctx.fillText(mk.label, lx + sw, ly);
          break;
        } // no free row: the marker key below the chart still names it
      });
      ctx.restore();
    }

    var tip = el(doc, 'div', 'chart-tip');
    tip.hidden = true;

    function onCursor(u) {
      var idx = u.cursor.idx;
      if (idx === null || idx === undefined || u.cursor.left < 0) {
        tip.hidden = true;
        if (self.hoverIdx !== null) { self.hoverIdx = null; self.renderLegend(); }
        return;
      }
      var m = self.model;
      tip.textContent = '';
      tip.appendChild(el(doc, 'div', 'tip-head', 'Strike ' + fmtLevel(m.x[idx])));
      m.series.forEach(function (s) {
        var row = el(doc, 'div', 'tip-row');
        var sw = el(doc, 'span', 'swatch s' + s.slot);
        row.appendChild(sw);
        row.appendChild(el(doc, 'span', 'tip-key', s.key));
        row.appendChild(el(doc, 'span', 'tip-val', fmtNum(s.values[idx], 1) + ' cr'));
        tip.appendChild(row);
      });
      tip.hidden = false;
      var ow = u.over.clientWidth, tw = tip.offsetWidth;
      var left = u.cursor.left + 14;
      if (left + tw > ow) left = u.cursor.left - tw - 14;
      tip.style.left = Math.max(0, left) + 'px';
      tip.style.top = Math.max(0, Math.min(u.cursor.top - 10, u.over.clientHeight - tip.offsetHeight)) + 'px';
      if (self.hoverIdx !== idx) { self.hoverIdx = idx; self.renderLegend(); }
    }

    var sz = this.size();
    var opts = {
      width: sz.width,
      height: sz.height,
      padding: [3 * 14 + 8, 12, 0, 0],
      legend: { show: false },
      cursor: { points: { show: false }, drag: { x: false, y: false }, y: false },
      scales: {
        x: { time: false, range: function () { return self.model.xRange.slice(); } },
        y: { range: function () { return self.model.yRange.slice(); } }
      },
      axes: [
        {
          stroke: ink3, font: font, space: 56, size: 36,
          grid: { show: false }, ticks: { stroke: axis, width: 1, size: 4 },
          values: function (u, vals) { return vals.map(function (v) { return fmtLevel(v); }); },
          label: 'Strike', labelFont: font, labelSize: 18
        },
        {
          stroke: ink3, font: font,
          grid: { stroke: grid, width: 1 }, ticks: { show: false },
          values: function (u, vals) { return vals.map(function (v) { return fmtNum(v, 0); }); },
          size: function (u, values) {
            if (!values) return 48;
            var longest = values.reduce(function (a, v) { return Math.max(a, String(v).length); }, 0);
            return Math.max(40, 10 + longest * 7);
          },
          label: '₹ cr per 1% move', labelFont: font, labelSize: 18
        }
      ],
      series: [{}].concat(model.series.map(function (s, i) {
        var c = seriesColor(doc, s.slot);
        return { label: s.key, stroke: c, fill: c, width: 0, paths: bars(i), points: { show: false } };
      })),
      hooks: {
        draw: [drawMarkers],
        setCursor: [onCursor]
      }
    };
    this.u = new uPlot(opts, data, this.mount);
    this.u.over.appendChild(tip);
    this.u.over.addEventListener('mouseleave', function () {
      tip.hidden = true;
      self.hoverIdx = null;
      self.renderLegend();
    });
  };

  GexChart.prototype.renderLegend = function () {
    var doc = this.doc, m = this.model, idx = this.hoverIdx, box = this.legend;
    box.textContent = '';
    if (!m) return;
    var caption = el(doc, 'span', 'legend-caption', idx === null || idx === undefined || !m.x.length
      ? 'Hover a strike for values'
      : 'At strike ' + fmtLevel(m.x[idx]) + ' (₹ cr)');
    box.appendChild(caption);
    m.series.forEach(function (s) {
      var item = el(doc, 'span', 'legend-item');
      item.appendChild(el(doc, 'span', 'swatch s' + s.slot));
      item.appendChild(el(doc, 'span', 'legend-key', s.key));
      if (idx !== null && idx !== undefined && idx < s.values.length) item.appendChild(el(doc, 'span', 'legend-val num', fmtNum(s.values[idx], 1)));
      box.appendChild(item);
    });
  };

  GexChart.prototype.renderMarkerKey = function () {
    var doc = this.doc, box = this.markerKey, m = this.model;
    box.textContent = '';
    m.markers.forEach(function (mk) {
      var li = el(doc, 'li', 'mk mk-' + mk.kind);
      var line = el(doc, 'span', 'mk-line' + (mk.slot ? ' s' + mk.slot : ''));
      li.appendChild(line);
      li.appendChild(el(doc, 'span', 'mk-label', mk.label));
      li.appendChild(el(doc, 'span', 'mk-val num', mk.kind === 'F' ? fmtNum(mk.x, 2) : fmtLevel(mk.x)));
      box.appendChild(li);
    });
    var absent = [];
    m.series.forEach(function (s) {
      if (!m.markers.some(function (mk) { return mk.kind === 'flip' && mk.key === s.key; })) absent.push(s.key + ' flip');
      if (!m.markers.some(function (mk) { return mk.kind === 'pin' && mk.key === s.key; })) absent.push(s.key + ' pin');
    });
    if (absent.length) box.appendChild(el(doc, 'li', 'mk mk-none', 'none: ' + absent.join(', ')));
  };

  /** Wire the dashboard to the page. Returns { render(snapshot, meta), renderStatus(status), tick() }. */
  function createDashboard(doc, uPlot) {
    function $(id) { return doc.getElementById(id); }
    var chart = new GexChart(doc, uPlot, $('gex-chart'), $('gex-legend'), $('gex-markers'));
    var last = { snap: null, meta: null, status: null, renderError: null };

    function setText(id, t) { var e = $(id); if (e) e.textContent = t; }

    function renderHeader(vm) {
      setText('hdr-title', vm.header.title);
      setText('hdr-spot', vm.header.spotText);
      setText('hdr-forward', vm.header.forwardText);
      setText('hdr-basis', vm.header.basisText);
      setText('hdr-minutes', vm.header.minutesText);
      setText('hdr-tyears', vm.header.tYearsText);
      setText('hdr-ts', vm.tsText);
      setText('hdr-schema', 'schema ' + vm.schema);
      var box = $('hdr-regimes');
      box.textContent = '';
      vm.header.regimes.forEach(function (r) {
        var b = el(doc, 'span', 'badge regime ' + r.regime.cls);
        b.appendChild(el(doc, 'span', 'swatch s' + r.slot));
        b.appendChild(el(doc, 'span', null, r.key + ' ' + r.regime.icon + ' ' + r.regime.short));
        b.title = r.key + ': ' + r.regime.label;
        box.appendChild(b);
      });
    }

    function cell(label, cls) {
      var td = el(doc, 'td', cls);
      td.setAttribute('data-label', label);
      return td;
    }

    function renderTable(vm) {
      var tb = $('conv-rows');
      tb.textContent = '';
      vm.conventions.forEach(function (c) {
        var tr = el(doc, 'tr');
        var th = el(doc, 'th');
        th.setAttribute('scope', 'row');
        var name = el(doc, 'div', 'conv-name');
        name.appendChild(el(doc, 'span', 'swatch s' + c.slot));
        name.appendChild(el(doc, 'span', 'conv-key', c.key));
        name.appendChild(el(doc, 'span', 'conv-desc', c.desc));
        th.appendChild(name);
        tr.appendChild(th);

        var net = cell('Net GEX (₹ cr)', 'num');
        net.textContent = c.netGexText;
        tr.appendChild(net);

        var flip = cell('Flip', 'num');
        flip.appendChild(el(doc, 'div', 'fig', c.flipText));
        flip.appendChild(el(doc, 'div', 'note', c.flipNote));
        tr.appendChild(flip);

        var pin = cell('Pin (H1 prediction)', 'num');
        pin.appendChild(el(doc, 'div', 'fig', c.pinText));
        pin.appendChild(el(doc, 'div', 'note', c.pinNote));
        pin.appendChild(el(doc, 'div', 'acc' + (c.hasHistory ? '' : ' none'), c.accuracyText));
        tr.appendChild(pin);

        var reg = cell('Regime', '');
        var b = el(doc, 'span', 'badge regime ' + c.regime.cls, c.regime.icon + ' ' + c.regime.label);
        reg.appendChild(b);
        tr.appendChild(reg);
        tb.appendChild(tr);
      });
    }

    function renderHealth(vm) {
      var h = vm.health;
      setText('h-p50', h.p50Text);
      setText('h-p99', h.p99Text);
      $('h-p99').classList.toggle('warn', h.p99Warn);
      setText('h-gaps', h.gapsText);
      $('h-gaps').classList.toggle('warn', h.gapsWarn);
      setText('h-quotes', h.quotesText);
      setText('h-quotes-pct', h.quotesPctText);
      var bar = $('h-quotes-bar');
      bar.style.width = h.quotesFrac === null ? '0%' : Math.max(0, Math.min(100, h.quotesFrac * 100)).toFixed(1) + '%';
      var box = $('h-nodes');
      box.textContent = '';
      if (!h.nodes.length) box.appendChild(el(doc, 'span', 'note', 'no nodes reported'));
      h.nodes.forEach(function (n) {
        var b = el(doc, 'span', 'badge node ' + (n.up ? 'up' : 'down'), (n.up ? '● ' : '✖ ') + 'Node ' + n.name + ' ' + n.text);
        box.appendChild(b);
      });
    }

    function render(snap, meta) {
      last.snap = snap;
      last.meta = meta || {};
      try {
        var vm = buildViewModel(snap);
        renderHeader(vm);
        renderTable(vm);
        renderHealth(vm);
        chart.update(vm.chart);
        last.renderError = null;
      } catch (err) {
        // kept until a render succeeds, so tick() shows it instead of the feed status
        last.renderError = 'render error: ' + err.message;
        console.error('render failed', err);
      }
      tick();
    }

    function renderStatus(status) {
      last.status = status;
      tick();
    }

    /** Refresh the time-dependent bits (age, retry countdown). Call a few times a second. */
    function tick() {
      var st = last.status || {};
      var now = Date.now();
      var mock = st.mode === 'mock' || (last.meta && last.meta.origin === 'mock');
      doc.body.classList.toggle('is-mock', !!mock);
      var src = $('st-source');
      if (mock) src.textContent = 'MOCK DATA';
      else if (last.snap) src.textContent = last.snap.source === 'live' ? 'LIVE' : last.snap.source === 'replay' ? 'REPLAY' : String(last.snap.source).toUpperCase();
      else src.textContent = 'WAITING';
      src.className = 'pill src ' + (mock ? 'mock' : last.snap ? String(last.snap.source) : 'waiting');
      setText('st-mock-reason', mock && st.mockReason ? '(' + st.mockReason + ')' : '');

      var age = fmtAge(last.meta && last.meta.receivedAt ? now - last.meta.receivedAt : null);
      var ageEl = $('st-age');
      ageEl.textContent = last.snap ? age.text : 'no data yet';
      ageEl.classList.toggle('warn', !!last.snap && age.stale);

      var ws = st.wsState || 'connecting';
      var wsText = ws;
      if (ws === 'closed' && st.retryAt) wsText = 'closed, retry in ' + Math.max(0, Math.ceil((st.retryAt - now) / 1000)) + ' s';
      var wsEl = $('st-ws');
      wsEl.textContent = wsText;
      wsEl.className = 'ws-state ' + ws;
      setText('st-error', last.renderError || st.lastError || '');
    }

    /** Rebuild the chart with the current theme's colours. */
    function retheme() {
      chart.destroy();
      if (last.snap) render(last.snap, last.meta);
    }

    return { render: render, renderStatus: renderStatus, tick: tick, retheme: retheme };
  }

  return {
    buildViewModel: buildViewModel,
    createDashboard: createDashboard
  };
});
