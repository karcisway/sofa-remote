'use strict';
// Sofa Remote client: renders widget layouts and talks to one or more remote.py servers over WebSocket.

// ------------------------------------------------------------------ helpers

const $ = (sel, el = document) => el.querySelector(sel);
function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'style') el.style.cssText = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (typeof v !== 'string' && k in el) el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of kids.flat()) if (c != null && c !== false) el.append(c.nodeType ? c : String(c));
  return el;
}
const store = {
  get(k, d) { try { const v = localStorage.getItem(k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch {} },
};
const vibrate = ms => { try { navigator.vibrate && navigator.vibrate(ms); } catch {} };
const sleep = ms => new Promise(r => setTimeout(r, ms));
const asList = v => (Array.isArray(v) ? v : [v]);
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const rid = () => Math.random().toString(36).slice(2, 9);

// Themes (colour sets in app.css, [data-theme]); the choice is stored on this phone.
const THEMES = [
  { id: 'night', name: 'Night', dark: true, colors: ['#0e0f13', '#1a1c23', '#4f8cff'] },
  { id: 'light', name: 'Light', colors: ['#f2f3f7', '#ffffff', '#3d7bff'] },
  { id: 'bubblegum', name: 'Bubblegum', colors: ['#fff0f6', '#ffd9ea', '#ff4f9a'] },
  { id: 'grape', name: 'Grape', dark: true, colors: ['#15101d', '#322447', '#b57bff'] },
  { id: 'mint', name: 'Mint', colors: ['#ecf9f3', '#d2f1e3', '#14a874'] },
];
function applyTheme(id) {
  const theme = THEMES.find(t => t.id === id) || THEMES[0];
  if (theme.id === 'night') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme.id;
  document.querySelector('meta[name=theme-color]')?.setAttribute('content', theme.colors[0]);
  return theme.id;
}
applyTheme(store.get('theme', 'night'));

// Icons: "lucide:play" is a Lucide icon from icons.svg (https://lucide.dev, ~1900 of them);
// anything else (emoji, "F5", "Esc") is shown as text.
const ICON_PREFIX = 'lucide:';
// The sprite goes into the page once and icons point into it (#i-name): a <use> of icons.svg#name
// made the browser look the file up again on every render, so icons blinked on each page change.
// Ids get "i-" so they can't clash with the page's own (#sheet, #view). Icons drawn before it
// arrives fill in by themselves.
fetch('icons.svg').then(r => r.text()).then(svg => {
  const box = h('div', { 'aria-hidden': 'true', style: 'position:absolute;width:0;height:0;overflow:hidden' });
  box.innerHTML = svg;
  for (const s of box.querySelectorAll('symbol')) s.id = 'i-' + s.id;
  document.body.prepend(box);
});
function svgIcon(name) {
  const NS = 'http://www.w3.org/2000/svg', svg = document.createElementNS(NS, 'svg'), use = document.createElementNS(NS, 'use');
  const attrs = { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': 2,
    'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true' };
  for (const [k, v] of Object.entries(attrs)) svg.setAttribute(k, v);
  use.setAttribute('href', '#i-' + name);
  svg.append(use);
  return svg;
}
function iconEl(spec, cls = 'ico') {
  const s = spec == null ? '' : String(spec);
  if (s.startsWith(ICON_PREFIX)) return h('span', { class: cls + ' svg-ico' }, svgIcon(s.slice(ICON_PREFIX.length)));
  return h('span', { class: cls }, s);
}

// ------------------------------------------------------------------ servers

// Binary fast path (see src/session.rs): [type u8][seq u16 LE][payload]
const B = { MOVE: 1, KEY_DOWN: 2, KEY_UP: 3, COMBO: 4, BTN_DOWN: 5, BTN_UP: 6, CLICK: 7, SCROLL: 8, PING: 9, RELEASE: 10, MOVE_TOTAL: 11, PAD: 12 };
const BUTTONS = { left: 0, right: 1, middle: 2 };
const CLICKS = { click: 0, dblclick: 1, rclick: 2, mclick: 3 };
// Virtual gamepad buttons (XInput bits, see src/gamepad.rs); lt/rt are the triggers
const PAD_BUTTONS = { up: 0x1, down: 0x2, left: 0x4, right: 0x8, start: 0x10, back: 0x20, ls: 0x40, rs: 0x80,
  lb: 0x100, rb: 0x200, guide: 0x400, a: 0x1000, b: 0x2000, x: 0x4000, y: 0x8000 };
const newPad = () => ({ buttons: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 });
// This phone's id: the PC keeps its virtual gamepad for it across reconnects
const CLIENT_ID = store.get('clientId', null) || (() => { const id = rid() + rid(); store.set('clientId', id); return id; })();
const ALIASES = { return: 'enter', escape: 'esc', control: 'ctrl', meta: 'win', cmd: 'win', pageup: 'pgup',
  pagedown: 'pgdn', delete: 'del', insert: 'ins', '=': 'plus', '-': 'minus', ',': 'comma', '.': 'period',
  '/': 'slash', ';': 'semicolon', "'": 'quote', '[': 'bracketleft', ']': 'bracketright', '\\': 'backslash',
  '`': 'backquote', ' ': 'space' };

class Server {
  constructor({ id, url, key, name }) {
    Object.assign(this, { id, url, key, name });
    this.config = null;
    this.latency = null;
    this.samples = [];
    this.pending = new Map();
    this.pings = new Map();
    this.seq = 0;
    this.bseq = 0;
    this.codes = null;      // key name -> code, from the server's hello
    this.dc = null;         // open WebRTC data channel, when in use
    this.retry = 500;
    this.closed = false;
    this.connect();
  }
  get label() { return this.name || new URL(this.url).host; }
  get online() { return !!this.ws && this.ws.readyState === 1; }
  get transport() { return this.dc ? 'WebRTC' : 'WebSocket'; }

  connect() {
    if (this.closed) return;
    const ws = new WebSocket(this.url.replace(/^http/, 'ws') + '/ws?k=' + encodeURIComponent(this.key) + '&c=' + CLIENT_ID);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    ws.onopen = () => {
      this.retry = 500;
      this.mx = this.my = 0;   // the server's move totals start at 0 for each connection
      this.pad = newPad();
      this.padOpened = false;
      this.padOpening = false;
      this.padRetryAt = 0;
      this.request({ t: 'config_get' }).then(r => this.setConfig(r.config)).catch(() => {});
      app.refreshStatus();
    };
    ws.onmessage = e => {
      if (typeof e.data !== 'string') return this.onBinary(e.data);
      try { this.onMessage(JSON.parse(e.data)); } catch {}
    };
    ws.onclose = () => {
      if (this.ws === ws) this.ws = null;
      this.stopRtc();
      this.latency = null;
      this.samples = [];
      for (const p of this.pending.values()) p.reject(new Error('disconnected'));
      this.pending.clear();
      app.refreshStatus();
      app.renderTabs();
      if (!this.closed) setTimeout(() => this.connect(), this.retry);
      this.retry = Math.min(this.retry * 2, 5000);
    };
  }
  close() { this.closed = true; this.stopRtc(); if (this.ws) this.ws.close(); }

  // ---- WebRTC: unordered, no-retransmit data channel for inputs (UDP: no TCP stalls on packet loss)
  async startRtc() {
    if (this.pc || !this.online || !this.rtcAvailable || store.get('transport', 'auto') === 'ws') return;
    const pc = new RTCPeerConnection({ iceServers: [] });
    this.pc = pc;
    const dc = pc.createDataChannel('input', { ordered: false, maxRetransmits: 0 });
    dc.binaryType = 'arraybuffer';
    dc.onopen = () => { if (this.pc === pc) { this.dc = dc; this.samples = []; app.refreshStatus(); } };
    dc.onclose = () => { if (this.dc === dc) { this.dc = null; app.refreshStatus(); } };
    dc.onmessage = e => this.onBinary(e.data);
    pc.onconnectionstatechange = () => { if (['failed', 'closed'].includes(pc.connectionState) && this.pc === pc) this.stopRtc(); };
    try {
      await pc.setLocalDescription(await pc.createOffer());
      await new Promise(res => {   // gather all candidates first (no trickle ICE)
        if (pc.iceGatheringState === 'complete') return res();
        pc.addEventListener('icegatheringstatechange', () => pc.iceGatheringState === 'complete' && res());
        setTimeout(res, 1500);
      });
      const r = await this.request({ t: 'rtc_offer', sdp: pc.localDescription.sdp }, 10000);
      if (this.pc === pc) await pc.setRemoteDescription({ type: 'answer', sdp: r.sdp });
    } catch (e) {
      if (this.pc === pc) this.stopRtc();
    }
  }
  stopRtc() {
    if (this.pc) try { this.pc.close(); } catch {}
    this.pc = null;
    this.dc = null;
  }

  onMessage(m) {
    if (m.t === 'hello') {
      this.name = m.name;
      this.codes = Array.isArray(m.keys) ? null : m.keys;
      Object.keys(m.keys || {}).forEach(k => app.keyNames.add(k));
      this.rtcAvailable = !!m.rtc;
      app.updateKeyList();
      app.saveServers();
      app.renderTabs();
      this.startRtc();
    } else if (m.t === 'reply' || m.t === 'pong') {
      const p = this.pending.get(m.id);
      if (!p) return;
      this.pending.delete(m.id);
      m.error ? p.reject(new Error(m.error)) : p.resolve(m);
    } else if (m.t === 'config') {
      this.setConfig(m.config);
    } else if (m.t === 'pad_driver') {
      this.padOpened = false;   // the next press plugs the controller in (or offers the install again)
      this.padRetryAt = 0;
      app.status(this.label + ': ' + (m.ok ? 'gamepad driver installed' : m.error), !m.ok, m.ok);
    } else if (m.t === 'focus') {
      app.onTextFocus(this, !!m.text);
    } else if (m.t === 'error') {
      app.status(this.label + ': ' + m.msg, true);
    }
  }
  setConfig(c) { this.config = c; app.onConfig(this); }

  send(m) {
    if (!this.online) return app.status(this.label + ' is offline', true);
    app.noteActivity();
    if (!this.sendBinary(m)) this.ws.send(JSON.stringify(m));
  }

  code(name) {
    if (!this.codes || name == null) return null;
    const k = String(name).toLowerCase();
    return this.codes[ALIASES[k] || k] ?? null;
  }
  // Input messages go as a few binary bytes; returns false for anything that needs JSON.
  sendBinary(m) {
    const codes = k => { const c = asList(k).map(x => this.code(x)); return c.includes(null) ? null : c; };
    let body;
    switch (m.t) {
      case 'key': body = codes(m.k); if (!body) return false; break;
      case 'combo': body = codes(m.keys); if (!body) return false; break;
      case 'down': case 'up': {
        const c = codes(m.keys); if (!c) return false;
        (m.t === 'down' ? c : [...c].reverse()).forEach(vk => this.packet(m.t === 'down' ? B.KEY_DOWN : B.KEY_UP, [vk], 'u16'));
        return true;
      }
      case 'mbtn': { const b = BUTTONS[m.b || 'left']; if (b == null) return false; return this.packet(m.down ? B.BTN_DOWN : B.BTN_UP, [b], 'u8'), true; }
      case 'release': this.pad = newPad(); return this.packet(B.RELEASE, [], 'u8'), true;
      case 'mouse':
        // moves: running totals, so a lost packet is made up by the next one (see session.rs)
        if (m.kind === 'move') { this.mx = (this.mx + m.dx) | 0; this.my = (this.my + m.dy) | 0; }
        if (m.kind === 'move' || m.kind === 'move_end') return this.packet(B.MOVE_TOTAL, [this.mx, this.my], 'i32', m.kind === 'move_end'), true;
        if (m.kind === 'scroll') return this.packet(B.SCROLL, [clamp(m.dy | 0, -32768, 32767), clamp(m.dx | 0, -32768, 32767)], 'i16'), true;
        if (m.kind in CLICKS) return this.packet(B.CLICK, [CLICKS[m.kind]], 'u8'), true;
        return false;
      default: return false;
    }
    // key press / shortcut: [n u8][codes u16...]
    const buf = new DataView(new ArrayBuffer(4 + body.length * 2));
    buf.setUint8(0, B.COMBO); buf.setUint16(1, this.nextSeq(), true); buf.setUint8(3, body.length);
    body.forEach((c, i) => buf.setUint16(4 + i * 2, c, true));
    this.transmit(buf.buffer, true);
    return true;
  }
  // Virtual gamepad: change some fields of the state and send all of it (see src/gamepad.rs)
  padSet(changes) {
    if (!this.online) return app.status(this.label + ' is offline', true);
    app.noteActivity();
    if (!this.padOpened && !this.padOpening && Date.now() >= this.padRetryAt) {
      this.padOpening = true;
      const ws = this.ws;
      this.request({ t: 'pad_open' }, 8000)
        .then(r => {
          if (this.ws !== ws) return;
          this.padOpened = true;
          // Input may have changed while Windows was creating the controller.
          this.padSet({});
          app.status(r.player ? `Gamepad ${r.player} connected on ${this.label}` : `Gamepad connected on ${this.label}`, false, true);
        })
        .catch(e => {
          if (this.ws !== ws) return;
          this.padOpened = false;
          this.padRetryAt = Date.now() + 2000;
          app.status(this.label + ': ' + e.message, true);
        })
        .finally(() => { if (this.ws === ws) this.padOpening = false; });
    }
    const p = Object.assign(this.pad, changes);
    const v = new DataView(new ArrayBuffer(15));
    v.setUint8(0, B.PAD); v.setUint16(1, this.nextSeq(), true);
    v.setUint16(3, p.buttons, true); v.setUint8(5, p.lt); v.setUint8(6, p.rt);
    [p.lx, p.ly, p.rx, p.ry].forEach((x, i) => v.setInt16(7 + i * 2, x, true));
    this.transmit(v.buffer, true);
  }
  padButtons(names, down) {
    const p = this.pad, c = {};
    for (const n of names) {
      const k = String(n).toLowerCase();
      if (k === 'lt' || k === 'rt') c[k] = down ? 255 : 0;
      else if (PAD_BUTTONS[k]) c.buttons = ((c.buttons ?? p.buttons) & ~PAD_BUTTONS[k]) | (down ? PAD_BUTTONS[k] : 0);
      else return app.status('Unknown gamepad button: ' + n, true);
    }
    this.padSet(c);
  }
  nextSeq() { this.bseq = (this.bseq % 65535) + 1; return this.bseq; }
  packet(type, values, kind, reliable = true) {
    const size = { u8: 1, u16: 2, i16: 2, i32: 4 }[kind];
    const v = new DataView(new ArrayBuffer(3 + values.length * size));
    v.setUint8(0, type); v.setUint16(1, this.nextSeq(), true);
    const set = { u8: (o, x) => v.setUint8(o, x), u16: (o, x) => v.setUint16(o, x, true), i16: (o, x) => v.setInt16(o, x, true), i32: (o, x) => v.setInt32(o, x, true) }[kind];
    values.forEach((x, i) => set(3 + i * size, x));
    this.transmit(v.buffer, reliable);
  }
  // Over WebRTC (lossy) every event that matters is sent 3 times; the server drops duplicates by seq.
  transmit(buf, reliable) {
    const dc = this.dc;
    if (dc && dc.readyState === 'open') {
      dc.send(buf);
      if (reliable) { setTimeout(() => dc.readyState === 'open' && dc.send(buf), 8); setTimeout(() => dc.readyState === 'open' && dc.send(buf), 40); }
    } else if (this.online) this.ws.send(buf);
  }
  onBinary(data) {
    const v = new DataView(data);
    if (v.byteLength >= 5 && v.getUint8(0) === B.PING) {
      const p = this.pings.get(v.getUint16(3, true));
      if (p) { this.pings.delete(v.getUint16(3, true)); p(); }
    }
  }
  request(m, timeout = 6000) {
    return new Promise((resolve, reject) => {
      if (!this.online) return reject(new Error(this.label + ' is offline'));
      const id = ++this.seq;
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ ...m, id }));
      setTimeout(() => { if (this.pending.delete(id)) reject(new Error('timeout')); }, timeout);
    });
  }
  // Round trip over the transport inputs actually use. Keeps the median of recent samples.
  ping(timeout = 3000) {
    if (!this.online || this.pinging) return Promise.resolve(null);   // skipped
    this.pinging = true;
    const id = this.nextSeq(), t = performance.now();
    return new Promise(resolve => {
      const done = ok => { this.pinging = false; resolve(ok); };
      this.pings.set(id, () => {
        const s = this.samples;
        s.push(performance.now() - t);
        if (s.length > 30) s.shift();
        const sorted = [...s].sort((a, b) => a - b), med = sorted[sorted.length >> 1];
        this.latency = med;
        this.stats = { min: sorted[0].toFixed(1), med: med.toFixed(1), max: sorted.at(-1).toFixed(1) };
        done(true);
      });
      setTimeout(() => { if (this.pings.delete(id)) done(false); }, timeout);
      const v = new DataView(new ArrayBuffer(5));
      v.setUint8(0, B.PING); v.setUint16(1, 0, true); v.setUint16(3, id, true);
      const dc = this.dc;
      if (dc && dc.readyState === 'open') dc.send(v.buffer); else this.ws.send(v.buffer);
    });
  }
}

