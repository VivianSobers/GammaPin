/*
 * GammaPin monitor feed (G5.1).
 *
 * Connects to ws(s)://<host>/ws, validates each C2 snapshot (docs/CONTRACTS.md)
 * and hands it to a render callback. Falls back to a mock feed built from
 * mock/snapshot.json when the URL has ?mock, or when /ws has not delivered a
 * snapshot shortly after load (e.g. the page is opened from a static server).
 *
 * Classic script: in the browser it defines window.GPFeed; under Node it
 * exports the same object so the pure helpers can be tested without a DOM.
 */
(function (root, factory) {
  'use strict';
  var api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.GPFeed = api;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var SCHEMA_VERSION = 1;
  var MOCK_FALLBACK_MS = 2500; // no valid /ws snapshot by then -> mock
  var MOCK_INTERVAL_MS = 1000;
  var BACKOFF_BASE_MS = 500;
  var BACKOFF_MAX_MS = 30000;

  // ---------------------------------------------------------------- pure helpers

  function isNum(x) { return typeof x === 'number' && isFinite(x); }
  function isInt(x) { return isNum(x) && Math.floor(x) === x; }
  function isObj(x) { return x !== null && typeof x === 'object' && !Array.isArray(x); }
  function isNumOrNull(x) { return x === null || isNum(x); }

  /**
   * Validate a parsed C2 snapshot. Returns { ok, errors[] }.
   * Rejects anything the panels could not render; accepts the documented
   * nullables (flip, pin, h1_accuracy) and a missing S1/S2.
   */
  function validateSnapshot(s) {
    var e = [];
    if (!isObj(s)) return { ok: false, errors: ['snapshot is not an object'] };
    if (s.v !== SCHEMA_VERSION) e.push('v must be ' + SCHEMA_VERSION + ' (got ' + JSON.stringify(s.v) + ')');
    if (!isInt(s.ts_ms)) e.push('ts_ms must be an integer');
    if (typeof s.source !== 'string') e.push('source must be a string');
    if (typeof s.underlying !== 'string') e.push('underlying must be a string');
    if (typeof s.expiry !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(s.expiry)) e.push('expiry must be YYYY-MM-DD');
    ['spot', 'forward', 't_years'].forEach(function (k) {
      if (!isNum(s[k])) e.push(k + ' must be a number');
    });

    var nStrikes = -1;
    if (!Array.isArray(s.strikes) || !s.strikes.every(isNum)) {
      e.push('strikes must be a number[]');
    } else {
      nStrikes = s.strikes.length;
      for (var i = 1; i < nStrikes; i++) {
        if (!(s.strikes[i] > s.strikes[i - 1])) { e.push('strikes must be ascending'); break; }
      }
    }

    if (!isObj(s.conventions) || Object.keys(s.conventions).length === 0) {
      e.push('conventions must be a non-empty object');
    } else {
      Object.keys(s.conventions).forEach(function (k) {
        var c = s.conventions[k], p = 'conventions.' + k;
        if (!isObj(c)) { e.push(p + ' must be an object'); return; }
        if (!Array.isArray(c.gex) || !c.gex.every(isNum)) e.push(p + '.gex must be a number[]');
        else if (nStrikes >= 0 && c.gex.length !== nStrikes) e.push(p + '.gex length ' + c.gex.length + ' != strikes length ' + nStrikes);
        if (!isNum(c.net_gex)) e.push(p + '.net_gex must be a number');
        if (!isNumOrNull(c.flip)) e.push(p + '.flip must be a number or null');
        if (!isNumOrNull(c.pin)) e.push(p + '.pin must be a number or null');
        if (c.regime !== 'long_gamma' && c.regime !== 'short_gamma') e.push(p + '.regime must be long_gamma or short_gamma');
      });
    }

    if (s.h1_accuracy !== null) {
      if (!isObj(s.h1_accuracy)) e.push('h1_accuracy must be an object or null');
      else Object.keys(s.h1_accuracy).forEach(function (k) {
        var a = s.h1_accuracy[k];
        // A single convention may be null (no history for it yet).
        if (a === null) return;
        if (!isObj(a) || !isNum(a.mae) || !isInt(a.n)) e.push('h1_accuracy.' + k + ' must be {mae: number, n: int}');
      });
    }

    var h = s.health;
    if (!isObj(h)) e.push('health must be an object');
    else {
      ['lag_ms_p50', 'lag_ms_p99'].forEach(function (k) { if (!isNum(h[k])) e.push('health.' + k + ' must be a number'); });
      ['gaps_today', 'quotes_used', 'quotes_total'].forEach(function (k) { if (!isInt(h[k])) e.push('health.' + k + ' must be an integer'); });
      if (!isObj(h.nodes)) e.push('health.nodes must be an object');
      else Object.keys(h.nodes).forEach(function (k) {
        if (h.nodes[k] !== 'up' && h.nodes[k] !== 'down') e.push('health.nodes.' + k + ' must be "up" or "down"');
      });
    }
    return { ok: e.length === 0, errors: e };
  }

  /** Parse a WebSocket text frame. Returns { ok, snapshot } or { ok:false, errors }. */
  function parseMessage(text) {
    var obj;
    try { obj = JSON.parse(text); } catch (err) { return { ok: false, errors: ['invalid JSON: ' + err.message] }; }
    var r = validateSnapshot(obj);
    return r.ok ? { ok: true, snapshot: obj } : { ok: false, errors: r.errors };
  }

  /** Reconnect delay for the n-th consecutive failure (0-based): exponential, capped, with jitter. */
  function backoffDelay(attempt, rand) {
    var r = typeof rand === 'function' ? rand() : Math.random();
    var base = Math.min(BACKOFF_MAX_MS, BACKOFF_BASE_MS * Math.pow(2, Math.max(0, attempt)));
    return Math.round(base * (0.8 + 0.4 * r));
  }

  /** ws(s)://<host>/ws for a Location-like object. */
  function wsUrl(loc) {
    return (loc.protocol === 'https:' ? 'wss://' : 'ws://') + loc.host + '/ws';
  }

  /** True when the query string asks for mock mode (?mock, ?mock=1, &mock). */
  function isMockRequested(search) {
    return /(^|[?&])mock(=[^&]*)?($|&)/.test(search || '');
  }

  function clone(x) { return JSON.parse(JSON.stringify(x)); }

  /**
   * One mock tick: a copy of `base` with spot on a small random walk, forward
   * keeping its basis, and GEX scaled per strike by a few percent. net_gex,
   * regime and pin are recomputed so the snapshot stays self-consistent with
   * C2; flip is kept from the fixture. `state` carries the walk between ticks.
   */
  function jitterSnapshot(base, state, rand, nowMs) {
    rand = rand || Math.random;
    var s = clone(base);
    state.drift = (state.drift || 0) + (rand() - 0.5) * 6; // ~±3 pts per tick
    state.drift = Math.max(-40, Math.min(40, state.drift));
    s.spot = round2(base.spot + state.drift);
    s.forward = round2(base.forward + state.drift + (rand() - 0.5) * 0.5);
    s.ts_ms = typeof nowMs === 'number' ? nowMs : Date.now();
    Object.keys(s.conventions).forEach(function (k) {
      var c = s.conventions[k];
      c.gex = c.gex.map(function (g) { return Math.round(g * (1 + (rand() - 0.5) * 0.08)); });
      c.net_gex = c.gex.reduce(function (a, b) { return a + b; }, 0);
      c.regime = c.net_gex > 0 ? 'long_gamma' : 'short_gamma';
      var best = -1, bestVal = 0;
      c.gex.forEach(function (g, i) { if (g > bestVal) { bestVal = g; best = i; } });
      c.pin = best >= 0 ? s.strikes[best] : null;
    });
    if (s.health) {
      s.health.lag_ms_p50 = Math.max(1, Math.round(base.health.lag_ms_p50 * (0.8 + 0.4 * rand())));
      s.health.lag_ms_p99 = Math.max(s.health.lag_ms_p50, Math.round(base.health.lag_ms_p99 * (0.8 + 0.4 * rand())));
    }
    return s;
  }

  function round2(x) { return Math.round(x * 100) / 100; }

  // ---------------------------------------------------------------- browser transport

  /**
   * Start the feed. opts:
   *   onSnapshot(snapshot, meta)  meta = { origin: 'ws' | 'mock', receivedAt }
   *   onStatus(status)            status = { mode, wsState, retryInMs, rejected, lastError, mockReason }
   *   location (default window.location), fetchMock (default fetch of mock/snapshot.json)
   */
  function start(opts) {
    var loc = opts.location || window.location;
    var onSnapshot = opts.onSnapshot || function () {};
    var onStatus = opts.onStatus || function () {};
    var forcedMock = isMockRequested(loc.search);

    var status = {
      mode: forcedMock ? 'mock' : 'connecting', // 'ws' | 'mock' | 'connecting'
      mockReason: forcedMock ? '?mock in URL' : null,
      wsState: forcedMock ? 'disabled' : 'connecting', // connecting | open | closed | disabled | unsupported
      retryInMs: null,
      rejected: 0,
      lastError: null
    };
    function emitStatus() { onStatus(Object.assign({}, status)); }

    var ws = null, attempt = 0, retryTimer = null, gotWsSnapshot = false;
    var mockTimer = null, mockBase = null, mockState = {};

    function startMock(reason) {
      if (mockTimer || gotWsSnapshot) return;
      status.mode = 'mock';
      status.mockReason = reason;
      emitStatus();
      var load = mockBase ? Promise.resolve(mockBase) : fetch('mock/snapshot.json', { cache: 'no-store' })
        .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); });
      load.then(function (base) {
        var v = validateSnapshot(base);
        if (!v.ok) throw new Error('mock fixture invalid: ' + v.errors.join('; '));
        mockBase = base;
        if (status.mode !== 'mock' || mockTimer) return;
        var tick = function () { onSnapshot(jitterSnapshot(mockBase, mockState), { origin: 'mock', receivedAt: Date.now() }); };
        tick();
        mockTimer = setInterval(tick, MOCK_INTERVAL_MS);
      }).catch(function (err) {
        status.lastError = 'mock: ' + err.message;
        emitStatus();
      });
    }

    function stopMock() {
      if (mockTimer) { clearInterval(mockTimer); mockTimer = null; }
    }

    function connect() {
      retryTimer = null;
      status.retryInMs = null;
      status.wsState = 'connecting';
      emitStatus();
      try {
        ws = new WebSocket(wsUrl(loc));
      } catch (err) {
        status.lastError = 'ws: ' + err.message;
        scheduleReconnect();
        return;
      }
      ws.onopen = function () {
        status.wsState = 'open';
        status.lastError = null;
        emitStatus();
      };
      ws.onmessage = function (ev) {
        var r = parseMessage(typeof ev.data === 'string' ? ev.data : '');
        if (!r.ok) {
          status.rejected += 1;
          status.lastError = 'rejected: ' + r.errors.slice(0, 3).join('; ');
          emitStatus();
          return;
        }
        attempt = 0; // healthy stream resets the backoff
        if (!gotWsSnapshot || status.mode !== 'ws') {
          gotWsSnapshot = true;
          stopMock();
          status.mode = 'ws';
          status.mockReason = null;
          emitStatus();
        }
        onSnapshot(r.snapshot, { origin: 'ws', receivedAt: Date.now() });
      };
      ws.onerror = function () { status.lastError = 'ws: connection error'; };
      ws.onclose = function () {
        ws = null;
        scheduleReconnect();
      };
    }

    function scheduleReconnect() {
      var d = backoffDelay(attempt++);
      status.wsState = 'closed';
      status.retryInMs = d;
      status.retryAt = Date.now() + d;
      emitStatus();
      retryTimer = setTimeout(connect, d);
    }

    if (forcedMock) {
      emitStatus();
      startMock('?mock in URL');
    } else if (typeof WebSocket === 'undefined') {
      status.wsState = 'unsupported';
      emitStatus();
      startMock('WebSocket unsupported');
    } else {
      connect();
      setTimeout(function () {
        if (!gotWsSnapshot) startMock('no /ws snapshot after ' + (MOCK_FALLBACK_MS / 1000) + ' s');
      }, MOCK_FALLBACK_MS);
    }

    return {
      stop: function () {
        stopMock();
        if (retryTimer) clearTimeout(retryTimer);
        if (ws) { ws.onclose = null; ws.close(); }
      }
    };
  }

  return {
    SCHEMA_VERSION: SCHEMA_VERSION,
    validateSnapshot: validateSnapshot,
    parseMessage: parseMessage,
    backoffDelay: backoffDelay,
    wsUrl: wsUrl,
    isMockRequested: isMockRequested,
    jitterSnapshot: jitterSnapshot,
    start: start
  };
});
