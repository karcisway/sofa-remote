const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');

// Load the actual client controller without booting the page or opening sockets.
const source = fs.readFileSync(path.join(__dirname, '../web/app.js'), 'utf8');
const serverCode = source.slice(source.indexOf('const B ='), source.indexOf('\nfunction findServer'));
function client() {
  let now = 10000;
  const context = vm.createContext({
    store: { get: () => 'test-client' },
    Date: { now: () => now },
    app: { noteActivity() {}, status() {} },
    URL,
  });
  const Server = vm.runInContext(serverCode + '\nServer', context);
  Server.prototype.connect = function () {
    this.ws = { readyState: 1 };
    this.pad = { buttons: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 };
    this.padOpened = this.padOpening = false;
    this.padRetryAt = 0;
  };
  const server = new Server({ url: 'http://localhost' });
  const requests = [], packets = [];
  server.request = () => new Promise((resolve, reject) => requests.push({ resolve, reject }));
  server.transmit = packet => packets.push(new DataView(packet));
  return { server, requests, packets, advance: ms => { now += ms; } };
}
const flush = () => new Promise(resolve => setImmediate(resolve));

test('failed controller connection retries after manual driver installation', async () => {
  const { server, requests, advance } = client();
  server.padSet({ buttons: 0x1000 });
  server.padSet({ buttons: 0 });
  assert.equal(requests.length, 1);
  assert.equal(server.padOpened, false);
  requests[0].reject(new Error('missing driver'));
  await flush();
  server.padSet({ buttons: 0x1000 });
  assert.equal(requests.length, 1);
  advance(2000);
  server.padSet({ buttons: 0 });
  assert.equal(requests.length, 2);
  requests[1].resolve({ player: 1 });
  await flush();
  assert.equal(server.padOpened, true);
  assert.equal(server.padOpening, false);
});

test('connection success sends the latest released state', async () => {
  const { server, requests, packets } = client();
  server.padSet({ buttons: 0x1000, lx: 123 });
  server.padSet({ buttons: 0, lx: 0 });
  requests[0].resolve({ player: 1 });
  await flush();
  assert.equal(packets.length, 3);
  assert.equal(packets.at(-1).getUint16(3, true), 0);
  assert.equal(packets.at(-1).getInt16(7, true), 0);
  assert.equal(requests.length, 1);
});

test('old connection replies cannot change a reconnected controller', async () => {
  const { server, requests } = client();
  server.padSet({ buttons: 0 });
  server.connect();
  server.padSet({ buttons: 0 });
  requests[0].resolve({ player: 1 });
  await flush();
  assert.equal(server.padOpened, false);
  assert.equal(server.padOpening, true);
  requests[1].resolve({ player: 2 });
  await flush();
  assert.equal(server.padOpened, true);
  assert.equal(server.padOpening, false);
});