function findServer(name) {
  return app.servers.find(s => s.name === name || s.id === name);
}
// Servers an action/widget targets: its own "server" field, or the layout's owner.
function targetsOf(item, srv) {
  const t = item && item.server;
  if (!t) return [srv];
  if (t === '*') return app.servers;
  return asList(t).map(findServer).filter(Boolean);
}

// ------------------------------------------------------------------ actions

// phase 'down' = finger pressed (or repeat tick), 'up' = released.
async function run(action, srv, phase = 'down') {
  if (action == null) return;
  if (typeof action === 'string') action = { key: action };
  const send = m => targetsOf(action, srv).forEach(s => s.send(m));

  if (action.hold != null) return send({ t: phase === 'down' ? 'down' : 'up', keys: asList(action.hold) });
  if (action.hold_mouse) return send({ t: 'mbtn', b: action.hold_mouse, down: phase === 'down' });
  if (action.pad != null) return targetsOf(action, srv).forEach(s => s.padButtons(asList(action.pad), phase === 'down'));
  if (phase !== 'down') return;

  if (action.key != null) send({ t: 'key', k: action.key });
  else if (action.keys) send({ t: 'combo', keys: asList(action.keys) });
  else if (action.down != null) send({ t: 'down', keys: asList(action.down) });
  else if (action.up != null) send({ t: 'up', keys: asList(action.up) });
  else if (action.text != null) send({ t: 'type', text: String(action.text) });
  else if (action.mouse) send({ t: 'mouse', kind: action.mouse });
  else if (action.scroll) send({ t: 'mouse', kind: 'scroll', dy: -Math.round(action.scroll * 120) });
  else if (action.move) send({ t: 'mouse', kind: 'move', dx: action.move[0] | 0, dy: action.move[1] | 0 });
  else if (action.open) send({ t: 'open', url: action.open });
  else if (action.power) {
    const what = POWER[action.power]?.text || action.power;
    for (const s of targetsOf(action, srv)) {
      s.request({ t: 'power', op: action.power })
        .then(() => app.status(`${what}: ${s.label}`, false, true))
        .catch(e => app.status(`${s.label}: ${e.message}`, true));
    }
  }
  else if (action.layout) app.showByName(action.layout);
  else if (action.seq) {
    for (const step of action.seq) {
      if (step && step.wait) await sleep(step.wait);
      else await run(typeof step === 'string' ? { key: step, server: action.server } : { server: action.server, ...step }, srv);
    }
  }
}

// Power actions (src/power.rs).
const POWER = {
  shutdown: { text: 'Shut down' },
  restart: { text: 'Restart' },
  hibernate: { text: 'Hibernate' },
  signout: { text: 'Sign out' },
  sleep: { text: 'Sleep' },
  lock: { text: 'Lock' },
  screenoff: { text: 'Screen off' },
};

// Every pressed control registers its release, so everything can be let go at once
const activeReleases = new Set();
function releaseEverything() {
  [...activeReleases].forEach(f => f());
  app.servers.forEach(s => s.online && s.send({ t: 'release' }));
}

// Press/release handling with multi-touch and optional hold-to-repeat.
// On a page that scrolls (#view.scrolls, see watchScroll) a touch only presses after SCROLL_WAIT ms
// or on release, so a finger that starts on a button can still scroll: the browser cancels the
// pointer when it takes the touch for scrolling, and nothing is sent. Pages that fit the screen
// (gamepads) press at once.
const SCROLL_WAIT = 120;
function pressable(el, fn, repeat) {
  let pid = null, pressed = false, t0 = 0, t1 = 0, t2 = 0;
  const press = () => {
    clearTimeout(t0);
    if (pid === null || pressed) return;
    pressed = true;
    try { el.setPointerCapture(pid); } catch {}
    activeReleases.add(end);
    vibrate(8);
    fn('down');
    if (repeat) t1 = setTimeout(() => (t2 = setInterval(() => fn('down'), typeof repeat === 'number' ? repeat : 110)), 400);
  };
  const end = () => {
    if (pid === null) return;
    pid = null;
    clearTimeout(t0); clearTimeout(t1); clearInterval(t2);
    el.classList.remove('down');
    activeReleases.delete(end);
    if (pressed) fn('up');
    pressed = false;
  };
  el.addEventListener('pointerdown', e => {
    if (pid !== null) return;
    pid = e.pointerId;
    el.classList.add('down');
    if (e.pointerType === 'touch' && el.closest('#view.scrolls')) t0 = setTimeout(press, SCROLL_WAIT);
    else { e.preventDefault(); press(); }
  });
  el.addEventListener('pointerup', e => {
    if (e.pointerId !== pid) return;
    press(); // a quick tap: press now, released just below
    end();
  });
  el.addEventListener('pointercancel', e => { if (e.pointerId === pid) end(); });
  el.addEventListener('lostpointercapture', e => { if (e.pointerId === pid && pressed) end(); });
  el.addEventListener('contextmenu', e => e.preventDefault());
}

