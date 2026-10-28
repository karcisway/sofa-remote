const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../web/app.js'), 'utf8');
const serverCode = source.slice(source.indexOf('const B ='), source.indexOf('\nfunction findServer'));
const appCode = source.slice(source.indexOf('const app = {'), source.lastIndexOf('\napp.init();'));
function load({ accepted = true, bridge } = {}) {
  const saved = new Map(), timers = [], sockets = [];
  let prompts = 0;
  const context = vm.createContext({
    store: { get: (key, fallback) => saved.get(key) ?? fallback, set: (key, value) => saved.set(key, value) },
    rid: () => 'test-client', URL,
    window: { SofaAndroid: bridge },
    confirm: () => { prompts++; return accepted; },
    setTimeout: callback => timers.push(callback),
    WebSocket: class {
      constructor() { sockets.push(this); }
      close() { this.readyState = 3; this.onclose?.(); }
    },
  });
  const { Server, app } = vm.runInContext(serverCode + '\n' + appCode + '\n({Server, app})', context);
  app.render = app.refreshStatus = app.renderTabs = () => {};
  const server = new Server({ id: 'secondary', url: 'http://pc', name: 'Living room' });
  app.servers = [server];
  app.currentServer = () => server;
  return { server, app, saved, timers, sockets, prompts: () => prompts };
}

test('disconnect stops a previously scheduled reconnect', () => {
  const { server, timers, sockets } = load();
  server.ws.onclose();
  assert.equal(timers.length, 1);
  server.close();
  timers[0]();
  assert.equal(sockets.length, 1);
  assert.equal(server.closed, true);
});

test('cancel leaves the offline connection available for retry', () => {
  const { server, app, prompts } = load({ accepted: false });
  app.disconnectOffline();
  assert.equal(prompts(), 1);
  assert.equal(server.closed, false);
  assert.equal(app.servers.length, 1);
});

test('confirmed disconnect removes the offline PC and its saved connection', () => {
  const { server, app, saved } = load();
  app.disconnectOffline();
  assert.equal(server.closed, true);
  assert.equal(app.servers.length, 0);
  assert.equal(saved.get('servers').length, 0);
});

test('Android primary disconnect uses the native confirmation', () => {
  let nativeRequests = 0;
  const { server, app, prompts } = load({ bridge: { disconnect: () => nativeRequests++ } });
  server.id = 'self';
  app.disconnectOffline();
  assert.equal(nativeRequests, 1);
  assert.equal(prompts(), 0);
  assert.equal(server.closed, false);
});

test('online status cannot disconnect a PC', () => {
  const { server, app, prompts } = load();
  server.ws.readyState = 1;
  app.disconnectOffline();
  assert.equal(prompts(), 0);
  assert.equal(server.closed, false);
});
