const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../web/app.js'), 'utf8');
const code = source.slice(source.indexOf('const POWER ='), source.indexOf('\n// Every pressed control'));
const { holdFor, defaultConfirm, HOLD_MS } = vm.runInContext(
  code + '\n({ holdFor, defaultConfirm, HOLD_MS })',
  vm.createContext({ clamp: (v, lo, hi) => Math.max(lo, Math.min(hi, v)) }),
);

test('shutting down, restarting, hibernating and signing out must be held by default', () => {
  for (const power of ['shutdown', 'restart', 'hibernate', 'signout']) assert.equal(holdFor({}, { power }), HOLD_MS, power);
});

test('sleep, lock and screen off run at once', () => {
  for (const power of ['sleep', 'lock', 'screenoff']) assert.equal(holdFor({}, { power }), 0, power);
});

test('confirm turns holding on for any button, sets its time, and off for the power ones', () => {
  assert.equal(holdFor({ confirm: true }, { keys: ['alt', 'f4'] }), HOLD_MS);
  assert.equal(holdFor({ confirm: 2500 }, { key: 'f' }), 2500);
  assert.equal(holdFor({ confirm: 1 }, { key: 'f' }), 200);
  assert.equal(holdFor({ confirm: false }, { power: 'shutdown' }), 0);
  assert.equal(holdFor({}, { key: 'f' }), 0);
});

test('game hold actions are never delayed', () => {
  assert.equal(holdFor({ confirm: true }, { hold: 'w' }), 0);
  assert.equal(holdFor({ confirm: true }, { pad: 'a' }), 0);
});

test('the editor default matches', () => {
  assert.equal(defaultConfirm({ power: 'shutdown' }), true);
  assert.equal(defaultConfirm({ power: 'sleep' }), false);
  assert.equal(defaultConfirm(null), false);
});