// ------------------------------------------------------------------ speech to text

// Start listening; handlers get text(t, final), end() and error(msg). Returns a stop function,
// or null when this phone can't do it here. The Android app uses the phone's own recognizer
// (WebView has no Web Speech API); browsers only allow the microphone on https or localhost.
function listen({ text, end, error }) {
  const bridge = window.SofaAndroid;
  if (bridge?.startSpeech) {
    window.sofaSpeech = ev => {
      if (ev.type === 'partial' || ev.type === 'final') text(ev.text, ev.type === 'final');
      else if (ev.type === 'error') error(ev.text);
      else if (ev.type === 'end') end();
    };
    bridge.startSpeech(navigator.language || '');
    return () => bridge.stopSpeech();
  }
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!SR || !window.isSecureContext) return null;
  const rec = new SR();
  rec.lang = navigator.language || 'en-US';
  rec.interimResults = true;
  rec.continuous = false;
  rec.onresult = e => {
    const r = e.results[e.results.length - 1];
    text(r[0].transcript.trim(), r.isFinal);
  };
  rec.onerror = e => error(e.error === 'not-allowed' ? 'Microphone permission denied' : 'Voice input: ' + e.error);
  rec.onend = end;
  rec.start();
  return () => rec.stop();
}

// ------------------------------------------------------------------ widgets

const WIDGET_TYPES = {
  button: { name: 'Button', size: [3, 1] },
  joystick: { name: 'Joystick (keys, mouse or gamepad)', size: [6, 4] },
  touchpad: { name: 'Touchpad', size: [12, 3] },
  keyboard: { name: 'Keyboard input', size: [12, 1] },
  clipboard: { name: 'Clipboard', size: [12, 5] },
  lockscreen: { name: 'Lock-screen controls', size: [12, 1] },
  label: { name: 'Text label', size: [12, 1] },
};

const WIDGETS = {
  button(w, srv) {
    const el = h('button', { class: 'btn ' + (w.style || ''), style: w.color ? `background:${w.color}` : null },
      w.icon ? iconEl(w.icon) : null,
      w.label ? h('small', {}, w.label) : null);
    let action = typeof w.action === 'string' ? { key: w.action } : w.action;
    if (action && w.server && !action.server) action = { ...action, server: w.server };
    pressable(el, phase => run(action, srv, phase), w.repeat);
    return el;
  },

  label(w) {
    return h('div', { class: 'label' }, w.text || w.label || '');
  },

  joystick(w, srv) {
    const base = h('div', { class: 'base' }), knob = h('div', { class: 'knob' });
    const el = h('div', { class: 'stick' }, base, knob, h('span', { class: 'stick-label' }, w.label || ''));
    const keys = w.keys || { up: 'w', left: 'a', down: 's', right: 'd' };
    const mouseMode = w.mode === 'mouse', padMode = w.mode === 'pad';
    const [xField, yField] = w.stick === 'right' ? ['rx', 'ry'] : ['lx', 'ly'];
    const padStick = (x, y) => targetsOf(w, srv).forEach(s => s.padSet({ [xField]: Math.round(x * 32767), [yField]: Math.round(-y * 32767) }));
    const dead = w.deadzone ?? 0.3, speed = w.speed ?? 20;
    let pid = null, cx = 0, cy = 0, r = 1, nx = 0, ny = 0, raf = 0, ax = 0, ay = 0, held = new Set();

    const setHeld = next => {
      for (const d of held) if (!next.has(d) && keys[d]) run({ hold: keys[d], server: w.server }, srv, 'up');
      for (const d of next) if (!held.has(d) && keys[d]) run({ hold: keys[d], server: w.server }, srv, 'down');
      held = next;
    };
    const mouseLoop = () => {
      const mag = Math.hypot(nx, ny);
      ax += nx * speed * mag; ay += ny * speed * mag;    // quadratic response: fine aim near centre
      const dx = Math.trunc(ax), dy = Math.trunc(ay);
      if (dx || dy) { targetsOf(w, srv).forEach(s => s.send({ t: 'mouse', kind: 'move', dx, dy })); ax -= dx; ay -= dy; }
      raf = requestAnimationFrame(mouseLoop);
    };
    const end = () => {
      if (pid === null) return;
      pid = null; nx = ny = 0;
      el.classList.remove('active');
      cancelAnimationFrame(raf);
      activeReleases.delete(end);
      if (padMode) padStick(0, 0);
      else if (!mouseMode) setHeld(new Set());
    };
    el.addEventListener('pointerdown', e => {
      e.preventDefault();
      if (pid !== null) return;
      pid = e.pointerId;
      try { el.setPointerCapture(pid); } catch {}
      const rect = el.getBoundingClientRect();
      r = Math.min(rect.width, rect.height) * 0.32;
      cx = e.clientX - rect.left; cy = e.clientY - rect.top;   // floating stick: centre where the thumb lands
      Object.assign(base.style, { left: cx + 'px', top: cy + 'px', width: 2 * r + 'px', height: 2 * r + 'px' });
      Object.assign(knob.style, { left: cx + 'px', top: cy + 'px' });
      el.classList.add('active');
      activeReleases.add(end);
      ax = ay = 0;
      if (mouseMode) raf = requestAnimationFrame(mouseLoop);
    });
    el.addEventListener('pointermove', e => {
      if (e.pointerId !== pid) return;
      const rect = el.getBoundingClientRect();
      let dx = (e.clientX - rect.left - cx) / r, dy = (e.clientY - rect.top - cy) / r;
      const mag = Math.hypot(dx, dy);
      if (mag > 1) { dx /= mag; dy /= mag; }
      nx = dx; ny = dy;
      Object.assign(knob.style, { left: cx + dx * r + 'px', top: cy + dy * r + 'px' });
      if (padMode) padStick(dx, dy);
      else if (!mouseMode) {
        const next = new Set();
        if (dx > dead) next.add('right'); else if (dx < -dead) next.add('left');
        if (dy > dead) next.add('down'); else if (dy < -dead) next.add('up');
        setHeld(next);
      }
    });
    for (const ev of ['pointerup', 'pointercancel', 'lostpointercapture'])
      el.addEventListener(ev, e => { if (e.pointerId === pid) end(); });
    return el;
  },

  touchpad(w, srv) {
    const wheel = h('div', { class: 'wheel' }, 'scroll');
    const rbtn = h('button', { class: 'pad-rclick', title: 'Right click', 'aria-label': 'Right click' }, 'R');
    // its own button: kept out of the pad's drag handling
    rbtn.addEventListener('pointerdown', e => { e.stopPropagation(); e.preventDefault(); navigator.vibrate?.(10); targetsOf(w, srv).forEach(s => s.send({ t: 'mouse', kind: 'rclick' })); });
    const el = h('div', { class: 'pad' }, w.label ?? 'Drag to move · tap to click · hold for right click · tap then drag to hold the button', wheel, rbtn);
    const sens = w.sens ?? 2.2;
    const send = m => targetsOf(w, srv).forEach(s => s.send(m));
    let pid = null, lx = 0, ly = 0, lt = 0, speed = 0, moved = 0, ax = 0, ay = 0, scrolling = false, longPress = null, longDone = false;
    // Tap then touch again within TAP_MS: the left button goes down and stays down while you drag
    // (select, move windows, resize); lifting the finger lets it go. A quick second tap is a
    // double click. So a single tap's click waits TAP_MS to see whether a second touch follows.
    const TAP_MS = 220;
    let tapTimer = null, dragging = false;
    // Sent straight from the event: the browser already delivers moves once per frame,
    // so waiting for another animation frame would only add up to 16 ms.
    const flush = () => {
      const dx = Math.trunc(ax), dy = Math.trunc(ay);
      if (scrolling) { if (dy) { send({ t: 'mouse', kind: 'scroll', dy: -dy * 4 }); ay -= dy; } ax = 0; }
      else if (dx || dy) { send({ t: 'mouse', kind: 'move', dx, dy }); ax -= dx; ay -= dy; }
    };
    const cancelLong = () => { clearTimeout(longPress); longPress = null; };
    el.addEventListener('pointerdown', e => {
      if (pid !== null) return;
      pid = e.pointerId;
      try { el.setPointerCapture(pid); } catch {}
      lx = e.clientX; ly = e.clientY; lt = e.timeStamp; speed = 0; moved = 0; ax = ay = 0; longDone = false;
      scrolling = e.target === wheel;
      if (tapTimer && !scrolling) {
        clearTimeout(tapTimer); tapTimer = null;
        dragging = true;
        send({ t: 'mbtn', b: 'left', down: true });
        navigator.vibrate?.(10);
        return;
      }
      // hold still for half a second: right click
      if (!scrolling) longPress = setTimeout(() => {
        longPress = null;
        if (moved >= 10) return;
        longDone = true;
        send({ t: 'mouse', kind: 'rclick' });
        navigator.vibrate?.(25);
      }, 500);
    });
    el.addEventListener('pointermove', e => {
      if (e.pointerId !== pid) return;
      const dx = e.clientX - lx, dy = e.clientY - ly;
      const dt = Math.max(e.timeStamp - lt, 4);
      lx = e.clientX; ly = e.clientY; lt = e.timeStamp;
      moved += Math.abs(dx) + Math.abs(dy);
      if (moved >= 10) cancelLong();
      // Acceleration on a smoothed speed (px/ms): slow moves are precise, fast ones cover the
      // screen, and the gain changes gradually so the cursor doesn't stutter.
      speed = speed * 0.6 + Math.hypot(dx, dy) / dt * 0.4;
      const k = sens * clamp(0.75 + speed * 0.5, 0.75, 2.2);
      ax += dx * k; ay += dy * k;
      flush();
    });
    el.addEventListener('pointerup', e => {
      if (e.pointerId !== pid) return;
      cancelLong();
      if (!scrolling && moved >= 6) send({ t: 'mouse', kind: 'move_end' });   // resend the final position reliably
      if (dragging) {
        dragging = false;
        send({ t: 'mbtn', b: 'left', down: false });
        if (moved < 6) send({ t: 'mouse', kind: 'click' });   // that was a double tap: second click
      } else if (moved < 6 && !scrolling && !longDone) {
        tapTimer = setTimeout(() => { tapTimer = null; send({ t: 'mouse', kind: 'click' }); }, TAP_MS);
      }
      pid = null;
    });
    el.addEventListener('pointercancel', () => {
      cancelLong();
      if (dragging) { dragging = false; send({ t: 'mbtn', b: 'left', down: false }); }
      pid = null;
    });
    return el;
  },

  keyboard(w, srv) {
    const input = h('input', { autocomplete: 'off', autocorrect: 'off', autocapitalize: 'off', spellcheck: 'false',
      enterkeyhint: 'send', placeholder: w.label || 'Tap here and type: goes live to the PC' });
    const send = m => targetsOf(w, srv).forEach(s => s.send(m));
    let sent = '', stopSpeech = null, spokenBase = '';
    // Diff against what was already sent, so autocorrect and suggestions work
    input.addEventListener('input', () => {
      const now = input.value;
      let p = 0;
      while (p < sent.length && p < now.length && sent[p] === now[p]) p++;
      const backspaces = sent.length - p, text = now.slice(p);
      sent = now;
      if (backspaces || text) send({ t: 'type', text, backspaces });
      if (now.length > 300 && !stopSpeech) { input.value = ''; sent = ''; }
    });
    const submit = () => { stopSpeech?.(); send({ t: 'key', k: 'enter' }); input.value = ''; sent = ''; };
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter') { e.preventDefault(); submit(); }
      else if (e.key === 'Backspace' && input.value === '') { e.preventDefault(); send({ t: 'key', k: 'backspace' }); }
    });

    // Speech to text: words go into the field (and so live to the PC); partial results are
    // corrected as recognition goes, the same way keyboard autocorrect is.
    const mic = h('button', { type: 'button', class: 'kbd-btn', title: 'Speak', 'aria-label': 'Speak' }, iconEl('lucide:mic'));
    const spoken = text => {
      input.value = spokenBase + text;
      input.dispatchEvent(new Event('input'));
    };
    const stopped = () => { stopSpeech = null; mic.classList.remove('listening'); };
    mic.addEventListener('click', () => {
      if (stopSpeech) return stopSpeech();
      spokenBase = input.value && !input.value.endsWith(' ') ? input.value + ' ' : input.value;
      stopSpeech = listen({
        text: (t, final) => { spoken(t); if (final) spokenBase = input.value + ' '; },
        end: stopped,
        error: msg => { stopped(); app.status(msg, true); },
      });
      if (stopSpeech) mic.classList.add('listening');
      else { input.focus(); app.status('Voice input needs the Android app or an https page: use the 🎤 key on your phone keyboard instead', true); }
    });
    const enter = h('button', { type: 'button', class: 'kbd-btn accent', title: 'Send (Enter)', 'aria-label': 'Send (Enter)', onclick: submit }, iconEl('lucide:corner-down-left'));
    // keep the phone keyboard open when tapping the buttons
    for (const b of [mic, enter]) b.addEventListener('pointerdown', e => document.activeElement === input && e.preventDefault());
    return h('div', { class: 'kbd' }, input, mic, enter);
  },

  clipboard(w, srv) {
    const s = () => targetsOf(w, srv)[0] || srv;
    const ta = h('textarea', { placeholder: 'Paste text or a link here (long-press › Paste)' });
    const btn = (icon, label, fn, cls = '') => h('button', { class: 'btn ' + cls, onclick: fn }, h('span', { class: 'ico', style: 'font-size:16px' }, icon), h('small', {}, label));
    const guard = async fn => { try { await fn(); } catch (e) { app.status(e.message, true); } };
    return h('div', { class: 'clip' }, ta,
      h('div', { class: 'row3' },
        btn('Open', 'in PC browser', () => guard(async () => {
          if (!ta.value.trim()) throw new Error('nothing to open');
          const r = await s().request({ t: 'open', url: ta.value });
          app.status('opened ' + r.url, false, true);
        }), 'accent'),
        btn('Paste', 'on PC', () => guard(() => s().request({ t: 'clip_set', text: ta.value, paste: true }))),
        btn('Type', 'on PC', () => s().send({ t: 'type', text: ta.value }))),
      h('div', { class: 'row3' },
        btn('Copy', 'to PC clipboard', () => guard(async () => { await s().request({ t: 'clip_set', text: ta.value }); app.status('copied to PC', false, true); })),
        btn('Get PC clipboard', 'and copy to phone', () => guard(async () => {
          const r = await s().request({ t: 'clip_get' });
          ta.value = r.text;
          try { await navigator.clipboard.writeText(r.text); app.status('copied to phone', false, true); return; } catch {}
          ta.focus(); ta.select();
          app.status(document.execCommand('copy') ? 'copied to phone' : 'loaded: long-press to copy', false, true);
        }), 'span2')),
      h('p', { class: 'note' }, 'Open takes a link, a bare address (example.com) or words to search. Paste sets the PC clipboard and presses Ctrl+V; Type types it key by key, for fields that block pasting.'));
  },

  lockscreen(w, srv) {
    const el = h('button', { class: 'btn lock-btn', onclick: () => toggleMedia(srv, w) });
    updateLockButtons(el);
    return el;
  },
};

// Lock-screen / headset controls: a near-silent looping audio makes the phone show a media notification
const media = { audio: null };
function silentWav(seconds = 10, rate = 8000) {
  const n = seconds * rate, buf = new ArrayBuffer(44 + n), v = new DataView(buf);
  const s = (o, str) => [...str].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  s(0, 'RIFF'); v.setUint32(4, 36 + n, true); s(8, 'WAVE'); s(12, 'fmt ');
  v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate, true); v.setUint16(32, 1, true); v.setUint16(34, 8, true);
  s(36, 'data'); v.setUint32(40, n, true);
  for (let i = 0; i < n; i++) v.setUint8(44 + i, 128 + (i % 2)); // inaudible, but not "digital silence"
  return URL.createObjectURL(new Blob([buf], { type: 'audio/wav' }));
}
function updateLockButtons(only) {
  const on = !!media.audio;
  for (const b of only ? [only] : document.querySelectorAll('.lock-btn')) {
    b.classList.toggle('on', on);
    b.replaceChildren(h('span', { class: 'ico', style: 'font-size:15px' }, on ? 'Lock-screen controls ON' : 'Enable lock-screen & headset controls'),
      h('small', {}, on ? 'tap to disable' : 'phone volume keys can’t be read by web pages; this is the closest thing'));
  }
}
async function toggleMedia(srv, w) {
  if (media.audio) { media.audio.pause(); media.audio = null; return updateLockButtons(); }
  const audio = new Audio(silentWav());
  audio.loop = true;
  try { await audio.play(); } catch (e) { return app.status('audio blocked: ' + e.message, true); }
  media.audio = audio;
  updateLockButtons();
  if (!('mediaSession' in navigator)) return app.status('no Media Session support', true);
  const ms = navigator.mediaSession, act = a => run({ key: a, server: w.server }, srv);
  ms.metadata = new MediaMetadata({ title: 'Sofa Remote', artist: 'Controlling ' + srv.label });
  const set = (a, f) => { try { ms.setActionHandler(a, f); } catch {} };
  set('play', () => { media.audio && media.audio.play(); ms.playbackState = 'playing'; act('playpause'); });
  set('pause', () => { media.audio && media.audio.pause(); ms.playbackState = 'paused'; act('playpause'); });
  set('nexttrack', () => act('next'));
  set('previoustrack', () => act('prev'));
  set('seekforward', () => act('right'));
  set('seekbackward', () => act('left'));
  ms.playbackState = 'playing';
}

// ------------------------------------------------------------------ layout rendering

// Marks the view .scrolls while its content is taller than the screen, so buttons there let the
// finger scroll (see pressable and app.css)
const scrollWatch = new ResizeObserver(([e]) => {
  const view = e.target.closest('#view') || e.target;
  view.classList.toggle('scrolls', view.scrollHeight > view.clientHeight + 1);
});
function watchScroll(view) {
  scrollWatch.disconnect();
  scrollWatch.observe(view);
  if (view.firstElementChild) scrollWatch.observe(view.firstElementChild);
}

function place(box, w) {
  box.style.gridColumn = w.x != null ? `${w.x + 1} / span ${w.w || 1}` : `span ${w.w || 1}`;
  box.style.gridRow = w.y != null ? `${w.y + 1} / span ${w.h || 1}` : `span ${w.h || 1}`;
}

function renderLayout(srv, layout, editing) {
  const cols = layout.cols || 12;
  const grid = h('div', { class: 'grid' + (layout.fit ? ' fit' : '') });
  grid.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`;
  if (layout.fit) grid.style.gridTemplateRows = `repeat(${layout.rows || 6}, minmax(0, 1fr))`;
  else grid.style.gridAutoRows = (layout.rowHeight || 64) + 'px';
  grid.boxes = new Map();
  (layout.widgets || []).forEach(w => {
    const box = h('div', { class: 'w' });
    grid.boxes.set(w, box);
    place(box, w);
    const make = WIDGETS[w.type] || WIDGETS.label;
    box.append(make(w, srv));
    if (editing) makeEditable(box, w, grid, layout);
    grid.append(box);
  });
  if (!layout.widgets || !layout.widgets.length)
    grid.append(h('div', { class: 'placeholder', style: 'grid-column:1/-1' }, editing ? 'Empty layout: tap “+ Widget”.' : 'Empty layout: tap the pencil to add widgets.'));
  return grid;
}

// ------------------------------------------------------------------ editor

function cellSize(grid, layout) {
  const r = grid.getBoundingClientRect(), gap = 10, cols = layout.cols || 12, rows = layout.rows || 6;
  return {
    w: (r.width - gap * (cols - 1)) / cols + gap,
    h: layout.fit ? (r.height - gap * (rows - 1)) / rows + gap : (layout.rowHeight || 64) + gap,
  };
}

const rect = o => ({ x: o.x || 0, y: o.y || 0, w: o.w || 1, h: o.h || 1 });
const hits = (a, b) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;

// Keep `fixed` where it is (if given) and move every widget it would cover. Scrolling layouts
// also float everything up, so no empty rows are left behind; fit-screen layouts keep their free
// placement and move covered widgets to the nearest free cell (overlapping only if the screen is full).
function resolveOverlaps(layout, fixed = null) {
  if (layout.allowOverlap) return;
  const cols = layout.cols || 12, maxRows = layout.fit ? layout.rows || 6 : Infinity;
  const placed = fixed ? [rect(fixed)] : [];
  const free = r => !placed.some(p => hits(p, r));
  const others = (layout.widgets || []).filter(o => o !== fixed)
    .sort((a, b) => (a.y || 0) - (b.y || 0) || (a.x || 0) - (b.x || 0));
  const settle = (o, r) => { o.x = r.x; o.y = r.y; placed.push(r); };

  if (maxRows === Infinity) {
    // Scrolling layout: top to bottom, each widget goes to the highest row where it fits
    for (const o of others) {
      const r = { ...rect(o), y: 0 };
      while (!free(r)) r.y++;
      settle(o, r);
    }
    return;
  }
  // Fit-screen layout: widgets not in the way keep their place first, so displaced ones can't
  // steal their cells; displaced ones go to the nearest free spot (or stay put if the screen is full)
  const displaced = [];
  for (const o of others) {
    const r = rect(o);
    if (free(r)) placed.push(r); else displaced.push(o);
  }
  for (const o of displaced) {
    const r = rect(o), spots = [];
    for (let y = 0; y + r.h <= maxRows; y++)
      for (let x = 0; x + r.w <= cols; x++) if (free({ ...r, x, y })) spots.push({ x, y });
    spots.sort((a, b) => Math.hypot(a.x - r.x, a.y - r.y) - Math.hypot(b.x - r.x, b.y - r.y));
    if (spots.length) Object.assign(r, spots[0]);
    settle(o, r);
  }
}

function makeEditable(box, w, grid, layout) {
  const handle = h('div', { class: 'resize' });
  box.append(handle);
  const cols = layout.cols || 12;
  let start = null;
  box.addEventListener('pointerdown', e => {
    e.preventDefault();
    try { box.setPointerCapture(e.pointerId); } catch {}
    start = { x: e.clientX, y: e.clientY, orig: { x: w.x || 0, y: w.y || 0, w: w.w || 1, h: w.h || 1 },
              resize: e.target === handle, cell: cellSize(grid, layout), moved: false,
              others: (layout.widgets || []).map(o => [o, o.x || 0, o.y || 0]) };
  });
  box.addEventListener('pointermove', e => {
    if (!start) return;
    if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) > 8) start.moved = true;
    if (!start.moved) return;
    box.classList.add('dragging');
    const dc = Math.round((e.clientX - start.x) / start.cell.w), dr = Math.round((e.clientY - start.y) / start.cell.h);
    const o = start.orig, maxRows = layout.fit ? layout.rows || 6 : 999;
    if (start.resize) {
      w.w = clamp(o.w + dc, 1, cols - o.x);
      w.h = clamp(o.h + dr, 1, maxRows - o.y);
    } else {
      w.x = clamp(o.x + dc, 0, cols - o.w);
      w.y = clamp(o.y + dr, 0, maxRows - o.h);
    }
    // Start from where the others were before this drag, so they slide back when space frees up
    for (const [other, x, y] of start.others) if (other !== w) { other.x = x; other.y = y; }
    resolveOverlaps(layout, w);
    for (const [other, b] of grid.boxes) place(b, other);
  });
  box.addEventListener('pointerup', () => {
    if (start && !start.moved) openWidgetSheet(layout, w);
    else if (start) {   // dropped: the moved widget floats up too
      resolveOverlaps(layout);
      for (const [other, b] of grid.boxes) place(b, other);
    }
    start = null;
    box.classList.remove('dragging');
  });
}

function sheet(title, body, buttons) {
  const dlg = $('#sheet');
  dlg.replaceChildren(h('form', { method: 'dialog', onsubmit: e => e.preventDefault() },
    h('h3', {}, title), body, h('div', { class: 'sheet-buttons' }, buttons)));
  dlg.showModal();
  return dlg;
}
const closeSheet = () => $('#sheet').close();
const field = (label, input) => h('label', {}, label, input);
const check = (label, input) => h('label', { class: 'check' }, input, label);
const sheetBtn = (text, onclick, cls) => h('button', { type: 'button', class: cls, onclick }, text);

const ACTION_KINDS = [
  ['key', 'Press key', 'playpause, space, f, volup, enter…'],
  ['keys', 'Shortcut (keys together)', 'ctrl+shift+t   (use “plus” for the + key)'],
  ['hold', 'Hold key(s) while pressed (games)', 'w   or   shift+w'],
  ['text', 'Type text', 'Hello! (\\n = Enter)'],
  ['mouse', 'Mouse click', 'click, dblclick, rclick, mclick'],
  ['hold_mouse', 'Hold mouse button (games)', 'left, right, middle'],
  ['pad', 'Gamepad button (virtual controller)', 'a, b, x, y, lb, rb, lt, rt, start, back, guide, ls, rs, up, down, left, right'],
  ['scroll', 'Scroll (notches, − = up)', '3   or   -3'],
  ['open', 'Open URL / search on PC', 'https://youtube.com'],
  ['power', 'Power (shut down, sleep…)', 'shutdown, restart, sleep, hibernate, lock, signout, screenoff'],
  ['layout', 'Switch to tab', 'Gamepad'],
  ['custom', 'Custom JSON (macros, seq…)', '{"seq": [{"keys": ["ctrl","l"]}, {"wait": 100}, {"text": "netflix.com\\n"}]}'],
];

function actionToFields(a) {
  if (a == null) return ['key', ''];
  if (typeof a === 'string') return ['key', a];
  const simple = Object.keys(a).filter(k => k !== 'server');
  if (simple.length === 1) {
    const k = simple[0], v = a[k];
    if (['key', 'text', 'mouse', 'hold_mouse', 'open', 'power', 'layout'].includes(k)) return [k, String(v).replace(/\n/g, '\\n')];
    if (k === 'keys' || k === 'hold' || k === 'pad') return [k, asList(v).join('+')];
    if (k === 'scroll') return [k, String(v)];
  }
  const { server, ...rest } = a;
  return ['custom', JSON.stringify(rest)];
}
function fieldsToAction(kind, value, server) {
  let a;
  if (kind === 'custom') a = JSON.parse(value || '{}');
  else if (kind === 'keys' || kind === 'hold' || kind === 'pad') {
    const parts = value.split('+').map(s => s.trim()).filter(Boolean);
    a = { [kind]: kind !== 'keys' && parts.length === 1 ? parts[0] : parts };
  } else if (kind === 'scroll') a = { scroll: Number(value) || 0 };
  else if (kind === 'text') a = { text: value.replace(/\\n/g, '\n') };
  else a = { [kind]: value.trim() };
  if (server) a.server = server === '*' ? '*' : server;
  return a;
}

function serverSelect(current, owner) {
  const sel = h('select', {},
    h('option', { value: '' }, `This layout’s PC (${owner.label})`),
    h('option', { value: '*', selected: current === '*' }, 'All connected PCs'),
    app.servers.filter(s => s !== owner).map(s => h('option', { value: s.name || s.id, selected: current === s.name || current === s.id }, s.label)));
  return sel;
}

// Icon field for the editor: free text (emoji, "F5") or a Lucide icon picked from a searchable grid.
let iconTags = null;   // icon name -> search words, loaded on first use
const loadIconTags = () => (iconTags ??= fetch('icon-tags.json').then(r => r.json()).catch(() => ({})));
const POPULAR_ICONS = ['play', 'pause', 'skip-back', 'skip-forward', 'rewind', 'fast-forward', 'volume-2', 'volume-1',
  'volume-x', 'mic', 'tv', 'monitor', 'maximize', 'power', 'moon', 'sun', 'house', 'search', 'gamepad-2', 'joystick',
  'crosshair', 'flame', 'zap', 'keyboard', 'mouse-pointer-click', 'arrow-up', 'arrow-down', 'arrow-left', 'arrow-right',
  'chevron-up', 'chevron-down', 'corner-down-left', 'delete', 'space', 'copy', 'clipboard-paste', 'undo-2', 'redo-2',
  'refresh-cw', 'x', 'check', 'heart', 'star', 'sparkles', 'music', 'film', 'camera', 'lock'];
function iconPicker(input) {
  const preview = h('span', { class: 'preview' });
  const showPreview = () => preview.replaceChildren(input.value ? iconEl(input.value) : '');
  const search = h('input', { type: 'search', placeholder: 'Search: play, volume, arrow, heart…', autocapitalize: 'off', autocomplete: 'off' });
  const grid = h('div', { class: 'icon-grid' });
  const panel = h('div', { class: 'icon-picker', hidden: true }, search, grid);
  const render = async () => {
    const tags = await loadIconTags(), q = search.value.trim().toLowerCase();
    let names = POPULAR_ICONS;
    if (q) {
      const rank = n => n === q ? 0 : n.startsWith(q) ? 1 : n.includes(q) ? 2 : tags[n].some(t => t.includes(q)) ? 3 : 9;
      names = Object.keys(tags).map(n => [rank(n), n]).filter(([r]) => r < 9).sort((a, b) => a[0] - b[0]).map(([, n]) => n);
    }
    const shown = names.slice(0, 120);
    grid.replaceChildren(...shown.map(n => h('button', { type: 'button', title: n, 'aria-label': n,
      class: input.value === ICON_PREFIX + n ? 'on' : null,
      onclick: () => { input.value = ICON_PREFIX + n; showPreview(); render(); } }, svgIcon(n))),
      ...(names.length > shown.length ? [h('div', { class: 'more' }, `${names.length - shown.length} more: type to narrow down`)] : []),
      ...(names.length ? [] : [h('div', { class: 'more' }, 'No icon found')]));
  };
  search.addEventListener('input', render);
  input.addEventListener('input', showPreview);
  showPreview();
  const toggle = h('button', { type: 'button', class: 'small', onclick: () => {
    panel.hidden = !panel.hidden;
    if (!panel.hidden) { render(); search.focus(); }
  } }, 'Icons');
  return h('div', {}, h('div', { class: 'icon-field' }, preview, input, toggle), panel);
}

function openWidgetSheet(layout, w, isNew = false) {
  const owner = app.edit.srv;
  const [kind0, value0] = actionToFields(w.action);
  const type = h('select', {}, Object.entries(WIDGET_TYPES).map(([k, t]) => h('option', { value: k, selected: k === (w.type || 'button') }, t.name)));
  const label = h('input', { value: w.label || '', placeholder: 'shown under the icon' });
  const icon = h('input', { value: w.icon || '', placeholder: 'pick one, or type text: F5' });
  const style = h('select', {}, ['', 'accent', 'dark'].map(s => h('option', { value: s, selected: (w.style || '') === s }, s || 'default')));
  const color = h('input', { value: w.color || '', placeholder: 'any CSS colour, e.g. #8a2be2' });
  const ww = h('input', { type: 'number', min: 1, max: layout.cols || 12, value: w.w || 1 });
  const wh = h('input', { type: 'number', min: 1, max: 40, value: w.h || 1 });
  const repeat = h('input', { type: 'checkbox', checked: !!w.repeat });
  const kind = h('select', {}, ACTION_KINDS.map(([k, n]) => h('option', { value: k, selected: k === kind0 }, n)));
  const value = h('input', { value: kind0 === 'custom' ? '' : value0, list: 'keylist', autocapitalize: 'off', autocorrect: 'off' });
  const custom = h('textarea', { class: 'code' }, kind0 === 'custom' ? value0 : '');
  const server = serverSelect(w.server, owner);
  const advInitial = JSON.stringify(w, null, 2);
  const adv = h('textarea', { class: 'code', style: 'min-height:180px' }, advInitial);

  const valueField = field('Value', value), customField = field('Action JSON', custom);
  const buttonFields = h('div', { style: 'display:contents' }, field('Icon', iconPicker(icon)), field('Style', style), field('Colour', color),
    field('Action', kind), valueField, customField, check('Repeat while held', repeat));
  const sync = () => {
    const isButton = type.value === 'button';
    buttonFields.style.display = isButton ? 'contents' : 'none';
    const hint = ACTION_KINDS.find(k => k[0] === kind.value);
    valueField.hidden = kind.value === 'custom';
    customField.hidden = kind.value !== 'custom';
    value.placeholder = hint ? hint[2] : '';
    if (kind.value === 'custom' && !custom.value) custom.value = hint[2];
  };
  type.addEventListener('change', () => {
    if (isNew) { const [sw, sh] = WIDGET_TYPES[type.value].size; ww.value = sw; wh.value = sh; }
    sync();
  });
  kind.addEventListener('change', sync);

  const body = h('div', { style: 'display:flex;flex-direction:column;gap:10px' },
    field('Type', type), field('Label', label), buttonFields,
    h('div', { class: 'pair' }, field('Width (columns)', ww), field('Height (rows)', wh)),
    field('Sends to', server),
    h('details', {}, h('summary', {}, 'Advanced: edit as JSON (joystick keys, speed, sensitivity…)'), adv));
  sync();

  const apply = () => {
    let out;
    try {
      if (adv.value.trim() !== advInitial.trim()) out = JSON.parse(adv.value);
      else {
        out = { ...w, type: type.value, label: label.value || undefined, w: +ww.value || 1, h: +wh.value || 1, server: server.value || undefined };
        if (type.value === 'button') {
          Object.assign(out, { icon: icon.value || undefined, style: style.value || undefined, color: color.value || undefined,
            repeat: repeat.checked || undefined, action: fieldsToAction(kind.value, kind.value === 'custom' ? custom.value : value.value) });
        }
        out = JSON.parse(JSON.stringify(out)); // drop undefined
      }
    } catch (e) { return app.status('Invalid JSON: ' + e.message, true); }
    const list = layout.widgets || (layout.widgets = []);
    const i = list.indexOf(w);
    if (i >= 0) list[i] = out; else list.push(out);
    resolveOverlaps(layout, out);
    resolveOverlaps(layout);
    closeSheet();
    app.render();
  };
  sheet(isNew ? 'New widget' : 'Edit widget', body, [
    isNew ? null : sheetBtn('Delete', () => {
      layout.widgets.splice(layout.widgets.indexOf(w), 1); resolveOverlaps(layout); closeSheet(); app.render();
    }, 'danger'),
    isNew ? null : sheetBtn('Duplicate', () => {
      const copy = { ...structuredClone(w), y: (w.y || 0) + (w.h || 1) };
      layout.widgets.push(copy); resolveOverlaps(layout, copy); resolveOverlaps(layout); closeSheet(); app.render();
    }),
    sheetBtn('Cancel', closeSheet),
    sheetBtn(isNew ? 'Add' : 'OK', apply, 'primary'),
  ]);
}

function openLayoutSheet(layout) {
  const { srv, draft } = app.edit;
  const name = h('input', { value: layout.name || '' });
  const icon = h('input', { value: layout.icon || '', placeholder: 'pick one, or an emoji' });
  const cols = h('input', { type: 'number', min: 1, max: 24, value: layout.cols || 12 });
  const rowHeight = h('input', { type: 'number', min: 30, max: 300, value: layout.rowHeight || 64 });
  const fit = h('input', { type: 'checkbox', checked: !!layout.fit });
  const rows = h('input', { type: 'number', min: 1, max: 24, value: layout.rows || 6 });
  const landscape = h('input', { type: 'checkbox', checked: !!layout.landscape });
  const overlap = h('input', { type: 'checkbox', checked: !!layout.allowOverlap });
  const idx = draft.layouts.indexOf(layout);
  const move = d => {
    const j = idx + d;
    if (j < 0 || j >= draft.layouts.length) return;
    [draft.layouts[idx], draft.layouts[j]] = [draft.layouts[j], draft.layouts[idx]];
    closeSheet(); app.render();
  };
  const body = h('div', { style: 'display:flex;flex-direction:column;gap:10px' },
    field('Name', name), field('Tab icon', iconPicker(icon)),
    h('div', { class: 'pair' }, field('Columns', cols), field('Row height (px)', rowHeight)),
    check('Fit the screen (fixed rows, no scrolling: for gamepads)', fit),
    field('Rows when fitting', rows),
    check('Landscape: rotate when going fullscreen', landscape),
    check('Allow widgets to overlap (otherwise moving one pushes the others aside)', overlap),
    h('div', { class: 'pair' }, sheetBtn('◀ Move tab left', () => move(-1), 'small'), sheetBtn('Move tab right ▶', () => move(1), 'small')),
    h('div', { class: 'pair' },
      sheetBtn('+ New blank layout', () => {
        const l = { id: rid(), name: 'New layout', cols: 12, rowHeight: 64, widgets: [] };
        draft.layouts.push(l); app.tab = `${srv.id}/${l.id}`; closeSheet(); app.render();
      }, 'small'),
      sheetBtn('Duplicate layout', () => {
        const l = { ...structuredClone(layout), id: rid(), name: (layout.name || 'Layout') + ' copy' };
        draft.layouts.splice(idx + 1, 0, l); app.tab = `${srv.id}/${l.id}`; closeSheet(); app.render();
      }, 'small')));
  sheet('Layout', body, [
    sheetBtn('Delete layout', () => {
      if (draft.layouts.length < 2) return app.status('Keep at least one layout', true);
      if (!confirm(`Delete “${layout.name}”?`)) return;
      draft.layouts.splice(idx, 1); app.tab = null; closeSheet(); app.render();
    }, 'danger'),
    sheetBtn('Cancel', closeSheet),
    sheetBtn('OK', () => {
      Object.assign(layout, { name: name.value || 'Layout', icon: icon.value || undefined, cols: +cols.value || 12, rowHeight: +rowHeight.value || 64,
        fit: fit.checked || undefined, rows: fit.checked ? +rows.value || 6 : undefined, landscape: landscape.checked || undefined,
        allowOverlap: overlap.checked || undefined });
      Object.keys(layout).forEach(k => layout[k] === undefined && delete layout[k]);
      closeSheet(); app.render();
    }, 'primary'),
  ]);
}

function openJsonSheet() {
  const { srv } = app.edit;
  const ta = h('textarea', { class: 'code', style: 'min-height:55dvh' }, JSON.stringify(app.edit.draft, null, 2));
  sheet(`All layouts on ${srv.label}`, h('div', { style: 'display:flex;flex-direction:column;gap:10px' },
    h('p', { class: 'note' }, 'This is the whole template for this PC (saved as layouts.json next to remote.py). Copy it to back up or move layouts between PCs.'), ta), [
    sheetBtn('Reset to default', async () => {
      if (!confirm(`Replace all layouts on ${srv.label} with the built-in default?`)) return;
      try { const r = await srv.request({ t: 'config_reset' }); app.edit = null; closeSheet(); srv.setConfig(r.config); app.render(); app.status('Reset to default', false, true); }
      catch (e) { app.status(e.message, true); }
    }, 'danger'),
    sheetBtn('Cancel', closeSheet),
    sheetBtn('Apply', () => {
      try {
        const c = JSON.parse(ta.value);
        if (!Array.isArray(c.layouts)) throw new Error('needs a "layouts" list');
        app.edit.draft = c; closeSheet(); app.render();
      } catch (e) { app.status('Invalid JSON: ' + e.message, true); }
    }, 'primary'),
  ]);
}

// ------------------------------------------------------------------ app

const app = {
  servers: [],
  tab: store.get('tab', null),   // "serverId/layoutId" or "settings"
  edit: null,                    // { srv, draft } while editing a server's layouts
  keyNames: new Set(),
  statusMsg: null,
  statusTimer: 0,

  async init() {
    // The key in the link (QR code) always wins over a saved one; then hide it from the address bar
    const params = new URLSearchParams(location.search), linkKey = params.get('k');
    if (linkKey) history.replaceState(null, '', location.pathname);
    let rejected = false;
    for (const key of [linkKey, store.get('selfKey', null), ''].filter((k, i, a) => k != null && a.indexOf(k) === i)) {
      const r = await this.trySelfKey(key);
      if (r === true) break;
      rejected = rejected || !!key || r === 403;
    }
    if (!this.servers.length && rejected) {
      store.set('selfKey', null);
      this.noKey = linkKey ? 'This link’s key is outdated: the PC has a new one.' : 'This phone has no valid key for this PC.';
    }
    for (const s of store.get('servers', [])) if (s.url !== location.origin) this.servers.push(new Server(s));

    $('#btn-edit').onclick = () => (this.edit ? this.cancelEdit() : this.startEdit());
    $('#btn-fs').onclick = $('#exit-immersive').onclick = () => this.toggleFullscreen();
    $('#ed-add').onclick = () => { const l = this.currentTab()?.layout; if (l) openWidgetSheet(l, this.newWidget(l), true); };
    $('#ed-layout').onclick = () => { const l = this.currentTab()?.layout; if (l) openLayoutSheet(l); };
    $('#ed-json').onclick = openJsonSheet;
    $('#ed-cancel').onclick = () => this.cancelEdit();
    $('#ed-save').onclick = () => this.saveEdit();
    document.addEventListener('fullscreenchange', () => this.renderView());
    document.addEventListener('visibilitychange', () => document.hidden && releaseEverything());
    addEventListener('blur', releaseEverything);
    addEventListener('keydown', e => this.hardwareKey(e));
    $('#btn-dev').onclick = () => this.openDeviceSheet();
    $('#ping').onclick = () => this.disconnectOffline();
    addEventListener('pointerdown', () => this.noteActivity(), { capture: true, passive: true });
    new ResizeObserver(() => this.fitTabs()).observe($('#tabs'));
    this.startTimers();
    this.render();
  },

  // Pings: a liveness check every 5 s; once a second while the latency is on screen. Keep-awake
  // heartbeats only while you're actually using the remote, and only when nothing else was sent.
  lastInput: -Infinity,
  lastSent: -Infinity,
  noteActivity() { this.lastSent = this.lastInput = performance.now(); },
  get showPing() { return store.get('showPing', false) || this.tab === 'settings'; },
  startTimers() {
    let lastLive = 0, lastShown = 0;
    setInterval(() => {
      if (document.hidden) return;
      const now = performance.now(), cur = this.currentServer();
      if (now - lastLive > 5000) {
        lastLive = now;
        this.servers.forEach(s => s.ping().then(ok => {
          if (ok === null) return;
          s.missed = ok ? 0 : (s.missed || 0) + 1;
          if (s.missed >= 2 && s.ws) s.ws.close();   // dead connection: reconnect
        }));
      } else if (this.showPing && now - lastShown > 1000) {
        lastShown = now;
        cur?.ping().then(() => this.refreshStatus());
      }
      const wake = store.get('wake', 'active');
      const using = wake === 'always' || (wake === 'active' && now - this.lastInput < 3000);
      if (using && now - this.lastSent > 80 && cur?.online) { this.lastSent = now; cur.ping(); }
    }, 50);
  },

  saveServers() {
    store.set('servers', this.servers.filter(s => s.id !== 'self').map(({ id, url, key, name }) => ({ id, url, key, name })));
  },
  // Asks this PC whether `key` is valid; on success connects to it. Returns true, or the HTTP status.
  async trySelfKey(key) {
    try {
      const r = await fetch('/api/self' + (key ? '?k=' + encodeURIComponent(key) : ''));
      if (!r.ok) return r.status;
      const self = await r.json();
      store.set('selfKey', self.key);
      this.servers = this.servers.filter(s => s.id !== 'self');
      this.servers.unshift(new Server({ id: 'self', url: location.origin, key: self.key, name: self.name }));
      return true;
    } catch { return 0; }
  },
  async enterKey(text) {
    text = text.trim();
    if (!text) return;
    let key = text;
    try { key = new URL(text).searchParams.get('k') || ''; } catch {}
    if (!key) return this.status('That link has no ?k= access key', true);
    const r = await this.trySelfKey(key);
    if (r !== true) return this.status(r === 403 ? 'Wrong key' : 'Could not reach the PC', true);
    this.noKey = null;
    this.render();
  },
  addServer(link) {
    let u;
    try { u = new URL(link.trim()); } catch { return this.status('Paste the full link from the PC console', true); }
    const key = u.searchParams.get('k');
    if (!key) return this.status('That link has no ?k= access key', true);
    const existing = this.servers.find(s => s.url === u.origin);
    if (existing) {   // same PC with a new key: update it instead of refusing
      if (existing.key === key) return this.status('Already added', true);
      existing.key = key;
      if (existing.ws) existing.ws.close(); else existing.connect();
      this.saveServers();
      return this.status('Key updated for ' + existing.label, false, true);
    }
    this.servers.push(new Server({ id: 's' + rid(), url: u.origin, key }));
    this.saveServers();
    this.render();
  },
  removeServer(s) {
    if (!confirm(`Remove ${s.label}?`)) return;
    s.close();
    this.servers = this.servers.filter(x => x !== s);
    this.saveServers();
    this.render();
  },
  disconnectOffline() {
    const s = this.currentServer();
    if (!s || s.online) return;
    if (s.id === 'self' && window.SofaAndroid?.disconnect) return window.SofaAndroid.disconnect();
    if (!confirm(`Disconnect from ${s.label}? Stop reconnecting to this PC?`)) return;
    s.close();
    this.servers = this.servers.filter(x => x !== s);
    if (s.id === 'self') {
      store.set('selfKey', null);
      this.noKey = 'Disconnected from ' + s.label + '.';
    }
    if (this.edit?.srv === s) this.edit = null;
    this.saveServers();
    this.render();
  },

  // Selected device: a server id, or 'all' to show every device's layouts together
  get device() {
    const d = store.get('device', null);
    if (d === 'all' || this.servers.some(s => s.id === d)) return d;
    return this.servers[0]?.id || 'all';
  },
  deviceServer() { return this.servers.find(s => s.id === this.device) || null; },
  currentServer() { return this.currentTab()?.srv || this.deviceServer() || this.servers[0]; },
  setDevice(id) {
    if (this.edit) return this.status('Save or cancel your edits first', true);
    store.set('device', id);
    this.tab = store.get('lastTabs', {})[id] || null;
    this.render();
  },
  openDeviceSheet() {
    const pick = id => { closeSheet(); this.setDevice(id); };
    const row = (id, dot, name, info) => h('button', { type: 'button', class: id === this.device ? 'active' : '', onclick: () => pick(id) },
      h('div', { class: 'row' }, dot, h('div', { class: 'grow' }, h('b', {}, name), h('small', {}, info))));
    sheet('Devices', h('div', { class: 'dev-list' },
      this.servers.map(s => row(s.id, h('span', { class: 'dot' + (s.online ? ' on' : '') }), s.label,
        s.online ? `${s.latency ?? '…'} ms · ${s.url}` : 'offline · ' + s.url)),
      this.servers.length > 1 ? row('all', h('span', { class: 'dot on', style: 'background:var(--acc)' }), 'All devices', 'every device’s layouts as tabs') : null), [
      window.SofaAndroid?.scanQr ? sheetBtn('Scan QR code', () => { closeSheet(); window.SofaAndroid.scanQr(); }) : null,
      sheetBtn('Add / manage devices', () => { closeSheet(); this.show('settings'); }),
      sheetBtn('Close', closeSheet),
    ]);
  },

  allTabs() {
    const loaded = this.servers.filter(s => s.config);
    const prefix = this.device === 'all' && loaded.length > 1;
    return loaded.flatMap(s => {
      const cfg = this.edit && this.edit.srv === s ? this.edit.draft : s.config;
      return (cfg.layouts || []).map((l, i) => ({
        id: `${s.id}/${l.id ?? i}`, srv: s, layout: l,
        label: (prefix ? s.label + ' · ' : '') + (l.name || 'Layout ' + (i + 1)),
        icon: l.icon || (l.name || '?').slice(0, 1).toUpperCase(),
      }));
    });
  },
  tabs() {
    const d = this.device;
    return d === 'all' ? this.allTabs() : this.allTabs().filter(t => t.srv.id === d);
  },
  // The remembered tab is kept even while its PC is still connecting; until then the first
  // available tab is shown without overwriting the choice.
  currentTab() {
    if (this.tab === 'settings') return null;
    const list = this.tabs();
    return list.find(t => t.id === this.tab) || (this.waitingForTab() ? null : list[0]) || null;
  },
  waitingForTab() {
    const sid = (this.tab || '').split('/')[0], s = this.servers.find(x => x.id === sid);
    return !!s && !s.config && !s.closed && (!s.ws || s.ws.readyState === 0 || s.online) && performance.now() < 4000;
  },
  show(id) {
    if (this.edit && id !== this.tab) {
      const t = this.tabs().find(x => x.id === id);
      if (!t || t.srv !== this.edit.srv) return this.status('Save or cancel your edits first', true);
    }
    this.tab = id;
    store.set('tab', id);
    store.set('lastTabs', { ...store.get('lastTabs', {}), [this.device]: id });
    this.render();
  },
  showByName(name) {
    const t = this.allTabs().find(x => x.layout.name === name || x.layout.id === name);
    if (!t) return;
    if (this.device !== 'all' && t.srv.id !== this.device) store.set('device', t.srv.id);
    this.show(t.id);
  },
  onConfig(srv) {
    if (this.edit && this.edit.srv === srv) return;   // keep the draft; the save will win
    this.render();
  },
  newWidget(layout) {
    const bottom = Math.max(0, ...(layout.widgets || []).map(w => (w.y || 0) + (w.h || 1)));
    return { type: 'button', x: 0, y: layout.fit ? 0 : bottom, w: 3, h: 1, icon: '★', label: 'New', action: { key: 'space' } };
  },

  startEdit() {
    const t = this.currentTab();
    if (!t) return this.status('Open a layout tab first', true);
    this.edit = { srv: t.srv, draft: structuredClone(t.srv.config) };
    this.render();
  },
  cancelEdit() { this.edit = null; this.render(); },
  async saveEdit() {
    const { srv, draft } = this.edit;
    try {
      await srv.request({ t: 'config_put', config: draft });
      srv.config = draft;
      this.edit = null;
      this.status('Saved on ' + srv.label, false, true);
      this.render();
    } catch (e) { this.status('Save failed: ' + e.message, true); }
  },

  render() {
    $('#edit-bar').hidden = !this.edit;
    document.body.classList.toggle('editing', !!this.edit);
    this.renderTabs();
    this.renderView();
  },
  renderTabs() {
    const nav = $('#tabs'), cur = this.currentTab()?.id ?? (this.tab === 'settings' ? 'settings' : null);
    nav.replaceChildren(
      ...this.tabs().map(t => h('button', { class: (t.id === cur ? 'active' : '') + (t.srv.online ? '' : ' off'),
        onclick: () => this.show(t.id), title: t.label, 'aria-label': t.label },
        iconEl(t.icon, 'tab-ico'), h('span', { class: 'tab-name' }, t.label))),
      h('button', { class: cur === 'settings' ? 'active' : '', onclick: () => this.show('settings'), 'aria-label': 'Settings', title: 'Settings' },
        iconEl('lucide:settings', 'tab-ico')));
    this.fitTabs();
  },
  // Full labels when they fit; icons only when they don't; scroll if even icons don't fit.
  fitTabs() {
    const nav = $('#tabs');
    nav.classList.remove('compact');
    if (nav.scrollWidth > nav.clientWidth + 1) nav.classList.add('compact');
    const active = $('.active', nav);
    if (active) active.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  },
  renderView() {
    const view = $('#view');
    const t = this.currentTab();
    if (!t && this.tab !== 'settings') {
      const dev = this.deviceServer();
      if (this.noKey && !this.servers.length) {
        const input = h('input', { type: 'text', placeholder: 'Key or link', autocapitalize: 'off', autocomplete: 'off', spellcheck: 'false',
          enterkeyhint: 'go', onkeydown: e => e.key === 'Enter' && this.enterKey(input.value) });
        view.replaceChildren(h('div', { class: 'placeholder keyform' }, h('b', {}, this.noKey),
          h('p', {}, 'Scan the QR code shown on the PC, or type the key (the part after ?k= in the link) or paste the whole link:'),
          h('div', { class: 'row' }, input, h('button', { class: 'small primary', onclick: () => this.enterKey(input.value) }, 'Connect'))));
        this.refreshStatus();
        return;
      }
      const msg = !this.servers.length ? 'Not connected' : dev ? `${dev.label}: ${dev.online ? 'loading…' : 'offline, reconnecting…'}` : 'Connecting…';
      view.replaceChildren(h('div', { class: 'placeholder' }, msg));
      this.refreshStatus();
      return;
    }
    const immersive = !!document.fullscreenElement && !!t?.layout.fit && !this.edit;
    document.body.classList.toggle('immersive', immersive);
    $('#exit-immersive').hidden = !immersive;
    view.replaceChildren(t ? renderLayout(t.srv, t.layout, !!this.edit && this.edit.srv === t.srv) : this.renderSettings());
    watchScroll(view);
    this.refreshStatus();
  },
  renderSettings() {
    const input = h('input', { placeholder: 'http://192.168.0.x:8765/?k=…', autocomplete: 'off', autocapitalize: 'off' });
    return h('div', { class: 'settings' },
      h('h2', {}, 'Connected PCs'),
      window.SofaAndroid?.scanQr ? h('button', { class: 'wide', onclick: () => window.SofaAndroid.scanQr() }, 'Scan QR code') : null,
      this.servers.map(s => h('div', { class: 'card row' },
        h('span', { class: 'dot' + (s.online ? ' on' : '') }),
        h('div', { class: 'grow' }, h('b', {}, s.label), h('small', { class: 'srv-stats', 'data-id': s.id }, this.serverInfo(s))),
        s.id === 'self' ? h('small', { style: 'color:var(--dim)' }, 'this PC') : h('button', { class: 'small', onclick: () => this.removeServer(s) }, 'Remove'))),
      h('div', { class: 'row' }, input, h('button', { class: 'small primary', onclick: () => this.addServer(input.value) }, 'Add')),
      h('p', { class: 'note' }, 'Run sofa-remote on another PC and paste the link it prints, then pick it with the device button at the top. Any button can also target another PC or all of them (“Sends to” when editing). Latency: min/median/max round trip.'),
      check('Open the keyboard when a text field is selected on the PC', h('input', { type: 'checkbox', checked: store.get('autoKb', true),
        onchange: e => { store.set('autoKb', e.target.checked); if (!e.target.checked) this.hideKeyBar(); } })),
      h('h2', {}, 'Look'),
      h('div', { class: 'themes' }, THEMES.map(t => h('button', {
        class: 'theme' + ((store.get('theme', 'night') === t.id) ? ' on' : ''),
        style: `--t-bg:${t.colors[0]};--t-fg:${t.dark ? '#e8e9ee' : '#1b1d24'}`,
        onclick: () => { store.set('theme', applyTheme(t.id)); this.render(); },
      }, h('span', { class: 'swatch' }, t.colors.map(c => h('i', { style: `--c:${c}` }))), t.name))),
      this.phoneButtonSettings(),
      h('h2', {}, 'Latency'),
      check('Show latency on screen', h('input', { type: 'checkbox', checked: store.get('showPing', false),
        onchange: e => { store.set('showPing', e.target.checked); this.refreshStatus(); } })),
      field('Connection for inputs', h('select', { onchange: e => {
        store.set('transport', e.target.value);
        this.servers.forEach(s => { s.stopRtc(); s.samples = []; s.startRtc(); });
        this.status('Switched', false, true);
      } }, [['auto', 'WebRTC (UDP) when available'], ['ws', 'WebSocket (TCP) only']]
          .map(([v, n]) => h('option', { value: v, selected: store.get('transport', 'auto') === v }, n)))),
      field('Keep phone Wi-Fi awake', h('select', { onchange: e => store.set('wake', e.target.value) },
        [['active', 'While using the remote (3 s after each touch)'], ['always', 'Always while open'], ['off', 'Off']]
          .map(([v, n]) => h('option', { value: v, selected: store.get('wake', 'active') === v }, n)))),
      h('p', { class: 'note' }, 'Inputs travel as a few binary bytes. WebRTC uses UDP: a lost Wi-Fi packet doesn’t hold up the ones behind it (with TCP it can stall for 100 ms+), and each key event is sent three times so one loss doesn’t matter. Keep-awake sends a tiny packet only when nothing else went out for 80 ms, so the phone’s Wi-Fi doesn’t doze off between your taps. The round trip shown includes the reply’s way back, which phones delay the most; your input only takes the one-way half. For the lowest numbers: 5 GHz Wi-Fi, PC on Ethernet, and on a Wi-Fi PC set the adapter’s power saving to “Maximum performance”.'),
      h('h2', {}, 'Layouts'),
      h('p', { class: 'note' }, 'Open a tab and tap the pencil: drag widgets to move them, drag the blue dot to resize, tap one to change what it does. “+ Widget” adds buttons, joysticks, touchpads and more. Save stores the layouts on that PC (layouts.json), replacing its default template; JSON › Reset brings the default back.'),
      h('h2', {}, 'Gaming'),
      h('p', { class: 'note' }, 'Use “Hold key” and “Hold mouse button” actions so keys stay down while your finger does, and joysticks for WASD or mouse-look. Several fingers work at once. The expand button goes fullscreen; layouts set to “fit” then fill the whole screen.'),
      h('button', { class: 'wide', onclick: () => { releaseEverything(); this.status('Released all keys', false, true); } }, 'Release all held keys'));
  },

  // The PC reports when a text field gets focus there: open a typing bar (and the phone keyboard)
  onTextFocus(srv, on) {
    const dev = this.deviceServer();
    if (!store.get('autoKb', true) || this.edit || (dev && dev !== srv)) return;
    if (!on) return this.hideKeyBar();
    const bar = $('#keybar');
    if (this.keyBarSrv !== srv) {
      this.keyBarSrv = srv;
      bar.replaceChildren(WIDGETS.keyboard({ label: 'Typing on ' + srv.label }, srv),
        h('button', { class: 'small', 'aria-label': 'Close', onclick: () => this.hideKeyBar() }, iconEl('lucide:x')));
    }
    bar.hidden = false;
    $('input', bar).focus({ preventScroll: true });   // browsers may need a tap on it to open the keyboard
    window.SofaAndroid?.showKeyboard?.();             // the Android app can always open it
  },
  hideKeyBar() {
    const bar = $('#keybar');
    if (bar.hidden) return;
    bar.hidden = true;
    if (bar.contains(document.activeElement)) document.activeElement.blur();
  },

  // Only inside the Android app, which exposes window.SofaAndroid: what the phone's hardware
  // buttons do (they work with the screen locked too, through a media notification).
  phoneButtonSettings() {
    const bridge = window.SofaAndroid;
    if (!bridge) return null;
    let cfg = {};
    try { cfg = JSON.parse(bridge.getButtons()); } catch {}
    const save = () => bridge.setButtons(JSON.stringify(cfg));
    const action = (label, k) => field(label, h('input', { value: cfg[k] ?? '', list: 'keylist', placeholder: 'nothing',
      autocapitalize: 'off', autocomplete: 'off', onchange: e => { cfg[k] = e.target.value.trim(); save(); } }));
    const toggle = (label, k) => check(label, h('input', { type: 'checkbox', checked: !!cfg[k],
      onchange: e => { cfg[k] = e.target.checked; save(); } }));
    return [
      h('h2', {}, 'Phone buttons'),
      toggle('Use the volume buttons', 'enabled'),
      toggle('Keep working with the screen locked', 'locked'),
      action('Volume up', 'up'),
      action('Volume down', 'down'),
      action('Both together', 'both'),
      field('“Both” window (ms)', h('input', { type: 'number', min: 0, max: 600, value: cfg.window ?? 150,
        onchange: e => { cfg.window = clamp(+e.target.value || 0, 0, 600); save(); } })),
      action('Headset / notification ⏯', 'play'),
      action('Headset / notification ⏭', 'next'),
      action('Headset / notification ⏮', 'prev'),
      toggle('Silent audio keep-alive (if locked buttons stop working)', 'silent'),
      h('p', { class: 'note' }, 'Actions: a key (volup, space, f, right…), a shortcut (ctrl+right), click / rclick / dblclick, scroll:3 (down; negative = up), text:hello, open:netflix.com. Leave empty for nothing. With volup/voldown on the two buttons, presses go out instantly and pressing both cancels them out; with other actions a single press waits for the “both” window first.'),
      h('button', { class: 'wide', onclick: () => bridge.changePc() }, 'Connect the app to another PC'),
      bridge.exit ? h('button', { class: 'wide danger', onclick: () => bridge.exit() }, 'Exit the app') : null,
      h('p', { class: 'note' }, 'Exit also stops the volume buttons and the media notification. Back twice does the same.'),
    ];
  },

  serverInfo(s) {
    if (!s.online) return s.url + ' · offline';
    return `${s.url} · ${s.transport}` + (s.stats ? ` · ${s.stats.min}/${s.stats.med}/${s.stats.max} ms` : '');
  },
  // Messages appear as a toast at the bottom
  status(msg, err = false, ok = false) {
    const el = $('#toast');
    el.textContent = msg;
    el.className = 'show' + (err ? ' err' : ok ? ' ok' : '');
    clearTimeout(this.statusTimer);
    this.statusTimer = setTimeout(() => (el.className = ''), 2500);
  },
  refreshStatus() {
    const dev = this.deviceServer(), btn = $('#btn-dev');
    btn.hidden = !this.servers.length || (this.servers.length < 2 && !!dev);
    btn.replaceChildren(
      h('span', { class: 'dot' + (dev ? (dev.online ? ' on' : '') : ' on'), style: dev ? null : 'background:var(--acc)' }),
      h('span', {}, dev ? dev.label : 'All'), iconEl('lucide:chevron-down'));
    // Offline is always shown; the latency only if enabled
    const srv = this.currentServer(), el = $('#ping');
    el.disabled = !srv || srv.online;
    el.title = srv && !srv.online ? 'Disconnect from ' + srv.label : '';
    el.setAttribute('aria-label', srv && !srv.online ? 'Offline. Disconnect from ' + srv.label : 'Connection latency');
    if (srv && !srv.online) { el.hidden = false; el.textContent = '○ offline'; el.className = 'err'; }
    else if (srv && store.get('showPing', false)) {
      el.hidden = false;
      el.textContent = (srv.latency != null ? srv.latency.toFixed(1) + ' ms' : '… ms') + (srv.dc ? ' · UDP' : ' · TCP');
      el.className = srv.latency == null ? '' : srv.latency < 15 ? 'ok' : srv.latency < 40 ? 'warn' : 'err';
    } else el.hidden = true;
    document.querySelectorAll('.srv-stats').forEach(e => {
      const s = this.servers.find(x => x.id === e.dataset.id);
      if (s) e.textContent = this.serverInfo(s);
    });
  },
  updateKeyList() {
    $('#keylist').replaceChildren(...[...this.keyNames].map(k => h('option', { value: k })));
  },

  async toggleFullscreen() {
    if (document.fullscreenElement) return document.exitFullscreen();
    try {
      await document.documentElement.requestFullscreen({ navigationUI: 'hide' });
      if (this.currentTab()?.layout.landscape) await screen.orientation.lock('landscape').catch(() => {});
    } catch { this.status('Fullscreen not available: use “Add to Home screen”', true); }
  },

  // Hardware keys (e.g. a Bluetooth keyboard) forwarded to the current PC
  hardwareKey(e) {
    if (e.target.matches('input, textarea, select') || $('#sheet').open) return;
    const map = { AudioVolumeUp: 'volup', AudioVolumeDown: 'voldown', AudioVolumeMute: 'mute', MediaPlayPause: 'playpause',
      ' ': 'playpause', ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'volup', ArrowDown: 'voldown',
      MediaTrackNext: 'next', MediaTrackPrevious: 'prev', f: 'f', Escape: 'esc' };
    const a = map[e.key], srv = this.currentTab()?.srv;
    if (a && srv) { e.preventDefault(); srv.send({ t: 'key', k: a }); }
  },
};

app.init();
